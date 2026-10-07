//! WebSocket streaming server implementation

use crate::streaming::client::Client;
use crate::streaming::config::{ApiAuthConfig, HttpBasicAuthConfig, StreamingConfig};
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::proto::{decode_client_message, encode_server_message};
use crate::streaming::protocol::{ClientMessage, ServerMessage, ThemeInfo};
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
/// cap: Bytes accepted in one inbound WebSocket message from a streaming client.
const WS_MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
/// cap: Bytes accepted in one inbound WebSocket frame from a streaming client.
const WS_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Maximum accepted `Input` message payload (SEC-005). Larger payloads are
/// logged and dropped — the WS frame cap (16 MiB) bounds transport memory,
/// this bounds how much a single message can push at the PTY.
/// cap: Bytes accepted in one Input message payload from a streaming client.
const MAX_INPUT_PAYLOAD_BYTES: usize = 64 * 1024;
/// Maximum accepted `Paste` message payload (SEC-005). Pastes are bulk
/// transfers, so the cap is higher than single keystroke Input.
/// cap: Bytes accepted in one Paste message payload from a streaming client.
const MAX_PASTE_PAYLOAD_BYTES: usize = 256 * 1024;

/// How long a raw connection may take to complete its WebSocket (and TLS)
/// handshake before the server drops it (SEC-004). Pre-upgrade connections
/// are unauthenticated, so they must not be held indefinitely.
const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

// `StreamingServer` by concern (ARC-003): client/session bookkeeping,
// listeners, per-message handlers, the WebSocket session loop, and the
// `send_*` broadcast API. Construction and `start` stay here.
mod listen;
mod messages;
mod send;
mod sessions;
mod ws_session;

mod http;
use http::*;

/// Request/response types for the tungstenite WS handshake header callback,
/// aliased for readability (the `Callback` trait fixes these exactly).
type WsHandshakeRequest = tokio_tungstenite::tungstenite::http::Request<()>;
type WsHandshakeResponse = tokio_tungstenite::tungstenite::http::Response<()>;
type WsHandshakeErrorResponse = tokio_tungstenite::tungstenite::http::Response<Option<String>>;

// =============================================================================
// Terminal Size Validation
// =============================================================================

/// Minimum terminal columns
pub const MIN_COLS: u16 = 2;
/// Minimum terminal rows
pub const MIN_ROWS: u16 = 1;
/// Maximum terminal columns
/// cap: Columns a streaming client may request for its terminal.
pub const MAX_COLS: u16 = 1000;
/// Maximum terminal rows
/// cap: Rows a streaming client may request for its terminal.
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
#[derive(Debug, Clone)]
pub struct ConnectionParams {
    /// Session ID (defaults to "default")
    pub session_id: String,
    /// Whether this connection is read-only
    pub readonly: bool,
    /// Preset name to use for session creation
    pub preset: Option<String>,
}

/// Session ids must match `[A-Za-z0-9_-]{1,64}` (SEC-011): they are used as
/// registry keys and passed to the session factory, so both charset and
/// length are bounded to keep malformed ids out of the session machinery.
fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl ConnectionParams {
    /// Parse connection parameters from a query string map
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let session_id = params
            .get("session")
            .cloned()
            .unwrap_or_else(|| "default".to_string());
        // SEC-011: session ids are registry keys and factory arguments —
        // restrict them to a bounded `[A-Za-z0-9_-]` charset so malformed
        // ids (path-ish strings, oversized values) never reach the session
        // registry or the spawn factory. Invalid ids fall back to the
        // default session.
        let session_id = if is_valid_session_id(&session_id) {
            session_id
        } else {
            crate::debug_error!(
                "STREAMING",
                "Rejecting invalid session id {:?} (must match [A-Za-z0-9_-]{{1,64}}); using default",
                session_id
            );
            "default".to_string()
        };
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
// Client-message dispatch inputs
// =============================================================================

/// The per-connection identity every client-message handler reads: the
/// transport name and client id for logs, and whether the client may
/// write.
#[derive(Debug, Clone, Copy)]
struct ConnCtx<'a> {
    transport_label: &'a str,
    client_id: uuid::Uuid,
    read_only: bool,
}

/// The fields of a decoded `ClientMessage::Mouse` that
/// [`StreamingServer::handle_mouse`] turns into a mouse report.
#[derive(Debug, Clone, Copy)]
struct MouseInput {
    col: u16,
    row: u16,
    button: u8,
    shift: bool,
    ctrl: bool,
    alt: bool,
    event_type: crate::streaming::protocol::MouseEventType,
}

// =============================================================================
// WebSocket transports
// =============================================================================

/// One client connection as [`StreamingServer::run_ws_session`] sees it:
/// decoded client messages in, server messages out. Each transport keeps
/// its own frame policy (ping/pong, text frames, undecodable frames) inside
/// `recv`, so the shared session loop does not change either path's
/// behavior.
trait WsTransport: Send {
    /// This connection's client id.
    fn id(&self) -> uuid::Uuid;
    /// The next client message; `Ok(None)` once the peer has closed. An
    /// `Err` ends the session.
    fn recv(&mut self) -> impl std::future::Future<Output = Result<Option<ClientMessage>>> + Send;
    /// Send one server message.
    fn send(&mut self, msg: ServerMessage) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Send a keepalive ping.
    fn ping(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Complete the closing handshake (best effort).
    fn close(self) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// tungstenite transport (plain TCP or TLS): protobuf and ping/pong live in
/// [`Client`]. A text or undecodable frame ends the session.
impl<S> WsTransport for Client<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    fn id(&self) -> uuid::Uuid {
        Client::id(self)
    }

    fn recv(&mut self) -> impl std::future::Future<Output = Result<Option<ClientMessage>>> + Send {
        Client::recv(self)
    }

    fn send(&mut self, msg: ServerMessage) -> impl std::future::Future<Output = Result<()>> + Send {
        Client::send(self, msg)
    }

    fn ping(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        Client::ping(self)
    }

    fn close(self) -> impl std::future::Future<Output = Result<()>> + Send {
        Client::close(self)
    }
}

/// axum transport (the HTTP-served `/ws` route). Text and undecodable
/// frames are logged and skipped; the session continues.
struct AxumTransport {
    id: uuid::Uuid,
    socket: axum::extract::ws::WebSocket,
}

impl WsTransport for AxumTransport {
    fn id(&self) -> uuid::Uuid {
        self.id
    }

    async fn recv(&mut self) -> Result<Option<ClientMessage>> {
        use axum::extract::ws::Message as AxumMessage;
        loop {
            match self.socket.recv().await {
                Some(Ok(AxumMessage::Binary(data))) => match decode_client_message(&data) {
                    Ok(msg) => return Ok(Some(msg)),
                    Err(e) => {
                        crate::debug_error!("STREAMING", "Failed to parse client message: {}", e);
                    }
                },
                Some(Ok(AxumMessage::Text(_))) => {
                    crate::debug_error!(
                        "STREAMING",
                        "Text messages not supported, use binary protocol"
                    );
                }
                Some(Ok(AxumMessage::Ping(_) | AxumMessage::Pong(_))) => {}
                Some(Ok(AxumMessage::Close(_))) | None => return Ok(None),
                Some(Err(e)) => return Err(StreamingError::WebSocketError(e.to_string())),
            }
        }
    }

    async fn send(&mut self, msg: ServerMessage) -> Result<()> {
        let bytes = encode_server_message(&msg)?;
        self.socket
            .send(axum::extract::ws::Message::Binary(bytes.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))
    }

    async fn ping(&mut self) -> Result<()> {
        self.socket
            .send(axum::extract::ws::Message::Ping(vec![].into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))
    }

    /// When the client initiated the close, tungstenite already queued our
    /// reply when it read their Close frame and a further send is
    /// rejected; `flush()` writes the queued reply instead.
    async fn close(mut self) -> Result<()> {
        use futures_util::SinkExt;
        if self
            .socket
            .send(axum::extract::ws::Message::Close(None))
            .await
            .is_err()
        {
            self.socket
                .flush()
                .await
                .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        }
        Ok(())
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
    /// par-mux roster watcher; when set, every client receives the roster on connect
    #[cfg(feature = "mux")]
    roster: std::sync::OnceLock<Arc<crate::streaming::RosterWatcher>>,
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

        // Apply the configured Kitty file-media gate to the caller-supplied
        // terminal too (SEC-114, matching the session-factory path below):
        // PTY output is untrusted input, so `t=t`/`t=f` APCs must not
        // read/delete arbitrary files just because this constructor bypassed
        // the factory.
        terminal
            .write()
            .set_allow_file_media(config.kitty_file_media);

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
            #[cfg(feature = "mux")]
            roster: std::sync::OnceLock::new(),
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
            #[cfg(feature = "mux")]
            roster: std::sync::OnceLock::new(),
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

    /// Start the streaming server
    pub async fn start(self: Arc<Self>) -> Result<()> {
        // SEC-202: the query-string key exists only because browser WebSocket
        // clients cannot set auth headers; it leaks into proxy logs and history.
        if self.config.allow_api_key_in_query && self.config.api_key.is_some() {
            log::warn!(
                "API key authentication via ?api_key= query parameter is enabled; \
                 query strings are recorded in proxy/access logs and browser history. \
                 Prefer the Authorization or X-API-Key header where the client allows it."
            );
        }
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

    // -- Backward-compatible send helpers (route to default session) --
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
        | ServerMessage::AgentRoster { .. }
        | ServerMessage::AgentStateChanged { .. }
        | ServerMessage::Pong => true,
    }
}

#[cfg(test)]
mod origin_tests;

impl std::fmt::Debug for StreamingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingServer")
            .field("addr", &self.addr)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
mod tests;
