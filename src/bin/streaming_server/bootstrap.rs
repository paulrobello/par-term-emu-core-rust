//! Server bootstrap for the standalone streaming server (ARC-005 split).
//!
//! Run-mode resolution, the macro-mode event loop state, the multi-session
//! PTY factory, and HTTP Basic Auth resolution. `main()` sequences these;
//! this module owns the wiring.

use anyhow::{Context, Result};
use par_term_emu_core_rust::pty_session::PtySession;
use par_term_emu_core_rust::streaming::{
    HttpBasicAuthConfig, SessionFactory, SessionFactoryResult, StreamSessionState, StreamingServer,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info, warn};

use crate::cli::Args;
use crate::theme::Theme;

/// How the server runs: macro playback against one dedicated PTY, or
/// interactive multi-session shells created on demand by the factory.
///
/// Resolved once from the CLI arguments so macro mode's precondition — a
/// live PTY session — holds by construction everywhere downstream, instead
/// of being re-asserted with `expect` at each use site.
pub enum RunMode {
    /// Play back a macro file against a dedicated single PTY session.
    Macro {
        /// Path of the YAML macro file to play.
        macro_file: String,
        /// The PTY session the macro plays into.
        pty_session: Arc<Mutex<PtySession>>,
    },
    /// Interactive shell mode; PTY sessions are created per-client by
    /// [`BinarySessionFactory`].
    Shell,
}

/// Resolve the run mode from CLI arguments, creating the macro-mode PTY
/// session (with the theme applied) when `--macro-file` is given.
pub fn resolve_run_mode(args: &Args, cols: u16, rows: u16, theme: &Theme) -> RunMode {
    match &args.macro_file {
        Some(macro_file) => {
            info!("Creating PTY session for macro mode ({}x{})", cols, rows);
            let ps = PtySession::new(cols as usize, rows as usize, args.scrollback);
            let terminal = ps.terminal();
            {
                let mut term = terminal.write();
                theme.apply(&mut term);
            }
            RunMode::Macro {
                macro_file: macro_file.clone(),
                pty_session: Arc::new(Mutex::new(ps)),
            }
        }
        None => RunMode::Shell,
    }
}

/// Run the macro-mode event loop (resize handling, PTY monitoring, event
/// polling) until Ctrl+C or PTY exit.
pub async fn run_macro_mode(
    pty_session: Arc<Mutex<PtySession>>,
    streaming_server: Arc<StreamingServer>,
    shell_command: Option<String>,
) -> Result<()> {
    let resize_rx = streaming_server.get_resize_receiver();
    let state = ServerState::new(
        pty_session,
        streaming_server,
        resize_rx,
        shell_command,
        false, // no restart in macro mode
    );
    state.run().await
}

/// Main event loop state for macro mode
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

    /// Poll terminal events and broadcast to clients
    async fn poll_terminal_events(&self) {
        let mut interval = tokio::time::interval(Duration::from_millis(50)); // 20Hz polling
        loop {
            interval.tick().await;

            let events = {
                let session = self.pty_session.lock();
                let terminal = session.terminal();
                let mut term = terminal.write();
                term.poll_events()
            };

            for event in events {
                if let Some(msg) =
                    par_term_emu_core_rust::streaming::protocol::terminal_event_to_server_message(
                        event,
                    )
                {
                    self.streaming_server.broadcast(msg);
                }
            }
        }
    }

    /// Run the main event loop
    async fn run(&self) -> Result<()> {
        let resize_handle = {
            let state = self.clone_state();
            tokio::spawn(async move {
                state.handle_resize_requests().await;
            })
        };

        let status_handle = {
            let state = self.clone_state();
            tokio::spawn(async move {
                state.handle_pty_status().await;
            })
        };

        let event_handle = {
            let state = self.clone_state();
            tokio::spawn(async move {
                state.poll_terminal_events().await;
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
        event_handle.abort();

        Ok(())
    }

    /// Clone the shared handles into a new ServerState
    fn clone_state(&self) -> Self {
        Self {
            pty_session: Arc::clone(&self.pty_session),
            streaming_server: Arc::clone(&self.streaming_server),
            resize_rx: Arc::clone(&self.resize_rx),
            shell_command: self.shell_command.clone(),
            restart_shell: self.restart_shell,
        }
    }
}

// =============================================================================
// Binary Session Factory (Multi-Session Support)
// =============================================================================

/// Factory for creating PTY-backed terminal sessions in the binary server.
///
/// Each session gets its own PtySession with an independent shell process.
pub struct BinarySessionFactory {
    /// Default shell command (None = auto-detect)
    default_shell: Option<String>,
    /// Scrollback buffer size for new terminals
    scrollback: usize,
    /// Theme to apply to new terminals
    theme: Option<Theme>,
    /// Whether to restart shells on exit
    restart_shell: bool,
    /// Per-session PTY sessions (session_id → PtySession)
    pub pty_sessions: Arc<parking_lot::RwLock<HashMap<String, Arc<Mutex<PtySession>>>>>,
    /// Reference to the streaming server (set after creation)
    streaming_server: Arc<parking_lot::RwLock<Option<Arc<StreamingServer>>>>,
    /// Whether to collect system resource statistics
    enable_system_stats: bool,
    /// System stats collection interval in seconds
    system_stats_interval_secs: u64,
}

impl BinarySessionFactory {
    pub fn new(
        default_shell: Option<String>,
        scrollback: usize,
        theme: Option<Theme>,
        restart_shell: bool,
        enable_system_stats: bool,
        system_stats_interval_secs: u64,
    ) -> Self {
        Self {
            default_shell,
            scrollback,
            theme,
            restart_shell,
            pty_sessions: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            streaming_server: Arc::new(parking_lot::RwLock::new(None)),
            enable_system_stats,
            system_stats_interval_secs,
        }
    }

    /// Set the streaming server reference (called after server creation)
    pub fn set_streaming_server(&self, server: Arc<StreamingServer>) {
        *self.streaming_server.write() = Some(server);
    }
}

impl SessionFactory for BinarySessionFactory {
    fn create_session(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
        shell_command: Option<&str>,
    ) -> std::result::Result<
        SessionFactoryResult,
        par_term_emu_core_rust::streaming::error::StreamingError,
    > {
        use par_term_emu_core_rust::streaming::error::StreamingError;

        info!("Creating session '{}' ({}x{})", session_id, cols, rows);

        // Create a new PtySession
        let pty_session = PtySession::new(cols as usize, rows as usize, self.scrollback);

        // Get the terminal and apply theme
        let terminal = pty_session.terminal();
        if let Some(ref theme) = self.theme {
            let mut term = terminal.write();
            theme.apply(&mut term);
        }

        let pty_session = Arc::new(Mutex::new(pty_session));

        // Spawn the shell
        {
            let mut session = pty_session.lock();
            let shell_cmd = shell_command.or(self.default_shell.as_deref());
            if let Some(cmd) = shell_cmd {
                session.spawn(cmd, &[]).map_err(|e| {
                    StreamingError::ServerError(format!(
                        "Failed to spawn shell '{}' for session '{}': {}",
                        cmd, session_id, e
                    ))
                })?;
            } else {
                session.spawn_shell().map_err(|e| {
                    StreamingError::ServerError(format!(
                        "Failed to spawn default shell for session '{}': {}",
                        session_id, e
                    ))
                })?;
            }
        }

        // Get PTY writer
        let pty_writer = {
            let session = pty_session.lock();
            session.get_writer()
        };

        // Store PTY session
        self.pty_sessions
            .write()
            .insert(session_id.to_string(), Arc::clone(&pty_session));

        Ok(SessionFactoryResult {
            terminal,
            pty_writer,
        })
    }

    fn setup_session(
        &self,
        session_id: &str,
        session: &Arc<StreamSessionState>,
    ) -> std::result::Result<(), par_term_emu_core_rust::streaming::error::StreamingError> {
        let pty_session = {
            let sessions = self.pty_sessions.read();
            sessions.get(session_id).cloned()
        };

        let pty_session = match pty_session {
            Some(s) => s,
            None => return Ok(()), // Already torn down
        };

        // Set up output callback
        let output_sender = session.get_output_sender();
        {
            let mut ps = pty_session.lock();
            ps.set_output_callback(Arc::new(move |data| {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = output_sender.try_send(text);
            }));
        }

        // Spawn resize handler for this session
        let resize_rx = session.get_resize_receiver();
        let pty_clone = Arc::clone(&pty_session);
        let session_id_clone = session_id.to_string();
        let server_ref = self.streaming_server.read().clone();
        tokio::spawn(async move {
            let mut rx = resize_rx.lock().await;
            while let Some((cols, rows)) = rx.recv().await {
                info!(
                    "Resizing session '{}' to {}x{}",
                    session_id_clone, cols, rows
                );
                let mut ps = pty_clone.lock();
                if let Err(e) = ps.resize(cols, rows) {
                    error!(
                        "Failed to resize PTY for session '{}': {}",
                        session_id_clone, e
                    );
                    continue;
                }
                // Broadcast resize to clients in this session
                if let Some(ref server) = server_ref {
                    server.send_to_session(
                        &session_id_clone,
                        par_term_emu_core_rust::streaming::protocol::ServerMessage::resize(
                            cols, rows,
                        ),
                    );
                }
            }
        });

        // Spawn PTY status monitor (restart logic)
        let pty_clone = Arc::clone(&pty_session);
        let session_id_clone = session_id.to_string();
        let restart_shell = self.restart_shell;
        let default_shell = self.default_shell.clone();
        let server_ref = self.streaming_server.read().clone();
        tokio::spawn(async move {
            loop {
                let should_restart = {
                    let ps = pty_clone.lock();
                    if !ps.is_running() {
                        info!(
                            "Session '{}' shell exited (restart={})",
                            session_id_clone, restart_shell
                        );
                        restart_shell
                    } else {
                        false
                    }
                };

                if should_restart {
                    time::sleep(Duration::from_millis(500)).await;
                    info!("Restarting shell for session '{}'", session_id_clone);

                    let restart_result = {
                        let mut ps = pty_clone.lock();
                        if let Some(ref shell) = default_shell {
                            ps.spawn(shell, &[])
                        } else {
                            ps.spawn_shell()
                        }
                    };

                    match restart_result {
                        Ok(_) => {
                            info!("Shell restarted for session '{}'", session_id_clone);
                            // Update PTY writer
                            let pty_writer = {
                                let ps = pty_clone.lock();
                                ps.get_writer()
                            };
                            if let Some(ref server) = server_ref {
                                if let Some(session) = server.get_session(&session_id_clone) {
                                    if let Some(writer) = pty_writer {
                                        session.set_pty_writer(writer);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "Failed to restart shell for session '{}': {} - retrying in 5s",
                                session_id_clone, e
                            );
                            time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                } else if !restart_shell {
                    let ps = pty_clone.lock();
                    if !ps.is_running() {
                        info!(
                            "Session '{}' shell exited and restart disabled",
                            session_id_clone
                        );
                        drop(ps); // Drop mutex guard before close_session to avoid deadlock
                        if let Some(ref server) = server_ref {
                            server.close_session(&session_id_clone, "Shell exited".to_string());
                        }
                        break;
                    }
                }

                time::sleep(Duration::from_millis(500)).await;
            }
        });

        // Spawn terminal event poller for this session
        let pty_clone = Arc::clone(&pty_session);
        let session_id_clone = session_id.to_string();
        let server_ref = self.streaming_server.read().clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50));
            loop {
                interval.tick().await;

                let events = {
                    let ps = pty_clone.lock();
                    let terminal = ps.terminal();
                    let mut term = terminal.write();
                    term.poll_events()
                };

                let server = match server_ref {
                    Some(ref s) => s,
                    None => continue,
                };

                for event in events {
                    if let Some(msg) = par_term_emu_core_rust::streaming::protocol::terminal_event_to_server_message(event) {
                        server.send_to_session(&session_id_clone, msg);
                    }
                }
            }
        });

        // Spawn system stats collection task if enabled
        if self.enable_system_stats {
            let session_id_clone = session_id.to_string();
            let server_ref = self.streaming_server.read().clone();
            let interval_secs = self.system_stats_interval_secs;
            tokio::spawn(async move {
                use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind};

                let refresh_kind = RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything());
                let mut sys = sysinfo::System::new_with_specifics(refresh_kind);
                let mut disks = Disks::new_with_refreshed_list();
                let mut networks = Networks::new_with_refreshed_list();

                // Collect static info once
                let hostname = sysinfo::System::host_name();
                let os_name = sysinfo::System::name();
                let os_version = sysinfo::System::os_version();
                let kernel_version = sysinfo::System::kernel_version();

                // Initial CPU refresh for baseline (first reading is always 0%)
                sys.refresh_specifics(refresh_kind);

                let mut interval = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
                // Skip first tick (happens immediately, CPU would be 0%)
                interval.tick().await;

                loop {
                    interval.tick().await;

                    let server = match server_ref {
                        Some(ref s) => s,
                        None => continue,
                    };

                    // Refresh all metrics
                    sys.refresh_specifics(refresh_kind);
                    disks.refresh(true);
                    networks.refresh(true);

                    // Build CPU stats
                    let cpu = {
                        let global = sys.global_cpu_usage();
                        let cores = sysinfo::System::physical_core_count().unwrap_or(0) as u32;
                        let per_core: Vec<f64> =
                            sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect();
                        let brand = sys.cpus().first().map(|c| c.brand().to_string());
                        let freq = sys.cpus().first().map(|c| c.frequency());
                        par_term_emu_core_rust::streaming::protocol::CpuStats {
                            overall_usage_percent: global as f64,
                            physical_core_count: cores,
                            per_core_usage_percent: per_core,
                            brand,
                            frequency_mhz: freq,
                        }
                    };

                    // Build memory stats
                    let memory = par_term_emu_core_rust::streaming::protocol::MemoryStats {
                        total_bytes: sys.total_memory(),
                        used_bytes: sys.used_memory(),
                        available_bytes: sys.available_memory(),
                        swap_total_bytes: sys.total_swap(),
                        swap_used_bytes: sys.used_swap(),
                    };

                    // Build disk stats
                    let disk_stats: Vec<par_term_emu_core_rust::streaming::protocol::DiskStats> =
                        disks
                            .iter()
                            .map(|d| par_term_emu_core_rust::streaming::protocol::DiskStats {
                                name: d.name().to_string_lossy().to_string(),
                                mount_point: d.mount_point().to_string_lossy().to_string(),
                                total_bytes: d.total_space(),
                                available_bytes: d.available_space(),
                                kind: format!("{:?}", d.kind()),
                                file_system: d.file_system().to_string_lossy().to_string(),
                                is_removable: d.is_removable(),
                            })
                            .collect();

                    // Build network stats
                    let network_stats: Vec<
                        par_term_emu_core_rust::streaming::protocol::NetworkInterfaceStats,
                    > = networks
                        .iter()
                        .map(|(name, data)| {
                            par_term_emu_core_rust::streaming::protocol::NetworkInterfaceStats {
                                name: name.to_string(),
                                received_bytes: data.received(),
                                transmitted_bytes: data.transmitted(),
                                total_received_bytes: data.total_received(),
                                total_transmitted_bytes: data.total_transmitted(),
                                packets_received: data.packets_received(),
                                packets_transmitted: data.packets_transmitted(),
                                errors_received: data.errors_on_received(),
                                errors_transmitted: data.errors_on_transmitted(),
                            }
                        })
                        .collect();

                    // Build load average
                    let load_avg = sysinfo::System::load_average();
                    let load_average = par_term_emu_core_rust::streaming::protocol::LoadAverage {
                        one_minute: load_avg.one,
                        five_minutes: load_avg.five,
                        fifteen_minutes: load_avg.fifteen,
                    };

                    let uptime = sysinfo::System::uptime();
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .ok();

                    let msg =
                        par_term_emu_core_rust::streaming::protocol::ServerMessage::system_stats(
                            Some(cpu),
                            Some(memory),
                            disk_stats,
                            network_stats,
                            Some(load_average),
                            hostname.clone(),
                            os_name.clone(),
                            os_version.clone(),
                            kernel_version.clone(),
                            Some(uptime),
                            timestamp,
                        );

                    server.send_to_session(&session_id_clone, msg);
                }
            });
        }

        Ok(())
    }

    fn is_session_alive(&self, session_id: &str) -> bool {
        self.pty_sessions
            .read()
            .get(session_id)
            .map(|ps| ps.lock().is_running())
            .unwrap_or(false)
    }

    fn teardown_session(&self, session_id: &str) {
        info!("Tearing down session '{}'", session_id);
        if let Some(pty_session) = self.pty_sessions.write().remove(session_id) {
            // Try to gracefully exit the shell
            let ps = pty_session.lock();
            if ps.is_running() {
                if let Some(writer) = ps.get_writer() {
                    let mut w = writer.lock();
                    let _ = w.write_all(b"exit\n");
                    let _ = w.flush();
                }
            }
        }
    }
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

/// True if `host` binds only to the loopback interface (not reachable from the network).
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h == "127.0.0.1" || h == "::1" || h == "localhost" || h.starts_with("127.")
}

/// Resolve HTTP Basic Auth configuration from CLI arguments
///
/// Priority: password_file > password_hash > password
pub fn resolve_http_basic_auth(args: &Args) -> Result<Option<HttpBasicAuthConfig>> {
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
        // Validate file permissions (should not be world/group readable)
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = fs::metadata(file_path) {
                let mode = metadata.mode();
                if mode & 0o077 != 0 {
                    warn!(
                        "Password file {} has insecure permissions {:o}. \
                         Consider restricting to owner-only (chmod 600).",
                        file_path,
                        mode & 0o777
                    );
                }
            }
        }

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
