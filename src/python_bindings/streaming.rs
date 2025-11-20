//! Python bindings for terminal streaming

use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;
use std::sync::Arc;

#[cfg(feature = "streaming")]
use crate::streaming::{StreamingServer, StreamingConfig};

/// Python wrapper for StreamingConfig
#[cfg(feature = "streaming")]
#[pyclass(name = "StreamingConfig")]
#[derive(Clone)]
pub struct PyStreamingConfig {
    inner: StreamingConfig,
}

#[cfg(feature = "streaming")]
#[pymethods]
impl PyStreamingConfig {
    #[new]
    #[pyo3(signature = (max_clients=1000, send_initial_screen=true, keepalive_interval=30, default_read_only=false))]
    fn new(
        max_clients: usize,
        send_initial_screen: bool,
        keepalive_interval: u64,
        default_read_only: bool,
    ) -> Self {
        Self {
            inner: StreamingConfig {
                max_clients,
                send_initial_screen,
                keepalive_interval,
                default_read_only,
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

    fn __repr__(&self) -> String {
        format!(
            "StreamingConfig(max_clients={}, send_initial_screen={}, keepalive_interval={}, default_read_only={})",
            self.inner.max_clients,
            self.inner.send_initial_screen,
            self.inner.keepalive_interval,
            self.inner.default_read_only
        )
    }
}

/// Python wrapper for StreamingServer
#[cfg(feature = "streaming")]
#[pyclass(name = "StreamingServer")]
pub struct PyStreamingServer {
    server: Option<Arc<StreamingServer>>,
    runtime: Arc<tokio::runtime::Runtime>,
    addr: String,
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
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create tokio runtime: {}", e)))?;

        // Get the terminal Arc from PyPtyTerminal
        let terminal_arc = pty_terminal.get_terminal_arc();

        // Get the PTY writer for input handling
        let pty_writer = pty_terminal.get_pty_writer();

        let mut server = if let Some(cfg) = config {
            StreamingServer::with_config(terminal_arc, addr.clone(), cfg.inner)
        } else {
            StreamingServer::new(terminal_arc, addr.clone())
        };

        // Set the PTY writer if available
        if let Some(writer) = pty_writer {
            server.set_pty_writer(writer);
        }

        let server = Arc::new(server);

        // Get the output sender channel from the server
        let output_sender = server.get_output_sender();

        // Create a callback that forwards PTY output to the streaming server
        let callback = Arc::new(move |data: &[u8]| {
            // Convert bytes to UTF-8 string (lossy conversion for invalid UTF-8)
            let output = String::from_utf8_lossy(data).to_string();
            // Send to streaming server (non-blocking)
            let _ = output_sender.send(output);
        });

        // Set the callback on the PTY terminal
        pty_terminal.set_output_callback(callback);

        Ok(Self {
            server: Some(server),
            runtime: Arc::new(runtime),
            addr,
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
                        eprintln!("Streaming server error: {}", e);
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
            let server = server.clone();
            let runtime = self.runtime.clone();

            Ok(runtime.block_on(async {
                server.client_count().await
            }))
        } else {
            Ok(0)
        }
    }

    /// Send output data to all connected clients
    ///
    /// Args:
    ///     data: The output data to send (ANSI escape sequences)
    fn send_output(&self, data: String) -> PyResult<()> {
        if let Some(server) = &self.server {
            server.send_output(data)
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
            let server = server.clone();
            let runtime = self.runtime.clone();

            runtime.block_on(async {
                server.send_resize(cols, rows).await;
            });
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a title change event to all clients
    ///
    /// Args:
    ///     title: The new terminal title
    fn send_title(&self, title: String) -> PyResult<()> {
        if let Some(server) = &self.server {
            let server = server.clone();
            let runtime = self.runtime.clone();

            runtime.block_on(async {
                server.send_title(title).await;
            });
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Server has been stopped"))
        }
    }

    /// Send a bell event to all clients
    fn send_bell(&self) -> PyResult<()> {
        if let Some(server) = &self.server {
            let server = server.clone();
            let runtime = self.runtime.clone();

            runtime.block_on(async {
                server.send_bell().await;
            });
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
            let runtime = self.runtime.clone();

            runtime.block_on(async {
                server.shutdown(reason).await;
            });
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
            if self.server.is_some() { "active" } else { "stopped" }
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
    fn new(_pty_terminal: &mut crate::python_bindings::pty::PyPtyTerminal, _addr: String) -> PyResult<Self> {
        Err(PyRuntimeError::new_err(
            "Streaming feature not enabled. Rebuild with --features streaming"
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
            "Streaming feature not enabled. Rebuild with --features streaming"
        ))
    }
}
