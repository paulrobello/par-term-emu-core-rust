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
    resize_rx: Option<std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>>>>,
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

        // Get channels before wrapping server in Arc
        let output_sender = server.get_output_sender();
        let resize_rx = server.get_resize_receiver();

        let server = Arc::new(server);

        // Create UTF-8 decoder state for handling partial sequences
        // Multi-byte UTF-8 characters may be split across PTY reads
        let utf8_buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Create a callback that forwards PTY output to the streaming server
        let callback = {
            let utf8_buffer = Arc::clone(&utf8_buffer);
            Arc::new(move |data: &[u8]| {
                // Append new data to buffer
                let mut buffer = utf8_buffer.lock().unwrap();
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
