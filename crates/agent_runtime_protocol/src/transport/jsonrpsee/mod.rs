//! jsonrpsee subscription carrier.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::{Channel, RawJsonRpcMessage};
use jsonrpsee::RpcModule;
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::params::ObjectParams;
use jsonrpsee::server::SubscriptionMessage as RpcSubscriptionMessage;
use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

#[cfg(test)]
mod test;

/// Runtime-to-server method carrying one complete logical JSON-RPC message.
pub const SEND_METHOD: &str = "send";
/// Runtime request that opens the server-to-runtime message subscription.
pub const SUBSCRIBE_METHOD: &str = "subscribe";
/// Server notification used for subscription items.
pub const MESSAGE_METHOD: &str = "message";
/// Runtime request that closes the message subscription.
pub const UNSUBSCRIBE_METHOD: &str = "unsubscribe";

/// Parameters used by a runtime to identify a logical connection subscription.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscribe {
    /// Runtime-chosen identity for this physical connection attempt.
    pub connection_id: String,
}

impl Subscribe {
    /// Construct subscription parameters.
    #[must_use]
    pub fn new(connection_id: impl Into<String>) -> Self {
        Self {
            connection_id: connection_id.into(),
        }
    }
}

/// A complete logical message sent from the runtime to the server.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMessage {
    /// Identifies the subscription that owns this message.
    pub connection_id: String,
    /// Complete logical JSON-RPC request, notification, or response.
    pub message: RawJsonRpcMessage,
}

impl RuntimeMessage {
    /// Wrap one logical message for the runtime-to-server carrier.
    #[must_use]
    pub fn new(connection_id: impl Into<String>, message: RawJsonRpcMessage) -> Self {
        Self {
            connection_id: connection_id.into(),
            message,
        }
    }
}

/// One server-to-runtime subscription item.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SubscriptionMessage {
    /// Complete logical JSON-RPC request, notification, or response.
    pub message: RawJsonRpcMessage,
}

impl SubscriptionMessage {
    /// Wrap one logical message for a subscription item.
    #[must_use]
    pub fn new(message: RawJsonRpcMessage) -> Self {
        Self { message }
    }
}

/// A logical channel accepted from a runtime subscription.
#[derive(Debug)]
pub struct IncomingConnection {
    connection_id: String,
    generation: u64,
    channel: Channel,
}

impl IncomingConnection {
    /// Return the runtime-chosen connection identity.
    #[must_use]
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// Consume the accepted connection and return its logical channel.
    #[must_use]
    pub fn into_channel(self) -> Channel {
        self.channel
    }
}

/// A jsonrpsee server module and its stream of accepted runtime subscriptions.
pub struct ServerTransport {
    state: ServerState,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<IncomingConnection>>,
}

#[derive(Clone)]
struct ServerState {
    routes: Arc<Mutex<HashMap<String, Route>>>,
    incoming: mpsc::UnboundedSender<IncomingConnection>,
    generation: Arc<AtomicU64>,
}

struct Route {
    generation: u64,
    inbound: futures::channel::mpsc::UnboundedSender<
        Result<RawJsonRpcMessage, agent_client_protocol::Error>,
    >,
    cancel: oneshot::Sender<()>,
}

impl ServerTransport {
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
    pub fn rpc_module(&self) -> Result<RpcModule<()>, jsonrpsee::core::RegisterMethodError> {
        let mut module = RpcModule::new(());
        let state = self.state.clone();
        module.register_async_method(SEND_METHOD, move |params, _, _| {
            let state = state.clone();
            async move {
                let delivery = params.parse::<RuntimeMessage>()?;
                let inbound = state
                    .routes
                    .lock()
                    .map_err(|_| internal_error("connection registry is unavailable"))?
                    .get(&delivery.connection_id)
                    .map(|route| route.inbound.clone())
                    .ok_or_else(|| connection_not_found(&delivery.connection_id))?;
                inbound
                    .unbounded_send(Ok(delivery.message))
                    .map_err(|_| connection_not_found(&delivery.connection_id))?;
                Ok::<(), ErrorObjectOwned>(())
            }
        })?;

        let state = self.state.clone();
        module.register_subscription(
            SUBSCRIBE_METHOD,
            MESSAGE_METHOD,
            UNSUBSCRIBE_METHOD,
            move |params, pending, _, _| {
                let state = state.clone();
                async move {
                    let subscribe = match params.parse::<Subscribe>() {
                        Ok(subscribe) => subscribe,
                        Err(error) => {
                            pending.reject(error).await;
                            return;
                        }
                    };
                    let Ok(sink) = pending.accept().await else {
                        return;
                    };

                    let generation = state.generation.fetch_add(1, Ordering::Relaxed);
                    let (connection, carrier) = Channel::duplex();
                    let Channel {
                        mut rx,
                        tx: inbound,
                    } = carrier;
                    let (cancel, mut cancelled) = oneshot::channel();
                    let previous = state.routes.lock().ok().and_then(|mut routes| {
                        routes.insert(
                            subscribe.connection_id.clone(),
                            Route {
                                generation,
                                inbound,
                                cancel,
                            },
                        )
                    });
                    if let Some(previous) = previous {
                        let _ = previous.cancel.send(());
                    }

                    if state
                        .incoming
                        .send(IncomingConnection {
                            connection_id: subscribe.connection_id.clone(),
                            generation,
                            channel: connection,
                        })
                        .is_err()
                    {
                        remove_route(&state, &subscribe.connection_id, generation);
                        return;
                    }

                    loop {
                        tokio::select! {
                            message = futures::StreamExt::next(&mut rx) => {
                                let Some(Ok(message)) = message else {
                                    break;
                                };
                                let Ok(message) = serde_json::value::to_raw_value(
                                    &SubscriptionMessage::new(message),
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
                    remove_route(&state, &subscribe.connection_id, generation);
                }
            },
        )?;

        Ok(module)
    }

    /// Wait for the next accepted runtime subscription.
    pub async fn accept(&self) -> Option<IncomingConnection> {
        let mut incoming = self.incoming.lock().await;
        loop {
            let connection = incoming.recv().await?;
            let is_active = self
                .state
                .routes
                .lock()
                .ok()
                .and_then(|routes| {
                    routes
                        .get(&connection.connection_id)
                        .map(|route| route.generation == connection.generation)
                })
                .unwrap_or(false);
            if is_active {
                return Some(connection);
            }
        }
    }
}

impl Default for ServerTransport {
    fn default() -> Self {
        Self::new()
    }
}

/// A failure in the jsonrpsee carrier.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// A jsonrpsee client operation failed.
    #[error("jsonrpsee client failed: {0}")]
    Client(String),
}

/// Subscribe a runtime-side jsonrpsee client and expose its logical channel.
pub async fn connect_runtime<C>(
    client: Arc<C>,
    connection_id: impl Into<String>,
) -> Result<Channel, TransportError>
where
    C: ClientT + SubscriptionClientT + Send + Sync + 'static,
{
    let connection_id = connection_id.into();
    let mut params = ObjectParams::new();
    params
        .insert("connectionId", &connection_id)
        .map_err(|error| TransportError::Client(error.to_string()))?;
    let mut subscription = client
        .subscribe::<SubscriptionMessage, _>(SUBSCRIBE_METHOD, params, UNSUBSCRIBE_METHOD)
        .await
        .map_err(|error| TransportError::Client(error.to_string()))?;
    let (connection, carrier) = Channel::duplex();
    let Channel {
        mut rx,
        tx: inbound,
    } = carrier;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                message = futures::StreamExt::next(&mut subscription) => {
                    let Some(Ok(message)) = message else {
                        break;
                    };
                    if inbound.unbounded_send(Ok(message.message)).is_err() {
                        break;
                    }
                }
                message = futures::StreamExt::next(&mut rx) => {
                    let Some(Ok(message)) = message else {
                        break;
                    };
                    let mut params = ObjectParams::new();
                    if params.insert("connectionId", &connection_id).is_err()
                        || params.insert("message", &message).is_err()
                    {
                        break;
                    }
                    if client.request::<(), _>(SEND_METHOD, params).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    Ok(connection)
}

fn remove_route(state: &ServerState, connection_id: &str, generation: u64) {
    if let Ok(mut routes) = state.routes.lock() {
        let owns_route = routes
            .get(connection_id)
            .is_some_and(|route| route.generation == generation);
        if owns_route {
            routes.remove(connection_id);
        }
    }
}

fn connection_not_found(connection_id: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        -32_004,
        "connection not found",
        Some(serde_json::json!({ "connectionId": connection_id })),
    )
}

fn internal_error(message: &'static str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32_603, message, None::<()>)
}
