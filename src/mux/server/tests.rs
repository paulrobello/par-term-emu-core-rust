use super::*;
use crate::mux::ids::WorkspaceId;

/// A fresh `TempDir` for a test's socket or state file: the directory
/// name carries OS-provided randomness, so no other test run can name the
/// same path (a `process::id()`-derived name repeats once the OS recycles
/// the pid, and an orphaned listener on it answers as a live server), and
/// its `Drop` removes everything inside even when the test panics.
fn temp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("par-mux-server-")
        .tempdir()
        .expect("create test temp dir")
}

#[test]
fn reply_is_error_detects_the_error_terminator() {
    assert!(reply_is_error(
        "%begin 1 2 1\ncan't find pane\n%error 1 2 1\n"
    ));
    assert!(!reply_is_error("%begin 1 2 1\n%end 1 2 1\n"));
    // Notifications pushed between commands are not replies.
    assert!(!reply_is_error("%output %1 61"));
}

/// ENH-037: the registration replay covers a held pane and a zoomed
/// window — pane exits first, then layout changes, both id-sorted — and
/// a tree with neither replays nothing.
#[test]
fn held_state_replay_reports_held_panes_and_zoomed_windows() {
    use crate::mux::dispatch::{dispatch_command, Ctx};
    use crate::mux::pane::ShellPaneFactory;

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
    let run = |line: &str| {
        dispatch_command(
            crate::mux::command::parse_command(line).unwrap(),
            &ctx,
            None,
            None,
        )
    };
    run("new-session -s main");
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    run(&format!("resize-pane -t %{} -Z", pane.0));
    tree.lock().pane_mut(pane).unwrap().mark_dead();

    let lines = {
        let guard = tree.lock();
        held_state_replay_lines(&guard)
    };
    assert_eq!(lines.len(), 2, "lines: {lines:?}");
    assert!(
        lines[0].starts_with(&format!("%pane-exited %{}", pane.0)),
        "lines: {lines:?}"
    );
    assert!(lines[1].starts_with("%layout-change @"), "lines: {lines:?}");
    assert!(
        lines[1].trim_end().ends_with("Z"),
        "zoom must carry raw flags Z: {:?}",
        lines[1]
    );
}

/// ENH-037: a fresh tree holds nothing, so registration replays nothing.
#[test]
fn a_fresh_tree_replays_nothing() {
    use crate::mux::pane::ShellPaneFactory;

    let tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
    assert!(held_state_replay_lines(&tree).is_empty());
}

/// Card 01a0ef3b: a client whose first command arrives after a pane's
/// death sees the death, not a live pane — `Registration::ensure`
/// queues the held-state `%pane-exited` on the client's own channel,
/// exactly once across repeat registrations, and joins the broadcast
/// set so later deaths arrive as ordinary pushes.
#[test]
fn registration_after_a_death_delivers_the_held_exit_once() {
    use crate::mux::dispatch::{dispatch_command, Ctx};
    use crate::mux::ipc::ConnectionAbort;
    use crate::mux::pane::ShellPaneFactory;

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
    dispatch_command(
        crate::mux::command::parse_command("new-session -s main").unwrap(),
        &ctx,
        None,
        None,
    );
    let pane = {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        let window = guard.session(session).unwrap().windows[0];
        guard.window(window).unwrap().panes()[0]
    };
    tree.lock().pane_mut(pane).unwrap().mark_dead();

    let (tx, rx) = std::sync::mpsc::sync_channel(16);
    let evicted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut registration = Registration {
        done: false,
        announced: false,
        abort: Some(ConnectionAbort::none()),
    };
    registration.ensure(&tree, &clients, 7, &tx, &evicted);
    registration.ensure(&tree, &clients, 7, &tx, &evicted);

    let mut lines = Vec::new();
    while let Ok(line) = rx.try_recv() {
        lines.push(line);
    }
    assert_eq!(lines.len(), 1, "one held exit, delivered once: {lines:?}");
    assert!(
        lines[0].starts_with(&format!("%pane-exited %{}", pane.0)),
        "lines: {lines:?}"
    );
    assert!(
        clients.lock().iter().any(|(id, _, _, _)| *id == 7),
        "the registering client joined the broadcast set"
    );
    let _ = tree.lock().pane_mut(pane).unwrap().kill();
}

#[test]
fn summarize_line_keeps_short_lines_and_cut_points_whole() {
    assert_eq!(
        summarize_line("capture-pane -t %1 -S -"),
        "capture-pane -t %1 -S -"
    );
    let long = "capture-pane -t %1 -S - ".to_string() + &"-e ".repeat(200);
    let summary = summarize_line(&long);
    assert!(summary.starts_with("capture-pane -t %1 -S - -e "));
    assert!(summary.contains(&format!("{} bytes total", long.len())));
    // The cut must not split a multi-byte char.
    let multibyte = "é".repeat(200);
    let cut = summarize_line(&multibyte);
    assert!(cut.contains(&format!("{} bytes total", multibyte.len())));
    assert!(cut.ends_with("bytes total)"));
    assert!(cut.is_char_boundary(cut.find("...").expect("ellipsis marker")));
}

/// SEC-133: a stuck thread is detached at the bound, not awaited.
#[test]
fn join_bounded_detaches_a_stuck_thread() {
    let stuck = std::thread::spawn(|| std::thread::sleep(Duration::from_secs(5)));
    let started = std::time::Instant::now();
    assert!(!join_bounded(stuck, Duration::from_millis(200)));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "gave up at the bound: {:?}",
        started.elapsed()
    );

    let quick = std::thread::spawn(|| {});
    assert!(join_bounded(quick, Duration::from_secs(1)));
}

/// SEC-132: typed input and clipboard content never reach the debug log.
#[test]
fn summarize_line_redacts_payload_commands() {
    let literal = summarize_line("send-keys -t %1 -l hunter2");
    assert!(!literal.contains("hunter2"), "payload leaked: {literal}");
    assert!(
        literal.contains("send-keys") && literal.contains("-t %1") && literal.contains("bytes"),
        "name, target and size survive: {literal}"
    );

    let hex = summarize_line("send-keys -t %1 -H 68 75 6e");
    assert!(!hex.contains("68 75"), "hex payload leaked: {hex}");

    let buffer = summarize_line("set-buffer topsecret");
    assert!(!buffer.contains("topsecret"), "buffer leaked: {buffer}");
    assert!(buffer.contains("set-buffer"), "name survives: {buffer}");

    // The target comes only from a leading -t: a -t inside the payload
    // must not smuggle a payload token into the log.
    let smuggled = summarize_line("send-keys -l 'x -t secret' -t %1");
    assert!(!smuggled.contains("secret"), "payload leaked: {smuggled}");

    // Unparseable shapes are redacted too (they are logged at level 1).
    let no_target = summarize_line("  send-keys hunter2");
    assert!(
        !no_target.contains("hunter2"),
        "payload leaked: {no_target}"
    );

    assert_eq!(summarize_line("list-panes"), "list-panes");
}

/// A reader that serves a fixed script of chunks and errors, one per
/// `fill_buf`, so the bounded fill can be driven through poll wakes.
struct ScriptedReader {
    script: std::collections::VecDeque<std::io::Result<Vec<u8>>>,
    current: Vec<u8>,
    pos: usize,
}

impl ScriptedReader {
    fn new(script: Vec<std::io::Result<Vec<u8>>>) -> Self {
        Self {
            script: script.into(),
            current: Vec::new(),
            pos: 0,
        }
    }
}

impl std::io::Read for ScriptedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for ScriptedReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos >= self.current.len() {
            match self.script.pop_front() {
                Some(Ok(chunk)) => {
                    self.current = chunk;
                    self.pos = 0;
                }
                Some(Err(err)) => return Err(err),
                None => return Ok(&[]),
            }
        }
        Ok(&self.current[self.pos..])
    }

    fn consume(&mut self, amount: usize) {
        self.pos += amount;
    }
}

#[test]
fn fill_line_bounded_trips_the_budget_without_a_newline() {
    let chunks = (0..4).map(|_| Ok(vec![b'x'; 64 * 1024])).collect();
    let mut reader = ScriptedReader::new(chunks);
    let mut buf = Vec::new();
    let fill = fill_line_bounded(&mut reader, &mut buf, 100_000).expect("no I/O error");
    assert_eq!(fill, LineFill::Oversize);
    assert_eq!(buf.len(), 100_001, "nothing past max + 1 is copied");
}

#[test]
fn fill_line_bounded_keeps_a_split_utf8_char_across_a_wake() {
    let mut reader = ScriptedReader::new(vec![
        Ok(b"set-buffer caf\xc3".to_vec()),
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut)),
        Ok(b"\xa9\n".to_vec()),
    ]);
    let mut buf = Vec::new();
    let wake = fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES)
        .expect_err("the wake propagates");
    assert_eq!(wake.kind(), std::io::ErrorKind::TimedOut);
    let fill = fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES)
        .expect("the line completes");
    assert_eq!(fill, LineFill::Complete);
    assert_eq!(
        String::from_utf8(buf).expect("valid UTF-8 once whole"),
        "set-buffer café\n"
    );
}

#[test]
fn fill_line_bounded_reports_eof_with_a_partial() {
    let mut reader = ScriptedReader::new(vec![Ok(b"version".to_vec())]);
    let mut buf = Vec::new();
    let fill =
        fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
    assert_eq!(fill, LineFill::Eof);
    assert_eq!(buf, b"version");
}

#[test]
fn fill_line_bounded_stops_at_the_first_newline() {
    let mut reader = ScriptedReader::new(vec![Ok(b"version\nlist-panes\n".to_vec())]);
    let mut buf = Vec::new();
    let fill =
        fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
    assert_eq!(fill, LineFill::Complete);
    assert_eq!(buf, b"version\n");
    buf.clear();
    let fill =
        fill_line_bounded(&mut reader, &mut buf, MAX_CONTROL_LINE_BYTES).expect("no I/O error");
    assert_eq!(fill, LineFill::Complete);
    assert_eq!(buf, b"list-panes\n", "the second line stays buffered");
}

/// QA-199: complete lines, then the unterminated final line before EOF,
/// then Closed.
#[test]
fn read_control_line_classifies_lines() {
    let evicted = AtomicBool::new(false);
    let mut reader = std::io::Cursor::new(b"a\nb".to_vec());
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Line("a\n".to_string())
    );
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Line("b".to_string())
    );
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Closed
    );
}

/// QA-199: an unterminated stream past the budget is Oversize.
#[test]
fn read_control_line_rejects_oversize() {
    let evicted = AtomicBool::new(false);
    let mut reader = std::io::Cursor::new(vec![b'x'; MAX_CONTROL_LINE_BYTES + 10]);
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Oversize
    );
}

/// QA-199: a non-UTF-8 line is flagged with its length, and framing
/// survives — the next line still reads.
#[test]
fn read_control_line_flags_invalid_utf8() {
    let evicted = AtomicBool::new(false);
    let mut reader = std::io::Cursor::new(b"\xff\nok\n".to_vec());
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Undecodable(2)
    );
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Line("ok\n".to_string())
    );
}

/// QA-199: a poll wake keeps the partial line; eviction observed at a
/// wake closes instead.
#[test]
fn read_control_line_survives_a_wake_and_stops_when_evicted() {
    let wake = || Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
    let evicted = AtomicBool::new(false);
    let mut reader = ScriptedReader::new(vec![Ok(b"vers".to_vec()), wake(), Ok(b"ion\n".to_vec())]);
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Line("version\n".to_string())
    );
    evicted.store(true, Ordering::Relaxed);
    let mut reader = ScriptedReader::new(vec![Ok(b"vers".to_vec()), wake()]);
    assert_eq!(
        read_control_line(&mut reader, &evicted),
        ControlLine::Closed
    );
}

#[cfg(unix)]
#[cfg(unix)]
#[test]
fn graceful_shutdown_pushes_exit_to_clients_before_closing() {
    use crate::mux::ipc::connect_local_stream;

    let dir = temp_dir();
    let path = dir.path().join("exit.sock");
    let server = MuxServer::bind(&path).expect("bind");
    let shutdown = server.shutdown_handle();
    std::thread::spawn(move || server.run());

    let stream = connect_local_stream(&path).expect("connect");
    // Reading happens on a thread with channel deadlines (the
    // interprocess stream exposes no set_read_timeout). Registration
    // must be PROVEN before requesting shutdown: a command round-trip
    // means handle_client has registered this connection's sender, so
    // the shutdown broadcast has someone to reach.
    let (tx, rx) = channel();
    let (reply_seen_tx, reply_seen_rx) = channel();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let mut writer = stream.try_clone().expect("clone stream");
        writeln!(writer, "list-sessions").expect("write command");
        writer.flush().expect("flush");
        let mut reader = BufReader::new(stream);
        let mut in_reply_block = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if line.starts_with("%begin") {
                in_reply_block = true;
            } else if line.starts_with("%end") || line.starts_with("%error") {
                in_reply_block = false;
                let _ = reply_seen_tx.send(());
            } else if !in_reply_block {
                let _ = tx.send(line);
            }
        }
    });
    reply_seen_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("command reply arrives first");
    shutdown.store(true, Ordering::Relaxed);

    let line = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("%exit arrives before the socket closes");
    assert_eq!(line, "%exit\n", "graceful shutdown pushes %exit first");
}

/// Card 01a0d9b47b26, exit-when-empty: a persisting daemon holding zero
/// sessions and zero clients exits through the ordinary shutdown path,
/// and its final save is deliberate — an existing last-good snapshot is
/// cleared, so the next start is fresh rather than a resurrection.
#[cfg(unix)]
#[test]
fn an_empty_persisting_server_exits_and_clears_the_snapshot() {
    use crate::mux::persist::{load_or_quarantine, Loaded};

    let dir = temp_dir();
    let path = dir.path().join("exit-empty.sock");
    let state_path = dir.path().join("state.json");
    // Content is irrelevant to this test: only the file's removal is
    // asserted (the origin routing is what decides fresh-vs-resurrect;
    // the snapshot-content matrix is persist.rs's suite).
    let lastgood = dir.path().join("state.json.lastgood");
    std::fs::write(&lastgood, b"seed").expect("seed the snapshot");

    let server = MuxServer::bind(&path).expect("bind");
    let (done_tx, done_rx) = channel();
    std::thread::spawn(move || {
        server.run_persisting(state_path);
        let _ = done_tx.send(());
    });

    // 30 s starvation bound, not a timing assertion: unloaded the exit
    // lands within the 300 ms test grace, but the daemon thread and
    // the final save can be starved well past 5 s under full-suite
    // gate load (same class as the generation-test deadlines, 44d4212).
    done_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the empty daemon exits without anyone asking");
    match load_or_quarantine(&dir.path().join("state.json")) {
        Loaded::State(state) => assert!(
            state.sessions.is_empty(),
            "the final save holds the honest empty state"
        ),
        other => panic!("a readable state file loaded as {other:?}"),
    }
    assert!(
        !lastgood.exists(),
        "the empty exit is deliberate — the snapshot is cleared"
    );
}

/// tmux exit-empty semantics: a connected client does NOT hold an
/// empty daemon. The last session dying under an attached client ends
/// the server after the grace, and the client receives the %exit every
/// shutdown broadcasts. (`daemon.exit-empty = false` is what holds it;
/// see the test below.)
#[cfg(unix)]
#[test]
fn a_connected_client_receives_exit_when_the_tree_empties() {
    use crate::mux::ipc::connect_local_stream;
    use std::io::{BufRead, BufReader, Write};

    let dir = temp_dir();
    let path = dir.path().join("empty-under-client.sock");
    let state_path = dir.path().join("state.json");
    let server = MuxServer::bind(&path).expect("bind");
    let (done_tx, done_rx) = channel();
    std::thread::spawn(move || {
        server.run_persisting(state_path);
        let _ = done_tx.send(());
    });

    let stream = connect_local_stream(&path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let round_trip = |writer: &mut LocalStream, command: &str| -> String {
        let mut reader = {
            let clone = writer.try_clone().expect("clone for reading");
            BufReader::new(clone)
        };
        writeln!(writer, "{command}").expect("write");
        writer.flush().expect("flush");
        let mut block = String::new();
        // One full reply block: %begin … %end, in bounded time.
        for _ in 0..16 {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            block.push_str(&line);
            if line.starts_with("%end") || line.starts_with("%error") {
                break;
            }
        }
        block
    };

    // Registration proof: a completed round trip means the client's
    // sender is in the registry, so the shutdown reaches it.
    let reply = round_trip(&mut writer, "list-sessions");
    assert!(reply.contains("%end"), "first reply arrives: {reply}");

    // The daemon exits under the client: one idle tick plus the grace
    // (300 ms test value). 30 s starvation bound, not a timing
    // assertion — the exit path can be starved past 5 s under
    // full-suite gate load (observed 2026-09-27, 1/2243; 44d4212).
    done_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the empty daemon exits even though a client is attached");

    // And the client learns through the broadcast %exit (ordering per
    // the graceful-shutdown contract: the notification precedes the
    // socket close).
    let mut reader = BufReader::new(stream);
    let mut saw_exit = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line.starts_with("%exit") => {
                saw_exit = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    assert!(saw_exit, "the attached client receives %exit");
}

/// `daemon.exit-empty = false` (the live applied config) holds an empty
/// persisting daemon past the grace — WITH a connected client, the
/// shape the default-config flip test above now covers from the other
/// side: clients do not hold an empty daemon, only the knob does. It
/// must not exit where the default does (the exit test above runs the
/// same shape without a published config).
#[cfg(unix)]
#[test]
fn exit_empty_false_keeps_an_empty_persisting_server_alive() {
    use crate::mux::ipc::connect_local_stream;
    use std::io::{BufRead, BufReader, Write};

    let dir = temp_dir();
    let path = dir.path().join("exit-empty-off.sock");
    let state_path = dir.path().join("state.json");
    let mut server = MuxServer::bind(&path).expect("bind");
    let config = EffectiveConfig {
        exit_empty: false,
        ..EffectiveConfig::default()
    };
    server.set_config(Arc::new(Mutex::new(config)));
    let (done_tx, done_rx) = channel();
    std::thread::spawn(move || {
        server.run_persisting(state_path);
        let _ = done_tx.send(());
    });

    // A registered client — under the old policy this was what held an
    // empty daemon; now only the knob does.
    let stream = connect_local_stream(&path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    writeln!(writer, "list-sessions").expect("write");
    writer.flush().expect("flush");
    let mut reader = BufReader::new(writer.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("first reply line");
    assert!(
        line.starts_with("%begin"),
        "registration round trip: {line}"
    );

    // Triple the grace (test value: 300 ms): a default-config daemon
    // exits within it even with the client attached, so the knob is
    // what holds this one up.
    done_rx
        .recv_timeout(std::time::Duration::from_millis(900))
        .expect_err("exit-empty=false holds the empty daemon past the grace");
}

/// Card 01a0d9b2fd2c: an inherited 256-descriptor soft limit must not
/// cap the daemon at ~60 panes. The soft limit is lowered to 256 the
/// way an inherited daemon limit looks, binding a server raises it
/// toward the hard limit, and 70 sessions open. At roughly four
/// descriptors per pane, 70 panes need more than 256 descriptors, so
/// the raise is what lets them all live; without it the spawns die with
/// EMFILE around the 55th. The guard restores the inherited limit while
/// the test unwinds, and the panes' own Drop kills their children.
#[cfg(unix)]
#[test]
fn more_than_60_panes_open_under_a_256_descriptor_soft_limit() {
    struct NofileGuard(libc::rlim_t, libc::rlim_t);
    impl Drop for NofileGuard {
        fn drop(&mut self) {
            let limit = libc::rlimit {
                rlim_cur: self.0,
                rlim_max: self.1,
            };
            // SAFETY: the values came from getrlimit at test start.
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
        }
    }
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid rlimit out-pointer.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0,
        "read the inherited limit"
    );
    let _guard = NofileGuard(limit.rlim_cur, limit.rlim_max);
    let inherited_hard = limit.rlim_max;
    limit.rlim_cur = 256;
    // SAFETY: 256 is below the inherited hard limit, and only rlim_cur
    // changes.
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
        0,
        "lower the soft limit the way an inherited daemon limit looks"
    );

    let dir = temp_dir();
    let path = dir.path().join("nofile.sock");
    let server = MuxServer::bind(&path).expect("bind raises the soft limit");

    // The raise must have actually happened: 256 is below any sane
    // hard limit, so the soft limit after bind is strictly higher.
    let mut raised = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid rlimit out-pointer.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut raised) },
        0,
        "read the limit after bind"
    );
    assert!(
        raised.rlim_cur > 256 || inherited_hard <= 256,
        "bind raised the soft limit above 256 (now {}, hard {inherited_hard})",
        raised.rlim_cur
    );

    let tree = Arc::clone(&server.tree);
    for pane in 1..=70u32 {
        let session = tree
            .lock()
            .new_session(&format!("nofile-{pane}"), 80, 24)
            .unwrap_or_else(|err| panic!("pane {pane} of 70 opened: {err}"));
        let _ = session;
    }
    assert_eq!(
        tree.lock().sessions.len(),
        70,
        "all 70 panes live under a 256-inherited soft limit"
    );
}

/// A listener fault must not silently discard unsaved work (card
/// 01a0d9b47393: EMFILE killed the daemon with no final save). The fault
/// is injected by dup2'ing /dev/null over the listener's fd — accept
/// then fails with a non-transient error (ENOTSOCK), and the listener's
/// own Drop still closes a valid fd, so the test cannot double-close a
/// recycled descriptor.
#[cfg(unix)]
#[test]
fn a_listener_fault_still_performs_the_final_save() {
    use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};
    use std::os::fd::{AsFd as _, AsRawFd as _};

    let dir = temp_dir();
    let path = dir.path().join("fault.sock");
    let state_path = state_file_in(&dir.path().join("state"), &path);
    let server = MuxServer::bind(&path).expect("bind");

    // The fd is captured before the move into the serving thread; the
    // mutation rides the wire first so the accept loop is provably live
    // before anything breaks. The enum wraps the ud-socket listener,
    // whose AsFd is the one fd access interprocess exposes.
    let listener_fd = match &server.listener {
        interprocess::local_socket::Listener::UdSocket(inner) => inner.as_fd().as_raw_fd(),
    };
    let fault_save_path = state_path.clone();
    let serving = std::thread::spawn(move || server.run_persisting(fault_save_path));
    let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
    client
        .send_checked("new-session -s kept")
        .expect("mutation lands");
    // Disconnect before the fault so this test exercises the plain
    // save-on-fault path; the silent-client variant
    // (`a_listener_fault_exits_even_with_a_silent_connected_client`)
    // covers the join staying bounded with a connected client.
    drop(client);
    std::thread::sleep(std::time::Duration::from_millis(100));

    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
    assert!(
        null >= 0,
        "open /dev/null: {}",
        std::io::Error::last_os_error()
    );
    assert_eq!(
        unsafe { libc::dup2(null, listener_fd) },
        listener_fd,
        "replace the listener fd: {}",
        std::io::Error::last_os_error()
    );
    unsafe { libc::close(null) };

    // The accept loop sees a non-socket and breaks; the fault exit must
    // still save. Bounded like the silent-client sibling below: a
    // regression that wedges the exit fails here instead of hanging.
    let (exited_tx, exited_rx) = channel();
    let mut serving = Some(serving);
    std::thread::spawn(move || {
        if let Some(handle) = serving.take() {
            let _ = handle.join();
        }
        let _ = exited_tx.send(());
    });
    let deadline = std::time::Duration::from_secs(10);
    assert!(
        exited_rx.recv_timeout(deadline).is_ok(),
        "the fault-exit path is held open past {deadline:?}"
    );
    match load_or_quarantine(&state_path) {
        Loaded::State(state) => assert_eq!(
            state.sessions.len(),
            1,
            "the fault-exit save captured the session"
        ),
        Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
            "no state was saved on the listener fault at {}",
            state_path.display()
        ),
    }
}

/// A listener fault must not wait on connected clients (card
/// 01a0da711dfb7cf397fa99ceaba76ae1): every handler thread holds a
/// persist clone, and a silent one — connected, sends nothing, socket
/// open — never drops it, so the fault-exit join could only complete by
/// the worker observing the shutdown flag, not by the channel closing.
/// The join is bounded: a completion channel fires within the deadline
/// or the test fails.
#[cfg(unix)]
#[test]
fn a_listener_fault_exits_even_with_a_silent_connected_client() {
    use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};
    use std::os::fd::{AsFd as _, AsRawFd as _};

    let dir = temp_dir();
    let path = dir.path().join("fault-silent.sock");
    let state_path = state_file_in(&dir.path().join("state"), &path);
    let server = MuxServer::bind(&path).expect("bind");

    let listener_fd = match &server.listener {
        interprocess::local_socket::Listener::UdSocket(inner) => inner.as_fd().as_raw_fd(),
    };
    let fault_save_path = state_path.clone();
    let serving = std::thread::spawn(move || server.run_persisting(fault_save_path));
    let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
    client
        .send_checked("new-session -s kept")
        .expect("mutation lands");
    // The silent client: connected, registered no command, sends
    // nothing, and stays alive past the fault. Its handler thread must
    // not hold the fault-exit join open.
    let silent = crate::mux::MuxClient::connect(&path).expect("silent client connects");
    std::thread::sleep(std::time::Duration::from_millis(100));

    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
    assert!(
        null >= 0,
        "open /dev/null: {}",
        std::io::Error::last_os_error()
    );
    assert_eq!(
        unsafe { libc::dup2(null, listener_fd) },
        listener_fd,
        "replace the listener fd: {}",
        std::io::Error::last_os_error()
    );
    unsafe { libc::close(null) };

    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    let mut serving = Some(serving);
    std::thread::spawn(move || {
        if let Some(handle) = serving.take() {
            let _ = handle.join();
        }
        let _ = exited_tx.send(());
    });
    let deadline = std::time::Duration::from_secs(10);
    assert!(
        exited_rx.recv_timeout(deadline).is_ok(),
        "the fault exit is held open past {deadline:?} — a silent client's \
         persist clone is pinning the persist worker's channel open"
    );
    match load_or_quarantine(&state_path) {
        Loaded::State(state) => assert_eq!(
            state.sessions.len(),
            1,
            "the fault-exit save captured the session"
        ),
        Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
            "no state was saved on the listener fault at {}",
            state_path.display()
        ),
    }
    drop(silent);
}

/// The requested-shutdown side of card 01a0da711dfb7cf397fa99ceaba76ae1:
/// SIGTERM and `kill-server` raise the same flag this test raises, and
/// the flag — not the persist channel's sender count — must bound the
/// exit join when a silent client's handler holds a sender clone.
#[cfg(unix)]
#[test]
fn a_requested_shutdown_exits_even_with_a_silent_connected_client() {
    use crate::mux::persist::{load_or_quarantine, state_file_in, Loaded};

    let dir = temp_dir();
    let path = dir.path().join("shutdown-silent.sock");
    let state_path = state_file_in(&dir.path().join("state"), &path);
    let server = MuxServer::bind(&path).expect("bind");
    let shutdown = server.shutdown_handle();
    let save_path = state_path.clone();
    let serving = std::thread::spawn(move || server.run_persisting(save_path));

    let mut client = crate::mux::MuxClient::connect(&path).expect("client connects");
    client
        .send_checked("new-session -s kept")
        .expect("mutation lands");
    // Silent: connected, never sends, outlives the shutdown.
    let silent = crate::mux::MuxClient::connect(&path).expect("silent client connects");
    std::thread::sleep(std::time::Duration::from_millis(100));
    shutdown.store(true, Ordering::Relaxed);

    let (exited_tx, exited_rx) = channel();
    let mut serving = Some(serving);
    std::thread::spawn(move || {
        if let Some(handle) = serving.take() {
            let _ = handle.join();
        }
        let _ = exited_tx.send(());
    });
    let deadline = std::time::Duration::from_secs(10);
    assert!(
        exited_rx.recv_timeout(deadline).is_ok(),
        "the requested-shutdown exit is held open past {deadline:?} — a silent \
         client's persist clone is pinning the persist worker's channel open"
    );
    match load_or_quarantine(&state_path) {
        Loaded::State(state) => assert_eq!(
            state.sessions.len(),
            1,
            "the shutdown save captured the session"
        ),
        Loaded::Fresh | Loaded::Quarantined { .. } => panic!(
            "no state was saved on the requested shutdown at {}",
            state_path.display()
        ),
    }
    drop(silent);
}

/// A shutdown removes the socket file: `--stop` reporting success
/// while the file stays behind makes `ls par-mux-*.sock` useless for
/// spotting live daemons. SIGTERM and `kill-server` raise the same
/// flag this test raises.
#[test]
fn a_shutdown_removes_the_socket_file() {
    let dir = temp_dir();
    let path = dir.path().join("stop-unlink.sock");
    let server = MuxServer::bind(&path).expect("bind");
    assert!(path.exists(), "the socket file exists while serving");
    let shutdown = server.shutdown_handle();
    let serving = std::thread::spawn(move || server.run());
    shutdown.store(true, Ordering::Relaxed);
    // Bounded like the silent-client siblings: a shutdown that never
    // finishes fails here instead of hanging the suite.
    let (exited_tx, exited_rx) = channel();
    let mut serving = Some(serving);
    std::thread::spawn(move || {
        if let Some(handle) = serving.take() {
            let _ = handle.join();
        }
        let _ = exited_tx.send(());
    });
    let deadline = std::time::Duration::from_secs(10);
    assert!(
        exited_rx.recv_timeout(deadline).is_ok(),
        "the shutdown exit is held open past {deadline:?}"
    );
    assert!(!path.exists(), "the socket file is removed after shutdown");
}

#[cfg(unix)]
#[test]
fn bind_replaces_a_stale_socket_file_and_sets_mode_0600() {
    let dir = temp_dir();
    let path = dir.path().join("stale.sock");
    std::fs::write(&path, b"stale junk").expect("write stale file");

    let server = MuxServer::bind(&path).expect("bind replaces a stale socket file");
    assert_eq!(server.path(), path.as_path());

    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path)
        .expect("socket file exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "socket must be owner-only");

    drop(server);
}

fn harness() -> (Arc<Mutex<MuxTree>>, Clients) {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    (tree, clients)
}

/// A factory whose panes stay silent, so a capture-range test sees only
/// the bytes it fed the terminal itself — no shell-prompt races.
struct SilentPaneFactory;

impl crate::mux::pane::PaneFactory for SilentPaneFactory {
    fn create_pane(
        &self,
        id: crate::mux::ids::PaneId,
        cols: u16,
        rows: u16,
        _command: Option<&str>,
        context: &crate::mux::pane::SpawnContext<'_>,
    ) -> Result<crate::mux::pane::MuxPane, crate::mux::pane::MuxError> {
        ShellPaneFactory::default().create_pane(id, cols, rows, Some("sleep 30"), context)
    }

    fn create_dead_pane(
        &self,
        id: crate::mux::ids::PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<crate::mux::pane::MuxPane, crate::mux::pane::MuxError> {
        ShellPaneFactory::default().create_dead_pane(id, cols, rows, command, exit_code)
    }
}

fn quiet_harness() -> (Arc<Mutex<MuxTree>>, Clients) {
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(SilentPaneFactory))));
    let clients = Arc::new(Mutex::new(Vec::new()));
    (tree, clients)
}

#[test]
fn pane_info_reports_the_window_and_the_pane_grid_size() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s info", 1, &tree, &clients, None);
    dispatch("refresh-client -t %0 -C 100x30", 2, &tree, &clients, None);
    let body = |reply: &str| -> Vec<String> {
        reply
            .lines()
            .filter(|l| !l.starts_with("%begin") && !l.starts_with("%end"))
            .map(str::to_string)
            .collect()
    };
    let reply = dispatch("pane-info -t %0", 3, &tree, &clients, None);
    // The harness pane runs a real shell, so the reply may carry the
    // optional `cmd=` token; the fixed prefix must stay parseable by
    // older clients either way.
    let line = body(&reply).join("");
    assert!(
        line == "%0 @0 100x30" || line.starts_with("%0 @0 100x30 cmd="),
        "pane-info keeps its fixed prefix: {reply}"
    );

    let split = dispatch("split-window -h -t %0", 4, &tree, &clients, None);
    assert!(!split.contains("%error"), "{split}");
    let reply = dispatch("pane-info -t %0", 5, &tree, &clients, None);
    let line = body(&reply).join("");
    assert!(
        line.starts_with("%0 @0 ") && !line.ends_with(" 100x30"),
        "a split re-fits the pane: {reply}"
    );

    let reply = dispatch("pane-info -t %99", 6, &tree, &clients, None);
    assert!(reply.contains("%error"), "{reply}");
}

#[test]
fn bare_new_window_targets_the_newest_session_and_errors_without_one() {
    let (tree, clients) = harness();
    // No sessions yet: bare new-window is an error, like tmux's
    // "no current client" refusal.
    let reply = dispatch("new-window", 1, &tree, &clients, None);
    assert!(reply.contains("%error"), "no sessions: {reply}");

    dispatch("new-session -s first", 2, &tree, &clients, None);
    dispatch("new-session -s second", 3, &tree, &clients, None);
    let second = tree.lock().sessions()[1];
    let first = tree.lock().sessions()[0];

    let reply = dispatch("new-window -n bare", 4, &tree, &clients, None);
    assert!(reply.contains("%end"), "bare new-window succeeds: {reply}");
    // The window landed in the most-recently-created session, not the
    // first one.
    assert_eq!(tree.lock().session(second).unwrap().windows.len(), 2);
    assert_eq!(tree.lock().session(first).unwrap().windows.len(), 1);
}

#[test]
fn new_window_dispatch_creates_a_window_and_wires_its_pane() {
    let (tree, clients) = harness();
    let session_reply = dispatch("new-session -s main", 1, &tree, &clients, None);
    assert!(session_reply.contains("%end"), "new-session succeeds");

    let session_id = tree.lock().sessions()[0];
    let reply = dispatch(
        &format!("new-window -t {session_id}"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "new-window succeeds: {reply}");

    let session = tree.lock().session(session_id).unwrap().windows.clone();
    assert_eq!(session.len(), 2, "session now has two windows");
}

#[test]
fn window_lifecycle_dispatch_round_trips() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    dispatch(
        &format!("new-window -t {session_id}"),
        2,
        &tree,
        &clients,
        None,
    );
    let window_id = tree.lock().session(session_id).unwrap().windows[1];

    let select = dispatch(
        &format!("select-window -t {window_id}"),
        3,
        &tree,
        &clients,
        None,
    );
    assert!(select.contains("%end"), "select-window succeeds");
    assert_eq!(tree.lock().session(session_id).unwrap().active, 1);

    let rename = dispatch(
        &format!("rename-window -t {window_id} scratch"),
        4,
        &tree,
        &clients,
        None,
    );
    assert!(rename.contains("%end"), "rename-window succeeds");
    assert_eq!(tree.lock().window(window_id).unwrap().name, "scratch");

    let list_windows = dispatch("list-windows", 5, &tree, &clients, None);
    assert!(list_windows.contains("scratch"));

    let list_sessions = dispatch("list-sessions", 6, &tree, &clients, None);
    assert!(list_sessions.contains("main"));

    let kill = dispatch(
        &format!("kill-window -t {window_id}"),
        7,
        &tree,
        &clients,
        None,
    );
    assert!(kill.contains("%end"), "kill-window succeeds");
    assert!(tree.lock().window(window_id).is_none());
}

/// Collect every broadcast a non-issuing client receives within a bounded
/// window after a dispatch, so lifecycle-notification assertions see the
/// push lines rather than the command's own reply block.
fn drain_broadcasts(rx: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
        lines.push(line);
    }
    lines
}

/// Card 01a0d9e6f012, criterion 1: a client's cell pixel report rides
/// `refresh-client -p` through the control protocol, lands daemon-wide,
/// and reaches the pane terminal's pixel state — what `CSI 14 t`/`16 t`
/// in the pane answer from. A `-C` grid resize afterwards keeps the
/// reported cells (every re-fit re-derives the totals).
#[test]
fn refresh_client_pixel_report_reaches_the_pane_terminal() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane = tree.lock().window(window_id).unwrap().panes()[0];

    dispatch(
        &format!("refresh-client -t {pane} -p 10x20"),
        2,
        &tree,
        &clients,
        None,
    );
    {
        let term = tree.lock().pane(pane).unwrap().terminal();
        let term = term.read();
        assert_eq!(
            (term.pixel_width, term.pixel_height),
            (800, 480),
            "an 80x24 pane at 10x20 px cells"
        );
        assert_eq!(
            term.graphics.cell_dimensions,
            (10, 20),
            "image cell-span math uses the reported cell size"
        );
    }

    // A later grid resize re-derives the pixel totals from the kept
    // cell report — the attach-and-resize flow par-term runs.
    dispatch(
        &format!("refresh-client -t {pane} -C 120x40 -p 10x20"),
        3,
        &tree,
        &clients,
        None,
    );
    let term = tree.lock().pane(pane).unwrap().terminal();
    let term = term.read();
    assert_eq!(
        (term.pixel_width, term.pixel_height),
        (1200, 800),
        "a 120x40 grid at the kept 10x20 px cells"
    );
}

#[test]
fn mutating_dispatches_broadcast_lifecycle_notifications_to_other_clients() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let first = tree.lock().window(window_id).unwrap().panes()[0];
    // A second, non-issuing client: everything it sees is a broadcast.
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    // new-window broadcasts %window-add naming the new window.
    dispatch(
        &format!("new-window -t {session_id} -n logs"),
        2,
        &tree,
        &clients,
        None,
    );
    let second_window = tree.lock().session(session_id).unwrap().windows[1];
    let lines = drain_broadcasts(&rx);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("%window-add") && l.contains(&second_window.to_string())),
        "new-window must broadcast %window-add naming it: {lines:?}"
    );

    // rename-window broadcasts %window-renamed with the new name.
    dispatch(
        &format!("rename-window -t {window_id} scratch"),
        3,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%window-renamed")
            && l.contains(&window_id.to_string())
            && l.contains("scratch")),
        "rename-window must broadcast %window-renamed: {lines:?}"
    );

    // split-window focuses the new pane; select-pane moves focus back.
    dispatch(
        &format!("split-window -t {first} -h"),
        4,
        &tree,
        &clients,
        None,
    );
    let _ = drain_broadcasts(&rx);
    dispatch(&format!("select-pane -t {first}"), 5, &tree, &clients, None);
    let lines = drain_broadcasts(&rx);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("%window-pane-changed") && l.contains(&first.to_string())),
        "select-pane must broadcast %window-pane-changed: {lines:?}"
    );

    // kill-pane changes geometry (and focus, since the active pane died).
    let second_pane = tree.lock().window(window_id).unwrap().panes()[1];
    dispatch(
        &format!("kill-pane -t {second_pane}"),
        6,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%layout-change")),
        "kill-pane must broadcast %layout-change: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("%window-pane-changed")),
        "kill-pane of the active pane must broadcast %window-pane-changed: {lines:?}"
    );

    // kill-window broadcasts %window-close naming the window.
    dispatch(
        &format!("kill-window -t {second_window}"),
        7,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("%window-close") && l.contains(&second_window.to_string())),
        "kill-window must broadcast %window-close naming it: {lines:?}"
    );
    // The session survives (its first window remains), so the set of
    // sessions did NOT change — the kill above must not have sent
    // %sessions-changed either.
    assert!(
        !lines.iter().any(|l| l.starts_with("%sessions-changed")),
        "a surviving session is not a session-set change: {lines:?}"
    );
}

/// Card 01a0d9b47b26: a client must learn its session is gone through a
/// defined line, not infer it from an empty tab set. tmux's cue is the
/// argument-less `%sessions-changed`, on both the create and destroy
/// sides of the session set.
#[test]
fn session_set_changes_broadcast_sessions_changed() {
    let (tree, clients) = quiet_harness();

    // An observer client: everything it sees is a broadcast.
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    // Create side: new-session lands in the observer's channel.
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%sessions-changed")),
        "new-session must broadcast %sessions-changed: {lines:?}"
    );

    // Destroy side: kill-window of the session's last window cascades to
    // the session, and the client learns it after the window-close line.
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    dispatch(
        &format!("kill-window -t {window_id}"),
        2,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    let close = lines
        .iter()
        .position(|l| l.starts_with("%window-close"))
        .expect("%window-close precedes the session cue");
    let changed = lines
        .iter()
        .position(|l| l.starts_with("%sessions-changed"))
        .expect("the emptied session must broadcast %sessions-changed");
    assert!(
        close < changed,
        "%window-close names the window first, %sessions-changed follows: {lines:?}"
    );
}

/// The kill-pane cascade sends the same cue: a session emptied through
/// its last pane is still a session-set change.
#[test]
fn a_kill_pane_that_empties_the_session_broadcasts_sessions_changed() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));
    dispatch(&format!("kill-pane -t {pane_id}"), 2, &tree, &clients, None);
    let lines = drain_broadcasts(&rx);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("%window-close") && l.contains(&window_id.to_string())),
        "the emptied window closes first: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("%sessions-changed")),
        "kill-pane's cascade to the session must broadcast %sessions-changed: {lines:?}"
    );
    assert!(tree.lock().session(session_id).is_none());
}

/// rename-session and kill-session (card 01a0ea74ec2e): the rename
/// broadcasts %session-renamed with the new name; the kill closes every
/// window in the session before the %sessions-changed cue.
#[test]
fn rename_and_kill_session_broadcast_their_notifications() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    dispatch(
        &format!("new-window -t {session_id} -n logs"),
        2,
        &tree,
        &clients,
        None,
    );
    let windows = tree.lock().session(session_id).unwrap().windows.clone();

    // An observer client registered before the mutations: everything it
    // sees is a broadcast.
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    dispatch(
        &format!("rename-session -t {session_id} 'Renamed Main'"),
        3,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%session-renamed")
            && l.contains(&session_id.to_string())
            && l.contains("Renamed Main")),
        "rename-session must broadcast %session-renamed with the new name: {lines:?}"
    );
    assert_eq!(
        tree.lock().session(session_id).unwrap().name,
        "Renamed Main",
        "the tree keeps the new name for future spawns"
    );

    dispatch(
        &format!("kill-session -t {session_id}"),
        4,
        &tree,
        &clients,
        None,
    );
    let lines = drain_broadcasts(&rx);
    for window in &windows {
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("%window-close") && l.contains(&window.to_string())),
            "kill-session must broadcast %window-close for {window}: {lines:?}"
        );
    }
    let close = lines
        .iter()
        .position(|l| l.starts_with("%window-close"))
        .expect("the killed windows close first");
    let changed = lines
        .iter()
        .position(|l| l.starts_with("%sessions-changed"))
        .expect("kill-session must broadcast %sessions-changed");
    assert!(
        close < changed,
        "%window-close lines precede %sessions-changed: {lines:?}"
    );
    assert!(
        tree.lock().session(session_id).is_none(),
        "the session is gone from the tree"
    );
    assert!(
        tree.lock().sessions().is_empty(),
        "kill-session leaves no orphaned windows or sessions behind"
    );
}

#[test]
fn new_session_broadcasts_window_add_and_notifies_the_issuer() {
    let (tree, clients) = harness();
    // The issuing client is NOT in the broadcast set: its channel receives
    // only what dispatch directs to it specifically.
    let (issuer_tx, issuer_rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    dispatch_issued(
        "new-session -s main",
        1,
        &tree,
        &clients,
        None,
        Some(&issuer_tx),
    );
    let lines = drain_broadcasts(&issuer_rx);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("%session-changed") && l.contains("main")),
        "the issuing client must be told %session-changed: {lines:?}"
    );
    // The broadcast set (no other clients here) separately receives
    // %window-add for the session's initial window; that half is covered
    // by mutating_dispatches_broadcast_lifecycle_notifications_to_other_clients.
}

#[test]
fn window_commands_report_an_error_block_for_an_unknown_target() {
    let (tree, clients) = harness();
    let reply = dispatch("select-window -t @999", 1, &tree, &clients, None);
    assert!(
        reply.contains("%error"),
        "unknown window is an error: {reply}"
    );
}

#[test]
fn split_window_dispatch_splits_and_broadcasts_a_layout_change() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    // A second client observes the broadcast.
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    let reply = dispatch(
        &format!("split-window -t {pane_id} -h -p 25"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "split-window succeeds: {reply}");

    // The pane behind new-session is an interactive shell (harness uses
    // ShellPaneFactory, not SilentPaneFactory), so its banner can
    // broadcast a %output before the %layout-change lands — poll for the
    // layout change instead of asserting the FIRST notification is it
    // (single-shot recv raced cmd.exe's banner on Windows, run 36051903299).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut notification = String::from("<no notification>");
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(msg) if msg.contains("%layout-change") => {
                notification = msg;
                break;
            }
            Ok(msg) => notification = msg,
            Err(_) => break,
        }
    }
    assert!(
        notification.contains("%layout-change"),
        "notification: {notification}"
    );
    assert!(
        notification.contains(&window_id.to_string()),
        "names the mutated window: {notification}"
    );

    // The split geometry is real: -p 25 gives the new pane 20 of 80 cols.
    let geo = {
        let guard = tree.lock();
        let window = guard.window(window_id).unwrap();
        window
            .layout
            .geometry(0, 0, window.cols as usize, window.rows as usize)
    };
    let widths: Vec<_> = geo.iter().map(|g| (g.pane, g.width)).collect();
    assert!(
        widths.contains(&(pane_id, 60)),
        "target keeps 60 cols: {widths:?}"
    );
    assert!(
        widths.iter().any(|&(_, width)| width == 20),
        "new pane gets 20 cols: {widths:?}"
    );
}

#[test]
fn pane_commands_dispatch_through_the_tree() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let first = tree.lock().window(window_id).unwrap().panes()[0];

    // Side by side, so the -R resize below moves the shared divider.
    let split = dispatch(
        &format!("split-window -t {first} -h"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(split.contains("%end"), "split-window succeeds: {split}");
    // The reply body carries the new pane id — a client needs it to
    // address the pane it just created.
    let second = tree.lock().window(window_id).unwrap().panes()[1];
    assert!(
        split.contains(&second.to_string()),
        "reply names the new pane: {split}"
    );

    let select = dispatch(&format!("select-pane -t {first}"), 3, &tree, &clients, None);
    assert!(select.contains("%end"), "select-pane succeeds: {select}");
    assert_eq!(tree.lock().window(window_id).unwrap().active, first);

    let resize = dispatch(
        &format!("resize-pane -t {first} -R 10"),
        4,
        &tree,
        &clients,
        None,
    );
    assert!(resize.contains("%end"), "resize-pane succeeds: {resize}");

    let swap = dispatch(
        &format!("swap-pane -t {first} -s {second}"),
        5,
        &tree,
        &clients,
        None,
    );
    assert!(swap.contains("%end"), "swap-pane succeeds: {swap}");
    assert_eq!(
        tree.lock().window(window_id).unwrap().panes(),
        vec![second, first],
        "the panes traded positions"
    );
}

#[test]
fn pane_commands_report_an_error_block_for_unknown_panes() {
    let (tree, clients) = harness();
    for command in [
        "split-window -t %999",
        "select-pane -t %999",
        "resize-pane -t %999 -R",
        "resize-pane -t %999 -x 40",
        "swap-pane -t %999 -s %998",
    ] {
        let reply = dispatch(command, 1, &tree, &clients, None);
        assert!(reply.contains("%error"), "{command} is an error: {reply}");
    }
}

#[test]
fn resize_pane_absolute_dispatches_and_tracks_the_terminals() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let first = tree.lock().window(window_id).unwrap().panes()[0];
    let split = dispatch(
        &format!("split-window -t {first} -h"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(split.contains("%end"));
    let second = tree.lock().window(window_id).unwrap().panes()[1];

    // A second client observes the size-driven re-layout.
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    let reply = dispatch(
        &format!("resize-pane -t {first} -x 25"),
        3,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "absolute resize succeeds: {reply}");

    // A resized pane's shell gets SIGWINCH and (bash under ConPTY)
    // emits a DECXCPR query whose %output can beat the %layout-change
    // to this observer — poll for the layout change instead of
    // asserting the FIRST notification is it (QA-143; same shape the
    // split-window test at the banner race hit, run 36051903299).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut notification = String::from("<no notification>");
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(msg) if msg.contains("%layout-change") => {
                notification = msg;
                break;
            }
            Ok(msg) => notification = msg,
            Err(_) => break,
        }
    }
    assert!(
        notification.contains("%layout-change"),
        "notification: {notification}"
    );

    let size_of = |pane| tree.lock().pane(pane).unwrap().terminal().read().size();
    assert_eq!(size_of(first), (25, 24));
    assert_eq!(
        size_of(second),
        (55, 24),
        "the sibling absorbs the difference on the wire too"
    );
}

#[test]
fn refresh_client_size_report_resizes_the_window_and_broadcasts() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    let reply = dispatch(
        &format!("refresh-client -t {pane_id} -C 120x40"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "-C report succeeds: {reply}");

    // Same drain as the resize test: the re-fitted pane's shell can
    // race a %output past the %layout-change after its SIGWINCH.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut notification = String::from("<no notification>");
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(msg) if msg.contains("%layout-change") => {
                notification = msg;
                break;
            }
            Ok(msg) => notification = msg,
            Err(_) => break,
        }
    }
    assert!(
        notification.contains("%layout-change") && notification.contains("120x40"),
        "the broadcast carries the new geometry: {notification}"
    );

    let guard = tree.lock();
    let window = guard.window(window_id).unwrap();
    assert_eq!((window.cols, window.rows), (120, 40));
    assert_eq!(
        guard.pane(pane_id).unwrap().terminal().read().size(),
        (120, 40),
        "the pane terminal was re-fitted"
    );
}

#[test]
fn refresh_client_size_report_rejects_an_unknown_pane() {
    let (tree, clients) = harness();
    let reply = dispatch("refresh-client -t %999 -C 120x40", 1, &tree, &clients, None);
    assert!(reply.contains("%error"));
}

#[test]
fn capture_pane_reports_the_pane_screen() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    // A freshly spawned pane's screen is empty until the shell writes a
    // prompt; assert the reply is well-formed rather than racing that.
    let reply = dispatch(
        &format!("capture-pane -t {pane_id} -p"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "capture-pane succeeds: {reply}");
}

#[test]
fn capture_pane_rejects_an_unknown_pane() {
    let (tree, clients) = harness();
    let reply = dispatch("capture-pane -t %999 -p", 1, &tree, &clients, None);
    assert!(reply.contains("%error"));
}

#[test]
fn capture_range_slices_history_with_tmux_negative_offsets() {
    // export_scrollback emits newest-first: h1 is the oldest history
    // line, h3 the newest (the line directly above a two-line screen).
    let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(-2), Some(-1));
    assert_eq!(out, "h2\nh3");
}

#[test]
fn capture_range_positive_offsets_address_screen_lines() {
    let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(0), Some(1));
    assert_eq!(out, "s1\ns2");
}

#[test]
fn capture_range_start_only_defaults_end_to_the_screen_bottom() {
    let out = capture_range("h3\nh2\nh1\n", "s1\ns2", Some(-3), None);
    assert_eq!(out, "h1\nh2\nh3\ns1\ns2");
}

#[test]
fn capture_range_end_only_defaults_start_to_the_first_screen_line() {
    let out = capture_range("h3\nh2\nh1\n", "s1\ns2", None, Some(0));
    assert_eq!(out, "s1");
}

#[test]
fn capture_range_clamps_offsets_beyond_the_buffer() {
    let out = capture_range("h1\n", "s1\ns2", Some(-99), Some(99));
    assert_eq!(out, "h1\ns1\ns2");
}

#[test]
fn capture_range_returns_empty_for_a_reversed_range() {
    let out = capture_range("h2\nh1\n", "s1", Some(1), Some(-1));
    assert_eq!(out, "");
}

#[test]
fn capture_pane_s_and_e_return_the_requested_line_range() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    // Thirty lines on a 24-row pane: L01..L06 scroll into history and
    // L07..L30 stay visible. No trailing newline, so L30 holds the last
    // row and nothing scrolls past it.
    let payload = (1..=30)
        .map(|n| format!("L{n:02}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    tree.lock()
        .pane_mut(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(payload.as_bytes());

    let reply = dispatch(
        &format!("capture-pane -t {pane_id} -p -S -2 -E -1"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "capture succeeds: {reply}");
    assert!(
        reply.contains("L05"),
        "second-to-last history line: {reply}"
    );
    assert!(reply.contains("L06"), "last history line: {reply}");
    assert!(
        !reply.contains("L04"),
        "-S -2 starts at the second history line back: {reply}"
    );
    assert!(
        !reply.contains("L07"),
        "-E -1 ends above the visible screen: {reply}"
    );
}

#[test]
fn capture_pane_e_returns_sgr_rows_and_plain_stays_plain() {
    let (tree, clients) = quiet_harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let window_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(window_id).unwrap().panes()[0];

    // Styled content at a known position: a bold, blue-background tag
    // on row 2 and plain text on row 4.
    let payload = b"\x1b[2;3H\x1b[1;44mTAG\x1b[0m\x1b[4;1Hplain";
    tree.lock()
        .pane_mut(pane_id)
        .unwrap()
        .terminal()
        .write()
        .process(payload);

    let plain = dispatch(
        &format!("capture-pane -t {pane_id} -p"),
        2,
        &tree,
        &clients,
        None,
    );
    let escaped = dispatch(
        &format!("capture-pane -t {pane_id} -p -e"),
        3,
        &tree,
        &clients,
        None,
    );
    assert!(plain.contains("%end"), "plain capture succeeds: {plain}");
    assert!(escaped.contains("%end"), "-e capture succeeds: {escaped}");

    // Without -e the reply stays the pre--e capture: the pane's
    // logical lines, no ESC byte.
    let expected = tree
        .lock()
        .pane(pane_id)
        .unwrap()
        .terminal()
        .read()
        .content();
    assert!(
        plain.contains("TAG") && plain.contains("plain"),
        "plain capture carries the text: {plain}"
    );
    assert!(
        plain.contains(&expected),
        "plain capture is content(): {plain:?} vs {expected:?}"
    );
    assert!(
        !plain.contains('\x1b'),
        "plain capture carries no ESC byte: {plain:?}"
    );

    // With -e the styled row carries its SGR run inline (reset, fg —
    // default White omitted — bg, per push_sgr_style's fixed order)
    // and a reset before the line break; the unstyled row stays
    // plain text.
    assert!(
        escaped.contains("\x1b[0;44;1") && escaped.contains("TAG\x1b[0m\n"),
        "-e capture carries the styled run inline, reset before the \
         line break: {escaped:?}"
    );
    assert!(
        escaped.contains("plain\n"),
        "-e capture keeps the unstyled row plain: {escaped:?}"
    );
}

#[test]
fn buffer_round_trips_through_set_and_show() {
    let (tree, clients) = harness();
    let empty = dispatch("show-buffer", 1, &tree, &clients, None);
    assert!(empty.contains("%error"), "no buffer yet: {empty}");

    let set = dispatch("set-buffer hello world", 2, &tree, &clients, None);
    assert!(set.contains("%end"), "set-buffer succeeds");

    let show = dispatch("show-buffer", 3, &tree, &clients, None);
    assert!(show.contains("hello world"), "show-buffer: {show}");
}

#[test]
fn set_buffer_quoted_payload_round_trips() {
    let (tree, clients) = harness();
    let set = dispatch(
        "set-buffer 'it'\\''s \"doubly\" quoted'",
        1,
        &tree,
        &clients,
        None,
    );
    assert!(set.contains("%end"), "quoted set-buffer succeeds: {set}");

    let show = dispatch("show-buffer", 2, &tree, &clients, None);
    assert!(
        show.contains("it's \"doubly\" quoted"),
        "quotes stripped, not stored: {show}"
    );
}

#[test]
fn set_buffer_hex_payload_is_byte_exact() {
    let (tree, clients) = harness();
    // "a\nb\n" — the trailing newline is exactly what the reply-block
    // format cannot express, so assert on the STORED content through
    // the tree rather than show-buffer.
    let set = dispatch("set-buffer -H 61 0a 62 0a", 1, &tree, &clients, None);
    assert!(set.contains("%end"), "hex set-buffer succeeds: {set}");

    let stored = tree.lock().get_buffer("default").map(str::to_string);
    assert_eq!(stored.as_deref(), Some("a\nb\n"), "stored byte-exact");
}

#[test]
fn set_buffer_hex_payload_rejects_garbage() {
    let (tree, clients) = harness();
    let bad = dispatch("set-buffer -H zz", 1, &tree, &clients, None);
    assert!(bad.contains("%error"), "invalid hex is an error: {bad}");
    let none = dispatch("set-buffer -H", 1, &tree, &clients, None);
    assert!(
        none.contains("%error"),
        "missing payload is an error: {none}"
    );
}

#[test]
fn paste_buffer_writes_the_buffer_to_the_target_pane() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let pane_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(pane_id).unwrap().panes()[0];

    dispatch("set-buffer echo par-mux-paste", 2, &tree, &clients, None);
    let reply = dispatch(
        &format!("paste-buffer -t {pane_id}"),
        3,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%end"), "paste-buffer succeeds: {reply}");
}

#[test]
fn paste_buffer_rejects_an_unknown_pane() {
    let (tree, clients) = harness();
    dispatch("set-buffer hi", 1, &tree, &clients, None);
    let reply = dispatch("paste-buffer -t %999", 2, &tree, &clients, None);
    assert!(reply.contains("%error"));
}

#[test]
fn paste_buffer_with_no_stored_buffer_is_an_error() {
    let (tree, clients) = harness();
    dispatch("new-session -s main", 1, &tree, &clients, None);
    let session_id = tree.lock().sessions()[0];
    let pane_id = tree.lock().session(session_id).unwrap().windows[0];
    let pane_id = tree.lock().window(pane_id).unwrap().panes()[0];

    let reply = dispatch(
        &format!("paste-buffer -t {pane_id}"),
        2,
        &tree,
        &clients,
        None,
    );
    assert!(reply.contains("%error"), "no buffer set: {reply}");
}

#[test]
fn mutating_dispatch_saves_state_and_read_only_dispatch_does_not() {
    let (tree, clients) = quiet_harness();
    let dir = temp_dir();
    let target = dir.path().join("state.json");

    let (tx, worker) = spawn_persist_worker(target.clone(), Arc::new(AtomicBool::new(false)));

    dispatch("list-sessions", 1, &tree, &clients, Some(&tx));
    assert!(
        !target.exists(),
        "a read-only dispatch must not touch the state file"
    );

    dispatch("new-session -s main", 2, &tree, &clients, Some(&tx));
    // The worker writes off the dispatch path, so poll for the landing
    // rather than asserting synchronously (the wait_until shape).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !target.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(target.exists(), "a mutating dispatch saves the state file");

    match crate::mux::persist::load_or_quarantine(&target) {
        crate::mux::persist::Loaded::State(state) => {
            assert_eq!(state.format_version, crate::mux::persist::FORMAT_VERSION);
            assert_eq!(state.sessions.len(), 1);
            assert_eq!(state.sessions[0].name, "main");
        }
        other => panic!("expected a readable state file, got {other:?}"),
    }

    // The channel closing (all senders dropped) is the worker's exit.
    drop(tx);
    let _ = worker.join();
}

/// A burst of queued states must coalesce: fewer writes than states,
/// and the last state written is the newest one sent.
#[test]
fn persist_worker_coalesces_a_burst_to_the_newest_state() {
    let (tx, rx) = channel::<(SaveOrigin, PersistState)>();
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let last_seen = Arc::new(Mutex::new(None::<u64>));
    let stop = Arc::new(AtomicBool::new(false));

    let write_count = Arc::clone(&writes);
    let seen = Arc::clone(&last_seen);
    let worker = std::thread::spawn(move || {
        persist_worker_loop(
            rx,
            Path::new("/nonexistent-par-mux-coalescing-test"),
            &stop,
            |_origin: SaveOrigin, state: &PersistState, _path| {
                write_count.fetch_add(1, Ordering::Relaxed);
                *seen.lock() = Some(state.saved_at_unix_ms);
                Ok(())
            },
        )
    });

    // Hand-constructed states with distinct capture stamps; the worker
    // cannot keep up with 50 in-flight sends, so writes < sends.
    let burst: u64 = 50;
    for stamp in 1..=burst {
        let state = PersistState {
            workspaces: Vec::new(),
            active_workspace: None,
            format_version: crate::mux::persist::FORMAT_VERSION,
            saved_at_unix_ms: stamp,
            next_ids: (0, 0, 0, 0),
            sessions: Vec::new(),
            buffers: std::collections::HashMap::new(),
        };
        tx.send((SaveOrigin::Command, state))
            .expect("worker owns the receiver");
    }
    drop(tx);
    worker.join().expect("worker exits when the channel closes");
    assert!(
        (writes.load(Ordering::Relaxed) as u64) < burst,
        "a burst of {burst} states coalesced to {} writes",
        writes.load(Ordering::Relaxed)
    );
    assert_eq!(
        last_seen.lock().as_ref().copied(),
        Some(burst),
        "the newest state is the one written"
    );
}

/// ARC-011: a client whose queue fills (it stopped reading the socket)
/// is evicted from the broadcast set, while a draining sibling still
/// receives every line pushed past the eviction.
#[test]
fn a_stalled_client_is_evicted_and_a_draining_sibling_keeps_every_line() {
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));

    // The stalled client: registered, never drained.
    let (stalled_tx, _stalled_rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
    let stalled_flag = Arc::new(AtomicBool::new(false));
    clients.lock().push((
        1,
        stalled_tx,
        Arc::clone(&stalled_flag),
        ConnectionAbort::none(),
    ));
    // The sibling: drains as lines arrive, like a healthy reader thread.
    let (sibling_tx, sibling_rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        2,
        sibling_tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    let mut received = Vec::new();
    for n in 0..=CLIENT_QUEUE_DEPTH {
        push_to_clients(&clients, format!("line-{n}"));
        while let Ok(line) = sibling_rx.try_recv() {
            received.push(line);
        }
    }

    assert_eq!(
        clients.lock().len(),
        1,
        "the stalled client was evicted; the draining sibling remains"
    );
    assert!(
        stalled_flag.load(Ordering::Relaxed),
        "eviction raised the flag the connection threads tear down on"
    );
    while let Ok(line) = sibling_rx.try_recv() {
        received.push(line);
    }
    assert_eq!(
        received.len(),
        CLIENT_QUEUE_DEPTH + 1,
        "the sibling saw every line, including the one that evicted the stalled client"
    );
    assert_eq!(received.first().map(String::as_str), Some("line-0"));
    let last = format!("line-{CLIENT_QUEUE_DEPTH}");
    assert_eq!(received.last().map(String::as_str), Some(last.as_str()));
}

/// Card 01a0d9b47dae7751a6c7e5a4a900be6e: eviction is by queue depth
/// only — intended (ARC-011's memory bound; see MUX.md's broadcast
/// eviction paragraph), and deliberately unlike tmux, which evicts a
/// control client by output age (300 s) while buffering without bound.
/// This pins the consequence of the depth policy: a client draining
/// continuously but slower than the producer — healthy, just slow — is
/// evicted once its backlog passes the cap, losing the queued tail. An
/// age-based policy replaces this test with a drains-slowly-survives
/// one.
#[test]
fn a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap() {
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = sync_channel::<String>(CLIENT_QUEUE_DEPTH);
    let flag = Arc::new(AtomicBool::new(false));
    clients
        .lock()
        .push((7, tx, Arc::clone(&flag), ConnectionAbort::none()));

    // Drains exactly one line per two pushes — continuously, but at
    // half the producer's rate, so the backlog grows ~0.5 lines per
    // push and passes the cap mid-burst.
    let burst = CLIENT_QUEUE_DEPTH * 3;
    let mut drained = 0;
    for n in 0..burst {
        push_to_clients(&clients, format!("line-{n}"));
        if n % 2 == 0 && rx.try_recv().is_ok() {
            drained += 1;
        }
    }
    // After eviction the sender is gone; the queue's remaining backlog
    // still drains, so `drained` settles just past the eviction point.
    while rx.try_recv().is_ok() {
        drained += 1;
    }

    assert!(
        flag.load(Ordering::Relaxed),
        "the slow drainer was evicted once its backlog passed the cap"
    );
    assert!(
        clients.lock().is_empty(),
        "the evicted client is no longer registered"
    );
    assert!(
        drained > CLIENT_QUEUE_DEPTH && drained < burst,
        "drained {drained} of {burst} — continuous but half-rate, evicted mid-burst"
    );
}

/// ENH-012: eviction must CLOSE the evicted client's connection, not
/// just stop queueing to it. Dropping the queue sender alone leaves the
/// client's writer thread blocked in a full socket buffer and its
/// `handle_client` thread parked in `lines()`, so the queued lines stay
/// pinned and the client never learns it was evicted (measured live:
/// daemon RSS held ~70 MiB and the stalled socket never EOF'd while the
/// flood continued; the moment it started reading, the writer resumed
/// feeding it the retained queue).
#[test]
fn an_evicted_clients_connection_closes() {
    use crate::mux::ipc::connect_local_stream;
    use std::io::{Read, Write};

    // A raw listener, not a MuxServer, so the shutdown-unlink never
    // runs for it — keep the socket inside a temp dir the harness
    // removes rather than littering the real $TMPDIR.
    let dir = temp_dir();
    let socket_path = dir.path().join("evict-close.sock");
    let listener = bind_local_listener(&socket_path).expect("binds the test listener");

    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::clone(&clients);
    let (tree, _) = harness();
    std::thread::spawn(
        move || match crate::mux::ipc::accept_connection(&listener) {
            Ok((stream, abort)) => handle_client(stream, tree, registry, None, None, abort, None),
            Err(err) => panic!("accept_connection failed: {err}"),
        },
    );

    let mut client = connect_local_stream(&socket_path).expect("connects");
    client
        .write_all(b"list-sessions\n")
        .expect("sends a command");

    // The first control command is what registers the client. The
    // deadline is load-tolerant (QA-228): it only bounds a FAILURE, so a
    // generous ceiling costs nothing on the fast path and stops a
    // parallel-load scheduler stall from failing the wait.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while clients.lock().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "client never registered"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // Overflow the queue: the CLIENT_QUEUE_DEPTH-th push fills it, the
    // next one evicts. Lines are ~1 KiB so the socket's own buffer
    // absorbs only a handful on macOS (measured 8 KiB both directions)
    // — but a larger kernel socket buffer (ubuntu CI runners) lets the
    // connection's writer drain past any fixed headroom, so push UNTIL
    // the eviction lands rather than a fixed count, bounded by a
    // deadline.
    let filler = "x".repeat(1000);
    let evicted_by = std::time::Instant::now() + Duration::from_secs(30);
    let mut n = 0usize;
    while !clients.lock().is_empty() {
        assert!(
            std::time::Instant::now() < evicted_by,
            "client never evicted after {n} flood lines"
        );
        push_to_clients(&clients, format!("flood-{n:05}-{filler}"));
        n += 1;
    }

    // The evicted client observes the connection closing: drain the
    // socket's residue, then EOF must arrive within the deadline. On
    // Unix the eviction poll (ENH-012) wakes this connection's
    // writer/reader on send/recv timeouts; on Windows eviction aborts
    // their blocked pipe I/O through the registry entry's
    // ConnectionAbort, and every server-side handle drops — including
    // the abort's and the re-canceller's — which is the EOF itself.
    {
        let (eof_tx, eof_rx) = channel::<()>();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            loop {
                match client.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => continue,
                }
            }
            let _ = eof_tx.send(());
        });
        // Load-tolerant ceiling (QA-228): the wake-and-drain normally
        // finishes within a couple of EVICTION_POLLs; under parallel
        // load the writer thread's scheduling can stretch far past the
        // old 5 s wall, so bound at 30 s instead.
        assert!(
            eof_rx.recv_timeout(Duration::from_secs(30)).is_ok(),
            "the evicted client's socket closed within 30 s of eviction"
        );
    }
    let _ = std::fs::remove_file(&socket_path);
}

/// QA-113: a panicking dispatcher is contained — the issuer gets an
/// error block, and the same "connection" answers the next command.
#[test]
fn a_panicking_command_yields_an_error_block_and_the_next_command_survives() {
    let (tree, clients) = harness();

    crate::mux::dispatch::PANIC_ON_COMMAND.with(|flag| flag.set(true));
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 1,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let poisoned = parse_command("list-sessions").expect("parses");
    let reply = dispatch_contained(poisoned, &ctx, None, None);
    assert!(
        reply.contains("%error") && reply.contains("internal error"),
        "a panicked command answers with an error block: {reply}"
    );
    crate::mux::dispatch::PANIC_ON_COMMAND.with(|flag| flag.set(false));

    // The tree lock unwound free; the next command on the same
    // connection dispatches normally.
    let ctx = Ctx {
        tree: &tree,
        clients: &clients,
        command_number: 2,
        shutdown: None,
        config: None,
        client_id: None,
    };
    let healthy = parse_command("list-sessions").expect("parses");
    let reply = dispatch_contained(healthy, &ctx, None, None);
    assert!(
        reply.contains("%end"),
        "the connection survives the panic: {reply}"
    );
}

/// A kill-pane that empties a window must BROADCAST %window-close:
/// `tree.kill_pane` closes the window (and an emptied session), but
/// the dispatch only queued a layout push for the window — which
/// resolves to nothing once the window is gone — so clients kept a
/// tab for a window that no longer existed.
#[cfg(unix)]
#[test]
fn killing_a_windows_last_pane_broadcasts_window_close() {
    let dir = temp_dir();
    let path = dir.path().join("lastpane.sock");
    let server = MuxServer::bind(&path).expect("bind");
    std::thread::spawn(move || server.run());

    let mut client = crate::mux::MuxClient::connect(&path).expect("connect");
    client.send("new-session -s t").expect("new-session");
    client.send("kill-pane -t %0").expect("kill-pane");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match client
            .notifications()
            .recv_timeout(std::time::Duration::from_millis(100))
        {
            Ok(crate::tmux_control::TmuxNotification::WindowClose { window_id }) => {
                assert_eq!(window_id, "@0");
                return;
            }
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "%window-close never arrived after kill-pane emptied the window"
                );
            }
            Err(_) => panic!("notification channel died before %window-close"),
        }
    }
}

// ---- ENH-039: per-pane hook-only endpoints ----

/// A pane tree with one pane (`%0`), built on the default factory.
fn pane_tree() -> Arc<Mutex<MuxTree>> {
    let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
    tree.new_session("t", 80, 24)
        .expect("the test session spawns");
    Arc::new(Mutex::new(tree))
}

/// The endpoint contract: a control command is refused with the
/// hook-only error and the connection closes; a hook report that omits
/// `pane_id` is filed for the bound pane; a report naming another pane
/// is refused by the binding; dropping the endpoint unlinks its socket.
#[test]
fn a_pane_endpoint_answers_hooks_and_refuses_control_commands() {
    use crate::mux::ipc::connect_local_stream;
    use std::str::FromStr;

    let dir = temp_dir();
    let control = dir.path().join("panehook.sock");
    let pane_id = PaneId::from_str("%0").unwrap();
    let tree = pane_tree();
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = pane_endpoint_channel();
    let endpoint = PaneEndpoint::bind(&control, pane_id, tx).expect("binds");

    // One accepted connection at a time: drain the endpoint's channel
    // and serve the connection off this test thread's tree.
    let serve_next = |rx: &PaneEndpointRx, tree: &Arc<Mutex<MuxTree>>, clients: &Clients| {
        let (stream, _abort, bound) = rx
            .inner
            .recv_timeout(Duration::from_secs(5))
            .expect("forwards");
        let tree = Arc::clone(tree);
        let clients = Arc::clone(clients);
        std::thread::spawn(move || serve_pane_connection(stream, bound, &tree, &clients));
    };

    // Refused: a control command on a hook-only endpoint — one JSON
    // error line, then the connection closes.
    let mut client = connect_local_stream(&endpoint.socket_path).expect("connects");
    client
        .write_all(b"capture-pane -t %0\n")
        .expect("sends the control command");
    serve_next(&rx, &tree, &clients);
    let mut reader = BufReader::new(client);
    let mut line = String::new();
    reader.read_line(&mut line).expect("reads the refusal");
    assert!(
        line.contains("hook-only endpoint"),
        "the control command is refused: {line}"
    );
    assert_eq!(
        reader.read_line(&mut String::new()).expect("eof"),
        0,
        "the connection closes after the refusal"
    );

    // Accepted: a report that omits pane_id is filed for the bound pane.
    let mut client = connect_local_stream(&endpoint.socket_path).expect("connects");
    writeln!(
        client,
        r#"{{"id":1,"method":"pane.report_agent","params":{{"agent":"claude","seq":1,"state":"working"}}}}"#
    )
    .expect("sends the report");
    serve_next(&rx, &tree, &clients);
    let mut line = String::new();
    BufReader::new(client)
        .read_line(&mut line)
        .expect("reads the ok reply");
    assert!(line.contains("\"result\":\"ok\""), "{line}");
    assert_eq!(
        tree.lock()
            .pane(pane_id)
            .expect("pane")
            .metadata()
            .get("agent_state")
            .map(String::as_str),
        Some("working"),
        "the pane-less report landed on the bound pane"
    );

    // Refused by the binding: a report naming another pane.
    let mut client = connect_local_stream(&endpoint.socket_path).expect("connects");
    writeln!(
        client,
        r#"{{"id":2,"method":"pane.report_agent","params":{{"pane_id":"%9","agent":"claude","seq":2,"state":"idle"}}}}"#
    )
    .expect("sends the cross-pane report");
    serve_next(&rx, &tree, &clients);
    let mut line = String::new();
    BufReader::new(client)
        .read_line(&mut line)
        .expect("reads the binding refusal");
    assert!(
        line.contains("pane_id does not match this endpoint"),
        "{line}"
    );

    // Dropping the endpoint unlinks its socket file.
    let socket_path = endpoint.socket_path.clone();
    drop(endpoint);
    assert!(
        !socket_path.exists(),
        "the endpoint socket is unlinked on drop"
    );
}

/// The startup sweep reclaims a stale `*.pane-*.sock` remnant and
/// leaves a bound listener alone.
#[cfg(unix)]
#[test]
fn the_startup_sweep_removes_a_stale_pane_endpoint_remnant() {
    use std::str::FromStr;

    let dir = temp_dir();
    let control = dir.path().join("sweeptest.sock");
    let stale = pane_endpoint_path(&control, PaneId::from_str("%7").unwrap());
    std::fs::write(&stale, b"stale remnant").expect("creates the remnant");
    let live_path = pane_endpoint_path(&control, PaneId::from_str("%8").unwrap());
    let live = bind_local_listener(&live_path).expect("binds the live endpoint");
    sweep_pane_endpoint_remnants(&control);
    assert!(!stale.exists(), "the stale remnant is reclaimed");
    assert!(live_path.exists(), "a bound endpoint is not touched");
    drop(live);
}

/// A pane endpoint beside a control socket whose sibling path exceeds
/// the platform socket-address limit fails the bind — the factory then
/// exports NO socket for the pane rather than falling back to the full
/// control socket.
#[cfg(unix)]
#[test]
fn an_over_long_pane_endpoint_path_fails_the_bind() {
    use std::str::FromStr;

    let dir = temp_dir();
    let long = dir.path().join(format!("{}.sock", "x".repeat(200)));
    let (tx, _rx) = pane_endpoint_channel();
    let result = PaneEndpoint::bind(&long, PaneId::from_str("%0").unwrap(), tx);
    assert!(
        result.is_err(),
        "an over-long pane socket path must fail the bind"
    );
}

// --- Workspaces (dispatch level) ---

#[test]
fn workspace_commands_dispatch_and_broadcast() {
    let (tree, clients) = quiet_harness();
    let (tx, rx) = sync_channel(CLIENT_QUEUE_DEPTH);
    clients.lock().push((
        u64::MAX,
        tx,
        Arc::new(AtomicBool::new(false)),
        ConnectionAbort::none(),
    ));

    // new-workspace replies with the id, selects it, and cues the roster.
    let reply = dispatch("new-workspace -n dev", 1, &tree, &clients, None);
    assert!(reply.contains("+0"), "reply carries the id: {reply}");
    // Its auto-created first tab is named "1", not after the workspace;
    // the session keeps the workspace's name.
    {
        let guard = tree.lock();
        let session = guard.sessions()[0];
        assert_eq!(guard.session(session).unwrap().name, "dev");
        let window = guard.session(session).unwrap().windows[0];
        assert_eq!(guard.window(window).unwrap().name, "1");
    }
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%workspaces-changed")),
        "new-workspace must cue the roster: {lines:?}"
    );

    // Sessions land in the active workspace; list-sessions carries the
    // workspace prefix.
    dispatch("new-session -s svc", 2, &tree, &clients, None);
    let reply = dispatch("list-sessions", 3, &tree, &clients, None);
    let body = reply
        .lines()
        .find(|l| l.contains("svc") && !l.starts_with('%'))
        .expect("a session line");
    // new-workspace auto-spawns the workspace's first session ($0,
    // named after the workspace), so an explicit new-session lands as
    // $1.
    assert_eq!(body, "+0: dev: $1: svc", "the documented line shape");

    // The workspace filter restricts the listing; a wrong one errors.
    dispatch("new-workspace -n other", 4, &tree, &clients, None);
    dispatch("new-session -t other -s side", 5, &tree, &clients, None);
    let reply = dispatch("list-sessions -t +0", 6, &tree, &clients, None);
    assert!(reply.contains("+0: dev: $1: svc"));
    assert!(!reply.contains("side"), "filtered to workspace +0");
    let reply = dispatch("list-sessions -t other", 7, &tree, &clients, None);
    assert!(reply.contains("side"));

    // list-workspaces renders `+N: name` with the active marker.
    let reply = dispatch("list-workspaces", 8, &tree, &clients, None);
    assert!(
        reply.contains("+0: dev") && reply.contains("+1: other active"),
        "active marker rides the active workspace line: {reply}"
    );

    // select-workspace cues the roster (selection is daemon state).
    let reply = dispatch("select-workspace -t +0", 9, &tree, &clients, None);
    assert!(!reply.contains("%error"));
    assert_eq!(tree.lock().active_workspace(), Some(WorkspaceId(0)));
    assert!(drain_broadcasts(&rx)
        .iter()
        .any(|l| l.starts_with("%workspaces-changed")));

    // rename-workspace.
    dispatch("rename-workspace -t +0 prod", 10, &tree, &clients, None);
    assert_eq!(tree.lock().workspace(WorkspaceId(0)).unwrap().name, "prod");
    let _ = drain_broadcasts(&rx);

    // kill-workspace takes the sessions' windows with it and cues BOTH
    // rosters.
    dispatch("kill-workspace -t prod", 11, &tree, &clients, None);
    {
        let guard = tree.lock();
        assert!(guard.workspace(WorkspaceId(0)).is_none());
        // 'other' carries its auto-spawned first session plus 'side'.
        assert_eq!(guard.sessions().len(), 2, "only 'other' survives");
    }
    let lines = drain_broadcasts(&rx);
    let closes = lines
        .iter()
        .filter(|l| l.starts_with("%window-close"))
        .count();
    assert!(closes >= 1, "one per killed window: {lines:?}");
    let sessions = lines
        .iter()
        .position(|l| l.starts_with("%sessions-changed"))
        .expect("session roster cue");
    let workspaces = lines
        .iter()
        .position(|l| l.starts_with("%workspaces-changed"))
        .expect("workspace roster cue");
    assert!(sessions < workspaces, "session cue precedes workspace cue");

    // A workspace holding exactly one session — new-workspace's
    // auto-spawned first session — then killing that session via
    // kill-session removes the emptied workspace — and cues the
    // workspace roster.
    dispatch("new-workspace -n transient", 12, &tree, &clients, None);
    let _ = drain_broadcasts(&rx);
    dispatch("kill-session -t transient", 14, &tree, &clients, None);
    let lines = drain_broadcasts(&rx);
    assert!(
        lines.iter().any(|l| l.starts_with("%workspaces-changed")),
        "a session death that empties its workspace cues the workspace roster: {lines:?}"
    );
    {
        let guard = tree.lock();
        assert!(guard.workspace(WorkspaceId(2)).is_none());
        assert_eq!(guard.workspaces().len(), 1, "'other' survives");
    }
}
