//! HTTP and WebSocket handshake plumbing: WS accept config and header
//! callback, API auth middleware, origin/CORS checks, security headers,
//! handshake auth, and the axum route handlers.

use super::*;

/// Build the [`WebSocketConfig`] applied to every WS acceptor.
pub(super) fn ws_accept_config() -> Option<WebSocketConfig> {
    // `WebSocketConfig` is `#[non_exhaustive]`, so use the builder API rather
    // than a struct literal.
    Some(
        WebSocketConfig::default()
            .max_message_size(Some(WS_MAX_MESSAGE_SIZE))
            .max_frame_size(Some(WS_MAX_FRAME_SIZE)),
    )
}

/// Build the WebSocket handshake header callback shared by the plain
/// (`start_websocket_only`) and TLS (`start_websocket_only_tls`) listeners.
///
/// The callback captures the request's URI query string into the returned
/// `Arc<Mutex<..>>` (read back by the caller after the handshake completes),
/// then enforces two handshake-time checks in order:
/// 1. Origin allowlist (SEC-005: CSRF-via-WebSocket defense) via
///    [`check_ws_origin`].
/// 2. API-key / HTTP Basic auth (if configured) via
///    [`validate_ws_handshake_auth`].
///
/// Both listeners MUST use this single factory so a future auth/origin fix
/// only needs to change one place instead of two.
///
/// The tungstenite `Callback` trait fixes `ErrorResponse` as
/// `HttpResponse<Option<String>>` — we cannot box or shrink it without
/// violating the external API contract.
#[allow(clippy::type_complexity, clippy::result_large_err)]
pub(super) fn build_ws_header_callback(
    api_key: Option<String>,
    basic_auth: Option<HttpBasicAuthConfig>,
    allowed_origins: Option<Vec<String>>,
    allow_api_key_in_query: bool,
) -> (
    impl FnOnce(
        &WsHandshakeRequest,
        WsHandshakeResponse,
    ) -> std::result::Result<WsHandshakeResponse, WsHandshakeErrorResponse>,
    Arc<Mutex<Option<String>>>,
) {
    let uri_query = Arc::new(Mutex::new(None::<String>));
    let uri_query_clone = Arc::clone(&uri_query);

    let callback = move |req: &WsHandshakeRequest,
                         resp: WsHandshakeResponse|
          -> std::result::Result<WsHandshakeResponse, WsHandshakeErrorResponse> {
        if let Some(q) = req.uri().query() {
            *uri_query_clone.lock() = Some(q.to_string());
        }

        // Validate Origin header (SEC-005: CSRF-via-WebSocket defense)
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if !check_ws_origin(origin, allowed_origins.as_deref()) {
            let reject = tokio_tungstenite::tungstenite::http::Response::builder()
                .status(403)
                .body(Some("Origin not allowed".to_string()))
                .expect("static rejection response body is always valid");
            return Err(reject);
        }

        // Validate auth if configured
        if (api_key.is_some() || basic_auth.is_some())
            && !validate_ws_handshake_auth(
                req,
                api_key.as_deref(),
                basic_auth.as_ref(),
                allow_api_key_in_query,
            )
        {
            let reject = tokio_tungstenite::tungstenite::http::Response::builder()
                .status(401)
                .body(Some("Unauthorized".to_string()))
                .expect("static rejection response body is always valid");
            return Err(reject);
        }

        Ok(resp)
    };

    (callback, uri_query)
}

/// Unified API authentication middleware for Axum.
/// Checks in order: Bearer header → X-API-Key header → ?api_key= query → Basic Auth header.
/// When both API key and Basic Auth are configured, either one satisfies auth.
#[cfg(feature = "streaming")]
pub(super) async fn api_auth_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
    auth_config: ApiAuthConfig,
) -> axum::response::Response {
    use axum::http::{header, HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    use subtle::ConstantTimeEq;

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let x_api_key_header = req
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Check Bearer token against API key
    if let Some(ref expected_key) = auth_config.api_key {
        if let Some(ref auth_value) = auth_header {
            if let Some(bearer_token) = auth_value.strip_prefix("Bearer ") {
                if bool::from(
                    bearer_token
                        .trim()
                        .as_bytes()
                        .ct_eq(expected_key.as_bytes()),
                ) {
                    return next.run(req).await;
                }
            }
        }

        // Check X-API-Key header
        if let Some(ref key) = x_api_key_header {
            if bool::from(key.as_bytes().ct_eq(expected_key.as_bytes())) {
                return next.run(req).await;
            }
        }

        // Check ?api_key= query param (only if explicitly allowed)
        if auth_config.allow_api_key_in_query {
            if let Some(query) = req.uri().query() {
                for pair in query.split('&') {
                    if let Some(value) = pair.strip_prefix("api_key=") {
                        if bool::from(value.as_bytes().ct_eq(expected_key.as_bytes())) {
                            return next.run(req).await;
                        }
                    }
                }
            }
        }
    }

    // Check HTTP Basic Auth
    if let Some(ref basic_config) = auth_config.http_basic_auth {
        if let Some(ref auth_value) = auth_header {
            if let Some(credentials) = auth_value.strip_prefix("Basic ") {
                if let Ok(decoded) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    credentials.trim(),
                ) {
                    if let Ok(credentials_str) = String::from_utf8(decoded) {
                        if let Some((username, password)) = credentials_str.split_once(':') {
                            if basic_config.verify(username, password) {
                                return next.run(req).await;
                            }
                        }
                    }
                }
            }
        }
    }

    // Build 401 response
    let mut response = (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    if auth_config.http_basic_auth.is_some() {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Terminal Server\""),
        );
    }
    response
}

/// Validate auth credentials during WebSocket handshake (for non-HTTP server modes).
/// Checks Bearer header → X-API-Key header → ?api_key= query (if allowed) → Basic Auth header.
/// Returns true if auth passes (or no auth is configured).
/// Check a WebSocket / HTTP request's `Origin` header against the security
/// policy (SEC-005, CSRF-via-WebSocket defense).
///
/// Rules:
/// - No `Origin` header (non-browser client: curl, native TUI, the embedded
///   library) → always allowed.
/// - `Origin` present + `allowed_origins` configured → allowed only if the
///   origin exactly matches an entry in the list.
/// - `Origin` present + no allowlist (default) → allowed only if the origin's
///   host is local (`localhost` / `127.0.0.1` / `::1`), blocking remote browser
///   origins (e.g. a malicious page on `evil.com`) from driving the PTY.
pub(super) fn check_ws_origin(origin: Option<&str>, allowed_origins: Option<&[String]>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    match allowed_origins {
        Some(list) => list.iter().any(|o| o == origin),
        None => is_local_origin(origin),
    }
}

/// True if `origin` (e.g. `http://localhost:8099`) points at a loopback host.
pub(super) fn is_local_origin(origin: &str) -> bool {
    let after_scheme = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = host_of_authority(authority);
    let h = host.to_ascii_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "::1"
}

/// Extract the host portion of a URL authority, stripping the port.
/// Handles IPv6 literals (`[::1]:8099` → `::1`).
pub(super) fn host_of_authority(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if let Some((host, _port)) = authority.rsplit_once(':') {
        host
    } else {
        authority
    }
}

/// Build a CORS layer for the HTTP server reflecting the `allowed_origins`
/// policy (SEC-005). When an allowlist is configured, only those origins may
/// make cross-origin browser requests; otherwise only local (loopback)
/// origins are allowed, mirroring the WebSocket default of `check_ws_origin`
/// (SEC-001).
pub(super) fn build_cors_layer(
    allowed_origins: &Option<Vec<String>>,
) -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use tower_http::cors::{AllowOrigin, Any, CorsLayer};
    match allowed_origins {
        Some(list) if !list.is_empty() => {
            let origins: Vec<HeaderValue> = list.iter().filter_map(|o| o.parse().ok()).collect();
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(Any)
                .allow_headers(Any)
        }
        // SEC-001: without an allowlist, deny remote browser origins the
        // ability to read HTTP responses cross-origin (no ACAO header),
        // matching what the WebSocket handlers reject outright.
        _ => CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(|origin, _| {
                origin.to_str().map(is_local_origin).unwrap_or(false)
            }))
            .allow_methods(Any)
            .allow_headers(Any),
    }
}

/// Anti-framing headers on every response (SEC-010). The served web
/// frontend drives a live shell; without `X-Frame-Options: DENY` and a
/// `frame-ancestors 'none'` CSP, a transparent iframe on an attacker page
/// plus cached Basic credentials lets the attacker clickjack keystrokes
/// into the terminal. Applied at the outermost router so static files and
/// API responses both carry it.
pub(super) async fn add_security_headers(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{HeaderName, HeaderValue};
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    res
}

pub(super) fn validate_ws_handshake_auth(
    req: &tokio_tungstenite::tungstenite::http::Request<()>,
    api_key: Option<&str>,
    basic_auth: Option<&HttpBasicAuthConfig>,
    allow_api_key_in_query: bool,
) -> bool {
    use subtle::ConstantTimeEq;

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let x_api_key_header = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

    // Check API key via Bearer header
    if let Some(expected_key) = api_key {
        if let Some(auth_value) = auth_header {
            if let Some(bearer_token) = auth_value.strip_prefix("Bearer ") {
                if bool::from(
                    bearer_token
                        .trim()
                        .as_bytes()
                        .ct_eq(expected_key.as_bytes()),
                ) {
                    return true;
                }
            }
        }

        // Check X-API-Key header
        if let Some(key) = x_api_key_header {
            if bool::from(key.as_bytes().ct_eq(expected_key.as_bytes())) {
                return true;
            }
        }

        // Check ?api_key= query param (only if explicitly allowed)
        if allow_api_key_in_query {
            if let Some(query) = req.uri().query() {
                for pair in query.split('&') {
                    if let Some(value) = pair.strip_prefix("api_key=") {
                        if bool::from(value.as_bytes().ct_eq(expected_key.as_bytes())) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    // Check HTTP Basic Auth
    if let Some(basic_config) = basic_auth {
        if let Some(auth_value) = auth_header {
            if let Some(credentials) = auth_value.strip_prefix("Basic ") {
                if let Ok(decoded) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    credentials.trim(),
                ) {
                    if let Ok(credentials_str) = String::from_utf8(decoded) {
                        if let Some((username, password)) = credentials_str.split_once(':') {
                            if basic_config.verify(username, password) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// Axum WebSocket handler (extracts query params for multi-session)
#[cfg(feature = "streaming")]
pub(super) async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    // SEC-005: reject browser connections whose Origin is not allowed.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if !check_ws_origin(origin, server.config.allowed_origins.as_deref()) {
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }

    let params = ConnectionParams::from_query(&query);
    ws.max_message_size(WS_MAX_MESSAGE_SIZE)
        .max_frame_size(WS_MAX_FRAME_SIZE)
        .on_upgrade(move |socket| async move {
            if let Err(e) = server.handle_axum_websocket(socket, params).await {
                crate::debug_error!("STREAMING", "WebSocket handler error: {}", e);
            }
        })
        .into_response()
}

/// Sessions list HTTP handler
#[cfg(feature = "streaming")]
pub(super) async fn sessions_handler(
    headers: axum::http::HeaderMap,
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    // SEC-002: reject browser requests whose Origin is not allowed; the
    // session list exposes live session ids (same guard as the WS handlers).
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if !check_ws_origin(origin, server.config.allowed_origins.as_deref()) {
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }

    let sessions = server.sessions.list_sessions();
    let max = server.config.max_sessions;
    let available = max.saturating_sub(sessions.len());
    axum::Json(serde_json::json!({
        "sessions": sessions,
        "max_sessions": max,
        "available": available,
    }))
    .into_response()
}

/// System stats WebSocket handler
#[cfg(feature = "streaming")]
pub(super) async fn stats_ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::Query(_query): axum::extract::Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    // Check if system stats are enabled
    if !server.config.enable_system_stats {
        return (StatusCode::NOT_FOUND, "System stats not enabled").into_response();
    }

    // SEC-005: reject browser connections whose Origin is not allowed.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if !check_ws_origin(origin, server.config.allowed_origins.as_deref()) {
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }

    // Note: API key auth is handled by the basic_auth middleware if configured
    let interval_secs = server.config.system_stats_interval_secs.max(1);

    ws.max_message_size(WS_MAX_MESSAGE_SIZE)
        .max_frame_size(WS_MAX_FRAME_SIZE)
        .on_upgrade(move |socket| async move {
            if let Err(e) = handle_stats_websocket(socket, interval_secs).await {
                crate::debug_error!("STREAMING", "Stats WebSocket error: {}", e);
            }
        })
        .into_response()
}

/// Handle stats-only WebSocket connection
#[cfg(feature = "streaming")]
pub(super) async fn handle_stats_websocket(
    socket: axum::extract::ws::WebSocket,
    interval_secs: u64,
) -> Result<()> {
    use axum::extract::ws::Message as AxumMessage;
    use futures_util::{SinkExt, StreamExt};
    use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind};

    let (mut sender, mut receiver) = socket.split();

    let refresh_kind = RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::everything())
        .with_memory(MemoryRefreshKind::everything());
    let mut sys = sysinfo::System::new_with_specifics(refresh_kind);
    let mut disks = Disks::new_with_refreshed_list();
    let mut networks = Networks::new_with_refreshed_list();

    // Collect static info once
    let hostname = sysinfo::System::host_name();
    let os_name = sysinfo::System::name();
    let os_version = sysinfo::System::os_version();
    let kernel_version = sysinfo::System::kernel_version();

    // Initial CPU refresh for baseline
    sys.refresh_specifics(refresh_kind);

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    interval.tick().await; // Skip first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Refresh all metrics
                sys.refresh_specifics(refresh_kind);
                disks.refresh(true);
                networks.refresh(true);

                // Build stats JSON
                let stats = serde_json::json!({
                    "cpu": {
                        "overall_usage_percent": sys.global_cpu_usage() as f64,
                        "physical_core_count": sysinfo::System::physical_core_count().unwrap_or(0),
                        "per_core_usage_percent": sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect::<Vec<_>>(),
                        "brand": sys.cpus().first().map(|c| c.brand().to_string()),
                        "frequency_mhz": sys.cpus().first().map(|c| c.frequency()),
                    },
                    "memory": {
                        "total_bytes": sys.total_memory(),
                        "used_bytes": sys.used_memory(),
                        "available_bytes": sys.available_memory(),
                        "swap_total_bytes": sys.total_swap(),
                        "swap_used_bytes": sys.used_swap(),
                    },
                    "disks": disks.iter().map(|d| serde_json::json!({
                        "name": d.name().to_string_lossy(),
                        "mount_point": d.mount_point().to_string_lossy(),
                        "total_bytes": d.total_space(),
                        "available_bytes": d.available_space(),
                        "kind": format!("{:?}", d.kind()),
                        "file_system": d.file_system().to_string_lossy(),
                        "is_removable": d.is_removable(),
                    })).collect::<Vec<_>>(),
                    "networks": networks.iter().map(|(name, data)| serde_json::json!({
                        "name": name,
                        "received_bytes": data.received(),
                        "transmitted_bytes": data.transmitted(),
                        "total_received_bytes": data.total_received(),
                        "total_transmitted_bytes": data.total_transmitted(),
                        "packets_received": data.packets_received(),
                        "packets_transmitted": data.packets_transmitted(),
                        "errors_received": data.errors_on_received(),
                        "errors_transmitted": data.errors_on_transmitted(),
                    })).collect::<Vec<_>>(),
                    "load_average": {
                        "one_minute": sysinfo::System::load_average().one,
                        "five_minutes": sysinfo::System::load_average().five,
                        "fifteen_minutes": sysinfo::System::load_average().fifteen,
                    },
                    "hostname": hostname,
                    "os_name": os_name,
                    "os_version": os_version,
                    "kernel_version": kernel_version,
                    "uptime_secs": sysinfo::System::uptime(),
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                });

                let json = serde_json::to_string(&stats).unwrap_or_default();
                if sender.send(AxumMessage::Text(json.into())).await.is_err() {
                    break; // Client disconnected
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(AxumMessage::Close(_))) | None => break,
                    Some(Ok(AxumMessage::Ping(data))) => {
                        let _ = sender.send(AxumMessage::Pong(data)).await;
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }

    Ok(())
}
