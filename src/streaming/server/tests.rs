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
    // Control: a filtered event type is still dropped.
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
use crate::terminal::Terminal;

#[tokio::test]
async fn test_output_sender() {
    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());

    let tx = server.get_output_sender();
    assert!(tx.try_send("test".to_string()).is_ok());
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
        server.handle_client_message(
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
        assert!(dispatch(false, msg).is_empty(), "input produces no reply");
        assert_eq!(
            session.metrics.dropped_messages.load(Ordering::Relaxed),
            i + 1
        );
    }

    // Non-input messages on the same writer-less session still reply.
    let replies = dispatch(false, ClientMessage::Ping);
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
    );
    assert!(replies.is_empty());
    assert_eq!(
        session.metrics.dropped_messages.load(Ordering::Relaxed),
        4,
        "read-only input is not a writer-detached drop"
    );
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
/// same way `build_http_app` wires it) and return the response.
async fn unauthenticated_response(
    auth_config: ApiAuthConfig,
) -> axum::http::Response<axum::body::Body> {
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    let app = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(move |req, next| {
            let auth_config = auth_config.clone();
            api_auth_middleware(req, next, auth_config)
        }));
    let req = axum::http::Request::builder()
        .uri("/ok")
        .body(axum::body::Body::empty())
        .expect("static request parts are valid");
    app.oneshot(req).await.expect("router is infallible")
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

// ─── InputRateLimiter tests ─────────────────────────────────────────────
