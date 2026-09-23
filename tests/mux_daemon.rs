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
