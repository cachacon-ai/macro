//! Run the logical protocol over a runtime-initiated jsonrpsee WebSocket.
//!
//! Run with:
//!
//! ```text
//! cargo run -p agent_runtime_protocol --example websocket
//! ```

use std::sync::Arc;

use agent_runtime_protocol::connection::{RuntimeConnection, ServerConnection};
use agent_runtime_protocol::transport::jsonrpsee::{ServerTransport, connect_runtime};
use jsonrpsee::server::Server;
use jsonrpsee::ws_client::WsClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transport = ServerTransport::new();
    let websocket = Server::builder().build("127.0.0.1:0").await?;
    let address = websocket.local_addr()?;
    let server_handle = websocket.start(transport.rpc_module()?);

    // The runtime initiates the WebSocket and then opens its message subscription.
    let client = Arc::new(
        WsClientBuilder::default()
            .build(format!("ws://{address}"))
            .await?,
    );
    let runtime_channel = connect_runtime(client, "connection-1").await?;
    let service_channel = transport
        .accept()
        .await
        .ok_or("runtime subscription closed before acceptance")?
        .into_channel();

    let service = ServerConnection::connect(service_channel, ());
    let runtime = RuntimeConnection::connect(runtime_channel, ());
    println!("runtime and service connected over WebSocket");

    drop(runtime);
    drop(service);
    server_handle.stop()?;
    server_handle.stopped().await;
    Ok(())
}
