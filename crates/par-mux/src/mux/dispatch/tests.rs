use super::{dispatch_command, resolve_start_dir, roster_row, Ctx, RosterTokens};
use crate::mux::command::{parse_command, COMMAND_SCHEMA_JSON};
use crate::mux::ids::PaneId;
use crate::mux::pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext};
use crate::mux::tree::MuxTree;
use base64::Engine as _;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
        Arc::new(AtomicUsize::new(0)),
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

/// The smallest-attached-client rule over `-C` reports: the window takes
/// the componentwise minimum of its viewers' reports, and a report that
/// does not move the window answers the issuer alone (the reporter still
/// needs the geometry for its re-seed) and wakes nobody else — without
/// the issuer scoping, two differently-sized attach clients re-reporting
/// in turn broadcast each other into a re-seed flip-flop (the manual-pass
/// htop sort-panel oscillation).
#[test]
fn size_reports_take_the_minimum_and_wake_only_on_a_move() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(DeadPaneFactory))));
    let clients: crate::mux::server::Clients = Arc::new(Mutex::new(Vec::new()));
    let channel = |clients: &crate::mux::server::Clients,
                   id: u64|
     -> (
        std::sync::mpsc::SyncSender<String>,
        std::sync::mpsc::Receiver<String>,
    ) {
        let (tx, rx) = std::sync::mpsc::sync_channel(4096);
        clients.lock().push((
            id,
            tx.clone(),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
            crate::mux::ipc::ConnectionAbort::none(),
        ));
        (tx, rx)
    };
    let (tx1, one) = channel(&clients, 1);
    let (_tx2, two) = channel(&clients, 2);
    let (_tx3, three) = channel(&clients, 3);
    let run_as =
        |client_id: u64, line: &str, issuer: &std::sync::mpsc::SyncSender<String>| -> String {
            let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
            let ctx = Ctx {
                layouts_dir: None,
                output_epoch: &__output_epoch,
                tree: &tree,
                clients: &clients,
                command_number: 7,
                shutdown: None,
                config: None,
                client_id: Some(client_id),
            };
            dispatch_command(parse_command(line).unwrap(), &ctx, None, Some(issuer))
        };
    let extent = || {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        let window = guard.window(window).unwrap();
        (window.id, window.cols, window.rows)
    };

    run_as(1, "new-session -s main", &tx1);

    // The spawn's own pushes (%window-add and friends) are background
    // noise for this test; only %layout-change lines are asserted.
    let next_layout = |rx: &std::sync::mpsc::Receiver<String>| -> String {
        loop {
            let line = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("a %layout-change arrives");
            if line.starts_with("%layout-change") {
                return line;
            }
        }
    };
    let no_layout = |rx: &std::sync::mpsc::Receiver<String>| {
        while let Ok(line) = rx.recv_timeout(std::time::Duration::from_millis(300)) {
            assert!(
                !line.starts_with("%layout-change"),
                "a report that moved nothing must not wake any other client: {line}"
            );
        }
    };

    // The only viewer's report sizes the window: broadcast, so every
    // client re-renders.
    let reply = run_as(1, "refresh-client -C 100x30", &tx1);
    assert!(!reply.contains("%error"), "the report must land: {reply}");
    let (window_id, cols, rows) = extent();
    assert_eq!((cols, rows), (100, 30));
    for rx in [&one, &two, &three] {
        next_layout(rx);
    }

    // A second client wide but short: each dimension takes the smaller
    // report (the asymmetric two-terminal case).
    run_as(2, "refresh-client -C 140x20", &_tx2);
    let (_, cols, rows) = extent();
    assert_eq!((cols, rows), (100, 20), "the componentwise minimum");
    for rx in [&one, &two, &three] {
        next_layout(rx);
    }

    // The first client re-reports its own size: the minimum holds, so
    // nothing moves.
    run_as(1, "refresh-client -C 100x30", &tx1);
    let (_, cols, rows) = extent();
    assert_eq!(
        (cols, rows),
        (100, 20),
        "a report above the minimum must not move the window"
    );
    // The issuer alone is answered — its re-seed still needs the
    // geometry — and nobody else is woken.
    let line = next_layout(&one);
    assert!(
        line.contains(&window_id.to_string()),
        "issuer-scoped geometry for the reported window: {line}"
    );
    no_layout(&two);
    no_layout(&three);

    let status = run_as(1, "pane-info -t %0", &tx1);
    assert!(status.contains("constraint_cols=attach:1"), "{status}");
    assert!(status.contains("constraint_rows=attach:2"), "{status}");
    assert!(status.contains("viewers=attach:1,attach:2"), "{status}");
    let marked = run_as(2, "pane-info -t %0 -M", &tx1);
    assert!(marked.contains("constraint_rows=stream:2"), "{marked}");
    assert!(
        marked.contains("viewers=attach:1 viewer_count="),
        "{marked}"
    );

    // The constraining client detaches (the server teardown's clearing):
    // the window grows to the remaining viewer's report.
    let resized = tree.lock().clear_client_view(2);
    assert_eq!(resized, vec![window_id], "the window grows back");
    let (_, cols, rows) = extent();
    assert_eq!((cols, rows), (100, 30));
    let status = run_as(1, "pane-info -t %0", &tx1);
    assert!(status.contains("constraint_rows=attach:1"), "{status}");
    assert!(!status.contains("stream:2"), "{status}");
}

/// Target-less with no sessions at all: an error naming the absence,
/// not a panic — the same outcome bare `new-window` gives.
#[test]
fn target_less_refresh_client_with_no_sessions_errors_cleanly() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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

/// The seed's `-G` section: a pane with graphics placements appends a
/// `%graphics-begin`/`%graphics-end` manifest after the screen bytes, so
/// a streaming mirror can reconstruct the placements into its own store;
/// the flagless reply stays screen-only.
#[test]
fn refresh_client_seed_with_graphics_carries_a_placement_manifest() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
        terminal
            .write()
            .process(b"\x1b_Ga=T,f=24,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
    }
    let plain = run(&format!("refresh-client -t %{}", pane.0));
    assert!(
        !plain.contains("%graphics-begin"),
        "the flagless seed stays screen-only: {plain}"
    );

    let reply = run(&format!("refresh-client -t %{} -G", pane.0));
    assert!(
        reply.contains("%graphics-begin") && reply.contains("%graphics-end"),
        "the -G seed carries the manifest: {reply}"
    );
    let line = reply
        .lines()
        .find(|l| l.starts_with("%gfx "))
        .expect("one manifest line per placement");
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields.len(), 7, "id col row width height base64: {line}");
    assert_eq!((fields[4], fields[5]), ("2", "2"), "the 2x2 kitty image");
    use base64::Engine as _;
    let pixels = base64::engine::general_purpose::STANDARD
        .decode(fields[6])
        .expect("the manifest carries base64 RGBA");
    assert_eq!(pixels.len(), 16, "2x2 RGB decoded to RGBA");
    let store = tree.lock().pane(pane).unwrap().terminal();
    let terminal = store.read();
    let stored = terminal
        .all_graphics()
        .iter()
        .next()
        .expect("the pane store holds the placement");
    assert_eq!(
        fields[1],
        stored.id.to_string(),
        "the manifest names the store id"
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
        Arc::new(AtomicUsize::new(0)),
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
    let row = roster_row(
        PaneId(4),
        "pi",
        "blocked",
        "hook",
        RosterTokens {
            reason: Some("waiting on telemetry=x hook"),
            telemetry: Some("dGVsZW1ldHJ5"),
            host_telemetry: Some("aG9zdA=="),
            activity_at: 0,
            report_at: 0,
            input_at: 0,
        },
        String::new(),
    );
    let line = row.to_string();
    let mut tokens = line.split_whitespace();
    assert_eq!(tokens.next(), Some("%4"), "pane is positional 0");
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
        roster_row(
            PaneId(4),
            "pi",
            "working",
            "hook",
            RosterTokens {
                reason: None,
                telemetry: None,
                host_telemetry: None,
                activity_at: 0,
                report_at: 0,
                input_at: 0,
            },
            String::new()
        )
        .to_string(),
        "%4 pi working hook",
        "absent fields (no ws among them) add nothing"
    );
    assert_eq!(
        crate::mux::ipc::AgentRow::parse(&line),
        Some(row),
        "the typed row round-trips"
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
    // The applied copy: what this (fake) daemon started with. The
    // stamped config path names a file that does not exist, so the
    // dispatch sees the absent-file shape regardless of the host's real
    // config (QA-196 forbids env mutation to pin one).
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig {
        state_dir: "/tmp/old-state".to_string(),
        config_path: Some("/tmp/par-mux-reload-test-absent.toml".into()),
        ..crate::mux::config::EffectiveConfig::default()
    }));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: Some(&applied),
    };
    // The dispatch re-reads the stamped path (above: absent), so the
    // reply is the all-unchanged shape no matter what the host's real
    // config.toml holds: 8 daemon settings + one line per
    // daemon.agent-append kind.
    let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
    let unchanged = reply.lines().filter(|l| l.contains("unchanged:")).count();
    assert_eq!(
        unchanged,
        8 + crate::mux::config::AGENT_APPEND_KINDS.len(),
        "an absent file leaves every setting unchanged: {reply}"
    );
    // The pure report over a written file: the moved/flipped settings
    // are restart-required, the live settings apply, the unstated one
    // stays unchanged.
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nstate-dir = \"/tmp/new-state\"\npane-endpoints = true\nsplit-start-dir = \"workspace-dir\"\n")
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
        report.contains("applied: daemon.split-start-dir")
            && applied_copy.split_start_dir == "workspace-dir",
        "the live split-start-dir applies: {report}"
    );
    assert!(
        report.contains("unchanged: daemon.socket"),
        "an unstated setting says unchanged: {report}"
    );
}

/// `set-exit-action`: the query form on a workspace that never chose
/// replies the daemon's folded default, the write form stores the
/// workspace's choice and replies its name, the query then replies the
/// workspace's own, and the choice survives into the persisted state the
/// next start restores.
#[test]
fn set_exit_action_queries_replies_and_persists() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    // The applied copy names a folded default of `hold` — what the
    // legacy `remain-on-exit = true` would resolve to.
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig {
        exit_action: crate::mux::config::ExitAction::Hold,
        ..crate::mux::config::EffectiveConfig::default()
    }));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: Some(&applied),
    };
    dispatch_command(parse_command("new-workspace").unwrap(), &ctx, None, None);
    let reply = dispatch_command(
        parse_command("set-exit-action -t +0").unwrap(),
        &ctx,
        None,
        None,
    );
    assert!(
        reply.lines().any(|l| l == "hold"),
        "the query replies the daemon default: {reply}"
    );
    let reply = dispatch_command(
        parse_command("set-exit-action -t +0 shell").unwrap(),
        &ctx,
        None,
        None,
    );
    assert!(
        reply.lines().any(|l| l == "shell"),
        "the write replies the stored value: {reply}"
    );
    let reply = dispatch_command(
        parse_command("set-exit-action -t +0").unwrap(),
        &ctx,
        None,
        None,
    );
    assert!(
        reply.lines().any(|l| l == "shell"),
        "the query now replies the workspace's own: {reply}"
    );
    // The persisted state carries the choice (the save the next start
    // restores).
    let state = tree.lock().collect_persist_capture().capture();
    assert_eq!(
        state.workspaces[0].exit_action.as_deref(),
        Some("shell"),
        "the choice rides the state save"
    );
}

/// Positive control for the stamped-path seam: the dispatch re-reads the
/// file the applied copy names — a planted temp file the canonical
/// lookup could never produce — not the host's real config.
#[test]
fn reload_config_reads_the_applied_copys_stamped_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[daemon]\nstate-dir = \"/tmp/new-state\"\npane-endpoints = true\n",
    )
    .unwrap();
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    // The applied copy names the planted file: the dispatch must diff
    // against IT, not the host's canonical config.
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig {
        state_dir: "/tmp/old-state".to_string(),
        config_path: Some(path),
        ..crate::mux::config::EffectiveConfig::default()
    }));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        client_id: None,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: Some(&applied),
    };
    let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
    assert!(
        reply.contains("restart-required: daemon.state-dir"),
        "the planted file's state-dir is restart-required: {reply}"
    );
    assert!(
        reply.contains("restart-required: daemon.pane-endpoints"),
        "the planted file's flipped bool is restart-required: {reply}"
    );
    // The pure report over the same file agrees with the dispatch.
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nstate-dir = \"/tmp/new-state\"\npane-endpoints = true\n")
            .unwrap();
    let mut applied_copy = applied.lock().clone();
    let report = crate::mux::config::reload_report(&mut applied_copy, Some(&file));
    assert!(
        report.contains("restart-required: daemon.state-dir"),
        "the pure report agrees with the dispatch: {report}"
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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

/// A successful reload-config broadcasts `%config-changed` to every
/// client (the cue attached clients re-apply their `[client]` settings
/// on); a refused one broadcasts nothing.
#[test]
fn reload_config_broadcasts_config_changed_only_on_success() {
    let h = Harness::new();
    let applied = Arc::new(Mutex::new(crate::mux::config::EffectiveConfig::default()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        client_id: None,
        tree: &h.tree,
        clients: &h.clients,
        command_number: 1,
        shutdown: None,
        config: Some(&applied),
    };
    let reply = dispatch_command(parse_command("reload-config").unwrap(), &ctx, None, None);
    assert!(!reply.contains("%error"), "{reply}");
    assert_eq!(h.drain(), vec!["%config-changed\n".to_string()]);

    let reply = h.run("reload-config");
    assert!(reply.contains("%error"), "no applied config: {reply}");
    assert!(h.drain().is_empty(), "a refused reload broadcasts nothing");
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
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
            crate::mux::ipc::ConnectionAbort::none(),
        ));
        Self {
            tree,
            clients,
            sink,
        }
    }

    /// The process-free harness over a caller-supplied factory — the
    /// cwd-recording split-start-dir tests' shape.
    fn with_factory(factory: Box<dyn PaneFactory>) -> Self {
        let tree = Arc::new(Mutex::new(MuxTree::new(factory)));
        let clients: crate::mux::server::Clients = Arc::new(Mutex::new(Vec::new()));
        let (sink_tx, sink) = std::sync::mpsc::sync_channel(4096);
        clients.lock().push((
            u64::MAX,
            sink_tx,
            Arc::new(AtomicUsize::new(0)),
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
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
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
        ("new-session -s main", "duplicate session: main"),
        (
            "new-workspace -n main --session-name main",
            "duplicate session: main",
        ),
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

/// new-workspace (card 01a18e57): the workspace keeps its requested name
/// while its first session auto-disambiguates against the GLOBAL session
/// roster (tmux-style `name`, `name-2`, `name-3`, …), so a session named
/// like the workspace — the bare default is `main` — anywhere in the
/// daemon never fails the create.
#[test]
fn new_workspace_auto_disambiguates_its_first_session_name() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    h.drain();

    // Bare `new-workspace` while a session `main` exists: the workspace is
    // still named `main`, its first session lands as `main-2`.
    let reply = h.run("new-workspace");
    let created = assert_ok(&reply);
    assert_eq!(created.len(), 1, "the reply is the workspace id: {reply:?}");
    {
        let guard = h.tree.lock();
        let ws_id = guard.active_workspace().unwrap();
        assert_eq!(guard.workspace(ws_id).unwrap().name, "main");
        let sid = guard.workspace(ws_id).unwrap().sessions[0];
        assert_eq!(guard.session(sid).unwrap().name, "main-2");
    }
    h.drain();

    // `-n other` while a session `other` exists in another workspace:
    // `other-2`.
    assert_ok(&h.run("new-session -s other -t +0"));
    assert_ok(&h.run("new-workspace -n other"));
    {
        let guard = h.tree.lock();
        let ws_id = guard.active_workspace().unwrap();
        assert_eq!(guard.workspace(ws_id).unwrap().name, "other");
        let sid = guard.workspace(ws_id).unwrap().sessions[0];
        assert_eq!(guard.session(sid).unwrap().name, "other-2");
    }
}

/// `--session-name` decouples the first session's name from the workspace
/// name; a clash on the EXPLICIT name refuses the whole command and leaves
/// no workspace row behind — the no-half-created-workspace invariant.
#[test]
fn new_workspace_session_name_decouples_and_explicit_clash_refuses() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    h.drain();

    // Decoupled naming: workspace `w1`, first session `solo`.
    assert_ok(&h.run("new-workspace -n w1 --session-name solo"));
    {
        let guard = h.tree.lock();
        let ws_id = guard.active_workspace().unwrap();
        assert_eq!(guard.workspace(ws_id).unwrap().name, "w1");
        let sid = guard.workspace(ws_id).unwrap().sessions[0];
        assert_eq!(guard.session(sid).unwrap().name, "solo");
    }
    h.drain();
    let before = h.snapshot();

    // An explicit clash refuses the whole command: no workspace row.
    let reply = h.run("new-workspace -n w2 --session-name main");
    assert_error(&reply, "duplicate session: main");
    assert_eq!(h.snapshot(), before, "the refused create changed the tree");
    assert_eq!(h.drain(), Vec::<String>::new(), "the refusal broadcast");
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

    // Duplicates cannot be minted any more — the unique-names rule — so
    // create a second session and stage the legacy state the ambiguity
    // message still answers.
    assert_ok(&h.run("new-session -s scratch"));
    {
        let mut guard = h.tree.lock();
        // sessions() walks a HashMap, so pick the session by name, not
        // by position.
        let scratch = guard
            .sessions()
            .into_iter()
            .find(|id| guard.session(*id).unwrap().name == "scratch")
            .expect("scratch session exists");
        guard.stage_legacy_duplicate_name(scratch, "main");
    }
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
fn resize_pane_left_grows_through_the_edge_fallback_and_down_adds_rows() {
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

    // tmux border semantics: -L moves p0's LEFT border left = grows — and
    // p0's left border is the window edge, so the T4.C fallback grows
    // through the opposite (right) divider instead.
    assert_ok(&h.run(&format!("resize-pane -t {p0} -L 5")));
    assert_eq!(rect(p0), (w0 + 5, h0), "-L 5 adds five columns");
    assert_ok(&h.run(&format!("resize-pane -t {p0} -D 3")));
    assert_eq!(rect(p0), (w0 + 5, h0 + 3), "-D 3 adds three rows");
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
/// leaves onto the newly displayed window, re-fits that window to its
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

    // A second window of the displayed session: exactly select-window's
    // tab-switch cues — no display-move follow.
    let second = assert_ok(&h.run(&format!("new-window -t {b}")))[0]
        .trim()
        .to_string();
    h.drain();
    assert_ok(&h.run(&format!("switch-client -t {second}")));
    let sent = h.drain();
    assert!(
        sent.iter()
            .any(|l| l.starts_with(&format!("%session-window-changed {b} {second}"))),
        "a same-session tab switch keeps select-window's cue: {sent:?}"
    );
    assert!(
        sent.iter()
            .any(|l| l.starts_with(&format!("%window-pane-changed {second} "))),
        "{sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|l| l.starts_with("%client-session-changed")),
        "the displayed session did not move: {sent:?}"
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
    // tmux's other switch-client flags are not par-mux's: an unknown flag
    // must not silently become the read-only query.
    assert!(parse_command("switch-client -n").is_err());
}

/// `client-snapshot -t <window>` (ENH-043) answers the attach client's
/// whole status read in one reply, in the fixed row order, with the owner
/// session and the owner's active window starred and a spaced pane title
/// carried as the rest of its row.
#[test]
fn client_snapshot_reports_every_status_fact_in_order() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s alpha"));
    assert_ok(&h.run("new-session -s beta"));
    let a = h.session_named("alpha");
    let b = h.session_named("beta");
    assert_ok(&h.run(&format!("new-window -t {b}")));
    let [b0, b1] = <[_; 2]>::try_from(h.windows_of(b)).unwrap();
    let [p1] = <[_; 1]>::try_from(h.panes_of(b1)).unwrap();
    assert_ok(&h.run(&format!("split-window -t {p1}")));
    let [q0, q1] = <[_; 2]>::try_from(h.panes_of(b1)).unwrap();
    assert_ok(&h.run(&format!("select-pane -t {q1} -T 'my build'")));
    assert_ok(&h.run(&format!("select-window -t {b0}")));
    let (ws, ws_name, b0_name, b1_name) = {
        let guard = h.tree.lock();
        let ws = guard.active_workspace().unwrap();
        (
            ws,
            guard.workspace(ws).unwrap().name.clone(),
            guard.window(b0).unwrap().name.clone(),
            guard.window(b1).unwrap().name.clone(),
        )
    };

    let body = assert_ok(&h.run(&format!("client-snapshot -t {b1}")));
    assert_eq!(
        body,
        vec![
            "snapshot 1".to_string(),
            format!("workspace {ws} * {ws_name}"),
            format!("session {a} - {ws} alpha"),
            format!("session {b} * {ws} beta"),
            format!("window {b0} * {b0_name}"),
            format!("window {b1} - {b1_name}"),
            format!("pane {q0}"),
            format!("pane {q1} my build"),
        ]
    );
}

/// A title carrying line breaks still rides one `pane` row.
#[test]
fn client_snapshot_keeps_a_multiline_title_on_one_row() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let p = h.panes_of(w)[0];
    // strip_controls in set_user_title would drop a raw newline, so plant
    // the newline in the OSC title the effective title falls back to.
    {
        let guard = h.tree.lock();
        let pane = guard.pane(p).unwrap();
        pane.terminal()
            .write()
            .process(b"\x1b]2;line one\nline two\x07");
    }
    let body = assert_ok(&h.run(&format!("client-snapshot -t {w}")));
    let panes: Vec<&String> = body.iter().filter(|l| l.starts_with("pane ")).collect();
    assert_eq!(panes.len(), 1, "one pane row: {body:?}");
    assert!(
        !panes[0].contains('\n') && !panes[0].contains('\r'),
        "{body:?}"
    );
}

#[test]
fn client_snapshot_of_an_unknown_window_is_an_error() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    assert_error(&h.run("client-snapshot -t @99"), "no such window: @99");
}

/// The `agent` rows are exactly the `list-agents` rows, `agent `-prefixed.
#[test]
fn client_snapshot_agent_rows_mirror_list_agents() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let p = h.panes_of(w)[0];
    {
        let mut guard = h.tree.lock();
        let pane = guard.pane_mut(p).unwrap();
        pane.set_metadata("agent", "claude");
        pane.set_metadata("agent_state", "blocked");
        pane.set_metadata("agent_message", "needs input");
    }
    let roster = assert_ok(&h.run("list-agents"));
    assert_eq!(roster.len(), 1, "{roster:?}");
    let body = assert_ok(&h.run(&format!("client-snapshot -t {w}")));
    let agents: Vec<String> = body
        .iter()
        .filter_map(|l| l.strip_prefix("agent ").map(str::to_string))
        .collect();
    assert_eq!(agents, roster);
    assert_eq!(body.last(), Some(&format!("agent {}", roster[0])));
}

/// Whether the git binary runs — the repo tests skip (answer nothing)
/// where it does not.
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A real one-commit repo under the OS temp dir, branch `name`, plus a
/// plain (non-repo) sibling. Returns `(repo, plain)`; both removed on
/// drop.
struct TempRepo {
    repo: std::path::PathBuf,
    plain: std::path::PathBuf,
}

impl TempRepo {
    fn new(tag: &str, branch: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "par-mux-snapshot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = root.join("repo");
        let plain = root.join("plain");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "core.fsmonitor=false", "-C"])
                .arg(&repo)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .unwrap()
        };
        git(&["init", "-b", branch]);
        std::fs::write(repo.join("f.txt"), "x\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-m",
            "c",
        ]);
        Self { repo, plain }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(
            self.repo
                .parent()
                .expect("the temp root is the repo's parent"),
        );
    }
}

/// Plant an OSC 7 cwd report in `pane`'s terminal — the chain's
/// pane-cwd source.
fn plant_osc7(h: &Harness, pane: crate::mux::ids::PaneId, dir: &std::path::Path) {
    let mut guard = h.tree.lock();
    let pane = guard.pane_mut(pane).unwrap();
    let seq = format!("\x1b]7;file://localhost{}\x07", dir.display());
    pane.terminal().write().process(seq.as_bytes());
}

/// The snapshot reply for a harness with the given daemon-side
/// `[client] git-status` gate (`None` = the no-config dispatch,
/// enabled). The config copy and the output epoch live in the caller so
/// the `Ctx` borrows stay valid.
fn snapshot_with_gate(
    h: &Harness,
    git_status: Option<bool>,
) -> (
    parking_lot::Mutex<crate::mux::config::EffectiveConfig>,
    crate::mux::server::OutputEpoch,
    Vec<String>,
) {
    let config = parking_lot::Mutex::new(crate::mux::config::EffectiveConfig {
        git_status: git_status.unwrap_or(true),
        ..crate::mux::config::EffectiveConfig::default()
    });
    let epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let window = h.windows_of(h.session_named("main"))[0];
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &epoch,
        tree: &h.tree,
        clients: &h.clients,
        command_number: 7,
        shutdown: None,
        client_id: None,
        config: Some(&config),
    };
    let reply = dispatch_command(
        parse_command(&format!("client-snapshot -t {window}")).unwrap(),
        &ctx,
        None,
        None,
    );
    (config, epoch, assert_ok(&reply))
}

/// Refresh the badge cache for the harness's live workspaces — the
/// refresher tick's work, run inline: the pins drive it the way the
/// daemon's thread does, since a reply path no longer resolves.
fn refresh_badges(h: &Harness) {
    let guard = h.tree.lock();
    crate::mux::git_status::refresh_known_badges(&guard, std::time::Duration::from_secs(9));
}

/// The snapshot's `git` row follows the ACTIVE pane's repo through the
/// real chain: OSC 7-reported cwds in two panes of the displayed
/// session's active window, the active pane's branch badges. A pane
/// parked in a non-repo drops the row. (Skipped where git is missing.)
#[test]
fn client_snapshot_git_badge_follows_the_active_pane() {
    if !git_available() {
        return;
    }
    crate::mux::git_status::reset_cache();
    let repos = TempRepo::new("chain-a", "feat/alpha");
    let repos_b = TempRepo::new("chain-b", "feat/beta");
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let [p0] = <[_; 1]>::try_from(h.panes_of(w)).unwrap();
    assert_ok(&h.run(&format!("split-window -t {p0}")));
    let [p0, p1] = <[_; 2]>::try_from(h.panes_of(w)).unwrap();
    plant_osc7(&h, p0, &repos.repo);
    plant_osc7(&h, p1, &repos_b.repo);
    // split-window activates the NEW pane (p1): its branch badges.
    refresh_badges(&h);
    let (_, _, body) = snapshot_with_gate(&h, None);
    let git_rows: Vec<&String> = body.iter().filter(|l| l.starts_with("git ")).collect();
    assert_eq!(git_rows.len(), 1, "one git row: {body:?}");
    assert_eq!(
        crate::mux::ipc::GitStatusRow::parse(git_rows[0].strip_prefix("git ").unwrap())
            .map(|r| r.branch),
        Some("feat/beta".to_string()),
        "{git_rows:?}"
    );
    // Selecting the other pane re-points the badge.
    assert_ok(&h.run(&format!("select-pane -t {p0}")));
    refresh_badges(&h);
    let (_, _, body) = snapshot_with_gate(&h, None);
    let git_rows: Vec<&String> = body.iter().filter(|l| l.starts_with("git ")).collect();
    assert_eq!(
        crate::mux::ipc::GitStatusRow::parse(git_rows[0].strip_prefix("git ").unwrap())
            .map(|r| r.branch),
        Some("feat/alpha".to_string()),
        "{git_rows:?}"
    );
    // A pane parked outside any repo drops the row entirely.
    plant_osc7(&h, p0, &repos.plain);
    refresh_badges(&h);
    let (_, _, body) = snapshot_with_gate(&h, None);
    assert!(
        !body.iter().any(|l| l.starts_with("git ")),
        "a non-repo cwd serves no row: {body:?}"
    );
}

/// The launch folder stands in when the pane reports no cwd, and the
/// daemon-side `[client] git-status = false` gate drops the row even
/// then.
#[test]
fn client_snapshot_git_badge_falls_back_and_respects_the_key() {
    if !git_available() {
        return;
    }
    crate::mux::git_status::reset_cache();
    let repos = TempRepo::new("fallback", "launch-branch");
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let ws = {
        let guard = h.tree.lock();
        guard.active_workspace().unwrap()
    };
    {
        let mut guard = h.tree.lock();
        guard
            .set_worktree_link(
                ws,
                Some(crate::mux::tree::WorktreeLink {
                    repo: repos.repo.clone(),
                    checkout: repos.repo.clone(),
                    branch: Some("launch-branch".to_string()),
                }),
            )
            .unwrap();
    }
    // No OSC 7, no child: the pane reports no cwd — the workspace's
    // launch folder (the link's checkout) answers.
    refresh_badges(&h);
    let (_, _, body) = snapshot_with_gate(&h, None);
    let git_rows: Vec<&String> = body.iter().filter(|l| l.starts_with("git ")).collect();
    assert_eq!(git_rows.len(), 1, "one git row: {body:?}");
    assert!(git_rows[0].contains(" launch-branch "), "{git_rows:?}");
    // The daemon-side gate: `[client] git-status = false` drops the row.
    let (_, _, body) = snapshot_with_gate(&h, Some(false));
    assert!(
        !body.iter().any(|l| l.starts_with("git ")),
        "a disabled key serves no row: {body:?}"
    );
}

/// The async-badge contract: a snapshot reply never runs git — with an
/// empty cache the `git` row is absent, and it appears only once the
/// refresher tick has resolved the workspace's chain.
#[test]
fn client_snapshot_serves_git_from_the_cache_alone() {
    if !git_available() {
        return;
    }
    crate::mux::git_status::reset_cache();
    let repos = TempRepo::new("cache-only", "cold-cache");
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let ws = {
        let guard = h.tree.lock();
        guard.active_workspace().unwrap()
    };
    {
        let mut guard = h.tree.lock();
        guard
            .set_worktree_link(
                ws,
                Some(crate::mux::tree::WorktreeLink {
                    repo: repos.repo.clone(),
                    checkout: repos.repo.clone(),
                    branch: Some("cold-cache".to_string()),
                }),
            )
            .unwrap();
    }
    // Cold cache: no row — and no git run behind the reply.
    let (_, _, body) = snapshot_with_gate(&h, None);
    assert!(
        !body.iter().any(|l| l.starts_with("git ")),
        "a cold cache serves no row: {body:?}"
    );
    // One refresher tick resolves the chain; the badge serves after.
    refresh_badges(&h);
    let (_, _, body) = snapshot_with_gate(&h, None);
    let git_rows: Vec<&String> = body.iter().filter(|l| l.starts_with("git ")).collect();
    assert_eq!(git_rows.len(), 1, "the tick's badge serves: {body:?}");
    assert!(git_rows[0].contains(" cold-cache "), "{git_rows:?}");
}

/// Records the per-spawn cwd each `create_pane` receives, then builds a
/// dead pane — the split-start-dir tests assert the dispatch→factory
/// handoff exactly, with no process involved (the harness's dead panes
/// have no child, so a kernel-cwd read can never answer).
struct CwdRecordingFactory {
    cwds: Arc<Mutex<Vec<Option<std::path::PathBuf>>>>,
}

impl PaneFactory for CwdRecordingFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        context: &SpawnContext<'_>,
    ) -> Result<MuxPane, MuxError> {
        self.cwds
            .lock()
            .push(context.cwd.map(std::path::Path::to_path_buf));
        DeadPaneFactory.create_pane(id, cols, rows, command, context)
    }

    fn create_dead_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<MuxPane, MuxError> {
        DeadPaneFactory.create_dead_pane(id, cols, rows, command, exit_code)
    }
}

/// `[daemon] split-start-dir = "previous-pane-dir"`: a `split-window`
/// with no `-c` starts the new pane in the target pane's live cwd (the
/// same OSC 7-tracked cwd the git badge resolves), while an explicit
/// `-c` always wins.
#[test]
fn split_window_start_dir_previous_pane_dir_and_explicit_c_wins() {
    let recorded = Arc::new(Mutex::new(Vec::<Option<std::path::PathBuf>>::new()));
    let h = Harness::with_factory(Box::new(CwdRecordingFactory {
        cwds: recorded.clone(),
    }));
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let [p0] = <[_; 1]>::try_from(h.panes_of(w)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    plant_osc7(&h, p0, dir.path());
    let config = Mutex::new(crate::mux::config::EffectiveConfig {
        split_start_dir: "previous-pane-dir".to_string(),
        ..crate::mux::config::EffectiveConfig::default()
    });
    let run = |line: &str| {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
            tree: &h.tree,
            clients: &h.clients,
            command_number: 7,
            shutdown: None,
            config: Some(&config),
            client_id: None,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    };
    assert_ok(&run(&format!("split-window -t {p0}")));
    let recorded_cwd = |recorded: &Arc<Mutex<Vec<Option<std::path::PathBuf>>>>| {
        recorded.lock().last().cloned().flatten()
    };
    assert_eq!(
        recorded_cwd(&recorded).as_deref(),
        Some(dir.path()),
        "the new pane spawns in the target pane's cwd: {:?}",
        recorded.lock()
    );
    // An explicit `-c` beats the setting.
    let override_dir = tempfile::tempdir().unwrap();
    assert_ok(&run(&format!(
        "split-window -t {p0} -c {}",
        override_dir.path().display()
    )));
    assert_eq!(
        recorded_cwd(&recorded).as_deref(),
        Some(override_dir.path()),
        "the explicit -c wins: {:?}",
        recorded.lock()
    );
}

/// `[daemon] split-start-dir = "workspace-dir"`: the new pane starts in
/// the workspace's launch folder (its worktree link's checkout); a
/// workspace without a link, and an unknown spelling, keep the factory
/// default (`None` = home in the real daemon).
#[test]
fn split_window_start_dir_workspace_dir_and_unknown_value_fall_back() {
    let recorded = Arc::new(Mutex::new(Vec::<Option<std::path::PathBuf>>::new()));
    let h = Harness::with_factory(Box::new(CwdRecordingFactory {
        cwds: recorded.clone(),
    }));
    assert_ok(&h.run("new-session -s main"));
    let w = h.windows_of(h.session_named("main"))[0];
    let [p0] = <[_; 1]>::try_from(h.panes_of(w)).unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let ws = {
        let guard = h.tree.lock();
        guard.active_workspace().unwrap()
    };
    {
        let mut guard = h.tree.lock();
        guard
            .set_worktree_link(
                ws,
                Some(crate::mux::tree::WorktreeLink {
                    repo: checkout.path().to_path_buf(),
                    checkout: checkout.path().to_path_buf(),
                    branch: Some("main".to_string()),
                }),
            )
            .unwrap();
    }
    let config = Mutex::new(crate::mux::config::EffectiveConfig {
        split_start_dir: "workspace-dir".to_string(),
        ..crate::mux::config::EffectiveConfig::default()
    });
    let run = |line: &str| {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
            tree: &h.tree,
            clients: &h.clients,
            command_number: 7,
            shutdown: None,
            config: Some(&config),
            client_id: None,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    };
    assert_ok(&run(&format!("split-window -t {p0}")));
    let recorded_cwd = |recorded: &Arc<Mutex<Vec<Option<std::path::PathBuf>>>>| {
        recorded.lock().last().cloned().flatten()
    };
    assert_eq!(
        recorded_cwd(&recorded).as_deref(),
        Some(checkout.path()),
        "the new pane spawns in the launch folder: {:?}",
        recorded.lock()
    );
    // A workspace without a worktree link keeps the factory default.
    let mut guard = h.tree.lock();
    guard.set_worktree_link(ws, None).unwrap();
    drop(guard);
    assert_ok(&run(&format!("split-window -t {p0}")));
    assert_eq!(
        recorded_cwd(&recorded),
        None,
        "no launch folder, no setting dir: {:?}",
        recorded.lock()
    );
    // An unknown spelling keeps the factory default too.
    let config = Mutex::new(crate::mux::config::EffectiveConfig {
        split_start_dir: "bogus-value".to_string(),
        ..crate::mux::config::EffectiveConfig::default()
    });
    let run = |line: &str| {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
            tree: &h.tree,
            clients: &h.clients,
            command_number: 7,
            shutdown: None,
            config: Some(&config),
            client_id: None,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    };
    assert_ok(&run(&format!("split-window -t {p0}")));
    assert_eq!(
        recorded_cwd(&recorded),
        None,
        "an unknown spelling keeps the default: {:?}",
        recorded.lock()
    );
}

/// `has-session`: ok on a live session, `%error` naming an absent one —
/// the exit-code probe agents script against.
#[test]
fn has_session_answers_ok_and_error_by_existence() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s work"));
    let ok = h.run("has-session -t work");
    assert!(!ok.contains("%error"), "a live session probes ok: {ok}");
    let missing = h.run("has-session -t nope");
    assert!(
        missing.contains("%error") && missing.contains("nope"),
        "an absent session errors by name: {missing}"
    );
}

/// `has-session -t $N`: the typed-id form is the same existence probe —
/// an id no session holds must error, not answer ok, or every script
/// branching on the exit code sees a phantom session.
#[test]
fn has_session_errors_for_a_typed_id_no_session_holds() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s work"));
    let live = h.run("has-session -t $0");
    assert!(
        !live.contains("%error"),
        "the live session's typed id probes ok: {live}"
    );
    let missing = h.run("has-session -t $99");
    assert!(
        missing.contains("%error") && missing.contains("$99"),
        "an id no session holds errors: {missing}"
    );
}

/// `set-option`/`show-options`: the two live daemon booleans round-trip;
/// anything else is refused as restart-required; a bad value is refused.
#[test]
fn set_option_and_show_options_round_trip_the_live_booleans() {
    let h = Harness::new();
    let config = Mutex::new(crate::mux::config::EffectiveConfig::default());
    let run = |line: &str| {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
            tree: &h.tree,
            clients: &h.clients,
            command_number: 7,
            shutdown: None,
            config: Some(&config),
            client_id: None,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    };
    let shown = assert_ok(&run("show-options"));
    assert!(
        shown.iter().any(|l| l == "exit-empty=on"),
        "the default prints on: {shown:?}"
    );
    assert_ok(&run("set-option exit-empty off"));
    let shown = assert_ok(&run("show-options"));
    assert!(
        shown.iter().any(|l| l == "exit-empty=off"),
        "the set landed: {shown:?}"
    );
    assert_ok(&run("set-option remain-on-exit true"));
    let shown = assert_ok(&run("show-options"));
    assert!(
        shown.iter().any(|l| l == "remain-on-exit=on"),
        "true parses to on: {shown:?}"
    );
    assert_error(
        &run("set-option prefix C-a"),
        "set-option: \"prefix\" is restart-required — set it in the config file and \
         reload-config; the live keys are exit-empty and remain-on-exit",
    );
    let bad = run("set-option exit-empty maybe");
    assert!(bad.contains("%error"), "a bad value is refused: {bad}");
}

/// R7: `list-panes -t` accepts a session (its active window's panes) and
/// a pane (its owning window's panes), not just a window — the one-rule
/// target grammar.
#[test]
fn list_panes_session_and_pane_targets_list_the_owning_window() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s work"));
    assert_ok(&h.run("split-window -t %0"));
    let by_window = assert_ok(&h.run("list-panes -t @0"));
    assert_eq!(by_window.len(), 2, "{by_window:?}");
    let by_session = assert_ok(&h.run("list-panes -t work"));
    assert_eq!(
        by_session, by_window,
        "a session target lists its active window's panes"
    );
    let by_pane = assert_ok(&h.run("list-panes -t %0"));
    assert_eq!(
        by_pane, by_window,
        "a pane target lists its owning window's panes"
    );
}

/// `-F` on the three list commands: variables expand, unknown names
/// vanish, literal text rides verbatim.
#[test]
fn list_format_flags_expand_variables() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s work"));
    let sessions = assert_ok(&h.run("list-sessions -F '#{session_id}=#{session_name}'"));
    assert_eq!(sessions, vec!["$0=work".to_string()], "{sessions:?}");
    let windows = assert_ok(&h.run("list-windows -t work -F '#{window_id} #{window_active}'"));
    assert_eq!(windows, vec!["@0 1".to_string()], "{windows:?}");
    let panes = assert_ok(&h.run("list-panes -t work -F '#{pane_id} #{unknown}'"));
    assert_eq!(panes, vec!["%0 ".to_string()], "{panes:?}");
}

/// `#{pane_title}` rides the -F expansion on both the bare global form
/// and the targeted form: the effective title (the user title when set,
/// else the OSC title). Control characters are already stripped by the
/// title sources, and the row builder flattens any CR/LF to spaces for
/// the line-framed reply — the shape the streamer roster binds.
#[test]
fn list_panes_format_carries_the_effective_pane_title() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s work"));
    assert_ok(&h.run("split-window -t %0"));
    assert_ok(&h.run("select-pane -t %1 -T 'user title'"));
    let titles = assert_ok(&h.run("list-panes -F '#{pane_id} #{pane_title}'"));
    assert_eq!(titles.len(), 2, "{titles:?}");
    assert_eq!(titles[0], "%0 ");
    assert_eq!(titles[1], "%1 user title");
    // A pane target lists its owning window (R7): both panes, the
    // targeted pane's title among them.
    let targeted = assert_ok(&h.run("list-panes -t %1 -F '#{pane_title}'"));
    assert_eq!(
        targeted,
        vec!["".to_string(), "user title".to_string()],
        "{targeted:?}"
    );
}

fn report_state(
    h: &Harness,
    pane: PaneId,
    agent: &str,
    state: &str,
    seq: u64,
) -> (
    String,
    Option<par_term_emu_core::tmux_control::TmuxNotification>,
) {
    crate::mux::hooks::handle_report(
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"{agent}","state":"{state}","seq":{seq},"source":"t"}}}}"#
        ),
        &h.tree,
    )
}

fn pushed_state(push: Option<par_term_emu_core::tmux_control::TmuxNotification>) -> String {
    match push {
        Some(par_term_emu_core::tmux_control::TmuxNotification::AgentStateChanged {
            state,
            ..
        }) => state,
        other => panic!("expected an agent-state push, got {other:?}"),
    }
}

/// herdr's `done`: an agent that finishes (working -> idle) while no
/// render client shows its window is stored and listed as `done`; the
/// first client view that shows the pane clears it to `idle` daemon-side
/// and broadcasts the flip with the claim's own provenance and workspace.
#[test]
fn done_holds_until_a_client_views_the_pane_then_clears_to_idle() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s shown"));
    assert_ok(&h.run("new-session -s background"));
    let shown_pane = h.panes_of(h.windows_of(h.session_named("shown"))[0])[0];
    let agent_pane = h.panes_of(h.windows_of(h.session_named("background"))[0])[0];
    // A render client views the OTHER session's window.
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {shown_pane} -C 100x30"),
    ));
    pushed_state(report_state(&h, agent_pane, "claude", "working", 1).1);
    assert_eq!(
        pushed_state(report_state(&h, agent_pane, "claude", "idle", 2).1),
        "done",
        "finished out of view is done"
    );
    let roster = assert_ok(&h.run("list-agents"));
    assert!(
        roster[0].starts_with(&format!("{agent_pane} claude done hook")),
        "{roster:?}"
    );

    // A command from a viewer still showing the other window clears
    // nothing.
    h.drain();
    assert_ok(&h.run_as(Some(1), "list-agents"));
    assert!(
        !h.drain()
            .iter()
            .any(|l| l.starts_with("%agent-state-changed")),
        "nothing was viewed"
    );

    // The client switches its view to the agent's window: seen.
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    let pushes = h.drain();
    let flip = pushes
        .iter()
        .find(|l| l.starts_with("%agent-state-changed"))
        .unwrap_or_else(|| panic!("the seen flip broadcasts: {pushes:?}"));
    assert!(
        flip.starts_with(&format!(
            "%agent-state-changed {agent_pane} claude idle source=hook ws="
        )),
        "{flip}"
    );
    let roster = assert_ok(&h.run("list-agents"));
    assert!(
        roster[0].starts_with(&format!("{agent_pane} claude idle hook")),
        "{roster:?}"
    );

    // While viewed, a finish is idle straight away.
    pushed_state(report_state(&h, agent_pane, "claude", "working", 3).1);
    assert_eq!(
        pushed_state(report_state(&h, agent_pane, "claude", "idle", 4).1),
        "idle",
        "finished in view is idle"
    );
}

/// An explicit `done` report is accepted; once a viewer shows the pane a
/// `done` report is stored as `idle` (seen the moment it arrives).
#[test]
fn an_explicit_done_report_is_accepted_and_seen_when_in_view() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s one"));
    let pane = h.panes_of(h.windows_of(h.session_named("one"))[0])[0];
    let (reply, push) = report_state(&h, pane, "pi", "done", 1);
    assert!(reply.contains(r#""result":"ok""#), "{reply}");
    assert_eq!(pushed_state(push), "done");
    let roster = assert_ok(&h.run("list-agents"));
    assert!(
        roster[0].starts_with(&format!("{pane} pi done hook")),
        "{roster:?}"
    );
    assert_ok(&h.run_as(Some(2), &format!("refresh-client -t {pane} -C 80x24")));
    assert_eq!(
        pushed_state(report_state(&h, pane, "pi", "done", 2).1),
        "idle"
    );
}

/// The stored agent state of `pane`'s claim.
fn stored_state(h: &Harness, pane: PaneId) -> Option<String> {
    h.tree
        .lock()
        .pane(pane)
        .and_then(|p| p.metadata().get("agent_state").cloned())
}

/// Drive `pane` through working -> idle as agent `claude`, starting at
/// sequence `seq`; the stored result.
fn finish(h: &Harness, pane: PaneId, seq: u64) -> String {
    pushed_state(report_state(h, pane, "claude", "working", seq).1);
    pushed_state(report_state(h, pane, "claude", "idle", seq + 1).1)
}

/// Whether anything broadcast since the last drain was a seen flip.
fn flipped(h: &Harness) -> bool {
    h.drain()
        .iter()
        .any(|l| l.starts_with("%agent-state-changed"))
}

/// A size report that arrives AFTER the client switched windows (the
/// renderer's report for the window it just left, still in flight) does
/// not move the seen focus back: the left window's finish stays `done`,
/// and a later finish there is `done` too. Landing on the window clears it.
#[test]
fn a_late_size_report_for_a_left_window_does_not_mark_it_seen() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let session = h.session_named("main");
    assert_ok(&h.run(&format!("new-window -t {session}")));
    let windows = h.windows_of(session);
    let (left, landed) = (windows[0], windows[1]);
    let agent_pane = h.panes_of(left)[0];
    let other_pane = h.panes_of(landed)[0];
    assert_ok(&h.run_as(Some(1), &format!("select-window -t {left}")));
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    // The client switches away and reports the window it landed on.
    assert_ok(&h.run_as(Some(1), &format!("select-window -t {landed}")));
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {other_pane} -C 100x30"),
    ));
    assert_eq!(finish(&h, agent_pane, 1), "done");
    h.drain();

    // The late report against the window it left.
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    assert!(!flipped(&h), "a stale report must not mark the finish seen");
    assert_eq!(stored_state(&h, agent_pane).as_deref(), Some("done"));
    assert_eq!(
        finish(&h, agent_pane, 3),
        "done",
        "a finish after the stale report is still unseen"
    );

    // Positive control: landing on the window is a view.
    assert_ok(&h.run_as(Some(1), &format!("select-window -t {left}")));
    assert!(flipped(&h), "landing on the window clears the done");
    assert_eq!(stored_state(&h, agent_pane).as_deref(), Some("idle"));
}

/// The same stale report after a cross-session `switch-client`: the left
/// session's window is still its session's active one, and the report
/// still must not count as a view.
#[test]
fn a_late_size_report_after_a_session_switch_does_not_mark_it_seen() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s left"));
    assert_ok(&h.run("new-session -s landed"));
    let left = h.session_named("left");
    let landed = h.session_named("landed");
    let agent_pane = h.panes_of(h.windows_of(left)[0])[0];
    let other_pane = h.panes_of(h.windows_of(landed)[0])[0];
    assert_ok(&h.run_as(Some(1), &format!("switch-client -t {left}")));
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    assert_ok(&h.run_as(Some(1), &format!("switch-client -t {landed}")));
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {other_pane} -C 100x30"),
    ));
    assert_eq!(finish(&h, agent_pane, 1), "done");
    h.drain();

    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    assert!(!flipped(&h), "a stale report must not mark the finish seen");
    assert_eq!(stored_state(&h, agent_pane).as_deref(), Some("done"));

    assert_ok(&h.run_as(Some(1), &format!("switch-client -t {left}")));
    assert!(flipped(&h), "switching back clears the done");
}

/// Passthrough attach shows ONE pane of a split window (select-pane, then
/// the window-wide size report): the sibling it does not show keeps its
/// `done` until it is the one shown.
#[test]
fn a_single_pane_view_does_not_mark_its_siblings_seen() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let window = h.windows_of(h.session_named("main"))[0];
    let shown = h.panes_of(window)[0];
    assert_ok(&h.run(&format!("split-window -t {shown}")));
    let sibling = *h
        .panes_of(window)
        .iter()
        .find(|p| **p != shown)
        .expect("the split made a sibling");
    assert_eq!(finish(&h, sibling, 1), "done", "nobody views anything yet");
    h.drain();

    // The passthrough wire order: select-pane, then the resync's report.
    assert_ok(&h.run_as(Some(1), &format!("select-pane -t {shown}")));
    assert_ok(&h.run_as(Some(1), &format!("refresh-client -t {shown} -C 100x30")));
    assert!(!flipped(&h), "the shown pane's view is not the sibling's");
    assert_eq!(stored_state(&h, sibling).as_deref(), Some("done"));
    assert_eq!(finish(&h, sibling, 3), "done", "still out of view");

    // Positive control: showing the sibling is a view.
    assert_ok(&h.run_as(Some(1), &format!("select-pane -t {sibling}")));
    assert!(flipped(&h), "showing the sibling clears it");
    assert_eq!(stored_state(&h, sibling).as_deref(), Some("idle"));
}

/// A zoomed window shows only its zoomed pane: the panes hidden behind
/// the zoom are not viewed.
#[test]
fn a_zoomed_window_views_only_the_zoomed_pane() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let window = h.windows_of(h.session_named("main"))[0];
    let first = h.panes_of(window)[0];
    assert_ok(&h.run(&format!("split-window -t {first}")));
    let second = *h.panes_of(window).iter().find(|p| **p != first).unwrap();
    assert_eq!(finish(&h, first, 1), "done");
    assert_ok(&h.run(&format!("resize-pane -Z -t {second}")));
    h.drain();
    assert_ok(&h.run_as(Some(1), &format!("refresh-client -t {second} -C 100x30")));
    assert!(!flipped(&h), "the pane behind the zoom is not viewed");
    assert_eq!(stored_state(&h, first).as_deref(), Some("done"));
}

/// The focus a switch pinned is released when its window dies: the
/// client's next report is where it is.
#[test]
fn a_report_after_the_focused_window_dies_moves_the_focus() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let session = h.session_named("main");
    assert_ok(&h.run(&format!("new-window -t {session}")));
    assert_ok(&h.run(&format!("new-window -t {session}")));
    let windows = h.windows_of(session);
    let (agent_window, doomed) = (windows[0], windows[2]);
    let agent_pane = h.panes_of(agent_window)[0];
    let doomed_pane = h.panes_of(doomed)[0];
    assert_ok(&h.run_as(Some(1), &format!("select-window -t {doomed}")));
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {doomed_pane} -C 100x30"),
    ));
    assert_eq!(finish(&h, agent_pane, 1), "done");
    assert_ok(&h.run(&format!("kill-window -t {doomed}")));
    h.drain();
    assert_ok(&h.run_as(
        Some(1),
        &format!("refresh-client -t {agent_pane} -C 100x30"),
    ));
    assert!(flipped(&h), "the report lands the focus on the live window");
}

/// `wait-output` searches what is already there before parking: the
/// matched line is the reply body, for a substring and a regex alike, and
/// a line scrolled into recent history still matches under the default
/// range (a marker printed and scrolled off within one PTY read).
#[test]
fn wait_output_matches_existing_screen_and_recent_history() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let p = h.panes_of(h.windows_of(h.session_named("main"))[0])[0];
    {
        let term = h.tree.lock().pane(p).unwrap().terminal();
        let mut term = term.write();
        term.process(b"MARKER-early 41\r\n");
        for i in 0..40 {
            term.process(format!("filler-{i}\r\n").as_bytes());
        }
        term.process(b"build READY-7 ok\r\n");
    }
    let visible = assert_ok(&h.run(&format!("capture-pane -t {p}"))).join("\n");
    assert!(
        !visible.contains("MARKER-early"),
        "the marker scrolled off the screen: {visible}"
    );
    assert_eq!(
        assert_ok(&h.run(&format!("wait-output -t {p} -m READY -T 0"))),
        vec!["build READY-7 ok".to_string()]
    );
    assert_eq!(
        assert_ok(&h.run(&format!(r"wait-output -t {p} -r 'READY-\d+ ok$' -T 0"))),
        vec!["build READY-7 ok".to_string()]
    );
    assert_eq!(
        assert_ok(&h.run(&format!("wait-output -t {p} -m MARKER-early -T 0"))),
        vec!["MARKER-early 41".to_string()],
        "recent history is searched by default"
    );
    // `-S 0` narrows the search to the screen, capture-pane's range rule;
    // the harness pane has no process, so the miss ends as an exit.
    assert_error(
        &h.run(&format!("wait-output -t {p} -m MARKER-early -S 0")),
        &format!("wait-output: pane {p} exited without a match"),
    );
}

/// The content `wait-output` matches is exactly `capture-pane`'s: every
/// line a `-S/-E` capture returns is matchable under the same range, and
/// a line outside it is not.
#[test]
fn wait_output_sees_what_capture_pane_sees() {
    let h = Harness::new();
    assert_ok(&h.run("new-session -s main"));
    let p = h.panes_of(h.windows_of(h.session_named("main"))[0])[0];
    {
        let term = h.tree.lock().pane(p).unwrap().terminal();
        let mut term = term.write();
        for i in 0..60 {
            term.process(format!("row-{i:02}\r\n").as_bytes());
        }
    }
    let captured = assert_ok(&h.run(&format!("capture-pane -t {p} -S -5 -E 2")));
    assert!(!captured.is_empty());
    for line in captured
        .iter()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
    {
        assert_eq!(
            assert_ok(&h.run(&format!("wait-output -t {p} -m '{line}' -S -5 -E 2 -T 0"))),
            vec![line.to_string()]
        );
    }
    assert_error(
        &h.run(&format!("wait-output -t {p} -m row-00 -S -5 -E 2")),
        &format!("wait-output: pane {p} exited without a match"),
    );
}

/// Unknown targets and an unmatched pattern on a live pane error inside
/// the reply block — the timeout message is the stable `timed out` text
/// scripts can tell apart from a bad target.
#[test]
fn wait_output_times_out_on_a_live_pane_and_rejects_unknown_targets() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
    assert_error(&run("wait-output -t %99 -m x -T 0"), "no such pane: %99");
    let started = std::time::Instant::now();
    let reply = run(&format!(
        "wait-output -t {pane} -m NEVER-PRINTED-9f3 -T 300"
    ));
    let waited = started.elapsed();
    assert_error(
        &reply,
        &format!("wait-output: timed out after 300ms without a match on {pane}"),
    );
    assert!(
        waited >= std::time::Duration::from_millis(300)
            && waited < std::time::Duration::from_secs(5),
        "the wait honors -T: {waited:?}"
    );
    let _ = tree.lock().pane_mut(pane).unwrap().kill();
}

/// A wait parked on a live pane holds no tree lock: another dispatch on
/// the same tree runs while it waits, and killing the pane ends the wait
/// with the `closed` error instead of leaving it parked.
#[test]
fn wait_output_holds_no_lock_and_ends_when_the_pane_closes() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = |n: u32| Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        tree: &tree,
        clients: &clients,
        command_number: n,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run =
        |n: u32, line: &str| dispatch_command(parse_command(line).unwrap(), &ctx(n), None, None);
    run(1, "new-session -s main");
    run(2, "split-window -t %0");
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            let started = std::time::Instant::now();
            let reply = run(3, "wait-output -t %0 -m NEVER-PRINTED-9f3 -T 20000");
            (reply, started.elapsed())
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        let started = std::time::Instant::now();
        let listed = run(4, "list-panes");
        assert!(listed.contains("%0") && listed.contains("%1"), "{listed}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a parked wait must not hold the tree lock"
        );
        assert!(!run(5, "kill-pane -t %0").contains("%error"));
        let (reply, waited) = waiting.join().expect("the wait thread");
        assert_error(&reply, "wait-output: pane %0 closed without a match");
        assert!(
            waited < std::time::Duration::from_secs(10),
            "the close ended the wait promptly: {waited:?}"
        );
    });
    let _ = tree.lock().pane_mut(PaneId(1)).unwrap().kill();
}

/// A wait whose client left the broadcast registry (hung up, evicted)
/// ends with the disconnect error instead of parking until `-T`.
#[test]
fn wait_output_ends_when_its_client_disconnects() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let run = |client_id: Option<u64>, line: &str| {
        let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
        let ctx = Ctx {
            layouts_dir: None,
            output_epoch: &__output_epoch,
            tree: &tree,
            clients: &clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    };
    run(None, "new-session -s main");
    let started = std::time::Instant::now();
    let reply = run(Some(42), "wait-output -t %0 -m NEVER-PRINTED-9f3 -T 20000");
    assert_error(&reply, "wait-output: client disconnected");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let _ = tree.lock().pane_mut(PaneId(0)).unwrap().kill();
}

/// The bounded history walk a `-S/-E` capture reads composes to exactly
/// what the full `export_scrollback` walk did, for every range shape.
#[test]
fn capture_history_tail_matches_the_full_export() {
    let mut term = par_term_emu_core::terminal::Terminal::with_scrollback(20, 5, 100);
    for i in 0..40 {
        term.process(format!("line-{i:02}\r\n").as_bytes());
    }
    let full = term.export_scrollback(par_term_emu_core::terminal::ExportFormat::Plain, None);
    let screen = term.content();
    for (start, end) in [
        (Some(-3), None),
        (Some(-100), Some(2)),
        (None, Some(-2)),
        (Some(-5), Some(-8)),
        (Some(0), Some(3)),
        (Some(2), Some(-4)),
        (Some(i64::MIN), Some(i64::MIN)),
    ] {
        assert_eq!(
            super::buffers::capture_text(&term, start, end, false),
            crate::mux::server::capture_range(&full, &screen, start, end),
            "-S {start:?} -E {end:?}"
        );
    }
}

/// H-API: `list-commands -J` replies with the embedded schema verbatim
/// (the same bytes `par-mux --print-schema` prints); the bare form keeps
/// the token lines.
#[test]
fn list_commands_j_replies_with_the_embedded_schema() {
    let h = Harness::new();
    let body = assert_ok(&h.run("list-commands -J")).join("\n");
    assert_eq!(body, COMMAND_SCHEMA_JSON.trim_end_matches('\n'));
    let schema: serde_json::Value = serde_json::from_str(&body).expect("JSON body");
    assert_eq!(schema["version"], 1);
    let lines = assert_ok(&h.run("list-commands"));
    assert!(
        lines.iter().any(|l| l == "list-commands schema"),
        "{lines:?}"
    );
}

/// Named-layout commands serve the dispatch layouts dir when one is set —
/// the persisting server's `--state-dir` seam — and never the platform
/// state dir in that case. `None` keeps the platform default, the shape
/// embedders dispatch with.
#[test]
fn layout_verbs_serve_the_dispatch_layouts_dir() {
    let store = tempfile::tempdir().expect("tempdir");
    let dir = store.path().to_path_buf();
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(DeadPaneFactory))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: Some(&dir),
        output_epoch: &__output_epoch,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);

    let reply = run("save-layout unit");
    assert!(!reply.contains("%error"), "save-layout: {reply}");
    let saved = dir.join("unit.json");
    assert!(
        saved.exists(),
        "the layout landed in the dispatch dir: {dir:?}"
    );

    let reply = run("list-layouts");
    assert!(
        reply.contains("unit"),
        "list-layouts sees the store: {reply}"
    );

    let reply = run("kill-layout unit");
    assert!(!reply.contains("%error"), "kill-layout: {reply}");
    assert!(!saved.exists(), "kill-layout removed the file");
}

/// The reply block's body lines (framing stripped).
fn body_lines_of(reply: &str) -> Vec<String> {
    reply
        .lines()
        .filter(|l| {
            !l.starts_with("%begin ") && !l.starts_with("%end ") && !l.starts_with("%error ")
        })
        .map(str::to_string)
        .collect()
}

/// The event ring replays what a disconnected client missed: the
/// lifecycle broadcasts a dispatch produces land in the ring under
/// increasing sequence numbers, `events` returns `head` plus the wire
/// lines, and `--since <head>` is the resume point (nothing new).
#[test]
fn the_event_ring_replays_lifecycle_lines_and_since_resumes() {
    let f = Harness::new();
    assert!(f.run("new-session -s ev").contains("%end"));

    let first = body_lines_of(&f.run("events"));
    assert!(first[0].starts_with("head "), "{first:?}");
    let head: u64 = first[0]
        .split_whitespace()
        .nth(1)
        .expect("head carries its number")
        .parse()
        .expect("head is a number");
    assert!(
        head >= 1,
        "the spawn's window-add is in the ring: {first:?}"
    );
    assert!(
        first[1..].iter().any(|l| l.contains("%window-add @")),
        "each event line is <seq> <at-ms> <wire line>: {first:?}"
    );
    assert!(
        first[1..].iter().all(|l| l.split_whitespace().count() >= 3),
        "seq and timestamp prefix every wire line: {first:?}"
    );

    assert_eq!(
        body_lines_of(&f.run(&format!("events --since {head}"))),
        vec![format!("head {head}")],
        "--since head is the resume point: nothing new"
    );

    // A later lifecycle broadcast records after head, and the full replay
    // carries it.
    assert!(f.run("rename-window -t ev renamed").contains("%end"));
    let replay = body_lines_of(&f.run("events --since 0"));
    assert!(
        replay.iter().any(|l| l.contains("%window-renamed @")),
        "the rename is recorded: {replay:?}"
    );
    assert!(
        replay[0].starts_with("head ") && replay.len() > 2,
        "the whole ring comes back with --since 0: {replay:?}"
    );
}

/// `--since` past the head is legal and empty — a consumer that paused
/// longer than the ring's capacity resyncs to the current head instead of
/// erroring.
#[test]
fn events_since_past_the_head_is_an_empty_ok() {
    let f = Harness::new();
    let reply = f.run("events --since 999999");
    assert!(reply.contains("%end"), "{reply}");
    assert_eq!(
        body_lines_of(&reply),
        vec!["head 0"],
        "an empty ring: {reply}"
    );
}

/// Client keystrokes land on exactly one daemon-side door — `send-keys`
/// (attach forwards its input as `send-keys -H` chunks, the streamer
/// bridge writes the same verb, and a control client spells it directly)
/// — so the client-input stamp lives in that one handler. pane-info and
/// the roster report it as `last_input_at=`, a token separate from the
/// hook path's `last_report_at=` (D35: a supervisor must know a human is
/// driving before it acts on a pane).
#[test]
fn send_keys_stamps_last_input_and_pane_info_and_roster_report_it() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
    // The hook half of the separation: an accepted agent report stamps
    // the report clock, never the client-input one.
    let (hook_reply, _) = crate::mux::hooks::handle_report(
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","seq":1,"state":"working"}}}}"#
        ),
        &tree,
    );
    assert!(
        hook_reply.contains("\"result\":\"ok\""),
        "the hook report must land: {hook_reply}"
    );
    assert_eq!(
        tree.lock().pane(pane).unwrap().last_client_input_at(),
        0,
        "a hook report is not client input"
    );
    let reply = run(&format!("send-keys -t {pane} -H 68"));
    assert!(!reply.contains("%error"), "send-keys failed: {reply}");
    assert!(
        tree.lock().pane(pane).unwrap().last_client_input_at() > 0,
        "a client keystroke stamps the pane"
    );
    let info = run(&format!("pane-info -t {pane}"));
    assert!(
        info.contains(" last_input_at="),
        "pane-info carries the client-input stamp: {info}"
    );
    let roster = run("list-agents");
    let row = roster
        .lines()
        .find(|line| line.starts_with(&pane.to_string()))
        .expect("the claimed pane is on the roster");
    assert!(
        row.contains(" last_report_at="),
        "the hook stamp keeps its own token: {row}"
    );
    assert!(
        row.contains(" last_input_at="),
        "the client stamp rides the roster: {row}"
    );
}

/// A `paste-buffer` is client input like a keystroke: the same
/// `last_input_at` stamp, so a supervisor reads the paste as human
/// presence too.
#[test]
fn paste_buffer_stamps_last_input_too() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
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
    let reply = run("set-buffer human-typed-text");
    assert!(!reply.contains("%error"), "set-buffer failed: {reply}");
    let reply = run(&format!("paste-buffer -t {pane}"));
    assert!(!reply.contains("%error"), "paste-buffer failed: {reply}");
    assert!(
        tree.lock().pane(pane).unwrap().last_client_input_at() > 0,
        "a client paste stamps the pane"
    );
    let info = run(&format!("pane-info -t {pane}"));
    assert!(
        info.contains(" last_input_at="),
        "pane-info carries the paste stamp: {info}"
    );
}

/// pane-info reports the pane's per-pane viewer count and
/// `last_viewed_at`: zero viewers and no stamp for a pane nobody shows;
/// one viewer and a fresh stamp once a client's view/focus lands on the
/// pane's window — the seen rule's zoomed-or-active slot (D35). A live
/// pane also reports its process group's cumulative CPU.
#[test]
fn pane_info_reports_viewer_count_and_last_viewed_at() {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    let __output_epoch: crate::mux::server::OutputEpoch = std::sync::Arc::default();
    let ctx = Ctx {
        layouts_dir: None,
        output_epoch: &__output_epoch,
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let run = |line: &str| dispatch_command(parse_command(line).unwrap(), &ctx, None, None);
    run("new-session -s main");
    let (window, pane) = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        let pane = guard.window(window).unwrap().panes()[0];
        (window, pane)
    };
    let info = run(&format!("pane-info -t {pane}"));
    assert!(
        info.contains(" viewer_count=0"),
        "nobody views the pane yet: {info}"
    );
    assert!(
        !info.contains("last_viewed_at"),
        "never viewed, never stamped: {info}"
    );
    {
        let mut guard = tree.lock();
        guard.set_client_focus(7, window, crate::mux::tree::FocusSource::Selection);
        guard.set_client_view(7, window, 80, 24, crate::mux::layout::PaneChrome::default());
    }
    let info = run(&format!("pane-info -t {pane}"));
    assert!(
        info.contains(" viewer_count=1"),
        "the attached viewer counts: {info}"
    );
    assert!(
        info.contains(" last_viewed_at="),
        "being viewed stamps the pane: {info}"
    );
    assert!(
        info.contains(" cpu_ms="),
        "a live pane reports its process group's CPU: {info}"
    );
    // The viewer leaves: the count drops and the stamp freezes at the
    // last moment of attention instead of vanishing.
    tree.lock().clear_client_view(7);
    let info = run(&format!("pane-info -t {pane}"));
    assert!(
        info.contains(" viewer_count=0"),
        "the detached viewer is gone: {info}"
    );
    assert!(
        info.contains(" last_viewed_at="),
        "the stamp outlives the viewer: {info}"
    );
}
