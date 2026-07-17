//! jsonrpsee WebSocket carrier.
//!
//! The runtime initiates the WebSocket, so it is the jsonrpsee client. A
//! jsonrpsee client can make requests and open subscriptions, but it cannot
//! register a handler for a new request initiated by the server. So the
//! service cannot send a normal top-level JSON-RPC request directly to the
//! runtime; instead we tunnel complete logical messages:
//!
//! - the runtime opens a subscription for service-to-runtime traffic;
//! - the service puts logical messages into subscription items;
//! - the runtime uses `send` for runtime-to-service traffic.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use jsonrpsee::ConnectionId;
use jsonrpsee::RpcModule;
use jsonrpsee::core::client::{ClientT, Subscription, SubscriptionClientT};
use jsonrpsee::core::params::ObjectParams;
use jsonrpsee::server::SubscriptionMessage as RpcSubscriptionMessage;
use jsonrpsee::types::ErrorObjectOwned;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Mutex as AsyncMutex, OnceCell, mpsc, oneshot};

use crate::domain::channel::{Channel, pump};
use crate::domain::ports::{Transport, TransportError};

#[cfg(test)]
mod test;

/// Runtime-to-server method carrying one complete logical message.
pub(crate) const SEND_METHOD: &str = "send";
/// Runtime request that opens the server-to-runtime message subscription.
pub(crate) const SUBSCRIBE_METHOD: &str = "subscribe";
/// Server notification used for subscription items.
pub(crate) const MESSAGE_METHOD: &str = "message";
/// Runtime request that closes the message subscription.
pub(crate) const UNSUBSCRIBE_METHOD: &str = "unsubscribe";

/// Wire framing for one logical message, in either direction.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Envelope<T> {
    message: T,
}

/// A jsonrpsee server module and its stream of accepted runtime connections.
///
/// One physical connection is exactly one logical session: the subscription
/// that opens it is keyed by jsonrpsee's own [`ConnectionId`], so there is no
/// caller-chosen identifier and no way to multiplex several logical sessions
/// over one socket.
pub struct ServerTransport<Tx, Rx> {
    state: ServerState<Tx, Rx>,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<Channel<Tx, Rx>>>,
}

struct ServerState<Tx, Rx> {
    routes: Arc<Mutex<HashMap<ConnectionId, Route<Rx>>>>,
    incoming: mpsc::UnboundedSender<Channel<Tx, Rx>>,
    // Guards against a stale subscription's cleanup racing with (and deleting)
    // a route a later subscription just installed for the same `ConnectionId`.
    // This is not connection multiplexing: a `ConnectionId` is one physical
    // socket, so this only matters if that socket calls `subscribe` again.
    generation: Arc<AtomicU64>,
}

impl<Tx, Rx> Clone for ServerState<Tx, Rx> {
    fn clone(&self) -> Self {
        Self {
            routes: Arc::clone(&self.routes),
            incoming: self.incoming.clone(),
            generation: Arc::clone(&self.generation),
        }
    }
}

struct Route<Rx> {
    generation: u64,
    inbound: UnboundedSender<Rx>,
    cancel: oneshot::Sender<()>,
}

impl<Tx, Rx> ServerTransport<Tx, Rx> {
    /// Construct an empty server carrier.
    #[must_use]
    pub fn new() -> Self {
        let (incoming_sender, incoming) = mpsc::unbounded_channel();
        Self {
            state: ServerState {
                routes: Arc::new(Mutex::new(HashMap::new())),
                incoming: incoming_sender,
                generation: Arc::new(AtomicU64::new(1)),
            },
            incoming: tokio::sync::Mutex::new(incoming),
        }
    }

    /// Build the RPC module that accepts runtime messages and subscriptions.
    pub fn rpc_module(&self) -> Result<RpcModule<()>, jsonrpsee::core::RegisterMethodError>
    where
        Tx: Clone + Serialize + Send + Sync + 'static,
        Rx: Clone + DeserializeOwned + Send + Sync + 'static,
    {
        let mut module = RpcModule::new(());
        let state = self.state.clone();
        module.register_async_method(SEND_METHOD, move |params, _, extensions| {
            let state = state.clone();
            async move {
                let connection_id = *extensions
                    .get::<ConnectionId>()
                    .ok_or_else(|| internal_error("connection id is unavailable"))?;
                let delivery = params.parse::<Envelope<Rx>>()?;
                let inbound = state
                    .routes
                    .lock()
                    .map_err(|_| internal_error("connection registry is unavailable"))?
                    .get(&connection_id)
                    .map(|route| route.inbound.clone())
                    .ok_or_else(no_active_subscription)?;
                inbound
                    .send(delivery.message)
                    .map_err(|_| no_active_subscription())?;
                Ok::<(), ErrorObjectOwned>(())
            }
        })?;

        let state = self.state.clone();
        module.register_subscription(
            SUBSCRIBE_METHOD,
            MESSAGE_METHOD,
            UNSUBSCRIBE_METHOD,
            move |_params, pending, _, extensions| {
                let state = state.clone();
                async move {
                    let Some(connection_id) = extensions.get::<ConnectionId>().copied() else {
                        pending
                            .reject(internal_error("connection id is unavailable"))
                            .await;
                        return;
                    };
                    let Ok(sink) = pending.accept().await else {
                        return;
                    };

                    let generation = state.generation.fetch_add(1, Ordering::Relaxed);
                    let (caller_side, worker_side) = Channel::<Tx, Rx>::duplex();
                    let Channel {
                        tx: worker_inbound,
                        rx: mut worker_outgoing,
                    } = worker_side;
                    let (cancel, mut cancelled) = oneshot::channel();
                    let previous = state.routes.lock().ok().and_then(|mut routes| {
                        routes.insert(
                            connection_id,
                            Route {
                                generation,
                                inbound: worker_inbound,
                                cancel,
                            },
                        )
                    });
                    if let Some(previous) = previous {
                        let _ = previous.cancel.send(());
                    }

                    if state.incoming.send(caller_side).is_err() {
                        remove_route(&state, connection_id, generation);
                        return;
                    }

                    loop {
                        tokio::select! {
                            message = worker_outgoing.recv() => {
                                let Some(message) = message else {
                                    break;
                                };
                                let Ok(message) = serde_json::value::to_raw_value(
                                    &Envelope { message },
                                ) else {
                                    break;
                                };
                                if sink.send(RpcSubscriptionMessage::from(message)).await.is_err() {
                                    break;
                                }
                            }
                            () = sink.closed() => break,
                            _ = &mut cancelled => break,
                        }
                    }
                    remove_route(&state, connection_id, generation);
                }
            },
        )?;

        Ok(module)
    }

    /// Wait for the next accepted runtime connection.
    pub async fn accept(&self) -> Option<Channel<Tx, Rx>> {
        self.incoming.lock().await.recv().await
    }
}

impl<Tx, Rx> Default for ServerTransport<Tx, Rx> {
    fn default() -> Self {
        Self::new()
    }
}

/// [`Transport`] adapter over a jsonrpsee client capable of both requests
/// and subscriptions.
///
/// Each [`JsonRpseeWire`] wraps one physical client connection, so it always
/// carries exactly one logical session. The server-to-runtime subscription
/// is opened lazily on the first [`Transport::send`] or [`Transport::recv`]
/// call and reused after.
pub struct JsonRpseeWire<C, Tx, Rx> {
    client: Arc<C>,
    // Establishing the subscription (the `OnceCell` itself) and polling it
    // (the inner `AsyncMutex`) are deliberately independent locks. `recv`
    // holds the inner mutex for the entire `next().await`, which can block
    // indefinitely waiting for the next item; if `send`'s `ensure_subscribed`
    // needed that same lock, one `recv` sitting idle between messages would
    // permanently deadlock every subsequent `send`. `OnceCell::get_or_try_init`
    // resolves near-instantly once initialized, so `send` never contends with
    // `recv`'s long-held poll.
    subscription: OnceCell<AsyncMutex<Subscription<Envelope<Rx>>>>,
    _tx: PhantomData<fn(Tx)>,
}

impl<C, Tx, Rx> JsonRpseeWire<C, Tx, Rx> {
    /// Wrap a jsonrpsee client for use as a runtime-side transport.
    #[must_use]
    pub fn new(client: Arc<C>) -> Self {
        Self {
            client,
            subscription: OnceCell::new(),
            _tx: PhantomData,
        }
    }
}

impl<C, Tx, Rx> JsonRpseeWire<C, Tx, Rx>
where
    C: ClientT + SubscriptionClientT + Send + Sync + 'static,
    Rx: DeserializeOwned + Send + Sync + 'static,
{
    /// Open the server-to-runtime subscription if it isn't already open.
    ///
    /// `send` and `recv` both call this before doing anything else. `send`
    /// depends on the subscription existing - the server's `send` handler
    /// looks up its route by the subscription's `ConnectionId` - so this
    /// guards against a runtime-to-server send racing ahead of the
    /// subscribe handshake on the same physical connection.
    async fn ensure_subscribed(
        &self,
    ) -> Result<&AsyncMutex<Subscription<Envelope<Rx>>>, TransportError> {
        self.subscription
            .get_or_try_init(|| async {
                self.client
                    .subscribe::<Envelope<Rx>, _>(
                        SUBSCRIBE_METHOD,
                        ObjectParams::new(),
                        UNSUBSCRIBE_METHOD,
                    )
                    .await
                    .map(AsyncMutex::new)
                    .map_err(|error| TransportError::Client(error.to_string()))
            })
            .await
    }
}

impl<C, Tx, Rx> Transport<Tx, Rx> for JsonRpseeWire<C, Tx, Rx>
where
    C: ClientT + SubscriptionClientT + Send + Sync + 'static,
    Tx: Serialize + Send + Sync + 'static,
    Rx: DeserializeOwned + Send + Sync + 'static,
{
    async fn send(&self, message: Tx) -> Result<(), TransportError> {
        self.ensure_subscribed().await?;
        let mut params = ObjectParams::new();
        params
            .insert("message", &message)
            .map_err(|error| TransportError::Client(error.to_string()))?;
        self.client
            .request::<(), _>(SEND_METHOD, params)
            .await
            .map_err(|error| TransportError::Client(error.to_string()))
    }

    async fn recv(&self) -> Result<Option<Rx>, TransportError> {
        let subscription = self.ensure_subscribed().await?;
        let mut opened = subscription.lock().await;
        match futures::StreamExt::next(&mut *opened).await {
            Some(Ok(envelope)) => Ok(Some(envelope.message)),
            Some(Err(error)) => Err(TransportError::Client(error.to_string())),
            None => Ok(None),
        }
    }
}

/// Bridge a runtime-side jsonrpsee client into its logical channel.
pub fn connect_runtime<C, Tx, Rx>(client: Arc<C>) -> Channel<Tx, Rx>
where
    C: ClientT + SubscriptionClientT + Send + Sync + 'static,
    Tx: Serialize + Send + Sync + 'static,
    Rx: DeserializeOwned + Send + Sync + 'static,
{
    pump(Arc::new(JsonRpseeWire::new(client)))
}

fn remove_route<Tx, Rx>(state: &ServerState<Tx, Rx>, connection_id: ConnectionId, generation: u64) {
    if let Ok(mut routes) = state.routes.lock() {
        let owns_route = routes
            .get(&connection_id)
            .is_some_and(|route| route.generation == generation);
        if owns_route {
            routes.remove(&connection_id);
        }
    }
}

fn no_active_subscription() -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        -32_004,
        "no active subscription for this connection",
        None::<()>,
    )
}

fn internal_error(message: &'static str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32_603, message, None::<()>)
}
