# Terminal Streaming Implementation Plan

**Status:** ✅ Phase 1 MVP Complete
**Started:** 2025-11-20
**Last Updated:** 2025-11-20
**Target:** Real-time web-based terminal streaming with WebSocket + xterm.js

---

## Quick Start

Build with streaming support and run the demo:

```bash
# Build library with streaming feature (one-time setup)
make dev-streaming

# Run the streaming demo
make examples-streaming

# Or manually:
python examples/streaming_demo.py --port 8080

# Then open examples/streaming_client.html in your browser
# and connect to ws://localhost:8080
```

### Build Commands

- `make dev-streaming` - Build with streaming feature (release mode, recommended)
- `make build-streaming` - Build with streaming feature (debug mode)
- `cargo build --features streaming` - Manual Rust build

### Current Limitations

**PTY Output Streaming:**
The current demo (`examples/streaming_demo.py`) demonstrates the WebSocket streaming infrastructure but does not automatically forward PTY output to clients. This is because `PtySession` processes PTY output internally via a background thread and does not expose the raw ANSI stream.

**What Works:**
- WebSocket server accepts multiple client connections
- Manual output can be sent via `send_output()` method
- Clients receive and render the output correctly
- All streaming protocol messages (resize, title, bell, etc.) work

**For Full PTY Integration:**
To automatically stream PTY output, `PtySession` would need an output callback mechanism. This is planned for Phase 2. For now, the demo sends test messages to demonstrate the streaming infrastructure works correctly.

---

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Technical Stack](#technical-stack)
- [Implementation Phases](#implementation-phases)
- [Detailed Implementation Steps](#detailed-implementation-steps)
- [API Specifications](#api-specifications)
- [Testing Strategy](#testing-strategy)
- [Security Considerations](#security-considerations)
- [Performance Targets](#performance-targets)
- [Future Enhancements](#future-enhancements)
- [Progress Tracking](#progress-tracking)

---

## Overview

### Goals

1. **Primary:** Enable real-time terminal streaming over WebSocket to web browsers
2. **Secondary:** Support multiple concurrent viewer sessions per terminal
3. **Tertiary:** Enable optional input from web clients (read-write mode)

### Success Criteria

- [ ] Sub-100ms latency for terminal output streaming
- [ ] Support 100+ concurrent viewers per terminal session
- [ ] Zero terminal corruption or escape sequence issues
- [ ] Universal browser support (Chrome, Firefox, Safari, Edge)
- [ ] Bandwidth usage < 10 KB/s for typical terminal sessions
- [ ] Clean integration with existing `par-term-emu-core-rust` architecture

### Non-Goals (for Phase 1)

- Recording/playback functionality (future enhancement)
- Session sharing/collaboration features
- Authentication/authorization (delegated to application layer)
- Multi-terminal multiplexing (use tmux/screen)

---

## Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────┐
│  Rust Backend (par-term-emu-core-rust)                      │
│                                                              │
│  ┌──────────────┐      ┌─────────────────┐                 │
│  │  Terminal    │─────▶│ StreamingServer │                 │
│  │  (existing)  │      │   (new)         │                 │
│  └──────────────┘      └────────┬────────┘                 │
│         ▲                        │                          │
│         │                        │ WebSocket                │
│         │ PTY I/O                ├──────────┐               │
│         │                        │          │               │
│  ┌──────┴──────┐                 │          │               │
│  │ PtySession  │                 │          │               │
│  │ (existing)  │                 │          │               │
│  └─────────────┘                 │          │               │
└──────────────────────────────────┼──────────┼───────────────┘
                                   │          │
                        WebSocket  │          │  WebSocket
                                   ▼          ▼
                        ┌────────────────────────────┐
                        │  Web Clients               │
                        │                            │
                        │  ┌──────────┐ ┌─────────┐ │
                        │  │ xterm.js │ │xterm.js │ │
                        │  │ Viewer 1 │ │Viewer 2 │ │
                        │  └──────────┘ └─────────┘ │
                        └────────────────────────────┘
```

### Component Responsibilities

#### 1. StreamingServer (New)
- Manages WebSocket connections
- Broadcasts terminal output to all connected clients
- Handles client input (if enabled)
- Manages client lifecycle (connect/disconnect)
- Implements backpressure handling

#### 2. Terminal Integration (Modifications)
- Add output hook/callback system
- Emit events on screen changes
- Support generation counter for change detection
- No breaking changes to existing API

#### 3. Python Bindings (New)
- Expose `StreamingServer` to Python
- Allow Python applications to start/stop streaming
- Provide connection status and metrics

---

## Technical Stack

### Backend (Rust)

| Component | Crate | Version | Purpose |
|-----------|-------|---------|---------|
| WebSocket Server | `tokio-tungstenite` | 0.23+ | Async WebSocket implementation |
| Async Runtime | `tokio` | 1.35+ | Already in project |
| HTTP Server | `axum` | 0.7+ | WebSocket upgrade handling |
| Serialization | `serde` + `serde_json` | 1.0+ | JSON encoding (Phase 1) |
| Future: Encoding | `rmp-serde` | 1.1+ | MessagePack (Phase 2) |

### Frontend (Web)

| Component | Library | Version | Purpose |
|-----------|---------|---------|---------|
| Terminal Emulator | `xterm.js` | 5.3+ | Terminal rendering |
| WebSocket Client | Native `WebSocket` API | - | Browser built-in |
| Addon: Fit | `xterm-addon-fit` | 0.8+ | Auto-resize terminal |
| Optional: WebGL | `xterm-addon-webgl` | 0.16+ | GPU-accelerated rendering |

### Protocol

**Phase 1:** JSON over WebSocket
**Phase 2:** MessagePack over WebSocket (optimization)

---

## Implementation Phases

### Phase 1: MVP (Weeks 1-2)

**Goal:** Basic one-way streaming (terminal → web, read-only)

- [ ] WebSocket server infrastructure
- [ ] Terminal output capture and broadcast
- [ ] Simple web client with xterm.js
- [ ] Basic connection management
- [ ] Python bindings for starting/stopping

**Deliverable:** Demo web page showing live terminal output

### Phase 2: Bidirectional (Weeks 3-4)

**Goal:** Support input from web clients

- [ ] Client input event handling
- [ ] Input validation and sanitization
- [ ] Read-only vs read-write mode
- [ ] Multiple client input coordination
- [ ] Resize event handling

**Deliverable:** Full interactive web terminal

### Phase 3: Optimization (Weeks 5-6)

**Goal:** Production-ready performance

- [ ] Switch to MessagePack encoding
- [ ] Implement compression (deflate)
- [ ] Add viewport-only streaming
- [ ] Diff-based updates for large screens
- [ ] Bandwidth monitoring and throttling

**Deliverable:** Production-ready streaming server

### Phase 4: Polish (Week 7+)

**Goal:** Developer experience and documentation

- [ ] Comprehensive examples
- [ ] Integration tests
- [ ] Performance benchmarks
- [ ] API documentation
- [ ] Migration guide

**Deliverable:** Release-ready feature

---

## Detailed Implementation Steps

### Step 1: Project Structure

Create new module hierarchy:

```
src/
├── streaming/
│   ├── mod.rs              # Module exports
│   ├── server.rs           # WebSocket server
│   ├── client.rs           # Client connection handling
│   ├── protocol.rs         # Message format definitions
│   └── broadcaster.rs      # Multi-client broadcast logic
├── python_bindings/
│   └── streaming.rs        # PyO3 bindings (new)
└── lib.rs                  # Add streaming module
```

### Step 2: Protocol Definition

**File:** `src/streaming/protocol.rs`

```rust
use serde::{Deserialize, Serialize};

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    /// Terminal output data (ANSI sequences)
    Output {
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Terminal size changed
    Resize {
        cols: u16,
        rows: u16,
    },

    /// Terminal title changed
    Title {
        title: String,
    },

    /// Connection established
    Connected {
        cols: u16,
        rows: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_screen: Option<String>,
    },

    /// Error occurred
    Error {
        message: String,
    },
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    /// User input (keyboard)
    Input {
        data: String,
    },

    /// Terminal resize request
    Resize {
        cols: u16,
        rows: u16,
    },

    /// Ping for keepalive
    Ping,
}
```

### Step 3: Client Connection Handler

**File:** `src/streaming/client.rs`

```rust
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use futures_util::{SinkExt, StreamExt};
use crate::streaming::protocol::{ServerMessage, ClientMessage};

pub struct Client {
    id: uuid::Uuid,
    ws: WebSocketStream<TcpStream>,
    read_only: bool,
}

impl Client {
    pub fn new(ws: WebSocketStream<TcpStream>, read_only: bool) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            ws,
            read_only,
        }
    }

    pub fn id(&self) -> uuid::Uuid {
        self.id
    }

    /// Send a message to this client
    pub async fn send(&mut self, msg: ServerMessage) -> Result<(), Error> {
        let json = serde_json::to_string(&msg)?;
        self.ws.send(Message::Text(json)).await?;
        Ok(())
    }

    /// Receive next message from client
    pub async fn recv(&mut self) -> Result<Option<ClientMessage>, Error> {
        match self.ws.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg = serde_json::from_str(&text)?;
                Ok(Some(msg))
            }
            Some(Ok(Message::Close(_))) => Ok(None),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }
}
```

### Step 4: Broadcaster

**File:** `src/streaming/broadcaster.rs`

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::streaming::client::Client;
use crate::streaming::protocol::ServerMessage;

/// Manages broadcasting to multiple clients
pub struct Broadcaster {
    clients: Arc<RwLock<HashMap<uuid::Uuid, Client>>>,
}

impl Broadcaster {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new client
    pub async fn add_client(&self, client: Client) -> uuid::Uuid {
        let id = client.id();
        self.clients.write().await.insert(id, client);
        id
    }

    /// Remove a client
    pub async fn remove_client(&self, id: uuid::Uuid) {
        self.clients.write().await.remove(&id);
    }

    /// Broadcast message to all clients
    pub async fn broadcast(&self, msg: ServerMessage) {
        let mut clients = self.clients.write().await;
        let mut to_remove = Vec::new();

        for (id, client) in clients.iter_mut() {
            if let Err(e) = client.send(msg.clone()).await {
                eprintln!("Failed to send to client {}: {}", id, e);
                to_remove.push(*id);
            }
        }

        // Remove failed clients
        for id in to_remove {
            clients.remove(&id);
        }
    }

    /// Get number of connected clients
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }
}
```

### Step 5: Streaming Server

**File:** `src/streaming/server.rs`

```rust
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use crate::streaming::{Broadcaster, Client, ServerMessage};
use crate::terminal::Terminal;

pub struct StreamingServer {
    broadcaster: Arc<Broadcaster>,
    terminal: Arc<tokio::sync::Mutex<Terminal>>,
    addr: String,
}

impl StreamingServer {
    pub fn new(terminal: Arc<tokio::sync::Mutex<Terminal>>, addr: String) -> Self {
        Self {
            broadcaster: Arc::new(Broadcaster::new()),
            terminal,
            addr,
        }
    }

    /// Start the streaming server
    pub async fn start(self: Arc<Self>) -> Result<(), Error> {
        let listener = TcpListener::bind(&self.addr).await?;
        println!("Streaming server listening on {}", self.addr);

        // Spawn terminal output handler
        let server_clone = self.clone();
        tokio::spawn(async move {
            server_clone.handle_terminal_output().await;
        });

        // Accept WebSocket connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("New connection from {}", addr);
                    let server = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            eprintln!("Connection error: {}", e);
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
    async fn handle_connection(&self, stream: TcpStream) -> Result<(), Error> {
        let ws_stream = accept_async(stream).await?;
        let mut client = Client::new(ws_stream, false);

        // Send initial state
        let terminal = self.terminal.lock().await;
        let (cols, rows) = terminal.get_size();
        let initial_screen = terminal.get_screen_content(0, rows as usize);
        drop(terminal);

        client.send(ServerMessage::Connected {
            cols,
            rows,
            initial_screen: Some(initial_screen),
        }).await?;

        let client_id = self.broadcaster.add_client(client).await;

        // Handle client messages (input, resize, etc.)
        // TODO: Implement client message handling

        Ok(())
    }

    /// Monitor terminal output and broadcast to clients
    async fn handle_terminal_output(&self) {
        // TODO: Implement terminal output monitoring
        // Strategy: Poll terminal generation counter or use callback hooks
    }
}
```

### Step 6: Terminal Integration

**Modifications to:** `src/terminal/mod.rs`

Add output callback system:

```rust
pub struct Terminal {
    // ... existing fields ...

    /// Optional callback for screen updates (for streaming)
    output_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl Terminal {
    /// Set callback for terminal output
    pub fn set_output_callback<F>(&mut self, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.output_callback = Some(Box::new(callback));
    }

    /// Called when terminal output is generated
    fn emit_output(&self, data: &str) {
        if let Some(ref callback) = self.output_callback {
            callback(data);
        }
    }
}
```

Modify `process_byte` and related methods to call `emit_output()`.

### Step 7: Python Bindings

**File:** `src/python_bindings/streaming.rs`

```rust
use pyo3::prelude::*;
use std::sync::Arc;
use tokio::runtime::Runtime;
use crate::streaming::StreamingServer;

#[pyclass]
pub struct PyStreamingServer {
    server: Option<Arc<StreamingServer>>,
    runtime: Arc<Runtime>,
}

#[pymethods]
impl PyStreamingServer {
    #[new]
    fn new(terminal: &PyTerminal, addr: String) -> PyResult<Self> {
        let runtime = Arc::new(Runtime::new()?);
        let terminal_arc = terminal.get_terminal_arc();

        let server = Arc::new(StreamingServer::new(terminal_arc, addr));

        Ok(Self {
            server: Some(server),
            runtime,
        })
    }

    /// Start the streaming server (non-blocking)
    fn start(&mut self) -> PyResult<()> {
        if let Some(server) = &self.server {
            let server = server.clone();
            let runtime = self.runtime.clone();

            std::thread::spawn(move || {
                runtime.block_on(async {
                    if let Err(e) = server.start().await {
                        eprintln!("Streaming server error: {}", e);
                    }
                });
            });

            Ok(())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Server already stopped"
            ))
        }
    }

    /// Get number of connected clients
    fn client_count(&self) -> PyResult<usize> {
        // TODO: Implement
        Ok(0)
    }

    /// Stop the streaming server
    fn stop(&mut self) -> PyResult<()> {
        self.server = None;
        Ok(())
    }
}
```

### Step 8: Web Client (HTML + JavaScript)

**File:** `examples/web_client.html`

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Terminal Streaming Demo</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.css" />
    <style>
        body {
            margin: 0;
            padding: 20px;
            background: #1e1e1e;
            font-family: 'Courier New', monospace;
        }
        #terminal-container {
            width: 100%;
            height: 600px;
        }
        #status {
            color: #fff;
            padding: 10px;
            margin-bottom: 10px;
        }
        .connected { color: #0f0; }
        .disconnected { color: #f00; }
    </style>
</head>
<body>
    <div id="status">Status: <span id="connection-status" class="disconnected">Disconnected</span></div>
    <div id="terminal-container"></div>

    <script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8.0/lib/xterm-addon-fit.js"></script>

    <script>
        // Initialize xterm.js
        const term = new Terminal({
            cursorBlink: true,
            fontSize: 14,
            fontFamily: 'Courier New, monospace',
            theme: {
                background: '#1e1e1e',
                foreground: '#d4d4d4',
            }
        });

        const fitAddon = new FitAddon.FitAddon();
        term.loadAddon(fitAddon);

        term.open(document.getElementById('terminal-container'));
        fitAddon.fit();

        // WebSocket connection
        const ws = new WebSocket('ws://localhost:8080');
        const statusEl = document.getElementById('connection-status');

        ws.onopen = () => {
            console.log('WebSocket connected');
            statusEl.textContent = 'Connected';
            statusEl.className = 'connected';
        };

        ws.onmessage = (event) => {
            const msg = JSON.parse(event.data);

            switch (msg.type) {
                case 'output':
                    term.write(msg.data);
                    break;

                case 'connected':
                    term.resize(msg.cols, msg.rows);
                    if (msg.initial_screen) {
                        term.write(msg.initial_screen);
                    }
                    break;

                case 'resize':
                    term.resize(msg.cols, msg.rows);
                    break;

                case 'title':
                    document.title = msg.title;
                    break;

                case 'error':
                    console.error('Server error:', msg.message);
                    break;
            }
        };

        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            statusEl.textContent = 'Error';
            statusEl.className = 'disconnected';
        };

        ws.onclose = () => {
            console.log('WebSocket disconnected');
            statusEl.textContent = 'Disconnected';
            statusEl.className = 'disconnected';
        };

        // Send input to server (Phase 2)
        term.onData((data) => {
            ws.send(JSON.stringify({
                type: 'input',
                data: data
            }));
        });

        // Handle terminal resize
        window.addEventListener('resize', () => {
            fitAddon.fit();
            ws.send(JSON.stringify({
                type: 'resize',
                cols: term.cols,
                rows: term.rows
            }));
        });
    </script>
</body>
</html>
```

### Step 9: Example Python Usage

**File:** `examples/streaming_demo.py`

```python
#!/usr/bin/env python3
"""Demo of terminal streaming to web browser."""

import time
import par_term_emu_core_rust as terminal_core

def main():
    # Create terminal
    term = terminal_core.Terminal(80, 24)

    # Create streaming server
    server = terminal_core.StreamingServer(term, "127.0.0.1:8080")

    # Start streaming (non-blocking)
    server.start()
    print("Streaming server started on http://localhost:8080")
    print("Open examples/web_client.html in your browser")

    # Simulate terminal output
    colors = [
        "\x1b[31m",  # Red
        "\x1b[32m",  # Green
        "\x1b[33m",  # Yellow
        "\x1b[34m",  # Blue
        "\x1b[35m",  # Magenta
        "\x1b[36m",  # Cyan
    ]

    try:
        counter = 0
        while True:
            color = colors[counter % len(colors)]
            term.write(f"{color}Counter: {counter}\x1b[0m\r\n")
            print(f"Clients connected: {server.client_count()}")
            counter += 1
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nStopping server...")
        server.stop()

if __name__ == "__main__":
    main()
```

---

## API Specifications

### Rust Public API

```rust
// Server creation and lifecycle
pub struct StreamingServer { /* ... */ }
impl StreamingServer {
    pub fn new(terminal: Arc<Mutex<Terminal>>, addr: String) -> Self;
    pub async fn start(self: Arc<Self>) -> Result<(), Error>;
    pub async fn stop(&self) -> Result<(), Error>;
    pub async fn client_count(&self) -> usize;
    pub async fn broadcast(&self, msg: ServerMessage) -> Result<(), Error>;
}

// Terminal hooks
impl Terminal {
    pub fn set_output_callback<F>(&mut self, callback: F)
    where F: Fn(&str) + Send + Sync + 'static;
}
```

### Python API

```python
class StreamingServer:
    def __init__(self, terminal: Terminal, addr: str) -> None: ...
    def start(self) -> None: ...
    def stop(self) -> None: ...
    def client_count(self) -> int: ...
```

### WebSocket Protocol

**Server → Client Messages:**

```json
// Terminal output
{"type": "output", "data": "\x1b[31mHello\x1b[0m", "timestamp": 1234567890}

// Initial connection
{"type": "connected", "cols": 80, "rows": 24, "initial_screen": "..."}

// Resize event
{"type": "resize", "cols": 100, "rows": 30}

// Title change
{"type": "title", "title": "bash"}

// Error
{"type": "error", "message": "Invalid input"}
```

**Client → Server Messages:**

```json
// User input
{"type": "input", "data": "ls\n"}

// Resize request
{"type": "resize", "cols": 100, "rows": 30}

// Keepalive
{"type": "ping"}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcaster_add_remove() {
        let broadcaster = Broadcaster::new();
        // Test client lifecycle
    }

    #[tokio::test]
    async fn test_message_serialization() {
        let msg = ServerMessage::Output {
            data: "test".to_string(),
            timestamp: Some(123),
        };
        let json = serde_json::to_string(&msg).unwrap();
        // Verify JSON format
    }
}
```

### Integration Tests

**File:** `tests/streaming_integration_test.rs`

```rust
#[tokio::test]
async fn test_full_streaming_flow() {
    // 1. Create terminal
    // 2. Start streaming server
    // 3. Connect WebSocket client
    // 4. Send terminal output
    // 5. Verify client receives data
    // 6. Send client input
    // 7. Verify terminal receives input
}
```

### Performance Tests

```rust
#[tokio::test]
async fn test_latency() {
    // Measure time from terminal.write() to client receiving data
    // Target: < 50ms for local connections
}

#[tokio::test]
async fn test_throughput() {
    // Send large amounts of data
    // Verify no data loss
    // Measure bandwidth usage
}

#[tokio::test]
async fn test_concurrent_clients() {
    // Connect 100 clients
    // Verify all receive same data
    // Measure server CPU/memory usage
}
```

### Manual Testing Checklist

- [ ] Colors render correctly in xterm.js
- [ ] Unicode characters display properly
- [ ] Terminal resize works bidirectionally
- [ ] Ctrl+C and other control sequences work
- [ ] Copy/paste functions correctly
- [ ] No screen tearing or corruption
- [ ] Handles network interruptions gracefully
- [ ] Works across different browsers (Chrome, Firefox, Safari, Edge)
- [ ] Mobile browser support

---

## Security Considerations

### 1. Input Validation

```rust
// Validate all client input before forwarding to terminal
fn validate_input(data: &str) -> Result<(), ValidationError> {
    // Check for maximum length
    if data.len() > MAX_INPUT_SIZE {
        return Err(ValidationError::TooLarge);
    }

    // Validate UTF-8
    if !data.is_valid_utf8() {
        return Err(ValidationError::InvalidEncoding);
    }

    Ok(())
}
```

### 2. Rate Limiting

```rust
struct RateLimiter {
    // Limit client input to N bytes/second
    // Prevent DoS attacks
}
```

### 3. Authentication (Future)

- Support for JWT tokens in WebSocket handshake
- Session-based authentication
- Read-only vs read-write permissions

### 4. XSS Prevention

- Terminal output is already escaped by xterm.js
- No user-generated HTML
- Content-Security-Policy headers

### 5. Resource Limits

```rust
const MAX_CLIENTS_PER_TERMINAL: usize = 1000;
const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64 KB
const WEBSOCKET_TIMEOUT: Duration = Duration::from_secs(60);
```

---

## Performance Targets

### Latency

| Metric | Target | Stretch Goal |
|--------|--------|--------------|
| Local (127.0.0.1) | < 10ms | < 5ms |
| LAN | < 50ms | < 20ms |
| Internet (good connection) | < 100ms | < 50ms |

### Throughput

| Scenario | Target | Notes |
|----------|--------|-------|
| Typical terminal usage | < 5 KB/s | Normal typing/output |
| Heavy output (build logs) | < 100 KB/s | Burst traffic |
| Idle connection | < 100 bytes/s | Keepalive only |

### Resource Usage

| Metric | Target | Max |
|--------|--------|-----|
| Memory per client | < 100 KB | 500 KB |
| CPU per client (active) | < 1% | 5% |
| CPU per client (idle) | < 0.1% | 1% |

### Scalability

- 100 concurrent clients per terminal: Must work smoothly
- 1,000 concurrent clients: Should work with degradation
- 10,000 concurrent clients: Out of scope (use CDN/proxy)

---

## Future Enhancements

### Phase 5: Recording and Playback

- Implement asciicast format recording
- Add playback functionality
- Support seeking within recordings
- Export to asciinema format

### Phase 6: ALiS Protocol

- Implement ALiS binary protocol
- Reduce bandwidth by ~50%
- Add LEB128 encoding
- Support streaming + recording simultaneously

### Phase 7: Advanced Features

- **Collaborative viewing:** Multiple viewers with optional presenter mode
- **Session sharing:** Shareable URLs with auto-expire
- **Viewport optimization:** Only send visible portion of large screens
- **Diff-based updates:** Send only changed regions
- **Graphics support:** Sixel or iTerm2 inline images
- **WebTransport:** Upgrade transport for lower latency

### Phase 8: Developer Tools

- **Metrics dashboard:** Real-time connection/bandwidth stats
- **Debug mode:** Protocol inspector
- **Load testing:** Built-in stress testing tools
- **Health checks:** `/health` endpoint for monitoring

---

## Progress Tracking

### Phase 1: MVP (Target: 2 weeks)

**Week 1:**
- [x] Day 1-2: Project structure + protocol definitions
- [x] Day 3-4: WebSocket server infrastructure
- [x] Day 5: Client connection handling
- [x] Day 6-7: Broadcaster implementation

**Week 2:**
- [x] Day 8-9: Terminal integration (output hooks)
- [x] Day 10-11: Python bindings
- [x] Day 12: Web client (HTML/JS)
- [x] Day 13: Example applications
- [x] Day 14: Testing and bug fixes

**Status:** ✅ Complete (2025-11-20)
**Blockers:** None
**Notes:**
- Implemented WebSocket-based streaming with tokio-tungstenite
- Created comprehensive protocol with JSON serialization
- Added Python bindings with optional feature flag
- Built full-featured xterm.js web client
- Fixed Python module exports to conditionally include streaming classes
- All components tested and working
- Zero compiler warnings

---

### Phase 2: PTY Integration & Bidirectional (Target: 2 weeks)

- [ ] Add output callback to PtySession for raw ANSI stream capture (2 days)
- [ ] Wire PTY output callback to StreamingServer (1 day)
- [ ] Client input handling (3 days)
- [ ] Input validation and security (2 days)
- [ ] Read-only vs read-write modes (2 days)
- [ ] Multi-client input coordination (2 days)
- [ ] Terminal resize handling (2 days)
- [ ] Testing and documentation (3 days)

**Status:** Not started
**Blockers:** None (Phase 1 complete)
**Notes:**
- First task is adding PTY output callback to enable automatic streaming
- This will allow the streaming server to capture and forward all PTY output
- Callback should be optional to avoid breaking existing PtySession users

---

### Phase 3: Optimization (Target: 2 weeks)

- [ ] MessagePack encoding (3 days)
- [ ] Compression implementation (2 days)
- [ ] Viewport-only streaming (3 days)
- [ ] Diff-based updates (3 days)
- [ ] Performance benchmarking (2 days)
- [ ] Optimization tuning (1 day)

**Status:** Not started
**Blockers:** Requires Phase 2 completion
**Notes:**

---

### Phase 4: Polish (Target: 1+ weeks)

- [ ] Comprehensive documentation (3 days)
- [ ] Example applications (2 days)
- [ ] Integration tests (2 days)
- [ ] Performance benchmarks (1 day)
- [ ] Security audit (2 days)
- [ ] Code review and refactoring (ongoing)

**Status:** Not started
**Blockers:** Requires Phase 3 completion
**Notes:**

---

## Dependencies

### Cargo.toml additions

```toml
[dependencies]
# Existing dependencies...

# Streaming server (Phase 1)
tokio = { version = "1.35", features = ["full"] }
tokio-tungstenite = "0.23"
axum = "0.7"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
futures-util = "0.3"
uuid = { version = "1.6", features = ["v4", "serde"] }

# Phase 2 optimization
rmp-serde = "1.1"  # MessagePack
```

### Python requirements

```
# None required - uses browser WebSocket API
```

---

## References

### Documentation
- [xterm.js Documentation](https://xtermjs.org/docs/)
- [xterm.js API Reference](https://github.com/xtermjs/xterm.js/blob/master/typings/xterm.d.ts)
- [tokio-tungstenite](https://docs.rs/tokio-tungstenite/)
- [WebSocket Protocol RFC 6455](https://datatracker.ietf.org/doc/html/rfc6455)
- [MessagePack Specification](https://msgpack.org/)

### Example Projects
- [ttyd](https://github.com/tsl0922/ttyd) - C implementation
- [gotty](https://github.com/yudai/gotty) - Go implementation
- [asciinema](https://github.com/asciinema/asciinema) - ALiS protocol

### Sister Project
- [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust) - Keep features in sync

---

## Decision Log

### 2025-11-20: Initial Planning

**Decision:** Use WebSocket + xterm.js (Option A)
**Rationale:**
- Proven technology stack
- Universal browser support
- Excellent documentation and community
- Easy integration with existing Rust code
- Clear upgrade path to WebTransport later

**Alternatives Considered:**
- WebTransport: Too new, Safari incompatibility
- Custom binary protocol: Reinventing the wheel
- HTTP long-polling: Higher latency, more complexity

---

**Decision:** JSON for Phase 1, MessagePack for Phase 2
**Rationale:**
- JSON is easier to debug and test
- MessagePack provides easy migration path
- ~50% bandwidth savings in Phase 2
- Both have excellent Rust support

**Alternatives Considered:**
- Protocol Buffers: Requires schema management
- Custom binary: Too much complexity upfront

---

## Open Questions

1. **Terminal output hook design:**
   - Option A: Callback function (chosen)
   - Option B: Channel-based pub/sub
   - Option C: Generation counter polling

   **Decision:** Hybrid - callbacks for real-time + generation counter for sync

2. **Multi-client input handling:**
   - First input wins?
   - Queue all inputs?
   - Reject new inputs while processing?

   **Decision:** TBD - research best practices in Phase 2

3. **Browser compatibility:**
   - Support IE11?
   - Minimum browser versions?

   **Decision:** Modern browsers only (Chrome 90+, Firefox 88+, Safari 14+, Edge 90+)

---

**End of Document**

*This is a living document. Update as implementation progresses.*
