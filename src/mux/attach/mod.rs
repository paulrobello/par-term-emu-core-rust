//! `par-mux attach` — the attach client (feature `attach`).
//!
//! This card is scaffolding: it carries the options, terminal guard, shared
//! connection layer ([`conn::AttachConn`]), and the handshake; the byte
//! pump, prefix router, and status line arrive with the next cards.

pub mod conn;

use crate::mux::resolve_socket_path;
use crate::tmux_control::TmuxNotification;
use std::io::Write;
use std::process::ExitCode;

/// Everything `par-mux attach [-t TARGET] [--prefix KEY] [NAME | --socket PATH]`
/// parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    /// Target daemon: an explicit `--socket` path wins over a positional
    /// `NAME`, which falls back to `$PAR_MUX_SOCKET`/the default socket.
    pub socket: Option<std::path::PathBuf>,
    /// Positional daemon name (the default-path shorthand).
    pub name: Option<String>,
    /// `-t TARGET`: the initial session/window/pane target, tmux-style —
    /// it resolves like a `send-keys -t` target once Phase A's view
    /// resolution lands. Carried only; nothing resolves it yet.
    pub target: Option<String>,
    /// `--prefix KEY`: the detach key chord, e.g. `C-b` (tmux spelling).
    /// Parsed loose here — Phase A's prefix router enforces the grammar.
    pub prefix: Option<String>,
}

impl AttachOptions {
    /// Resolve the daemon socket path (same precedence as the daemon and
    /// `--cmd`: explicit `--socket`, then `NAME`, then `$PAR_MUX_SOCKET`,
    /// then the unnamed default).
    pub fn socket_path(&self) -> std::path::PathBuf {
        resolve_socket_path(
            self.socket.as_deref(),
            self.name.as_deref(),
            std::env::var_os("PAR_MUX_SOCKET").as_deref(),
        )
    }
}

/// Error paths the attach entry point maps to exit codes.
#[derive(Debug)]
pub enum AttachError {
    /// No live daemon owns the socket. Attach never auto-spawns one.
    NoDaemon(std::path::PathBuf),
    /// The socket is served but the handshake failed mid-way.
    Handshake(std::io::Error),
}

/// Attach entry point. Returns the process exit code.
///
/// Standing client-mode contract: a failed connect is reported as
/// "no daemon running on <path>" and exits 1 — attach never starts a
/// daemon, matching `--cmd`/`--stop` (docs/MUX.md Client Mode).
pub fn run(options: &AttachOptions) -> ExitCode {
    match run_inner(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(AttachError::NoDaemon(path)) => {
            eprintln!(
                "par-mux: no daemon running on {} — start one with `par-mux --socket {}`",
                path.display(),
                path.display()
            );
            ExitCode::FAILURE
        }
        Err(AttachError::Handshake(err)) => {
            eprintln!("par-mux: attach failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The attach session: connect, handshake, registration replay, then (this
/// card) report scaffolding status and exit cleanly.
fn run_inner(options: &AttachOptions) -> Result<(), AttachError> {
    let path = options.socket_path();
    let conn = conn::AttachConn::connect(&path).map_err(|_| AttachError::NoDaemon(path))?;

    // Client-contract warnings: stamp mismatch (warn-only) and pre-feature
    // daemon (assume no features — already reflected in daemon_features()).
    if let Some(warning) = &conn.warnings().stamp_mismatch {
        eprintln!("{warning}");
    }

    // Registration replay (held panes' %pane-exited, zoomed windows'
    // %layout-change) queued at registration — drained here; the Phase A
    // passthrough renderer is what consumes them for real.
    let replay = conn.drain_pending_events();
    if !replay.is_empty() {
        let _ = std::io::stderr().write_all(registration_replay_note(&replay).as_bytes());
    }

    // Terminal setup/teardown guard: raw mode while attached, restored on
    // every exit path. The Phase A session keeps the guard alive for its
    // byte pump; this card's session ends right after the report.
    let _guard = TerminalGuard::enter();

    let _ = std::io::stderr().write_all(b"attach: passthrough rendering arrives in Phase A\n");
    drop(_guard);
    Ok(())
}

/// A short stderr line naming the replayed state kinds, for operator
/// visibility until the Phase A renderer replaces it.
fn registration_replay_note(replay: &[TmuxNotification]) -> String {
    let held = replay
        .iter()
        .filter(|n| matches!(n, TmuxNotification::PaneExited { .. }))
        .count();
    let zoomed = replay
        .iter()
        .filter(|n| matches!(n, TmuxNotification::LayoutChange { .. }))
        .count();
    format!(
        "par-mux: registration replay queued for the Phase A renderer \
         ({held} held pane(s), {zoomed} zoomed window(s))\n"
    )
}

/// RAII raw-mode guard: enters raw mode on construction, restores on drop —
/// including on error/panic unwind, so a failed attach never leaves the
/// host terminal in raw mode.
struct TerminalGuard {
    entered: bool,
}

impl TerminalGuard {
    fn enter() -> Self {
        // Raw mode needs a tty; under a test harness (no tty) this is a
        // no-op so the scaffolding stays exercisable headless.
        let entered = crossterm::terminal::enable_raw_mode().is_ok();
        Self { entered }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.entered {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::{emit_block, MuxServer};
    // The Listener trait supplies `.accept()` on unix; the Windows named-pipe
    // listener accepts via its own inherent impl, so this import is unused there.
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::{Arc, Mutex};

    /// A temp-dir socket path unique to this test process.
    fn test_socket(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("par-mux-attach-{}-{tag}.sock", std::process::id()));
        path
    }

    /// A hand-rolled daemon substitute: accepts one connection, records
    /// every control line it receives (in order), and answers each with a
    /// scripted reply chosen by the command name. Replies use the real
    /// wire framing (`emit_block`) so the client parser sees the protocol
    /// exactly as the daemon writes it. The listener is consumed by the
    /// accept thread; the socket path is what the test connects to.
    struct FakeDaemon {
        /// (command name, full command line) per control line, in order.
        received: Receiver<(String, String)>,
    }

    impl FakeDaemon {
        fn bind(tag: &str) -> (Self, PathBuf) {
            let path = test_socket(tag);
            let _ = std::fs::remove_file(&path);
            let listener = crate::mux::bind_local_listener(&path).expect("bind fake daemon");
            let (tx, received) = channel();
            std::thread::spawn(move || {
                if let Ok(stream) = listener.accept() {
                    serve_one(stream, tx);
                }
            });
            (Self { received }, path)
        }
    }

    /// Serve one connection: record lines, answer per the scripted table.
    fn serve_one(stream: crate::mux::LocalStream, tx: std::sync::mpsc::Sender<(String, String)>) {
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            let reply = match name.as_str() {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                _ => String::new(),
            };
            number += 1;
            tx.send((name, trimmed.to_owned())).ok();
            writer
                .write_all(emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    }

    /// Waits for the recorded lines, tolerating the reader thread's delay.
    fn recorded(rx: &Receiver<(String, String)>, n: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for _ in 0..n {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(line) => out.push(line),
                Err(_) => break,
            }
        }
        out
    }

    /// The daemon answers version and list-commands; the handshake must
    /// send version, then list-commands, then set-client-colors, then
    /// refresh-client with -C and -p — in that order.
    #[test]
    fn handshake_sends_version_list_commands_colors_refresh_in_order() {
        let (daemon, path) = FakeDaemon::bind("order");
        {
            let conn = conn::AttachConn::connect(&path).expect("connect");
            assert_eq!(conn.daemon_stamp(), Some("9.9.9+deadbeef"));
            assert!(conn.has_feature("replay-held-state"));
        }
        let lines = recorded(&daemon.received, 4);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client"
            ],
            "handshake order: {lines:?}"
        );
        let colors = &lines[2].1;
        assert!(colors.starts_with("set-client-colors -f "), "got: {colors}");
        assert!(colors.contains(" -b "), "got: {colors}");
        let refresh = &lines[3].1;
        assert!(refresh.contains(" -C "), "got: {refresh}");
        assert!(refresh.contains(" -p "), "got: {refresh}");
    }

    /// A pre-feature daemon rejects list-commands; the handshake proceeds
    /// with no features and still completes steps 3-4.
    #[test]
    fn pre_feature_daemon_leads_to_empty_features_and_completed_handshake() {
        let path = test_socket("prefeature");
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                serve_pre_feature(stream, tx);
            }
        });
        let conn = conn::AttachConn::connect(&path).expect("connect");
        assert!(conn.daemon_features().is_empty());
        assert!(!conn.has_feature("replay-held-state"));
        let lines = recorded(&rx, 4);
        let names: Vec<&str> = lines.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "version",
                "list-commands",
                "set-client-colors",
                "refresh-client"
            ]
        );
    }

    /// A daemon from before `list-commands`: %error on it, everything else
    /// answered.
    fn serve_pre_feature(
        stream: crate::mux::LocalStream,
        tx: std::sync::mpsc::Sender<(String, String)>,
    ) {
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            number += 1;
            tx.send((name.clone(), trimmed.to_owned())).ok();
            let reply = match name.as_str() {
                "version" => "9.9.9+deadbeef".to_string(),
                _ => String::new(),
            };
            let ok = name == "version" || name == "set-client-colors" || name == "refresh-client";
            writer
                .write_all(emit_block(number, &reply, ok).as_bytes())
                .ok();
            writer.flush().ok();
        }
    }

    /// The stamp quiet rule: equal stamps are quiet; a same-version
    /// `+unknown` pair is quiet (unprovable); anything else warns.
    #[test]
    fn stamp_quiet_rule() {
        use super::conn::stamp_mismatch_is_quiet as quiet;
        assert!(quiet("1.2.3+abc", "1.2.3+abc"));
        assert!(quiet("1.2.3+unknown", "1.2.3+whatever"));
        assert!(quiet("1.2.3+abc", "1.2.3+unknown"));
        assert!(!quiet("1.2.3+abc", "1.2.4+abc"));
        assert!(!quiet("1.2.3+abc", "1.2.3+def"));
    }

    /// `list-commands` reply parsing: the `features` line's tokens, unknown
    /// lines ignored.
    #[test]
    fn feature_token_parsing() {
        use super::conn::parse_feature_tokens;
        let body = vec![
            "capture-pane escape".to_string(),
            "features replay-held-state".to_string(),
            "something else".to_string(),
        ];
        assert_eq!(
            parse_feature_tokens(&body),
            vec!["replay-held-state".to_string()]
        );
        assert!(parse_feature_tokens(&[]).is_empty());
    }

    /// attach with no daemon on the path reports the no-daemon error
    /// instead of spawning one.
    #[test]
    fn no_daemon_path_reports_no_daemon_without_spawning() {
        let path = test_socket("nodaemon");
        let _ = std::fs::remove_file(&path);
        let options = AttachOptions {
            socket: Some(path.clone()),
            name: None,
            target: None,
            prefix: None,
        };
        assert_eq!(run(&options), ExitCode::FAILURE);
        // And nothing appeared on the path: no auto-spawn.
        assert!(!path.exists(), "attach must not spawn a daemon");
    }

    /// A real server answers the whole handshake with ok blocks and the
    /// session exits cleanly with the scaffolding notice.
    #[test]
    fn end_to_end_with_real_server() {
        let path = test_socket("real");
        let _ = std::fs::remove_file(&path);
        let server = MuxServer::bind(&path).expect("bind server");
        let handle = Arc::new(Mutex::new(Some(server.shutdown_handle())));
        let server_thread = std::thread::spawn(move || server.run());
        // Give the accept loop a moment; `run()` returns only at shutdown.
        std::thread::sleep(std::time::Duration::from_millis(100));

        let options = AttachOptions {
            socket: Some(path.clone()),
            name: None,
            target: None,
            prefix: None,
        };
        assert_eq!(run(&options), ExitCode::SUCCESS);

        handle
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        server_thread.join().expect("server thread");
    }

    /// AttachOptions resolves --socket over NAME and falls back to the env
    /// var in the documented precedence.
    #[test]
    fn socket_path_precedence() {
        let explicit = AttachOptions {
            socket: Some(PathBuf::from("/tmp/explicit.sock")),
            name: Some("work".to_string()),
            target: None,
            prefix: None,
        };
        assert_eq!(explicit.socket_path(), PathBuf::from("/tmp/explicit.sock"));
        let named = AttachOptions {
            socket: None,
            name: Some("work".to_string()),
            target: None,
            prefix: None,
        };
        assert_eq!(named.socket_path(), crate::mux::default_socket_path("work"));
        let fallback = AttachOptions {
            socket: None,
            name: None,
            target: None,
            prefix: None,
        };
        // No env var in the test harness -> the unnamed default.
        if std::env::var_os("PAR_MUX_SOCKET").is_none() {
            assert_eq!(
                fallback.socket_path(),
                crate::mux::default_socket_path("default")
            );
        }
    }
}
