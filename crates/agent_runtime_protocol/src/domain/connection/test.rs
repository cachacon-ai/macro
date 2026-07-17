use std::sync::Arc;

use agent_client_protocol::Channel as AcpChannel;
use agent_client_protocol::RawJsonRpcMessage;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::{Duration, timeout};

use super::*;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<SystemEvent>>>);

impl SystemEventHandler for Events {
    async fn handle(&self, event: SystemEvent) {
        self.0.lock().await.push(event);
    }
}

struct Commands;

impl CommandHandler for Commands {
    async fn handle(&self, command: Command) -> CommandResult {
        let Command::Unknown(name) = command;
        CommandResult::completed_with(json!({ "handled": name }))
    }
}

struct PendingCommand;

impl CommandHandler for PendingCommand {
    async fn handle(&self, _command: Command) -> CommandResult {
        std::future::pending().await
    }
}

struct FailingCommand;

impl CommandHandler for FailingCommand {
    async fn handle(&self, _command: Command) -> CommandResult {
        CommandResult::failed("example command failure")
    }
}

struct PanickingCommand;

impl CommandHandler for PanickingCommand {
    async fn handle(&self, _command: Command) -> CommandResult {
        panic!("example command handler panic")
    }
}

/// Connect a server/runtime pair, discarding their ACP channels.
///
/// Safe as long as the test never pushes ACP traffic: the driver only treats
/// a closed ACP channel as fatal once it actually tries to use it.
fn connections() -> (ServerConnection, RuntimeConnection, Events) {
    let (server_channel, runtime_channel) = Channel::duplex();
    let events = Events::default();
    let (server, _server_acp) = ServerConnection::connect(server_channel, events.clone());
    let (runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, Commands);
    (server, runtime, events)
}

/// Connect a server/runtime pair, keeping both ACP channels.
fn connections_with_acp() -> (
    ServerConnection,
    AcpChannel,
    RuntimeConnection,
    AcpChannel,
    Events,
) {
    let (server_channel, runtime_channel) = Channel::duplex();
    let events = Events::default();
    let (server, server_acp) = ServerConnection::connect(server_channel, events.clone());
    let (runtime, runtime_acp) = RuntimeConnection::connect(runtime_channel, Commands);
    (server, server_acp, runtime, runtime_acp, events)
}

#[tokio::test]
async fn unit_handlers_support_connections_that_only_use_acp() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let (_server, _server_acp) = ServerConnection::connect(server_channel, ());
    let (_runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, ());
}

#[tokio::test]
async fn command_request_round_trips_with_its_result() {
    let (server, _runtime, _events) = connections();

    let result = timeout(
        Duration::from_secs(1),
        server.command(Command::Unknown("runtime/configure".to_owned())),
    )
    .await
    .expect("command should not hang")
    .expect("command should succeed");

    assert_eq!(
        result,
        CommandResult::completed_with(json!({ "handled": "runtime/configure" }))
    );
}

#[tokio::test]
async fn cancelling_a_command_removes_its_pending_entry() {
    let (server_channel, _runtime_channel) = Channel::duplex();
    let (server, _server_acp) = ServerConnection::connect(server_channel, Events::default());
    let mut command = Box::pin(server.command(Command::Unknown("example/pending".to_owned())));

    assert!(futures::poll!(&mut command).is_pending());
    assert_eq!(server.pending.lock().unwrap().len(), 1);

    drop(command);

    assert!(server.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failing_command_handler_returns_a_failed_outcome() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let (server, _server_acp) = ServerConnection::connect(server_channel, Events::default());
    let (_runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, FailingCommand);

    let result = timeout(
        Duration::from_secs(1),
        server.command(Command::Unknown("example/fail".to_owned())),
    )
    .await
    .expect("command should not hang")
    .expect("connection should stay open");

    assert_eq!(result, CommandResult::failed("example command failure"));
}

#[tokio::test]
async fn panicking_command_handler_returns_a_failed_outcome() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let (server, _server_acp) = ServerConnection::connect(server_channel, Events::default());
    let (_runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, PanickingCommand);

    let result = timeout(
        Duration::from_secs(1),
        server.command(Command::Unknown("example/panic".to_owned())),
    )
    .await
    .expect("panicking command handler should receive a response")
    .expect("connection should stay open");

    assert_eq!(result, CommandResult::failed("command handler panicked"));
}

#[tokio::test]
async fn concurrent_commands_are_correlated_by_command_id() {
    let (server, _runtime, _events) = connections();

    let (first, second) = tokio::join!(
        server.command(Command::Unknown("runtime/first".to_owned())),
        server.command(Command::Unknown("runtime/second".to_owned())),
    );

    assert_eq!(
        first.expect("first command should succeed"),
        CommandResult::completed_with(json!({ "handled": "runtime/first" }))
    );
    assert_eq!(
        second.expect("second command should succeed"),
        CommandResult::completed_with(json!({ "handled": "runtime/second" }))
    );
}

#[tokio::test]
async fn dropping_runtime_fails_a_pending_command() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let (server, _server_acp) = ServerConnection::connect(server_channel, Events::default());
    let (runtime, _runtime_acp) = RuntimeConnection::connect(runtime_channel, PendingCommand);
    let command = server.command(Command::Unknown("runtime/wait".to_owned()));
    tokio::pin!(command);

    tokio::select! {
        result = &mut command => panic!("command completed before shutdown: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    drop(runtime);

    let error = timeout(Duration::from_secs(1), command)
        .await
        .expect("pending command should not hang after shutdown")
        .expect_err("pending command should fail");
    assert!(matches!(error, ConnectionError::Closed));
}

#[tokio::test]
async fn system_event_is_dispatched_without_a_response() {
    let (_server, runtime, events) = connections();
    runtime
        .system_event(SystemEvent::Unknown("runtime/ready".to_owned()))
        .expect("event should send");

    timeout(Duration::from_secs(1), async {
        loop {
            if events.0.lock().await.len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("event should be dispatched");

    assert_eq!(
        events.0.lock().await[0],
        SystemEvent::Unknown("runtime/ready".to_owned())
    );
}

#[tokio::test]
async fn acp_channels_carry_raw_sdk_messages_in_both_directions() {
    let (_server, mut server_acp, _runtime, mut runtime_acp, _events) = connections_with_acp();

    let initialize = RawJsonRpcMessage::request(
        "initialize".to_owned(),
        json!({ "protocolVersion": 1 }),
        agent_client_protocol::schema::v1::RequestId::Number(1),
    )
    .unwrap();
    server_acp.tx.unbounded_send(Ok(initialize)).unwrap();

    let delivered = timeout(Duration::from_secs(1), runtime_acp.rx.next())
        .await
        .expect("ACP request should not hang")
        .expect("ACP channel should remain open")
        .expect("ACP request should be valid");
    assert_eq!(
        serde_json::to_value(delivered).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": 1 },
        })
    );

    runtime_acp
        .tx
        .unbounded_send(Ok(RawJsonRpcMessage::response(
            agent_client_protocol::schema::v1::RequestId::Number(1),
            Ok(json!({ "protocolVersion": 1 })),
        )))
        .unwrap();
    let response = timeout(Duration::from_secs(1), server_acp.rx.next())
        .await
        .expect("ACP response should not hang")
        .expect("ACP channel should remain open")
        .expect("ACP response should be valid");
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "protocolVersion": 1 },
        })
    );
}

#[tokio::test]
async fn dropping_a_connection_closes_its_acp_channel() {
    let (_server, mut server_acp, _runtime, _runtime_acp, _events) = connections_with_acp();

    drop(_runtime);

    let closed = timeout(Duration::from_secs(1), server_acp.rx.next())
        .await
        .expect("ACP channel should close promptly");
    assert!(closed.is_none());
}

#[tokio::test]
async fn official_acp_client_and_agent_connect_directly_to_exposed_channels() {
    use agent_client_protocol::schema::ProtocolVersion;
    use agent_client_protocol::schema::v1::{InitializeRequest, InitializeResponse};
    use agent_client_protocol::{Agent, Client};

    let (_server, server_acp, _runtime, runtime_acp, _events) = connections_with_acp();

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

    let response = timeout(
        Duration::from_secs(1),
        Client.connect_with(server_acp, async |connection| {
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await
        }),
    )
    .await
    .expect("official ACP initialize should not hang")
    .expect("official ACP initialize should succeed");
    assert_eq!(response.protocol_version, ProtocolVersion::V1);

    agent.abort();
    assert!(
        agent
            .await
            .expect_err("agent task should be cancelled")
            .is_cancelled(),
        "the long-lived agent driver should only be stopped explicitly"
    );
}
