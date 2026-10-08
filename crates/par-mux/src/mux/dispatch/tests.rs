use super::{dispatch_command, resolve_start_dir, roster_row_entry, Ctx};
use crate::mux::command::parse_command;
use crate::mux::ids::PaneId;
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext};
use crate::mux::tree::MuxTree;
use base64::Engine as _;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `clear-history` wipes scrollback and the visible screen through the
/// pane's own emulator (ED 3), so neither a grid-length check nor the
/// daemon's own capture path finds the prefilled content afterward.
#[test]
fn clear_history_wipes_scrollback_and_screen() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    run("new-session -s main");
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    {
        let terminal = tree.lock().pane(pane).unwrap().terminal();
        let mut term = terminal.write();
        for i in 0..60 {
            term.process(format!("line-{i}\r\n").as_bytes());
        }
    }
    assert!(
        tree.lock()
            .pane(pane)
            .unwrap()
            .terminal()
            .read()
            .grid()
            .scrollback_len()
            > 0,
        "the prefill must produce scrollback"
    );

    let reply = run(&format!("clear-history -t {pane}"));
    assert!(!reply.contains("%error"), "clear failed: {reply}");

    let term = tree.lock().pane(pane).unwrap().terminal();
    assert_eq!(
        term.read().grid().scrollback_len(),
        0,
        "scrollback must be gone"
    );
    let capture = run(&format!("capture-pane -t {pane}"));
    assert!(
        !capture.contains("line-"),
        "the visible screen must be wiped: {capture}"
    );
}

/// The attach handshake's target-less `refresh-client -C WxH -p WxH`:
/// a pure size report. `-p` lands daemon-wide, `-C` resizes the newest
/// session's active window (the same stand-in bare `new-window` uses),
/// `%layout-change` broadcasts, and — the gap this form exists to close
/// — the command never errors and never replays a pane's screen.
#[test]
fn target_less_refresh_client_sizes_the_newest_active_window() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    // Two sessions: newest = second. Its active window is what -C hits.
    run("new-session -s first");
    run("new-session -s second");
    // A registered broadcast sink, so the resize's %layout-change has
    // somewhere to land (the a_failed_respawn test's client shape).
    let (sink_tx, sink_rx) = std::sync::mpsc::sync_channel(4096);
    clients.lock().push((
        u64::MAX,
        sink_tx,
        Arc::new(AtomicBool::new(false)),
        crate::mux::ipc::ConnectionAbort::none(),
    ));
    let (second_window, pane) = {
        let guard = tree.lock();
        let session = *guard
            .sessions()
            .iter()
            .find(|s| guard.session(**s).unwrap().name == "second")
            .expect("the second session exists");
        let window = guard.session(session).unwrap().windows[0];
        let pane = guard.window(window).unwrap().panes()[0];
        (window, pane)
    };
    {
        let guard = tree.lock();
        assert_eq!(
            guard.window(second_window).unwrap().cols,
            80,
            "the default grid starts at 80 columns"
        );
    }

    let reply = run("refresh-client -C 100x40 -p 12x24");
    assert!(
        !reply.contains("%error"),
        "the target-less size report must land, not error: {reply}"
    );
    // The resize broadcasts %layout-change to registered clients — here
    // the one sink the test pushed into the registry. A ConPTY pane's
    // init bytes (the ESC[6n probe, Windows) ride the same queue and
    // can land first when the spawn is slow; skip anything that is not
    // the layout change.
    let broadcast = loop {
        let item = sink_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the resize broadcast a layout change");
        if item.starts_with("%layout-change") {
            break item;
        }
    };
    assert!(
        broadcast.contains(&format!("%layout-change {second_window} ")),
        "the resize broadcasts geometry: {broadcast}"
    );
    let guard = tree.lock();
    let window = guard.window(second_window).unwrap();
    assert_eq!(
        (window.cols, window.rows),
        (100, 40),
        "the newest session's active window takes the reported size"
    );
    // -p is daemon-wide.
    assert_eq!(
        guard
            .pane(pane)
            .unwrap()
            .terminal()
            .read()
            .graphics
            .cell_dimensions,
        (12, 24),
        "the pixel report lands daemon-wide"
    );
}

/// Target-less with no sessions at all: an error naming the absence,
/// not a panic — the same outcome bare `new-window` gives.
#[test]
fn target_less_refresh_client_with_no_sessions_errors_cleanly() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let reply = dispatch_command(
        parse_command("refresh-client -C 100x40").unwrap(),
        &ctx,
        None,
        None,
    );
    assert!(
        reply.contains("%error") && reply.contains("no sessions"),
        "an empty tree rejects the size report: {reply}"
    );
}

/// The -t form keeps its contract: a screen-restore replay only ever
/// happens for it. The target-less size report of the same daemon
/// carries no pane content.
#[test]
fn target_less_refresh_client_never_replays_a_screen() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    run("new-session -s main");
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    {
        let terminal = tree.lock().pane(pane).unwrap().terminal();
        let mut term = terminal.write();
        term.process(b"PANE-MARKER");
    }
    let reply = run("refresh-client -C 120x40");
    assert!(
        !reply.contains("PANE-MARKER"),
        "a size report must never replay a pane screen: {reply}"
    );
}

/// Spawns the first pane as a shell; every later spawn fails.
struct FirstSpawnOnly(AtomicBool);

impl PaneFactory for FirstSpawnOnly {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        _command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        if self.0.swap(true, Ordering::SeqCst) {
            return Err(MuxError::NoSuchPane(id));
        }
        ShellPaneFactory::default().create_pane(id, cols, rows, None, context)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
}

/// ARC-089: `respawn-pane -k` stops forwarding the old process before
/// the off-lock spawn; when that spawn fails the old pane is still
/// the live pane, so its output must forward again.
#[test]
fn a_failed_respawn_keeps_the_live_panes_output_forwarding() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(FirstSpawnOnly(
        AtomicBool::new(false),
    )))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    run("new-session -s main");
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(4096);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        crate::mux::ipc::ConnectionAbort::none(),
    ));

    let reply = run(&format!("respawn-pane -k -t {pane}"));
    assert!(reply.contains("%error"), "the spawn failed: {reply}");

    tree.lock()
        .pane_mut(pane)
        .unwrap()
        .write(b"echo ARC089-STILL-WIRED\r")
        .unwrap();
    let wanted = format!("%output {pane} ");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut seen = String::new();
    while !seen.contains("ARC089-STILL-WIRED") {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            // Payload only, so a marker split across two PTY reads
            // still joins up.
            Ok(line) => {
                if let Some(data) = line.strip_prefix(&wanted) {
                    seen.push_str(data.trim_end_matches('\n'));
                }
            }
            Err(_) => panic!("the live pane's output stopped forwarding: {seen:?}"),
        }
    }
    let _ = tree.lock().pane_mut(pane).unwrap().kill();
}

/// The `-c` degrade rule (card 01a0d9b2fb02): an existing directory
/// passes through untouched; a missing one falls back to home with a
/// note naming both — the command must not fail because a directory
/// this process does not control vanished.
#[test]
fn a_gone_start_directory_degrades_to_home_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let (cwd, note) = resolve_start_dir(dir.path().to_str());
    assert_eq!(cwd.as_deref(), Some(dir.path()));
    assert!(note.is_none(), "an existing dir needs no note");

    let (cwd, note) = resolve_start_dir(Some("/par-mux-test-no-such-dir"));
    let home = dirs::home_dir().unwrap();
    assert_eq!(cwd.as_deref(), Some(home.as_path()));
    let note = note.expect("the fallback is visible");
    assert!(
        note.contains("/par-mux-test-no-such-dir is gone") && note.contains("pane started in"),
        "note names the gone dir and the landing dir: {note}"
    );
}

#[test]
fn roster_row_entry_encodes_the_reason_as_one_unambiguous_token() {
    // ARC-060: a reason ending in `hook` or containing `telemetry=`
    // cannot be confused with the source column or the key=value
    // tail — every token after source is key=value and base64.
    let entry = roster_row_entry(
        "pi",
        "blocked",
        "hook",
        Some("waiting on telemetry=x hook"),
        Some("dGVsZW1ldHJ5"),
        Some("aG9zdA=="),
    );
    let mut tokens = entry.split_whitespace();
    assert_eq!(tokens.next(), Some("pi"), "agent is positional 1");
    assert_eq!(tokens.next(), Some("blocked"), "state is positional 2");
    assert_eq!(tokens.next(), Some("hook"), "source is positional 3");
    let rest: Vec<&str> = tokens.collect();
    assert_eq!(rest.len(), 3, "one token per optional field: {rest:?}");
    assert_eq!(
        rest[0].split_once('=').map(|(k, _)| k),
        Some("reason"),
        "reason precedes the telemetry tokens"
    );
    for token in &rest {
        let (key, value) = token.split_once('=').expect("key=value token");
        assert!(
            !value.contains(char::is_whitespace),
            "{key} token is whitespace-free"
        );
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .is_ok(),
            "{key} token is standard base64"
        );
    }
    let reason_b64 = rest[0].strip_prefix("reason=").unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(reason_b64)
        .unwrap();
    assert_eq!(
        decoded, b"waiting on telemetry=x hook",
        "the reason round-trips with both ambiguity shapes intact"
    );
    assert!(
        !rest.contains(&"hook"),
        "no token can be mistaken for the source"
    );

    // Absent fields add nothing: the row keeps its exact shorter shape.
    assert_eq!(
        roster_row_entry("pi", "working", "hook", None, None, None),
        "pi working hook"
    );
}

/// `reload-config` reports restart-required per changed daemon setting,
/// diffing the re-read file against the applied copy the server
/// published.
#[test]
fn reload_config_reports_restart_required_per_changed_setting() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    // The applied copy: what this (fake) daemon started with.
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig {
        state_dir: "/tmp/old-state".to_string(),
        ..crate::mux::config::EffectiveConfig::default()
    }));
    let ctx = Ctx {
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: Some(&applied),
    };
    // No canonical config file is readable in the test harness (and
    // QA-196 forbids env mutation to pin one): the dispatch with an
    // absent file is the all-unchanged reply shape.
    let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
    let unchanged = reply.lines().filter(|l| l.contains("unchanged:")).count();
    assert_eq!(
        unchanged, 6,
        "an absent file leaves every setting unchanged: {reply}"
    );
    // The pure report over a written file: the moved/flipped settings
    // are restart-required, the unstated one stays unchanged.
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nstate-dir = \"/tmp/new-state\"\npane-endpoints = true\n")
            .unwrap();
    let mut applied_copy = applied.lock().clone();
    let report = crate::mux::config::reload_report(&mut applied_copy, Some(&file));
    assert!(
        report.contains("restart-required: daemon.state-dir"),
        "the moved state-dir is reported: {report}"
    );
    assert!(
        report.contains("restart-required: daemon.pane-endpoints"),
        "the flipped bool is reported: {report}"
    );
    assert!(
        report.contains("unchanged: daemon.socket"),
        "an unstated setting says unchanged: {report}"
    );
}

/// A server with no applied config (the embedder/test shape) answers
/// reload-config with an explicit error, not a fake success.
#[test]
fn reload_config_without_an_applied_copy_errors() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
    assert!(
        reply.contains("%error") && reply.contains("no applied config"),
        "honest refusal: {reply}"
    );
}

/// reload-config takes no arguments (the reject_positionals rule the
/// other no-start-command commands follow).
#[test]
fn reload_config_rejects_positionals() {
    assert!(parse_command("reload-config now").is_err());
    assert!(parse_command("reload-config").is_ok());
}

/// remain-on-exit is the one LIVE daemon setting: a reload-config
/// dispatch whose file states a different value applies it to the
/// server's applied copy on the spot — the next observed death honors
/// the new value with no restart.
#[test]
fn reload_config_applies_remain_on_exit_to_the_applied_copy() {
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig::default()));
    // No canonical config file is readable in the harness (QA-196
    // forbids env mutation to pin one), so the live-apply is driven
    // through the pure report the dispatch calls.
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nremain-on-exit = true").unwrap();
    let report = crate::mux::config::reload_report(&mut applied.lock(), Some(&file));
    assert!(
        report.contains("applied: daemon.remain-on-exit"),
        "the live setting applies: {report}"
    );
    assert!(
        applied.lock().remain_on_exit,
        "the applied copy now holds the dead panes"
    );
}

/// Typed ids resolve without an existence check, so each handler's
/// tree operation must reject an unknown one itself. These four did
/// not: an unknown window swapped with itself replied `%end` and
/// broadcast `%sessions-changed`; an unknown workspace listed as an
/// empty success; an unknown swap-pane source was reported as "in
/// different windows"; an unknown pane joined onto itself was
/// reported as "cannot be moved onto itself".
#[test]
fn unknown_typed_ids_are_rejected_as_missing_not_misreported() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    run("new-session -s main");
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    for (line, expected) in [
        ("swap-window -s @99 -t @99", "no such window: @99"),
        ("list-sessions -t +99", "no such workspace: +99"),
        (&*format!("swap-pane -s %99 -t {pane}"), "no such pane: %99"),
        ("join-pane -s %99 -t %99", "no such pane: %99"),
    ] {
        let reply = run(line);
        let lines: Vec<&str> = reply.lines().collect();
        assert_eq!(lines.len(), 3, "`{line}`: {reply:?}");
        assert_eq!(lines[1], expected, "`{line}`: {reply:?}");
        assert!(lines[2].starts_with("%error "), "`{line}`: {reply:?}");
    }
    let _ = tree.lock().pane_mut(pane).unwrap().kill();
}

/// Creates every pane dead (no process): the tree mechanics run for
/// real, but no shell starts, so no `%output` races the assertions.
pub(crate) struct DeadPaneFactory;

impl PaneFactory for DeadPaneFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        _context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.create_dead_pane(id, cols, rows, command, Some(0))
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
}

/// A dispatch harness over a process-free tree with one registered
/// broadcast sink.
struct Harness {
    tree: Arc<Mutex<MuxTree>>,
    clients: crate::mux::server::Clients,
    sink: std::sync::mpsc::Receiver<String>,
}

impl Harness {
    fn new() -> Self {
        let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(DeadPaneFactory))));
        let clients: crate::mux::server::Clients = Arc::new(Mutex::new(Vec::new()));
        let (sink_tx, sink) = std::sync::mpsc::sync_channel(4096);
        clients.lock().push((
            u64::MAX,
            sink_tx,
            Arc::new(AtomicBool::new(false)),
            crate::mux::ipc::ConnectionAbort::none(),
        ));
        Self {
            tree,
            clients,
            sink,
        }
    }

    fn run_as(&self, client_id: Option<u64>, line: &str) -> String {
        let ctx = Ctx {
            tree: &self.tree,
            clients: &self.clients,
            command_number: 7,
            shutdown: None,
            config: None,
            client_id,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    }

    fn run(&self, line: &str) -> String {
        self.run_as(None, line)
    }

    /// Everything broadcast since the last drain, one entry per send.
    fn drain(&self) -> Vec<String> {
        self.sink.try_iter().collect()
    }

    /// A stable picture of the tree: workspaces, sessions, windows in
    /// order, and each window's panes and extent.
    fn snapshot(&self) -> String {
        let guard = self.tree.lock();
        let mut out = String::new();
        let mut sessions = guard.sessions();
        sessions.sort();
        for ws in guard.workspaces() {
            let ws = guard.workspace(ws).unwrap();
            out.push_str(&format!("ws {} {} {:?};", ws.id, ws.name, ws.sessions));
        }
        for s in sessions {
            let session = guard.session(s).unwrap();
            out.push_str(&format!(
                "s {} {} active={} ",
                session.id, session.name, session.active
            ));
            for w in &session.windows {
                let window = guard.window(*w).unwrap();
                out.push_str(&format!(
                    "[{} {} {}x{} {:?}]",
                    window.id,
                    window.name,
                    window.cols,
                    window.rows,
                    window.panes()
                ));
            }
            out.push(';');
        }
        out
    }

    fn session_named(&self, name: &str) -> crate::mux::ids::SessionId {
        let guard = self.tree.lock();
        guard
            .sessions()
            .into_iter()
            .find(|s| guard.session(*s).unwrap().name == name)
            .expect("session exists")
    }

    fn windows_of(&self, session: crate::mux::ids::SessionId) -> Vec<crate::mux::ids::WindowId> {
        self.tree.lock().session(session).unwrap().windows.clone()
    }

    fn panes_of(&self, window: crate::mux::ids::WindowId) -> Vec<PaneId> {
        self.tree.lock().window(window).unwrap().panes()
    }
}

/// Splits a reply block into (body lines, closed with `%end`).
fn reply_parts(reply: &str) -> (Vec<&str>, bool) {
    let lines: Vec<&str> = reply.lines().collect();
    assert!(
        lines.first().is_some_and(|l| l.starts_with("%begin ")),
        "a reply opens with %begin: {reply:?}"
    );
    let last = lines.last().copied().unwrap_or_default();
    let ok = if last.starts_with("%end ") {
        true
    } else if last.starts_with("%error ") {
        false
    } else {
        panic!("a reply closes with %end or %error: {reply:?}")
    };
    (lines[1..lines.len() - 1].to_vec(), ok)
}

fn assert_error(reply: &str, expected: &str) {
    let (body, ok) = reply_parts(reply);
    assert!(!ok, "expected an %error block: {reply:?}");
    assert_eq!(body, vec![expected], "error text for: {reply:?}");
}

fn assert_ok(reply: &str) -> Vec<String> {
    let (body, ok) = reply_parts(reply);
    assert!(ok, "expected an %end block: {reply:?}");
    body.into_iter().map(str::to_string).collect()
}

/// Every targeted command answers an unknown id or name with the exact
/// `no such …` text inside an `%error` block, and leaves the tree and
/// the broadcast stream untouched — a typo'd target must never land on
/// some other object or announce a change that did not happen.
#[test]
fn unknown_targets_error_exactly_and_change_nothing() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    assert_ok(&h.run("set-buffer clip"));
    h.drain();
    let before = h.snapshot();
    let cases: &[(&str, &str)] = &[
        ("send-keys -t %99 x", "no such pane: %99"),
        ("refresh-client -t %99 -C 100x40", "no such pane: %99"),
        ("kill-pane -t %99", "no such pane: %99"),
        ("split-window -t %99", "no such pane: %99"),
        ("select-pane -t %99", "no such pane: %99"),
        ("select-pane -t %99 -T title", "no such pane: %99"),
        ("pane-title -t %99", "no such pane: %99"),
        ("clear-history -t %99", "no such pane: %99"),
        ("resize-pane -t %99 -L 2", "no such pane: %99"),
        ("swap-pane -s %99 -t %0", "no such pane: %99"),
        ("swap-pane -s %0 -t %99", "no such pane: %99"),
        ("break-pane -s %99", "no such pane: %99"),
        ("join-pane -s %99 -t %0", "no such pane: %99"),
        ("join-pane -s %0 -t %99", "no such pane: %99"),
        ("respawn-pane -k -t %99", "no such pane: %99"),
        ("capture-pane -t %99", "no such pane: %99"),
        ("paste-buffer -t %99", "no such pane: %99"),
        ("select-pane -t ghost", "no such pane: ghost"),
        ("list-panes -t @99", "no such window: @99"),
        ("select-window -t @99", "no such window: @99"),
        ("kill-window -t @99", "no such window: @99"),
        ("kill-window -t nope", "no such window: nope"),
        ("rename-window -t @99 x", "no such window: @99"),
        ("move-window -s @99 -t 0", "no such window: @99"),
        ("move-window -s nope -t 0", "no such window: nope"),
        ("swap-window -s nope -t @0", "no such window: nope"),
        ("swap-window -s @99 -t @99", "no such window: @99"),
        ("swap-pane -s %99 -t %99", "no such pane: %99"),
        ("join-pane -s %99 -t %99", "no such pane: %99"),
        ("new-session -s extra -t +99", "no such workspace: +99"),
        ("new-session -s extra -t nope", "no such workspace: nope"),
        ("new-window -t @99", "no such window: @99"),
        ("list-windows -t $99", "no such session: $99"),
        ("rename-session -t $99 x", "no such session: $99"),
        ("rename-session -t nope x", "no such session: nope"),
        ("kill-session -t $99", "no such session: $99"),
        ("kill-session -t nope", "no such session: nope"),
        ("set-environment -t $99 K V", "no such session: $99"),
        ("list-sessions -t +99", "no such workspace: +99"),
        ("select-workspace -t +99", "no such workspace: +99"),
        ("select-workspace -t nope", "no such workspace: nope"),
        ("rename-workspace -t +99 x", "no such workspace: +99"),
        ("rename-workspace -t nope x", "no such workspace: nope"),
        ("kill-workspace -t +99", "no such workspace: +99"),
        ("kill-workspace -t nope", "no such workspace: nope"),
    ];
    let mut mismatches = Vec::new();
    for (line, expected) in cases {
        let reply = h.run(line);
        let (body, ok) = reply_parts(&reply);
        if ok || body != vec![*expected] {
            mismatches.push(format!("`{line}`: want {expected:?}, got {reply:?}"));
        }
        assert_eq!(h.snapshot(), before, "`{line}` changed the tree");
        assert_eq!(h.drain(), Vec::<String>::new(), "`{line}` broadcast");
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A bad target is reported before an empty buffer: `paste-buffer -t
/// %99` with nothing buffered names the missing pane, not the buffer.
#[test]
fn paste_buffer_reports_the_bad_target_before_the_empty_buffer() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let reply = h.run("paste-buffer -t %99");
    let (body, ok) = reply_parts(&reply);
    assert!(!ok, "{reply:?}");
    assert_eq!(body, vec!["no such pane: %99"], "{reply:?}");
    let reply = h.run("paste-buffer -t %0");
    let (body, ok) = reply_parts(&reply);
    assert!(!ok && body == vec!["no buffers"], "{reply:?}");
}

/// A name shared by two objects is refused with every candidate listed,
/// and neither object is touched — guessing one would kill the wrong
/// window.
#[test]
fn ambiguous_names_list_the_candidates_and_touch_nothing() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let s = h.session_named("main");
    assert_ok(&h.run(&format!("new-window -t {s}")));
    let windows = h.windows_of(s);
    assert_eq!(windows.len(), 2);
    for w in &windows {
        assert_ok(&h.run(&format!("rename-window -t {w} dup")));
    }
    let panes: Vec<PaneId> = windows.iter().map(|w| h.panes_of(*w)[0]).collect();
    for p in &panes {
        assert_ok(&h.run(&format!("select-pane -t {p} -T twin")));
    }
    h.drain();
    let before = h.snapshot();

    assert_error(
        &h.run("kill-window -t dup"),
        &format!(
            "ambiguous window target: dup (matching: {}, {})",
            windows[0], windows[1]
        ),
    );
    assert_error(
        &h.run("kill-pane -t twin"),
        &format!(
            "ambiguous pane target: twin (matching: {}, {})",
            panes[0], panes[1]
        ),
    );
    assert_eq!(
        h.snapshot(),
        before,
        "the ambiguous refusals touched nothing"
    );
    assert!(h.drain().is_empty());

    assert_ok(&h.run("new-session -s main"));
    let before = h.snapshot();
    let mut sessions = h.tree.lock().sessions();
    sessions.sort();
    assert_error(
        &h.run("kill-session -t main"),
        &format!(
            "ambiguous session target: main (matching: {}, {})",
            sessions[0], sessions[1]
        ),
    );
    assert_eq!(
        h.snapshot(),
        before,
        "neither same-named session was killed"
    );
}

/// `move-window -s @N -t <index>` reorders the session's window list,
/// keeps the active window active by identity, and cues clients with
/// `%sessions-changed`; an index past the end clamps to the end.
#[test]
fn move_window_reorders_keeps_active_and_cues_sessions_changed() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let s = h.session_named("main");
    assert_ok(&h.run(&format!("new-window -t {s}")));
    assert_ok(&h.run(&format!("new-window -t {s}")));
    let [w0, w1, w2] = <[_; 3]>::try_from(h.windows_of(s)).unwrap();
    assert_ok(&h.run(&format!("select-window -t {w1}")));
    h.drain();

    assert_ok(&h.run(&format!("move-window -s {w2} -t 0")));
    assert_eq!(h.windows_of(s), vec![w2, w0, w1]);
    let active = {
        let guard = h.tree.lock();
        let session = guard.session(s).unwrap();
        session.windows[session.active]
    };
    assert_eq!(active, w1, "the active window stays active through a move");
    assert_eq!(h.drain(), vec!["%sessions-changed\n".to_string()]);

    assert_ok(&h.run(&format!("move-window -s {w2} -t 99")));
    assert_eq!(
        h.windows_of(s),
        vec![w0, w1, w2],
        "an index past the end clamps"
    );
}

/// Windows of two different sessions cannot swap — that would move
/// ownership, not order — and the refusal names both windows.
#[test]
fn swap_window_across_sessions_is_refused_by_name() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s a"));
    assert_ok(&h.run("new-session -s b"));
    let wa = h.windows_of(h.session_named("a"))[0];
    let wb = h.windows_of(h.session_named("b"))[0];
    h.drain();
    let before = h.snapshot();
    assert_error(
        &h.run(&format!("swap-window -s {wa} -t {wb}")),
        &format!("windows {wa} and {wb} are in different sessions"),
    );
    assert_eq!(h.snapshot(), before);
    assert!(h.drain().is_empty());
}

/// Breaking a window's only pane out closes the source window
/// (`%window-close`) and announces the new one (`%window-add`); the
/// reply names the new window.
#[test]
fn break_pane_of_a_lone_pane_closes_the_source_window() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let s = h.session_named("main");
    let source = h.windows_of(s)[0];
    let pane = h.panes_of(source)[0];
    h.drain();

    let body = assert_ok(&h.run(&format!("break-pane -s {pane} -n solo")));
    let new_window = h.windows_of(s)[0];
    assert_ne!(new_window, source);
    assert_eq!(body, vec![new_window.to_string()]);
    assert_eq!(h.panes_of(new_window), vec![pane]);
    let sent = h.drain();
    assert!(
        sent.iter()
            .any(|l| l.starts_with(&format!("%window-add {new_window}"))),
        "the new window is announced: {sent:?}"
    );
    assert!(
        sent.contains(&format!("%window-close {source}\n")),
        "the emptied source window closes: {sent:?}"
    );
    assert!(h.tree.lock().window(source).is_none());
}

/// join-pane out of a two-pane window keeps the source window alive and
/// re-lays out both windows; joining a session's last pane away removes
/// that session and cues `%sessions-changed`.
#[test]
fn join_pane_relayouts_both_windows_and_reaps_an_emptied_session() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s a"));
    assert_ok(&h.run("new-session -s b"));
    let wa = h.windows_of(h.session_named("a"))[0];
    let wb = h.windows_of(h.session_named("b"))[0];
    let a0 = h.panes_of(wa)[0];
    let b0 = h.panes_of(wb)[0];
    assert_ok(&h.run(&format!("split-window -t {a0}")));
    let a1 = *h.panes_of(wa).iter().find(|p| **p != a0).unwrap();
    h.drain();

    assert_ok(&h.run(&format!("join-pane -s {a1} -t {b0}")));
    assert_eq!(h.panes_of(wa), vec![a0], "the source keeps its other pane");
    assert!(h.panes_of(wb).contains(&a1));
    let sent = h.drain();
    for w in [wa, wb] {
        assert!(
            sent.iter()
                .any(|l| l.starts_with(&format!("%layout-change {w} "))),
            "both windows re-lay out ({w}): {sent:?}"
        );
    }
    assert!(!sent.iter().any(|l| l.starts_with("%window-close")));

    // Now move session a's last pane away: its window and session go.
    assert_ok(&h.run(&format!("join-pane -s {a0} -t {b0}")));
    let sent = h.drain();
    assert!(sent.contains(&format!("%window-close {wa}\n")), "{sent:?}");
    assert!(
        sent.contains(&"%sessions-changed\n".to_string()),
        "{sent:?}"
    );
    let names: Vec<String> = {
        let guard = h.tree.lock();
        guard
            .sessions()
            .into_iter()
            .map(|s| guard.session(s).unwrap().name.clone())
            .collect()
    };
    assert_eq!(names, vec!["b".to_string()]);
    assert_eq!(h.panes_of(wb).len(), 3);
}

/// `resize-pane -L`/`-D` shrink a pane's width and grow its height by
/// the requested cells, bounded by the window.
#[test]
fn resize_pane_left_and_down_move_the_split_by_the_requested_cells() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let p0 = h.panes_of(w)[0];
    let rect = |pane: PaneId| {
        let guard = h.tree.lock();
        let window = guard.window(w).unwrap();
        window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize)
            .into_iter()
            .find(|g| g.pane == pane)
            .map(|g| (g.width, g.height))
            .unwrap()
    };
    // A horizontal split (side by side), then a vertical one in p0.
    assert_ok(&h.run(&format!("split-window -h -t {p0}")));
    assert_ok(&h.run(&format!("split-window -v -t {p0}")));
    let (w0, h0) = rect(p0);
    h.drain();

    assert_ok(&h.run(&format!("resize-pane -t {p0} -L 5")));
    assert_eq!(rect(p0), (w0 - 5, h0), "-L 5 takes five columns");
    assert_ok(&h.run(&format!("resize-pane -t {p0} -D 3")));
    assert_eq!(rect(p0), (w0 - 5, h0 + 3), "-D 3 adds three rows");
    let sent = h.drain();
    assert_eq!(
        sent.iter()
            .filter(|l| l.starts_with(&format!("%layout-change {w} ")))
            .count(),
        2,
        "each resize re-lays out the window once: {sent:?}"
    );
}

/// `capture-pane -e` keeps SGR styling; the plain form strips it.
#[test]
fn capture_pane_escape_flag_controls_styling() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let p = h.panes_of(h.windows_of(h.session_named("main"))[0])[0];
    {
        let term = h.tree.lock().pane(p).unwrap().terminal();
        term.write().process(b"\x1b[31mRED-TEXT\x1b[0m\r\n");
    }
    let plain = assert_ok(&h.run(&format!("capture-pane -t {p}"))).join("\n");
    let styled = assert_ok(&h.run(&format!("capture-pane -e -t {p}"))).join("\n");
    assert!(
        plain.contains("RED-TEXT") && !plain.contains('\x1b'),
        "{plain:?}"
    );
    assert!(
        styled.contains("RED-TEXT") && styled.contains("\x1b[") && styled.contains("31"),
        "the -e capture keeps the red SGR: {styled:?}"
    );
}

/// Without a server's shutdown flag (an embedder dispatch), kill-server
/// has nothing to stop and says so rather than pretending.
#[test]
fn kill_server_without_a_running_server_errors() {
    let h = Harness::new();
    assert_error(
        &h.run("kill-server"),
        "kill-server: no running server to stop",
    );
}

/// select-workspace moves every client view shown in the workspace it
/// leaves onto the newly displayed window, re-fits that window to the
/// smallest reporting client, and tells clients which session they now
/// show.
#[test]
fn select_workspace_moves_reporting_client_views_and_refits() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s first"));
    // A client reports 100x30 against the newest (first) session.
    assert_ok(&h.run_as(Some(1), "refresh-client -C 100x30"));
    let first_window = h.windows_of(h.session_named("first"))[0];
    {
        let guard = h.tree.lock();
        let w = guard.window(first_window).unwrap();
        assert_eq!(
            (w.cols, w.rows),
            (100, 30),
            "the report sizes the shown window"
        );
    }
    let original_ws = h.tree.lock().active_workspace().unwrap();
    assert_ok(&h.run("new-workspace -n other"));
    let other_ws = h.tree.lock().active_workspace().unwrap();
    assert_ne!(other_ws, original_ws);
    h.drain();

    // Switching back: the client's view follows from the left
    // workspace (`other`) only if it was shown there — it was not, so
    // switch to `other` first and then back to observe the follow.
    assert_ok(&h.run(&format!("select-workspace -t {original_ws}")));
    h.drain();
    assert_ok(&h.run(&format!("select-workspace -t {other_ws}")));
    let other_window = {
        let guard = h.tree.lock();
        let s = guard.active_session().unwrap();
        let session = guard.session(s).unwrap();
        session.windows[session.active]
    };
    let sent = h.drain();
    assert!(
        sent.contains(&"%workspaces-changed\n".to_string()),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|l| l.starts_with(&format!("%client-session-changed {other_ws} "))),
        "clients learn the session the display moved to: {sent:?}"
    );
    let guard = h.tree.lock();
    let w = guard.window(other_window).unwrap();
    assert_eq!(
        (w.cols, w.rows),
        (100, 30),
        "the followed view re-fits the new window to the client's report"
    );
}

/// switch-client (card 01a11bd1): the bare form answers the displayed
/// session; a target moves the displayed pointer — the target's workspace
/// becomes active and the session that workspace's active one — and,
/// when the displayed window moved, tells clients through
/// %client-session-changed. Re-switching to the shown session is silent.
#[test]
fn switch_client_queries_and_moves_the_displayed_session() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s a"));
    assert_ok(&h.run("new-workspace -n other"));
    assert_ok(&h.run("new-session -s b -t other"));
    // new-session -t other leaves `b` other's active session; the
    // workspace-spawned first session is the sibling.
    let b = h.session_named("b");
    let a = h.session_named("a");
    assert_eq!(assert_ok(&h.run("switch-client")), vec![b.to_string()]);
    h.drain();

    // Cross-workspace switch to `a`: the workspace and session move.
    assert_ok(&h.run(&format!("switch-client -t {a}")));
    assert_eq!(h.tree.lock().active_session(), Some(a));
    assert_eq!(assert_ok(&h.run("switch-client")), vec![a.to_string()]);
    let sent = h.drain();
    assert!(
        sent.contains(&"%workspaces-changed\n".to_string()),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|l| l.starts_with("%client-session-changed ") && l.contains(&a.to_string())),
        "{sent:?}"
    );

    // A window target selects the window too and lands on its session.
    let b_window = h.windows_of(b)[0];
    assert_ok(&h.run(&format!("switch-client -t {b_window}")));
    assert_eq!(h.tree.lock().active_session(), Some(b));
    h.drain();

    // Already displayed: no broadcast.
    assert_ok(&h.run(&format!("switch-client -t {b}")));
    let sent = h.drain();
    assert!(
        !sent
            .iter()
            .any(|l| l.starts_with("%client-session-changed")),
        "a no-op switch must not echo a follow: {sent:?}"
    );

    // Unknown targets fail without moving anything.
    assert!(h.run("switch-client -t $99").contains("%error"));
    assert_eq!(h.tree.lock().active_session(), Some(b));
}

/// The persistence rule: the query form is read-only, a switch saves.
#[test]
fn switch_client_mutates_only_with_a_target() {
    assert!(!parse_command("switch-client").unwrap().mutates());
    assert!(parse_command("switch-client -t $0").unwrap().mutates());
}
