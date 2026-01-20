//! Python bindings for terminal streaming

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[cfg(feature = "streaming")]
use crate::streaming::{StreamingConfig, StreamingServer, TlsConfig};
#[cfg(feature = "streaming")]
use std::sync::Arc;

#[cfg(feature = "streaming")]
type ResizeReceiver =
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>>>;

/// Python wrapper for StreamingConfig
#[cfg(feature = "streaming")]
#[pyclass(name = "StreamingConfig")]
pub struct PyStreamingConfig {
    inner: StreamingConfig,
}

#[cfg(feature = "streaming")]
impl Clone for PyStreamingConfig {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(feature = "streaming")]
#[pymethods]
impl PyStreamingConfig {
    #[new]
    #[pyo3(signature = (max_clients=1000, send_initial_screen=true, keepalive_interval=30, default_read_only=false, initial_cols=0, initial_rows=0, enable_http=false, web_root="./web_term"))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        max_clients: usize,
        send_initial_screen: bool,
        keepalive_interval: u64,
        default_read_only: bool,
        initial_cols: u16,
        initial_rows: u16,
        enable_http: bool,
        web_root: &str,
    ) -> Self {
        Self {
            inner: StreamingConfig {
                max_clients,
                send_initial_screen,
                keepalive_interval,
                default_read_only,
                enable_http,
                web_root: web_root.to_string(),
                initial_cols,
                initial_rows,
                tls: None, // TLS configuration via set_tls_from_files/set_tls_from_pem
                http_basic_auth: None, // HTTP Basic Auth not exposed to Python (use CLI flags instead)
            },
        }
    }

    /// Get the maximum number of clients
    #[getter]
    fn max_clients(&self) -> usize {
        self.inner.max_clients
    }

    /// Set the maximum number of clients
    #[setter]
    fn set_max_clients(&mut self, max_clients: usize) {
        self.inner.max_clients = max_clients;
    }

    /// Get whether to send initial screen
    #[getter]
    fn send_initial_screen(&self) -> bool {
        self.inner.send_initial_screen
    }

    /// Set whether to send initial screen
    #[setter]
    fn set_send_initial_screen(&mut self, send_initial_screen: bool) {
        self.inner.send_initial_screen = send_initial_screen;
    }

    /// Get keepalive interval in seconds
    #[getter]
    fn keepalive_interval(&self) -> u64 {
        self.inner.keepalive_interval
    }

    /// Set keepalive interval in seconds
    #[setter]
    fn set_keepalive_interval(&mut self, keepalive_interval: u64) {
        self.inner.keepalive_interval = keepalive_interval;
    }

    /// Get default read-only mode
    #[getter]
    fn default_read_only(&self) -> bool {
        self.inner.default_read_only
    }

    /// Set default read-only mode
    #[setter]
    fn set_default_read_only(&mut self, default_read_only: bool) {
        self.inner.default_read_only = default_read_only;
    }

    /// Get initial terminal columns (0 = use terminal's current size)
    #[getter]
    fn initial_cols(&self) -> u16 {
        self.inner.initial_cols
    }

    /// Set initial terminal columns (0 = use terminal's current size)
    #[setter]
    fn set_initial_cols(&mut self, initial_cols: u16) {
        self.inner.initial_cols = initial_cols;
    }

    /// Get initial terminal rows (0 = use terminal's current size)
    #[getter]
    fn initial_rows(&self) -> u16 {
        self.inner.initial_rows
    }

    /// Set initial terminal rows (0 = use terminal's current size)
    #[setter]
    fn set_initial_rows(&mut self, initial_rows: u16) {
        self.inner.initial_rows = initial_rows;
    }

    /// Get whether HTTP static file serving is enabled
    #[getter]
    fn enable_http(&self) -> bool {
        self.inner.enable_http
    }

    /// Set whether HTTP static file serving is enabled
    #[setter]
    fn set_enable_http(&mut self, enable_http: bool) {
        self.inner.enable_http = enable_http;
    }

    /// Get the web root directory for static files
    #[getter]
    fn web_root(&self) -> String {
        self.inner.web_root.clone()
    }

    /// Set the web root directory for static files
    #[setter]
    fn set_web_root(&mut self, web_root: String) {
        self.inner.web_root = web_root;
    }

    fn __repr__(&self) -> String {
        let tls_status = if self.inner.tls.is_some() {
            ", tls=enabled"
        } else {
            ""
        };
        format!(
            "StreamingConfig(max_clients={}, send_initial_screen={}, keepalive_interval={}, default_read_only={}, initial_cols={}, initial_rows={}, enable_http={}, web_root='{}'{})",
            self.inner.max_clients,
            self.inner.send_initial_screen,
            self.inner.keepalive_interval,
            self.inner.default_read_only,
            self.inner.initial_cols,
            self.inner.initial_rows,
            self.inner.enable_http,
            self.inner.web_root,
            tls_status
        )
    }

    /// Configure TLS from separate certificate and key files
    ///
    /// Args:
    ///     cert_path: Path to PEM certificate file (may contain certificate chain)
    ///     key_path: Path to PEM private key file
    ///
    /// Raises:
    ///     RuntimeError: If files cannot be read or parsed
    fn set_tls_from_files(&mut self, cert_path: &str, key_path: &str) -> PyResult<()> {
        let tls_config = TlsConfig::from_files(cert_path, key_path)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to load TLS config: {}", e)))?;
        self.inner.tls = Some(tls_config);
        Ok(())
    }

    /// Configure TLS from a combined PEM file
    ///
    /// Args:
    ///     pem_path: Path to PEM file containing both certificate chain and private key
    ///
    /// Raises:
    ///     RuntimeError: If file cannot be read or parsed
    fn set_tls_from_pem(&mut self, pem_path: &str) -> PyResult<()> {
        let tls_config = TlsConfig::from_pem(pem_path)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to load TLS config: {}", e)))?;
        self.inner.tls = Some(tls_config);
        Ok(())
    }

    /// Check if TLS is configured
    ///
    /// Returns:
    ///     bool: True if TLS is configured, False otherwise
    #[getter]
    fn tls_enabled(&self) -> bool {
        self.inner.tls.is_some()
    }

    /// Disable TLS (clear TLS configuration)
    fn disable_tls(&mut self) {
        self.inner.tls = None;
    }
}

/// Python wrapper for StreamingServer
#[cfg(feature = "streaming")]
#[pyclass(name = "StreamingServer")]
pub struct PyStreamingServer {
    server: Option<Arc<StreamingServer>>,
    runtime: Arc<tokio::runtime::Runtime>,
    addr: String,
    resize_rx: Option<ResizeReceiver>,
}

#[cfg(feature = "streaming")]
#[pymethods]
impl PyStreamingServer {
    /// Create a new streaming server
    ///
    /// Args:
    ///     pty_terminal: The PyPtyTerminal instance to stream (mutable to set callback)
    ///     addr: The address to bind to (e.g., "127.0.0.1:8080")
    ///     config: Optional StreamingConfig for server configuration
    #[new]
    #[pyo3(signature = (pty_terminal, addr, config=None))]
    fn new(
        pty_terminal: &mut crate::python_bindings::pty::PyPtyTerminal,
        addr: String,
        config: Option<PyStreamingConfig>,
    ) -> PyResult<Self> {
        let runtime = tokio::runtime::Runtime::new().map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to create tokio runtime: {}", e))
        })?;

        // Get the terminal Arc from PyPtyTerminal
        let terminal_arc = pty_terminal.get_terminal_arc();

        // Get the PTY writer for input handling
        let pty_writer = pty_terminal.get_pty_writer();

        let server = if let Some(cfg) = config {
            StreamingServer::with_config(terminal_arc, addr.clone(), cfg.inner)
        } else {
            StreamingServer::new(terminal_arc, addr.clone())
        };

        // Set the PTY writer if available
        if let Some(writer) = pty_writer {
            server.set_pty_writer(writer);
        }

        // Get channels before wrapping server in Arc
        let output_sender = server.get_output_sender();
        let resize_rx = server.get_resize_receiver();

        let server = Arc::new(server);

        // Create UTF-8 decoder state for handling partial sequences
        // Multi-byte UTF-8 characters may be split across PTY reads
        let utf8_buffer = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));

        // Create a callback that forwards PTY output to the streaming server
        let callback = {
            let utf8_buffer = Arc::clone(&utf8_buffer);
            Arc::new(move |data: &[u8]| {
                // Append new data to buffer
                let mut buffer = utf8_buffer.lock();
                buffer.extend_from_slice(data);

                // Try to convert as much as possible to valid UTF-8
                match std::str::from_utf8(&buffer) {
                    Ok(valid_str) => {
                        // All bytes are valid UTF-8
                        let output = valid_str.to_string();
                        buffer.clear();
                        let _ = output_sender.send(output);
                    }
                    Err(error) => {
                        // Find how much is valid
                        let valid_up_to = error.valid_up_to();

                        if valid_up_to > 0 {
                            // Send the valid portion
                            let valid_str = std::str::from_utf8(&buffer[..valid_up_to]).unwrap();
                            let output = valid_str.to_string();
                            let _ = output_sender.send(output);

                            // Keep only the incomplete sequence for next time
                            buffer.drain(..valid_up_to);
                        }

                        // If buffer gets too large (>100 bytes of invalid data),
                        // it's probably not a partial sequence, flush it
                        if buffer.len() > 100 {
                            let output = String::from_utf8_lossy(&buffer).to_string();
                            buffer.clear();
                            let _ = output_sender.send(output);
                        }
                    }
                }
            })
        };

        // Set the callback on the PTY terminal
        pty_terminal.set_output_callback(callback);

        Ok(Self {
            server: Some(server),
            runtime: Arc::new(runtime),
            addr,
            resize_rx: Some(resize_rx),
        })
    }

    /// Start the streaming server (non-blocking)
    ///
    /// This spawns the server in a background thread
    fn start(&mut self) -> PyResult<()> {
        if let Some(server) = &self.server {
            let server = server.clone();
            let runtime = self.runtime.clone();

            // Spawn server in background thread
            std::thread::spawn(move || {
                runtime.block_on(async {
                    if let Err(e) = server.start().await {
                        crate::debug_error!("STREAMING", "Streaming server error: {}", e);
                    }
                });
            });

            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Get the number of connected clients
    fn client_count(&self) -> PyResult<usize> {
        if let Some(server) = &self.server {
            Ok(server.client_count())
        } else {
            Ok(0)
        }
    }

    /// Get the maximum number of clients allowed
    fn max_clients(&self) -> PyResult<usize> {
        if let Some(server) = &self.server {
            Ok(server.max_clients())
        } else {
            Ok(0)
        }
    }

    /// Set the theme to be sent to clients on connection
    ///
    /// Note: This method is not available after the server is wrapped in Arc.
    /// Set the theme before starting the server by creating a new server instance
    /// or use the CLI --theme flag instead.
    ///
    /// Args:
    ///     name: Theme name (e.g., "iterm2-dark")
    ///     background: RGB tuple for background color (r, g, b)
    ///     foreground: RGB tuple for foreground color (r, g, b)
    ///     normal: List of 8 RGB tuples for normal ANSI colors 0-7
    ///     bright: List of 8 RGB tuples for bright ANSI colors 8-15
    #[staticmethod]
    fn create_theme_info(
        name: String,
        background: (u8, u8, u8),
        foreground: (u8, u8, u8),
        normal: Vec<(u8, u8, u8)>,
        bright: Vec<(u8, u8, u8)>,
    ) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
        use pyo3::types::PyDict;

        if normal.len() != 8 {
            return Err(PyRuntimeError::new_err(
                "normal must contain exactly 8 RGB tuples",
            ));
        }
        if bright.len() != 8 {
            return Err(PyRuntimeError::new_err(
                "bright must contain exactly 8 RGB tuples",
            ));
        }

        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("name", name)?;
            dict.set_item("background", background)?;
            dict.set_item("foreground", foreground)?;
            dict.set_item("normal", normal)?;
            dict.set_item("bright", bright)?;
            Ok(dict.into())
        })
    }

    /// Send output data to all connected clients
    ///
    /// Args:
    ///     data: The output data to send (ANSI escape sequences)
    fn send_output(&self, data: String) -> PyResult<()> {
        if let Some(server) = &self.server {
            server
                .send_output(data)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to send output: {}", e)))
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a resize event to all clients
    ///
    /// Args:
    ///     cols: Number of columns
    ///     rows: Number of rows
    fn send_resize(&self, cols: u16, rows: u16) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_resize(cols, rows);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Poll for resize requests from clients (non-blocking)
    ///
    /// Returns:
    ///     Optional tuple of (cols, rows) if a resize request is pending, None otherwise
    ///
    /// This should be called periodically from the main event loop.
    /// When a resize is received, call pty_terminal.resize(cols, rows) to apply it.
    fn poll_resize(&self) -> PyResult<Option<(u16, u16)>> {
        if let Some(ref resize_rx) = self.resize_rx {
            let resize_rx = resize_rx.clone();
            let runtime = self.runtime.clone();

            Ok(runtime.block_on(async {
                // Try to receive without blocking
                resize_rx.lock().await.try_recv().ok()
            }))
        } else {
            Ok(None)
        }
    }

    /// Send a title change event to all clients
    ///
    /// Args:
    ///     title: The new terminal title
    fn send_title(&self, title: String) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_title(title);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a bell event to all clients
    fn send_bell(&self) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_bell();
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Shutdown the server and disconnect all clients
    ///
    /// Args:
    ///     reason: Reason for shutdown
    fn shutdown(&mut self, reason: String) -> PyResult<()> {
        if let Some(server) = self.server.take() {
            server.shutdown(reason);
            Ok(())
        } else {
            Ok(()) // Already stopped
        }
    }

    /// Get the server address
    #[getter]
    fn addr(&self) -> String {
        self.addr.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamingServer(addr='{}', clients={})",
            self.addr,
            if self.server.is_some() {
                "active"
            } else {
                "stopped"
            }
        )
    }
}

// For non-streaming builds, provide stub classes
#[cfg(not(feature = "streaming"))]
#[pyclass(name = "StreamingServer")]
pub struct PyStreamingServer;

#[cfg(not(feature = "streaming"))]
#[pymethods]
impl PyStreamingServer {
    #[new]
    fn new(
        _pty_terminal: &mut crate::python_bindings::pty::PyPtyTerminal,
        _addr: String,
    ) -> PyResult<Self> {
        Err(PyRuntimeError::new_err(
            "Streaming feature not enabled. Rebuild with --features streaming",
        ))
    }
}

#[cfg(not(feature = "streaming"))]
#[pyclass(name = "StreamingConfig")]
pub struct PyStreamingConfig;

#[cfg(not(feature = "streaming"))]
#[pymethods]
impl PyStreamingConfig {
    #[new]
    fn new() -> PyResult<Self> {
        Err(PyRuntimeError::new_err(
            "Streaming feature not enabled. Rebuild with --features streaming",
        ))
    }
}

// =============================================================================
// Binary Protocol Functions
// =============================================================================

/// Encode a server message to binary protobuf format
///
/// Args:
///     message_type: Type of message ("output", "resize", "title", "bell", "connected", "error", "shutdown", "cursor", "refresh", "pong")
///     **kwargs: Message-specific arguments:
///         - output: data (str), timestamp (optional int)
///         - resize: cols (int), rows (int)
///         - title: title (str)
///         - bell: no arguments
///         - pong: no arguments
///         - connected: cols (int), rows (int), session_id (str), initial_screen (optional str), theme (optional dict with name, background, foreground, normal, bright)
///         - error: message (str), code (optional str)
///         - shutdown: reason (str)
///         - cursor: col (int), row (int), visible (bool)
///         - refresh: cols (int), rows (int), screen_content (str)
///
/// Returns:
///     bytes: Binary protobuf encoded message
///
/// Raises:
///     RuntimeError: If encoding fails or streaming feature not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
#[pyo3(signature = (message_type, **kwargs))]
pub fn encode_server_message<'py>(
    py: Python<'py>,
    message_type: &str,
    kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::streaming::protocol::ServerMessage;

    // Helper closure to get a value from kwargs
    let get_str = |key: &str| -> Option<String> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };
    let get_u16 = |key: &str| -> Option<u16> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };
    let get_bool = |key: &str| -> Option<bool> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };

    let msg = match message_type {
        "output" => {
            let data = get_str("data").unwrap_or_default();
            ServerMessage::output(data)
        }
        "resize" => {
            let cols = get_u16("cols").unwrap_or(80);
            let rows = get_u16("rows").unwrap_or(24);
            ServerMessage::resize(cols, rows)
        }
        "title" => {
            let title = get_str("title").unwrap_or_default();
            ServerMessage::title(title)
        }
        "bell" => ServerMessage::bell(),
        "pong" => ServerMessage::pong(),
        "connected" => {
            use crate::streaming::protocol::ThemeInfo;

            let cols = get_u16("cols").unwrap_or(80);
            let rows = get_u16("rows").unwrap_or(24);
            let session_id =
                get_str("session_id").unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let initial_screen = get_str("initial_screen");

            // Try to extract theme from kwargs
            let theme: Option<ThemeInfo> = kwargs
                .and_then(|k| k.get_item("theme").ok().flatten())
                .and_then(|v| {
                    // Extract theme dict fields
                    let name: String = v.get_item("name").ok()?.extract().ok()?;
                    let background: (u8, u8, u8) = v.get_item("background").ok()?.extract().ok()?;
                    let foreground: (u8, u8, u8) = v.get_item("foreground").ok()?.extract().ok()?;
                    let normal_vec: Vec<(u8, u8, u8)> =
                        v.get_item("normal").ok()?.extract().ok()?;
                    let bright_vec: Vec<(u8, u8, u8)> =
                        v.get_item("bright").ok()?.extract().ok()?;

                    if normal_vec.len() != 8 || bright_vec.len() != 8 {
                        return None;
                    }

                    let mut normal = [(0u8, 0u8, 0u8); 8];
                    let mut bright = [(0u8, 0u8, 0u8); 8];
                    for (i, c) in normal_vec.into_iter().enumerate() {
                        normal[i] = c;
                    }
                    for (i, c) in bright_vec.into_iter().enumerate() {
                        bright[i] = c;
                    }

                    Some(ThemeInfo {
                        name,
                        background,
                        foreground,
                        normal,
                        bright,
                    })
                });

            match (initial_screen, theme) {
                (Some(screen), Some(theme)) => ServerMessage::connected_with_screen_and_theme(
                    cols, rows, screen, session_id, theme,
                ),
                (Some(screen), None) => {
                    ServerMessage::connected_with_screen(cols, rows, screen, session_id)
                }
                (None, Some(theme)) => {
                    ServerMessage::connected_with_theme(cols, rows, session_id, theme)
                }
                (None, None) => ServerMessage::connected(cols, rows, session_id),
            }
        }
        "error" => {
            let message = get_str("message").unwrap_or_else(|| "Unknown error".to_string());
            let code = get_str("code");
            match code {
                Some(c) => ServerMessage::error_with_code(message, c),
                None => ServerMessage::error(message),
            }
        }
        "shutdown" => {
            let reason = get_str("reason").unwrap_or_else(|| "Server shutdown".to_string());
            ServerMessage::shutdown(reason)
        }
        "cursor" => {
            let col = get_u16("col").unwrap_or(0);
            let row = get_u16("row").unwrap_or(0);
            let visible = get_bool("visible").unwrap_or(true);
            ServerMessage::cursor(col, row, visible)
        }
        "refresh" => {
            let cols = get_u16("cols").unwrap_or(80);
            let rows = get_u16("rows").unwrap_or(24);
            let screen_content = get_str("screen_content").unwrap_or_default();
            ServerMessage::refresh(cols, rows, screen_content)
        }
        _ => {
            return Err(PyRuntimeError::new_err(format!(
                "Unknown message type: {}. Valid types: output, resize, title, bell, pong, connected, error, shutdown, cursor, refresh",
                message_type
            )));
        }
    };

    let encoded = crate::streaming::encode_server_message(&msg)
        .map_err(|e| PyRuntimeError::new_err(format!("Encoding error: {}", e)))?;

    Ok(PyBytes::new(py, &encoded))
}

/// Decode a binary protobuf server message
///
/// Args:
///     data: Binary protobuf encoded message
///
/// Returns:
///     dict: Decoded message with 'type' key and message-specific fields
///
/// Raises:
///     RuntimeError: If decoding fails or streaming feature not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
pub fn decode_server_message<'py>(
    py: Python<'py>,
    data: &[u8],
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    use crate::streaming::protocol::ServerMessage;
    use pyo3::types::PyDict;

    let msg = crate::streaming::decode_server_message(data)
        .map_err(|e| PyRuntimeError::new_err(format!("Decoding error: {}", e)))?;

    let dict = PyDict::new(py);

    match msg {
        ServerMessage::Output { data, timestamp } => {
            dict.set_item("type", "output")?;
            dict.set_item("data", data)?;
            dict.set_item("timestamp", timestamp)?;
        }
        ServerMessage::Resize { cols, rows } => {
            dict.set_item("type", "resize")?;
            dict.set_item("cols", cols)?;
            dict.set_item("rows", rows)?;
        }
        ServerMessage::Title { title } => {
            dict.set_item("type", "title")?;
            dict.set_item("title", title)?;
        }
        ServerMessage::Bell => {
            dict.set_item("type", "bell")?;
        }
        ServerMessage::Connected {
            cols,
            rows,
            initial_screen,
            session_id,
            theme,
        } => {
            dict.set_item("type", "connected")?;
            dict.set_item("cols", cols)?;
            dict.set_item("rows", rows)?;
            dict.set_item("initial_screen", initial_screen)?;
            dict.set_item("session_id", session_id)?;
            if let Some(t) = theme {
                let theme_dict = PyDict::new(py);
                theme_dict.set_item("name", t.name)?;
                theme_dict.set_item(
                    "background",
                    (t.background.0, t.background.1, t.background.2),
                )?;
                theme_dict.set_item(
                    "foreground",
                    (t.foreground.0, t.foreground.1, t.foreground.2),
                )?;
                dict.set_item("theme", theme_dict)?;
            } else {
                dict.set_item("theme", py.None())?;
            }
        }
        ServerMessage::Refresh {
            cols,
            rows,
            screen_content,
        } => {
            dict.set_item("type", "refresh")?;
            dict.set_item("cols", cols)?;
            dict.set_item("rows", rows)?;
            dict.set_item("screen_content", screen_content)?;
        }
        ServerMessage::CursorPosition { col, row, visible } => {
            dict.set_item("type", "cursor")?;
            dict.set_item("col", col)?;
            dict.set_item("row", row)?;
            dict.set_item("visible", visible)?;
        }
        ServerMessage::Error { message, code } => {
            dict.set_item("type", "error")?;
            dict.set_item("message", message)?;
            dict.set_item("code", code)?;
        }
        ServerMessage::Shutdown { reason } => {
            dict.set_item("type", "shutdown")?;
            dict.set_item("reason", reason)?;
        }
        ServerMessage::Pong => {
            dict.set_item("type", "pong")?;
        }
    }

    Ok(dict)
}

/// Encode a client message to binary protobuf format
///
/// Args:
///     message_type: Type of message ("input", "resize", "ping", "refresh", "subscribe")
///     **kwargs: Message-specific arguments:
///         - input: data (str)
///         - resize: cols (int), rows (int)
///         - ping: no arguments
///         - refresh: no arguments
///         - subscribe: events (list of str: "output", "cursor", "bell", "title", "resize")
///
/// Returns:
///     bytes: Binary protobuf encoded message
///
/// Raises:
///     RuntimeError: If encoding fails or streaming feature not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
#[pyo3(signature = (message_type, **kwargs))]
pub fn encode_client_message<'py>(
    py: Python<'py>,
    message_type: &str,
    kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::streaming::protocol::{ClientMessage, EventType};

    // Helper closure to get a value from kwargs
    let get_str = |key: &str| -> Option<String> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };
    let get_u16 = |key: &str| -> Option<u16> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };
    let get_vec_str = |key: &str| -> Option<Vec<String>> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };

    let msg = match message_type {
        "input" => {
            let data = get_str("data").unwrap_or_default();
            ClientMessage::input(data)
        }
        "resize" => {
            let cols = get_u16("cols").unwrap_or(80);
            let rows = get_u16("rows").unwrap_or(24);
            ClientMessage::resize(cols, rows)
        }
        "ping" => ClientMessage::Ping,
        "refresh" => ClientMessage::RequestRefresh,
        "subscribe" => {
            let events_strs = get_vec_str("events").unwrap_or_default();
            let events: Vec<EventType> = events_strs
                .iter()
                .filter_map(|s| match s.as_str() {
                    "output" => Some(EventType::Output),
                    "cursor" => Some(EventType::Cursor),
                    "bell" => Some(EventType::Bell),
                    "title" => Some(EventType::Title),
                    "resize" => Some(EventType::Resize),
                    _ => None,
                })
                .collect();
            ClientMessage::subscribe(events)
        }
        _ => {
            return Err(PyRuntimeError::new_err(format!(
                "Unknown message type: {}. Valid types: input, resize, ping, refresh, subscribe",
                message_type
            )));
        }
    };

    let encoded = crate::streaming::encode_client_message(&msg)
        .map_err(|e| PyRuntimeError::new_err(format!("Encoding error: {}", e)))?;

    Ok(PyBytes::new(py, &encoded))
}

/// Decode a binary protobuf client message
///
/// Args:
///     data: Binary protobuf encoded message
///
/// Returns:
///     dict: Decoded message with 'type' key and message-specific fields
///
/// Raises:
///     RuntimeError: If decoding fails or streaming feature not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
pub fn decode_client_message<'py>(
    py: Python<'py>,
    data: &[u8],
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    use crate::streaming::protocol::ClientMessage;
    use pyo3::types::PyDict;

    let msg = crate::streaming::decode_client_message(data)
        .map_err(|e| PyRuntimeError::new_err(format!("Decoding error: {}", e)))?;

    let dict = PyDict::new(py);

    match msg {
        ClientMessage::Input { data } => {
            dict.set_item("type", "input")?;
            dict.set_item("data", data)?;
        }
        ClientMessage::Resize { cols, rows } => {
            dict.set_item("type", "resize")?;
            dict.set_item("cols", cols)?;
            dict.set_item("rows", rows)?;
        }
        ClientMessage::Ping => {
            dict.set_item("type", "ping")?;
        }
        ClientMessage::RequestRefresh => {
            dict.set_item("type", "refresh")?;
        }
        ClientMessage::Subscribe { events } => {
            dict.set_item("type", "subscribe")?;
            let event_strs: Vec<&str> = events
                .iter()
                .map(|e| match e {
                    crate::streaming::protocol::EventType::Output => "output",
                    crate::streaming::protocol::EventType::Cursor => "cursor",
                    crate::streaming::protocol::EventType::Bell => "bell",
                    crate::streaming::protocol::EventType::Title => "title",
                    crate::streaming::protocol::EventType::Resize => "resize",
                })
                .collect();
            dict.set_item("events", event_strs)?;
        }
    }

    Ok(dict)
}

// Stub functions for non-streaming builds
#[cfg(not(feature = "streaming"))]
#[pyfunction]
#[pyo3(signature = (_message_type, **_kwargs))]
pub fn encode_server_message<'py>(
    _py: Python<'py>,
    _message_type: &str,
    _kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    Err(PyRuntimeError::new_err(
        "Streaming feature not enabled. Rebuild with --features streaming",
    ))
}

#[cfg(not(feature = "streaming"))]
#[pyfunction]
pub fn decode_server_message<'py>(
    _py: Python<'py>,
    _data: &[u8],
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    Err(PyRuntimeError::new_err(
        "Streaming feature not enabled. Rebuild with --features streaming",
    ))
}

#[cfg(not(feature = "streaming"))]
#[pyfunction]
#[pyo3(signature = (_message_type, **_kwargs))]
pub fn encode_client_message<'py>(
    _py: Python<'py>,
    _message_type: &str,
    _kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    Err(PyRuntimeError::new_err(
        "Streaming feature not enabled. Rebuild with --features streaming",
    ))
}

#[cfg(not(feature = "streaming"))]
#[pyfunction]
pub fn decode_client_message<'py>(
    _py: Python<'py>,
    _data: &[u8],
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    Err(PyRuntimeError::new_err(
        "Streaming feature not enabled. Rebuild with --features streaming",
    ))
}
