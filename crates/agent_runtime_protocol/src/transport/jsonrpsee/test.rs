use agent_client_protocol::RawJsonRpcMessage;
use futures::StreamExt;
use jsonrpsee::server::Server;
use jsonrpsee::ws_client::WsClientBuilder;
use serde_json::{Value, json};
use tokio::time::{Duration, timeout};

use super::*;
use crate::connection::{CommandHandler, RuntimeConnection, ServerConnection, SystemEventHandler};
use crate::schema::v0::{Command, CommandName, CommandResult, SystemEvent, SystemEventName};

fn event() -> RawJsonRpcMessage {
    RawJsonRpcMessage::notification(
        "system_event".to_owned(),
        json!({
            "eventId": "event-1",
            "sequence": 1,
            "name": "runtime/ready",
            "occurredAt": "2026-07-17T00:00:00Z",
        }),
    )
    .unwrap()
}

fn command() -> RawJsonRpcMessage {
    RawJsonRpcMessage::request(
        "command".to_owned(),
        json!({
            "commandId": "command-1",
            "name": "runtime/configure",
        }),
        agent_client_protocol::schema::v1::RequestId::Number(1),
    )
    .unwrap()
}

#[test]
fn carrier_names_and_payloads_are_unprefixed_and_exact() {
    assert_eq!(SEND_METHOD, "send");
    assert_eq!(SUBSCRIBE_METHOD, "subscribe");
    assert_eq!(MESSAGE_METHOD, "message");
    assert_eq!(UNSUBSCRIBE_METHOD, "unsubscribe");
    assert_eq!(
        serde_json::to_value(Subscribe::new("connection-1")).unwrap(),
        json!({ "connectionId": "connection-1" })
    );
    assert_eq!(
        serde_json::to_value(RuntimeMessage::new("connection-1", event())).unwrap(),
        json!({
            "connectionId": "connection-1",
            "message": {
                "jsonrpc": "2.0",
                "method": "system_event",
                "params": {
                    "eventId": "event-1",
                    "sequence": 1,
                    "name": "runtime/ready",
                    "occurredAt": "2026-07-17T00:00:00Z",
                }
            }
        })
    );
}

#[tokio::test]
async fn rpc_module_routes_both_directions_through_the_subscription() {
    let transport = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let (_, mut subscription) = module
        .raw_json_request(
            r#"{"jsonrpc":"2.0","method":"subscribe","params":{"connectionId":"connection-1"},"id":1}"#,
            1,
        )
        .await
        .expect("subscription should open");
    let incoming = timeout(Duration::from_secs(1), transport.accept())
        .await
        .expect("accept should not hang")
        .expect("connection should be accepted");
    assert_eq!(incoming.connection_id(), "connection-1");
    let mut server = incoming.into_channel();

    server.tx.unbounded_send(Ok(command())).unwrap();
    let physical = timeout(Duration::from_secs(1), subscription.recv())
        .await
        .expect("subscription message should not hang")
        .expect("subscription should remain open");
    let physical: Value = serde_json::from_str(physical.get()).unwrap();
    assert_eq!(physical["method"], "message");
    assert_eq!(physical["params"]["result"]["message"]["method"], "command");
    assert_eq!(physical["params"]["result"]["message"]["id"], 1);

    let runtime_message = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "send",
        "params": RuntimeMessage::new("connection-1", event()),
    }))
    .unwrap();
    module
        .raw_json_request(&runtime_message, 1)
        .await
        .expect("runtime message should dispatch");
    let delivered = timeout(Duration::from_secs(1), server.rx.next())
        .await
        .expect("runtime message should not hang")
        .expect("logical channel should remain open")
        .expect("logical message should be valid");
    assert_eq!(
        serde_json::to_value(delivered).unwrap()["method"],
        "system_event"
    );
}

#[tokio::test]
async fn replacement_subscription_owns_the_connection_route() {
    let transport = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let (_, mut old_subscription) = module
        .raw_json_request(
            r#"{"jsonrpc":"2.0","method":"subscribe","params":{"connectionId":"connection-1"},"id":1}"#,
            1,
        )
        .await
        .expect("first subscription should open");
    let old_connection = transport.accept().await.expect("first connection");

    let (_, mut new_subscription) = module
        .raw_json_request(
            r#"{"jsonrpc":"2.0","method":"subscribe","params":{"connectionId":"connection-1"},"id":2}"#,
            2,
        )
        .await
        .expect("replacement subscription should open");
    let new_connection = transport.accept().await.expect("replacement connection");
    let new_server = new_connection.into_channel();
    new_server.tx.unbounded_send(Ok(command())).unwrap();

    let replacement_message = timeout(Duration::from_secs(1), new_subscription.recv())
        .await
        .expect("replacement should receive promptly")
        .expect("replacement should remain open");
    let replacement_message: Value =
        serde_json::from_str(replacement_message.get()).expect("message should be JSON");
    assert_eq!(
        replacement_message["params"]["result"]["message"]["method"],
        "command"
    );

    let old_closed = timeout(Duration::from_secs(1), old_subscription.recv())
        .await
        .expect("old subscription should close promptly");
    assert!(old_closed.is_none());
    drop(old_connection);
}

#[tokio::test]
async fn accept_skips_a_superseded_queued_connection() {
    let transport = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let (_, mut old_subscription) = module
        .raw_json_request(
            r#"{"jsonrpc":"2.0","method":"subscribe","params":{"connectionId":"connection-1"},"id":1}"#,
            1,
        )
        .await
        .expect("first subscription should open");
    let (_, mut new_subscription) = module
        .raw_json_request(
            r#"{"jsonrpc":"2.0","method":"subscribe","params":{"connectionId":"connection-1"},"id":2}"#,
            2,
        )
        .await
        .expect("replacement subscription should open");

    let old_closed = timeout(Duration::from_secs(1), old_subscription.recv())
        .await
        .expect("superseded subscription should close promptly");
    assert!(old_closed.is_none());

    let accepted = timeout(Duration::from_secs(1), transport.accept())
        .await
        .expect("accept should not hang")
        .expect("active replacement should be accepted");
    let active = accepted.into_channel();
    active
        .tx
        .unbounded_send(Ok(command()))
        .expect("accepted connection should still own the active route");

    let replacement_message = timeout(Duration::from_secs(1), new_subscription.recv())
        .await
        .expect("replacement should receive promptly")
        .expect("replacement should remain open");
    let replacement_message: Value = serde_json::from_str(replacement_message.get()).unwrap();
    assert_eq!(
        replacement_message["params"]["result"]["message"]["method"],
        "command"
    );
}

#[tokio::test]
async fn send_rejects_an_unknown_connection() {
    let transport = ServerTransport::new();
    let module = transport.rpc_module().expect("RPC module should build");
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "send",
        "params": RuntimeMessage::new("missing", event()),
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
    let transport = ServerTransport::new();
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

    let mut runtime = connect_runtime(Arc::clone(&client), "connection-1")
        .await
        .expect("runtime should subscribe");
    let mut server = timeout(Duration::from_secs(1), transport.accept())
        .await
        .expect("server accept should not hang")
        .expect("server should accept runtime")
        .into_channel();

    runtime.tx.unbounded_send(Ok(event())).unwrap();
    let delivered = timeout(Duration::from_secs(1), server.rx.next())
        .await
        .expect("runtime-to-server should not hang")
        .expect("server logical channel should remain open")
        .expect("event should be valid");
    assert_eq!(
        serde_json::to_value(delivered).unwrap()["method"],
        "system_event"
    );

    server.tx.unbounded_send(Ok(command())).unwrap();
    let delivered = timeout(Duration::from_secs(1), runtime.rx.next())
        .await
        .expect("server-to-runtime should not hang")
        .expect("runtime logical channel should remain open")
        .expect("command should be valid");
    assert_eq!(
        serde_json::to_value(delivered).unwrap()["method"],
        "command"
    );

    handle.stop().expect("server should stop");
    handle.stopped().await;
}

struct Commands;

impl CommandHandler for Commands {
    async fn handle(
        &self,
        command: Command,
    ) -> Result<CommandResult, agent_client_protocol::Error> {
        Ok(CommandResult::completed_with(json!({
            "commandId": command.command_id,
        })))
    }
}

struct Events(tokio::sync::mpsc::UnboundedSender<SystemEvent>);

impl SystemEventHandler for Events {
    async fn handle(&self, event: SystemEvent) -> Result<(), agent_client_protocol::Error> {
        self.0
            .send(event)
            .expect("event receiver should remain open");
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn role_connections_run_over_a_runtime_initiated_websocket() {
    let transport = ServerTransport::new();
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
    let runtime_channel = connect_runtime(client, "connection-1").await.unwrap();
    let server_channel = transport.accept().await.unwrap().into_channel();
    let (event_sender, mut event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let server_connection = ServerConnection::connect(server_channel, Events(event_sender));
    let runtime_connection = RuntimeConnection::connect(runtime_channel, Commands);

    runtime_connection
        .system_event(SystemEvent::new(
            "event-1",
            1,
            SystemEventName::Unknown("runtime/ready".to_owned()),
            "2026-07-17T00:00:00Z",
        ))
        .unwrap();
    let received = timeout(Duration::from_secs(1), event_receiver.recv())
        .await
        .expect("event should not hang")
        .expect("event channel should remain open");
    assert_eq!(received.event_id, "event-1");

    let result = timeout(
        Duration::from_secs(1),
        server_connection.command(Command::new(
            "command-1",
            CommandName::Unknown("runtime/configure".to_owned()),
        )),
    )
    .await
    .expect("command should not hang")
    .expect("command should succeed");
    assert_eq!(
        result,
        CommandResult::completed_with(json!({ "commandId": "command-1" }))
    );

    handle.stop().unwrap();
    handle.stopped().await;
}
