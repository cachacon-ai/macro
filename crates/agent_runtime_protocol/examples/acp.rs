//! Connect the official ACP Client and Agent through an Agent Runtime connection.
//!
//! Run with:
//!
//! ```text
//! cargo run -p agent_runtime_protocol --example acp
//! ```

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, InitializeResponse};
use agent_client_protocol::{Agent, Channel, Client};
use agent_runtime_protocol::connection::{AgentTarget, RuntimeConnection, ServerConnection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (service_channel, runtime_channel) = Channel::duplex();
    let service = ServerConnection::connect(service_channel, ());
    let runtime = RuntimeConnection::connect(runtime_channel, ());
    let target = AgentTarget::new("primary", "agent-instance-1");

    // These are official agent_client_protocol::Channel values. No adapter API or
    // string serialization is needed by either ACP component.
    let service_acp = service.acp(target.clone())?;
    let runtime_acp = runtime.acp(target)?;

    let agent = tokio::spawn(async move {
        Agent
            .builder()
            .on_receive_request(
                async move |request: InitializeRequest, responder, _connection| {
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(runtime_acp)
            .await
    });

    let response = Client
        .connect_with(service_acp, async |connection| {
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await
        })
        .await?;
    println!(
        "official ACP initialize completed with {:?}",
        response.protocol_version
    );

    agent.abort();
    let _ = agent.await;
    Ok(())
}
