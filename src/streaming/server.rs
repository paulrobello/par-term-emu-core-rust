//! WebSocket streaming server implementation

use crate::mouse::{MouseEncoding, MouseMode};
use crate::streaming::client::Client;
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::proto::{decode_client_message, encode_server_message};
use crate::streaming::protocol::{ServerMessage, ThemeInfo};
use crate::terminal::{SelectionMode, Terminal};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_hdr_async;

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
#[derive(Debug)]
pub struct TlsConfig {
    /// Certificate chain in DER format
    pub certs: Vec<CertificateDer<'static>>,
    /// Private key in DER format
    pub key: PrivateKeyDer<'static>,
}

impl Clone for TlsConfig {
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl TlsConfig {
    /// Create TLS config from separate certificate and private key PEM files
    ///
    /// # Arguments
    /// * `cert_path` - Path to certificate PEM file (may contain certificate chain)
    /// * `key_path` - Path to private key PEM file
    ///
    /// # Errors
    /// Returns error if files cannot be read or parsed
    pub fn from_files<P: AsRef<Path>>(cert_path: P, key_path: P) -> Result<Self> {
        let cert_path = cert_path.as_ref();
        let key_path = key_path.as_ref();

        // Load certificates
        let cert_file = File::open(cert_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open certificate file '{}': {}",
                cert_path.display(),
                e
            ))
        })?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| {
                StreamingError::ServerError(format!(
                    "Failed to parse certificate file '{}': {}",
                    cert_path.display(),
                    e
                ))
            })?;

        if certs.is_empty() {
            return Err(StreamingError::ServerError(format!(
                "No certificates found in '{}'",
                cert_path.display()
            )));
        }

        // Load private key
        let key_file = File::open(key_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open key file '{}': {}",
                key_path.display(),
                e
            ))
        })?;
        let mut key_reader = BufReader::new(key_file);
        let key = rustls_pemfile::private_key(&mut key_reader)
            .map_err(|e| {
                StreamingError::ServerError(format!(
                    "Failed to parse key file '{}': {}",
                    key_path.display(),
                    e
                ))
            })?
            .ok_or_else(|| {
                StreamingError::ServerError(format!(
                    "No private key found in '{}'",
                    key_path.display()
                ))
            })?;

        Ok(Self { certs, key })
    }

    /// Create TLS config from a single PEM file containing both certificate and key
    ///
    /// # Arguments
    /// * `pem_path` - Path to PEM file containing certificate chain and private key
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed
    pub fn from_pem<P: AsRef<Path>>(pem_path: P) -> Result<Self> {
        let pem_path = pem_path.as_ref();

        let pem_file = File::open(pem_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open PEM file '{}': {}",
                pem_path.display(),
                e
            ))
        })?;
        let mut reader = BufReader::new(pem_file);

        // Read all items from PEM file
        let mut certs: Vec<CertificateDer<'static>> = Vec::new();
        let mut key: Option<PrivateKeyDer<'static>> = None;

        for item in rustls_pemfile::read_all(&mut reader) {
            match item {
                Ok(rustls_pemfile::Item::X509Certificate(cert)) => {
                    certs.push(cert);
                }
                Ok(rustls_pemfile::Item::Pkcs1Key(k)) => {
                    key = Some(PrivateKeyDer::Pkcs1(k));
                }
                Ok(rustls_pemfile::Item::Pkcs8Key(k)) => {
                    key = Some(PrivateKeyDer::Pkcs8(k));
                }
                Ok(rustls_pemfile::Item::Sec1Key(k)) => {
                    key = Some(PrivateKeyDer::Sec1(k));
                }
                Ok(_) => {
                    // Ignore other items (CRLs, etc.)
                }
                Err(e) => {
                    return Err(StreamingError::ServerError(format!(
                        "Failed to parse PEM file '{}': {}",
                        pem_path.display(),
                        e
                    )));
                }
            }
        }

        if certs.is_empty() {
            return Err(StreamingError::ServerError(format!(
                "No certificates found in '{}'",
                pem_path.display()
            )));
        }

        let key = key.ok_or_else(|| {
            StreamingError::ServerError(format!("No private key found in '{}'", pem_path.display()))
        })?;

        Ok(Self { certs, key })
    }

    /// Build a rustls ServerConfig from this TLS configuration
    fn build_rustls_config(&self) -> Result<RustlsServerConfig> {
        RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), self.key.clone_key())
            .map_err(|e| StreamingError::ServerError(format!("Failed to build TLS config: {}", e)))
    }
}

/// HTTP Basic Authentication configuration
///
/// Supports password verification via:
/// - Clear text comparison
/// - htpasswd hash formats: bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
#[derive(Debug, Clone)]
pub struct HttpBasicAuthConfig {
    /// Username for authentication
    pub username: String,
    /// Password storage - either clear text or htpasswd hash
    pub password: PasswordConfig,
}

/// Password storage configuration
#[derive(Debug, Clone)]
pub enum PasswordConfig {
    /// Clear text password (compared directly)
    ClearText(String),
    /// htpasswd format hash (bcrypt, apr1, sha1, md5crypt)
    Hash(String),
}

impl HttpBasicAuthConfig {
    /// Create a new HTTP Basic Auth config with clear text password
    pub fn with_password(username: String, password: String) -> Self {
        Self {
            username,
            password: PasswordConfig::ClearText(password),
        }
    }

    /// Create a new HTTP Basic Auth config with htpasswd hash
    pub fn with_hash(username: String, hash: String) -> Self {
        Self {
            username,
            password: PasswordConfig::Hash(hash),
        }
    }

    /// Verify a password against this config
    pub fn verify(&self, username: &str, password: &str) -> bool {
        if username != self.username {
            return false;
        }

        match &self.password {
            PasswordConfig::ClearText(expected) => password == expected,
            PasswordConfig::Hash(hash) => {
                // Use htpasswd-verify crate to check the password
                // Format: "username:hash" for htpasswd library
                let htpasswd_line = format!("{}:{}", self.username, hash);
                let htpasswd = htpasswd_verify::Htpasswd::from(htpasswd_line.as_str());
                htpasswd.check(username, password)
            }
        }
    }
}

/// Configuration for the streaming server
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Maximum number of concurrent clients
    pub max_clients: usize,
    /// Whether to send initial screen content on connect
    pub send_initial_screen: bool,
    /// Keepalive ping interval in seconds (0 = disabled)
    pub keepalive_interval: u64,
    /// Default mode for new clients (true = read-only, false = read-write)
    pub default_read_only: bool,
    /// Enable HTTP static file serving
    pub enable_http: bool,
    /// Web root directory for static files (default: "./web_term")
    pub web_root: String,
    /// Initial terminal columns (0 = use terminal's current size)
    pub initial_cols: u16,
    /// Initial terminal rows (0 = use terminal's current size)
    pub initial_rows: u16,
    /// TLS configuration for secure connections (None = no TLS)
    pub tls: Option<TlsConfig>,
    /// HTTP Basic Authentication configuration (None = no auth)
    pub http_basic_auth: Option<HttpBasicAuthConfig>,
    /// Maximum number of concurrent sessions (default: 10)
    pub max_sessions: usize,
    /// Idle session timeout in seconds (0 = never timeout, default: 300)
    pub session_idle_timeout: u64,
    /// Shell presets: name → shell command
    pub presets: HashMap<String, String>,
    /// Maximum clients per session (0 = unlimited)
    pub max_clients_per_session: usize,
    /// Input rate limit in bytes per second (0 = unlimited)
    pub input_rate_limit_bytes_per_sec: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_clients: 1000,
            send_initial_screen: true,
            keepalive_interval: 30,
            default_read_only: false,
            enable_http: false,
            web_root: "./web_term".to_string(),
            initial_cols: 0,
            initial_rows: 0,
            tls: None,
            http_basic_auth: None,
            max_sessions: 10,
            session_idle_timeout: 900,
            presets: HashMap::new(),
            max_clients_per_session: 0,
            input_rate_limit_bytes_per_sec: 0,
        }
    }
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

/// Get current time as epoch milliseconds
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// =============================================================================
// Session Metrics
// =============================================================================

/// Per-session metrics for observability
pub struct SessionMetrics {
    /// Total messages sent to clients
    pub messages_sent: AtomicUsize,
    /// Total output bytes sent to clients
    pub bytes_sent: AtomicUsize,
    /// Total input bytes received from clients
    pub input_bytes: AtomicUsize,
    /// Total errors encountered
    pub errors: AtomicUsize,
    /// Total messages dropped (e.g., no receivers)
    pub dropped_messages: AtomicUsize,
    /// Last broadcast time (epoch millis)
    pub last_broadcast_time: AtomicU64,
}

impl SessionMetrics {
    /// Create new zeroed metrics
    fn new() -> Self {
        Self {
            messages_sent: AtomicUsize::new(0),
            bytes_sent: AtomicUsize::new(0),
            input_bytes: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            dropped_messages: AtomicUsize::new(0),
            last_broadcast_time: AtomicU64::new(0),
        }
    }
}

// =============================================================================
// Session State
// =============================================================================

/// Per-session state extracted from StreamingServer for multi-session support
pub struct SessionState {
    /// Unique session identifier
    pub id: String,
    /// Terminal instance for this session
    pub terminal: Arc<Mutex<Terminal>>,
    /// Broadcast channel for sending output to all clients in this session
    broadcast_tx: broadcast::Sender<ServerMessage>,
    /// Channel for sending output data into the broadcaster loop (bounded for backpressure)
    output_tx: mpsc::Sender<String>,
    /// Receiver end of the output channel (consumed by broadcaster loop)
    output_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>,
    /// PTY writer for sending client input (optional, only set if PTY is available)
    #[allow(clippy::type_complexity)]
    pty_writer: std::sync::RwLock<Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>>>,
    /// Channel for sending resize requests
    resize_tx: mpsc::UnboundedSender<(u16, u16)>,
    /// Receiver for resize requests
    resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
    /// Number of clients connected to this session
    client_count: AtomicUsize,
    /// When the last client disconnected (for idle timeout)
    last_client_disconnect: parking_lot::RwLock<Option<tokio::time::Instant>>,
    /// When this session was created (Unix epoch seconds)
    created_at: u64,
    /// Shutdown signal for this session's broadcaster loop
    shutdown: Arc<tokio::sync::Notify>,
    /// Optional theme for this session
    theme: Option<ThemeInfo>,
    /// Whether to send initial screen content on connect
    send_initial_screen: bool,
    /// Per-session metrics
    pub metrics: SessionMetrics,
}

impl SessionState {
    /// Create a new session state
    pub fn new(
        id: String,
        terminal: Arc<Mutex<Terminal>>,
        theme: Option<ThemeInfo>,
        send_initial_screen: bool,
    ) -> Self {
        let (output_tx, output_rx) = mpsc::channel(1000);
        let (broadcast_tx, _) = broadcast::channel(100);
        let (resize_tx, resize_rx) = mpsc::unbounded_channel();

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id,
            terminal,
            broadcast_tx,
            output_tx,
            output_rx: Arc::new(tokio::sync::Mutex::new(output_rx)),
            pty_writer: std::sync::RwLock::new(None),
            resize_tx,
            resize_rx: Arc::new(tokio::sync::Mutex::new(resize_rx)),
            client_count: AtomicUsize::new(0),
            last_client_disconnect: parking_lot::RwLock::new(None),
            created_at,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            theme,
            send_initial_screen,
            metrics: SessionMetrics::new(),
        }
    }

    /// Try to add a client to this session. Returns true if successful.
    /// When `max_per_session > 0`, uses CAS loop to enforce the limit atomically.
    pub fn try_add_client(&self, max_per_session: usize) -> bool {
        if max_per_session == 0 {
            self.client_count.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        loop {
            let current = self.client_count.load(Ordering::Relaxed);
            if current >= max_per_session {
                return false;
            }
            if self
                .client_count
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Remove a client from this session.
    pub fn remove_client(&self) {
        let prev = self.client_count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Was the last client - record disconnect time
            *self.last_client_disconnect.write() = Some(tokio::time::Instant::now());
        }
    }

    /// Build a Connected message from current terminal state
    pub fn build_connect_message(&self, client_id: &str, readonly: bool) -> ServerMessage {
        let terminal = self.terminal.lock();
        let (cols, rows) = terminal.size();

        let initial_screen = if self.send_initial_screen {
            Some(terminal.export_visible_screen_styled())
        } else {
            None
        };

        let badge = terminal.evaluate_badge();
        let faint_alpha = Some(terminal.faint_text_alpha());
        let cwd = terminal.current_directory().map(|s| s.to_string());
        let mok_mode = Some(terminal.modify_other_keys_mode() as u32);

        ServerMessage::connected_full(
            cols as u16,
            rows as u16,
            initial_screen,
            self.id.clone(),
            self.theme.clone(),
            badge,
            faint_alpha,
            cwd,
            mok_mode,
            Some(client_id.to_string()),
            Some(readonly),
        )
    }

    /// Build ModeChanged messages for all active (non-default) terminal modes.
    ///
    /// Used to sync terminal mode state to clients connecting to existing sessions.
    /// Returns a list of `ServerMessage::ModeChanged` for each mode that differs
    /// from its default value.
    pub fn build_mode_sync_messages(&self) -> Vec<ServerMessage> {
        let terminal = self.terminal.lock();
        let mut messages = Vec::new();

        // Mouse tracking mode
        let mouse_mode = terminal.mouse_mode();
        if mouse_mode != MouseMode::Off {
            let mode_name = match mouse_mode {
                MouseMode::X10 => "mouse_x10",
                MouseMode::Normal => "mouse_normal",
                MouseMode::ButtonEvent => "mouse_button_event",
                MouseMode::AnyEvent => "mouse_any_event",
                MouseMode::Off => unreachable!(),
            };
            messages.push(ServerMessage::mode_changed(mode_name.to_string(), true));
        }

        // Mouse encoding (if not default)
        let mouse_encoding = terminal.mouse_encoding();
        if mouse_encoding != MouseEncoding::Default {
            let encoding_name = match mouse_encoding {
                MouseEncoding::Utf8 => "mouse_utf8",
                MouseEncoding::Sgr => "mouse_sgr",
                MouseEncoding::Urxvt => "mouse_urxvt",
                MouseEncoding::Default => unreachable!(),
            };
            messages.push(ServerMessage::mode_changed(encoding_name.to_string(), true));
        }

        // Bracketed paste mode (DECSET 2004)
        if terminal.bracketed_paste() {
            messages.push(ServerMessage::mode_changed(
                "bracketed_paste".to_string(),
                true,
            ));
        }

        // Application cursor keys (DECCKM)
        if terminal.application_cursor() {
            messages.push(ServerMessage::mode_changed(
                "application_cursor".to_string(),
                true,
            ));
        }

        // Focus tracking (DECSET 1004)
        if terminal.focus_tracking() {
            messages.push(ServerMessage::mode_changed(
                "focus_tracking".to_string(),
                true,
            ));
        }

        // Cursor visibility (DECTCEM) - default is visible, so send if hidden
        if !terminal.cursor().visible {
            messages.push(ServerMessage::mode_changed(
                "cursor_visible".to_string(),
                false,
            ));
        }

        // Alternate screen buffer
        if terminal.is_alt_screen_active() {
            messages.push(ServerMessage::mode_changed(
                "alternate_screen".to_string(),
                true,
            ));
        }

        // Origin mode (DECOM)
        if terminal.origin_mode() {
            messages.push(ServerMessage::mode_changed("origin_mode".to_string(), true));
        }

        // Insert mode (IRM)
        if terminal.insert_mode() {
            messages.push(ServerMessage::mode_changed("insert_mode".to_string(), true));
        }

        // Auto-wrap mode (DECAWM) - default is true, so send if disabled
        if !terminal.auto_wrap_mode() {
            messages.push(ServerMessage::mode_changed("auto_wrap".to_string(), false));
        }

        messages
    }

    /// Set the PTY writer for handling client input
    pub fn set_pty_writer(&self, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>) {
        if let Ok(mut guard) = self.pty_writer.write() {
            *guard = Some(writer);
        }
    }

    /// Get a clone of the output sender channel
    pub fn get_output_sender(&self) -> mpsc::Sender<String> {
        self.output_tx.clone()
    }

    /// Get a clone of the resize receiver
    pub fn get_resize_receiver(
        &self,
    ) -> Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>> {
        Arc::clone(&self.resize_rx)
    }

    /// Broadcast a message to all clients in this session
    pub fn broadcast(&self, msg: ServerMessage) {
        match self.broadcast_tx.send(msg) {
            Ok(_) => {
                self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.metrics
                    .dropped_messages
                    .fetch_add(1, Ordering::Relaxed);
                // No receivers — normal when 0 clients connected
            }
        }
    }

    /// Run the output broadcaster loop for this session
    pub async fn output_broadcaster_loop(&self) {
        let mut rx = self.output_rx.lock().await;
        let mut buffer = String::new();
        let mut last_flush = tokio::time::Instant::now();

        const BATCH_WINDOW: Duration = Duration::from_millis(16);
        const MAX_BATCH_SIZE: usize = 8192;

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    crate::debug_info!("STREAMING", "Session {} broadcaster received shutdown signal", self.id);
                    if !buffer.is_empty() {
                        let data_len = buffer.len();
                        let msg = ServerMessage::output(buffer);
                        self.broadcast(msg);
                        self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                    }
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Some(data) => {
                            if !data.is_empty() {
                                buffer.push_str(&data);
                                if buffer.len() > MAX_BATCH_SIZE {
                                    let data_len = buffer.len();
                                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                                    self.broadcast(msg);
                                    self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                                    self.metrics.last_broadcast_time.store(now_millis(), Ordering::Relaxed);
                                    last_flush = tokio::time::Instant::now();
                                }
                            }
                        }
                        None => {
                            if !buffer.is_empty() {
                                let data_len = buffer.len();
                                let msg = ServerMessage::output(buffer);
                                self.broadcast(msg);
                                self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                            }
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(last_flush + BATCH_WINDOW), if !buffer.is_empty() => {
                    let data_len = buffer.len();
                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                    self.broadcast(msg);
                    self.metrics.bytes_sent.fetch_add(data_len, Ordering::Relaxed);
                    self.metrics.last_broadcast_time.store(now_millis(), Ordering::Relaxed);
                    last_flush = tokio::time::Instant::now();
                }
            }
        }
    }

    /// Signal this session to shut down
    pub fn shutdown(&self, reason: String) {
        crate::debug_info!("STREAMING", "Shutting down session {}: {}", self.id, reason);
        let msg = ServerMessage::shutdown(reason);
        self.broadcast(msg);
        self.shutdown.notify_waiters();
    }

    /// Get the number of clients connected to this session
    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::Relaxed)
    }

    /// Check if this session is idle (no clients and past timeout)
    pub fn is_idle(&self, timeout: Duration) -> bool {
        if self.client_count() > 0 {
            return false;
        }
        if let Some(last_disconnect) = *self.last_client_disconnect.read() {
            last_disconnect.elapsed() >= timeout
        } else {
            false
        }
    }

    /// Get session info for the /sessions endpoint
    pub fn session_info(&self) -> SessionInfo {
        let terminal = self.terminal.lock();
        let (cols, rows) = terminal.size();
        let cwd = terminal.current_directory().map(|s| s.to_string());

        let idle_seconds = if self.client_count() == 0 {
            self.last_client_disconnect
                .read()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0)
        } else {
            0
        };

        SessionInfo {
            id: self.id.clone(),
            created: self.created_at,
            clients: self.client_count(),
            idle_seconds,
            cols: cols as u16,
            rows: rows as u16,
            cwd,
            messages_sent: self.metrics.messages_sent.load(Ordering::Relaxed),
            bytes_sent: self.metrics.bytes_sent.load(Ordering::Relaxed),
            input_bytes: self.metrics.input_bytes.load(Ordering::Relaxed),
            errors: self.metrics.errors.load(Ordering::Relaxed),
            dropped_messages: self.metrics.dropped_messages.load(Ordering::Relaxed),
        }
    }
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("id", &self.id)
            .field("client_count", &self.client_count())
            .field("created_at", &self.created_at)
            .field("send_initial_screen", &self.send_initial_screen)
            .finish()
    }
}

/// Session information returned by the /sessions endpoint
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    /// Session identifier
    pub id: String,
    /// Creation timestamp (Unix epoch seconds)
    pub created: u64,
    /// Number of connected clients
    pub clients: usize,
    /// Seconds since last client disconnected (0 if clients are connected)
    pub idle_seconds: u64,
    /// Terminal columns
    pub cols: u16,
    /// Terminal rows
    pub rows: u16,
    /// Current working directory
    pub cwd: Option<String>,
    /// Total messages sent to clients
    pub messages_sent: usize,
    /// Total output bytes sent
    pub bytes_sent: usize,
    /// Total input bytes received
    pub input_bytes: usize,
    /// Total errors encountered
    pub errors: usize,
    /// Total messages dropped
    pub dropped_messages: usize,
}

// =============================================================================
// Session Registry
// =============================================================================

/// Thread-safe registry of active sessions
pub struct SessionRegistry {
    sessions: parking_lot::RwLock<HashMap<String, Arc<SessionState>>>,
    max_sessions: usize,
}

impl SessionRegistry {
    /// Create a new session registry
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: parking_lot::RwLock::new(HashMap::new()),
            max_sessions,
        }
    }

    /// Get a session by ID
    pub fn get(&self, id: &str) -> Option<Arc<SessionState>> {
        self.sessions.read().get(id).cloned()
    }

    /// Insert a session. Returns error if max_sessions would be exceeded.
    pub fn insert(&self, id: String, session: Arc<SessionState>) -> Result<()> {
        let mut sessions = self.sessions.write();
        if sessions.len() >= self.max_sessions && !sessions.contains_key(&id) {
            return Err(StreamingError::MaxSessionsReached);
        }
        sessions.insert(id, session);
        Ok(())
    }

    /// Remove a session by ID
    pub fn remove(&self, id: &str) -> Option<Arc<SessionState>> {
        self.sessions.write().remove(id)
    }

    /// Get the number of active sessions
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Get IDs of sessions that are idle past the given timeout
    pub fn idle_sessions(&self, timeout: Duration) -> Vec<String> {
        self.sessions
            .read()
            .iter()
            .filter(|(_, s)| s.is_idle(timeout))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// List all sessions for the /sessions endpoint
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.sessions
            .read()
            .values()
            .map(|s| s.session_info())
            .collect()
    }
}

// =============================================================================
// Session Factory
// =============================================================================

/// Result returned by SessionFactory::create_session
pub struct SessionFactoryResult {
    /// The terminal instance for the new session
    pub terminal: Arc<Mutex<Terminal>>,
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
        session: &Arc<SessionState>,
    ) -> std::result::Result<(), StreamingError>;

    /// Teardown a session (e.g., kill PTY process)
    fn teardown_session(&self, session_id: &str);

    /// Check if a session's backing process is still alive
    fn is_session_alive(&self, _session_id: &str) -> bool {
        true
    }
}

/// Default session factory that wraps a single pre-existing terminal.
/// Only allows the "default" session. Used for backward compatibility.
#[allow(dead_code)]
struct DefaultSessionFactory {
    terminal: Arc<Mutex<Terminal>>,
}

impl SessionFactory for DefaultSessionFactory {
    fn create_session(
        &self,
        session_id: &str,
        _cols: u16,
        _rows: u16,
        _shell_command: Option<&str>,
    ) -> std::result::Result<SessionFactoryResult, StreamingError> {
        if session_id != "default" {
            return Err(StreamingError::SessionNotFound(session_id.to_string()));
        }
        Ok(SessionFactoryResult {
            terminal: Arc::clone(&self.terminal),
            pty_writer: None,
        })
    }

    fn setup_session(
        &self,
        _session_id: &str,
        _session: &Arc<SessionState>,
    ) -> std::result::Result<(), StreamingError> {
        Ok(())
    }

    fn teardown_session(&self, _session_id: &str) {}
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
// Input Rate Limiter
// =============================================================================

/// Token bucket rate limiter for per-client input
struct InputRateLimiter {
    tokens: f64,
    max_tokens: f64,
    rate: f64,
    last_check: tokio::time::Instant,
}

impl InputRateLimiter {
    /// Create a new rate limiter with the given bytes-per-second rate.
    /// Burst capacity is 2x the rate.
    fn new(bytes_per_sec: usize) -> Self {
        let rate = bytes_per_sec as f64;
        let max_tokens = rate * 2.0;
        Self {
            tokens: max_tokens,
            max_tokens,
            rate,
            last_check: tokio::time::Instant::now(),
        }
    }

    /// Try to consume `bytes` tokens. Returns true if allowed.
    fn try_consume(&mut self, bytes: usize) -> bool {
        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(self.last_check).as_secs_f64();
        self.last_check = now;

        // Replenish tokens
        self.tokens = (self.tokens + elapsed * self.rate).min(self.max_tokens);

        let cost = bytes as f64;
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

// =============================================================================
// Guards
// =============================================================================

/// Guard that decrements session client count when dropped
struct SessionClientGuard {
    session: Arc<SessionState>,
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
    default_session: Option<Arc<SessionState>>,
}

impl StreamingServer {
    /// Create a new streaming server (backward-compatible single-session mode)
    pub fn new(terminal: Arc<Mutex<Terminal>>, addr: String) -> Self {
        Self::with_config(terminal, addr, StreamingConfig::default())
    }

    /// Create a new streaming server with custom configuration (backward-compatible)
    pub fn with_config(
        terminal: Arc<Mutex<Terminal>>,
        addr: String,
        config: StreamingConfig,
    ) -> Self {
        let sessions = SessionRegistry::new(config.max_sessions);

        // Create default session
        let default_session = Arc::new(SessionState::new(
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
            // We can't directly modify the theme on SessionState without interior mutability,
            // but new sessions created by the factory will pick up the theme from
            // resolve_session. For the default session created in with_config, the theme
            // is set at construction time. Since set_theme is called before start(), we
            // need to recreate the default session with the theme.
            // However, the simplest approach is to store theme on the server and use it
            // when building connect messages from the default session.
            let _ = session; // Theme is used via server.theme in build_connect_message fallback
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
    pub fn get_session(&self, session_id: &str) -> Option<Arc<SessionState>> {
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
    ) -> Result<Arc<SessionState>> {
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

            let session = Arc::new(SessionState::new(
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

        // Build router
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/sessions", get(sessions_handler))
            .fallback_service(ServeDir::new(&self.config.web_root))
            .with_state(self.clone());

        // Add basic auth middleware if configured
        let app = if let Some(ref auth_config) = self.config.http_basic_auth {
            let auth_config = auth_config.clone();
            app.layer(axum::middleware::from_fn(move |req, next| {
                let auth_config = auth_config.clone();
                basic_auth_middleware(req, next, auth_config)
            }))
        } else {
            app
        };

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

        // Build router
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/sessions", get(sessions_handler))
            .fallback_service(ServeDir::new(&self.config.web_root))
            .with_state(self.clone());

        // Add basic auth middleware if configured
        let app = if let Some(ref auth_config) = self.config.http_basic_auth {
            let auth_config = auth_config.clone();
            app.layer(axum::middleware::from_fn(move |req, next| {
                let auth_config = auth_config.clone();
                basic_auth_middleware(req, next, auth_config)
            }))
        } else {
            app
        };

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
                        // Accept WebSocket with header callback to capture URI query
                        let uri_query = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
                        let uri_query_clone = std::sync::Arc::clone(&uri_query);

                        let ws_result = accept_hdr_async(stream, move |req: &tokio_tungstenite::tungstenite::http::Request<()>, resp: tokio_tungstenite::tungstenite::http::Response<()>| {
                            if let Some(q) = req.uri().query() {
                                *uri_query_clone.lock().unwrap() = Some(q.to_string());
                            }
                            Ok(resp)
                        }).await;

                        match ws_result {
                            Ok(ws_stream) => {
                                let query_str = uri_query.lock().unwrap().take();
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
                                // Accept WebSocket with header callback to capture URI query
                                let uri_query =
                                    std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
                                let uri_query_clone = std::sync::Arc::clone(&uri_query);

                                let ws_result = accept_hdr_async(
                                    tls_stream,
                                    move |req: &tokio_tungstenite::tungstenite::http::Request<
                                        (),
                                    >,
                                          resp: tokio_tungstenite::tungstenite::http::Response<
                                        (),
                                    >| {
                                        if let Some(q) = req.uri().query() {
                                            *uri_query_clone.lock().unwrap() = Some(q.to_string());
                                        }
                                        Ok(resp)
                                    },
                                )
                                .await;

                                match ws_result {
                                    Ok(ws_stream) => {
                                        let query_str = uri_query.lock().unwrap().take();
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

    /// Handle a new WebSocket connection (already upgraded)
    async fn handle_connection_ws(
        self: &Arc<Self>,
        ws_stream: tokio_tungstenite::WebSocketStream<TcpStream>,
        params: &ConnectionParams,
    ) -> Result<()> {
        // Resolve session first (before reserving client slots)
        let session = self.resolve_session(params)?;

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

        // Determine readonly
        let read_only = params.readonly || self.config.default_read_only;

        let mut client = Client::new(ws_stream, read_only);
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
            "Client {} connected to session {} (total: {})",
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
                            crate::debug_error!("STREAMING", "Client {} error: {}", client_id, e);
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
                                            crate::debug_error!("STREAMING", "Rate limit exceeded for client {}", client_id);
                                            continue;
                                        }
                                    }
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        session.metrics.input_bytes.fetch_add(data.len(), Ordering::Relaxed);
                                        if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                            use std::io::Write;
                                            if let Err(e) = w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
                                                crate::debug_error!("STREAMING", "PTY write error for session {}: {}", session.id, e);
                                                session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                    if let Err(e) = validate_terminal_size(cols, rows) {
                                        crate::debug_error!("STREAMING", "Client {} sent invalid resize: {}", client_id, e);
                                    } else {
                                        let _ = session.resize_tx.send((cols, rows));
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Ping => {
                                    if let Err(e) = client.send(ServerMessage::pong()).await {
                                        crate::debug_error!("STREAMING", "Failed to send pong to client {}: {}", client_id, e);
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                    let refresh_msg = {
                                        if let Ok(terminal) = Ok::<_, ()>(terminal_for_refresh.lock()) {
                                            let content = terminal.export_visible_screen_styled();
                                            let (cols, rows) = terminal.size();
                                            Some(ServerMessage::refresh(cols as u16, rows as u16, content))
                                        } else {
                                            None
                                        }
                                    };
                                    if let Some(msg) = refresh_msg {
                                        if let Err(e) = client.send(msg).await {
                                            crate::debug_error!("STREAMING", "Failed to send refresh to client {}: {}", client_id, e);
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
                                            let mut terminal = session.terminal.lock();
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
                                            if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                use std::io::Write;
                                                if let Err(e) = w.write_all(&bytes).and_then(|_| w.flush()) {
                                                    crate::debug_error!("STREAMING", "PTY mouse write error for session {}: {}", session.id, e);
                                                    session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::FocusChange { focused } => {
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        let bytes = {
                                            let terminal = session.terminal.lock();
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
                                            if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                use std::io::Write;
                                                if let Err(e) = w.write_all(&bytes).and_then(|_| w.flush()) {
                                                    crate::debug_error!("STREAMING", "PTY focus write error for session {}: {}", session.id, e);
                                                    session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Paste { content } => {
                                    if read_only { continue; }
                                    if let Some(ref mut limiter) = rate_limiter {
                                        if !limiter.try_consume(content.len()) {
                                            crate::debug_error!("STREAMING", "Rate limit exceeded for client {}", client_id);
                                            continue;
                                        }
                                    }
                                    if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        let terminal = session.terminal.lock();
                                        session.metrics.input_bytes.fetch_add(content.len(), Ordering::Relaxed);
                                        if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
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
                                }
                                crate::streaming::protocol::ClientMessage::SelectionRequest {
                                    start_col, start_row, end_col, end_row, mode,
                                } => {
                                    let selection_msg = {
                                        let mut terminal = session.terminal.lock();
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
                                                let mut terminal = session.terminal.lock();
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
                                                let terminal = session.terminal.lock();
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
                            }
                        }
                        None => {
                            crate::debug_info!("STREAMING", "Client {} disconnected from session {}", client_id, session.id);
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
                        crate::debug_error!("STREAMING", "Failed to ping client {}: {}", client_id, e);
                        break;
                    }
                }
            }
        }

        crate::debug_info!(
            "STREAMING",
            "Client {} cleanup (remaining: {})",
            client_id,
            self.client_count() - 1
        );

        Ok(())
    }

    /// Handle a new TLS WebSocket connection (already upgraded)
    async fn handle_tls_connection_ws(
        self: &Arc<Self>,
        ws_stream: tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>,
        params: &ConnectionParams,
    ) -> Result<()> {
        // Resolve session first
        let session = self.resolve_session(params)?;

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

        // Send initial connection message
        let connect_msg = session.build_connect_message(&client_id.to_string(), read_only);

        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        let msg_bytes = encode_server_message(&connect_msg)?;
        ws_tx
            .send(Message::Binary(msg_bytes.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        // Sync terminal mode state for existing sessions
        for mode_msg in session.build_mode_sync_messages() {
            let mode_bytes = encode_server_message(&mode_msg)?;
            ws_tx
                .send(Message::Binary(mode_bytes.into()))
                .await
                .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;
        }

        crate::debug_info!(
            "STREAMING",
            "TLS Client {} connected to session {} (total: {})",
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
                        Some(Ok(Message::Binary(data))) => {
                            match decode_client_message(&data) {
                                Ok(client_msg) => {
                                    match client_msg {
                                        crate::streaming::protocol::ClientMessage::Input { data } => {
                                            if read_only {
                                                continue;
                                            }
                                            if let Some(ref mut limiter) = rate_limiter {
                                                if !limiter.try_consume(data.len()) {
                                                    crate::debug_error!("STREAMING", "Rate limit exceeded for TLS client {}", client_id);
                                                    continue;
                                                }
                                            }
                                            if let Some(writer) = session.pty_writer.read().ok().and_then(|g| g.clone()) {
                                                session.metrics.input_bytes.fetch_add(data.len(), Ordering::Relaxed);
                                                if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                    use std::io::Write;
                                                    if let Err(e) = w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
                                                        crate::debug_error!("STREAMING", "PTY write error for TLS session {}: {}", session.id, e);
                                                        session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                                    }
                                                }
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                            if let Err(e) = validate_terminal_size(cols, rows) {
                                                crate::debug_error!("STREAMING", "TLS client {} sent invalid resize: {}", client_id, e);
                                            } else {
                                                let _ = resize_tx.send((cols, rows));
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Ping => {
                                            if let Ok(bytes) = encode_server_message(&ServerMessage::pong()) {
                                                let _ = ws_tx.send(Message::Binary(bytes.into())).await;
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                            let refresh_msg = {
                                                if let Ok(terminal) = Ok::<_, ()>(terminal_for_refresh.lock()) {
                                                    let content = terminal.export_visible_screen_styled();
                                                    let (cols, rows) = terminal.size();
                                                    Some(ServerMessage::refresh(cols as u16, rows as u16, content))
                                                } else {
                                                    None
                                                }
                                            };
                                            if let Some(msg) = refresh_msg {
                                                if let Ok(bytes) = encode_server_message(&msg) {
                                                    let _ = ws_tx.send(Message::Binary(bytes.into())).await;
                                                }
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
                                    crate::debug_error!("STREAMING", "Failed to parse TLS client message: {}", e);
                                }
                            }
                        }
                        Some(Ok(Message::Text(_))) => {
                            crate::debug_error!("STREAMING", "Text messages not supported, use binary protocol");
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let _ = ws_tx.send(Message::Pong(data)).await;
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) | None => {
                            crate::debug_info!("STREAMING", "TLS Client {} disconnected from session {}", client_id, session.id);
                            break;
                        }
                        Some(Ok(Message::Frame(_))) => {}
                        Some(Err(e)) => {
                            crate::debug_error!("STREAMING", "TLS WebSocket error: {}", e);
                            break;
                        }
                    }
                }

                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if should_send(&msg, &subscriptions) {
                            if let Ok(bytes) = encode_server_message(&msg) {
                                if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
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
                    if ws_tx.send(Message::Ping(vec![].into())).await.is_err() {
                        crate::debug_error!("STREAMING", "Failed to ping TLS client {}", client_id);
                        break;
                    }
                }
            }
        }

        crate::debug_info!(
            "STREAMING",
            "TLS Client {} cleanup (remaining: {})",
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
                                                if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                    use std::io::Write;
                                                    if let Err(e) = w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
                                                        crate::debug_error!("STREAMING", "PTY write error for Axum session {}: {}", session.id, e);
                                                        session.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                                    }
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
                                            let refresh_msg = {
                                                if let Ok(terminal) = Ok::<_, ()>(terminal_for_refresh.lock()) {
                                                    let content = terminal.export_visible_screen_styled();
                                                    let (cols, rows) = terminal.size();
                                                    Some(ServerMessage::refresh(cols as u16, rows as u16, content))
                                                } else {
                                                    None
                                                }
                                            };
                                            if let Some(msg) = refresh_msg {
                                                if let Ok(bytes) = encode_server_message(&msg) {
                                                    let _ = ws_tx.send(AxumMessage::Binary(bytes.into())).await;
                                                }
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
        // Always send system messages
        ServerMessage::Connected { .. }
        | ServerMessage::Refresh { .. }
        | ServerMessage::Error { .. }
        | ServerMessage::Shutdown { .. }
        | ServerMessage::Pong => true,
    }
}

/// HTTP Basic Authentication middleware for Axum
#[cfg(feature = "streaming")]
async fn basic_auth_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
    auth_config: HttpBasicAuthConfig,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    if let Some(auth_value) = auth_header {
        if let Some(credentials) = auth_value.strip_prefix("Basic ") {
            if let Ok(decoded) = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                credentials.trim(),
            ) {
                if let Ok(credentials_str) = String::from_utf8(decoded) {
                    if let Some((username, password)) = credentials_str.split_once(':') {
                        if auth_config.verify(username, password) {
                            return next.run(req).await;
                        }
                    }
                }
            }
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"Terminal Server\"")],
        "Unauthorized",
    )
        .into_response()
}

/// Axum WebSocket handler (extracts query params for multi-session)
#[cfg(feature = "streaming")]
async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    let params = ConnectionParams::from_query(&query);
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = server.handle_axum_websocket(socket, params).await {
            crate::debug_error!("STREAMING", "WebSocket handler error: {}", e);
        }
    })
}

/// Sessions list HTTP handler
#[cfg(feature = "streaming")]
async fn sessions_handler(
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    let sessions = server.sessions.list_sessions();
    let max = server.config.max_sessions;
    let available = max.saturating_sub(sessions.len());
    axum::Json(serde_json::json!({
        "sessions": sessions,
        "max_sessions": max,
        "available": available,
    }))
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
    async fn test_streaming_server_creation() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());
        assert_eq!(server.addr, "127.0.0.1:0");
    }

    #[tokio::test]
    async fn test_streaming_config_default() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_clients, 1000);
        assert!(config.send_initial_screen);
        assert_eq!(config.keepalive_interval, 30);
        assert!(!config.default_read_only);
        assert_eq!(config.max_sessions, 10);
        assert_eq!(config.session_idle_timeout, 900);
        assert!(config.presets.is_empty());
        assert_eq!(config.max_clients_per_session, 0);
        assert_eq!(config.input_rate_limit_bytes_per_sec, 0);
    }

    #[tokio::test]
    async fn test_output_sender() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());

        let tx = server.get_output_sender();
        assert!(tx.try_send("test".to_string()).is_ok());
    }

    #[tokio::test]
    async fn test_session_state_creation() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = SessionState::new("test-session".to_string(), terminal, None, true);
        assert_eq!(session.id, "test-session");
        assert_eq!(session.client_count(), 0);
        assert!(session.created_at > 0);
    }

    #[tokio::test]
    async fn test_session_state_client_count() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = SessionState::new("sess".to_string(), terminal, None, true);

        assert_eq!(session.client_count(), 0);
        assert!(session.try_add_client(0)); // 0 = unlimited
        assert_eq!(session.client_count(), 1);
        assert!(session.try_add_client(0));
        assert_eq!(session.client_count(), 2);
        session.remove_client();
        assert_eq!(session.client_count(), 1);
        session.remove_client();
        assert_eq!(session.client_count(), 0);
    }

    #[tokio::test]
    async fn test_session_state_idle_detection() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = SessionState::new("sess".to_string(), terminal, None, true);

        // No clients, no disconnect time yet → not idle
        assert!(!session.is_idle(Duration::from_secs(1)));

        // Add and remove a client to set disconnect time
        session.try_add_client(0);
        session.remove_client();

        // Just disconnected, should not be idle with long timeout
        assert!(!session.is_idle(Duration::from_secs(3600)));

        // Should be idle with zero timeout
        assert!(session.is_idle(Duration::from_secs(0)));
    }

    #[tokio::test]
    async fn test_session_registry_basic() {
        let registry = SessionRegistry::new(10);
        assert_eq!(registry.session_count(), 0);

        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = Arc::new(SessionState::new("s1".to_string(), terminal, None, true));

        registry
            .insert("s1".to_string(), Arc::clone(&session))
            .unwrap();
        assert_eq!(registry.session_count(), 1);

        let retrieved = registry.get("s1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "s1");

        assert!(registry.get("s2").is_none());

        let removed = registry.remove("s1");
        assert!(removed.is_some());
        assert_eq!(registry.session_count(), 0);
    }

    #[tokio::test]
    async fn test_session_registry_max_sessions() {
        let registry = SessionRegistry::new(2);

        for i in 0..2 {
            let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
            let session = Arc::new(SessionState::new(format!("s{}", i), terminal, None, true));
            registry.insert(format!("s{}", i), session).unwrap();
        }

        // Third insert should fail
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = Arc::new(SessionState::new("s2".to_string(), terminal, None, true));
        let result = registry.insert("s2".to_string(), session);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StreamingError::MaxSessionsReached
        ));
    }

    #[tokio::test]
    async fn test_session_registry_list_sessions() {
        let registry = SessionRegistry::new(10);

        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let session = Arc::new(SessionState::new("s1".to_string(), terminal, None, true));
        registry.insert("s1".to_string(), session).unwrap();

        let sessions = registry.list_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].cols, 80);
        assert_eq!(sessions[0].rows, 24);
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
    async fn test_session_info_serialization() {
        let info = SessionInfo {
            id: "test".to_string(),
            created: 1234567890,
            clients: 2,
            idle_seconds: 0,
            cols: 80,
            rows: 24,
            cwd: Some("/home/user".to_string()),
            messages_sent: 0,
            bytes_sent: 0,
            input_bytes: 0,
            errors: 0,
            dropped_messages: 0,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"id\":\"test\""));
        assert!(json.contains("\"clients\":2"));
        assert!(json.contains("\"cols\":80"));
    }

    #[tokio::test]
    async fn test_default_session_exists() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));

        let params = ConnectionParams::from_uri_query(None);
        let session = server.resolve_session(&params);
        assert!(session.is_ok());
        assert_eq!(session.unwrap().id, "default");
    }

    #[tokio::test]
    async fn test_resolve_nonexistent_session_no_factory() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));

        let params = ConnectionParams::from_uri_query(Some("session=nonexistent"));
        let result = server.resolve_session(&params);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StreamingError::SessionNotFound(_)
        ));
    }
}
