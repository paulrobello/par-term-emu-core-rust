//! Prefix routing, dead panes, resync, and status-line cursor tracking.

use super::*;

/// Prefix routing: d detaches; a plain byte goes to the pane as hex;
/// prefix prefix sends the prefix byte itself.
#[test]
fn prefix_router_detaches_forwards_and_sends_literal_prefix() {
    let mut session = Session {
        conn: test_dead_conn("router"),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // Plain bytes forward when the pane is live.
    assert!(!session.route_bytes(b"hello"));
    assert!(session.route_bytes(&[0x02, b'd']), "prefix d detaches");
    assert!(
        !session.route_bytes(&[0x02, 0x02]),
        "prefix prefix consumes without detaching"
    );
}

/// A held-dead focused pane takes no bytes: with `exited` set, plain
/// stdin bytes are dropped, so the daemon's NotStartedError `%error`
/// reply to a dead-pane send-keys never has a chance to (a) hit the
/// wire at all or (b) the bytes themselves never echo into the pane's
/// frozen screen. Prefix chords still route.
#[test]
fn dead_pane_takes_no_stdin_bytes_but_prefix_keys_still_route() {
    let mut session = Session {
        conn: test_dead_conn("deadpane-stdin"),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    assert!(
        !session.route_bytes(b"typed while dead"),
        "typing into a dead pane never detaches"
    );
    // Prefix keys still route on a dead pane.
    assert!(
        session.route_bytes(&[0x02, b'd']),
        "prefix d still detaches"
    );
    assert!(
        !session.route_bytes(&[0x02, 0x02]),
        "prefix prefix still consumes"
    );
}

/// The status line carries the respawn hint beside the exit code —
/// the cue that tells a user why their typing does nothing.
#[test]
fn status_line_names_the_respawn_chord_when_the_pane_is_dead() {
    let mut session = Session {
        conn: test_dead_conn("deadpane-status"),
        socket_path: PathBuf::from("/nonexistent"),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: "work".to_string(),
        workspaces: vec![
            ("+0".to_string(), "main".to_string()),
            ("+1".to_string(), "lab".to_string()),
        ],
        active_workspace: Some("+0".to_string()),
        session_workspace: Some("+0".to_string()),
        pane_title: "bash".to_string(),
        agents: 0,
        exited: Some(Some(7)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    let line = session.status_line();
    assert!(
        line.contains("(exited 7 — C-b r respawns)"),
        "the held-dead cue names the respawn chord: {line}"
    );
    session.exited = Some(None);
    assert!(
        session
            .status_line()
            .contains("(exited ? — C-b r respawns)"),
        "the unknown-code form carries the hint too"
    );
    session.exited = None;
    assert!(
        !session.status_line().contains("respawns"),
        "a live pane's line has no exited cue"
    );
}

/// The status draw's cursor placement tracks the shadow emulator: feed
/// it a replay that parks the cursor at a known cell, then a %output
/// The resync seeds the shadow with the daemon's screen-restore byte
/// stream VERBATIM: the stream ends with the pane's real cursor CUP,
/// so the shadow's tracked cursor is the pane's truth. A trailing
/// newline appended after that CUP parks the shadow — and with it
/// every status draw's absolute placement — one row below the prompt,
/// and only the shell's next repaint papers over it (the manual-pass
/// cursor bug).
#[test]
fn resync_feeds_the_shadow_the_restore_stream_verbatim() {
    let (_daemon, path) = FakeDaemon::bind("resync-cursor");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.resync();
    let cursor = session.emulator.terminal().cursor();
    assert_eq!(
        (cursor.col, cursor.row),
        (8, 1),
        "the shadow cursor must sit exactly where the restore stream's final CUP put it \
         (right after `prompt> `), not one row below"
    );
}

/// Every pane-show re-fits the target window BEFORE the replay: the
/// handshake's target-less -C sizes only the newest session's active
/// window, so a window restored at another size never resized and its
/// child ran at the stale height (the manual-pass htop report; render
/// mode's switch path has always reported). The report precedes the
/// replay — the replay must encode the post-resize screen. Unix-only:
/// the assert pins `terminal_grid`'s non-tty (80, 24) fallback (an
/// interactive Windows console session reports its real grid —
/// measured on the Windows VM, 2026-10-04).
#[cfg(unix)]
#[test]
fn resync_size_reports_the_target_window_before_the_replay() {
    let (_daemon, path) = FakeDaemon::bind("resync-size");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.resync();
    // The connect-time handshake sends its own target-less size report
    // (refresh-client -C WxH -p); the resync's targeted report is the
    // first refresh-client naming the pane.
    let report = loop {
        match _daemon
            .received
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok((name, line)) if name == "refresh-client" && line.contains("-t %0") => {
                break line;
            }
            Ok(_) => continue,
            Err(_) => panic!("no targeted size report arrived"),
        }
    };
    assert!(
        report.contains("-C 80x23"),
        "the size report names the pane at the content region: {report}"
    );
    let second = wait_for_line(&_daemon.received, "refresh-client");
    assert!(
        second.contains("-t %0") && !second.contains("-C"),
        "the replay follows without a size: {second}"
    );
}

/// The shown pane is selected daemon-side before the window-wide size
/// report on EVERY resync — an explicit `-t %N` attach and the reconnect
/// included, not only in-session switches — so the daemon's seen rule
/// (the reported window's active pane) views the pane passthrough shows
/// and not a sibling.
#[test]
fn resync_selects_the_shown_pane_before_the_size_report() {
    let (_daemon, path) = FakeDaemon::bind("resync-select");
    let mut session = passthrough_session(
        conn::AttachConn::connect(&path).expect("connect"),
        "@0",
        "%3",
    );
    session.resync();
    let mut sent = Vec::new();
    while let Ok((name, line)) = _daemon
        .received
        .recv_timeout(std::time::Duration::from_secs(5))
    {
        if name == "select-pane" || (name == "refresh-client" && line.contains("-t %3")) {
            sent.push(line);
        }
        if sent.len() == 2 {
            break;
        }
    }
    assert_eq!(sent.len(), 2, "select and a targeted refresh: {sent:?}");
    assert_eq!(
        sent[0], "select-pane -t %3",
        "the select comes first: {sent:?}"
    );
}

/// chunk containing a CUP and a line feed; the bytes `draw_status`
/// emits must end with an absolute CUP at the EMULATOR's tracked cell,
/// not a bare ESC8 restore. A pane scroll landing between the draw and
/// the placement cannot make a fresh absolute position wrong — that is
/// the manual-pass race this kills (ESC7..ESC8 alone restored a
/// position one line off after a scroll).
#[test]
fn status_draw_places_the_cursor_at_the_tracked_cell() {
    let (_daemon, path) = FakeDaemon::bind("tracked-cursor");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };

    // Replay that parks the cursor (CUP row 3 col 8), then a %output
    // chunk that homes, writes "line", and line feeds: the emulator
    // must end at col 4, row 1 (LF moves down, column preserved).
    session.emulator.feed(b"\x1b[3;8H");
    session.emulator.feed(b"\x1b[1;1Hline\n");
    let cursor = session.emulator.terminal().cursor();
    assert_eq!(
        (cursor.col, cursor.row),
        (4, 1),
        "the emulator's tracked cursor must follow the scripted stream"
    );

    // The draw's emission: the placement CUP is the LAST sequence in
    // the draw and names the tracked cell.
    let bytes = session.status_draw_bytes(24, 80);
    let expected = b"\x1b[2;5H"; // row+1 ; col+1, 1-indexed
    let pos = bytes
        .windows(expected.len())
        .rposition(|w| w == expected)
        .unwrap_or_else(|| {
            panic!(
                "the draw must end with an absolute CUP at the tracked cell \
                 (row 4, col 8): {:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
    // It is the draw's LAST sequence: everything after it is nothing.
    assert_eq!(
        pos + expected.len(),
        bytes.len(),
        "the tracked-cell CUP must close the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // The wrap survives (protects the draw against interleaved output
    // within itself) but the restore no longer closes it.
    assert!(
        bytes.windows(2).any(|w| w == b"\x1b7") && bytes.windows(2).any(|w| w == b"\x1b8"),
        "the ESC7/ESC8 wrap stays inside the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // The DECSTBM reserve semantics are unchanged.
    assert!(
        bytes.windows(7).any(|w| w == b"\x1b[1;23r"),
        "the draw still reserves the content region (rows 1..23): {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Card 01a11d62 (passthrough "drops" the rich demo's paragraphs): the
/// pane's PTY is the host grid minus the status row, so a pane scrolling
/// at its bottom keeps its cursor on content row 23 (1-based). The shadow
/// emulator must share that geometry: a full-grid shadow tracked the
/// cursor one row lower — on the status row, OUTSIDE the `1;23` scroll
/// region — and every draw parked the host cursor there, where line feeds
/// never scroll, so all later output overwrote one line and never reached
/// the host's scrollback.
#[test]
fn status_draw_keeps_the_cursor_inside_the_content_region_after_a_scroll() {
    let (_daemon, path) = FakeDaemon::bind("tracked-scroll");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // First draw fits the shadow to the host grid's pane geometry.
    let _ = session.status_draw_bytes(24, 80);
    // The pane scrolls: far more line feeds than it has rows.
    session.emulator.feed(&b"line\r\n".repeat(60));
    let bytes = session.status_draw_bytes(24, 80);
    assert!(
        bytes.ends_with(b"\x1b[23;1H"),
        "a scrolled pane's cursor sits on its last row — content row 23, \
         inside the 1;23 scroll region, never the status row 24: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Dead pane: the tracked cell is the frozen screen's cell — feeding
/// the emulator the frozen stream is what `resync` does, so the same
/// shape holds with `exited` set; the placement does not move to a
/// default position when the pane dies.
#[test]
fn status_draw_tracks_the_frozen_cell_when_the_pane_is_dead() {
    let (_daemon, path) = FakeDaemon::bind("dead-tracked");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    session.emulator.feed(b"\x1b[5;3H");
    let bytes = session.status_draw_bytes(24, 80);
    let expected = b"\x1b[5;3H";
    let pos = bytes
        .windows(expected.len())
        .rposition(|w| w == expected)
        .unwrap_or_else(|| {
            panic!(
                "the dead pane's placement CUP names the frozen cell: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
    assert_eq!(
        pos + expected.len(),
        bytes.len(),
        "the frozen-cell CUP closes the draw: {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

/// Prefix r on a held-dead pane respawns it daemon-side and resumes
/// forwarding; the plain bytes typed afterwards go to the pane.
#[test]
fn respawn_chord_resumes_forwarding_after_revival() {
    let (daemon, path) = FakeDaemon::bind("respawn");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: Some(Some(0)),
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // The fake daemon answers every unknown command with an ok empty
    // block, so respawn-pane succeeds and clears the dead flag.
    assert!(
        !session.route_bytes(&[0x02, b'r']),
        "prefix r never detaches"
    );
    assert!(session.exited.is_none(), "a successful respawn clears it");
    // Forwarding resumed: the next plain byte is on the wire.
    assert!(!session.route_bytes(b"x"));
    // Handshake lines first (version, list-commands, set-client-colors,
    // refresh-client -C), then the chord and respawn's resync
    // (select-pane, then refresh-client -t) — the wire evidence
    // forwarding is live again.
    let lines = recorded(&daemon.received, 7);
    assert_eq!(lines[4].0, "respawn-pane", "the chord respawned: {lines:?}");
    assert_eq!(
        (lines[5].0.as_str(), lines[6].0.as_str()),
        ("select-pane", "refresh-client"),
        "the respawn resync rode the revived connection: {lines:?}"
    );
}

/// A Session for routing tests only: its connection is a closed
/// channel pair, so sends fail silently (routing logic does not care).
fn test_dead_conn(tag: &str) -> conn::AttachConn {
    let path = test_socket(&format!("deadconn-{tag}"));
    let _ = std::fs::remove_file(&path);
    // connect() fails without a listener, which is exactly the "dead"
    // connection a routing test wants — but Session needs a value.
    // Build one over a listener the test immediately drops: every
    // send fails, which is the behavior route_bytes tolerates.
    match conn::AttachConn::connect(&path) {
        Ok(conn) => conn,
        Err(_) => {
            // Fall back: bind-accept-drop. One bound listener that
            // answers nothing, then closes.
            let listener = crate::mux::bind_local_listener(&path).expect("bind");
            std::thread::spawn(move || {
                // The Listener trait supplies `.accept()` on unix only;
                // the Windows named-pipe listener accepts inherently.
                #[cfg(unix)]
                use interprocess::local_socket::traits::Listener as _;
                let _ = listener.accept();
            });
            conn::AttachConn::connect(&path).expect("connect to the accepting listener")
        }
    }
}
