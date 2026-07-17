use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use agent_client_protocol::{Channel, RawJsonRpcMessage};
use futures::StreamExt;
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::{Duration, timeout};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

use super::*;
use crate::schema::v0::{CommandName, SystemEventName};

#[derive(Clone, Default)]
struct TraceCapture {
    next_span_id: Arc<AtomicU64>,
    records: Arc<std::sync::Mutex<Vec<CapturedTrace>>>,
}

#[derive(Debug)]
struct CapturedTrace {
    level: tracing::Level,
    fields: Vec<(String, String)>,
}

#[derive(Default)]
struct FieldVisitor(Vec<(String, String)>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }
}

impl TraceCapture {
    fn capture(&self, metadata: &Metadata<'_>, fields: FieldVisitor) {
        self.records.lock().unwrap().push(CapturedTrace {
            level: *metadata.level(),
            fields: fields.0,
        });
    }
}

impl Subscriber for TraceCapture {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let mut fields = FieldVisitor::default();
        attributes.record(&mut fields);
        self.capture(attributes.metadata(), fields);
        Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = FieldVisitor::default();
        event.record(&mut fields);
        self.capture(event.metadata(), fields);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<SystemEvent>>>);

impl SystemEventHandler for Events {
    async fn handle(&self, event: SystemEvent) -> Result<(), agent_client_protocol::Error> {
        self.0.lock().await.push(event);
        Ok(())
    }
}

struct Commands;

impl CommandHandler for Commands {
    async fn handle(
        &self,
        command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        Ok(CommandResult::completed_with(json!({
            "handled": command.command_id,
        })))
    }
}

struct PendingCommand;

impl CommandHandler for PendingCommand {
    async fn handle(
        &self,
        _command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        std::future::pending().await
    }
}

struct FailingCommand;

impl CommandHandler for FailingCommand {
    async fn handle(
        &self,
        _command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        Err(
            agent_client_protocol::Error::new(-32042, "example command failure")
                .data(json!({ "retryable": false })),
        )
    }
}

struct PanickingCommand;

impl CommandHandler for PanickingCommand {
    async fn handle(
        &self,
        _command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        panic!("example command handler panic")
    }
}

fn connections() -> (ServerConnection, RuntimeConnection, Events) {
    let (server_channel, runtime_channel) = Channel::duplex();
    let events = Events::default();
    let server = ServerConnection::connect(server_channel, events.clone());
    let runtime = RuntimeConnection::connect(runtime_channel, Commands);
    (server, runtime, events)
}

#[tokio::test]
async fn unit_handlers_support_connections_that_only_use_acp() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let _server = ServerConnection::connect(server_channel, ());
    let _runtime = RuntimeConnection::connect(runtime_channel, ());
}

#[tokio::test]
async fn command_request_round_trips_with_its_result() {
    let (server, _runtime, _events) = connections();

    let result = timeout(
        Duration::from_secs(1),
        server.command(Command::new(
            "command-1",
            CommandName::Unknown("runtime/configure".to_owned()),
        )),
    )
    .await
    .expect("command should not hang")
    .expect("command should succeed");

    assert_eq!(
        result,
        CommandResult::completed_with(json!({ "handled": "command-1" }))
    );
}

#[tokio::test]
async fn cancelling_a_command_removes_its_pending_entry() {
    let (server_channel, _runtime_channel) = Channel::duplex();
    let server = ServerConnection::connect(server_channel, Events::default());
    let mut command = Box::pin(server.command(Command::new(
        "command-1",
        CommandName::Unknown("example/pending".to_owned()),
    )));

    assert!(futures::poll!(&mut command).is_pending());
    assert_eq!(server.pending.lock().unwrap().len(), 1);

    drop(command);

    assert!(server.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn command_error_preserves_jsonrpc_code_message_and_data() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let server = ServerConnection::connect(server_channel, Events::default());
    let _runtime = RuntimeConnection::connect(runtime_channel, FailingCommand);

    let error = timeout(
        Duration::from_secs(1),
        server.command(Command::new(
            "command-1",
            CommandName::Unknown("example/fail".to_owned()),
        )),
    )
    .await
    .expect("command should not hang")
    .expect_err("command should fail");

    let ConnectionError::CommandFailed(error) = error else {
        panic!("expected a structured command failure");
    };
    assert_eq!(i32::from(error.code), -32042);
    assert_eq!(error.message, "example command failure");
    assert_eq!(error.data, Some(json!({ "retryable": false })));
}

#[tokio::test]
async fn runtime_preserves_string_jsonrpc_request_ids() {
    let (mut service_channel, runtime_channel) = Channel::duplex();
    let _runtime = RuntimeConnection::connect(runtime_channel, Commands);
    let command = Command::new(
        "command-1",
        CommandName::Unknown("example/string-id".to_owned()),
    );
    let request = RawJsonRpcMessage::request(
        COMMAND_METHOD.to_owned(),
        serde_json::to_value(command).unwrap(),
        agent_client_protocol::schema::v1::RequestId::Str("request-one".to_owned()),
    )
    .unwrap();

    service_channel.tx.unbounded_send(Ok(request)).unwrap();
    let response = timeout(Duration::from_secs(1), service_channel.rx.next())
        .await
        .expect("string-id command should receive a response")
        .expect("runtime should keep the channel open")
        .expect("runtime should send a valid response");

    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "request-one",
            "result": {
                "status": "completed",
                "value": { "handled": "command-1" },
            },
        })
    );
}

#[tokio::test]
async fn malformed_command_params_receive_an_invalid_params_response() {
    let (mut service_channel, runtime_channel) = Channel::duplex();
    let _runtime = RuntimeConnection::connect(runtime_channel, Commands);
    let request = RawJsonRpcMessage::request(
        COMMAND_METHOD.to_owned(),
        json!({
            "commandId": 123,
            "name": "example/malformed",
        }),
        agent_client_protocol::schema::v1::RequestId::Str("request-one".to_owned()),
    )
    .unwrap();

    service_channel.tx.unbounded_send(Ok(request)).unwrap();
    let response = timeout(Duration::from_secs(1), service_channel.rx.next())
        .await
        .expect("malformed command should receive a response")
        .expect("runtime should keep the channel open")
        .expect("runtime should send a valid response");

    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["id"], "request-one");
    assert_eq!(response["error"]["code"], -32602);
}

#[tokio::test]
async fn panicking_command_handler_returns_an_internal_error() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let server = ServerConnection::connect(server_channel, Events::default());
    let _runtime = RuntimeConnection::connect(runtime_channel, PanickingCommand);

    let error = timeout(
        Duration::from_secs(1),
        server.command(Command::new(
            "command-1",
            CommandName::Unknown("example/panic".to_owned()),
        )),
    )
    .await
    .expect("panicking command handler should receive a response")
    .expect_err("panicking command handler should return an error");

    let ConnectionError::CommandFailed(error) = error else {
        panic!("expected a structured command failure");
    };
    assert_eq!(i32::from(error.code), -32603);
    assert_eq!(error.message, "Internal error");
}

#[test]
fn unroutable_acp_delivery_emits_a_trace_with_its_target() {
    let subscriber = TraceCapture::default();
    let records = Arc::clone(&subscriber.records);
    let (channel, _peer) = Channel::duplex();
    let router = AcpRouter::new(channel.tx);
    let delivery = AcpMessage::new(
        "message-1",
        "missing-agent",
        "missing-instance",
        RawJsonRpcMessage::notification("session/cancel".to_owned(), json!({})).unwrap(),
    );
    let result = tracing::subscriber::with_default(subscriber, || router.route(delivery));
    assert!(result.is_err());
    let traces = std::mem::take(&mut *records.lock().unwrap());

    assert!(
        traces.iter().any(|trace| {
            trace
                .fields
                .iter()
                .any(|(name, value)| name == "agent_id" && value.contains("missing-agent"))
                && trace.fields.iter().any(|(name, value)| {
                    name == "agent_instance_id" && value.contains("missing-instance")
                })
        }),
        "{traces:?}"
    );
    assert!(
        traces
            .iter()
            .any(|trace| trace.fields.iter().any(|(name, _)| name == "error")),
        "{traces:?}"
    );
    assert!(
        traces
            .iter()
            .any(|trace| trace.level == tracing::Level::TRACE),
        "{traces:?}"
    );
    assert!(
        traces.iter().all(
            |trace| trace.level != tracing::Level::WARN && trace.level != tracing::Level::ERROR
        ),
        "{traces:?}"
    );
}

#[tokio::test]
async fn concurrent_commands_are_correlated_by_jsonrpc_id() {
    let (server, _runtime, _events) = connections();

    let (first, second) = tokio::join!(
        server.command(Command::new(
            "command-1",
            CommandName::Unknown("runtime/first".to_owned()),
        )),
        server.command(Command::new(
            "command-2",
            CommandName::Unknown("runtime/second".to_owned()),
        )),
    );

    assert_eq!(
        first.expect("first command should succeed"),
        CommandResult::completed_with(json!({ "handled": "command-1" }))
    );
    assert_eq!(
        second.expect("second command should succeed"),
        CommandResult::completed_with(json!({ "handled": "command-2" }))
    );
}

#[tokio::test]
async fn dropping_runtime_fails_a_pending_command() {
    let (server_channel, runtime_channel) = Channel::duplex();
    let server = ServerConnection::connect(server_channel, Events::default());
    let runtime = RuntimeConnection::connect(runtime_channel, PendingCommand);
    let command = server.command(Command::new(
        "command-1",
        CommandName::Unknown("runtime/wait".to_owned()),
    ));
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
async fn closing_an_acp_router_before_opening_a_channel_is_persistent() {
    let (channel, _peer) = Channel::duplex();
    let router = AcpRouter::new(channel.tx);

    router.close();

    let error = router
        .open(AgentTarget::new("agent", "instance"))
        .expect_err("closed router must reject new ACP channels");
    assert!(matches!(error, ConnectionError::Closed));
}

#[tokio::test]
async fn system_event_is_dispatched_without_a_response() {
    let (_server, runtime, events) = connections();
    runtime
        .system_event(SystemEvent::new(
            "event-1",
            1,
            SystemEventName::Unknown("runtime/ready".to_owned()),
            "2026-07-17T00:00:00Z",
        ))
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

    assert_eq!(events.0.lock().await[0].event_id, "event-1");
}

#[tokio::test]
async fn acp_channels_carry_raw_sdk_messages_in_both_directions() {
    let (server, runtime, _events) = connections();
    let target = AgentTarget::new("primary", "agent-instance-1");
    let mut server_acp = server.acp(target.clone()).expect("server ACP channel");
    let mut runtime_acp = runtime.acp(target).expect("runtime ACP channel");

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
async fn acp_channels_are_isolated_by_agent_and_instance() {
    let (server, runtime, _events) = connections();
    let first = AgentTarget::new("first", "instance-1");
    let second = AgentTarget::new("second", "instance-1");
    let first_server = server.acp(first.clone()).unwrap();
    let second_server = server.acp(second.clone()).unwrap();
    let mut first_runtime = runtime.acp(first).unwrap();
    let mut second_runtime = runtime.acp(second).unwrap();

    first_server
        .tx
        .unbounded_send(Ok(RawJsonRpcMessage::notification(
            "first/message".to_owned(),
            json!({}),
        )
        .unwrap()))
        .unwrap();
    second_server
        .tx
        .unbounded_send(Ok(RawJsonRpcMessage::notification(
            "second/message".to_owned(),
            json!({}),
        )
        .unwrap()))
        .unwrap();

    let first = first_runtime.rx.next().await.unwrap().unwrap();
    let second = second_runtime.rx.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap()["method"],
        "first/message"
    );
    assert_eq!(
        serde_json::to_value(second).unwrap()["method"],
        "second/message"
    );
}

#[tokio::test]
async fn dropping_a_connection_closes_its_acp_channels() {
    let (server, runtime, _events) = connections();
    let target = AgentTarget::new("primary", "instance-1");
    let mut server_acp = server.acp(target.clone()).unwrap();
    let _runtime_acp = runtime.acp(target).unwrap();

    drop(runtime);

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

    let (server, runtime, _events) = connections();
    let target = AgentTarget::new("primary", "agent-instance-1");
    let server_acp = server.acp(target.clone()).expect("server ACP channel");
    let runtime_acp = runtime.acp(target).expect("runtime ACP channel");

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
