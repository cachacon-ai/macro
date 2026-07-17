//! Exchange an application-defined command and system event in memory.
//!
//! Run with:
//!
//! ```text
//! cargo run -p agent_runtime_protocol --example commands_and_events
//! ```

use agent_client_protocol::Channel;
use agent_runtime_protocol::connection::{
    CommandHandler, RuntimeConnection, ServerConnection, SystemEventHandler,
};
use agent_runtime_protocol::schema::v0::{
    Command, CommandName, CommandResult, SystemEvent, SystemEventName,
};
use serde_json::json;
use tokio::sync::mpsc;

struct RuntimeCommands;

impl CommandHandler for RuntimeCommands {
    async fn handle(
        &self,
        command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        println!("runtime received command: {}", command.name);
        Ok(CommandResult::completed_with(json!({
            "handled": command.name,
        })))
    }
}

struct ServiceEvents(mpsc::UnboundedSender<SystemEvent>);

impl SystemEventHandler for ServiceEvents {
    async fn handle(&self, event: SystemEvent) -> Result<(), agent_client_protocol::Error> {
        self.0
            .send(event)
            .map_err(agent_client_protocol::util::internal_error)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (service_channel, runtime_channel) = Channel::duplex();
    let (event_sender, mut events) = mpsc::unbounded_channel();

    let service = ServerConnection::connect(service_channel, ServiceEvents(event_sender));
    let runtime = RuntimeConnection::connect(runtime_channel, RuntimeCommands);

    runtime.system_event(SystemEvent::new(
        "event-1",
        1,
        SystemEventName::Unknown("example/connected".to_owned()),
        "2026-07-17T00:00:00Z",
    ))?;
    let event = events.recv().await.ok_or("event stream closed")?;
    println!("service received event: {}", event.name);

    let result = service
        .command(Command::new(
            "command-1",
            CommandName::Unknown("example/echo".to_owned()),
        ))
        .await?;
    println!("service received command result: {result:?}");

    Ok(())
}
