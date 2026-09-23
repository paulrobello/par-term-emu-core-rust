//! End-to-end smoke test for the refactored WebSocket session dispatch
//! (ARC-004 / QA-002, ARC-001).
//!
//! Exercises the plain-WS path (`handle_connection_ws` → `run_ws_session`)
//! against a real `StreamingServer` brought up via its public `start()`
//! entrypoint. Verifies the full round trip: TCP connect → WS handshake →
//! initial `Connected` message → `Ping` request → `Pong` response.
//!
//! This is the strongest guard against subtle async regressions introduced by
//! the deduplication of the three WS handlers into the shared `run_ws_session`.
//!
//! The second test covers ARC-001: the HTTP-served axum path (`/ws`) must
//! forward `Mouse` input to the session's PTY writer — that path used to
//! silently drop Mouse/Focus/Paste/Selection/Clipboard messages behind a
//! `_ => {}` wildcard.
//!
//! Requires the `streaming` feature. Run with:
//!   cargo test --test test_ws_smoke --no-default-features \
//!     --features pyo3/auto-initialize,streaming

#![cfg(feature = "streaming")]

use futures_util::{SinkExt, StreamExt};
use par_term_emu_core_rust::mouse::{MouseEncoding, MouseMode};
use par_term_emu_core_rust::streaming::proto::{decode_server_message, encode_client_message};
use par_term_emu_core_rust::streaming::protocol::{ClientMessage, ServerMessage};
use par_term_emu_core_rust::streaming::{StreamingConfig, StreamingServer};
use par_term_emu_core_rust::terminal::Terminal;
use parking_lot::{Mutex, RwLock};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
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

/// PTY writer stub whose writes stall for a fixed time, simulating a
/// non-reading foreground process with a full kernel buffer (SEC-005).
struct StallingWriter {
    stall: Duration,
    captured: Arc<Mutex<Vec<u8>>>,
}

impl Write for StallingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.stall);
        self.captured.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// SEC-005: a stalled PTY write must not block the session loop. The write
/// runs on the blocking pool, so a Ping sent right after an Input whose PTY
/// write stalls for 3 seconds must still round-trip promptly — with the old
/// synchronous write inside the select loop, the Pong could only arrive
/// after the stall.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_pty_write_does_not_block_ping_round_trip() {
    let port = ephemeral_port();
    let addr = format!("127.0.0.1:{}", port);

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let server = Arc::new(StreamingServer::new(terminal, addr.clone()));
    server.set_pty_writer(Arc::new(Mutex::new(Box::new(StallingWriter {
        stall: Duration::from_secs(3),
        captured: Arc::clone(&captured),
    }) as Box<dyn Write + Send>)));

    let server_handle = tokio::spawn(async move { server.start().await });

    // Wait briefly for the listener to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let url = format!("ws://{}", addr);
    let (mut ws, _response) = connect_async(url).await.expect("WS handshake");

    // Drain the initial Connected message.
    let first = ws.next().await.expect("server sent a message").unwrap();
    match first {
        Message::Binary(data) => {
            let msg = decode_server_message(&data).expect("decode Connected");
            assert!(matches!(msg, ServerMessage::Connected { .. }));
        }
        other => panic!("expected Binary Connected, got {:?}", other),
    }

    // An Input whose PTY write stalls for 3s, then an immediate Ping.
    let input = ClientMessage::Input {
        data: "x".repeat(16),
    };
    ws.send(Message::Binary(
        encode_client_message(&input).unwrap().into(),
    ))
    .await
    .expect("send Input");

    let ping_bytes = encode_client_message(&ClientMessage::Ping).unwrap();
    ws.send(Message::Binary(ping_bytes.into()))
        .await
        .expect("send Ping");

    // The Pong must arrive well before the 3s write stall elapses.
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut saw_pong = false;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Binary(data)))) => {
                let msg = decode_server_message(&data).expect("decode reply");
                if matches!(msg, ServerMessage::Pong) {
                    saw_pong = true;
                    break;
                }
            }
            Ok(Some(Ok(other))) => eprintln!("ignoring non-binary frame: {:?}", other),
            Ok(Some(Err(e))) => panic!("ws error waiting for pong: {}", e),
            Ok(None) => panic!("stream closed before pong arrived"),
            Err(_) => break, // deadline hit
        }
    }
    assert!(
        saw_pong,
        "Pong did not arrive within 1.5s — the stalled PTY write is blocking the session loop"
    );

    server_handle.abort();
}

/// SEC-004: pre-handshake slot reservation. With `max_clients = N`, the
/// (N+1)-th raw TCP connection that never upgrades is refused promptly
/// (closed by the server) instead of being held open indefinitely — slots
/// are now reserved before the WebSocket handshake, so unauthenticated
/// pre-upgrade connections cannot occupy tasks uncapped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extra_raw_connection_beyond_max_clients_is_refused() {
    use tokio::io::AsyncReadExt;

    let port = ephemeral_port();
    let addr = format!("127.0.0.1:{}", port);

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = {
        let config = StreamingConfig {
            max_clients: 2,
            ..Default::default()
        };
        Arc::new(StreamingServer::with_config(terminal, addr.clone(), config))
    };
    let server_handle = tokio::spawn(async move { server.start().await });

    // Wait briefly for the listener to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Two raw connections that never send a WebSocket handshake: both hold
    // a reserved slot (up to the 10s handshake timeout).
    let s1 = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let s2 = tokio::net::TcpStream::connect(&addr).await.unwrap();
    // Give the accept loop a moment to process both accepts.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The third connection exceeds max_clients and must be closed by the
    // server (EOF on read) rather than left lingering. WHICH of the three
    // raw connections ends up the excess one is a scheduling race — slot
    // reservation happens in the per-connection handshake task, not at
    // accept, so under load s3's task can reserve ahead of s1's or s2's —
    // so poll all three and require exactly one closure within a deadline
    // that covers both the immediate refusal and the slower
    // handshake-timeout path; the two slot-holders must survive.
    let s3 = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let mut streams = vec![s1, s2, s3];
    let mut buf = [0u8; 16];
    let close_by = tokio::time::Instant::now() + Duration::from_secs(15);
    while streams.len() > 2 {
        assert!(
            tokio::time::Instant::now() < close_by,
            "no connection was closed within 15s — the excess one is lingering"
        );
        for i in (0..streams.len()).rev() {
            match tokio::time::timeout(Duration::from_millis(50), streams[i].read(&mut buf)).await {
                Ok(Ok(0)) => {
                    streams.swap_remove(i); // EOF — server closed this one
                }
                Ok(Ok(n)) => panic!("expected close, server sent {} bytes: {:?}", n, &buf[..n]),
                Ok(Err(e)) => panic!("read error on refused connection: {}", e),
                Err(_) => {}
            }
        }
    }

    // Sanity: the two slot-holders are still open (only the handshake
    // timeout, not this test, closes them).
    for slot_holder in &mut streams {
        assert!(
            tokio::time::timeout(Duration::from_millis(50), slot_holder.read(&mut buf))
                .await
                .is_err(),
            "a slot-holding connection was closed"
        );
    }

    drop(streams);
    server_handle.abort();
}

/// PTY writer stub that records every byte written through it.
struct CapturingWriter {
    captured: Arc<Mutex<Vec<u8>>>,
}

impl Write for CapturingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.captured.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// ARC-001: a `Mouse` message sent over the axum HTTP-served path
/// (`start_with_http`, `/ws` endpoint) must reach the session's PTY writer.
/// That loop used to drop Mouse/Focus/Paste/Selection/Clipboard messages via
/// a `_ => {}` wildcard, so browser clients in `--http` mode silently lost
/// mouse input. Both loops now share `handle_client_message`; this test
/// guards the browser-reachable path end to end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axum_http_path_forwards_mouse_to_pty_writer() {
    let port = ephemeral_port();
    let addr = format!("127.0.0.1:{}", port);

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));

    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let server = {
        let config = StreamingConfig {
            enable_http: true,
            ..Default::default()
        };
        Arc::new(StreamingServer::with_config(
            Arc::clone(&terminal),
            addr.clone(),
            config,
        ))
    };
    server.set_pty_writer(Arc::new(Mutex::new(Box::new(CapturingWriter {
        captured: Arc::clone(&captured),
    }) as Box<dyn Write + Send>)));

    let server_handle = tokio::spawn(async move { server.start().await });

    // Wait briefly for the listener to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Connect a real WS client to the axum-served /ws endpoint.
    let url = format!("ws://{}/ws", addr);
    let (mut ws, _response) = connect_async(url).await.expect("WS handshake via axum /ws");

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

    // Enable SGR mouse tracking, as a real frontend does (DECSET 1000/1006);
    // with tracking off `report_mouse` legitimately encodes nothing.
    {
        let mut term = terminal.write();
        term.set_mouse_mode(MouseMode::Normal);
        term.set_mouse_encoding(MouseEncoding::Sgr);
    }

    // Left-button press at col=10, row=4.
    let mouse = ClientMessage::Mouse {
        col: 10,
        row: 4,
        button: 0,
        shift: false,
        ctrl: false,
        alt: false,
        event_type: "press".to_string(),
    };
    ws.send(Message::Binary(
        encode_client_message(&mouse).unwrap().into(),
    ))
    .await
    .expect("send Mouse");

    // SGR encoding of button 0 press at (10, 4): CSI < 0 ; 11 ; 5 M
    // (1-based coords in the escape sequence).
    let expected: &[u8] = b"\x1b[<0;11;5M";
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        {
            let buf = captured.lock();
            if buf.as_slice() == expected {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "PTY writer never received the mouse bytes; captured so far: {:?}",
            captured.lock()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    server_handle.abort();
}
