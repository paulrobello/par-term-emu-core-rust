//! CLI definition for the standalone streaming server (ARC-005 split).
//!
//! Clap argument surface, value parsers, and TTY size detection. All options
//! can also be set via environment variables with the `PAR_TERM_` prefix.

use anyhow::Result;
use clap::Parser;

use crate::theme::Theme;

/// Get the current terminal size from the TTY
#[cfg(unix)]
pub fn get_tty_size() -> Option<(u16, u16)> {
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
pub fn get_tty_size() -> Option<(u16, u16)> {
    // On Windows, we could use GetConsoleScreenBufferInfo, but for simplicity
    // we return None and let the caller use defaults
    None
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

/// Parse a preset in "name=command" format
fn parse_preset(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("Invalid preset format '{}'. Expected name=command", s))?;
    let name = s[..pos].to_string();
    let command = s[pos + 1..].to_string();
    if name.is_empty() {
        return Err("Preset name cannot be empty".to_string());
    }
    if command.is_empty() {
        return Err(format!("Preset '{}' command cannot be empty", name));
    }
    Ok((name, command))
}

/// Command line arguments
#[derive(Parser, Debug)]
#[command(name = "par-term-streamer")]
#[command(version, about = "Terminal streaming server with WebSocket support")]
pub struct Args {
    /// Host address to bind to
    #[arg(long, default_value = "127.0.0.1", env = "PAR_TERM_HOST")]
    pub host: String,

    /// Port to bind to
    #[arg(long, short = 'p', default_value = "8099", env = "PAR_TERM_PORT")]
    pub port: u16,

    /// Terminal size in COLSxROWS format (e.g., 120x40)
    /// Overrides --cols and --rows if specified
    #[arg(long, short = 's', value_parser = parse_size, env = "PAR_TERM_SIZE")]
    pub size: Option<(u16, u16)>,

    /// Terminal columns (width)
    #[arg(long, default_value = "80", env = "PAR_TERM_COLS")]
    pub cols: u16,

    /// Terminal rows (height)
    #[arg(long, default_value = "24", env = "PAR_TERM_ROWS")]
    pub rows: u16,

    /// Use current terminal size (from TTY)
    /// Overrides --size, --cols, and --rows if specified
    #[arg(long, env = "PAR_TERM_USE_TTY_SIZE")]
    pub use_tty_size: bool,

    /// Scrollback buffer size (lines)
    #[arg(long, default_value = "10000", env = "PAR_TERM_SCROLLBACK")]
    pub scrollback: usize,

    /// Shell command to run (auto-detect if not specified)
    #[arg(long, env = "PAR_TERM_SHELL")]
    pub shell: Option<String>,

    /// Command to execute after shell starts (sent as input after 1 second delay)
    #[arg(long, short = 'c', env = "PAR_TERM_COMMAND")]
    pub command: Option<String>,

    /// Color theme
    #[arg(
        long,
        default_value = "iterm2-dark",
        value_parser = clap::builder::PossibleValuesParser::new(Theme::available()),
        env = "PAR_TERM_THEME"
    )]
    pub theme: String,

    /// API key for WebSocket authentication (optional)
    /// Clients must provide this via Authorization header or X-API-Key header
    #[arg(long, env = "PAR_TERM_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Allow API key authentication via query parameter (?api_key=...).
    /// Disabled by default because query params are logged by proxies and saved in browser history.
    #[arg(long, env = "PAR_TERM_ALLOW_API_KEY_IN_QUERY")]
    pub allow_api_key_in_query: bool,

    /// Allowed browser origins for WebSocket and CORS (SEC-005).
    /// Repeatable, e.g. --allowed-origins https://app.example.com.
    /// When omitted, only non-browser clients and local (loopback) browser
    /// origins are accepted; set this to allow specific remote browser origins.
    #[arg(
        long,
        value_name = "ORIGIN",
        env = "PAR_TERM_ALLOWED_ORIGINS",
        value_delimiter = ','
    )]
    pub allowed_origins: Option<Vec<String>>,

    /// Maximum number of concurrent clients
    #[arg(long, default_value = "100", env = "PAR_TERM_MAX_CLIENTS")]
    pub max_clients: usize,

    /// Keepalive ping interval in seconds (0 to disable)
    #[arg(long, default_value = "30", env = "PAR_TERM_KEEPALIVE")]
    pub keepalive: u64,

    /// Enable verbose logging
    #[arg(long, short = 'v', env = "PAR_TERM_VERBOSE")]
    pub verbose: bool,

    /// Enable HTTP static file serving
    #[arg(long, env = "PAR_TERM_ENABLE_HTTP")]
    pub enable_http: bool,

    /// Web root directory for static files
    #[arg(long, default_value = "./web_term", env = "PAR_TERM_WEB_ROOT")]
    pub web_root: String,

    /// Macro file to play back instead of running a shell
    #[arg(long, env = "PAR_TERM_MACRO_FILE")]
    pub macro_file: Option<String>,

    /// Macro playback speed multiplier (1.0 = normal, 2.0 = 2x speed)
    #[arg(long, default_value = "1.0", env = "PAR_TERM_MACRO_SPEED")]
    pub macro_speed: f64,

    /// Loop macro playback continuously
    #[arg(long, env = "PAR_TERM_MACRO_LOOP")]
    pub macro_loop: bool,

    /// Disable automatic shell restart when it exits
    /// By default, the shell is automatically restarted when it exits
    #[arg(long, env = "PAR_TERM_NO_RESTART_SHELL")]
    pub no_restart_shell: bool,

    /// Download prebuilt web frontend from GitHub releases
    /// When specified, downloads and extracts frontend to web-root, then exits
    #[arg(long, env = "PAR_TERM_DOWNLOAD_FRONTEND")]
    pub download_frontend: bool,

    /// Version of web frontend to download (e.g., "0.14.0")
    /// Defaults to "latest" which fetches the most recent release
    #[arg(long, default_value = "latest", env = "PAR_TERM_FRONTEND_VERSION")]
    pub frontend_version: String,

    /// TLS certificate file (PEM format)
    /// Use with --tls-key for separate cert/key files
    #[arg(long, requires = "tls_key", env = "PAR_TERM_TLS_CERT")]
    pub tls_cert: Option<String>,

    /// TLS private key file (PEM format)
    /// Use with --tls-cert for separate cert/key files
    #[arg(long, requires = "tls_cert", env = "PAR_TERM_TLS_KEY")]
    pub tls_key: Option<String>,

    /// Combined TLS PEM file containing both certificate and private key
    /// Alternative to using --tls-cert and --tls-key
    #[arg(long, conflicts_with_all = ["tls_cert", "tls_key"], env = "PAR_TERM_TLS_PEM")]
    pub tls_pem: Option<String>,

    // HTTP Basic Auth options
    /// Username for HTTP Basic Authentication
    #[arg(long, env = "PAR_TERM_HTTP_USER")]
    pub http_user: Option<String>,

    /// Password for HTTP Basic Authentication (clear text)
    /// Mutually exclusive with --http-password-hash
    #[arg(
        long,
        env = "PAR_TERM_HTTP_PASSWORD",
        conflicts_with = "http_password_hash",
        hide_env_values = true
    )]
    pub http_password: Option<String>,

    /// Password hash for HTTP Basic Authentication (htpasswd format)
    /// Supports: bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
    /// Mutually exclusive with --http-password
    #[arg(
        long,
        env = "PAR_TERM_HTTP_PASSWORD_HASH",
        conflicts_with = "http_password",
        hide_env_values = true
    )]
    pub http_password_hash: Option<String>,

    /// File containing password (reads first line)
    /// If line starts with $ or {SHA}, treated as hash; otherwise as clear text
    /// Overrides --http-password and --http-password-hash
    #[arg(long, env = "PAR_TERM_HTTP_PASSWORD_FILE")]
    pub http_password_file: Option<String>,

    // Multi-session options
    /// Maximum number of concurrent terminal sessions
    #[arg(long, default_value = "10", env = "PAR_TERM_MAX_SESSIONS")]
    pub max_sessions: usize,

    /// Idle session timeout in seconds (0 = never timeout)
    /// Sessions with no connected clients will be reaped after this duration
    #[arg(long, default_value = "900", env = "PAR_TERM_SESSION_IDLE_TIMEOUT")]
    pub session_idle_timeout: u64,

    /// Shell presets (can specify multiple: --preset python=python3 --preset node=node)
    /// Clients connect with ?preset=name to use a specific preset
    #[arg(long, value_parser = parse_preset)]
    pub preset: Vec<(String, String)>,

    /// Maximum clients per session (0 = unlimited)
    #[arg(long, default_value = "0", env = "PAR_TERM_MAX_CLIENTS_PER_SESSION")]
    pub max_clients_per_session: usize,

    /// Input rate limit in bytes per second (0 = unlimited)
    #[arg(long, default_value = "0", env = "PAR_TERM_INPUT_RATE_LIMIT")]
    pub input_rate_limit: usize,

    /// Enable system resource statistics collection (CPU, memory, disk, network)
    #[arg(long, env = "PAR_TERM_ENABLE_SYSTEM_STATS")]
    pub enable_system_stats: bool,

    /// System stats collection interval in seconds
    #[arg(long, default_value = "5", env = "PAR_TERM_SYSTEM_STATS_INTERVAL")]
    pub system_stats_interval: u64,
}
