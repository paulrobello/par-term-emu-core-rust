//! Standalone Terminal Streaming Server
//!
//! A standalone executable for streaming terminal sessions over WebSocket.
//! This server creates a PTY terminal, starts a shell, and streams all terminal
//! output in real-time via WebSocket to connected clients.
//!
//! ## Features
//!
//! - Real-time terminal streaming via WebSocket
//! - Optional authentication (API key in header or URL param)
//! - Configurable color themes
//! - Graceful shutdown handling
//! - Automatic terminal resize support
//!
//! ## Usage
//!
//! ```bash
//! par-term-streamer --host 127.0.0.1 --port 8080 --theme iTerm2-dark
//! ```
//!
//! ## Authentication
//!
//! To enable authentication, use the `--api-key` flag:
//!
//! ```bash
//! par-term-streamer --api-key my-secret-key
//! ```
//!
//! Clients can then authenticate using either:
//! - Header: `Authorization: Bearer my-secret-key`
//! - URL param: `ws://localhost:8080?api_key=my-secret-key`

use anyhow::{Context, Result};
use clap::Parser;
use par_term_emu_core_rust::{
    color::Color,
    macros::{KeyParser, Macro, MacroEvent, MacroPlayback},
    pty_session::PtySession,
    streaming::{protocol::ThemeInfo, StreamingConfig, StreamingServer},
    terminal::Terminal,
};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info};

/// Terminal color theme definition
#[derive(Debug, Clone)]
struct Theme {
    name: String,
    background: Color,
    foreground: Color,
    normal: [Color; 8],
    bright: [Color; 8],
}

impl Theme {
    /// Create iTerm2 dark theme
    fn iterm2_dark() -> Self {
        Self {
            name: "iTerm2-dark".to_string(),
            background: Color::Rgb(0, 0, 0),
            foreground: Color::Rgb(255, 255, 255),
            normal: [
                Color::Rgb(0, 0, 0),
                Color::Rgb(201, 27, 0),
                Color::Rgb(0, 194, 0),
                Color::Rgb(199, 196, 0),
                Color::Rgb(2, 37, 199),
                Color::Rgb(201, 48, 199),
                Color::Rgb(0, 197, 199),
                Color::Rgb(199, 199, 199),
            ],
            bright: [
                Color::Rgb(104, 104, 104),
                Color::Rgb(255, 110, 103),
                Color::Rgb(95, 249, 103),
                Color::Rgb(254, 251, 103),
                Color::Rgb(104, 113, 255),
                Color::Rgb(255, 118, 255),
                Color::Rgb(96, 253, 255),
                Color::Rgb(255, 255, 255),
            ],
        }
    }

    /// Create Monokai theme
    fn monokai() -> Self {
        Self {
            name: "monokai".to_string(),
            background: Color::Rgb(12, 12, 12),
            foreground: Color::Rgb(217, 217, 217),
            normal: [
                Color::Rgb(26, 26, 26),
                Color::Rgb(244, 0, 95),
                Color::Rgb(152, 224, 36),
                Color::Rgb(253, 151, 31),
                Color::Rgb(157, 101, 255),
                Color::Rgb(244, 0, 95),
                Color::Rgb(88, 209, 235),
                Color::Rgb(196, 197, 181),
            ],
            bright: [
                Color::Rgb(98, 94, 76),
                Color::Rgb(244, 0, 95),
                Color::Rgb(152, 224, 36),
                Color::Rgb(224, 213, 97),
                Color::Rgb(157, 101, 255),
                Color::Rgb(244, 0, 95),
                Color::Rgb(88, 209, 235),
                Color::Rgb(246, 246, 239),
            ],
        }
    }

    /// Create Dracula theme
    fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            background: Color::Rgb(40, 42, 54),
            foreground: Color::Rgb(248, 248, 242),
            normal: [
                Color::Rgb(33, 34, 44),
                Color::Rgb(255, 85, 85),
                Color::Rgb(80, 250, 123),
                Color::Rgb(241, 250, 140),
                Color::Rgb(189, 147, 249),
                Color::Rgb(255, 121, 198),
                Color::Rgb(139, 233, 253),
                Color::Rgb(248, 248, 242),
            ],
            bright: [
                Color::Rgb(98, 114, 164),
                Color::Rgb(255, 110, 110),
                Color::Rgb(105, 255, 148),
                Color::Rgb(255, 255, 165),
                Color::Rgb(214, 172, 255),
                Color::Rgb(255, 146, 223),
                Color::Rgb(164, 255, 255),
                Color::Rgb(255, 255, 255),
            ],
        }
    }

    /// Create Solarized Dark theme
    fn solarized_dark() -> Self {
        Self {
            name: "solarized-dark".to_string(),
            background: Color::Rgb(0, 43, 54),
            foreground: Color::Rgb(131, 148, 150),
            normal: [
                Color::Rgb(7, 54, 66),
                Color::Rgb(220, 50, 47),
                Color::Rgb(133, 153, 0),
                Color::Rgb(181, 137, 0),
                Color::Rgb(38, 139, 210),
                Color::Rgb(211, 54, 130),
                Color::Rgb(42, 161, 152),
                Color::Rgb(238, 232, 213),
            ],
            bright: [
                Color::Rgb(0, 43, 54),
                Color::Rgb(203, 75, 22),
                Color::Rgb(88, 110, 117),
                Color::Rgb(101, 123, 131),
                Color::Rgb(131, 148, 150),
                Color::Rgb(108, 113, 196),
                Color::Rgb(147, 161, 161),
                Color::Rgb(253, 246, 227),
            ],
        }
    }

    /// Get theme by name
    fn by_name(name: &str) -> Option<Self> {
        match name {
            "iterm2-dark" => Some(Self::iterm2_dark()),
            "monokai" => Some(Self::monokai()),
            "dracula" => Some(Self::dracula()),
            "solarized-dark" => Some(Self::solarized_dark()),
            _ => None,
        }
    }

    /// Get list of available theme names
    fn available() -> Vec<&'static str> {
        vec!["iterm2-dark", "monokai", "dracula", "solarized-dark"]
    }

    /// Apply theme to terminal
    fn apply(&self, terminal: &mut Terminal) {
        terminal.set_default_bg(self.background);
        terminal.set_default_fg(self.foreground);

        // Set normal colors (0-7)
        for (i, color) in self.normal.iter().enumerate() {
            let _ = terminal.set_ansi_palette_color(i, *color);
        }

        // Set bright colors (8-15)
        for (i, color) in self.bright.iter().enumerate() {
            let _ = terminal.set_ansi_palette_color(i + 8, *color);
        }
    }

    /// Convert theme to protocol ThemeInfo for sending to clients
    fn to_protocol(&self) -> ThemeInfo {
        ThemeInfo {
            name: self.name.clone(),
            background: self.background.to_rgb(),
            foreground: self.foreground.to_rgb(),
            normal: [
                self.normal[0].to_rgb(),
                self.normal[1].to_rgb(),
                self.normal[2].to_rgb(),
                self.normal[3].to_rgb(),
                self.normal[4].to_rgb(),
                self.normal[5].to_rgb(),
                self.normal[6].to_rgb(),
                self.normal[7].to_rgb(),
            ],
            bright: [
                self.bright[0].to_rgb(),
                self.bright[1].to_rgb(),
                self.bright[2].to_rgb(),
                self.bright[3].to_rgb(),
                self.bright[4].to_rgb(),
                self.bright[5].to_rgb(),
                self.bright[6].to_rgb(),
                self.bright[7].to_rgb(),
            ],
        }
    }
}

/// Parse terminal size from "COLSxROWS" format (e.g., "120x40")
fn parse_size(s: &str) -> Result<(u16, u16), String> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid size format '{}'. Expected COLSxROWS (e.g., 120x40)",
            s
        ));
    }
    let cols = parts[0]
        .parse::<u16>()
        .map_err(|_| format!("Invalid columns value: {}", parts[0]))?;
    let rows = parts[1]
        .parse::<u16>()
        .map_err(|_| format!("Invalid rows value: {}", parts[1]))?;
    if cols == 0 || rows == 0 {
        return Err("Columns and rows must be greater than 0".to_string());
    }
    Ok((cols, rows))
}

/// Command line arguments
#[derive(Parser, Debug)]
#[command(name = "par-term-streamer")]
#[command(version, about = "Terminal streaming server with WebSocket support")]
struct Args {
    /// Host address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind to
    #[arg(long, short = 'p', default_value = "8099")]
    port: u16,

    /// Terminal size in COLSxROWS format (e.g., 120x40)
    /// Overrides --cols and --rows if specified
    #[arg(long, short = 's', value_parser = parse_size)]
    size: Option<(u16, u16)>,

    /// Terminal columns (width)
    #[arg(long, default_value = "80")]
    cols: u16,

    /// Terminal rows (height)
    #[arg(long, default_value = "24")]
    rows: u16,

    /// Scrollback buffer size (lines)
    #[arg(long, default_value = "10000")]
    scrollback: usize,

    /// Shell command to run (auto-detect if not specified)
    #[arg(long)]
    shell: Option<String>,

    /// Command to execute after shell starts (sent as input after 1 second delay)
    #[arg(long, short = 'c')]
    command: Option<String>,

    /// Color theme
    #[arg(
        long,
        default_value = "iterm2-dark",
        value_parser = clap::builder::PossibleValuesParser::new(Theme::available())
    )]
    theme: String,

    /// API key for authentication (optional)
    /// Clients must provide this via Authorization header or api_key URL param
    #[arg(long)]
    api_key: Option<String>,

    /// Maximum number of concurrent clients
    #[arg(long, default_value = "100")]
    max_clients: usize,

    /// Keepalive ping interval in seconds (0 to disable)
    #[arg(long, default_value = "30")]
    keepalive: u64,

    /// Enable verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,

    /// Enable HTTP static file serving
    #[arg(long)]
    enable_http: bool,

    /// Web root directory for static files
    #[arg(long, default_value = "./web_term")]
    web_root: String,

    /// Macro file to play back instead of running a shell
    #[arg(long)]
    macro_file: Option<String>,

    /// Macro playback speed multiplier (1.0 = normal, 2.0 = 2x speed)
    #[arg(long, default_value = "1.0")]
    macro_speed: f64,

    /// Loop macro playback continuously
    #[arg(long)]
    macro_loop: bool,
}

/// Main event loop state
struct ServerState {
    pty_session: Arc<Mutex<PtySession>>,
    streaming_server: Arc<StreamingServer>,
    resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
}

impl ServerState {
    /// Create new server state
    fn new(
        pty_session: Arc<Mutex<PtySession>>,
        streaming_server: Arc<StreamingServer>,
        resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
    ) -> Self {
        Self {
            pty_session,
            streaming_server,
            resize_rx,
        }
    }

    /// Handle resize requests from clients
    async fn handle_resize_requests(&self) {
        let mut rx = self.resize_rx.lock().await;

        while let Some((cols, rows)) = rx.recv().await {
            info!("Resizing terminal to {}x{}", cols, rows);

            // Resize the PTY session (this also resizes the terminal)
            if let Ok(mut session) = self.pty_session.lock() {
                if let Err(e) = session.resize(cols, rows) {
                    error!("Failed to resize PTY: {}", e);
                    continue;
                }
            }

            // Broadcast resize to all clients
            self.streaming_server.send_resize(cols, rows).await;
        }
    }

    /// Monitor PTY status
    async fn handle_pty_status(&self) {
        loop {
            // Check if PTY is still running
            {
                let session = match self.pty_session.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };

                if !session.is_running() {
                    info!("PTY session has exited");
                    break;
                }
            }

            // Check PTY status every 500ms
            time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Run the main event loop
    async fn run(&self) -> Result<()> {
        let resize_handle = {
            let state = self.clone();
            tokio::spawn(async move {
                state.handle_resize_requests().await;
            })
        };

        let status_handle = {
            let state = self.clone();
            tokio::spawn(async move {
                state.handle_pty_status().await;
            })
        };

        // Wait for Ctrl+C
        signal::ctrl_c().await?;
        info!("Received shutdown signal");

        // Cancel background tasks
        resize_handle.abort();
        status_handle.abort();

        Ok(())
    }
}

impl Clone for ServerState {
    fn clone(&self) -> Self {
        Self {
            pty_session: Arc::clone(&self.pty_session),
            streaming_server: Arc::clone(&self.streaming_server),
            resize_rx: Arc::clone(&self.resize_rx),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .with_thread_ids(false)
        .init();

    info!("Starting terminal streaming server");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));

    // Determine terminal size (--size takes precedence over --cols/--rows)
    let (cols, rows) = args.size.unwrap_or((args.cols, args.rows));

    // Create PTY session (this creates its own terminal internally)
    info!("Creating PTY session ({}x{})", cols, rows);
    let pty_session = PtySession::new(cols as usize, rows as usize, args.scrollback);

    // Get the terminal from the PTY session
    let terminal = pty_session.terminal();

    // Apply theme to the terminal
    let theme = Theme::by_name(&args.theme)
        .ok_or_else(|| anyhow::anyhow!("Unknown theme: {}", args.theme))?;
    info!("Applying theme: {}", theme.name);
    {
        let mut term = terminal.lock().unwrap();
        theme.apply(&mut term);
    }

    let pty_session = Arc::new(Mutex::new(pty_session));

    // Create streaming server configuration
    let config = StreamingConfig {
        max_clients: args.max_clients,
        send_initial_screen: true,
        keepalive_interval: args.keepalive,
        default_read_only: false,
        enable_http: args.enable_http,
        web_root: args.web_root.clone(),
        initial_cols: cols,
        initial_rows: rows,
    };

    // Create streaming server
    let addr = format!("{}:{}", args.host, args.port);
    info!("Creating streaming server on {}", addr);

    let mut streaming_server =
        StreamingServer::with_config(Arc::clone(&terminal), addr.clone(), config);

    // Set theme on streaming server
    streaming_server.set_theme(theme.to_protocol());

    // Get resize receiver for handling resize requests
    let resize_rx = streaming_server.get_resize_receiver();

    // Get output sender for the callback (before Arc)
    let output_sender = streaming_server.get_output_sender();

    // Check if we should play back a macro or run a shell
    if let Some(macro_file) = &args.macro_file {
        info!("Loading macro file: {}", macro_file);
        let macro_data = Macro::load_yaml(macro_file)
            .context(format!("Failed to load macro file: {}", macro_file))?;

        info!("Macro loaded: {}", macro_data.name);
        if let Some(desc) = &macro_data.description {
            info!("Description: {}", desc);
        }
        info!("Events: {}", macro_data.events.len());
        info!("Speed: {}x", args.macro_speed);
        if args.macro_loop {
            info!("Loop: enabled");
        }

        // Spawn macro playback task
        let pty_session_clone = Arc::clone(&pty_session);
        let output_sender_clone = output_sender.clone();
        let macro_speed = args.macro_speed;
        let macro_loop = args.macro_loop;
        tokio::spawn(async move {
            loop {
                let mut playback = MacroPlayback::with_speed(macro_data.clone(), macro_speed);
                info!("Starting macro playback: {}", playback.name());

                while !playback.is_finished() {
                    if let Some(event) = playback.next_event() {
                        match event {
                            MacroEvent::KeyPress { key, .. } => {
                                // Convert key to bytes and send to terminal
                                let bytes = KeyParser::parse_key(&key);
                                if let Ok(mut session) = pty_session_clone.lock() {
                                    // Write directly to terminal for macro playback
                                    session.write(&bytes).ok();
                                }
                            }
                            MacroEvent::Delay { duration, .. } => {
                                tokio::time::sleep(Duration::from_millis(
                                    (duration as f64 / macro_speed) as u64,
                                ))
                                .await;
                            }
                            MacroEvent::Screenshot { label, .. } => {
                                if let Some(label) = label {
                                    info!("Screenshot trigger: {}", label);
                                } else {
                                    info!("Screenshot trigger");
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }

                info!("Macro playback finished");
                if !macro_loop {
                    break;
                }
                info!("Restarting macro playback (loop enabled)");
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });

        // Set up output callback to send PTY output to streaming server
        {
            let mut session = pty_session.lock().unwrap();
            session.set_output_callback(Arc::new(move |data| {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = output_sender_clone.send(text);
            }));
        }

        // No PTY writer needed for macro playback
    } else {
        // Start shell FIRST (so PTY writer becomes available)
        info!("Starting shell");
        {
            let mut session = pty_session.lock().unwrap();
            if let Some(shell) = &args.shell {
                session
                    .spawn(shell, &[])
                    .context(format!("Failed to start shell: {}", shell))?;
            } else {
                session.spawn_shell().context("Failed to start shell")?;
            }
        }

        // Set up output callback to send PTY output to streaming server
        {
            let mut session = pty_session.lock().unwrap();
            session.set_output_callback(Arc::new(move |data| {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = output_sender.send(text);
            }));
        }

        // Get PTY writer for client input (AFTER shell is spawned)
        let pty_writer = {
            let session = pty_session.lock().unwrap();
            session.get_writer()
        };

        if let Some(writer) = pty_writer {
            streaming_server.set_pty_writer(writer);
        }
    }

    let streaming_server = Arc::new(streaming_server);

    // Print startup information
    println!("\n{}", "=".repeat(60));
    println!("  Terminal Streaming Server");
    println!("{}", "=".repeat(60));

    if args.enable_http {
        println!("\n  HTTP Server: http://{}", addr);
        println!("  WebSocket URL: ws://{}/ws", addr);
        println!("  Web Root: {}", args.web_root);
    } else {
        println!("\n  WebSocket URL: ws://{}", addr);
    }

    if let Some(api_key) = &args.api_key {
        println!("\n  Authentication: ENABLED");
        println!("  API Key: {}", "*".repeat(api_key.len().min(8)));
        println!("\n  Connect with:");
        println!("    - Header: Authorization: Bearer <api-key>");
        if args.enable_http {
            println!("    - URL: ws://{}/ws?api_key=<api-key>", addr);
        } else {
            println!("    - URL: ws://{}?api_key=<api-key>", addr);
        }
    } else {
        println!("\n  Authentication: DISABLED");
        if args.enable_http {
            println!("  WebSocket: ws://{}/ws", addr);
        } else {
            println!("  Connect to: ws://{}", addr);
        }
    }

    println!("\n  Theme: {}", theme.name);
    println!("  Terminal: {}x{}", cols, rows);
    println!("  Max clients: {}", args.max_clients);

    if let Some(macro_file) = &args.macro_file {
        println!("\n  Mode: MACRO PLAYBACK");
        println!("  Macro file: {}", macro_file);
        println!("  Speed: {}x", args.macro_speed);
        println!(
            "  Loop: {}",
            if args.macro_loop {
                "enabled"
            } else {
                "disabled"
            }
        );
    } else {
        println!("\n  Mode: INTERACTIVE SHELL");
        if let Some(command) = &args.command {
            println!("  Initial command: {}", command);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("\nPress Ctrl+C to stop the server\n");

    // Create server state
    let state = ServerState::new(
        Arc::clone(&pty_session),
        Arc::clone(&streaming_server),
        resize_rx,
    );

    // Start streaming server in background
    let server_handle = {
        let streaming_server = Arc::clone(&streaming_server);
        tokio::spawn(async move {
            if let Err(e) = streaming_server.start().await {
                error!("Streaming server error: {}", e);
            }
        })
    };

    // Send initial command after delay if specified (only for shell mode, not macro mode)
    if let Some(command) = &args.command {
        if args.macro_file.is_none() {
            let pty_session_clone = Arc::clone(&pty_session);
            let command = command.clone();
            tokio::spawn(async move {
                // Wait 1 second for shell prompt to settle
                time::sleep(Duration::from_secs(1)).await;
                info!("Sending initial command: {}", command);

                if let Ok(session) = pty_session_clone.lock() {
                    if let Some(writer) = session.get_writer() {
                        if let Ok(mut w) = writer.lock() {
                            // Send command followed by newline
                            let cmd_with_newline = format!("{}\n", command);
                            if let Err(e) = w.write_all(cmd_with_newline.as_bytes()) {
                                error!("Failed to send initial command: {}", e);
                            }
                            let _ = w.flush();
                        }
                    }
                }
            });
        }
    }

    // Run main event loop (this blocks until Ctrl+C)
    state.run().await?;

    // Cleanup
    info!("Shutting down...");

    // Shutdown streaming server
    streaming_server
        .shutdown("Server shutting down".to_string())
        .await;

    // Stop PTY
    if let Ok(session) = pty_session.lock() {
        if session.is_running() {
            // Try to gracefully exit the shell
            if let Some(writer) = session.get_writer() {
                if let Ok(mut w) = writer.lock() {
                    let _ = w.write_all(b"exit\n");
                    let _ = w.flush();
                }
            }
        }
    }

    // Wait a bit for graceful shutdown
    time::sleep(Duration::from_millis(500)).await;

    // Cancel server task
    server_handle.abort();

    info!("Goodbye!");

    Ok(())
}
