//! The daemon requirement: panes outlive their clients, and two starts race safely.

#![cfg(feature = "mux")]

mod common;

use common::{command, pane_ids, wait_listening, MuxFixture};
use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::{connect_local_stream, prepare_socket_path, MuxServer};
use std::io::BufReader;
use std::time::{Duration, Instant};

#[test]
fn panes_survive_every_client_disconnecting() {
    let fixture = MuxFixture::new("survive");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());

    // First client: create a session, then disconnect entirely.
    let pane_line = {
        let stream = connect_local_stream(path).expect("first connect");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s survive");
        let listed = command(&mut writer, &mut reader, "list-panes");
        listed.join("")
        // Both ends drop here — the client is gone.
    };
    assert!(pane_line.contains('%'), "a pane was created: {pane_line}");

    // The requirement is that the server is STILL listening after its last
    // client left. Poll the socket (bounded) instead of sleeping a guessed
    // 300ms: if the server wrongly exited on the disconnect, the connect
    // below fails and the test fails, on fast and slow machines alike.
    wait_listening(path);

    // Second client: the session must still be there.
    let stream = connect_local_stream(path).expect(
        "server must still be listening after its last client left — this is the \
         whole requirement: par-term restarting must not kill sessions",
    );
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    let listed = command(&mut writer, &mut reader, "list-panes").join("");
    assert!(
        listed.contains('%'),
        "the pane created by the first client must still exist: {listed}"
    );

    drop(writer);
    let _ = handle;
}

/// Pane titles: `select-pane -T` sets a user title, every connected client
/// is told via `%pane-title-changed`, a client that attaches AFTER the set
/// reads it back through the `pane-title` query, and `-T ''` clears with
/// the same broadcast.
#[test]
fn pane_titles_broadcast_query_and_clear() {
    let fixture = MuxFixture::new("titles");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    wait_listening(path);

    // Client A creates the session; client B attaches BEFORE any title
    // change (a push is never replayed to a late client — that is what
    // the query below is for); then A sets a quoted, spaced title.
    let stream_a = connect_local_stream(path).expect("client A connects");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s titles");
    let pane = pane_ids(&command(&mut writer_a, &mut reader_a, "list-panes").join(""))
        .first()
        .expect("a pane exists")
        .clone();

    let stream_b = connect_local_stream(path).expect("client B connects");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    // A connection joins the broadcast set on its FIRST control command
    // (the registration model in handle_client), so B establishes itself
    // before the change it must observe.
    command(&mut writer_b, &mut reader_b, "list-panes");

    command(
        &mut writer_a,
        &mut reader_a,
        &format!("select-pane -t {pane} -T 'My build pane'"),
    );

    // Client B polls a benign command and scans everything it reads for
    // the broadcast: pushed lines sit in the socket buffer and drain ahead
    // of the next reply block, so a poll loop with no sleep-ordering
    // assumption is the honest shape.
    let set_line = format!("%pane-title-changed {pane} My build pane");
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        seen.push_str(&command(&mut writer_b, &mut reader_b, "list-panes").join(""));
        if seen.lines().any(|l| l == set_line) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "client B never saw {set_line:?}; read so far: {seen}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    // A client attaching AFTER the set reads the title through the query.
    let stream_c = connect_local_stream(path).expect("client C connects");
    let mut writer_c = stream_c.try_clone().expect("clone");
    let mut reader_c = BufReader::new(stream_c);
    let queried = command(
        &mut writer_c,
        &mut reader_c,
        &format!("pane-title -t {pane}"),
    )
    .join("");
    assert!(
        queried.lines().any(|l| l.trim() == "My build pane"),
        "the query returns the user title: {queried}"
    );

    // Clearing broadcasts the empty-title form of the same line.
    command(
        &mut writer_a,
        &mut reader_a,
        &format!("select-pane -t {pane} -T ''"),
    );
    let clear_line = format!("%pane-title-changed {pane}");
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        seen.push_str(&command(&mut writer_b, &mut reader_b, "list-panes").join(""));
        if seen.lines().any(|l| l == clear_line) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "client B never saw the clear line {clear_line:?}; read: {seen}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let after = command(
        &mut writer_c,
        &mut reader_c,
        &format!("pane-title -t {pane}"),
    )
    .join("");
    assert!(
        !after.contains("My build pane"),
        "the cleared title no longer reports: {after}"
    );

    drop(writer_a);
    drop(writer_b);
    drop(writer_c);
    let _ = handle;
}

#[test]
fn prepare_socket_path_makes_auto_spawn_race_safe() {
    let fixture = MuxFixture::new("race");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("first bind wins");
    let _handle = std::thread::spawn(move || server.run());

    // Wait for the listener to be reachable.
    wait_listening(path);

    // A second would-be daemon must refuse rather than steal the path.
    let err =
        prepare_socket_path(path).expect_err("a second daemon must not reclaim a live socket");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
}

/// D3.5, Task 3.4: a server bound to a restored tree serves the prior pane
/// ids and the prior screen + scrollback, with a fresh process behind them.
/// The content pane runs `sleep` on both sides of the save/restore, so the
/// only bytes in its terminal are the ones the test fed directly.
#[cfg(unix)]
#[test]
fn a_restored_tree_serves_its_prior_ids_and_content() {
    use par_term_emu_core_rust::mux::pane::ShellPaneFactory;
    use par_term_emu_core_rust::mux::persist::{load_or_quarantine, save_to, Loaded};
    use par_term_emu_core_rust::mux::tree::MuxTree;

    let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
    tree.new_session("main", 80, 24).expect("session creates");
    let window = tree.sessions()[0];
    let window = tree.session(window).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];
    let quiet = tree
        .split_pane(
            first,
            par_term_emu_core_rust::mux::SplitDirection::Vertical,
            0.5,
            Some("sleep 60"),
        )
        .expect("quiet pane splits");
    {
        let terminal = tree.pane(quiet).unwrap().terminal();
        let mut guard = terminal.write();
        for i in 0..40 {
            guard.process(format!("scroll line {i:02}\r\n").as_bytes());
        }
        guard.process(b"ZQX-RESTORED-SCREEN");
    }

    // Save → load → rebuild is the daemon restart, minus the process
    // boundary; bind_with_tree is what startup does with the result.
    let state_dir = tempfile::Builder::new()
        .prefix("par-mux-")
        .tempdir()
        .expect("create state temp dir");
    let target = state_dir.path().join("restore.state.json");
    save_to(&tree, &target).expect("save");
    let state = match load_or_quarantine(&target) {
        Loaded::State(state) => state,
        other => panic!("expected a readable state file, got {other:?}"),
    };
    let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("state rebuilds");

    let fixture = MuxFixture::new("restore");
    let path = fixture.socket();
    let server = MuxServer::bind_with_tree(path, restored).expect("bind");
    std::thread::spawn(move || server.run());

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);

    let listed = command(&mut writer, &mut reader, "list-panes").join("");
    assert!(
        listed.contains(&quiet.to_string()),
        "the restored pane keeps its id {}: {listed}",
        quiet
    );

    let screen = command(
        &mut writer,
        &mut reader,
        &format!("refresh-client -t {quiet}"),
    )
    .join("");
    assert!(
        screen.contains("ZQX-RESTORED-SCREEN"),
        "the restored pane shows its prior screen: {screen}"
    );

    let history = command(
        &mut writer,
        &mut reader,
        &format!("capture-pane -t {quiet} -S -40"),
    )
    .join("");
    assert!(
        history.contains("scroll line 05"),
        "the restored pane keeps its scrollback: {history}"
    );
}

/// Task 3.5: a clean SIGTERM stops the daemon with a final save — the state
/// file reflects every completed mutation, so the next start restores from
/// it rather than starting empty.
#[cfg(unix)]
#[test]
fn sigterm_saves_state_on_the_way_out() {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    use par_term_emu_core_rust::mux::persist::{load_or_quarantine, Loaded};

    let fixture = MuxFixture::new("sigterm");
    let path = fixture.socket();
    let state_path = fixture.state_path();

    let mut child = common::spawn_daemon(&fixture);

    // Wait for the listener, then drive two structural mutations.
    wait_listening(path);
    let stream = connect_local_stream(path).expect("daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s sig");
    let listed = command(&mut writer, &mut reader, "list-panes").join("");
    // Body lines sit between %begin/%end; a pane id is '%' followed by a digit.
    let pane = listed
        .lines()
        .find(|l| {
            l.trim_start()
                .strip_prefix('%')
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .expect("a pane was created")
        .trim()
        .to_string();
    command(
        &mut writer,
        &mut reader,
        &format!("split-window -t {pane} -h"),
    );

    // Content AFTER the last structural mutation: send-keys does not trigger
    // a per-mutation save, so only the shutdown save can capture it.
    command(
        &mut writer,
        &mut reader,
        &format!("send-keys -t {pane} 'echo ZQX-SHUTDOWN-MARKER' Enter"),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let screen = command(
            &mut writer,
            &mut reader,
            &format!("refresh-client -t {pane}"),
        )
        .join("");
        if screen.contains("ZQX-SHUTDOWN-MARKER") || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    drop((writer, reader));

    signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).expect("SIGTERM delivered");
    let status = child.wait().expect("daemon exits");
    assert!(
        status.success(),
        "a clean SIGTERM exits 0 after saving, got {status:?}"
    );

    match load_or_quarantine(&state_path) {
        Loaded::State(state) => {
            assert_eq!(state.sessions.len(), 1, "the session survived");
            assert_eq!(
                state.sessions[0].windows[0].panes.len(),
                2,
                "the split survived"
            );
            // The marker was typed after the split's per-mutation save, so
            // its presence proves the SHUTDOWN save ran. Cells serialize as
            // per-character objects, so the text must be reassembled from
            // the JSON value rather than searched for as a substring.
            let value = serde_json::to_value(&state).expect("state serializes");
            let screen_text: String = value["sessions"][0]["windows"][0]["panes"][0]["terminal"]
                ["grid"]["cells"]
                .as_array()
                .expect("cells are an array")
                .iter()
                .filter_map(|cell| cell["c"].as_str())
                .collect();
            assert!(
                screen_text.contains("ZQX-SHUTDOWN-MARKER"),
                "the shutdown save captured content typed after the last structural save; screen: {screen_text:?}"
            );
        }
        other => panic!("expected the shutdown save, got {other:?}"),
    }
}

/// A pane whose child exits is reaped daemon-side (tmux semantics: a pane
/// dies with its process) and clients are told — `%layout-change` for a
/// surviving window, `%window-close` when the dead pane was the last one.
/// Before the reaper, the pane sat dead in the tree forever and clients
/// stared at a frozen pane (`exit` in par-term stuck exactly there).
#[test]
fn a_pane_whose_child_exits_is_reaped_and_broadcast() {
    let fixture = MuxFixture::new("reap");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s reap");
    command(&mut writer, &mut reader, "split-window -h -t %0");

    // Exit the FIRST pane's shell; the reaper must close it within a bound.
    command(&mut writer, &mut reader, "send-keys -t %0 'exit' Enter");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut reaped = false;
    while Instant::now() < deadline && !reaped {
        let listed = command(&mut writer, &mut reader, "list-panes").join(" ");
        reaped = pane_ids(&listed).len() == 1;
        if !reaped {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert!(
        reaped,
        "the dead pane must leave the tree within 10s of its child exiting"
    );

    // The LAST pane exiting closes the window instead.
    command(&mut writer, &mut reader, "send-keys -t %1 'exit' Enter");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut closed = false;
    while Instant::now() < deadline && !closed {
        let windows = command(&mut writer, &mut reader, "list-windows").join(" ");
        // The reply block itself always carries %begin/%end lines — the
        // window set is empty when no @N id appears in it.
        closed = !windows.contains('@');
        if !closed {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let final_panes = command(&mut writer, &mut reader, "list-panes").join(" ");
    let final_windows = command(&mut writer, &mut reader, "list-windows").join(" ");
    assert!(
        closed,
        "the window must close when its last pane's child exits — panes:          {final_panes:?}, windows: {final_windows:?}"
    );
}

/// `kill-server` stops the daemon through the same path SIGTERM takes: the
/// reply block answers first, the accept loop exits, and the final state
/// save captures the tree — which is what `par-mux --stop` / `--restart`
/// rely on (restart restores exactly what the stop saved).
#[test]
fn kill_server_stops_the_daemon_after_a_final_save() {
    let fixture = MuxFixture::new("killsrv");
    let path = fixture.socket().to_path_buf();
    let state_dir = tempfile::Builder::new()
        .prefix("par-mux-killsrv-")
        .tempdir()
        .expect("state dir");
    let state_path = par_term_emu_core_rust::mux::persist::state_file_in(state_dir.path(), &path);
    let server = MuxServer::bind(&path).expect("bind");
    let run_state = state_path.clone();
    let handle = std::thread::spawn(move || server.run_persisting(run_state));

    let stream = connect_local_stream(&path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s killsrv");
    command(&mut writer, &mut reader, "split-window -h -t %0");
    let reply = command(&mut writer, &mut reader, "kill-server").join(" ");
    assert!(
        !reply.contains("%error"),
        "kill-server must succeed on a running server: {reply}"
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "the server never stopped");
        std::thread::sleep(Duration::from_millis(50));
    }
    handle.join().expect("server thread");

    let saved = std::fs::read_to_string(&state_path).expect("final save written");
    let state: serde_json::Value = serde_json::from_str(&saved).expect("valid state json");
    let panes: usize = state["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .flat_map(|s| s["windows"].as_array().cloned().unwrap_or_default())
        .map(|w| w["panes"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(panes, 2, "the final save captured both panes: {saved:.200}");
}

/// A split's new pane must push `%output` like every other pane. The
/// split handler once created the pane without wiring its output sink: the
/// daemon grid filled (capture-pane showed it) but no `%output` line ever
/// left, so every client rendered the new split pane blank.
#[test]
fn a_split_pane_pushes_its_output_to_clients() {
    let fixture = MuxFixture::new("splitout");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_term_emu_core_rust::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s splitout").expect("new-session");
    let reply = client
        .send("split-window -h -t %0")
        .expect("split")
        .join("");
    let new_pane = reply.trim().to_string();
    assert!(
        new_pane.starts_with('%'),
        "split replies the new pane id: {reply}"
    );

    // Case-folded marker: the typed command's echo carries the uppercase
    // form only, so only real output satisfies the wait.
    client
        .send(&format!(
            "send-keys -t {new_pane} 'echo SPLIT-OUT | tr A-Z a-z' Enter"
        ))
        .expect("send-keys");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw = false;
    while Instant::now() < deadline && !saw {
        if let Ok(par_term_emu_core_rust::tmux_control::TmuxNotification::Output {
            pane_id,
            data,
        }) = client
            .notifications()
            .recv_timeout(Duration::from_millis(250))
        {
            saw = pane_id == new_pane && String::from_utf8_lossy(&data).contains("split-out");
        }
    }
    assert!(
        saw,
        "the split's new pane {new_pane} must push its output as %output"
    );
}

/// The `version` command: a daemon answers with its build stamp — one line,
/// exactly `mux::build_stamp()`, with no session state involved — so a
/// client can compare the daemon's core build against its own and surface a
/// stale daemon instead of silently missing daemon-side fixes.
#[test]
fn version_reports_the_daemon_build_stamp() {
    let fixture = MuxFixture::new("version");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    let reply = command(&mut writer, &mut reader, "version").join("");

    let expected = par_term_emu_core_rust::mux::build_stamp();
    assert!(
        reply.contains(expected),
        "the daemon must answer `version` with its build stamp {expected:?}: {reply}"
    );
    // One body line, not a block of diagnostics — clients parse it as the
    // whole identity.
    let body = reply
        .lines()
        .filter(|l| !l.starts_with("%begin") && !l.starts_with("%end"))
        .count();
    assert_eq!(body, 1, "the stamp is exactly one line: {reply}");

    drop(writer);
    let _ = handle;
}

/// The auto-spawned daemon must leave the spawner's session (the gap audit
/// behind card 01a0d9b2f53e killed real daemons this way: a SIGHUP to the
/// spawner's process group took the daemon and every pane with it, no
/// save). `setsid` in `spawn_daemon` puts it in a fresh session with no
/// controlling tty, so terminal-generated signals can never reach it —
/// asserted here as: the daemon leads its own session, that session is not
/// the test's, and it still serves.
#[cfg(unix)]
#[test]
fn the_auto_spawned_daemon_runs_in_its_own_session() {
    use nix::unistd::{getpgid, getpgrp, getsid, Pid};
    use par_term_emu_core_rust::mux::MuxClient;

    let fixture = MuxFixture::new("setsid");
    let mut client =
        MuxClient::connect_or_spawn_at(fixture.socket()).expect("the daemon spawns and serves");
    let daemon = client
        .spawned_daemon_pid()
        .expect("this client started its daemon");
    let daemon = Pid::from_raw(daemon as i32);

    let session = getsid(Some(daemon)).expect("the daemon's session");
    let group = getpgid(Some(daemon)).expect("the daemon's process group");
    assert_eq!(
        session.as_raw(),
        daemon.as_raw(),
        "setsid ran: the daemon leads its own session"
    );
    assert_ne!(
        session.as_raw(),
        getsid(None).expect("the test's own session").as_raw(),
        "the daemon left the spawner's session"
    );
    assert_ne!(
        group.as_raw(),
        getpgrp().as_raw(),
        "the daemon left the spawner's process group"
    );

    let reply = client
        .send_checked("list-sessions")
        .expect("the detached daemon still serves");
    assert!(reply.ok, "list-sessions succeeds: {:?}", reply.body);

    client.kill_spawned_daemon().expect("teardown kill");
}

/// Terminal-generated signals must not stop a serving daemon (tmux ignores
/// the same set on its server): SIGHUP from a closing terminal, SIGINT and
/// SIGQUIT from Ctrl-C / Ctrl-\, SIGTSTP from Ctrl-Z, SIGPIPE from a client
/// socket closing mid-write. SIGTERM stays the clean shutdown path, re-proven
/// by the teardown.
#[cfg(unix)]
#[test]
fn terminal_signals_leave_the_daemon_serving() {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    let fixture = MuxFixture::new("sigignore");
    let mut child = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let stream = connect_local_stream(fixture.socket()).expect("daemon accepts");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    command(&mut writer, &mut reader, "new-session -s live");

    let daemon = Pid::from_raw(child.id() as i32);
    for sig in [
        Signal::SIGHUP,
        Signal::SIGINT,
        Signal::SIGQUIT,
        Signal::SIGTSTP,
        Signal::SIGPIPE,
    ] {
        signal::kill(daemon, sig).unwrap_or_else(|e| panic!("{sig:?} delivered: {e}"));
        // A daemon that died sees EOF (command's own assert) or stops
        // answering (the read blocks); either way this line is the failure.
        let reply = command(&mut writer, &mut reader, "list-sessions").join("");
        assert!(
            reply.contains("live"),
            "the daemon still serves after {sig:?}: {reply}"
        );
    }
    drop((writer, reader));

    common::sigterm_clean(&mut child);
}

/// `--restart` must leave a daemon that survives the terminal it was typed
/// into: it detaches before serving (fork + setsid + stdio to /dev/null,
/// tmux's daemon(1,0) shape), so the invocation itself returns immediately
/// and the serving process leads its own session. Unfixed, `--restart &`
/// stays in the shell's job and closing that terminal kills every pane.
#[cfg(unix)]
#[test]
fn restart_detaches_before_serving() {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    use std::process::{Command, Stdio};

    let fixture = MuxFixture::new("restart-detach");

    // The parent half: must exit promptly and successfully.
    let mut restart = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .arg("--restart")
        .env_remove("PAR_MUX_ENV")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("--restart spawns");
    // Bounded: unfixed, --restart serves in this process forever, and a bare
    // wait() would hang the suite instead of reporting the failure.
    let deadline = Instant::now() + Duration::from_secs(15);
    let exited = loop {
        match restart.try_wait().expect("--restart is waitable") {
            Some(status) => break status,
            None if Instant::now() > deadline => {
                let _ = restart.kill();
                panic!("--restart never returned; it must detach instead of serving in-process")
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert!(
        exited.success(),
        "--restart reports success from its parent half: {exited:?}"
    );

    // The serving half: a detached grandchild owns the socket.
    let deadline = Instant::now() + Duration::from_secs(15);
    while connect_local_stream(fixture.socket()).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let stream = connect_local_stream(fixture.socket()).expect("the detached daemon serves");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    let reply = command(&mut writer, &mut reader, "list-sessions").join("");
    assert!(!reply.contains("%error"), "fresh daemon serves: {reply}");
    drop((writer, reader));

    // Detach proof: find the serving process by its unique socket path (ps
    // only locates the pid — macOS `ps -o sess` reports 0 for every process,
    // so the session facts come from the getsid/getpgid syscalls instead),
    // then assert it leads its own session and is neither the exited
    // parent's pid nor the test's session.
    let mut daemon_pid = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon_pid.is_none() && Instant::now() < deadline {
        let ps = Command::new("ps")
            .args(["-ww", "-axo", "pid=,args="])
            .output()
            .expect("ps runs");
        let needle = fixture.socket().to_string_lossy().to_string();
        for line in String::from_utf8_lossy(&ps.stdout).lines() {
            let Some(pid) = line.split_whitespace().next() else {
                continue;
            };
            if !line.contains("par-mux") || !line.contains(&needle) {
                continue;
            }
            let pid: i32 = pid.parse().unwrap();
            assert_ne!(
                pid,
                restart.id() as i32,
                "the server is not the exited --restart parent"
            );
            daemon_pid = Some(pid);
            break;
        }
        if daemon_pid.is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let Some(daemon_pid) = daemon_pid else {
        panic!(
            "no serving par-mux process found for {}",
            fixture.socket().display()
        );
    };
    let daemon_pid = Pid::from_raw(daemon_pid);
    let session = nix::unistd::getsid(Some(daemon_pid)).expect("the serving process's session");
    assert_eq!(
        session.as_raw(),
        daemon_pid.as_raw(),
        "setsid ran: the serving process leads its own session"
    );
    assert_ne!(
        session.as_raw(),
        nix::unistd::getsid(None)
            .expect("the test's session")
            .as_raw(),
        "the serving process left the invoker's session"
    );
    assert_ne!(
        nix::unistd::getpgid(Some(daemon_pid))
            .expect("the serving process's group")
            .as_raw(),
        nix::unistd::getpgrp().as_raw(),
        "the serving process left the invoker's process group"
    );

    // Teardown: the documented stop path, which also waits for the exit.
    let status = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--stop")
        .env_remove("PAR_MUX_ENV")
        .status()
        .expect("--stop runs");
    assert!(
        status.success(),
        "--stop reaps the detached daemon: {status:?}"
    );
    // The stopped pid must be gone (bounded probe; init reaps the orphan).
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if signal::kill(daemon_pid, Some(Signal::SIGCONT)).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// EMFILE on accept must not kill the daemon (card 01a0d9b47393: unfixed, a
/// `ulimit -n 48` daemon plus 40 raw connections ended it — no log, no
/// final save, stale socket left). tmux's server pauses accepting on
/// ENFILE/EMFILE and retries; here the daemon runs with a 48-descriptor
/// table, silent raw connections exhaust it, and the daemon must keep
/// answering an already-accepted client.
#[cfg(unix)]
#[test]
fn accept_emfile_keeps_the_daemon_serving() {
    use par_term_emu_core_rust::mux::MuxClient;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let fixture = MuxFixture::new("emfile");
    let mut command = Command::new(env!("CARGO_BIN_EXE_par-mux"));
    command
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .env_remove("PAR_MUX_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The audit's live shape: enough descriptors to boot and serve, few
    // enough that a burst of silent clients exhausts the table.
    unsafe {
        command.pre_exec(|| {
            let mut limit: libc::rlimit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            limit.rlim_cur = limit.rlim_max.min(48);
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = common::DaemonGuard::wrap(command.spawn().expect("low-limit daemon spawns"));
    wait_listening(fixture.socket());
    let mut client = MuxClient::connect(fixture.socket()).expect("daemon accepts");
    client
        .send_checked("new-session -s emfile")
        .expect("session created");

    // Exhaust: each silent connection is accepted and holds its descriptor
    // (a handler thread blocks reading it forever). Client-side failures
    // (backlog full once the daemon stops accepting) are simply skipped.
    let mut held = Vec::new();
    for _ in 0..80 {
        if let Ok(stream) = connect_local_stream(fixture.socket()) {
            held.push(stream);
        }
    }
    assert!(!held.is_empty(), "the exhaustion connections started");

    // Settle: the daemon must drain the backlog into accepts before the
    // limit bites. Pre-fix, EMFILE breaks the accept loop and the daemon
    // exits within this window (observed via BrokenPipe on the established
    // client); post-fix it backs off and the deadline simply expires.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().expect("daemon is waitable").is_some() {
            panic!("the daemon died under descriptor exhaustion; accept must back off on EMFILE");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let reply = client
        .send_checked("list-sessions")
        .expect("the daemon still answers with a full descriptor table");
    assert!(reply.ok, "list-sessions succeeds: {:?}", reply.body);
    assert!(
        reply.body.iter().any(|line| line.contains("emfile")),
        "the session is listed: {:?}",
        reply.body
    );

    drop(held);
    drop(client);
    common::sigterm_clean(&mut child);
}

/// SEC-104: a control line over the 1 MiB budget is answered with one
/// `%error` block and the connection is closed — and the daemon keeps
/// serving other clients afterwards. The budget check fires before the
/// newline arm, so this complete-but-oversized line covers the
/// unterminated-growth case too.
#[test]
fn oversized_control_line_gets_error_and_close() {
    use std::io::{BufRead, Write as _};

    let fixture = MuxFixture::new("sec104");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    wait_listening(path);

    {
        let stream = connect_local_stream(path).expect("connect");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);

        let oversized = format!("send-keys -l {}\n", "x".repeat(1024 * 1024 + 64));
        // The daemon stops reading once the budget trips, so the tail of the
        // write may hit a closed socket — that teardown is expected, and the
        // reply below is the assertion that matters.
        let _ = writer.write_all(oversized.as_bytes());
        let _ = writer.flush();

        let mut reply = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    reply.push_str(&line);
                    if line.starts_with("%error") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        assert!(
            reply.contains("%error"),
            "an over-budget line must be answered with an %error block: {reply}"
        );
        assert!(
            reply.contains("1 MiB"),
            "the error must name the budget: {reply}"
        );
    }

    // The daemon is unharmed: a fresh client gets a normal reply.
    wait_listening(path);
    let stream = connect_local_stream(path).expect("second connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    let listed = command(&mut writer, &mut reader, "list-panes").join("");
    assert!(
        listed.contains('%'),
        "the daemon must keep serving other clients: {listed}"
    );

    drop(writer);
    drop(reader);
    let _ = handle;
    drop(fixture);
}
