fn roster_msg() -> ServerMessage {
    ServerMessage::AgentRoster { agents: vec![] }
}

fn delta_msg() -> ServerMessage {
    ServerMessage::AgentStateChanged {
        agent: crate::streaming::protocol::AgentEntry::default(),
        released: false,
    }
}

#[test]
fn should_send_passes_roster_messages_through_any_filter() {
    use crate::streaming::protocol::EventType;
    let only_bell: Option<std::collections::HashSet<EventType>> =
        Some([EventType::Bell].into_iter().collect());
    let empty: Option<std::collections::HashSet<EventType>> =
        Some(std::collections::HashSet::new());
    for subs in [&only_bell, &empty, &None] {
        assert!(should_send(&roster_msg(), subs));
        assert!(should_send(&delta_msg(), subs));
    }
    // Control: a filtered event type is still dropped, and delivered when
    // the client subscribes to Bell.
    assert!(should_send(&ServerMessage::Bell, &only_bell));
    assert!(!should_send(&ServerMessage::Bell, &empty));
}

#[tokio::test]
async fn broadcast_all_reaches_every_session() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());
    let second = Arc::new(StreamSessionState::new(
        "second".to_string(),
        Arc::new(RwLock::new(Terminal::new(80, 24))),
        None,
        false,
    ));
    server
        .sessions
        .insert("second".to_string(), Arc::clone(&second))
        .unwrap();
    let mut rx_default = server
        .sessions
        .get("default")
        .unwrap()
        .broadcast_tx
        .subscribe();
    let mut rx_second = second.broadcast_tx.subscribe();

    server.broadcast_all(delta_msg());

    assert!(matches!(
        rx_default.try_recv(),
        Ok(ServerMessage::AgentStateChanged { .. })
    ));
    assert!(matches!(
        rx_second.try_recv(),
        Ok(ServerMessage::AgentStateChanged { .. })
    ));
}

use super::*;
use crate::streaming::session::OutputChunk;
use crate::terminal::Terminal;

#[tokio::test]
async fn test_output_sender() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());

    let tx = server.get_output_sender();
    assert!(tx.try_send(OutputChunk::plain("test".to_string())).is_ok());
}

#[tokio::test]
async fn test_streaming_server_creation() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());
    assert_eq!(server.addr, "127.0.0.1:0");
}

#[tokio::test]
async fn test_connection_params_defaults() {
    let params = ConnectionParams::from_uri_query(None);
    assert_eq!(params.session_id, "default");
    assert!(!params.readonly);
    assert!(params.preset.is_none());
}

#[tokio::test]
async fn test_connection_params_parsing() {
    let params =
        ConnectionParams::from_uri_query(Some("session=my-sess&readonly=true&preset=python"));
    assert_eq!(params.session_id, "my-sess");
    assert!(params.readonly);
    assert_eq!(params.preset, Some("python".to_string()));
}

#[tokio::test]
async fn test_connection_params_partial() {
    let params = ConnectionParams::from_uri_query(Some("readonly=1"));
    assert_eq!(params.session_id, "default");
    assert!(params.readonly);
    assert!(params.preset.is_none());
}

/// SEC-011: session ids that do not match `[A-Za-z0-9_-]{1,64}` are
/// rejected (never reach the session registry or factory) and the
/// connection falls back to the default session.
#[tokio::test]
async fn test_connection_params_rejects_invalid_session_id() {
    for query in [
        "session=../x",
        "session=has%20space",
        "session=../etc/passwd",
        "session=a/b",
        "session=..",
        "session=.",
    ] {
        let params = ConnectionParams::from_uri_query(Some(query));
        assert_eq!(
            params.session_id, "default",
            "session id from {:?} must be rejected",
            query
        );
    }
    // Over-length id (65+ chars) is rejected.
    let long_id = "a".repeat(65);
    let params = ConnectionParams::from_uri_query(Some(&format!("session={}", long_id)));
    assert_eq!(params.session_id, "default");

    // Legal ids still pass: dashes, underscores, mixed case, 64 chars.
    let params = ConnectionParams::from_uri_query(Some("session=my-Sess_ion-42"));
    assert_eq!(params.session_id, "my-Sess_ion-42");
    let ok_id = "a".repeat(64);
    let params = ConnectionParams::from_uri_query(Some(&format!("session={}", ok_id)));
    assert_eq!(params.session_id, ok_id);
}

/// Session factory stub that counts spawns (SEC-011 test).
struct CountingFactory {
    spawns: Arc<AtomicUsize>,
}

impl SessionFactory for CountingFactory {
    fn create_session(
        &self,
        _session_id: &str,
        _cols: u16,
        _rows: u16,
        _shell_command: Option<&str>,
    ) -> std::result::Result<SessionFactoryResult, StreamingError> {
        self.spawns.fetch_add(1, Ordering::Relaxed);
        Ok(SessionFactoryResult {
            terminal: Arc::new(RwLock::new(Terminal::new(80, 24))),
            pty_writer: None,
        })
    }
    fn setup_session(
        &self,
        _session_id: &str,
        _session: &Arc<StreamSessionState>,
    ) -> std::result::Result<(), StreamingError> {
        Ok(())
    }
    fn teardown_session(&self, _session_id: &str) {}
}

/// SEC-011: the global client slot must be reserved before the session
/// is resolved/created, so a server at max_clients rejects a new
/// connection with a fresh session id without ever invoking the
/// session factory (no PTY spawn for a connection that will be
/// refused anyway).
#[tokio::test]
async fn max_clients_reached_rejects_before_session_spawn() {
    let config = StreamingConfig {
        max_clients: 1,
        ..StreamingConfig::default()
    };
    let spawns = Arc::new(AtomicUsize::new(0));
    let factory = Arc::new(CountingFactory {
        spawns: Arc::clone(&spawns),
    });
    let server = Arc::new(StreamingServer::with_factory(
        "127.0.0.1:0".to_string(),
        config,
        factory,
    ));

    // First client takes the only slot.
    assert!(server.try_add_client());
    let _held = GlobalClientGuard { server: &server };

    // A second client with a fresh session id is rejected by the slot
    // reservation — the factory never spawns.
    assert!(!server.try_add_client());
    assert_eq!(spawns.load(Ordering::Relaxed), 0);

    // Positive control: with the slot released, resolving a fresh
    // session id does spawn exactly one session through prepare.
    drop(_held);
    let params = ConnectionParams {
        session_id: "fresh-session".to_string(),
        readonly: false,
        preset: None,
        metadata: false,
    };
    let (_session, _g, _s, _ro) = server
        .prepare_ws_session(&params, GlobalClientGuard { server: &server })
        .await
        .expect("fresh session resolves once a slot is free");
    assert_eq!(spawns.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_default_session_exists() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));

    let params = ConnectionParams::from_uri_query(None);
    let session = server.resolve_session(&params);
    assert!(session.is_ok());
    assert_eq!(session.unwrap().id, "default");
}

#[tokio::test]
async fn test_resolve_nonexistent_session_no_factory() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));

    let params = ConnectionParams::from_uri_query(Some("session=nonexistent"));
    let result = server.resolve_session(&params);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::SessionNotFound(_)
    ));
}

/// ARC-002: the registry is the default session's only owner. Closing it
/// in single-session mode must leave no stale second handle: a later
/// resolve is a clean `SessionNotFound` instead of a dead-session
/// handout, and server broadcasts stop at the registry miss.
#[tokio::test]
async fn close_default_session_leaves_no_stale_owner() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));

    let old = server
        .get_session("default")
        .expect("default session exists");
    let mut rx = old.broadcast_tx.subscribe();

    assert!(server.close_session("default", "test close".to_string()));
    assert!(server.get_session("default").is_none());

    // Drain the shutdown notice the close itself broadcast, then prove a
    // server-level broadcast no longer reaches the dead session.
    while rx.try_recv().is_ok() {}
    server.broadcast(ServerMessage::output("after close".to_string()));
    assert!(rx.try_recv().is_err());

    // Reconnect path: a clean miss, never the dead session.
    let params = ConnectionParams::from_uri_query(None);
    assert!(matches!(
        server.resolve_session(&params),
        Err(StreamingError::SessionNotFound(_))
    ));
}

/// Input against a session whose PTY writer is detached (never
/// attached, or detached by [`StreamSessionState::shutdown`]) is dropped
/// and counted in `dropped_messages`, and the connection stays open: a
/// reconnect would find the same writer-less session. Non-input
/// messages still get their replies, and read-only viewers' input is
/// ignored before the writer check.
#[tokio::test]
async fn input_without_pty_writer_is_counted_and_dropped() {
    use crate::streaming::protocol::ClientMessage;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
    // The default session has no factory behind it, hence no writer.
    let session = server.get_session("default").expect("default session");
    assert!(!StreamingServer::session_has_writer(&session));

    let dispatch = |read_only: bool, msg: ClientMessage| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only,
                    },
                    &mut None,
                    &mut None,
                    msg,
                )
                .await
        }
    };

    let inputs = [
        ClientMessage::Input {
            data: "q".to_string(),
        },
        ClientMessage::Paste {
            content: "pasted".to_string(),
        },
        ClientMessage::Mouse {
            col: 1,
            row: 1,
            button: 0,
            shift: false,
            ctrl: false,
            alt: false,
            event_type: crate::streaming::protocol::MouseEventType::Press,
        },
        ClientMessage::FocusChange { focused: true },
    ];
    for (i, msg) in inputs.into_iter().enumerate() {
        assert!(
            dispatch(false, msg).await.is_empty(),
            "input produces no reply"
        );
        assert_eq!(
            session.metrics.dropped_messages.load(Ordering::Relaxed),
            i + 1
        );
    }

    // Non-input messages on the same writer-less session still reply.
    let replies = dispatch(false, ClientMessage::Ping).await;
    assert!(matches!(replies.as_slice(), [ServerMessage::Pong]));
    assert_eq!(
        session.metrics.dropped_messages.load(Ordering::Relaxed),
        4,
        "ping is not input and must not count as a drop"
    );

    // Read-only viewers are exempt: their input is ignored by the
    // handler, not counted by the detached-writer guard.
    let replies = dispatch(
        true,
        ClientMessage::Input {
            data: "q".to_string(),
        },
    )
    .await;
    assert!(replies.is_empty());
    assert_eq!(
        session.metrics.dropped_messages.load(Ordering::Relaxed),
        4,
        "read-only input is not a writer-detached drop"
    );
}

/// Upload messages on a session without a mux factory have nowhere to
/// stage: every verb is refused synchronously with an `upload_finished`
/// error, and a read-only viewer is refused before the pipe lookup.
#[tokio::test]
async fn upload_messages_without_a_mux_pipe_are_refused_inline() {
    use crate::streaming::protocol::ClientMessage;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
    let session = server.get_session("default").expect("default session");

    let dispatch = |read_only: bool, msg: ClientMessage| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only,
                    },
                    &mut None,
                    &mut None,
                    msg,
                )
                .await
        }
    };

    let refused = |msg: ClientMessage| async {
        match dispatch(false, msg).await.as_slice() {
            [ServerMessage::UploadFinished {
                upload_id: 0,
                path: None,
                error: Some(reason),
            }] => reason.clone(),
            other => panic!("expected one upload_finished refusal, got {other:?}"),
        }
    };

    assert!(
        refused(ClientMessage::UploadBegin {
            filename: "x.png".into(),
            total_bytes: 5,
        })
        .await
        .contains("mux-backed"),
        "no-pipe begin must refuse with the mux reason"
    );
    assert!(
        refused(ClientMessage::UploadChunk {
            data: b"xy".to_vec()
        })
        .await
        .contains("mux-backed"),
        "no-pipe chunk must refuse with the mux reason"
    );
    assert!(
        refused(ClientMessage::UploadCommit)
            .await
            .contains("mux-backed"),
        "no-pipe commit must refuse with the mux reason"
    );
    assert!(
        refused(ClientMessage::UploadAbort)
            .await
            .contains("mux-backed"),
        "no-pipe abort must refuse with the mux reason"
    );

    // A read-only viewer is refused before the pipe lookup.
    match dispatch(
        true,
        ClientMessage::UploadBegin {
            filename: "x.png".into(),
            total_bytes: 5,
        },
    )
    .await
    .as_slice()
    {
        [ServerMessage::UploadFinished {
            error: Some(reason),
            ..
        }] => assert!(reason.contains("read-only")),
        other => panic!("expected the read-only refusal, got {other:?}"),
    }
}

// =========================================================================
// Terminal Size Validation Tests
// =========================================================================

#[tokio::test]
async fn test_validate_terminal_size_valid_min() {
    let result = validate_terminal_size(MIN_COLS, MIN_ROWS);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), (MIN_COLS, MIN_ROWS));
}

#[tokio::test]
async fn test_validate_terminal_size_valid_max() {
    let result = validate_terminal_size(MAX_COLS, MAX_ROWS);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), (MAX_COLS, MAX_ROWS));
}

#[tokio::test]
async fn test_validate_terminal_size_valid_typical() {
    let result = validate_terminal_size(80, 24);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), (80, 24));
}

#[tokio::test]
async fn test_validate_terminal_size_cols_below_min() {
    let result = validate_terminal_size(1, 24);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_cols_zero() {
    let result = validate_terminal_size(0, 24);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_cols_above_max() {
    let result = validate_terminal_size(1001, 24);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_rows_below_min() {
    let result = validate_terminal_size(80, 0);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_rows_above_max() {
    let result = validate_terminal_size(80, 501);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_both_invalid() {
    let result = validate_terminal_size(0, 0);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

#[tokio::test]
async fn test_validate_terminal_size_max_u16() {
    let result = validate_terminal_size(u16::MAX, u16::MAX);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StreamingError::InvalidInput(_)
    ));
}

// =========================================================================
// HttpBasicAuthConfig Tests
// =========================================================================

// =========================================================================
// SessionRegistry Tests
// =========================================================================

// =========================================================================
// StreamingConfig Tests
// =========================================================================

// =========================================================================
// ApiAuthConfig Tests
// =========================================================================

/// Send one credential-less GET through `api_auth_middleware` (wired the
/// same way `build_http_app` wires it) against the given target URI and
/// return the response.
async fn auth_middleware_response(
    auth_config: ApiAuthConfig,
    auth_throttle: Arc<Mutex<AuthFailureThrottle>>,
    target_uri: &str,
) -> axum::http::Response<axum::body::Body> {
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    let app = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(move |req, next| {
            let auth_config = auth_config.clone();
            let auth_throttle = auth_throttle.clone();
            api_auth_middleware(req, next, auth_config, auth_throttle)
        }));
    let req = axum::http::Request::builder()
        .uri(target_uri)
        .body(axum::body::Body::empty())
        .expect("static request parts are valid");
    app.oneshot(req).await.expect("router is infallible")
}

/// Send one credential-less GET to `/ok` through `api_auth_middleware`.
async fn unauthenticated_response(
    auth_config: ApiAuthConfig,
) -> axum::http::Response<axum::body::Body> {
    auth_middleware_response(
        auth_config,
        Arc::new(Mutex::new(AuthFailureThrottle::default())),
        "/ok",
    )
    .await
}

#[tokio::test]
async fn test_auth_middleware_basic_auth_401_carries_challenge() {
    let res = unauthenticated_response(ApiAuthConfig {
        api_key: None,
        http_basic_auth: Some(HttpBasicAuthConfig::with_password(
            "admin".to_string(),
            "secret".to_string(),
        )),
        allow_api_key_in_query: false,
    })
    .await;
    assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        res.headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Basic realm=\"Terminal Server\"")
    );
}

#[tokio::test]
async fn test_auth_middleware_api_key_only_401_has_no_challenge() {
    let res = unauthenticated_response(ApiAuthConfig {
        api_key: Some("key".to_string()),
        http_basic_auth: None,
        allow_api_key_in_query: false,
    })
    .await;
    assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert!(!res
        .headers()
        .contains_key(axum::http::header::WWW_AUTHENTICATE));
}

// ─── Query API-key percent-decoding (SEC-002) ─────────────────────────────

#[test]
fn test_percent_decode_query_component() {
    assert_eq!(
        percent_decode_query_component("plain-key_42"),
        "plain-key_42"
    );
    assert_eq!(percent_decode_query_component("a%26b%3Dc%25d"), "a&b=c%d");
    // Encoded plus decodes; a raw plus passes through unchanged.
    assert_eq!(percent_decode_query_component("%2B"), "+");
    assert_eq!(percent_decode_query_component("a+b"), "a+b");
    // Malformed escapes are left as-is (the comparison then fails safely).
    assert_eq!(percent_decode_query_component("100%"), "100%");
    assert_eq!(percent_decode_query_component("%zz"), "%zz");
    assert_eq!(percent_decode_query_component("%2"), "%2");
}

#[tokio::test]
async fn test_auth_middleware_query_api_key_percent_decoded() {
    // A key containing characters a raw query cannot carry must match via
    // the query channel once URL-encoded (SEC-002).
    let res = auth_middleware_response(
        ApiAuthConfig {
            api_key: Some("s3cr&t=k%y+".to_string()),
            http_basic_auth: None,
            allow_api_key_in_query: true,
        },
        Arc::new(Mutex::new(AuthFailureThrottle::default())),
        "/ok?api_key=s3cr%26t%3Dk%25y%2B",
    )
    .await;
    assert_eq!(res.status(), axum::http::StatusCode::OK);

    // The same key sent raw cannot match: the `&` splits the query.
    let res = auth_middleware_response(
        ApiAuthConfig {
            api_key: Some("s3cr&t=k%y+".to_string()),
            http_basic_auth: None,
            allow_api_key_in_query: true,
        },
        Arc::new(Mutex::new(AuthFailureThrottle::default())),
        "/ok?api_key=s3cr&t=k%y+",
    )
    .await;
    assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[test]
fn test_validate_ws_handshake_auth_query_api_key_percent_decoded() {
    let req = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri("ws://127.0.0.1:8099/ws?api_key=a%26b%3Dc")
        .body(())
        .expect("static request parts are valid");
    assert!(validate_ws_handshake_auth(&req, Some("a&b=c"), None, true));
}

// ─── Static-route Host allowlist (2026-11-06 audit SEC-003) ────────────────

#[test]
fn test_static_host_guard_bind_classification() {
    for loopback in [
        "127.0.0.1:8099",
        "localhost:8099",
        "[::1]:8099",
        "127.0.0.5:9000",
    ] {
        assert!(
            StaticHostGuard::from_bind_addr(loopback).is_enabled(),
            "{loopback} is a loopback bind and must enable the guard"
        );
    }
    for remote in [
        "0.0.0.0:8099",
        "192.168.1.5:8099",
        "[::]:8099",
        "example.com:8099",
    ] {
        assert!(
            !StaticHostGuard::from_bind_addr(remote).is_enabled(),
            "{remote} is not a loopback bind; the guard must stay inactive"
        );
    }
}

#[tokio::test]
async fn test_static_host_guard_rejects_non_loopback_host_on_loopback_bind() {
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    // Same shape as build_http_app: the guard layers the static fallback
    // only; the API routes keep their own checks.
    let api_routes = Router::new().route("/ws", get(|| async { "ws" }));
    let static_routes =
        Router::new()
            .fallback(|| async { "static" })
            .layer(axum::middleware::from_fn_with_state(
                StaticHostGuard::from_bind_addr("127.0.0.1:8099"),
                static_host_guard_middleware,
            ));
    let app = api_routes.merge(static_routes);

    // A rebinding attack sends its own Host: refused on the static route.
    let req = axum::http::Request::builder()
        .uri("/index.html")
        .header("host", "attacker.example")
        .body(axum::body::Body::empty())
        .expect("static request parts are valid");
    let res = app
        .clone()
        .oneshot(req)
        .await
        .expect("router is infallible");
    assert_eq!(res.status(), axum::http::StatusCode::FORBIDDEN);

    // Loopback Hosts (with or without port, IPv6 brackets) pass.
    for host in ["localhost:8099", "127.0.0.1", "[::1]:8099"] {
        let req = axum::http::Request::builder()
            .uri("/index.html")
            .header("host", host)
            .body(axum::body::Body::empty())
            .expect("static request parts are valid");
        let res = app
            .clone()
            .oneshot(req)
            .await
            .expect("router is infallible");
        assert_eq!(res.status(), axum::http::StatusCode::OK, "Host {host}");
    }

    // A missing Host header is a non-browser client and passes, mirroring
    // check_ws_origin's absent-Origin rule (a rebinding page always names
    // its own host).
    let req = axum::http::Request::builder()
        .uri("/index.html")
        .body(axum::body::Body::empty())
        .expect("static request parts are valid");
    let res = app
        .clone()
        .oneshot(req)
        .await
        .expect("router is infallible");
    assert_eq!(res.status(), axum::http::StatusCode::OK);

    // The API routes are outside the guard: a non-loopback Host still
    // reaches them (their own auth/origin checks apply).
    let req = axum::http::Request::builder()
        .uri("/ws")
        .header("host", "attacker.example")
        .body(axum::body::Body::empty())
        .expect("static request parts are valid");
    let res = app.oneshot(req).await.expect("router is infallible");
    assert_eq!(res.status(), axum::http::StatusCode::OK);
}

// ─── Failed-auth throttling (2026-11-06 audit SEC-004) ─────────────────────

#[test]
fn test_auth_failure_throttle_escalates_and_caps() {
    let mut throttle = AuthFailureThrottle::default();
    let expected_ms = [50u64, 100, 200, 400, 800, 1600, 3200, 5000, 5000];
    for (failures, ms) in expected_ms.iter().enumerate() {
        assert_eq!(
            throttle.record_failure(),
            Duration::from_millis(*ms),
            "delay after {failures} prior failures"
        );
        assert_eq!(throttle.consecutive_failures(), failures as u32 + 1);
    }
}

#[test]
fn test_auth_failure_throttle_decays_after_quiet_window() {
    let mut throttle = AuthFailureThrottle::default();
    let t0 = std::time::Instant::now();
    assert_eq!(throttle.record_failure_at(t0), Duration::from_millis(50));
    assert_eq!(
        throttle.record_failure_at(t0 + Duration::from_secs(5)),
        Duration::from_millis(100)
    );
    assert_eq!(throttle.consecutive_failures(), 2);
    // A quiet window past DECAY restarts the escalation from the base.
    let t2 = t0 + Duration::from_secs(5) + AuthFailureThrottle::DECAY + Duration::from_secs(1);
    assert_eq!(throttle.record_failure_at(t2), Duration::from_millis(50));
    assert_eq!(throttle.consecutive_failures(), 1);
}

#[tokio::test]
async fn test_auth_middleware_failures_escalate_throttle_counter() {
    // Assert the counter state, not wall-clock (the three sleeps total
    // 350 ms of real time in this test).
    let throttle = Arc::new(Mutex::new(AuthFailureThrottle::default()));
    let auth_config = ApiAuthConfig {
        api_key: Some("key".to_string()),
        http_basic_auth: None,
        allow_api_key_in_query: false,
    };
    for _ in 0..3 {
        let res = auth_middleware_response(auth_config.clone(), Arc::clone(&throttle), "/ok").await;
        assert_eq!(res.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
    assert_eq!(throttle.lock().consecutive_failures(), 3);
}

// ─── ClipboardRequest set gating (SEC-003 follow-up) ──────────────────────

#[tokio::test]
async fn clipboard_set_applies_and_broadcasts_by_default() {
    let server = Arc::new(StreamingServer::new(
        Arc::new(RwLock::new(Terminal::new(80, 24))),
        "127.0.0.1:0".to_string(),
    ));
    let session = server.sessions.get("default").unwrap();
    let mut rx = session.broadcast_tx.subscribe();

    let replies = server.handle_clipboard_request(
        &session,
        ConnCtx {
            transport_label: "test",
            client_id: uuid::Uuid::new_v4(),
            read_only: false,
        },
        "set".to_string(),
        Some("hi".to_string()),
        None,
    );

    assert!(replies.is_empty());
    assert_eq!(session.terminal.read().clipboard(), Some("hi"));
    assert!(matches!(
        rx.try_recv(),
        Ok(ServerMessage::ClipboardSync { .. })
    ));
}

/// The streaming set path honors the same `allow_clipboard_write` gate as
/// OSC 52 writes — off means no store and no `clipboard_sync` broadcast.
#[tokio::test]
async fn clipboard_set_denied_when_allow_clipboard_write_is_off() {
    let server = Arc::new(StreamingServer::new(
        Arc::new(RwLock::new(Terminal::new(80, 24))),
        "127.0.0.1:0".to_string(),
    ));
    let session = server.sessions.get("default").unwrap();
    session.terminal.write().set_allow_clipboard_write(false);
    let mut rx = session.broadcast_tx.subscribe();

    let replies = server.handle_clipboard_request(
        &session,
        ConnCtx {
            transport_label: "t",
            client_id: uuid::Uuid::new_v4(),
            read_only: false,
        },
        "set".to_string(),
        Some("hi".to_string()),
        None,
    );

    assert!(replies.is_empty());
    assert_eq!(session.terminal.read().clipboard(), None);
    assert!(rx.try_recv().is_err());
}

/// A `SnapshotRequest` must come back as a `SemanticSnapshot` reply whose
/// payload carries the scoped content, and an unknown scope must come back
/// as an `Error` naming the valid scopes.
#[tokio::test]
async fn snapshot_request_replies_with_semantic_snapshot() {
    use crate::streaming::protocol::ClientMessage;
    use crate::terminal::semantic_snapshot::SemanticSnapshot;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    terminal.write().process(b"hello snapshot\r\n".as_slice());
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
    let session = server.get_session("default").expect("default session");

    let dispatch = |msg: ClientMessage| async {
        server
            .handle_client_message(
                &session,
                super::ConnCtx {
                    transport_label: "ws-test",
                    client_id: uuid::Uuid::new_v4(),
                    read_only: false,
                },
                &mut None,
                &mut None,
                msg,
            )
            .await
    };

    for scope in ["visible", "recent", "full"] {
        let replies = dispatch(ClientMessage::SnapshotRequest {
            scope: scope.to_string(),
            max_commands: if scope == "recent" { Some(3) } else { None },
        })
        .await;
        let snapshot_json = match replies.as_slice() {
            [ServerMessage::SemanticSnapshot { snapshot_json }] => snapshot_json.clone(),
            other => panic!("scope {scope}: expected one SemanticSnapshot reply, got {other:?}"),
        };
        let snap: SemanticSnapshot = serde_json::from_str(&snapshot_json)
            .unwrap_or_else(|e| panic!("scope {scope}: bad snapshot JSON: {e}"));
        assert!(
            snap.visible_text.contains("hello snapshot"),
            "scope {scope}: visible_text must carry the screen content"
        );
    }

    let replies = dispatch(ClientMessage::SnapshotRequest {
        scope: "bogus".to_string(),
        max_commands: None,
    })
    .await;
    let message = match replies.as_slice() {
        [ServerMessage::Error { message, .. }] => message.clone(),
        other => panic!("invalid scope: expected one Error reply, got {other:?}"),
    };
    assert!(message.contains("visible"));
}

/// `PaneCreateRequest` against a default-configured server is refused
/// identically whether the connection is writable or read-only: the
/// feature is off (`allow_pane_create` defaults to false), and the
/// disabled gate is the least-revealing answer, so it fires before
/// anything else is probed.
#[tokio::test]
async fn pane_create_is_refused_when_the_flag_is_off() {
    use crate::streaming::protocol::ClientMessage;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
    let session = server.get_session("default").expect("default session");

    let dispatch = |read_only: bool, msg: ClientMessage| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only,
                    },
                    &mut None,
                    &mut None,
                    msg,
                )
                .await
        }
    };
    let request = || ClientMessage::PaneCreateRequest {
        correlation_id: "corr-1".to_string(),
        session_id: "$0".to_string(),
        name: None,
    };

    for read_only in [false, true] {
        match dispatch(read_only, request()).await.as_slice() {
            [ServerMessage::Error { message, code }] => {
                assert_eq!(
                    message, "pane creation is disabled on this server",
                    "default-off refusal message must be exact (read_only={read_only})"
                );
                assert_eq!(
                    code.as_deref(),
                    Some("pane_create_disabled"),
                    "default-off refusal code must be exact (read_only={read_only})"
                );
            }
            other => panic!("expected one pane_create_disabled refusal, got {other:?}"),
        }
    }
}

/// With `allow_pane_create` on, the gate still holds where creation must
/// not happen: a read-only connection is refused `pane_create_read_only`
/// regardless of the flag, and a writable connection with no mux-backed
/// factory is refused `pane_create_no_daemon`.
#[tokio::test]
async fn pane_create_is_refused_read_only_and_without_a_daemon_when_enabled() {
    use crate::streaming::protocol::ClientMessage;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::with_config(
        terminal,
        "127.0.0.1:0".to_string(),
        crate::streaming::StreamingConfig {
            allow_pane_create: true,
            ..Default::default()
        },
    ));
    let session = server.get_session("default").expect("default session");

    let dispatch = |read_only: bool, msg: ClientMessage| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only,
                    },
                    &mut None,
                    &mut None,
                    msg,
                )
                .await
        }
    };
    let request = || ClientMessage::PaneCreateRequest {
        correlation_id: "corr-2".to_string(),
        session_id: "$0".to_string(),
        name: None,
    };

    match dispatch(true, request()).await.as_slice() {
        [ServerMessage::Error { code, .. }] => {
            assert_eq!(
                code.as_deref(),
                Some("pane_create_read_only"),
                "read-only refusal regardless of the flag"
            );
        }
        other => panic!("expected the pane_create_read_only refusal, got {other:?}"),
    }

    match dispatch(false, request()).await.as_slice() {
        [ServerMessage::Error { code, .. }] => {
            assert_eq!(
                code.as_deref(),
                Some("pane_create_no_daemon"),
                "no mux factory behind the session"
            );
        }
        other => panic!("expected the pane_create_no_daemon refusal, got {other:?}"),
    }
}

/// Card 01a1d0d5 criterion 1: an optional name rides the request, but the
/// streamer rejects one over the byte bound or carrying control characters
/// with `pane_create_failed` BEFORE any daemon path is probed (the
/// validation sits between the read-only rung and the no-daemon rung).
#[tokio::test]
async fn pane_create_rejects_names_over_the_bound_or_with_control_characters() {
    use crate::streaming::protocol::ClientMessage;

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::with_config(
        terminal,
        "127.0.0.1:0".to_string(),
        crate::streaming::StreamingConfig {
            allow_pane_create: true,
            ..Default::default()
        },
    ));
    let session = server.get_session("default").expect("default session");

    let dispatch = |name: Option<String>| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only: false,
                    },
                    &mut None,
                    &mut None,
                    ClientMessage::PaneCreateRequest {
                        correlation_id: "corr-3".to_string(),
                        session_id: "$0".to_string(),
                        name,
                    },
                )
                .await
        }
    };

    for (label, name) in [
        ("over-bound", Some("x".repeat(65))),
        ("control char", Some("bad\nname".to_string())),
    ] {
        match dispatch(name).await.as_slice() {
            [ServerMessage::Error { message, code }] => {
                assert_eq!(code.as_deref(), Some("pane_create_failed"), "{label}");
                assert!(
                    message.starts_with("invalid pane name"),
                    "{label}: {message}"
                );
            }
            other => panic!("{label}: expected pane_create_failed, got {other:?}"),
        }
    }

    // A name at exactly the bound is accepted past validation (reaches the
    // no-daemon rung on this factory-less server).
    match dispatch(Some("x".repeat(64))).await.as_slice() {
        [ServerMessage::Error { code, .. }] => {
            assert_eq!(code.as_deref(), Some("pane_create_no_daemon"));
        }
        other => panic!("expected pane_create_no_daemon at the bound, got {other:?}"),
    }
}

// ─── Mouse move normalization (raw SGR button bits) ─────────────────────

/// The (PTY writer, capture buffer) pair [`capture_writer`] hands back.
type CapturedWriter = (
    std::sync::Arc<parking_lot::Mutex<Box<dyn std::io::Write + Send>>>,
    std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
);

/// A PTY writer backed by a shared buffer so a test can assert the exact
/// bytes the serialized input path delivered, in order.
fn capture_writer() -> CapturedWriter {
    let captured = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<u8>::new()));
    let writer = std::sync::Arc::new(parking_lot::Mutex::new(Box::new(CaptureWriter {
        buf: std::sync::Arc::clone(&captured),
    })
        as Box<dyn std::io::Write + Send>));
    (writer, captured)
}

/// The `Box<dyn Write + Send>` payload behind [`capture_writer`]: appends
/// every write into the shared buffer.
struct CaptureWriter {
    buf: std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buf.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Poll until the captured PTY bytes end with `needle`; returns the buffer.
fn wait_for_writer_bytes(
    captured: &std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
    needle: &[u8],
) -> Vec<u8> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let buf = captured.lock().clone();
        if buf.ends_with(needle) {
            return buf;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "PTY writer never received {needle:?}; have {:?}",
            captured.lock(),
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

async fn dispatch_mouse(
    server: &std::sync::Arc<StreamingServer>,
    session: &std::sync::Arc<StreamSessionState>,
    read_only: bool,
    button: u8,
    event_type: crate::streaming::protocol::MouseEventType,
) {
    let mut subscriptions = None;
    let mut rate_limiter = None;
    server
        .handle_client_message(
            session,
            super::ConnCtx {
                transport_label: "ws-test",
                client_id: uuid::Uuid::new_v4(),
                read_only,
            },
            &mut subscriptions,
            &mut rate_limiter,
            crate::streaming::protocol::ClientMessage::Mouse {
                col: 1,
                row: 1,
                button,
                shift: false,
                ctrl: false,
                alt: false,
                event_type,
            },
        )
        .await;
}

/// A server session with SGR any-event tracking armed and a capturing PTY
/// writer attached, ready to receive mouse input.
fn mouse_session() -> (
    std::sync::Arc<StreamingServer>,
    std::sync::Arc<StreamSessionState>,
    std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
) {
    let server = Arc::new(StreamingServer::new(
        Arc::new(RwLock::new(Terminal::new(80, 24))),
        "127.0.0.1:0".to_string(),
    ));
    let session = server.get_session("default").expect("default session");
    {
        let mut terminal = session.terminal.write();
        terminal.set_mouse_mode(crate::mouse::MouseMode::AnyEvent);
        terminal.set_mouse_encoding(crate::mouse::MouseEncoding::Sgr);
    }
    let (writer, captured) = capture_writer();
    session.set_pty_writer(writer);
    (server, session, captured)
}

/// A logical `move` (button 0 left-held, 3 buttonless) and an already-raw
/// one (32/35) all encode as SGR motion reports `<32` / `<35`; modifiers
/// keep ORing their bits (shift adds 4) on top of the motion bit.
#[tokio::test]
async fn mouse_move_normalizes_logical_button_to_raw_sgr_bits() {
    use crate::streaming::protocol::MouseEventType;

    let (server, session, captured) = mouse_session();

    for (button, expected) in [
        (0u8, &b"\x1b[<32;2;2M"[..]),
        (3, &b"\x1b[<35;2;2M"[..]),
        (32, &b"\x1b[<32;2;2M"[..]),
        (35, &b"\x1b[<35;2;2M"[..]),
    ] {
        dispatch_mouse(&server, &session, false, button, MouseEventType::Move).await;
        wait_for_writer_bytes(&captured, expected);
    }
}

/// `press`/`release` pass the button through verbatim in both spellings —
/// the logical 0-press / 3-release pair and raw values — and a raw press
/// code (the motion bit set) is not rewritten.
#[tokio::test]
async fn mouse_press_and_release_buttons_stay_verbatim() {
    use crate::streaming::protocol::MouseEventType;

    let (server, session, captured) = mouse_session();

    for (button, event_type, expected) in [
        (0u8, MouseEventType::Press, &b"\x1b[<0;2;2M"[..]),
        (3, MouseEventType::Release, &b"\x1b[<3;2;2m"[..]),
        (1, MouseEventType::Press, &b"\x1b[<1;2;2M"[..]),
        (2, MouseEventType::Press, &b"\x1b[<2;2;2M"[..]),
        (32, MouseEventType::Press, &b"\x1b[<32;2;2M"[..]),
    ] {
        dispatch_mouse(&server, &session, false, button, event_type).await;
        wait_for_writer_bytes(&captured, expected);
    }
}

/// A read-only viewer's `move` never reaches the serialized input path:
/// nothing is enqueued, and the silence is not counted as a drop.
#[tokio::test]
async fn read_only_mouse_move_sends_nothing() {
    use crate::streaming::protocol::MouseEventType;

    let (server, session, captured) = mouse_session();

    dispatch_mouse(&server, &session, true, 0, MouseEventType::Move).await;
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        captured.lock().is_empty(),
        "read-only move must not enqueue PTY bytes"
    );
    assert_eq!(
        session.metrics.dropped_messages.load(Ordering::Relaxed),
        0,
        "read-only silence is not a drop"
    );
}

/// The normalized `move` rides the same serialized input path as
/// keystrokes (QA-110): one blocking drain task writes queue order, so a
/// move flanked by keystrokes cannot overtake or trail them.
#[tokio::test]
async fn mouse_move_rides_the_serialized_input_path_with_keystrokes() {
    use crate::streaming::protocol::{ClientMessage, MouseEventType};

    let (server, session, captured) = mouse_session();
    let dispatch = |msg: ClientMessage| {
        let server = Arc::clone(&server);
        let session = Arc::clone(&session);
        async move {
            server
                .handle_client_message(
                    &session,
                    super::ConnCtx {
                        transport_label: "ws-test",
                        client_id: uuid::Uuid::new_v4(),
                        read_only: false,
                    },
                    &mut None,
                    &mut None,
                    msg,
                )
                .await
        }
    };

    dispatch(ClientMessage::Input {
        data: "q".to_string(),
    })
    .await;
    dispatch_mouse(&server, &session, false, 0, MouseEventType::Move).await;
    dispatch(ClientMessage::Input {
        data: "x".to_string(),
    })
    .await;

    wait_for_writer_bytes(&captured, b"q\x1b[<32;2;2Mx");
}

// ─── InputRateLimiter tests ─────────────────────────────────────────────
