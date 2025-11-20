//! WebSocket streaming server implementation

use crate::streaming::broadcaster::Broadcaster;
use crate::streaming::client::Client;
use crate::streaming::error::{Result, StreamingError};
use crate::streaming::protocol::ServerMessage;
use crate::terminal::Terminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, broadcast};
use tokio::time;
use tokio_tungstenite::accept_async;

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
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_clients: 1000,
            send_initial_screen: true,
            keepalive_interval: 30,
            default_read_only: false,
        }
    }
}

/// WebSocket streaming server for terminal sessions
pub struct StreamingServer {
    /// Broadcaster for managing multiple clients
    broadcaster: Arc<Broadcaster>,
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
    pty_writer: Option<Arc<Mutex<Box<dyn std::io::Write + Send>>>>,
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
        let broadcaster = Arc::new(Broadcaster::with_max_clients(config.max_clients));
        let (output_tx, output_rx) = mpsc::unbounded_channel();
        // Create broadcast channel for sending output to all clients (buffer 100 messages)
        let (broadcast_tx, _) = broadcast::channel(100);

        Self {
            broadcaster,
            terminal,
            addr,
            config,
            output_tx,
            output_rx: Arc::new(tokio::sync::Mutex::new(output_rx)),
            broadcast_tx,
            pty_writer: None,
        }
    }

    /// Set the PTY writer for handling client input
    ///
    /// This should be called before starting the server if PTY input is supported
    pub fn set_pty_writer(&mut self, writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>) {
        self.pty_writer = Some(writer);
    }

    /// Get a clone of the output sender channel
    ///
    /// This can be used to send terminal output to all connected clients
    pub fn get_output_sender(&self) -> mpsc::UnboundedSender<String> {
        self.output_tx.clone()
    }

    /// Get the current number of connected clients
    pub async fn client_count(&self) -> usize {
        self.broadcaster.client_count().await
    }

    /// Broadcast a message to all clients
    pub async fn broadcast(&self, msg: ServerMessage) {
        self.broadcaster.broadcast(msg).await;
    }

    /// Start the streaming server
    ///
    /// This method will block until the server is stopped
    pub async fn start(self: Arc<Self>) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        println!("Streaming server listening on {}", self.addr);

        // Spawn output broadcaster task
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.output_broadcaster_loop().await;
        });

        // Spawn keepalive task if enabled
        if self.config.keepalive_interval > 0 {
            let server_clone = self.clone();
            tokio::spawn(async move {
                server_clone.keepalive_loop().await;
            });
        }

        // Accept WebSocket connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("New connection from {}", addr);
                    let server = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            eprintln!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Handle a new WebSocket connection
    async fn handle_connection(&self, stream: TcpStream) -> Result<()> {
        // Upgrade to WebSocket
        let ws_stream = accept_async(stream)
            .await
            .map_err(|e| StreamingError::WebSocketError(e.to_string()))?;

        let mut client = Client::new(ws_stream, self.config.default_read_only);
        let client_id = client.id();

        // Send initial connection message
        let (cols, rows, initial_screen) = {
            let terminal = self.terminal.lock().unwrap();
            let (cols, rows) = terminal.size();

            let initial_screen = if self.config.send_initial_screen {
                Some(terminal.content())
            } else {
                None
            };

            (cols as u16, rows as u16, initial_screen)
        };

        let connect_msg = if let Some(screen) = initial_screen {
            ServerMessage::connected_with_screen(cols, rows, screen, client_id.to_string())
        } else {
            ServerMessage::connected(cols, rows, client_id.to_string())
        };

        client.send(connect_msg).await?;

        // Add client to broadcaster (takes ownership, so we need to change this)
        // For now, DON'T add to broadcaster - handle everything here
        // TODO: Refactor broadcaster to allow both sending and receiving per client

        println!("Client {} connected (1 total)", client_id);

        // Get PTY writer if available
        let pty_writer = self.pty_writer.clone();

        // Subscribe to output broadcasts
        let mut output_rx = self.broadcast_tx.subscribe();

        // Handle client input and output in this task
        loop {
            tokio::select! {
                // Receive message from client (input from web terminal)
                msg = client.recv() => {
                    match msg? {
                        Some(client_msg) => {
                            match client_msg {
                                crate::streaming::protocol::ClientMessage::Input { data } => {
                                    // Write input to PTY if available
                                    if let Some(ref writer) = pty_writer {
                                        eprintln!("[Input] Received {} bytes from client, writing to PTY", data.len());
                                        if let Ok(mut w) = writer.lock() {
                                            use std::io::Write;
                                            let _ = w.write_all(data.as_bytes());
                                            let _ = w.flush();
                                        }
                                    }
                                }
                                crate::streaming::protocol::ClientMessage::Resize { cols, rows } => {
                                    eprintln!("[Input] Client requested resize to {}x{}", cols, rows);
                                    // TODO: Implement resize handling
                                }
                                crate::streaming::protocol::ClientMessage::Ping => {
                                    // Pings are handled automatically by Client::recv()
                                }
                                crate::streaming::protocol::ClientMessage::RequestRefresh => {
                                    eprintln!("[Input] Client requested screen refresh");
                                    // TODO: Implement screen refresh
                                }
                                crate::streaming::protocol::ClientMessage::Subscribe { .. } => {
                                    eprintln!("[Input] Client sent subscribe message (not implemented)");
                                    // TODO: Implement subscription handling
                                }
                            }
                        }
                        None => {
                            // Client disconnected
                            println!("Client {} disconnected", client_id);
                            break;
                        }
                    }
                }

                // Receive output to broadcast to client
                output_msg = output_rx.recv() => {
                    if let Ok(msg) = output_msg {
                        client.send(msg).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Output broadcaster loop - forwards terminal output to all clients
    async fn output_broadcaster_loop(&self) {
        let mut rx = self.output_rx.lock().await;

        while let Some(data) = rx.recv().await {
            if !data.is_empty() {
                let subscriber_count = self.broadcast_tx.receiver_count();
                eprintln!("[Broadcaster] Received {} bytes, broadcasting to {} clients", data.len(), subscriber_count);
                let msg = ServerMessage::output(data);
                // Ignore send errors (means no receivers)
                let _ = self.broadcast_tx.send(msg);
            }
        }
    }

    /// Keepalive loop - periodically pings all clients
    async fn keepalive_loop(&self) {
        let interval = Duration::from_secs(self.config.keepalive_interval);
        let mut ticker = time::interval(interval);

        loop {
            ticker.tick().await;
            self.broadcaster.ping_all().await;
        }
    }

    /// Send terminal output to all connected clients
    pub fn send_output(&self, data: String) -> Result<()> {
        self.output_tx
            .send(data)
            .map_err(|_| StreamingError::ServerError("Output channel closed".to_string()))
    }

    /// Send a resize event to all clients
    pub async fn send_resize(&self, cols: u16, rows: u16) {
        let msg = ServerMessage::resize(cols, rows);
        self.broadcaster.broadcast(msg).await;
    }

    /// Send a title change event to all clients
    pub async fn send_title(&self, title: String) {
        let msg = ServerMessage::title(title);
        self.broadcaster.broadcast(msg).await;
    }

    /// Send a bell event to all clients
    pub async fn send_bell(&self) {
        let msg = ServerMessage::bell();
        self.broadcaster.broadcast(msg).await;
    }

    /// Shutdown the server and disconnect all clients
    pub async fn shutdown(&self, reason: String) {
        let msg = ServerMessage::shutdown(reason);
        self.broadcaster.broadcast(msg).await;
        self.broadcaster.disconnect_all().await;
    }
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
