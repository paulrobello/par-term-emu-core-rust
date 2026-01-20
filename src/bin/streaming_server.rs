//! Standalone Terminal Streaming Server
//!
//! A standalone executable for streaming terminal sessions over WebSocket.
//! This server creates a PTY terminal, starts a shell, and streams all terminal
//! output in real-time via WebSocket to connected clients.
//!
//! ## Features
//!
//! - Real-time terminal streaming via WebSocket
//! - Optional WebSocket authentication (API key in header or URL param)
//! - Optional HTTP Basic Authentication for web frontend
//! - Environment variable support for all CLI options
//! - Configurable color themes
//! - Graceful shutdown handling
//! - Automatic terminal resize support
//! - TLS/SSL support
//!
//! ## Usage
//!
//! ```bash
//! par-term-streamer --host 127.0.0.1 --port 8080 --theme iterm2-dark
//! ```
//!
//! ## Environment Variables
//!
//! All CLI options can be set via environment variables with `PAR_TERM_` prefix:
//!
//! ```bash
//! export PAR_TERM_HOST=0.0.0.0
//! export PAR_TERM_PORT=8080
//! export PAR_TERM_HTTP_USER=admin
//! export PAR_TERM_HTTP_PASSWORD=secret
//! par-term-streamer --enable-http
//! ```
//!
//! ## WebSocket Authentication
//!
//! To enable WebSocket authentication, use the `--api-key` flag:
//!
//! ```bash
//! par-term-streamer --api-key my-secret-key
//! ```
//!
//! Clients can then authenticate using either:
//! - Header: `Authorization: Bearer my-secret-key`
//! - URL param: `ws://localhost:8080?api_key=my-secret-key`
//!
//! ## HTTP Basic Authentication
//!
//! To enable HTTP Basic Auth for the web frontend:
//!
//! ```bash
//! # With clear text password
//! par-term-streamer --enable-http --http-user admin --http-password secret
//!
//! # With htpasswd hash (bcrypt, apr1, sha1, md5crypt)
//! par-term-streamer --enable-http --http-user admin --http-password-hash '$apr1$...'
//!
//! # With password from file (auto-detects hash vs clear text)
//! par-term-streamer --enable-http --http-user admin --http-password-file /path/to/password
//! ```

// Use jemalloc for better server performance (5-15% throughput improvement)
// Only available on non-Windows platforms
#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use anyhow::{Context, Result};
use clap::Parser;
use flate2::read::GzDecoder;
use par_term_emu_core_rust::{
    color::Color,
    macros::{KeyParser, Macro, MacroEvent, MacroPlayback},
    pty_session::PtySession,
    streaming::{
        protocol::ThemeInfo, HttpBasicAuthConfig, StreamingConfig, StreamingServer, TlsConfig,
    },
    terminal::Terminal,
};
use parking_lot::Mutex;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tar::Archive;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info};

/// Get the current terminal size from the TTY
#[cfg(unix)]
fn get_tty_size() -> Option<(u16, u16)> {
    use std::io::IsTerminal;
    use std::os::unix::io::AsRawFd;

    let stdout = std::io::stdout();
    if !stdout.is_terminal() {
        return None;
    }

    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        let fd = stdout.as_raw_fd();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            Some((ws.ws_col, ws.ws_row))
        } else {
            None
        }
    }
}

/// Get the current terminal size from the TTY (Windows stub)
#[cfg(not(unix))]
fn get_tty_size() -> Option<(u16, u16)> {
    // On Windows, we could use GetConsoleScreenBufferInfo, but for simplicity
    // we return None and let the caller use defaults
    None
}

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
    #[arg(long, default_value = "127.0.0.1", env = "PAR_TERM_HOST")]
    host: String,

    /// Port to bind to
    #[arg(long, short = 'p', default_value = "8099", env = "PAR_TERM_PORT")]
    port: u16,

    /// Terminal size in COLSxROWS format (e.g., 120x40)
    /// Overrides --cols and --rows if specified
    #[arg(long, short = 's', value_parser = parse_size, env = "PAR_TERM_SIZE")]
    size: Option<(u16, u16)>,

    /// Terminal columns (width)
    #[arg(long, default_value = "80", env = "PAR_TERM_COLS")]
    cols: u16,

    /// Terminal rows (height)
    #[arg(long, default_value = "24", env = "PAR_TERM_ROWS")]
    rows: u16,

    /// Use current terminal size (from TTY)
    /// Overrides --size, --cols, and --rows if specified
    #[arg(long, env = "PAR_TERM_USE_TTY_SIZE")]
    use_tty_size: bool,

    /// Scrollback buffer size (lines)
    #[arg(long, default_value = "10000", env = "PAR_TERM_SCROLLBACK")]
    scrollback: usize,

    /// Shell command to run (auto-detect if not specified)
    #[arg(long, env = "PAR_TERM_SHELL")]
    shell: Option<String>,

    /// Command to execute after shell starts (sent as input after 1 second delay)
    #[arg(long, short = 'c', env = "PAR_TERM_COMMAND")]
    command: Option<String>,

    /// Color theme
    #[arg(
        long,
        default_value = "iterm2-dark",
        value_parser = clap::builder::PossibleValuesParser::new(Theme::available()),
        env = "PAR_TERM_THEME"
    )]
    theme: String,

    /// API key for WebSocket authentication (optional)
    /// Clients must provide this via Authorization header or api_key URL param
    #[arg(long, env = "PAR_TERM_API_KEY")]
    api_key: Option<String>,

    /// Maximum number of concurrent clients
    #[arg(long, default_value = "100", env = "PAR_TERM_MAX_CLIENTS")]
    max_clients: usize,

    /// Keepalive ping interval in seconds (0 to disable)
    #[arg(long, default_value = "30", env = "PAR_TERM_KEEPALIVE")]
    keepalive: u64,

    /// Enable verbose logging
    #[arg(long, short = 'v', env = "PAR_TERM_VERBOSE")]
    verbose: bool,

    /// Enable HTTP static file serving
    #[arg(long, env = "PAR_TERM_ENABLE_HTTP")]
    enable_http: bool,

    /// Web root directory for static files
    #[arg(long, default_value = "./web_term", env = "PAR_TERM_WEB_ROOT")]
    web_root: String,

    /// Macro file to play back instead of running a shell
    #[arg(long, env = "PAR_TERM_MACRO_FILE")]
    macro_file: Option<String>,

    /// Macro playback speed multiplier (1.0 = normal, 2.0 = 2x speed)
    #[arg(long, default_value = "1.0", env = "PAR_TERM_MACRO_SPEED")]
    macro_speed: f64,

    /// Loop macro playback continuously
    #[arg(long, env = "PAR_TERM_MACRO_LOOP")]
    macro_loop: bool,

    /// Disable automatic shell restart when it exits
    /// By default, the shell is automatically restarted when it exits
    #[arg(long, env = "PAR_TERM_NO_RESTART_SHELL")]
    no_restart_shell: bool,

    /// Download prebuilt web frontend from GitHub releases
    /// When specified, downloads and extracts frontend to web-root, then exits
    #[arg(long, env = "PAR_TERM_DOWNLOAD_FRONTEND")]
    download_frontend: bool,

    /// Version of web frontend to download (e.g., "0.14.0")
    /// Defaults to "latest" which fetches the most recent release
    #[arg(long, default_value = "latest", env = "PAR_TERM_FRONTEND_VERSION")]
    frontend_version: String,

    /// TLS certificate file (PEM format)
    /// Use with --tls-key for separate cert/key files
    #[arg(long, requires = "tls_key", env = "PAR_TERM_TLS_CERT")]
    tls_cert: Option<String>,

    /// TLS private key file (PEM format)
    /// Use with --tls-cert for separate cert/key files
    #[arg(long, requires = "tls_cert", env = "PAR_TERM_TLS_KEY")]
    tls_key: Option<String>,

    /// Combined TLS PEM file containing both certificate and private key
    /// Alternative to using --tls-cert and --tls-key
    #[arg(long, conflicts_with_all = ["tls_cert", "tls_key"], env = "PAR_TERM_TLS_PEM")]
    tls_pem: Option<String>,

    // HTTP Basic Auth options
    /// Username for HTTP Basic Authentication
    #[arg(long, env = "PAR_TERM_HTTP_USER")]
    http_user: Option<String>,

    /// Password for HTTP Basic Authentication (clear text)
    /// Mutually exclusive with --http-password-hash
    #[arg(
        long,
        env = "PAR_TERM_HTTP_PASSWORD",
        conflicts_with = "http_password_hash"
    )]
    http_password: Option<String>,

    /// Password hash for HTTP Basic Authentication (htpasswd format)
    /// Supports: bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
    /// Mutually exclusive with --http-password
    #[arg(
        long,
        env = "PAR_TERM_HTTP_PASSWORD_HASH",
        conflicts_with = "http_password"
    )]
    http_password_hash: Option<String>,

    /// File containing password (reads first line)
    /// If line starts with $ or {SHA}, treated as hash; otherwise as clear text
    /// Overrides --http-password and --http-password-hash
    #[arg(long, env = "PAR_TERM_HTTP_PASSWORD_FILE")]
    http_password_file: Option<String>,
}

/// Main event loop state
struct ServerState {
    pty_session: Arc<Mutex<PtySession>>,
    streaming_server: Arc<StreamingServer>,
    resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
    shell_command: Option<String>,
    restart_shell: bool,
}

impl ServerState {
    /// Create new server state
    fn new(
        pty_session: Arc<Mutex<PtySession>>,
        streaming_server: Arc<StreamingServer>,
        resize_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<(u16, u16)>>>,
        shell_command: Option<String>,
        restart_shell: bool,
    ) -> Self {
        Self {
            pty_session,
            streaming_server,
            resize_rx,
            shell_command,
            restart_shell,
        }
    }

    /// Handle resize requests from clients
    async fn handle_resize_requests(&self) {
        let mut rx = self.resize_rx.lock().await;

        while let Some((cols, rows)) = rx.recv().await {
            info!("Resizing terminal to {}x{}", cols, rows);

            // Resize the PTY session (this also resizes the terminal)
            {
                let mut session = self.pty_session.lock();
                if let Err(e) = session.resize(cols, rows) {
                    error!("Failed to resize PTY: {}", e);
                    continue;
                }
            }

            // Broadcast resize to all clients
            self.streaming_server.send_resize(cols, rows);
        }
    }

    /// Monitor PTY status and restart shell if configured
    async fn handle_pty_status(&self) {
        info!(
            "PTY status monitor started (restart_shell={})",
            self.restart_shell
        );

        loop {
            // Check if PTY is still running
            let should_restart = {
                let session = self.pty_session.lock();

                if !session.is_running() {
                    info!(
                        "PTY session has exited (restart_shell={})",
                        self.restart_shell
                    );
                    self.restart_shell
                } else {
                    false
                }
            };

            if should_restart {
                info!("Will restart shell in 500ms...");

                // Small delay before restart
                time::sleep(Duration::from_millis(500)).await;

                info!("Attempting to restart shell...");

                // Restart the shell
                let restart_result = {
                    let mut session = self.pty_session.lock();

                    if let Some(ref shell) = self.shell_command {
                        info!("Spawning custom shell: {}", shell);
                        session.spawn(shell, &[])
                    } else {
                        info!("Spawning default shell");
                        session.spawn_shell()
                    }
                };

                match restart_result {
                    Ok(_) => {
                        info!("Shell restarted successfully");

                        // Update the PTY writer in the streaming server
                        let pty_writer = {
                            let session = self.pty_session.lock();
                            session.get_writer()
                        };

                        if let Some(writer) = pty_writer {
                            info!("Updated PTY writer in streaming server");
                            self.streaming_server.set_pty_writer(writer);
                        } else {
                            error!("Failed to get PTY writer after restart");
                        }
                    }
                    Err(e) => {
                        error!("Failed to restart shell: {} - will retry in 5s", e);
                        // Wait a bit before trying again
                        time::sleep(Duration::from_secs(5)).await;
                    }
                }
            } else if !self.restart_shell {
                // If restart is disabled, check if shell exited and break
                let session = self.pty_session.lock();

                if !session.is_running() {
                    info!("Shell exited and restart is disabled, stopping monitor");
                    break;
                }
            }

            // Check PTY status every 500ms
            time::sleep(Duration::from_millis(500)).await;
        }

        info!("PTY status monitor exiting");
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

        // Wait for either Ctrl+C or PTY exit (when restart is disabled)
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received shutdown signal");
            }
            _ = status_handle => {
                // PTY exited and restart is disabled
                info!("Shell exited, shutting down server");
            }
        }

        // Signal broadcaster to shut down (prevents hang on shell exit)
        self.streaming_server
            .shutdown("Server shutting down".to_string());

        // Cancel background tasks
        resize_handle.abort();

        Ok(())
    }
}

impl Clone for ServerState {
    fn clone(&self) -> Self {
        Self {
            pty_session: Arc::clone(&self.pty_session),
            streaming_server: Arc::clone(&self.streaming_server),
            resize_rx: Arc::clone(&self.resize_rx),
            shell_command: self.shell_command.clone(),
            restart_shell: self.restart_shell,
        }
    }
}

/// GitHub API response for release information
#[derive(serde::Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

/// GitHub API response for release asset
#[derive(serde::Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

const GITHUB_REPO: &str = "paulrobello/par-term-emu-core-rust";
const FRONTEND_ARCHIVE_PREFIX: &str = "par-term-web-frontend-v";

/// Download and extract the web frontend from GitHub releases
async fn download_frontend(version: &str, web_root: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("par-term-streamer")
        .timeout(Duration::from_secs(60))
        .build()
        .context("Failed to create HTTP client")?;

    // Get release info from GitHub API
    let release_url = if version == "latest" {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            GITHUB_REPO
        )
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/tags/v{}",
            GITHUB_REPO, version
        )
    };

    println!("Fetching release info from GitHub...");
    let response = client
        .get(&release_url)
        .send()
        .await
        .context("Failed to fetch release info from GitHub")?;

    if !response.status().is_success() {
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            if version == "latest" {
                anyhow::bail!("No releases found for this repository");
            } else {
                anyhow::bail!("Release version '{}' not found", version);
            }
        }
        anyhow::bail!(
            "GitHub API request failed with status: {}",
            response.status()
        );
    }

    let release: GitHubRelease = response
        .json()
        .await
        .context("Failed to parse GitHub release info")?;

    println!("Found release: {}", release.tag_name);

    // Find the tar.gz frontend archive
    let archive_asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.starts_with(FRONTEND_ARCHIVE_PREFIX) && asset.name.ends_with(".tar.gz")
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Web frontend archive not found in release {}. Available assets: {}",
                release.tag_name,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    println!("Downloading: {}", archive_asset.name);
    println!("From: {}", archive_asset.browser_download_url);

    // Download the archive
    let response = client
        .get(&archive_asset.browser_download_url)
        .send()
        .await
        .context("Failed to download frontend archive")?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download archive: HTTP {}", response.status());
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        println!("Download size: {} bytes", len);
    }

    let archive_bytes = response
        .bytes()
        .await
        .context("Failed to read archive content")?;

    println!("Downloaded {} bytes", archive_bytes.len());

    // Create web root directory if it doesn't exist
    let web_root_path = Path::new(web_root);
    if web_root_path.exists() {
        println!("Clearing existing web root: {}", web_root);
        fs::remove_dir_all(web_root_path)
            .context(format!("Failed to remove existing directory: {}", web_root))?;
    }
    fs::create_dir_all(web_root_path)
        .context(format!("Failed to create web root directory: {}", web_root))?;

    // Extract the tar.gz archive
    println!("Extracting to: {}", web_root);
    let tar_gz = GzDecoder::new(archive_bytes.as_ref());
    let mut archive = Archive::new(tar_gz);

    archive
        .unpack(web_root_path)
        .context("Failed to extract archive")?;

    // Count extracted files
    let file_count = count_files(web_root_path)?;
    println!(
        "Successfully extracted {} files to {}",
        file_count, web_root
    );

    // Verify index.html exists
    let index_path = web_root_path.join("index.html");
    if !index_path.exists() {
        println!("Warning: index.html not found in extracted content");
    } else {
        println!("Frontend ready at: {}/index.html", web_root);
    }

    Ok(())
}

/// Count files recursively in a directory
fn count_files(path: &Path) -> Result<usize> {
    let mut count = 0;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path)?;
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Determine if a password string looks like an htpasswd hash
fn looks_like_hash(s: &str) -> bool {
    // bcrypt: $2a$, $2b$, $2y$
    // apr1: $apr1$
    // MD5 crypt: $1$
    // SHA1: {SHA}
    s.starts_with("$2a$")
        || s.starts_with("$2b$")
        || s.starts_with("$2y$")
        || s.starts_with("$apr1$")
        || s.starts_with("$1$")
        || s.starts_with("{SHA}")
}

/// Resolve HTTP Basic Auth configuration from CLI arguments
///
/// Priority: password_file > password_hash > password
fn resolve_http_basic_auth(args: &Args) -> Result<Option<HttpBasicAuthConfig>> {
    // If no username is provided, no auth is configured
    let username = match &args.http_user {
        Some(u) => u.clone(),
        None => {
            // Check if any password options are provided without a username
            if args.http_password.is_some()
                || args.http_password_hash.is_some()
                || args.http_password_file.is_some()
            {
                anyhow::bail!(
                    "HTTP Basic Auth password options require --http-user to be specified"
                );
            }
            return Ok(None);
        }
    };

    // Priority: file > hash > clear text
    if let Some(ref file_path) = args.http_password_file {
        // Read password from file (first line)
        let file = fs::File::open(file_path)
            .context(format!("Failed to open password file: {}", file_path))?;
        let reader = BufReader::new(file);
        let first_line = reader
            .lines()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Password file is empty: {}", file_path))?
            .context("Failed to read password file")?;

        let password_value = first_line.trim();
        if password_value.is_empty() {
            anyhow::bail!("Password file contains empty line: {}", file_path);
        }

        // Determine if it's a hash or clear text
        if looks_like_hash(password_value) {
            info!("Using password hash from file: {}", file_path);
            return Ok(Some(HttpBasicAuthConfig::with_hash(
                username,
                password_value.to_string(),
            )));
        } else {
            info!("Using clear text password from file: {}", file_path);
            return Ok(Some(HttpBasicAuthConfig::with_password(
                username,
                password_value.to_string(),
            )));
        }
    }

    if let Some(ref hash) = args.http_password_hash {
        info!("Using password hash from argument/environment");
        return Ok(Some(HttpBasicAuthConfig::with_hash(username, hash.clone())));
    }

    if let Some(ref password) = args.http_password {
        info!("Using clear text password from argument/environment");
        return Ok(Some(HttpBasicAuthConfig::with_password(
            username,
            password.clone(),
        )));
    }

    // Username provided but no password - this is an error
    anyhow::bail!(
        "--http-user requires one of: --http-password, --http-password-hash, or --http-password-file"
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --download-frontend command
    if args.download_frontend {
        println!("par-term-streamer v{}", env!("CARGO_PKG_VERSION"));
        println!("Downloading web frontend...\n");

        download_frontend(&args.frontend_version, &args.web_root).await?;

        println!("\nTo run the server with the downloaded frontend:");
        println!(
            "  par-term-streamer --enable-http --web-root {}",
            args.web_root
        );
        return Ok(());
    }

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

    // Determine terminal size
    // Priority: --use-tty-size > --size > --cols/--rows
    let (cols, rows) = if args.use_tty_size {
        match get_tty_size() {
            Some(size) => {
                info!("Using TTY size: {}x{}", size.0, size.1);
                size
            }
            None => {
                eprintln!("Warning: Could not get TTY size, using defaults (80x24)");
                (80, 24)
            }
        }
    } else {
        args.size.unwrap_or((args.cols, args.rows))
    };

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
        let mut term = terminal.lock();
        theme.apply(&mut term);
    }

    let pty_session = Arc::new(Mutex::new(pty_session));

    // Load TLS configuration if provided
    let tls_config = if let Some(pem_path) = &args.tls_pem {
        info!("Loading TLS from PEM file: {}", pem_path);
        Some(TlsConfig::from_pem(pem_path).context("Failed to load TLS PEM file")?)
    } else if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        info!("Loading TLS from cert: {}, key: {}", cert_path, key_path);
        Some(
            TlsConfig::from_files(cert_path, key_path)
                .context("Failed to load TLS certificate/key")?,
        )
    } else {
        None
    };

    let use_tls = tls_config.is_some();

    // Resolve HTTP Basic Auth configuration
    let http_basic_auth = resolve_http_basic_auth(&args)?;

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
        tls: tls_config,
        http_basic_auth: http_basic_auth.clone(),
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
                                {
                                    let mut session = pty_session_clone.lock();
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
            let mut session = pty_session.lock();
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
            let mut session = pty_session.lock();
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
            let mut session = pty_session.lock();
            session.set_output_callback(Arc::new(move |data| {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = output_sender.send(text);
            }));
        }

        // Get PTY writer for client input (AFTER shell is spawned)
        let pty_writer = {
            let session = pty_session.lock();
            session.get_writer()
        };

        if let Some(writer) = pty_writer {
            streaming_server.set_pty_writer(writer);
        }
    }

    let streaming_server = Arc::new(streaming_server);

    // Print startup information
    let http_scheme = if use_tls { "https" } else { "http" };
    let ws_scheme = if use_tls { "wss" } else { "ws" };

    println!("\n{}", "=".repeat(60));
    println!("  Terminal Streaming Server");
    if use_tls {
        println!("  (TLS/SSL ENABLED)");
    }
    println!("{}", "=".repeat(60));

    if args.enable_http {
        println!("\n  HTTP Server: {}://{}", http_scheme, addr);
        println!("  WebSocket URL: {}://{}/ws", ws_scheme, addr);
        println!("  Web Root: {}", args.web_root);
    } else {
        println!("\n  WebSocket URL: {}://{}", ws_scheme, addr);
    }

    // WebSocket API key authentication
    if let Some(api_key) = &args.api_key {
        println!("\n  WebSocket Auth: ENABLED (API Key)");
        println!("  API Key: {}", "*".repeat(api_key.len().min(8)));
        println!("\n  Connect with:");
        println!("    - Header: Authorization: Bearer <api-key>");
        if args.enable_http {
            println!("    - URL: {}://{}/ws?api_key=<api-key>", ws_scheme, addr);
        } else {
            println!("    - URL: {}://{}?api_key=<api-key>", ws_scheme, addr);
        }
    } else {
        println!("\n  WebSocket Auth: DISABLED");
        if args.enable_http {
            println!("  WebSocket: {}://{}/ws", ws_scheme, addr);
        } else {
            println!("  Connect to: {}://{}", ws_scheme, addr);
        }
    }

    // HTTP Basic Authentication
    if http_basic_auth.is_some() {
        println!("\n  HTTP Basic Auth: ENABLED");
    } else if args.enable_http {
        println!("\n  HTTP Basic Auth: DISABLED (no password protection)");
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
        println!(
            "  Shell restart: {}",
            if args.no_restart_shell {
                "disabled"
            } else {
                "enabled (default)"
            }
        );
    }

    println!("\n{}", "=".repeat(60));
    println!("\nPress Ctrl+C to stop the server\n");

    // Create server state
    // Shell restart only applies to shell mode, not macro mode
    let restart_shell = args.macro_file.is_none() && !args.no_restart_shell;
    let state = ServerState::new(
        Arc::clone(&pty_session),
        Arc::clone(&streaming_server),
        resize_rx,
        args.shell.clone(),
        restart_shell,
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

                {
                    let session = pty_session_clone.lock();
                    if let Some(writer) = session.get_writer() {
                        {
                            let mut w = writer.lock();
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
    streaming_server.shutdown("Server shutting down".to_string());

    // Stop PTY
    {
        let session = pty_session.lock();
        if session.is_running() {
            // Try to gracefully exit the shell
            if let Some(writer) = session.get_writer() {
                {
                    let mut w = writer.lock();
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
