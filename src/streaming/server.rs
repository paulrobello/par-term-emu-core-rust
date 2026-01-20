//! WebSocket streaming server implementation

use crate::streaming::client::Client;
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::proto::{decode_client_message, encode_server_message};
use crate::streaming::protocol::{ServerMessage, ThemeInfo};
use crate::terminal::Terminal;
use parking_lot::Mutex;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::accept_async;

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
        }
    }
}

/// Guard that decrements client count when dropped
struct ClientGuard<'a> {
    server: &'a StreamingServer,
}

impl<'a> Drop for ClientGuard<'a> {
    fn drop(&mut self) {
        self.server.remove_client();
    }
}

/// WebSocket streaming server for terminal sessions
pub struct StreamingServer {
    /// Atomic counter for tracking connected clients
    client_count: AtomicUsize,
    /// Shared terminal instance
    terminal: Arc<Mutex<Terminal>>,
    /// Server bind address
    addr: String,
    /// Server configuration
    config: StreamingConfig,
    /// Channel for sending output to broadcaster
    output_tx: mpsc::UnboundedSender<String>,
    /// Channel for receiving output from terminal
    output_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
    /// Broadcast channel for sending output to all clients
    broadcast_tx: broadcast::Sender<ServerMessage>,
    /// PTY writer for sending client input (optional, only set if PTY is available)
    /// Wrapped in RwLock for interior mutability (allows updating through Arc)
    #[allow(clippy::type_complexity)]
    pty_writer: std::sync::RwLock<Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>>>,
    /// Channel for sending resize requests to main thread
    resize_tx: mpsc::UnboundedSender<(u16, u16)>,
    /// Shared receiver for resize requests (wrapped for thread-safe access from Python)
    resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
    /// Optional theme information to send to clients
    theme: Option<ThemeInfo>,
    /// Shutdown signal for broadcaster loop
    shutdown: Arc<tokio::sync::Notify>,
}

impl StreamingServer {
    /// Create a new streaming server
    pub fn new(terminal: Arc<Mutex<Terminal>>, addr: String) -> Self {
        Self::with_config(terminal, addr, StreamingConfig::default())
    }

    /// Create a new streaming server with custom configuration
    pub fn with_config(
        terminal: Arc<Mutex<Terminal>>,
        addr: String,
        config: StreamingConfig,
    ) -> Self {
        let (output_tx, output_rx) = mpsc::unbounded_channel();
        // Create broadcast channel for sending output to all clients (buffer 100 messages)
        let (broadcast_tx, _) = broadcast::channel(100);
        // Create resize request channel
        let (resize_tx, resize_rx) = mpsc::unbounded_channel();

        Self {
            client_count: AtomicUsize::new(0),
            terminal,
            addr,
            config,
            output_tx,
            output_rx: Arc::new(tokio::sync::Mutex::new(output_rx)),
            broadcast_tx,
            pty_writer: std::sync::RwLock::new(None),
            resize_tx,
            resize_rx: Arc::new(tokio::sync::Mutex::new(resize_rx)),
            theme: None,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Set the theme to be sent to clients on connection
    pub fn set_theme(&mut self, theme: ThemeInfo) {
        self.theme = Some(theme);
    }

    /// Set the PTY writer for handling client input
    ///
    /// This should be called before starting the server if PTY input is supported.
    /// Can be called multiple times (e.g., after shell restart) as it uses interior mutability.
    pub fn set_pty_writer(&self, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>) {
        if let Ok(mut guard) = self.pty_writer.write() {
            *guard = Some(writer);
        }
    }

    /// Get a clone of the output sender channel
    ///
    /// This can be used to send terminal output to all connected clients
    pub fn get_output_sender(&self) -> mpsc::UnboundedSender<String> {
        self.output_tx.clone()
    }

    /// Get a clone of the resize receiver
    ///
    /// This can be used by the main thread to poll for resize requests from clients
    pub fn get_resize_receiver(
        &self,
    ) -> Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>> {
        Arc::clone(&self.resize_rx)
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
                Err(_) => continue, // Another thread modified, retry
            }
        }
    }

    /// Decrement the client count
    fn remove_client(&self) {
        self.client_count.fetch_sub(1, Ordering::SeqCst);
    }

    /// Broadcast a message to all clients via the broadcast channel
    pub fn broadcast(&self, msg: ServerMessage) {
        let _ = self.broadcast_tx.send(msg);
    }

    /// Start the streaming server
    ///
    /// This method will block until the server is stopped.
    /// If TLS is configured, the server will use HTTPS/WSS instead of HTTP/WS.
    pub async fn start(self: Arc<Self>) -> Result<()> {
        // Choose implementation based on config
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

    /// Start server with HTTP static file serving using Axum
    #[cfg(feature = "streaming")]
    async fn start_with_http(self: Arc<Self>) -> Result<()> {
        use axum::{routing::get, Router};
        use tower_http::services::ServeDir;

        crate::debug_info!("STREAMING", "Server with HTTP listening on {}", self.addr);

        // Spawn output broadcaster task
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.output_broadcaster_loop().await;
        });

        // Build router with optional basic auth middleware
        let app = Router::new()
            .route("/ws", get(ws_handler))
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

        // Spawn output broadcaster task
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.output_broadcaster_loop().await;
        });

        // Build router with optional basic auth middleware
        let app = Router::new()
            .route("/ws", get(ws_handler))
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

        // Spawn output broadcaster task
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.output_broadcaster_loop().await;
        });

        // Accept WebSocket connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    // Check max_clients before accepting
                    if !self.can_accept_client() {
                        crate::debug_error!(
                            "STREAMING",
                            "Max clients reached ({}), rejecting connection from {}",
                            self.config.max_clients,
                            addr
                        );
                        continue;
                    }

                    // Enable TCP_NODELAY for lower latency on small messages (keystrokes)
                    // This disables Nagle's algorithm which can add up to 40ms delay
                    if let Err(e) = stream.set_nodelay(true) {
                        crate::debug_error!("STREAMING", "Failed to set TCP_NODELAY: {}", e);
                    }

                    crate::debug_info!("STREAMING", "New connection from {}", addr);
                    let server = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            crate::debug_error!(
                                "STREAMING",
                                "Connection error from {}: {}",
                                addr,
                                e
                            );
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

        // Spawn output broadcaster task
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.output_broadcaster_loop().await;
        });

        // Accept TLS connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    // Check max_clients before accepting
                    if !self.can_accept_client() {
                        crate::debug_error!(
                            "STREAMING",
                            "Max clients reached ({}), rejecting TLS connection from {}",
                            self.config.max_clients,
                            addr
                        );
                        continue;
                    }

                    // Enable TCP_NODELAY for lower latency on small messages (keystrokes)
                    // This disables Nagle's algorithm which can add up to 40ms delay
                    if let Err(e) = stream.set_nodelay(true) {
                        crate::debug_error!("STREAMING", "Failed to set TCP_NODELAY: {}", e);
                    }

                    crate::debug_info!("STREAMING", "New TLS connection from {}", addr);
                    let server = self.clone();
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        // Perform TLS handshake
                        match acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                if let Err(e) = server.handle_tls_connection(tls_stream).await {
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

    /// Handle a new WebSocket connection
    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        // Try to reserve a client slot (atomic check-and-increment)
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }

        // Ensure we decrement the count when we exit this function
        let _guard = ClientGuard { server: self };

        // Upgrade to WebSocket
        let ws_stream = accept_async(stream)
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        let mut client = Client::new(ws_stream, self.config.default_read_only);
        let client_id = client.id();

        // Send initial connection message with visible screen snapshot
        let (cols, rows, initial_screen) = {
            let terminal = self.terminal.lock();
            let (cols, rows) = terminal.size();

            let initial_screen = if self.config.send_initial_screen {
                // Export only visible screen (no scrollback) with ANSI styling
                Some(terminal.export_visible_screen_styled())
            } else {
                None
            };

            (cols as u16, rows as u16, initial_screen)
        };

        let connect_msg = match (initial_screen, self.theme.clone()) {
            (Some(screen), Some(theme)) => ServerMessage::connected_with_screen_and_theme(
                cols,
                rows,
                screen,
                client_id.to_string(),
                theme,
            ),
            (Some(screen), None) => {
                ServerMessage::connected_with_screen(cols, rows, screen, client_id.to_string())
            }
            (None, Some(theme)) => {
                ServerMessage::connected_with_theme(cols, rows, client_id.to_string(), theme)
            }
            (None, None) => ServerMessage::connected(cols, rows, client_id.to_string()),
        };

        client.send(connect_msg).await?;

        crate::debug_info!(
            "STREAMING",
            "Client {} connected (total: {})",
            client_id,
            self.client_count()
        );

        let read_only = client.is_read_only();

        // Subscribe to output broadcasts
        let mut output_rx = self.broadcast_tx.subscribe();

        // Clone terminal for screen refresh
        let terminal_for_refresh = Arc::clone(&self.terminal);

        // Setup keepalive timer if enabled
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));

        // Handle client input and output in this task
        loop {
            tokio::select! {
                // Receive message from client (input from web terminal)
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
                                    // Check if client is allowed to send input
                                    if read_only {
                                        // Silently ignore input from read-only clients
                                        continue;
                                    }

                                    // Always fetch latest PTY writer (may be updated after shell restart)
                                    if let Some(writer) = self.pty_writer.read().ok().and_then(|g| g.clone()) {
                                        if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                            use std::io::Write;
                                            let _ = w.write_all(data.as_bytes());
                                            let _ = w.flush();
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                    // Send resize request to main thread
                                    // The main thread will call pty_terminal.resize() which:
                                    // 1. Resizes the terminal buffer
                                    // 2. Resizes the PTY (sends SIGWINCH to shell)
                                    let _ = self.resize_tx.send((cols, rows));
                                }
                                crate::streaming::protocol::ClientMessage::Ping => {
                                    // Send pong response
                                    if let Err(e) = client.send(ServerMessage::pong()).await {
                                        crate::debug_error!("STREAMING", "Failed to send pong to client {}: {}", client_id, e);
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                    // Send current visible screen content to client as refresh message
                                    let refresh_msg = {
                                        if let Ok(terminal) = Ok::<_, ()>(terminal_for_refresh.lock()) {
                                            let content = terminal.export_visible_screen_styled();
                                            let (cols, rows) = terminal.size();

                                            Some(ServerMessage::refresh(
                                                cols as u16,
                                                rows as u16,
                                                content
                                            ))
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
                                crate::streaming::protocol::ClientMessage::Subscribe { .. } => {
                                    // TODO: Implement subscription handling
                                }
                            }
                        }
                        None => {
                            // Client disconnected
                            crate::debug_info!("STREAMING", "Client {} disconnected", client_id);
                            break;
                        }
                        }
                    }
                }

                // Receive output to broadcast to client
                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if client.send(msg).await.is_err() {
                            break;
                        }
                    }
                }

                // Send keepalive ping if enabled
                _ = async {
                    if let Some(ref mut timer) = keepalive_timer {
                        timer.tick().await
                    } else {
                        // Never fires if keepalive is disabled
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

    /// Handle a new TLS WebSocket connection (WSS)
    async fn handle_tls_connection(
        &self,
        stream: tokio_rustls::server::TlsStream<TcpStream>,
    ) -> Result<()> {
        use tokio_tungstenite::accept_async as accept_async_tls;

        // Try to reserve a client slot (atomic check-and-increment)
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }

        // Ensure we decrement the count when we exit this function
        let _guard = ClientGuard { server: self };

        // Upgrade TLS stream to WebSocket
        let ws_stream = accept_async_tls(stream)
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        // Create a TLS client (different from regular Client due to stream type)
        let client_id = uuid::Uuid::new_v4();
        let read_only = self.config.default_read_only;

        // Send initial connection message with visible screen snapshot
        let (cols, rows, initial_screen) = {
            let terminal = self.terminal.lock();
            let (cols, rows) = terminal.size();

            let initial_screen = if self.config.send_initial_screen {
                Some(terminal.export_visible_screen_styled())
            } else {
                None
            };

            (cols as u16, rows as u16, initial_screen)
        };

        let connect_msg = match (initial_screen, self.theme.clone()) {
            (Some(screen), Some(theme)) => ServerMessage::connected_with_screen_and_theme(
                cols,
                rows,
                screen,
                client_id.to_string(),
                theme,
            ),
            (Some(screen), None) => {
                ServerMessage::connected_with_screen(cols, rows, screen, client_id.to_string())
            }
            (None, Some(theme)) => {
                ServerMessage::connected_with_theme(cols, rows, client_id.to_string(), theme)
            }
            (None, None) => ServerMessage::connected(cols, rows, client_id.to_string()),
        };

        // Use futures to split and handle the WebSocket
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        // Send connection message
        let msg_bytes = encode_server_message(&connect_msg)?;
        ws_tx
            .send(Message::Binary(msg_bytes.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        crate::debug_info!(
            "STREAMING",
            "TLS Client {} connected (total: {})",
            client_id,
            self.client_count()
        );

        // Subscribe to output broadcasts
        let mut output_rx = self.broadcast_tx.subscribe();

        // Clone terminal for screen refresh
        let terminal_for_refresh = Arc::clone(&self.terminal);
        let resize_tx = self.resize_tx.clone();

        // Setup keepalive timer if enabled
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));

        // Handle client input and output
        loop {
            tokio::select! {
                // Receive message from client (binary protobuf)
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

                                            // Pull the latest PTY writer each time so restarts are handled
                                            if let Some(writer) = self.pty_writer.read().ok().and_then(|g| g.clone()) {
                                                if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                    use std::io::Write;
                                                    let _ = w.write_all(data.as_bytes());
                                                    let _ = w.flush();
                                                }
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                            let _ = resize_tx.send((cols, rows));
                                        }
                                        crate::streaming::protocol::ClientMessage::Ping => {
                                            // Send pong response
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
                                        crate::streaming::protocol::ClientMessage::Subscribe { .. } => {
                                            // TODO: Implement subscription handling
                                        }
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
                        Some(Ok(Message::Pong(_))) => {
                            // Pong received
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            crate::debug_info!("STREAMING", "TLS Client {} disconnected", client_id);
                            break;
                        }
                        Some(Ok(Message::Frame(_))) => {
                            // Raw frames, ignore
                        }
                        Some(Err(e)) => {
                            crate::debug_error!("STREAMING", "TLS WebSocket error: {}", e);
                            break;
                        }
                    }
                }

                // Receive output to broadcast to client (binary protobuf)
                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if let Ok(bytes) = encode_server_message(&msg) {
                            if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }

                // Send keepalive ping if enabled
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

    /// Output broadcaster loop - forwards terminal output to all clients
    ///
    /// Implements time-based batching to reduce message frequency:
    /// - Collects output within a 16ms window (one frame at 60fps)
    /// - Flushes immediately if buffer exceeds 8KB
    /// - Reduces WebSocket message overhead by 50-80% during burst output
    async fn output_broadcaster_loop(&self) {
        let mut rx = self.output_rx.lock().await;
        let mut buffer = String::new();
        let mut last_flush = tokio::time::Instant::now();

        // Batching configuration
        const BATCH_WINDOW: Duration = Duration::from_millis(16); // One frame at 60fps
        const MAX_BATCH_SIZE: usize = 8192; // 8KB max before forced flush

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = self.shutdown.notified() => {
                    crate::debug_info!("STREAMING", "Broadcaster received shutdown signal");
                    // Flush any remaining data before exiting
                    if !buffer.is_empty() {
                        let msg = ServerMessage::output(buffer);
                        let _ = self.broadcast_tx.send(msg);
                    }
                    break;
                }
                // Receive new output data
                msg = rx.recv() => {
                    match msg {
                        Some(data) => {
                            if !data.is_empty() {
                                buffer.push_str(&data);

                                // Flush immediately if buffer is large
                                if buffer.len() > MAX_BATCH_SIZE {
                                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                                    let _ = self.broadcast_tx.send(msg);
                                    last_flush = tokio::time::Instant::now();
                                }
                            }
                        }
                        None => {
                            // Channel closed, flush remaining and exit
                            if !buffer.is_empty() {
                                let msg = ServerMessage::output(buffer);
                                let _ = self.broadcast_tx.send(msg);
                            }
                            break;
                        }
                    }
                }
                // Timeout - flush batched output
                _ = tokio::time::sleep_until(last_flush + BATCH_WINDOW), if !buffer.is_empty() => {
                    let msg = ServerMessage::output(std::mem::take(&mut buffer));
                    let _ = self.broadcast_tx.send(msg);
                    last_flush = tokio::time::Instant::now();
                }
            }
        }
    }

    /// Send terminal output to all connected clients
    pub fn send_output(&self, data: String) -> Result<()> {
        self.output_tx
            .send(data)
            .map_err(|_| StreamingError::ServerError("Output channel closed".to_string()))
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

    /// Shutdown the server and disconnect all clients
    ///
    /// This broadcasts a shutdown message to all clients and signals
    /// the broadcaster loop to exit gracefully.
    pub fn shutdown(&self, reason: String) {
        crate::debug_info!("STREAMING", "Shutting down server: {}", reason);
        let msg = ServerMessage::shutdown(reason);
        self.broadcast(msg);
        // Signal the broadcaster loop to exit
        self.shutdown.notify_waiters();
    }

    /// Handle Axum WebSocket connection
    #[cfg(feature = "streaming")]
    async fn handle_axum_websocket(&self, socket: axum::extract::ws::WebSocket) -> Result<()> {
        use axum::extract::ws::Message as AxumMessage;
        use futures_util::{SinkExt, StreamExt};

        // Try to reserve a client slot (atomic check-and-increment)
        if !self.try_add_client() {
            return Err(StreamingError::MaxClientsReached);
        }

        // Ensure we decrement the count when we exit this function
        let _guard = ClientGuard { server: self };

        let client_id = uuid::Uuid::new_v4();

        // Split the WebSocket into sender and receiver
        let (mut ws_tx, mut ws_rx) = socket.split();

        // Send initial connection message with visible screen snapshot
        let (cols, rows, initial_screen) = {
            let terminal = self.terminal.lock();
            let (cols, rows) = terminal.size();

            let initial_screen = if self.config.send_initial_screen {
                Some(terminal.export_visible_screen_styled())
            } else {
                None
            };

            (cols as u16, rows as u16, initial_screen)
        };

        let connect_msg = match (initial_screen, self.theme.clone()) {
            (Some(screen), Some(theme)) => ServerMessage::connected_with_screen_and_theme(
                cols,
                rows,
                screen,
                client_id.to_string(),
                theme,
            ),
            (Some(screen), None) => {
                ServerMessage::connected_with_screen(cols, rows, screen, client_id.to_string())
            }
            (None, Some(theme)) => {
                ServerMessage::connected_with_theme(cols, rows, client_id.to_string(), theme)
            }
            (None, None) => ServerMessage::connected(cols, rows, client_id.to_string()),
        };

        // Send connection message as binary protobuf
        let msg_bytes = encode_server_message(&connect_msg)?;
        ws_tx
            .send(AxumMessage::Binary(msg_bytes.into()))
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        crate::debug_info!(
            "STREAMING",
            "Axum WebSocket client {} connected (total: {})",
            client_id,
            self.client_count()
        );

        let read_only = self.config.default_read_only;

        // Subscribe to output broadcasts
        let mut output_rx = self.broadcast_tx.subscribe();

        // Clone terminal for screen refresh
        let terminal_for_refresh = Arc::clone(&self.terminal);
        let resize_tx = self.resize_tx.clone();

        // Setup keepalive timer if enabled
        let keepalive_interval = if self.config.keepalive_interval > 0 {
            Some(Duration::from_secs(self.config.keepalive_interval))
        } else {
            None
        };
        let mut keepalive_timer = keepalive_interval.map(|d| tokio::time::interval(d));

        // Handle client input and output
        loop {
            tokio::select! {
                // Receive message from client (binary protobuf)
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(AxumMessage::Binary(data))) => {
                            // Parse client message from binary protobuf
                            match decode_client_message(&data) {
                                Ok(client_msg) => {
                                    match client_msg {
                                        crate::streaming::protocol::ClientMessage::Input { data } => {
                                            if read_only {
                                                continue;
                                            }

                                            // Always grab latest PTY writer (shell may have restarted)
                                            if let Some(writer) = self.pty_writer.read().ok().and_then(|g| g.clone()) {
                                                if let Ok(mut w) = Ok::<_, ()>(writer.lock()) {
                                                    use std::io::Write;
                                                    let _ = w.write_all(data.as_bytes());
                                                    let _ = w.flush();
                                                }
                                            }
                                        }
                                        crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                            let _ = resize_tx.send((cols, rows));
                                        }
                                        crate::streaming::protocol::ClientMessage::Ping => {
                                            // Send pong response
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
                                        crate::streaming::protocol::ClientMessage::Subscribe { .. } => {
                                            // TODO: Implement subscription handling
                                        }
                                    }
                                }
                                Err(e) => {
                                    crate::debug_error!("STREAMING", "Failed to parse client message: {}", e);
                                }
                            }
                        }
                        Some(Ok(AxumMessage::Text(_))) => {
                            // Text messages not supported in binary protocol
                            crate::debug_error!("STREAMING", "Text messages not supported, use binary protocol");
                        }
                        Some(Ok(AxumMessage::Ping(_))) => {
                            // Axum handles pings automatically
                        }
                        Some(Ok(AxumMessage::Pong(_))) => {
                            // Pong received
                        }
                        Some(Ok(AxumMessage::Close(_))) | None => {
                            crate::debug_info!("STREAMING", "Axum Client {} disconnected", client_id);
                            break;
                        }
                        Some(Err(e)) => {
                            crate::debug_error!("STREAMING", "WebSocket error: {}", e);
                            break;
                        }
                    }
                }

                // Receive output to broadcast to client (binary protobuf)
                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        if let Ok(bytes) = encode_server_message(&msg) {
                            if ws_tx.send(AxumMessage::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }

                // Send keepalive ping if enabled
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

/// HTTP Basic Authentication middleware for Axum
///
/// Validates Authorization header with Basic credentials against the provided config.
/// Returns 401 Unauthorized with WWW-Authenticate header if authentication fails.
#[cfg(feature = "streaming")]
async fn basic_auth_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
    auth_config: HttpBasicAuthConfig,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    // Extract Authorization header
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    if let Some(auth_value) = auth_header {
        // Check if it's Basic auth
        if let Some(credentials) = auth_value.strip_prefix("Basic ") {
            // Decode base64 credentials
            if let Ok(decoded) = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                credentials.trim(),
            ) {
                if let Ok(credentials_str) = String::from_utf8(decoded) {
                    // Split username:password
                    if let Some((username, password)) = credentials_str.split_once(':') {
                        // Verify credentials
                        if auth_config.verify(username, password) {
                            return next.run(req).await;
                        }
                    }
                }
            }
        }
    }

    // Return 401 Unauthorized with WWW-Authenticate header
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"Terminal Server\"")],
        "Unauthorized",
    )
        .into_response()
}

/// Axum WebSocket handler
#[cfg(feature = "streaming")]
async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State(server): axum::extract::State<Arc<StreamingServer>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = server.handle_axum_websocket(socket).await {
            crate::debug_error!("STREAMING", "WebSocket handler error: {}", e);
        }
    })
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
    }

    #[tokio::test]
    async fn test_output_sender() {
        let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
        let server = StreamingServer::new(terminal, "127.0.0.1:0".to_string());

        let tx = server.get_output_sender();
        assert!(tx.send("test".to_string()).is_ok());
    }
}
