use jsonrpsee::server::Server;
use jsonrpsee::ws_client::WsClientBuilder;
use serde_json::{Value, json};
use tokio::time::{Duration, timeout};

use super::*;
use crate::domain::connection::{
    CommandHandler, RuntimeConnection, ServerConnection, SystemEventHandler,
};
use crate::domain::schema::v0::{
    Command, CommandRequest, CommandResult, SystemEvent, ToRuntimeMessage, ToServerMessage,
};

fn event() -> ToServerMessage {
    ToServerMessage::Event {
        event: SystemEvent::Unknown("runtime/ready".to_owned()),
    }
}

fn command() -> ToRuntimeMessage {
    ToRuntimeMessage::Command(CommandRequest::new(
        "command-1",
        Command::Unknown("runtime/configure".to_owned()),
    ))
}

#[test]
fn carrier_names_are_unprefixed_and_exact() {
    assert_eq!(SEND_METHOD, "send");
    assert_eq!(SUBSCRIBE_METHOD, "subscribe");
    assert_eq!(MESSAGE_METHOD, "message");
    assert_eq!(UNSUBSCRIBE_METHOD, "unsubscribe");
}

#[tokio::test]
async fn rpc_module_routes_both_directions_through_the_subscription() {
    let transport: ServerTransport<ToRuntimeMessage, ToServerMessage> = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let (_, mut subscription) = module
        .raw_json_request(r#"{"jsonrpc":"2.0","method":"subscribe","id":1}"#, 1)
        .await
        .expect("subscription should open");
    let server = timeout(Duration::from_secs(1), transport.accept())
        .await
        .expect("accept should not hang")
        .expect("connection should be accepted");

    server.tx.send(command()).unwrap();
    let physical = timeout(Duration::from_secs(1), subscription.recv())
        .await
        .expect("subscription message should not hang")
        .expect("subscription should remain open");
    let physical: Value = serde_json::from_str(physical.get()).unwrap();
    assert_eq!(physical["method"], "message");
    assert_eq!(physical["params"]["result"]["message"]["type"], "command");
    assert_eq!(
        physical["params"]["result"]["message"]["commandId"],
        "command-1"
    );

    let mut server = server;
    let runtime_message = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "send",
        "params": { "message": event() },
    }))
    .unwrap();
    module
        .raw_json_request(&runtime_message, 1)
        .await
        .expect("runtime message should dispatch");
    let delivered = timeout(Duration::from_secs(1), server.rx.recv())
        .await
        .expect("runtime message should not hang")
        .expect("logical channel should remain open");
    assert_eq!(
        serde_json::to_value(delivered).unwrap(),
        serde_json::to_value(event()).unwrap()
    );
}

#[tokio::test]
async fn send_rejects_a_connection_with_no_active_subscription() {
    let transport: ServerTransport<ToRuntimeMessage, ToServerMessage> = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "send",
        "params": { "message": event() },
    }))
    .unwrap();

    let (response, _) = module
        .raw_json_request(&request, 1)
        .await
        .expect("request should produce a JSON-RPC error");
    let response: Value = serde_json::from_str(response.get()).unwrap();
    assert_eq!(response["error"]["code"], -32_004);
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_initiates_a_real_websocket_and_exchanges_logical_messages() {
    let transport: ServerTransport<ToRuntimeMessage, ToServerMessage> = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let server = Server::builder()
        .build("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = server.local_addr().expect("server address");
    let handle = server.start(module);
    let client = Arc::new(
        WsClientBuilder::default()
            .build(format!("ws://{address}"))
            .await
            .expect("runtime should connect"),
    );

    let mut runtime = connect_runtime::<_, ToServerMessage, ToRuntimeMessage>(Arc::clone(&client));
    let mut server = timeout(Duration::from_secs(1), transport.accept())
        .await
        .expect("server accept should not hang")
        .expect("server should accept runtime");

    runtime.tx.send(event()).unwrap();
    let delivered = timeout(Duration::from_secs(1), server.rx.recv())
        .await
        .expect("runtime-to-server should not hang")
        .expect("server logical channel should remain open");
    assert_eq!(
        serde_json::to_value(delivered).unwrap(),
        serde_json::to_value(event()).unwrap()
    );

    server.tx.send(command()).unwrap();
    let delivered = timeout(Duration::from_secs(1), runtime.rx.recv())
        .await
        .expect("server-to-runtime should not hang")
        .expect("runtime logical channel should remain open");
    let ToRuntimeMessage::Command(CommandRequest { command_id, .. }) = delivered else {
        panic!("expected a command");
    };
    assert_eq!(command_id, "command-1");

    handle.stop().expect("server should stop");
    handle.stopped().await;
}

struct Commands;

impl CommandHandler for Commands {
    async fn handle(&self, command: Command) -> CommandResult {
        let Command::Unknown(name) = command;
        CommandResult::completed_with(json!({ "handled": name }))
    }
}

struct Events(tokio::sync::mpsc::UnboundedSender<SystemEvent>);

impl SystemEventHandler for Events {
    async fn handle(&self, event: SystemEvent) {
        self.0
            .send(event)
            .expect("event receiver should remain open");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn role_connections_run_over_a_runtime_initiated_websocket() {
    let transport: ServerTransport<ToRuntimeMessage, ToServerMessage> = ServerTransport::new();
    let server = Server::builder()
        .build("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = server.local_addr().unwrap();
    let handle = server.start(transport.rpc_module().unwrap());
    let client = Arc::new(
        WsClientBuilder::default()
            .build(format!("ws://{address}"))
            .await
            .expect("runtime should connect"),
    );
    let runtime_channel = connect_runtime::<_, ToServerMessage, ToRuntimeMessage>(client);
    let server_channel = transport.accept().await.unwrap();
    let (event_sender, mut event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let (server_connection, _server_acp) =
        ServerConnection::connect(server_channel, Events(event_sender));
    let (runtime_connection, _runtime_acp) = RuntimeConnection::connect(runtime_channel, Commands);

    runtime_connection
        .system_event(SystemEvent::Unknown("runtime/ready".to_owned()))
        .unwrap();
    let received = timeout(Duration::from_secs(1), event_receiver.recv())
        .await
        .expect("event should not hang")
        .expect("event channel should remain open");
    assert_eq!(received, SystemEvent::Unknown("runtime/ready".to_owned()));

    let result = timeout(
        Duration::from_secs(1),
        server_connection.command(Command::Unknown("runtime/configure".to_owned())),
    )
    .await
    .expect("command should not hang")
    .expect("command should succeed");
    assert_eq!(
        result,
        CommandResult::completed_with(json!({ "handled": "runtime/configure" }))
    );

    handle.stop().unwrap();
    handle.stopped().await;
}
