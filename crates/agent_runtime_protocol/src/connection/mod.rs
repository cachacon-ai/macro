//! Role-oriented logical protocol connections.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::RequestId;
use agent_client_protocol::{Channel, JsonRpcMessage, JsonRpcResponse, RawJsonRpcMessage};
use futures::channel::mpsc::UnboundedSender;
use futures::{FutureExt, StreamExt};
use serde_json::Value;
use tokio::sync::{oneshot, watch};

use crate::schema::v0::{
    ACP_METHOD, AcpMessage, COMMAND_METHOD, Command, CommandResult, SYSTEM_EVENT_METHOD,
    SystemEvent,
};

#[cfg(test)]
mod test;

type CommandReply = oneshot::Sender<Result<CommandResult, ConnectionError>>;
type PendingCommands = Arc<Mutex<HashMap<i64, CommandReply>>>;

struct PendingRequestGuard {
    request_id: i64,
    pending: PendingCommands,
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.request_id);
        }
    }
}

/// Identifies one execution of an ACP agent within a runtime.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AgentTarget {
    agent_id: String,
    agent_instance_id: String,
}

impl AgentTarget {
    /// Construct an agent target from its logical and process identifiers.
    #[must_use]
    pub fn new(agent_id: impl Into<String>, agent_instance_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            agent_instance_id: agent_instance_id.into(),
        }
    }

    /// Return the stable logical agent identifier.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Return the identifier for this execution of the agent.
    #[must_use]
    pub fn agent_instance_id(&self) -> &str {
        &self.agent_instance_id
    }
}

/// Handles a command delivered to an Agent Runtime.
///
/// Use `()` when the connection only carries ACP. It rejects any command as
/// an unknown method.
pub trait CommandHandler: Send + Sync + 'static {
    /// Execute the command and return its logical JSON-RPC result.
    fn handle(
        &self,
        command: Command,
    ) -> impl Future<Output = Result<CommandResult, agent_client_protocol::Error>> + Send;
}

impl CommandHandler for () {
    fn handle(
        &self,
        _command: Command,
    ) -> impl Future<Output = Result<CommandResult, agent_client_protocol::Error>> + Send {
        std::future::ready(Err(agent_client_protocol::Error::method_not_found()))
    }
}

/// Handles a system event delivered to an Agent Service.
///
/// Use `()` when the connection only carries ACP. It ignores any system event.
pub trait SystemEventHandler: Send + Sync + 'static {
    /// Observe a runtime or agent state transition.
    fn handle(
        &self,
        event: SystemEvent,
    ) -> impl Future<Output = Result<(), agent_client_protocol::Error>> + Send;
}

impl SystemEventHandler for () {
    fn handle(
        &self,
        _event: SystemEvent,
    ) -> impl Future<Output = Result<(), agent_client_protocol::Error>> + Send {
        std::future::ready(Ok(()))
    }
}

/// A failure while using a logical protocol connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The logical connection has closed.
    #[error("connection closed")]
    Closed,
    /// A message did not conform to the role or schema expected by the receiver.
    #[error("invalid protocol message: {0}")]
    InvalidMessage(String),
    /// The remote peer returned a JSON-RPC error for a command.
    #[error("command failed: {0}")]
    CommandFailed(#[source] agent_client_protocol::Error),
}

/// Agent Service-side access to one logical runtime connection.
pub struct ServerConnection {
    outbound: UnboundedSender<Result<RawJsonRpcMessage, agent_client_protocol::Error>>,
    pending: PendingCommands,
    next_request_id: AtomicU64,
    acp: AcpRouter,
    driver: tokio::task::AbortHandle,
}

impl ServerConnection {
    /// Attach the service role to a logical message channel.
    #[must_use]
    pub fn connect<H>(channel: Channel, system_events: H) -> Self
    where
        H: SystemEventHandler,
    {
        let Channel { rx, tx } = channel;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let acp = AcpRouter::new(tx.clone());
        let driver = tokio::spawn(run_server(
            rx,
            Arc::new(system_events),
            Arc::clone(&pending),
            acp.clone(),
        ))
        .abort_handle();

        Self {
            outbound: tx,
            pending,
            next_request_id: AtomicU64::new(1),
            acp,
            driver,
        }
    }

    /// Send a correlated command request and wait for its result.
    pub async fn command(&self, command: Command) -> Result<CommandResult, ConnectionError> {
        let request_id = i64::try_from(self.next_request_id.fetch_add(1, Ordering::Relaxed))
            .map_err(|_| ConnectionError::Closed)?;
        let message = RawJsonRpcMessage::request(
            COMMAND_METHOD.to_owned(),
            serde_json::to_value(command)
                .map_err(|error| ConnectionError::InvalidMessage(error.to_string()))?,
            RequestId::Number(request_id),
        )
        .map_err(|error| ConnectionError::InvalidMessage(error.to_string()))?;
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| ConnectionError::Closed)?
            .insert(request_id, sender);
        let _pending_request = PendingRequestGuard {
            request_id,
            pending: Arc::clone(&self.pending),
        };

        if self.outbound.unbounded_send(Ok(message)).is_err() {
            return Err(ConnectionError::Closed);
        }

        receiver.await.unwrap_or(Err(ConnectionError::Closed))
    }

    /// Open an official ACP SDK channel for one agent execution.
    pub fn acp(&self, target: AgentTarget) -> Result<Channel, ConnectionError> {
        self.acp.open(target)
    }
}

impl Drop for ServerConnection {
    fn drop(&mut self) {
        self.acp.close();
        self.driver.abort();
    }
}

/// Agent Runtime-side access to one logical service connection.
pub struct RuntimeConnection {
    outbound: UnboundedSender<Result<RawJsonRpcMessage, agent_client_protocol::Error>>,
    acp: AcpRouter,
    driver: tokio::task::AbortHandle,
}

impl RuntimeConnection {
    /// Attach the runtime role to a logical message channel.
    #[must_use]
    pub fn connect<H>(channel: Channel, commands: H) -> Self
    where
        H: CommandHandler,
    {
        let Channel { rx, tx } = channel;
        let acp = AcpRouter::new(tx.clone());
        let driver = tokio::spawn(run_runtime(rx, tx.clone(), Arc::new(commands), acp.clone()))
            .abort_handle();
        Self {
            outbound: tx,
            acp,
            driver,
        }
    }

    /// Send a system event notification to the Agent Service.
    pub fn system_event(&self, event: SystemEvent) -> Result<(), ConnectionError> {
        let message = RawJsonRpcMessage::notification(
            SYSTEM_EVENT_METHOD.to_owned(),
            serde_json::to_value(event)
                .map_err(|error| ConnectionError::InvalidMessage(error.to_string()))?,
        )
        .map_err(|error| ConnectionError::InvalidMessage(error.to_string()))?;
        self.outbound
            .unbounded_send(Ok(message))
            .map_err(|_| ConnectionError::Closed)
    }

    /// Open an official ACP SDK channel for one agent execution.
    pub fn acp(&self, target: AgentTarget) -> Result<Channel, ConnectionError> {
        self.acp.open(target)
    }
}

impl Drop for RuntimeConnection {
    fn drop(&mut self) {
        self.acp.close();
        self.driver.abort();
    }
}

type LogicalSender = UnboundedSender<Result<RawJsonRpcMessage, agent_client_protocol::Error>>;

#[derive(Clone)]
struct AcpRouter {
    outbound: LogicalSender,
    inbound: Arc<Mutex<HashMap<AgentTarget, LogicalSender>>>,
    next_message_id: Arc<AtomicU64>,
    shutdown: watch::Sender<bool>,
}

impl AcpRouter {
    fn new(outbound: LogicalSender) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            outbound,
            inbound: Arc::new(Mutex::new(HashMap::new())),
            next_message_id: Arc::new(AtomicU64::new(1)),
            shutdown,
        }
    }

    fn open(&self, target: AgentTarget) -> Result<Channel, ConnectionError> {
        if *self.shutdown.borrow() {
            return Err(ConnectionError::Closed);
        }
        let (peer, bridge) = Channel::duplex();
        let Channel { mut rx, tx } = bridge;
        let mut inbound = self.inbound.lock().map_err(|_| ConnectionError::Closed)?;
        if inbound.contains_key(&target) {
            return Err(ConnectionError::InvalidMessage(format!(
                "ACP channel already open for {}/{}",
                target.agent_id, target.agent_instance_id
            )));
        }
        inbound.insert(target.clone(), tx);
        drop(inbound);

        let outbound = self.outbound.clone();
        let inbound = Arc::clone(&self.inbound);
        let next_message_id = Arc::clone(&self.next_message_id);
        let mut shutdown = self.shutdown.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = rx.next() => {
                        let Some(Ok(message)) = message else {
                            break;
                        };
                        let message_id = next_message_id.fetch_add(1, Ordering::Relaxed);
                        let delivery = AcpMessage::new(
                            format!("acp-{message_id}"),
                            target.agent_id.clone(),
                            target.agent_instance_id.clone(),
                            message,
                        );
                        let Ok(params) = serde_json::to_value(delivery) else {
                            break;
                        };
                        let Ok(message) =
                            RawJsonRpcMessage::notification(ACP_METHOD.to_owned(), params)
                        else {
                            break;
                        };
                        if outbound.unbounded_send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    _ = shutdown.changed() => break,
                }
            }

            if let Ok(mut channels) = inbound.lock() {
                channels.remove(&target);
            }
        });

        Ok(peer)
    }

    #[tracing::instrument(
        name = "route_acp_message",
        level = "trace",
        skip(self, delivery),
        fields(
            message_id = %delivery.message_id,
            agent_id = %delivery.agent_id,
            agent_instance_id = %delivery.agent_instance_id,
        ),
        err(level = "trace")
    )]
    fn route(&self, delivery: AcpMessage) -> Result<(), ConnectionError> {
        let target = AgentTarget::new(delivery.agent_id, delivery.agent_instance_id);
        let sender = self
            .inbound
            .lock()
            .map_err(|_| ConnectionError::Closed)?
            .get(&target)
            .cloned()
            .ok_or_else(|| {
                ConnectionError::InvalidMessage(format!(
                    "no ACP channel for {}/{}",
                    target.agent_id, target.agent_instance_id
                ))
            })?;
        sender
            .unbounded_send(Ok(delivery.message))
            .map_err(|_| ConnectionError::Closed)
    }

    fn close(&self) {
        self.shutdown.send_replace(true);
        if let Ok(mut channels) = self.inbound.lock() {
            channels.clear();
        }
    }
}

async fn run_server<H>(
    mut inbound: futures::channel::mpsc::UnboundedReceiver<
        Result<RawJsonRpcMessage, agent_client_protocol::Error>,
    >,
    system_events: Arc<H>,
    pending: PendingCommands,
    acp: AcpRouter,
) where
    H: SystemEventHandler,
{
    while let Some(message) = inbound.next().await {
        let Ok(message) = message else {
            break;
        };
        let Ok(value) = serde_json::to_value(&message) else {
            continue;
        };

        if let Some(method) = value.get("method").and_then(Value::as_str) {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            match method {
                SYSTEM_EVENT_METHOD => {
                    if let Ok(event) = SystemEvent::parse_message(method, &params) {
                        let _ = system_events.handle(event).await;
                    }
                }
                ACP_METHOD => {
                    if let Ok(delivery) = AcpMessage::parse_message(method, &params) {
                        let _ = acp.route(delivery);
                    }
                }
                _ => {}
            }
            continue;
        }

        let Some(request_id) = value.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let sender = pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&request_id));
        let Some(sender) = sender else {
            continue;
        };
        let result = if let Some(result) = value.get("result") {
            CommandResult::from_value(COMMAND_METHOD, result.clone())
                .map_err(|error| ConnectionError::InvalidMessage(error.to_string()))
        } else if let Some(error) = value.get("error") {
            serde_json::from_value(error.clone())
                .map_err(|error| {
                    ConnectionError::InvalidMessage(format!(
                        "invalid command error response: {error}"
                    ))
                })
                .and_then(|error| Err(ConnectionError::CommandFailed(error)))
        } else {
            Err(ConnectionError::InvalidMessage(
                "command response contains neither result nor error".to_owned(),
            ))
        };
        let _ = sender.send(result);
    }

    if let Ok(mut commands) = pending.lock() {
        for (_, sender) in commands.drain() {
            let _ = sender.send(Err(ConnectionError::Closed));
        }
    }
    acp.close();
}

async fn run_runtime<H>(
    mut inbound: futures::channel::mpsc::UnboundedReceiver<
        Result<RawJsonRpcMessage, agent_client_protocol::Error>,
    >,
    outbound: LogicalSender,
    commands: Arc<H>,
    acp: AcpRouter,
) where
    H: CommandHandler,
{
    let mut commands_in_flight = tokio::task::JoinSet::new();
    loop {
        let message = tokio::select! {
            message = inbound.next() => message,
            _ = commands_in_flight.join_next(), if !commands_in_flight.is_empty() => continue,
        };
        let Some(message) = message else {
            break;
        };
        let Ok(message) = message else {
            break;
        };
        let request_id = match &message {
            RawJsonRpcMessage::Request(request) => Some(request.id.clone()),
            RawJsonRpcMessage::Notification(_) | RawJsonRpcMessage::Response(_) => None,
        };
        let Ok(value) = serde_json::to_value(&message) else {
            continue;
        };
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            continue;
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);

        match method {
            COMMAND_METHOD => {
                let Some(request_id) = request_id else {
                    continue;
                };
                let command = match Command::parse_message(method, &params) {
                    Ok(command) => command,
                    Err(error) => {
                        let error =
                            agent_client_protocol::Error::invalid_params().data(error.to_string());
                        let response = RawJsonRpcMessage::response(request_id, Err(error));
                        let _ = outbound.unbounded_send(Ok(response));
                        continue;
                    }
                };
                let commands = Arc::clone(&commands);
                let outbound = outbound.clone();
                commands_in_flight.spawn(async move {
                    let result = AssertUnwindSafe(async { commands.handle(command).await })
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| Err(agent_client_protocol::Error::internal_error()))
                        .and_then(|result| result.into_json(COMMAND_METHOD));
                    let response = RawJsonRpcMessage::response(request_id, result);
                    let _ = outbound.unbounded_send(Ok(response));
                });
            }
            ACP_METHOD => {
                if let Ok(delivery) = AcpMessage::parse_message(method, &params) {
                    let _ = acp.route(delivery);
                }
            }
            _ => {}
        }
    }
    acp.close();
}
