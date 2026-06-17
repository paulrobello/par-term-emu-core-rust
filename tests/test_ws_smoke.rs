//! End-to-end smoke test for the refactored WebSocket session dispatch
//! (ARC-004 / QA-002).
//!
//! Exercises the plain-WS path (`handle_connection_ws` → `run_ws_session`)
//! against a real `StreamingServer` brought up via its public `start()`
//! entrypoint. Verifies the full round trip: TCP connect → WS handshake →
//! initial `Connected` message → `Ping` request → `Pong` response.
//!
//! This is the strongest guard against subtle async regressions introduced by
//! the deduplication of the three WS handlers into the shared `run_ws_session`.
//!
//! Requires the `streaming` feature. Run with:
//!   cargo test --test test_ws_smoke --no-default-features \
//!     --features pyo3/auto-initialize,streaming

#![cfg(feature = "streaming")]

use futures_util::{SinkExt, StreamExt};
use par_term_emu_core_rust::streaming::proto::{decode_server_message, encode_client_message};
use par_term_emu_core_rust::streaming::protocol::{ClientMessage, ServerMessage};
use par_term_emu_core_rust::streaming::StreamingServer;
use par_term_emu_core_rust::terminal::Terminal;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Grab an ephemeral free port from the OS (bind to :0, read the port, drop).
fn ephemeral_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Bring up a StreamingServer via its public `start()` entrypoint, connect a
/// real WS client, and verify the handshake + a Ping/Pong round trip through
/// the shared `run_ws_session`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_smoke_ping_pong_round_trip() {
    let port = ephemeral_port();
    let addr = format!("127.0.0.1:{}", port);

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, addr.clone()));
    let server_handle = tokio::spawn(async move { server.start().await });

    // Wait briefly for the listener to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Connect a real WS client.
    let url = format!("ws://{}", addr);
    let (mut ws, _response) = connect_async(url).await.expect("WS handshake");

    // Expect an initial Connected message from the server.
    let first = ws.next().await.expect("server sent a message").unwrap();
    match first {
        Message::Binary(data) => {
            let msg = decode_server_message(&data).expect("decode Connected");
            assert!(
                matches!(msg, ServerMessage::Connected { .. }),
                "expected Connected, got {:?}",
                msg
            );
        }
        other => panic!("expected Binary Connected, got {:?}", other),
    }

    // Send a Ping and expect a Pong.
    let ping_bytes = encode_client_message(&ClientMessage::Ping).unwrap();
    ws.send(Message::Binary(ping_bytes.into()))
        .await
        .expect("send Ping");

    let mut saw_pong = false;
    for _ in 0..10 {
        match ws.next().await {
            Some(Ok(Message::Binary(data))) => {
                let msg = decode_server_message(&data).expect("decode reply");
                if matches!(msg, ServerMessage::Pong) {
                    saw_pong = true;
                    break;
                }
            }
            Some(Ok(other)) => eprintln!("ignoring non-binary frame: {:?}", other),
            Some(Err(e)) => panic!("ws error waiting for pong: {}", e),
            None => panic!("stream closed before pong arrived"),
        }
    }
    assert!(saw_pong, "did not receive Pong within message budget");

    server_handle.abort();
}
