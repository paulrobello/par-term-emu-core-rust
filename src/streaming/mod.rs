//! Terminal streaming over WebSocket
//!
//! This module provides real-time terminal streaming capabilities using WebSocket
//! protocol, allowing terminals to be viewed remotely in web browsers.
//!
//! # Features
//!
//! - Real-time bidirectional streaming (terminal output and user input)
//! - Multiple concurrent viewers per terminal session
//! - Sub-100ms latency for local connections
//! - Universal browser support via xterm.js integration
//! - Optional read-only mode for viewers
//!
//! # Architecture
//!
//! ```text
//! Terminal → StreamingServer → WebSocket → Browser (xterm.js)
//!         ↑                                     ↓
//!         └──────── Input events ──────────────┘
//! ```
//!
//! # Example Usage
//!
//! ```rust,no_run
//! # #[cfg(feature = "streaming")]
//! # {
//! use par_term_emu_core_rust::terminal::Terminal;
//! use par_term_emu_core_rust::streaming::StreamingServer;
//! use std::sync::Arc;
//! use parking_lot::Mutex;
//!
//! #[tokio::main]
//! async fn main() {
//!     let terminal = Arc::new(Mutex::new(Terminal::new(80, 24)));
//!     let server = StreamingServer::new(terminal, "127.0.0.1:8080".to_string());
//!
//!     // Start streaming (this will block)
//!     server.start().await.unwrap();
//! }
//! # }
//! ```

pub mod error;
pub mod protocol;

#[cfg(feature = "streaming")]
pub mod proto;

#[cfg(feature = "streaming")]
pub mod client;

#[cfg(feature = "streaming")]
pub mod broadcaster;

#[cfg(feature = "streaming")]
pub mod server;

// Re-export main types
pub use error::{Result, StreamingError};
pub use protocol::{ClientMessage, EventType, ServerMessage};

#[cfg(feature = "streaming")]
pub use broadcaster::Broadcaster;

#[cfg(feature = "streaming")]
pub use client::Client;

#[cfg(feature = "streaming")]
pub use server::{
    HttpBasicAuthConfig, PasswordConfig, StreamingConfig, StreamingServer, TlsConfig,
};

#[cfg(feature = "streaming")]
pub use proto::{
    decode_client_message, decode_server_message, encode_client_message, encode_server_message,
};
