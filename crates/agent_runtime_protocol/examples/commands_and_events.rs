//! Exchange an application-defined command and system event in memory.
//!
//! Run with:
//!
//! ```text
//! cargo run -p agent_runtime_protocol --example commands_and_events
//! ```

use agent_runtime_protocol::domain::channel::Channel;
use agent_runtime_protocol::domain::connection::{
    CommandHandler, RuntimeConnection, ServerConnection, SystemEventHandler,
};
use agent_runtime_protocol::domain::schema::v0::{Command, CommandResult, SystemEvent};
use serde_json::json;
use tokio::sync::mpsc;

struct RuntimeCommands;

impl CommandHandler for RuntimeCommands {
    async fn handle(&self, command: Command) -> CommandResult {
        let Command::Unknown(name) = &command else {
            return CommandResult::failed("unrecognized command");
        };
        println!("runtime received command: {name}");
        CommandResult::completed_with(json!({ "handled": name }))
    }
}

struct ServiceEvents(mpsc::UnboundedSender<SystemEvent>);

impl SystemEventHandler for ServiceEvents {
    async fn handle(&self, event: SystemEvent) {
        let _ = self.0.send(event);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (service_channel, runtime_channel) = Channel::duplex();
    let (event_sender, mut events) = mpsc::unbounded_channel();

    let (service, _service_acp) =
        ServerConnection::connect(service_channel, ServiceEvents(event_sender));
    let (runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, RuntimeCommands);

    runtime.system_event(SystemEvent::Unknown("example/connected".to_owned()))?;
    let event = events.recv().await.ok_or("event stream closed")?;
    let SystemEvent::Unknown(name) = &event else {
        unreachable!("SystemEvent has only one variant today");
    };
    println!("service received event: {name}");

    let result = service
        .command(Command::Unknown("example/echo".to_owned()))
        .await?;
    println!("service received command result: {result:?}");

    Ok(())
}
