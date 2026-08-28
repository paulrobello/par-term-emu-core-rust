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
//! par-term-streamer --host 127.0.0.1 --port 8099 --theme iterm2-dark
//! ```
//!
//! ## Environment Variables
//!
//! All CLI options can be set via environment variables with `PAR_TERM_` prefix:
//!
//! ```bash
//! export PAR_TERM_HOST=0.0.0.0
//! export PAR_TERM_PORT=8099
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
//! - URL param: `ws://localhost:8099?api_key=my-secret-key`
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
//!
//! ## Module layout (ARC-005 split)
//!
//! - [`cli`] — clap argument surface, value parsers, TTY size detection
//! - [`frontend_download`] — `--download-frontend` GitHub release fetcher
//! - [`bootstrap`] — run-mode resolution, session factory, event loop, auth
//! - [`theme`] — terminal color themes

// Use jemalloc for better server performance (5-15% throughput improvement)
// Only available on non-Windows platforms
#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use anyhow::{Context, Result};
use clap::Parser;
use par_term_emu_core_rust::{
    macros::{KeyParser, Macro, MacroEvent, MacroPlayback},
    streaming::{SessionFactory, StreamingConfig, StreamingServer, TlsConfig},
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time;
use tracing::{error, info};

mod bootstrap;
mod cli;
mod frontend_download;
mod theme;

use bootstrap::{run_macro_mode, BinarySessionFactory, RunMode};
use cli::Args;
use theme::Theme;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --download-frontend command
    if args.download_frontend {
        println!("par-term-streamer v{}", env!("CARGO_PKG_VERSION"));
        println!("Downloading web frontend...\n");

        frontend_download::download_frontend(&args.frontend_version, &args.web_root).await?;

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
        match cli::get_tty_size() {
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

    // Resolve theme
    let theme = Theme::by_name(&args.theme)
        .ok_or_else(|| anyhow::anyhow!("Unknown theme: {}", args.theme))?;
    info!("Using theme: {}", theme.name);

    // Resolve run mode; macro mode creates its dedicated PTY session here,
    // before any server wiring starts (replaces downstream `expect` panics)
    let run_mode = bootstrap::resolve_run_mode(&args, cols, rows, &theme);

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
    let http_basic_auth = bootstrap::resolve_http_basic_auth(&args)?;

    // Build presets map from CLI args
    let presets: HashMap<String, String> = args.preset.iter().cloned().collect();
    if !presets.is_empty() {
        info!("Registered presets:");
        for (name, cmd) in &presets {
            info!("  {} → {}", name, cmd);
        }
    }

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
        max_sessions: args.max_sessions,
        session_idle_timeout: args.session_idle_timeout,
        presets,
        max_clients_per_session: args.max_clients_per_session,
        input_rate_limit_bytes_per_sec: args.input_rate_limit,
        enable_system_stats: args.enable_system_stats,
        system_stats_interval_secs: args.system_stats_interval,
        api_key: args.api_key.clone(),
        allow_api_key_in_query: args.allow_api_key_in_query,
        allowed_origins: args.allowed_origins.clone(),
    };

    // Create streaming server
    let addr = format!("{}:{}", args.host, args.port);
    info!("Creating streaming server on {}", addr);

    // SEC-002: Warn loudly when binding a non-loopback interface without any
    // authentication configured. A public bind with no auth exposes an
    // interactive shell (the PTY) to anyone who can reach the port.
    if !bootstrap::is_loopback_host(&args.host)
        && args.api_key.is_none()
        && http_basic_auth.is_none()
    {
        eprintln!();
        eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
        eprintln!(
            "!!  SECURITY WARNING: binding {} WITHOUT AUTHENTICATION       !!",
            addr
        );
        eprintln!("!!  The standalone streamer exposes an interactive shell over !!");
        eprintln!("!!  WebSocket. Anyone who can reach this port gets full shell !!");
        eprintln!("!!  access with your privileges.                              !!");
        eprintln!("!!  Fix one of:                                               !!");
        eprintln!("!!    - bind loopback:  --host 127.0.0.1  (the default)       !!");
        eprintln!("!!    - add an API key: --api-key <secret>                    !!");
        eprintln!("!!    - add HTTP Basic: --http-user <user> --http-password ... !!");
        eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
        eprintln!();
    }
    if args.enable_system_stats {
        info!(
            "System stats enabled (interval: {}s)",
            args.system_stats_interval
        );
    }

    let restart_shell = args.macro_file.is_none() && !args.no_restart_shell;

    // Create the session factory (shell mode) and the streaming server
    let (factory, mut streaming_server) = match &run_mode {
        RunMode::Shell => {
            // Multi-session mode with factory
            let factory = Arc::new(BinarySessionFactory::new(
                args.shell.clone(),
                args.scrollback,
                Some(theme.clone()),
                restart_shell,
                args.enable_system_stats,
                args.system_stats_interval,
            ));
            let server = StreamingServer::with_factory(
                addr.clone(),
                config,
                Arc::clone(&factory) as Arc<dyn SessionFactory>,
            );
            (Some(factory), server)
        }
        RunMode::Macro { pty_session, .. } => {
            // Macro mode - single-session backward compatible
            let terminal = {
                let ps = pty_session.lock();
                ps.terminal()
            };
            (
                None,
                StreamingServer::with_config(terminal, addr.clone(), config),
            )
        }
    };

    // Set theme on streaming server
    streaming_server.set_theme(theme.to_protocol());

    let streaming_server = Arc::new(streaming_server);

    // Wire factory's server reference (needed for per-session event broadcasting)
    if let Some(ref factory) = factory {
        factory.set_streaming_server(Arc::clone(&streaming_server));
    }

    // Check if we should play back a macro or run a shell
    if let RunMode::Macro {
        macro_file,
        pty_session: macro_pty,
    } = &run_mode
    {
        // Get output sender for the callback
        let output_sender = streaming_server.get_output_sender();

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
        let pty_session_clone = Arc::clone(macro_pty);
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
            let mut session = macro_pty.lock();
            session.set_output_callback(Arc::new(move |data| {
                let text = String::from_utf8_lossy(data).to_string();
                let _ = output_sender_clone.try_send(text);
            }));
        }

        // No PTY writer needed for macro playback
    } else {
        // Shell mode: create the "default" session via factory
        info!("Creating default session via factory");
        let default_params =
            par_term_emu_core_rust::streaming::ConnectionParams::from_query(&HashMap::new());
        streaming_server
            .resolve_session(&default_params)
            .context("Failed to create default session")?;
        info!("Default session created successfully");
    }

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
        println!("    - Header: X-API-Key: <api-key>");
        if args.allow_api_key_in_query {
            if args.enable_http {
                println!("    - URL: {}://{}/ws?api_key=<api-key>", ws_scheme, addr);
            } else {
                println!("    - URL: {}://{}?api_key=<api-key>", ws_scheme, addr);
            }
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
    println!("  Max sessions: {}", args.max_sessions);
    if args.session_idle_timeout > 0 {
        println!("  Session idle timeout: {}s", args.session_idle_timeout);
    } else {
        println!("  Session idle timeout: disabled");
    }

    if !args.preset.is_empty() {
        println!("\n  Presets:");
        for (name, cmd) in &args.preset {
            println!("    {} → {}", name, cmd);
        }
    }

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
        println!("\n  Mode: INTERACTIVE SHELL (multi-session)");
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
        if args.enable_http {
            println!("  Sessions endpoint: {}://{}/sessions", http_scheme, addr);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("\nPress Ctrl+C to stop the server\n");

    // Start streaming server in background
    let server_handle = {
        let streaming_server = Arc::clone(&streaming_server);
        tokio::spawn(async move {
            if let Err(e) = streaming_server.start().await {
                error!("Streaming server error: {}", e);
            }
        })
    };

    if let RunMode::Macro {
        pty_session: macro_pty,
        ..
    } = &run_mode
    {
        // Macro mode: run the event loop (resize handling, PTY monitoring)
        run_macro_mode(
            Arc::clone(macro_pty),
            Arc::clone(&streaming_server),
            args.shell.clone(),
        )
        .await?;
    } else {
        // Shell mode: factory handles per-session resize, PTY monitoring, and event polling.
        // Send initial command to default session if specified
        if let Some(command) = &args.command {
            let factory_ref = factory.clone();
            let command = command.clone();
            tokio::spawn(async move {
                // Wait 1 second for shell prompt to settle
                time::sleep(Duration::from_secs(1)).await;
                info!("Sending initial command: {}", command);

                if let Some(ref factory) = factory_ref {
                    let sessions = factory.pty_sessions.read();
                    if let Some(pty_session) = sessions.get("default") {
                        let session = pty_session.lock();
                        if let Some(writer) = session.get_writer() {
                            let mut w = writer.lock();
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

        // Wait for Ctrl+C
        signal::ctrl_c()
            .await
            .context("Failed to listen for Ctrl+C")?;
        info!("Received shutdown signal");
    }

    // Cleanup
    info!("Shutting down...");

    // Shutdown streaming server
    streaming_server.shutdown("Server shutting down".to_string());

    // Teardown all factory sessions
    if let Some(ref factory) = factory {
        let session_ids: Vec<String> = factory.pty_sessions.read().keys().cloned().collect();
        for id in session_ids {
            factory.teardown_session(&id);
        }
    }

    // Stop macro mode PTY
    if let RunMode::Macro { pty_session, .. } = &run_mode {
        let session = pty_session.lock();
        if session.is_running() {
            if let Some(writer) = session.get_writer() {
                let mut w = writer.lock();
                let _ = w.write_all(b"exit\n");
                let _ = w.flush();
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
