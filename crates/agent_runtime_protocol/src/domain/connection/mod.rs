//! Role-oriented logical protocol connections.
//!
//! A connection hosts exactly one agent execution, so there is no routing
//! table: the single [`agent_client_protocol::Channel`] handed back by
//! [`ServerConnection::connect`] and [`RuntimeConnection::connect`] carries
//! all of this connection's ACP traffic.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::Channel as AcpChannel;
use futures::FutureExt;
use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::domain::channel::Channel;
use crate::domain::schema::v0::{
    AcpMessage, Command, CommandOutcome, CommandRequest, CommandResult, SystemEvent,
    ToRuntimeMessage, ToServerMessage,
};

#[cfg(test)]
mod test;

/// The logical channel carried by an Agent Service's side of a connection.
pub type ServerChannel = Channel<ToRuntimeMessage, ToServerMessage>;
/// The logical channel carried by an Agent Runtime's side of a connection.
pub type RuntimeChannel = Channel<ToServerMessage, ToRuntimeMessage>;

type CommandReply = oneshot::Sender<Result<CommandResult, ConnectionError>>;
type PendingCommands = Arc<Mutex<HashMap<String, CommandReply>>>;

struct PendingRequestGuard {
    command_id: String,
    pending: PendingCommands,
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.command_id);
        }
    }
}

/// Handles a command delivered to an Agent Runtime.
///
/// Use `()` when the connection only carries ACP. It fails any command as
/// unrecognized.
pub trait CommandHandler: Send + Sync + 'static {
    /// Execute the command and return its result.
    fn handle(&self, command: Command) -> impl Future<Output = CommandResult> + Send;
}

impl CommandHandler for () {
    fn handle(&self, _command: Command) -> impl Future<Output = CommandResult> + Send {
        std::future::ready(CommandResult::failed("method not found"))
    }
}

/// Handles a system event delivered to an Agent Service.
///
/// Use `()` when the connection only carries ACP. It ignores any system event.
pub trait SystemEventHandler: Send + Sync + 'static {
    /// Observe a runtime or agent state transition.
    fn handle(&self, event: SystemEvent) -> impl Future<Output = ()> + Send;
}

impl SystemEventHandler for () {
    fn handle(&self, _event: SystemEvent) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

/// A failure while using a logical protocol connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    /// The logical connection has closed.
    #[error("connection closed")]
    Closed,
}

/// Agent Service-side access to one logical runtime connection.
pub struct ServerConnection {
    outbound: UnboundedSender<ToRuntimeMessage>,
    pending: PendingCommands,
    next_command_id: AtomicU64,
    driver: tokio::task::AbortHandle,
}

impl ServerConnection {
    /// Attach the service role to a logical message channel.
    ///
    /// Returns the connection handle alongside an official ACP
    /// [`AcpChannel`] for the single agent execution this connection hosts.
    #[must_use]
    pub fn connect<H>(channel: ServerChannel, system_events: H) -> (Self, AcpChannel)
    where
        H: SystemEventHandler,
    {
        let Channel {
            tx: outbound,
            rx: inbound,
        } = channel;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (acp, acp_driver) = AcpChannel::duplex();
        let driver = tokio::spawn(run_server(
            inbound,
            outbound.clone(),
            Arc::new(system_events),
            Arc::clone(&pending),
            acp_driver,
        ))
        .abort_handle();

        (
            Self {
                outbound,
                pending,
                next_command_id: AtomicU64::new(1),
                driver,
            },
            acp,
        )
    }

    /// Send a correlated command request and wait for its result.
    pub async fn command(&self, command: Command) -> Result<CommandResult, ConnectionError> {
        let command_id = format!(
            "cmd-{}",
            self.next_command_id.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| ConnectionError::Closed)?
            .insert(command_id.clone(), sender);
        let _pending_request = PendingRequestGuard {
            command_id: command_id.clone(),
            pending: Arc::clone(&self.pending),
        };

        if self
            .outbound
            .send(ToRuntimeMessage::Command(CommandRequest::new(
                command_id, command,
            )))
            .is_err()
        {
            return Err(ConnectionError::Closed);
        }

        receiver.await.unwrap_or(Err(ConnectionError::Closed))
    }
}

impl Drop for ServerConnection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Agent Runtime-side access to one logical service connection.
pub struct RuntimeConnection {
    outbound: UnboundedSender<ToServerMessage>,
    driver: tokio::task::AbortHandle,
}

impl RuntimeConnection {
    /// Attach the runtime role to a logical message channel.
    ///
    /// Returns the connection handle alongside an official ACP
    /// [`AcpChannel`] for the single agent execution this connection hosts.
    #[must_use]
    pub fn connect<H>(channel: RuntimeChannel, commands: H) -> (Self, AcpChannel)
    where
        H: CommandHandler,
    {
        let Channel {
            tx: outbound,
            rx: inbound,
        } = channel;
        let (acp, acp_driver) = AcpChannel::duplex();
        let driver = tokio::spawn(run_runtime(
            inbound,
            outbound.clone(),
            Arc::new(commands),
            acp_driver,
        ))
        .abort_handle();
        (Self { outbound, driver }, acp)
    }

    /// Send a system event notification to the Agent Service.
    pub fn system_event(&self, event: SystemEvent) -> Result<(), ConnectionError> {
        self.outbound
            .send(ToServerMessage::Event { event })
            .map_err(|_| ConnectionError::Closed)
    }
}

impl Drop for RuntimeConnection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

async fn run_server<H>(
    mut inbound: tokio::sync::mpsc::UnboundedReceiver<ToServerMessage>,
    outbound: UnboundedSender<ToRuntimeMessage>,
    system_events: Arc<H>,
    pending: PendingCommands,
    mut acp: AcpChannel,
) where
    H: SystemEventHandler,
{
    // Whether the caller's ACP channel is still open. A caller that only
    // wants commands/events is free to drop its ACP channel immediately;
    // that must not tear down command/event handling, so once the ACP side
    // closes we simply stop selecting on it instead of breaking the loop.
    let mut acp_open = true;
    loop {
        tokio::select! {
            message = inbound.recv() => {
                let Some(message) = message else {
                    break;
                };
                match message {
                    ToServerMessage::Event { event } => {
                        system_events.handle(event).await;
                    }
                    ToServerMessage::Acp(AcpMessage(raw)) => {
                        if acp_open && acp.tx.unbounded_send(Ok(raw)).is_err() {
                            acp_open = false;
                        }
                    }
                    ToServerMessage::CommandResult(CommandOutcome { command_id, result }) => {
                        let sender = pending
                            .lock()
                            .ok()
                            .and_then(|mut pending| pending.remove(&command_id));
                        if let Some(sender) = sender {
                            let _ = sender.send(Ok(result));
                        }
                    }
                }
            }
            message = acp.rx.next(), if acp_open => {
                match message {
                    Some(Ok(raw)) => {
                        if outbound.send(ToRuntimeMessage::Acp(AcpMessage(raw))).is_err() {
                            break;
                        }
                    }
                    _ => acp_open = false,
                }
            }
        }
    }

    if let Ok(mut commands) = pending.lock() {
        for (_, sender) in commands.drain() {
            let _ = sender.send(Err(ConnectionError::Closed));
        }
    }
}

async fn run_runtime<H>(
    mut inbound: tokio::sync::mpsc::UnboundedReceiver<ToRuntimeMessage>,
    outbound: UnboundedSender<ToServerMessage>,
    commands: Arc<H>,
    mut acp: AcpChannel,
) where
    H: CommandHandler,
{
    let mut commands_in_flight = tokio::task::JoinSet::new();
    // See the matching comment in `run_server`: dropping the ACP channel must
    // not tear down command handling.
    let mut acp_open = true;
    loop {
        tokio::select! {
            message = inbound.recv() => {
                let Some(message) = message else {
                    break;
                };
                match message {
                    ToRuntimeMessage::Acp(AcpMessage(raw)) => {
                        if acp_open && acp.tx.unbounded_send(Ok(raw)).is_err() {
                            acp_open = false;
                        }
                    }
                    ToRuntimeMessage::Command(CommandRequest { command_id, command }) => {
                        let commands = Arc::clone(&commands);
                        let outbound = outbound.clone();
                        commands_in_flight.spawn(async move {
                            let result = AssertUnwindSafe(async { commands.handle(command).await })
                                .catch_unwind()
                                .await
                                .unwrap_or_else(|_| {
                                    CommandResult::failed("command handler panicked")
                                });
                            let _ = outbound.send(ToServerMessage::CommandResult(
                                CommandOutcome::new(command_id, result),
                            ));
                        });
                    }
                }
            }
            message = acp.rx.next(), if acp_open => {
                match message {
                    Some(Ok(raw)) => {
                        if outbound.send(ToServerMessage::Acp(AcpMessage(raw))).is_err() {
                            break;
                        }
                    }
                    _ => acp_open = false,
                }
            }
            _ = commands_in_flight.join_next(), if !commands_in_flight.is_empty() => {}
        }
    }
}
