//! WebSocket streaming server implementation

use crate::streaming::client::Client;
use crate::streaming::config::{ApiAuthConfig, HttpBasicAuthConfig, StreamingConfig};
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::proto::{decode_client_message, encode_server_message};
use crate::streaming::protocol::{ServerMessage, ThemeInfo};
use crate::streaming::rate_limit::InputRateLimiter;
use crate::streaming::session::{now_millis, SessionRegistry, StreamSessionState};
use crate::terminal::{SelectionMode, Terminal};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// TLS/SSL configuration for secure connections
///
/// Supports loading certificates and keys from files (PEM or DER format).
/// For PEM files, you can provide a combined certificate chain or separate files.
///
/// # Examples
///
/// ```rust,no_run
/// use par_term_emu_core_rust::streaming::TlsConfig;
///
/// // Using separate certificate and key files
/// let tls = TlsConfig::from_files("cert.pem", "key.pem").unwrap();
///
/// // Using a combined PEM file (certificate + key in one file)
/// let tls = TlsConfig::from_pem("combined.pem").unwrap();
/// ```
///
/// # WebSocket size limits
///
/// Inbound WebSocket frames are capped at [`WS_MAX_MESSAGE_SIZE`] /
/// [`WS_MAX_FRAME_SIZE`] bytes (16 MiB each). This is well above any
/// legitimate terminal streaming frame but far below tungstenite's 64 MiB
/// default, limiting the blast radius of a malicious or buggy client.
const WS_MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
const WS_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Request/response types for the tungstenite WS handshake header callback,
/// aliased for readability (the `Callback` trait fixes these exactly).
type WsHandshakeRequest = tokio_tungstenite::tungstenite::http::Request<()>;
type WsHandshakeResponse = tokio_tungstenite::tungstenite::http::Response<()>;
type WsHandshakeErrorResponse = tokio_tungstenite::tungstenite::http::Response<Option<String>>;

/// Build the [`WebSocketConfig`] applied to every WS acceptor.
fn ws_accept_config() -> Option<WebSocketConfig> {
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
fn build_ws_header_callback(
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
// =============================================================================
// Terminal Size Validation
// =============================================================================

/// Minimum terminal columns
pub const MIN_COLS: u16 = 2;
/// Minimum terminal rows
pub const MIN_ROWS: u16 = 1;
/// Maximum terminal columns
pub const MAX_COLS: u16 = 1000;
/// Maximum terminal rows
pub const MAX_ROWS: u16 = 500;

/// Validate terminal size is within acceptable bounds
pub fn validate_terminal_size(cols: u16, rows: u16) -> Result<(u16, u16)> {
    if !(MIN_COLS..=MAX_COLS).contains(&cols) || !(MIN_ROWS..=MAX_ROWS).contains(&rows) {
        return Err(StreamingError::InvalidInput(format!(
            "Terminal size {}x{} out of range ({}-{}x{}-{})",
            cols, rows, MIN_COLS, MAX_COLS, MIN_ROWS, MAX_ROWS
        )));
    }
    Ok((cols, rows))
}

// =============================================================================
// Session Factory
// =============================================================================

/// Result returned by SessionFactory::create_session
pub struct SessionFactoryResult {
    /// The terminal instance for the new session
    pub terminal: Arc<RwLock<Terminal>>,
    /// Optional PTY writer for the new session
    pub pty_writer: Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>>,
}

/// Trait for creating new sessions on demand
///
/// Implement this trait to customize how sessions are created (e.g., spawning
/// PTY processes, configuring terminals, etc.)
pub trait SessionFactory: Send + Sync {
    /// Create a new session with the given parameters
    ///
    /// # Arguments
    /// * `session_id` - Unique identifier for the session
    /// * `cols` - Terminal columns
    /// * `rows` - Terminal rows
    /// * `shell_command` - Optional shell command (from preset resolution)
    fn create_session(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
        shell_command: Option<&str>,
    ) -> std::result::Result<SessionFactoryResult, StreamingError>;

    /// Setup a session after creation (e.g., spawn background tasks)
    fn setup_session(
        &self,
        session_id: &str,
        session: &Arc<StreamSessionState>,
    ) -> std::result::Result<(), StreamingError>;

    /// Teardown a session (e.g., kill PTY process)
    fn teardown_session(&self, session_id: &str);

    /// Check if a session's backing process is still alive
    fn is_session_alive(&self, _session_id: &str) -> bool {
        true
    }
}

// =============================================================================
// Connection Parameters
// =============================================================================

/// Parsed connection parameters from URL query string
pub struct ConnectionParams {
    /// Session ID (defaults to "default")
    pub session_id: String,
    /// Whether this connection is read-only
    pub readonly: bool,
    /// Preset name to use for session creation
    pub preset: Option<String>,
}

impl ConnectionParams {
    /// Parse connection parameters from a query string map
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let session_id = params
            .get("session")
            .cloned()
            .unwrap_or_else(|| "default".to_string());
        let readonly = params
            .get("readonly")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let preset = params.get("preset").cloned();

        Self {
            session_id,
            readonly,
            preset,
        }
    }

    /// Parse connection parameters from a URI query string
    pub fn from_uri_query(query: Option<&str>) -> Self {
        let params: HashMap<String, String> = query
            .unwrap_or("")
            .split('&')
            .filter(|s| !s.is_empty())
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?.to_string();
                let value = parts.next().unwrap_or("").to_string();
                Some((key, value))
            })
            .collect();

        Self::from_query(&params)
    }
}
// =============================================================================
// Guards
// =============================================================================

/// Guard that decrements session client count when dropped
struct SessionClientGuard {
    session: Arc<StreamSessionState>,
}

impl Drop for SessionClientGuard {
    fn drop(&mut self) {
        self.session.remove_client();
    }
}

/// Guard that decrements global client count when dropped
struct GlobalClientGuard<'a> {
    server: &'a StreamingServer,
}

impl<'a> Drop for GlobalClientGuard<'a> {
    fn drop(&mut self) {
        self.server.remove_client();
    }
}

// =============================================================================
// Streaming Server
// =============================================================================

/// WebSocket streaming server for terminal sessions
pub struct StreamingServer {
    /// Atomic counter for tracking total connected clients across all sessions
    client_count: AtomicUsize,
    /// Server bind address
    addr: String,
    /// Server configuration
    config: StreamingConfig,
    /// Registry of active sessions
    sessions: SessionRegistry,
    /// Factory for creating new sessions on demand
    session_factory: Option<Arc<dyn SessionFactory>>,
    /// Optional theme information to send to clients
    theme: Option<ThemeInfo>,
    /// Global shutdown signal
    shutdown: Arc<tokio::sync::Notify>,
    /// The default session (for backward-compatible single-session mode)
    default_session: Option<Arc<StreamSessionState>>,
}

impl StreamingServer {
    /// Create a new streaming server (backward-compatible single-session mode)
    pub fn new(terminal: Arc<RwLock<Terminal>>, addr: String) -> Self {
        Self::with_config(terminal, addr, StreamingConfig::default())
    }

    /// Create a new streaming server with custom configuration (backward-compatible)
    pub fn with_config(
        terminal: Arc<RwLock<Terminal>>,
        addr: String,
        config: StreamingConfig,
    ) -> Self {
        let sessions = SessionRegistry::new(config.max_sessions);

        // Create default session
        let default_session = Arc::new(StreamSessionState::new(
            "default".to_string(),
            terminal,
            None,
            config.send_initial_screen,
        ));

        // Insert into registry
        let _ = sessions.insert("default".to_string(), Arc::clone(&default_session));

        Self {
            client_count: AtomicUsize::new(0),
            addr,
            config,
            sessions,
            session_factory: None,
            theme: None,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            default_session: Some(default_session),
        }
    }

    /// Create a streaming server with a session factory for multi-session support
    pub fn with_factory(
        addr: String,
        config: StreamingConfig,
        factory: Arc<dyn SessionFactory>,
    ) -> Self {
        let sessions = SessionRegistry::new(config.max_sessions);

        Self {
            client_count: AtomicUsize::new(0),
            addr,
            config,
            sessions,
            session_factory: Some(factory),
            theme: None,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            default_session: None,
        }
    }

    /// Set the theme to be sent to clients on connection
    pub fn set_theme(&mut self, theme: ThemeInfo) {
        self.theme = Some(theme.clone());
        // Also update theme on any existing sessions
        if let Some(ref session) = self.default_session {
            // We can't directly modify the theme on StreamSessionState without interior mutability,
            // but new sessions created by the factory will pick up the theme from
            // resolve_session. For the default session created in with_config, the theme
            // is set at construction time. Since set_theme is called before start(), we
            // need to recreate the default session with the theme.
            // However, the simplest approach is to store theme on the server and use it
            // when building connect messages from the default session.
            // Theme is used via server.theme in build_connect_message fallback
            let _session = session;
        }
    }

    // -- Backward-compatible single-session accessors --

    /// Set the PTY writer for handling client input (routes to default session)
    pub fn set_pty_writer(&self, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>) {
        if let Some(ref session) = self.default_session {
            session.set_pty_writer(writer);
        }
    }

    /// Get a clone of the output sender channel (routes to default session)
    pub fn get_output_sender(&self) -> mpsc::Sender<String> {
        if let Some(ref session) = self.default_session {
            session.get_output_sender()
        } else {
            // Create a dummy channel that will never be read
            let (tx, _rx) = mpsc::channel(1);
            tx
        }
    }

    /// Get a clone of the resize receiver (routes to default session)
    pub fn get_resize_receiver(
        &self,
    ) -> Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>> {
        if let Some(ref session) = self.default_session {
            session.get_resize_receiver()
        } else {
            let (_tx, rx) = mpsc::unbounded_channel();
            Arc::new(tokio::sync::Mutex::new(rx))
        }
    }

    /// Get the current number of connected clients
    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Get the maximum number of clients allowed
    pub fn max_clients(&self) -> usize {
        self.config.max_clients
    }

    /// Check if the server can accept more clients
    fn can_accept_client(&self) -> bool {
        self.client_count.load(Ordering::Relaxed) < self.config.max_clients
    }

    /// Increment the client count. Returns false if max_clients would be exceeded.
    fn try_add_client(&self) -> bool {
        loop {
            let current = self.client_count.load(Ordering::Relaxed);
            if current >= self.config.max_clients {
                return false;
            }
            match self.client_count.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => continue,
            }
        }
    }

    /// Decrement the client count
    fn remove_client(&self) {
        self.client_count.fetch_sub(1, Ordering::SeqCst);
    }

    /// Broadcast a message to all clients in the default session
    pub fn broadcast(&self, msg: ServerMessage) {
        if let Some(ref session) = self.default_session {
            session.broadcast(msg);
        }
    }

    /// Send a message to a specific session
    pub fn send_to_session(&self, session_id: &str, msg: ServerMessage) {
        if let Some(session) = self.sessions.get(session_id) {
            session.broadcast(msg);
        }
    }

    /// Broadcast a message to all clients of a specific session
    pub fn broadcast_to_session(&self, session_id: &str, msg: ServerMessage) {
        if let Some(session) = self.sessions.get(session_id) {
            let _ = session.broadcast_tx.send(msg);
        } else if let Some(ref session) = self.default_session {
            let _ = session.broadcast_tx.send(msg);
        }
    }

    /// Get a session by ID from the registry
    pub fn get_session(&self, session_id: &str) -> Option<Arc<StreamSessionState>> {
        self.sessions.get(session_id)
    }

    /// Close a session: remove from registry, shut it down, and tear down factory resources.
    /// Factory teardown is delayed 500ms so clients receive the shutdown message.
    pub fn close_session(&self, session_id: &str, reason: String) -> bool {
        if let Some(session) = self.sessions.remove(session_id) {
            session.shutdown(reason);
            if let Some(ref factory) = self.session_factory {
                let factory = Arc::clone(factory);
                let id = session_id.to_string();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    factory.teardown_session(&id);
                });
            }
            crate::debug_info!("STREAMING", "Closed session: {}", session_id);
            true
        } else {
            false
        }
    }

    /// Resolve a session from connection parameters
    ///
    /// 1. If session already exists in registry, return it
    /// 2. If factory is available, create a new session
    /// 3. If no factory and id == "default", return default session
    /// 4. Otherwise, error
    pub fn resolve_session(
        self: &Arc<Self>,
        params: &ConnectionParams,
    ) -> Result<Arc<StreamSessionState>> {
        let session_id = &params.session_id;

        // Check if session already exists
        if let Some(session) = self.sessions.get(session_id) {
            return Ok(session);
        }

        // Try to create via factory
        if let Some(ref factory) = self.session_factory {
            // Resolve shell command from preset if specified
            let shell_command = if let Some(ref preset_name) = params.preset {
                let cmd = self
                    .config
                    .presets
                    .get(preset_name)
                    .ok_or_else(|| StreamingError::InvalidPreset(preset_name.clone()))?;
                Some(cmd.as_str())
            } else {
                None
            };

            // Get terminal size from config or defaults
            let cols = if self.config.initial_cols > 0 {
                self.config.initial_cols
            } else {
                80
            };
            let rows = if self.config.initial_rows > 0 {
                self.config.initial_rows
            } else {
                24
            };

            let (cols, rows) = validate_terminal_size(cols, rows)?;

            let result = factory.create_session(session_id, cols, rows, shell_command)?;

            let session = Arc::new(StreamSessionState::new(
                session_id.clone(),
                result.terminal,
                self.theme.clone(),
                self.config.send_initial_screen,
            ));

            if let Some(writer) = result.pty_writer {
                session.set_pty_writer(writer);
            }

            // Insert into registry
            self.sessions
                .insert(session_id.clone(), Arc::clone(&session))?;

            // Setup session (spawn background tasks, etc.)
            factory.setup_session(session_id, &session)?;

            // Spawn broadcaster loop for this session
            let session_clone = Arc::clone(&session);
            tokio::spawn(async move {
                session_clone.output_broadcaster_loop().await;
            });

            return Ok(session);
        }

        // No factory - check if asking for default
        if session_id == "default" {
            if let Some(ref default) = self.default_session {
                return Ok(Arc::clone(default));
            }
        }

        Err(StreamingError::SessionNotFound(session_id.clone()))
    }

    /// Start the streaming server
    pub async fn start(self: Arc<Self>) -> Result<()> {
        let use_tls = self.config.tls.is_some();

        if self.config.enable_http {
            if use_tls {
                self.start_with_https().await
            } else {
                self.start_with_http().await
            }
        } else if use_tls {
            self.start_websocket_only_tls().await
        } else {
            self.start_websocket_only().await
        }
    }

    /// Spawn the session reaper task (always runs for dead session cleanup)
    fn spawn_idle_reaper(self: &Arc<Self>) {
        let server = Arc::clone(self);
        tokio::spawn(async move {
            server.session_reaper().await;
        });
    }

    /// Session reaper - periodically checks for idle and dead sessions
    async fn session_reaper(self: Arc<Self>) {
        let idle_timeout = if self.config.session_idle_timeout > 0 {
            Some(Duration::from_secs(self.config.session_idle_timeout))
        } else {
            None
        };
        let mut interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            interval.tick().await;

            // Idle timeout reaping (if configured)
            if let Some(timeout) = idle_timeout {
                let idle_ids = self.sessions.idle_sessions(timeout);
                for id in idle_ids {
                    // Allow reaping default in factory mode only
                    if id == "default" && self.session_factory.is_none() {
                        continue;
                    }
                    if self.close_session(&id, "Session idle timeout".to_string()) {
                        crate::debug_info!("STREAMING", "Reaped idle session: {}", id);
                    }
                }
            }

            // Dead session reaping (always)
            self.reap_dead_sessions();

            // Broadcaster health check
            self.check_broadcaster_health();
        }
    }

    /// Reap sessions whose PTY process has exited and have no clients
    fn reap_dead_sessions(&self) {
        if let Some(ref factory) = self.session_factory {
            let session_ids: Vec<String> = self
                .sessions
                .list_sessions()
                .iter()
                .filter(|s| s.clients == 0)
                .map(|s| s.id.clone())
                .collect();
            for id in session_ids {
                if !factory.is_session_alive(&id)
                    && self.close_session(&id, "Dead session (PTY exited)".to_string())
                {
                    crate::debug_info!("STREAMING", "Reaped dead session: {}", id);
                }
            }
        }
    }

    /// Check broadcaster health — warn if no broadcasts for 30s with active clients
    fn check_broadcaster_health(&self) {
        let now = now_millis();
        for info in self.sessions.list_sessions() {
            if info.clients > 0 {
                if let Some(session) = self.sessions.get(&info.id) {
                    let last = session.metrics.last_broadcast_time.load(Ordering::Relaxed);
                    if last > 0 && now.saturating_sub(last) > 30_000 {
                        crate::debug_error!(
                            "STREAMING",
                            "Session {} broadcaster may be stalled ({}s since last broadcast, {} clients)",
                            info.id,
                            (now - last) / 1000,
                            info.clients
                        );
                    }
                }
            }
        }
    }

    /// Spawn broadcaster loop for the default session
    fn spawn_default_broadcaster(self: &Arc<Self>) {
        if let Some(ref session) = self.default_session {
            let session = Arc::clone(session);
            tokio::spawn(async move {
                session.output_broadcaster_loop().await;
            });
        }
    }

    /// Start server with HTTP static file serving using Axum
    #[cfg(feature = "streaming")]
    async fn start_with_http(self: Arc<Self>) -> Result<()> {
        use axum::{routing::get, Router};
        use tower_http::services::ServeDir;

        crate::debug_info!("STREAMING", "Server with HTTP listening on {}", self.addr);

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        // Build API routes (protected by auth)
        let api_routes = Router::new()
            .route("/ws", get(ws_handler))
            .route("/sessions", get(sessions_handler))
            .route("/stats", get(stats_ws_handler));

        // Apply auth middleware to API routes only if configured
        let auth_config = ApiAuthConfig {
            api_key: self.config.api_key.clone(),
            http_basic_auth: self.config.http_basic_auth.clone(),
            allow_api_key_in_query: self.config.allow_api_key_in_query,
        };
        let api_routes = if auth_config.is_configured() {
            api_routes.layer(axum::middleware::from_fn(move |req, next| {
                let auth_config = auth_config.clone();
                api_auth_middleware(req, next, auth_config)
            }))
        } else {
            api_routes
        };

        // Merge API routes with unprotected static file serving
        let app = api_routes
            .fallback_service(ServeDir::new(&self.config.web_root))
            .with_state(self.clone())
            .layer(build_cors_layer(&self.config.allowed_origins));

        // Start server
        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .map_err(|e| StreamingError::ServerError(format!("Failed to bind: {}", e)))?;

        axum::serve(listener, app.into_make_service())
            .await
            .map_err(|e| StreamingError::ServerError(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Start server with HTTPS/TLS static file serving using Axum
    #[cfg(feature = "streaming")]
    async fn start_with_https(self: Arc<Self>) -> Result<()> {
        use axum::{routing::get, Router};
        use axum_server::tls_rustls::RustlsConfig;
        use tower_http::services::ServeDir;

        let tls_config = self
            .config
            .tls
            .as_ref()
            .ok_or_else(|| StreamingError::ServerError("TLS config required".to_string()))?;

        crate::debug_info!(
            "STREAMING",
            "Server with HTTPS/TLS listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        // Build API routes (protected by auth)
        let api_routes = Router::new()
            .route("/ws", get(ws_handler))
            .route("/sessions", get(sessions_handler))
            .route("/stats", get(stats_ws_handler));

        // Apply auth middleware to API routes only if configured
        let auth_config = ApiAuthConfig {
            api_key: self.config.api_key.clone(),
            http_basic_auth: self.config.http_basic_auth.clone(),
            allow_api_key_in_query: self.config.allow_api_key_in_query,
        };
        let api_routes = if auth_config.is_configured() {
            api_routes.layer(axum::middleware::from_fn(move |req, next| {
                let auth_config = auth_config.clone();
                api_auth_middleware(req, next, auth_config)
            }))
        } else {
            api_routes
        };

        // Merge API routes with unprotected static file serving
        let app = api_routes
            .fallback_service(ServeDir::new(&self.config.web_root))
            .with_state(self.clone())
            .layer(build_cors_layer(&self.config.allowed_origins));

        // Build TLS config for axum-server
        let rustls_config = RustlsConfig::from_der(
            tls_config.certs.iter().map(|c| c.to_vec()).collect(),
            tls_config.key.secret_der().to_vec(),
        )
        .await
        .map_err(|e| StreamingError::ServerError(format!("Failed to create TLS config: {}", e)))?;

        // Parse address for axum-server
        let addr: std::net::SocketAddr = self.addr.parse().map_err(|e| {
            StreamingError::ServerError(format!("Invalid address '{}': {}", self.addr, e))
        })?;

        // Start HTTPS server
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|e| StreamingError::ServerError(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Start WebSocket-only server (original implementation)
    async fn start_websocket_only(self: Arc<Self>) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        crate::debug_info!(
            "STREAMING",
            "WebSocket-only server listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        // Accept WebSocket connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    if !self.can_accept_client() {
                        crate::debug_error!(
                            "STREAMING",
                            "Max clients reached ({}), rejecting connection from {}",
                            self.config.max_clients,
                            addr
                        );
                        continue;
                    }

                    if let Err(e) = stream.set_nodelay(true) {
                        crate::debug_error!("STREAMING", "Failed to set TCP_NODELAY: {}", e);
                    }

                    crate::debug_info!("STREAMING", "New connection from {}", addr);
                    let server = self.clone();
                    tokio::spawn(async move {
                        // Accept WebSocket with header callback to capture URI query and validate auth
                        let (header_callback, uri_query) = build_ws_header_callback(
                            server.config.api_key.clone(),
                            server.config.http_basic_auth.clone(),
                            server.config.allowed_origins.clone(),
                            server.config.allow_api_key_in_query,
                        );

                        // The tungstenite `Callback` trait fixes `ErrorResponse` as
                        // `HttpResponse<Option<String>>` — we cannot box or shrink it
                        // without violating the external API contract.
                        let ws_result = accept_hdr_async_with_config(
                            stream,
                            header_callback,
                            ws_accept_config(),
                        )
                        .await;

                        match ws_result {
                            Ok(ws_stream) => {
                                let query_str = uri_query.lock().take();
                                let params = ConnectionParams::from_uri_query(query_str.as_deref());
                                if let Err(e) =
                                    server.handle_connection_ws(ws_stream, &params).await
                                {
                                    crate::debug_error!(
                                        "STREAMING",
                                        "Connection error from {}: {}",
                                        addr,
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                crate::debug_error!(
                                    "STREAMING",
                                    "WebSocket handshake failed from {}: {}",
                                    addr,
                                    e
                                );
                            }
                        }
                    });
                }
                Err(e) => {
                    crate::debug_error!("STREAMING", "Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Start WebSocket-only server with TLS (WSS)
    async fn start_websocket_only_tls(self: Arc<Self>) -> Result<()> {
        let tls_config = self
            .config
            .tls
            .as_ref()
            .ok_or_else(|| StreamingError::ServerError("TLS config required".to_string()))?;

        let rustls_config = tls_config.build_rustls_config()?;
        let acceptor = TlsAcceptor::from(Arc::new(rustls_config));

        let listener = TcpListener::bind(&self.addr).await?;
        crate::debug_info!(
            "STREAMING",
            "WebSocket-only server with TLS (WSS) listening on {}",
            self.addr
        );

        self.spawn_default_broadcaster();
        self.spawn_idle_reaper();

        // Accept TLS connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    if !self.can_accept_client() {
                        crate::debug_error!(
                            "STREAMING",
                            "Max clients reached ({}), rejecting TLS connection from {}",
                            self.config.max_clients,
                            addr
                        );
                        continue;
                    }

                    if let Err(e) = stream.set_nodelay(true) {
                        crate::debug_error!("STREAMING", "Failed to set TCP_NODELAY: {}", e);
                    }

                    crate::debug_info!("STREAMING", "New TLS connection from {}", addr);
                    let server = self.clone();
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                // Accept WebSocket with header callback to capture URI query and validate auth
                                let (header_callback, uri_query) = build_ws_header_callback(
                                    server.config.api_key.clone(),
                                    server.config.http_basic_auth.clone(),
                                    server.config.allowed_origins.clone(),
                                    server.config.allow_api_key_in_query,
                                );

                                // Same as above: ErrorResponse type is fixed by the
                                // tungstenite Callback trait and cannot be reduced.
                                let ws_result = accept_hdr_async_with_config(
                                    tls_stream,
                                    header_callback,
                                    ws_accept_config(),
                                )
                                .await;

                                match ws_result {
                                    Ok(ws_stream) => {
                                        let query_str = uri_query.lock().take();
                                        let params =
                                            ConnectionParams::from_uri_query(query_str.as_deref());
                                        if let Err(e) = server
                                            .handle_tls_connection_ws(ws_stream, &params)
                                            .await
                                        {
                                            crate::debug_error!(
                                                "STREAMING",
                                                "TLS connection error from {}: {}",
                                                addr,
                                                e
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        crate::debug_error!(
                                            "STREAMING",
                                            "TLS WebSocket handshake failed from {}: {}",
                                            addr,
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                crate::debug_error!(
                                    "STREAMING",
                                    "TLS handshake failed from {}: {}",
                                    addr,
                                    e
                                );
                            }
                        }
                    });
                }
                Err(e) => {
                    crate::debug_error!("STREAMING", "Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Build a refresh message from the session's current visible terminal
    /// state. Pure (no side effects, no async) so the tungstenite and axum
    /// WebSocket handlers share one implementation.
    fn build_refresh_message(
        terminal_for_refresh: &Arc<RwLock<Terminal>>,
    ) -> Option<ServerMessage> {
        let terminal = terminal_for_refresh.read();
        let content = terminal.export_visible_screen_styled();
        let (cols, rows) = terminal.size();
        Some(ServerMessage::refresh(cols as u16, rows as u16, content))
    }

    /// Build a semantic-snapshot message (or an error message for an invalid
    /// scope). Pure (no side effects, no async) so the tungstenite and axum
    /// WebSocket handlers share one implementation.
    ///
    /// Returns `Some(msg)` for a valid scope (including the snapshot payload),
    /// `Some(error_msg)` for an invalid scope string, and `None` only when a
    /// valid scope produced no payload (currently never).
    fn build_snapshot_message(
        terminal_for_refresh: &Arc<RwLock<Terminal>>,
        scope: &str,
        max_commands: Option<u32>,
    ) -> ServerMessage {
        use crate::terminal::semantic_snapshot::SnapshotScope;
        match scope {
            "visible" => {
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Visible),
                )
            }
            "recent" => {
                let n = max_commands.unwrap_or(10) as usize;
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Recent(n)),
                )
            }
            "full" => {
                let terminal = terminal_for_refresh.read();
                ServerMessage::semantic_snapshot(
                    terminal.get_semantic_snapshot_json(SnapshotScope::Full),
                )
            }
            other => ServerMessage::error(format!(
                "Invalid snapshot scope '{}': must be 'visible', 'recent', or 'full'",
                other
            )),
        }
    }

    /// Handle a new WebSocket connection (already upgraded)
    async fn handle_connection_ws(
        self: &Arc<Self>,
        ws_stream: tokio_tungstenite::WebSocketStream<TcpStream>,
        params: &ConnectionParams,
    ) -> Result<()> {
        let (session, _global_guard, _session_guard, read_only) =
            self.prepare_ws_session(params)?;
        let client = Client::new(ws_stream, read_only);
        self.run_ws_session(client, session, read_only, "Client")
            .await
    }

    /// Handle a new TLS WebSocket connection (already upgraded)
    async fn handle_tls_connection_ws(
        self: &Arc<Self>,
        ws_stream: tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>,
        params: &ConnectionParams,
    ) -> Result<()> {
        let (session, _global_guard, _session_guard, read_only) =
            self.prepare_ws_session(params)?;
        let client = Client::new(ws_stream, read_only);
        self.run_ws_session(client, session, read_only, "TLS Client")
            .await
    }

    /// Common pre-loop setup shared by both tungstenite WebSocket handlers.
    ///
    /// Resolves the session, reserves the global + per-session client slots
    /// (returning RAII guards whose `Drop` releases them), and computes the
    /// read-only flag. The caller wraps the accepted stream in a `Client<S>`
    /// and hands it to `run_ws_session`.
    fn prepare_ws_session(
        self: &Arc<Self>,
        params: &ConnectionParams,
    ) -> Result<(
        Arc<StreamSessionState>,
        GlobalClientGuard<'_>,
        SessionClientGuard,
        bool,
    )> {
        let session = self.resolve_session(params)?;
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }
        let global_guard = GlobalClientGuard { server: self };
        if !session.try_add_client(self.config.max_clients_per_session) {
            return Err(StreamingError::MaxClientsReached);
        }
        let session_guard = SessionClientGuard {
            session: Arc::clone(&session),
        };
        let read_only = params.readonly || self.config.default_read_only;
        Ok((session, global_guard, session_guard, read_only))
    }

    /// Shared dispatch loop for both tungstenite WebSocket transports
    /// (plain TCP and TLS). The transport stream type is captured by the
    /// `Client<S>` generic; all protobuf encode/decode and ping/pong handling
    /// lives in `Client`. `transport_label` is used only in debug logs so the
    /// two transports remain distinguishable.
    ///
    /// This implements the full client-message dispatch (all `ClientMessage`
    /// arms). Both transports now share identical message handling —
    /// previously the TLS path silently dropped Mouse/Focus/Paste/Selection/
    /// Clipboard messages; they are now handled uniformly.
    async fn run_ws_session<S>(
        self: &Arc<Self>,
        mut client: Client<S>,
        session: Arc<StreamSessionState>,
        read_only: bool,
        transport_label: &'static str,
    ) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let client_id = client.id();

        // Send initial connection message
        let connect_msg = session.build_connect_message(&client_id.to_string(), read_only);
        client.send(connect_msg).await?;

        // Sync terminal mode state for existing sessions
        for mode_msg in session.build_mode_sync_messages() {
            client.send(mode_msg).await?;
        }

        crate::debug_info!(
            "STREAMING",
            "{} {} connected to session {} (total: {})",
            transport_label,
            client_id,
            session.id,
            self.client_count()
        );

        // Subscribe to session broadcasts
        let mut output_rx = session.broadcast_tx.subscribe();

        let terminal_for_refresh = Arc::clone(&session.terminal);

        // Setup keepalive timer
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));
        let mut subscriptions: Option<
            std::collections::HashSet<crate::streaming::protocol::EventType>,
        > = None;
        let mut rate_limiter = if self.config.input_rate_limit_bytes_per_sec > 0 {
            Some(InputRateLimiter::new(
                self.config.input_rate_limit_bytes_per_sec,
            ))
        } else {
            None
        };

        loop {
            tokio::select! {
                msg = client.recv() => {
                    match msg {
                        Err(e) => {
                            crate::debug_error!("STREAMING", "{} {} error: {}", transport_label, client_id, e);
                            break;
                        }
                        Ok(msg_opt) => match msg_opt {
                        Some(client_msg) => {
                            match client_msg {
                                crate::streaming::protocol::ClientMessage::Input { data } => {
                                    if read_only {
                                        continue;
                                    }
                                    if let Some(ref mut limiter) = rate_limiter {
                                        if !limiter.try_consume(data.len()) {
                                            crate::debug_error!("STREAMING", "Rate limit exceeded for {} {}", transport_label, client_id);
                                            continue;
                                        }
                                    }
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        session.metrics.input_bytes.fetch_add(data.len(), Ordering::Relaxed);
                                        let mut w = writer.lock();
                                        use std::io::Write;
                                        if let Err(e) = w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
                                            crate::debug_error!("STREAMING", "PTY write error for session {}: {}", session.id, e);
                                            session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                    if let Err(e) = validate_terminal_size(cols, rows) {
                                        crate::debug_error!("STREAMING", "{} {} sent invalid resize: {}", transport_label, client_id, e);
                                    } else {
                                        let _ = session.resize_tx.send((cols, rows));
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Ping => {
                                    if let Err(e) = client.send(ServerMessage::pong()).await {
                                        crate::debug_error!("STREAMING", "Failed to send pong to {} {}: {}", transport_label, client_id, e);
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                    if let Some(msg) = Self::build_refresh_message(&terminal_for_refresh) {
                                        if let Err(e) = client.send(msg).await {
                                            crate::debug_error!("STREAMING", "Failed to send refresh to {} {}: {}", transport_label, client_id, e);
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Subscribe { events } => {
                                    subscriptions = Some(events.into_iter().collect());
                                }
                                crate::streaming::protocol::ClientMessage::Mouse {
                                    col, row, button, shift, ctrl, alt, event_type,
                                } => {
                                    if read_only { continue; }
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        let bytes = {
                                            let mut terminal = session.terminal.write();
                                            // Build modifiers bitmask: shift=1, meta/alt=2, ctrl=4
                                            let mods = if shift { 1u8 } else { 0 }
                                                | if alt { 2 } else { 0 }
                                                | if ctrl { 4 } else { 0 };
                                            let pressed = event_type != "release";
                                            let mouse_event = crate::mouse::MouseEvent::new(
                                                button,
                                                col as usize,
                                                row as usize,
                                                pressed,
                                                mods,
                                            );
                                            terminal.report_mouse(mouse_event)
                                        };
                                        if !bytes.is_empty() {
                                            session.metrics.input_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
                                            let mut w = writer.lock();
                                            use std::io::Write;
                                            if let Err(e) = w.write_all(&bytes).and_then(|_| w.flush()) {
                                                crate::debug_error!("STREAMING", "PTY mouse write error for session {}: {}", session.id, e);
                                                session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::FocusChange { focused } => {
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        let bytes = {
                                            let terminal = session.terminal.write();
                                            if terminal.focus_tracking() {
                                                if focused {
                                                    terminal.report_focus_in()
                                                } else {
                                                    terminal.report_focus_out()
                                                }
                                            } else {
                                                Vec::new()
                                            }
                                        };
                                        if !bytes.is_empty() {
                                            session.metrics.input_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
                                            let mut w = writer.lock();
                                            use std::io::Write;
                                            if let Err(e) = w.write_all(&bytes).and_then(|_| w.flush()) {
                                                crate::debug_error!("STREAMING", "PTY focus write error for session {}: {}", session.id, e);
                                                session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Paste { content } => {
                                    if read_only { continue; }
                                    if let Some(ref mut limiter) = rate_limiter {
                                        if !limiter.try_consume(content.len()) {
                                            crate::debug_error!("STREAMING", "Rate limit exceeded for {} {}", transport_label, client_id);
                                            continue;
                                        }
                                    }
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        let terminal = session.terminal.write();
                                        session.metrics.input_bytes.fetch_add(content.len(), Ordering::Relaxed);
                                        let mut w = writer.lock();
                                        use std::io::Write;
                                        let result = if terminal.bracketed_paste() {
                                            w.write_all(terminal.bracketed_paste_start())
                                                .and_then(|_| w.write_all(content.as_bytes()))
                                                .and_then(|_| w.write_all(terminal.bracketed_paste_end()))
                                                .and_then(|_| w.flush())
                                        } else {
                                            w.write_all(content.as_bytes())
                                                .and_then(|_| w.flush())
                                        };
                                        if let Err(e) = result {
                                            crate::debug_error!("STREAMING", "PTY paste write error for session {}: {}", session.id, e);
                                            session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::SelectionRequest {
                                    start_col, start_row, end_col, end_row, mode,
                                } => {
                                    let selection_msg = {
                                        let mut terminal = session.terminal.write();
                                        if mode == "clear" {
                                            terminal.clear_selection();
                                            Some(ServerMessage::selection_cleared())
                                        } else if mode == "word" {
                                            terminal.select_word_at(start_col as usize, start_row as usize);
                                            if let Some(sel) = terminal.get_selection() {
                                                let text = terminal.get_selected_text();
                                                Some(ServerMessage::selection_changed(
                                                    Some(sel.start.0 as u16),
                                                    Some(sel.start.1 as u16),
                                                    Some(sel.end.0 as u16),
                                                    Some(sel.end.1 as u16),
                                                    text,
                                                    "chars".to_string(),
                                                    false,
                                                ))
                                            } else {
                                                None
                                            }
                                        } else if mode == "line" {
                                            terminal.select_line(start_row as usize);
                                            if let Some(sel) = terminal.get_selection() {
                                                let text = terminal.get_selected_text();
                                                Some(ServerMessage::selection_changed(
                                                    Some(sel.start.0 as u16),
                                                    Some(sel.start.1 as u16),
                                                    Some(sel.end.0 as u16),
                                                    Some(sel.end.1 as u16),
                                                    text,
                                                    "line".to_string(),
                                                    false,
                                                ))
                                            } else {
                                                None
                                            }
                                        } else {
                                            let sel_mode = match mode.as_str() {
                                                "block" => SelectionMode::Block,
                                                "line" => SelectionMode::Line,
                                                _ => SelectionMode::Character,
                                            };
                                            terminal.set_selection(
                                                (start_col as usize, start_row as usize),
                                                (end_col as usize, end_row as usize),
                                                sel_mode,
                                            );
                                            let text = terminal.get_selected_text();
                                            Some(ServerMessage::selection_changed(
                                                Some(start_col),
                                                Some(start_row),
                                                Some(end_col),
                                                Some(end_row),
                                                text,
                                                mode,
                                                false,
                                            ))
                                        }
                                    };
                                    if let Some(msg) = selection_msg {
                                        self.broadcast_to_session(&session.id, msg);
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::ClipboardRequest {
                                    operation, content, target,
                                } => {
                                    match operation.as_str() {
                                        "set" => {
                                            if let Some(ref text) = content {
                                                let mut terminal = session.terminal.write();
                                                terminal.set_clipboard(Some(text.clone()));
                                                self.broadcast_to_session(
                                                    &session.id,
                                                    ServerMessage::clipboard_sync(
                                                        "set".to_string(),
                                                        text.clone(),
                                                        target,
                                                    ),
                                                );
                                            }
                                        }
                                        "get" => {
                                            let clipboard = {
                                                let terminal = session.terminal.write();
                                                terminal.clipboard().unwrap_or_default().to_string()
                                            };
                                            let response = ServerMessage::clipboard_sync(
                                                "get_response".to_string(),
                                                clipboard,
                                                target,
                                            );
                                            let _ = client.send(response).await;
                                        }
                                        _ => {}
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::SnapshotRequest { scope, max_commands } => {
                                    let msg = Self::build_snapshot_message(&terminal_for_refresh, &scope, max_commands);
                                    if let Err(e) = client.send(msg).await {
                                        crate::debug_error!("STREAMING", "Failed to send snapshot to {} {}: {}", transport_label, client_id, e);
                                    }
                                }
                            }
                        }
                        None => {
                            crate::debug_info!("STREAMING", "{} {} disconnected from session {}", transport_label, client_id, session.id);
                            break;
                        }
                        }
                    }
                }

                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if should_send(&msg, &subscriptions)
                            && client.send(msg).await.is_err() {
                                break;
                            }
                    }
                }

                _ = async {
                    if let Some(ref mut timer) = keepalive_timer {
                        timer.tick().await
                    } else {
                        std::future::pending::<tokio::time::Instant>().await
                    }
                } => {
                    if let Err(e) = client.ping().await {
                        crate::debug_error!("STREAMING", "Failed to ping {} {}: {}", transport_label, client_id, e);
                        break;
                    }
                }
            }
        }

        crate::debug_info!(
            "STREAMING",
            "{} {} cleanup (remaining: {})",
            transport_label,
            client_id,
            self.client_count() - 1
        );

        Ok(())
    }

    // -- Backward-compatible send helpers (route to default session) --

    /// Send terminal output to all connected clients
    pub fn send_output(&self, data: String) -> Result<()> {
        if let Some(ref session) = self.default_session {
            match session.output_tx.try_send(data) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    session
                        .metrics
                        .dropped_messages
                        .fetch_add(1, Ordering::Relaxed);
                    Ok(()) // Drop silently under backpressure
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamingError::ServerError(
                    "Output channel closed".to_string(),
                )),
            }
        } else {
            Err(StreamingError::ServerError(
                "No default session".to_string(),
            ))
        }
    }

    /// Send a resize event to all clients
    pub fn send_resize(&self, cols: u16, rows: u16) {
        let msg = ServerMessage::resize(cols, rows);
        self.broadcast(msg);
    }

    /// Send a title change event to all clients
    pub fn send_title(&self, title: String) {
        let msg = ServerMessage::title(title);
        self.broadcast(msg);
    }

    /// Send a bell event to all clients
    pub fn send_bell(&self) {
        let msg = ServerMessage::bell();
        self.broadcast(msg);
    }

    /// Send a CWD changed event to all clients
    pub fn send_cwd_changed(
        &self,
        old_cwd: Option<String>,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
    ) {
        let msg = ServerMessage::cwd_changed_full(old_cwd, new_cwd, hostname, username, timestamp);
        self.broadcast(msg);
    }

    /// Send a trigger matched event to all clients
    #[allow(clippy::too_many_arguments)]
    pub fn send_trigger_matched(
        &self,
        trigger_id: u64,
        row: u16,
        col: u16,
        end_col: u16,
        text: String,
        captures: Vec<String>,
        timestamp: u64,
    ) {
        let msg = ServerMessage::trigger_matched(
            trigger_id, row, col, end_col, text, captures, timestamp,
        );
        self.broadcast(msg);
    }

    /// Send a trigger action notify event to all clients
    pub fn send_action_notify(&self, trigger_id: u64, title: String, message: String) {
        let msg = ServerMessage::action_notify(trigger_id, title, message);
        self.broadcast(msg);
    }

    /// Send a trigger action mark line event to all clients
    pub fn send_action_mark_line(
        &self,
        trigger_id: u64,
        row: u16,
        label: Option<String>,
        color: Option<(u8, u8, u8)>,
    ) {
        let msg = ServerMessage::action_mark_line(trigger_id, row, label, color);
        self.broadcast(msg);
    }

    /// Send a mode changed event to all clients
    pub fn send_mode_changed(&self, mode: String, enabled: bool) {
        let msg = ServerMessage::mode_changed(mode, enabled);
        self.broadcast(msg);
    }

    /// Send a graphics added event to all clients
    pub fn send_graphics_added(&self, row: u16) {
        let msg = ServerMessage::graphics_added(row);
        self.broadcast(msg);
    }

    /// Send a hyperlink added event to all clients
    pub fn send_hyperlink_added(&self, url: String, row: u16, col: u16, id: Option<String>) {
        let msg = match id {
            Some(id) => ServerMessage::hyperlink_added_with_id(url, row, col, id),
            None => ServerMessage::hyperlink_added(url, row, col),
        };
        self.broadcast(msg);
    }

    /// Send a user variable changed event to all clients
    pub fn send_user_var_changed(&self, name: String, value: String, old_value: Option<String>) {
        let msg = ServerMessage::user_var_changed_full(name, value, old_value);
        self.broadcast(msg);
    }

    /// Send a progress bar changed event to all clients
    pub fn send_progress_bar_changed(
        &self,
        action: crate::terminal::ProgressBarAction,
        id: String,
        state: Option<crate::terminal::ProgressState>,
        percent: Option<u8>,
        label: Option<String>,
    ) {
        let msg = ServerMessage::progress_bar_changed(action, id, state, percent, label);
        self.broadcast(msg);
    }

    /// Send a cursor position event to all clients
    pub fn send_cursor_position(&self, col: u16, row: u16, visible: bool) {
        let msg = ServerMessage::cursor(col, row, visible);
        self.broadcast(msg);
    }

    /// Send a badge changed event to all clients
    pub fn send_badge_changed(&self, badge: Option<String>) {
        let msg = ServerMessage::badge_changed(badge);
        self.broadcast(msg);
    }

    /// Shutdown the server and disconnect all clients
    pub fn shutdown(&self, reason: String) {
        crate::debug_info!("STREAMING", "Shutting down server: {}", reason);
        let msg = ServerMessage::shutdown(reason);
        self.broadcast(msg);
        self.shutdown.notify_waiters();
    }

    /// Handle Axum WebSocket connection
    #[cfg(feature = "streaming")]
    async fn handle_axum_websocket(
        self: &Arc<Self>,
        socket: axum::extract::ws::WebSocket,
        params: ConnectionParams,
    ) -> Result<()> {
        use axum::extract::ws::Message as AxumMessage;
        use futures_util::{SinkExt, StreamExt};

        // Resolve session first
        let session = self.resolve_session(&params)?;

        // Try to reserve a global client slot
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }
        let _global_guard = GlobalClientGuard { server: self };

        // Try to add client to session
        if !session.try_add_client(self.config.max_clients_per_session) {
            return Err(StreamingError::MaxClientsReached);
        }
        let _session_guard = SessionClientGuard {
            session: Arc::clone(&session),
        };

        let read_only = params.readonly || self.config.default_read_only;

        let client_id = uuid::Uuid::new_v4();

        let (mut ws_tx, mut ws_rx) = socket.split();

        // Send initial connection message
        let connect_msg = session.build_connect_message(&client_id.to_string(), read_only);
        let msg_bytes = encode_server_message(&connect_msg)?;
        ws_tx
            .send(AxumMessage::Binary(msg_bytes.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        // Sync terminal mode state for existing sessions
        for mode_msg in session.build_mode_sync_messages() {
            let mode_bytes = encode_server_message(&mode_msg)?;
            ws_tx
                .send(AxumMessage::Binary(mode_bytes.into()))
                .await
                .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        }

        crate::debug_info!(
            "STREAMING",
            "Axum WebSocket client {} connected to session {} (total: {})",
            client_id,
            session.id,
            self.client_count()
        );

        // Subscribe to session broadcasts
        let mut output_rx = session.broadcast_tx.subscribe();

        let terminal_for_refresh = Arc::clone(&session.terminal);
        let resize_tx = session.resize_tx.clone();

        // Setup keepalive timer
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));
        let mut subscriptions: Option<
            std::collections::HashSet<crate::streaming::protocol::EventType>,
        > = None;
        let mut rate_limiter = if self.config.input_rate_limit_bytes_per_sec > 0 {
            Some(InputRateLimiter::new(
                self.config.input_rate_limit_bytes_per_sec,
            ))
        } else {
            None
        };

        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(AxumMessage::Binary(data))) => {
                            match decode_client_message(&data) {
                                Ok(client_msg) => {
                                    match client_msg {
                                        crate::streaming::protocol::ClientMessage::Input { data } => {
                                            if read_only {
                                                continue;
                                            }
                                            if let Some(ref mut limiter) = rate_limiter {
                                                if !limiter.try_consume(data.len()) {
                                                    crate::debug_error!("STREAMING", "Rate limit exceeded for Axum client {}", client_id);
                                                    continue;
                                                }
                                            }
                                            if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                                session.metrics.input_bytes.fetch_add(data.len(), Ordering::Relaxed);
                                                let mut w = writer.lock();
                                                use std::io::Write;
                                                if let Err(e) = w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
                                                    crate::debug_error!("STREAMING", "PTY write error for Axum session {}: {}", session.id, e);
                                                    session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                            if let Err(e) = validate_terminal_size(cols, rows) {
                                                crate::debug_error!("STREAMING", "Axum client {} sent invalid resize: {}", client_id, e);
                                            } else {
                                                let _ = resize_tx.send((cols, rows));
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Ping => {
                                            if let Ok(bytes) = encode_server_message(&ServerMessage::pong()) {
                                                let _ = ws_tx.send(AxumMessage::Binary(bytes.into())).await;
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                            if let Some(msg) = Self::build_refresh_message(&terminal_for_refresh) {
                                                if let Ok(bytes) = encode_server_message(&msg) {
                                                    let _ = ws_tx.send(AxumMessage::Binary(bytes.into())).await;
                                                }
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::SnapshotRequest { scope, max_commands } => {
                                            let msg = Self::build_snapshot_message(&terminal_for_refresh, &scope, max_commands);
                                            if let Ok(bytes) = encode_server_message(&msg) {
                                                let _ = ws_tx.send(AxumMessage::Binary(bytes.into())).await;
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Subscribe { events } => {
                                            subscriptions = Some(events.into_iter().collect());
                                        }
                                        // Mouse, Focus, Paste, Selection, Clipboard handled only in primary handlers
                                        _ => {}
                                    }
                                }
                                Err(e) => {
                                    crate::debug_error!("STREAMING", "Failed to parse client message: {}", e);
                                }
                            }
                        }
                        Some(Ok(AxumMessage::Text(_))) => {
                            crate::debug_error!("STREAMING", "Text messages not supported, use binary protocol");
                        }
                        Some(Ok(AxumMessage::Ping(_))) => {}
                        Some(Ok(AxumMessage::Pong(_))) => {}
                        Some(Ok(AxumMessage::Close(_))) | None => {
                            crate::debug_info!("STREAMING", "Axum Client {} disconnected from session {}", client_id, session.id);
                            break;
                        }
                        Some(Err(e)) => {
                            crate::debug_error!("STREAMING", "WebSocket error: {}", e);
                            break;
                        }
                    }
                }

                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if should_send(&msg, &subscriptions) {
                            if let Ok(bytes) = encode_server_message(&msg) {
                                if ws_tx.send(AxumMessage::Binary(bytes.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }

                _ = async {
                    if let Some(ref mut timer) = keepalive_timer {
                        timer.tick().await
                    } else {
                        std::future::pending::<tokio::time::Instant>().await
                    }
                } => {
                    if ws_tx.send(AxumMessage::Ping(vec![].into())).await.is_err() {
                        crate::debug_error!("STREAMING", "Failed to ping Axum client {}", client_id);
                        break;
                    }
                }
            }
        }

        crate::debug_info!(
            "STREAMING",
            "Axum Client {} cleanup (remaining: {})",
            client_id,
            self.client_count() - 1
        );

        Ok(())
    }
}

/// Check if a message should be sent based on client's subscription filter
fn should_send(
    msg: &ServerMessage,
    subscriptions: &Option<std::collections::HashSet<crate::streaming::protocol::EventType>>,
) -> bool {
    use crate::streaming::protocol::EventType;
    let subs = match subscriptions {
        Some(s) => s,
        None => return true, // No filter = send everything
    };

    match msg {
        ServerMessage::Output { .. } => subs.contains(&EventType::Output),
        ServerMessage::CursorPosition { .. } => subs.contains(&EventType::Cursor),
        ServerMessage::Bell => subs.contains(&EventType::Bell),
        ServerMessage::Title { .. } => subs.contains(&EventType::Title),
        ServerMessage::Resize { .. } => subs.contains(&EventType::Resize),
        ServerMessage::CwdChanged { .. } => subs.contains(&EventType::Cwd),
        ServerMessage::TriggerMatched { .. } => subs.contains(&EventType::Trigger),
        ServerMessage::ActionNotify { .. } | ServerMessage::ActionMarkLine { .. } => {
            subs.contains(&EventType::Action)
        }
        ServerMessage::ModeChanged { .. } => subs.contains(&EventType::Mode),
        ServerMessage::GraphicsAdded { .. } => subs.contains(&EventType::Graphics),
        ServerMessage::HyperlinkAdded { .. } => subs.contains(&EventType::Hyperlink),
        ServerMessage::UserVarChanged { .. } => subs.contains(&EventType::UserVar),
        ServerMessage::ProgressBarChanged { .. } => subs.contains(&EventType::ProgressBar),
        ServerMessage::BadgeChanged { .. } => subs.contains(&EventType::Badge),
        ServerMessage::SelectionChanged { .. } => subs.contains(&EventType::Selection),
        ServerMessage::ClipboardSync { .. } => subs.contains(&EventType::Clipboard),
        ServerMessage::ShellIntegrationEvent { .. } => subs.contains(&EventType::Shell),
        ServerMessage::SystemStats { .. } => subs.contains(&EventType::SystemStats),
        ServerMessage::ZoneOpened { .. }
        | ServerMessage::ZoneClosed { .. }
        | ServerMessage::ZoneScrolledOut { .. } => subs.contains(&EventType::Zone),
        ServerMessage::EnvironmentChanged { .. } => subs.contains(&EventType::Environment),
        ServerMessage::RemoteHostTransition { .. } => subs.contains(&EventType::RemoteHost),
        ServerMessage::SubShellDetected { .. } => subs.contains(&EventType::SubShell),
        ServerMessage::SemanticSnapshot { .. } => subs.contains(&EventType::Snapshot),
        ServerMessage::FileTransferStarted { .. }
        | ServerMessage::FileTransferProgress { .. }
        | ServerMessage::FileTransferCompleted { .. }
        | ServerMessage::FileTransferFailed { .. } => subs.contains(&EventType::FileTransfer),
        ServerMessage::UploadRequested { .. } => subs.contains(&EventType::UploadRequest),
        ServerMessage::ScreenCleared { .. } => subs.contains(&EventType::ScreenCleared),
        // Always send system messages
        ServerMessage::Connected { .. }
        | ServerMessage::Refresh { .. }
        | ServerMessage::Error { .. }
        | ServerMessage::Shutdown { .. }
        | ServerMessage::Pong => true,
    }
}

/// Unified API authentication middleware for Axum.
/// Checks in order: Bearer header → X-API-Key header → ?api_key= query → Basic Auth header.
/// When both API key and Basic Auth are configured, either one satisfies auth.
#[cfg(feature = "streaming")]
async fn api_auth_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
    auth_config: ApiAuthConfig,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
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
    let mut headers = Vec::new();
    if auth_config.http_basic_auth.is_some() {
        headers.push((header::WWW_AUTHENTICATE, "Basic realm=\"Terminal Server\""));
    }

    let mut response = (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    for (key, value) in headers {
        response.headers_mut().insert(key, value.parse().unwrap());
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
fn check_ws_origin(origin: Option<&str>, allowed_origins: Option<&[String]>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    match allowed_origins {
        Some(list) => list.iter().any(|o| o == origin),
        None => is_local_origin(origin),
    }
}

/// True if `origin` (e.g. `http://localhost:8099`) points at a loopback host.
fn is_local_origin(origin: &str) -> bool {
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
fn host_of_authority(authority: &str) -> &str {
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
fn build_cors_layer(allowed_origins: &Option<Vec<String>>) -> tower_http::cors::CorsLayer {
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

#[cfg(test)]
mod origin_tests {
    use super::{
        build_cors_layer, check_ws_origin, is_local_origin, sessions_handler, StreamingConfig,
        StreamingServer,
    };
    use crate::terminal::Terminal;
    use parking_lot::RwLock;
    use std::sync::Arc;
    use tower::ServiceExt; // .oneshot for in-process router tests

    #[test]
    fn no_origin_is_allowed_non_browser() {
        assert!(check_ws_origin(None, None));
        assert!(check_ws_origin(
            None,
            Some(&["https://app.example.com".to_string()][..])
        ));
    }

    #[test]
    fn default_rejects_remote_browser_origin() {
        // CSRF-via-WebSocket: a remote page must be blocked when no allowlist is set.
        assert!(!check_ws_origin(Some("https://evil.com"), None));
        assert!(!check_ws_origin(Some("http://attacker.example:8080"), None));
    }

    #[test]
    fn default_allows_local_browser_origin() {
        assert!(check_ws_origin(Some("http://localhost:8099"), None));
        assert!(check_ws_origin(Some("https://127.0.0.1:8099"), None));
        assert!(check_ws_origin(Some("http://[::1]:8099"), None));
    }

    #[test]
    fn allowlist_enforced_when_configured() {
        let list = vec!["https://app.example.com".to_string()];
        let allowed: &[String] = &list;
        assert!(check_ws_origin(
            Some("https://app.example.com"),
            Some(allowed)
        ));
        assert!(!check_ws_origin(Some("https://evil.com"), Some(allowed)));
        // Even a local origin is rejected if it's not in the explicit allowlist.
        assert!(!check_ws_origin(
            Some("http://localhost:8099"),
            Some(allowed)
        ));
    }

    #[test]
    fn is_local_origin_host_extraction() {
        assert!(is_local_origin("http://localhost:8099"));
        assert!(is_local_origin("https://127.0.0.1:443"));
        assert!(is_local_origin("http://[::1]:8099"));
        assert!(!is_local_origin("http://localhost.evil.com:8099"));
        assert!(!is_local_origin("https://example.com"));
        // A look-alike host must not match.
        assert!(!is_local_origin("http://127.0.0.1.evil.com"));
    }

    fn origin_request(
        method: &str,
        uri: &str,
        origin: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn cors_without_allowlist_mirrors_ws_local_origin_default() {
        use axum::{routing::get, Router};

        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(build_cors_layer(&None));

        // Remote origin: no Access-Control-Allow-Origin header, so a browser
        // blocks the cross-origin read.
        let res = app
            .clone()
            .oneshot(origin_request("GET", "/ok", Some("https://evil.example")))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert!(!res.headers().contains_key("access-control-allow-origin"));

        // Local origin: allowed and echoed back.
        let res = app
            .clone()
            .oneshot(origin_request("GET", "/ok", Some("http://127.0.0.1:8099")))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("http://127.0.0.1:8099")
        );
    }

    #[tokio::test]
    async fn cors_with_allowlist_passes_listed_origin_only() {
        use axum::{routing::get, Router};

        let allowed = Some(vec!["https://app.example.com".to_string()]);
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(build_cors_layer(&allowed));

        let res = app
            .clone()
            .oneshot(origin_request(
                "GET",
                "/ok",
                Some("https://app.example.com"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://app.example.com")
        );

        // A local origin is not in the explicit allowlist → no ACAO, same
        // exact-match semantics as check_ws_origin.
        let res = app
            .oneshot(origin_request("GET", "/ok", Some("http://localhost:8099")))
            .await
            .unwrap();
        assert!(!res.headers().contains_key("access-control-allow-origin"));
    }

    #[tokio::test]
    async fn sessions_endpoint_rejects_disallowed_origin() {
        use axum::{routing::get, Router};

        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
        let app = Router::new()
            .route("/sessions", get(sessions_handler))
            .with_state(server);

        // Cross-origin browser request → 403.
        let res = app
            .clone()
            .oneshot(origin_request(
                "GET",
                "/sessions",
                Some("https://evil.example"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 403);

        // Local origin → 200 with the session list.
        let res = app
            .clone()
            .oneshot(origin_request(
                "GET",
                "/sessions",
                Some("http://localhost:8099"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        let body = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"sessions\""));

        // No Origin header (non-browser client such as curl) → 200.
        let res = app
            .oneshot(origin_request("GET", "/sessions", None))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    #[tokio::test]
    async fn sessions_endpoint_allows_configured_origin() {
        use axum::{routing::get, Router};

        let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
        let config = StreamingConfig {
            allowed_origins: Some(vec!["https://app.example.com".to_string()]),
            ..StreamingConfig::default()
        };
        let server = Arc::new(StreamingServer::with_config(
            terminal,
            "127.0.0.1:0".to_string(),
            config,
        ));
        let app = Router::new()
            .route("/sessions", get(sessions_handler))
            .with_state(server);

        let res = app
            .clone()
            .oneshot(origin_request(
                "GET",
                "/sessions",
                Some("https://app.example.com"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);

        // An origin not on the allowlist is still rejected.
        let res = app
            .oneshot(origin_request(
                "GET",
                "/sessions",
                Some("https://evil.example"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), 403);
    }
}

fn validate_ws_handshake_auth(
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
async fn ws_handler(
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
async fn sessions_handler(
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
async fn stats_ws_handler(
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
async fn handle_stats_websocket(
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

impl std::fmt::Debug for StreamingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingServer")
            .field("addr", &self.addr)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
mod tests {
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

    // ─── InputRateLimiter tests ─────────────────────────────────────────────
}
