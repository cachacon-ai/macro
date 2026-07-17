//! Interactive end-to-end example with a mock container and Claude Code.
//!
//! The process contains two sides:
//!
//! - the service accepts a runtime-initiated WebSocket, prints protocol events,
//!   and provides an interactive prompt on stdin;
//! - a Tokio task stands in for a container, connects its runtime to the service,
//!   and starts Zed's Claude Code ACP adapter with `npx`.
//!
//! Prerequisites:
//!
//! - Node.js and `npx` must be installed;
//! - `ANTHROPIC_API_KEY` must be available to the process;
//! - the first run may take longer while `npx` downloads the adapter.
//!
//! Run with:
//!
//! ```text
//! cargo run -p agent_runtime_protocol --example mock_container
//! ```
//!
//! Enter prompts at `you>`. Enter `/quit` or send EOF to stop.

use std::error::Error;
use std::io::{self as stdio, Write};
use std::sync::Arc;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SessionNotification, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, Channel, Client, ConnectTo, ConnectionTo};
use agent_runtime_protocol::connection::{AgentTarget, RuntimeConnection, ServerConnection};
use agent_runtime_protocol::transport::jsonrpsee::{ServerTransport, connect_runtime};
use jsonrpsee::server::Server;
use jsonrpsee::ws_client::WsClientBuilder;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Notify;

#[cfg(test)]
#[path = "mock_container/test.rs"]
mod test;

type AnyError = Box<dyn Error + Send + Sync>;

const CONNECTION_ID: &str = "mock-container-connection";
const AGENT_ID: &str = "claude";
const AGENT_INSTANCE_ID: &str = "claude-instance-1";
const ZED_CLAUDE_CODE_ACP: &str = "@zed-industries/claude-code-acp@0.16.2";

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    let carrier = ServerTransport::new();
    let websocket = Server::builder().build("127.0.0.1:0").await?;
    let address = websocket.local_addr()?;
    let websocket_handle = websocket.start(carrier.rpc_module()?);
    println!("service listening on ws://{address}");

    let acp_ready = Arc::new(Notify::new());
    let mut container = tokio::spawn(run_mock_container(
        format!("ws://{address}"),
        Arc::clone(&acp_ready),
    ));
    let incoming = accept_runtime(&carrier, &mut container).await?;
    println!(
        "service accepted runtime connection {}",
        incoming.connection_id()
    );

    let service = ServerConnection::connect(incoming.into_channel(), ());
    wait_for_acp(&acp_ready, &mut container).await?;

    let target = AgentTarget::new(AGENT_ID, AGENT_INSTANCE_ID);
    let acp = service.acp(target)?;
    let chat = run_interactive_client(acp);
    tokio::pin!(chat);

    let result = tokio::select! {
        result = &mut chat => {
            container.abort();
            let _ = container.await;
            result
        }
        container_result = &mut container => {
            match container_result {
                Ok(Ok(())) => Err("mock container exited unexpectedly".into()),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(Box::new(error) as AnyError),
            }
        }
    };

    drop(service);
    websocket_handle.stop()?;
    websocket_handle.stopped().await;
    result
}

async fn accept_runtime(
    carrier: &ServerTransport,
    container: &mut tokio::task::JoinHandle<Result<(), AnyError>>,
) -> Result<agent_runtime_protocol::transport::jsonrpsee::IncomingConnection, AnyError> {
    tokio::select! {
        incoming = carrier.accept() => incoming
            .ok_or_else(|| "mock container disconnected before subscribing".into()),
        result = &mut *container => Err(container_startup_error(result)),
    }
}

async fn wait_for_acp(
    ready: &Notify,
    container: &mut tokio::task::JoinHandle<Result<(), AnyError>>,
) -> Result<(), AnyError> {
    tokio::select! {
        () = ready.notified() => Ok(()),
        result = &mut *container => Err(container_startup_error(result)),
    }
}

fn container_startup_error(
    result: Result<Result<(), AnyError>, tokio::task::JoinError>,
) -> AnyError {
    match result {
        Ok(Ok(())) => "mock container exited before startup completed".into(),
        Ok(Err(error)) => error,
        Err(error) => Box::new(error),
    }
}

fn cancelled_permission_response() -> RequestPermissionResponse {
    RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
}

async fn run_mock_container(websocket_url: String, acp_ready: Arc<Notify>) -> Result<(), AnyError> {
    let client = Arc::new(WsClientBuilder::default().build(websocket_url).await?);
    let channel = connect_runtime(client, CONNECTION_ID).await?;
    let runtime = RuntimeConnection::connect(channel, ());

    let target = AgentTarget::new(AGENT_ID, AGENT_INSTANCE_ID);
    let acp = runtime.acp(target)?;
    acp_ready.notify_one();

    // Construct the command explicitly because the SDK's convenience helper now
    // points at the renamed package. This example intentionally runs Zed's wrapper.
    let claude_code = AcpAgent::from_args(["npx", "-y", ZED_CLAUDE_CODE_ACP])?;
    ConnectTo::<Client>::connect_to(claude_code, acp)
        .await
        .map_err(Into::into)
}

async fn run_interactive_client(acp: Channel) -> Result<(), AnyError> {
    Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _connection| {
                println!("\n[ACP session event] {:#?}", notification.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                println!("\n[ACP permission request] {request:#?}");
                println!("[ACP permission cancelled: this example does not prompt for approval]");
                responder.respond(cancelled_permission_response())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(acp, |connection: ConnectionTo<Agent>| async move {
            let initialized = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            println!("[ACP initialized] {initialized:#?}");

            let cwd = std::env::current_dir().map_err(|error| {
                agent_client_protocol::Error::internal_error()
                    .data(format!("cannot determine working directory: {error}"))
            })?;
            let session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            println!("[ACP session started] {}", session.session_id);
            println!("type a prompt, or /quit to exit");

            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            loop {
                print!("you> ");
                stdio::stdout()
                    .flush()
                    .map_err(agent_client_protocol::util::internal_error)?;
                let Some(line) = lines
                    .next_line()
                    .await
                    .map_err(agent_client_protocol::util::internal_error)?
                else {
                    break;
                };
                let line = line.trim();
                if line == "/quit" {
                    break;
                }
                if line.is_empty() {
                    continue;
                }

                let response = connection
                    .send_request(PromptRequest::new(
                        session.session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new(line))],
                    ))
                    .block_task()
                    .await?;
                println!("\n[ACP prompt completed] {response:#?}");
            }

            Ok(())
        })
        .await?;
    Ok(())
}
