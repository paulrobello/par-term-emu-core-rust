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
#[pyclass(name = "StreamingConfig", from_py_object)]
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
    #[pyo3(signature = (max_clients=1000, send_initial_screen=true, keepalive_interval=30, default_read_only=false, initial_cols=0, initial_rows=0, enable_http=false, web_root="./web_term", max_clients_per_session=0, input_rate_limit_bytes_per_sec=0, enable_system_stats=false, system_stats_interval_secs=5, api_key=None, allow_api_key_in_query=false, allowed_origins=None))]
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
        max_clients_per_session: usize,
        input_rate_limit_bytes_per_sec: usize,
        enable_system_stats: bool,
        system_stats_interval_secs: u64,
        api_key: Option<String>,
        allow_api_key_in_query: bool,
        allowed_origins: Option<Vec<String>>,
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
                max_sessions: 10,
                session_idle_timeout: 900,
                presets: std::collections::HashMap::new(),
                max_clients_per_session,
                input_rate_limit_bytes_per_sec,
                enable_system_stats,
                system_stats_interval_secs,
                api_key,
                allow_api_key_in_query,
                allowed_origins,
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

    /// Get the maximum number of concurrent sessions
    #[getter]
    fn max_sessions(&self) -> usize {
        self.inner.max_sessions
    }

    /// Set the maximum number of concurrent sessions
    #[setter]
    fn set_max_sessions(&mut self, max_sessions: usize) {
        self.inner.max_sessions = max_sessions;
    }

    /// Get the idle session timeout in seconds (0 = never timeout)
    #[getter]
    fn session_idle_timeout(&self) -> u64 {
        self.inner.session_idle_timeout
    }

    /// Set the idle session timeout in seconds (0 = never timeout)
    #[setter]
    fn set_session_idle_timeout(&mut self, session_idle_timeout: u64) {
        self.inner.session_idle_timeout = session_idle_timeout;
    }

    /// Get the maximum clients per session (0 = unlimited)
    #[getter]
    fn max_clients_per_session(&self) -> usize {
        self.inner.max_clients_per_session
    }

    /// Set the maximum clients per session (0 = unlimited)
    #[setter]
    fn set_max_clients_per_session(&mut self, max_clients_per_session: usize) {
        self.inner.max_clients_per_session = max_clients_per_session;
    }

    /// Get the input rate limit in bytes per second (0 = unlimited)
    #[getter]
    fn input_rate_limit_bytes_per_sec(&self) -> usize {
        self.inner.input_rate_limit_bytes_per_sec
    }

    /// Set the input rate limit in bytes per second (0 = unlimited)
    #[setter]
    fn set_input_rate_limit_bytes_per_sec(&mut self, input_rate_limit_bytes_per_sec: usize) {
        self.inner.input_rate_limit_bytes_per_sec = input_rate_limit_bytes_per_sec;
    }

    /// Get whether system stats collection is enabled
    #[getter]
    fn enable_system_stats(&self) -> bool {
        self.inner.enable_system_stats
    }

    /// Set whether system stats collection is enabled
    #[setter]
    fn set_enable_system_stats(&mut self, enable_system_stats: bool) {
        self.inner.enable_system_stats = enable_system_stats;
    }

    /// Get the system stats collection interval in seconds
    #[getter]
    fn system_stats_interval_secs(&self) -> u64 {
        self.inner.system_stats_interval_secs
    }

    /// Set the system stats collection interval in seconds
    #[setter]
    fn set_system_stats_interval_secs(&mut self, system_stats_interval_secs: u64) {
        self.inner.system_stats_interval_secs = system_stats_interval_secs;
    }

    /// Get the API key for authentication (None if not set)
    #[getter]
    fn api_key(&self) -> Option<String> {
        self.inner.api_key.clone()
    }

    /// Set the API key for authentication (None to disable)
    #[setter]
    fn set_api_key(&mut self, api_key: Option<String>) {
        self.inner.api_key = api_key;
    }

    /// Get whether API key authentication via query parameter is allowed
    #[getter]
    fn allow_api_key_in_query(&self) -> bool {
        self.inner.allow_api_key_in_query
    }

    /// Set whether to allow API key authentication via query parameter.
    /// Disabled by default because query params are logged by proxies and saved in browser history.
    #[setter]
    fn set_allow_api_key_in_query(&mut self, allow: bool) {
        self.inner.allow_api_key_in_query = allow;
    }

    /// Get the allowed browser origins allowlist (None = local/non-browser only).
    #[getter]
    fn allowed_origins(&self) -> Option<Vec<String>> {
        self.inner.allowed_origins.clone()
    }

    /// Set the allowed browser origins for WebSocket and CORS (SEC-005).
    /// When None, only non-browser clients and local (loopback) browser origins
    /// are accepted. Set to a list of origin strings (e.g.
    /// ["https://app.example.com"]) to allow specific remote browser origins.
    #[setter]
    fn set_allowed_origins(&mut self, origins: Option<Vec<String>>) {
        self.inner.allowed_origins = origins;
    }

    fn __repr__(&self) -> String {
        let tls_status = if self.inner.tls.is_some() {
            ", tls=enabled"
        } else {
            ""
        };
        let api_key_status = if self.inner.api_key.is_some() {
            ", api_key=***"
        } else {
            ""
        };
        format!(
            "StreamingConfig(max_clients={}, send_initial_screen={}, keepalive_interval={}, default_read_only={}, initial_cols={}, initial_rows={}, enable_http={}, web_root='{}'{}{}, enable_system_stats={}, system_stats_interval_secs={})",
            self.inner.max_clients,
            self.inner.send_initial_screen,
            self.inner.keepalive_interval,
            self.inner.default_read_only,
            self.inner.initial_cols,
            self.inner.initial_rows,
            self.inner.enable_http,
            self.inner.web_root,
            tls_status,
            api_key_status,
            self.inner.enable_system_stats,
            self.inner.system_stats_interval_secs,
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
                        if output_sender.try_send(output).is_err() {
                            crate::debug_info!("STREAMING", "Output channel full, message dropped");
                        }
                    }
                    Err(error) => {
                        // Find how much is valid
                        let valid_up_to = error.valid_up_to();

                        if valid_up_to > 0 {
                            // Send the valid portion
                            let valid_str = std::str::from_utf8(&buffer[..valid_up_to])
                                .expect("valid_up_to guarantees valid UTF-8");
                            let output = valid_str.to_string();
                            if output_sender.try_send(output).is_err() {
                                crate::debug_info!(
                                    "STREAMING",
                                    "Output channel full, message dropped"
                                );
                            }

                            // Keep only the incomplete sequence for next time
                            buffer.drain(..valid_up_to);
                        }

                        // If buffer gets too large (>100 bytes of invalid data),
                        // it's probably not a partial sequence, flush it
                        if buffer.len() > 100 {
                            let output = String::from_utf8_lossy(&buffer).to_string();
                            buffer.clear();
                            if output_sender.try_send(output).is_err() {
                                crate::debug_info!(
                                    "STREAMING",
                                    "Output channel full, message dropped"
                                );
                            }
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

    /// Send a mode changed event to all clients
    fn send_mode_changed(&self, mode: String, enabled: bool) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_mode_changed(mode, enabled);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a graphics added event to all clients
    fn send_graphics_added(&self, row: u16) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_graphics_added(row);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a hyperlink added event to all clients
    #[pyo3(signature = (url, row, col, id=None))]
    fn send_hyperlink_added(
        &self,
        url: String,
        row: u16,
        col: u16,
        id: Option<String>,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_hyperlink_added(url, row, col, id);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a user variable changed event to all clients
    #[pyo3(signature = (name, value, old_value=None))]
    fn send_user_var_changed(
        &self,
        name: String,
        value: String,
        old_value: Option<String>,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_user_var_changed(name, value, old_value);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a cursor position event to all clients
    fn send_cursor_position(&self, col: u16, row: u16, visible: bool) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_cursor_position(col, row, visible);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a badge changed event to all clients
    #[pyo3(signature = (badge=None))]
    fn send_badge_changed(&self, badge: Option<String>) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_badge_changed(badge);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a trigger action notify event to all clients
    ///
    /// Args:
    ///     trigger_id: ID of the trigger that produced this action
    ///     title: Notification title
    ///     message: Notification message
    fn send_action_notify(&self, trigger_id: u64, title: String, message: String) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_action_notify(trigger_id, title, message);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a trigger action mark line event to all clients
    ///
    /// Args:
    ///     trigger_id: ID of the trigger that produced this action
    ///     row: Row to mark
    ///     label: Optional label for the mark
    ///     color: Optional RGB color tuple (r, g, b)
    #[pyo3(signature = (trigger_id, row, label=None, color=None))]
    fn send_action_mark_line(
        &self,
        trigger_id: u64,
        row: u16,
        label: Option<String>,
        color: Option<(u8, u8, u8)>,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_action_mark_line(trigger_id, row, label, color);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a CWD changed event to all clients
    ///
    /// Args:
    ///     new_cwd: The new working directory path
    ///     old_cwd: The previous working directory path (optional)
    ///     hostname: Hostname associated with the CWD (optional)
    ///     username: Username associated with the CWD (optional)
    ///     timestamp: Unix timestamp of the change
    #[pyo3(signature = (new_cwd, old_cwd=None, hostname=None, username=None, timestamp=0))]
    fn send_cwd_changed(
        &self,
        new_cwd: String,
        old_cwd: Option<String>,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_cwd_changed(old_cwd, new_cwd, hostname, username, timestamp);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a trigger matched event to all clients
    ///
    /// Args:
    ///     trigger_id: ID of the trigger that matched
    ///     row: Row where the match occurred
    ///     col: Starting column of the match
    ///     end_col: Ending column of the match
    ///     text: The matched text
    ///     captures: List of capture group strings
    ///     timestamp: Unix timestamp of the match
    #[pyo3(signature = (trigger_id, row, col, end_col, text, captures=vec![], timestamp=0))]
    #[allow(clippy::too_many_arguments)]
    fn send_trigger_matched(
        &self,
        trigger_id: u64,
        row: u16,
        col: u16,
        end_col: u16,
        text: String,
        captures: Vec<String>,
        timestamp: u64,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_trigger_matched(trigger_id, row, col, end_col, text, captures, timestamp);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a progress bar changed event to all clients
    ///
    /// Args:
    ///     action: Action string ("set", "remove", or "remove_all")
    ///     id: Progress bar identifier
    ///     state: Optional ProgressState enum value
    ///     percent: Optional progress percentage (0-100)
    ///     label: Optional label text
    #[pyo3(signature = (action, id, state=None, percent=None, label=None))]
    fn send_progress_bar_changed(
        &self,
        action: String,
        id: String,
        state: Option<super::enums::PyProgressState>,
        percent: Option<u8>,
        label: Option<String>,
    ) -> PyResult<()> {
        if let Some(server) = &self.server {
            let action = match action.as_str() {
                "set" => crate::terminal::ProgressBarAction::Set,
                "remove" => crate::terminal::ProgressBarAction::Remove,
                "remove_all" => crate::terminal::ProgressBarAction::RemoveAll,
                _ => {
                    return Err(PyRuntimeError::new_err(format!(
                        "Invalid action '{}': must be 'set', 'remove', or 'remove_all'",
                        action
                    )));
                }
            };
            let state = state.map(|s| s.into());
            server.send_progress_bar_changed(action, id, state, percent, label);
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
///     message_type: Type of message (the `type` tag of the decoded dict,
///         e.g. "output", "connected", "cursor")
///     **kwargs: Message-specific fields (one per decoded dict key; missing
///         or wrong-typed values fall back to per-field defaults)
///
/// Returns:
///     bytes: Binary protobuf encoded message
///
/// Raises:
///     RuntimeError: If the type is unknown, encoding fails, or the
///         streaming feature is not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
#[pyo3(signature = (message_type, **kwargs))]
pub fn encode_server_message<'py>(
    py: Python<'py>,
    message_type: &str,
    kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::streaming::protocol::ServerMessage;

    let msg = ServerMessage::from_py_kwargs(message_type, kwargs)?.ok_or_else(|| {
        PyRuntimeError::new_err(format!(
            "Unknown message type: {}. Valid types: {}",
            message_type,
            ServerMessage::py_type_tags().join(", ")
        ))
    })?;

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
    let msg = crate::streaming::decode_server_message(data)
        .map_err(|e| PyRuntimeError::new_err(format!("Decoding error: {}", e)))?;

    msg.to_py_dict(py)
}

/// Encode a client message to binary protobuf format
///
/// Args:
///     message_type: Type of message (the `type` tag of the decoded dict,
///         e.g. "input", "resize", "subscribe")
///     **kwargs: Message-specific fields (one per decoded dict key; missing
///         or wrong-typed values fall back to per-field defaults)
///
/// Returns:
///     bytes: Binary protobuf encoded message
///
/// Raises:
///     RuntimeError: If the type is unknown, encoding fails, or the
///         streaming feature is not enabled
#[cfg(feature = "streaming")]
#[pyfunction]
#[pyo3(signature = (message_type, **kwargs))]
pub fn encode_client_message<'py>(
    py: Python<'py>,
    message_type: &str,
    kwargs: Option<&Bound<'py, pyo3::types::PyDict>>,
) -> PyResult<Bound<'py, PyBytes>> {
    use crate::streaming::protocol::ClientMessage;

    let msg = ClientMessage::from_py_kwargs(message_type, kwargs)?.ok_or_else(|| {
        PyRuntimeError::new_err(format!(
            "Unknown message type: {}. Valid types: {}",
            message_type,
            ClientMessage::py_type_tags().join(", ")
        ))
    })?;

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
    let msg = crate::streaming::decode_client_message(data)
        .map_err(|e| PyRuntimeError::new_err(format!("Decoding error: {}", e)))?;

    msg.to_py_dict(py)
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
