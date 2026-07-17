//! Interface-conformance tests shared across every [`Transport`] implementation.
//!
//! Each implementation is exercised through the same assertion body, run as
//! an outer `#[test]` per implementation. This is the pattern the transport
//! port exists to enable: swapping [`FakeTransport`] for
//! [`crate::outbound::jsonrpsee::JsonRpseeWire`] costs nothing beyond
//! supplying a different way to observe the transport's counterpart.

use std::future::Future;
use std::sync::Arc;

use jsonrpsee::server::Server;
use jsonrpsee::ws_client::WsClientBuilder;
use tokio::time::{Duration, timeout};

use crate::domain::channel::pump;
use crate::domain::ports::Transport;
use crate::outbound::jsonrpsee::{JsonRpseeWire, ServerTransport};
use crate::testing::fake_wire::FakeTransport;

/// Interface-conformance assertion shared by every [`Transport`] implementation:
/// a message sent through the connected channel reaches the transport's
/// counterpart. Each implementation supplies its own way to observe that
/// counterpart, since that's the one part a transport port intentionally
/// doesn't model.
async fn asserts_runtime_to_server_delivery<T, F, Fut>(transport: Arc<T>, receive_from_transport: F)
where
    T: Transport<String, String> + Send + Sync + 'static,
    F: FnOnce() -> Fut,
    Fut: Future<Output = String>,
{
    let channel = pump(transport);
    channel.tx.send("system_event".to_owned()).unwrap();

    let received = timeout(Duration::from_secs(1), receive_from_transport())
        .await
        .expect("transport should receive the message promptly");
    assert_eq!(received, "system_event");
}

#[tokio::test]
async fn fake_transport_delivers_runtime_to_server_messages() {
    let (transport, mut probe) = FakeTransport::new();
    asserts_runtime_to_server_delivery(
        Arc::new(transport),
        || async move { probe.next_send().await },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn jsonrpsee_transport_delivers_runtime_to_server_messages() {
    let transport: ServerTransport<String, String> = ServerTransport::new();
    let server = Server::builder()
        .build("127.0.0.1:0")
        .await
        .expect("server should bind");
    let address = server.local_addr().expect("server address");
    let handle = server.start(transport.rpc_module().expect("RPC module should build"));
    let client = Arc::new(
        WsClientBuilder::default()
            .build(format!("ws://{address}"))
            .await
            .expect("runtime should connect"),
    );

    asserts_runtime_to_server_delivery(Arc::new(JsonRpseeWire::new(client)), || async {
        let mut channel = timeout(Duration::from_secs(1), transport.accept())
            .await
            .expect("server accept should not hang")
            .expect("server should accept runtime");
        channel
            .rx
            .recv()
            .await
            .expect("server logical channel should remain open")
    })
    .await;

    handle.stop().expect("server should stop");
    handle.stopped().await;
}
