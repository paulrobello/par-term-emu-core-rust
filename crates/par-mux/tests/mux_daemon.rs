//! The daemon requirement: panes outlive their clients, and two starts race safely.

#![cfg(feature = "mux")]

// ARC-106: cargo sets CARGO_BIN_EXE_par-mux even when the bin's
// required-features are unmet, so a plain-`mux` build would silently exec a
// stale target/debug/par-mux. Fail loudly instead.
#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{command, pane_ids, wait_listening, MuxFixture};
use interprocess::TryClone as _;
use par_mux::mux::{connect_local_stream, prepare_socket_path, MuxServer};
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
    use par_mux::mux::pane::ShellPaneFactory;
    use par_mux::mux::persist::{load_or_quarantine, save_to, Loaded};
    use par_mux::mux::tree::MuxTree;

    let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
    tree.new_session("main", 80, 24).expect("session creates");
    let window = tree.sessions()[0];
    let window = tree.session(window).unwrap().windows[0];
    let first = tree.window(window).unwrap().panes()[0];
    let quiet = tree
        .split_pane(
            first,
            par_mux::mux::SplitDirection::Vertical,
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
    use par_mux::mux::persist::{load_or_quarantine, Loaded};

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

/// A pane whose child exits is HELD (remain-on-exit), not removed: the
/// reaper announces `%pane-exited` with the exit code, the pane keeps its
/// id, window, and frozen screen, and `respawn-pane` restarts the
/// process in place — refusing a live pane without `-k`, restarting it
/// with `-k`, and restarting a dead pane with no flag at all.
#[test]
fn a_dead_pane_is_held_announced_and_respawnable() {
    let fixture = MuxFixture::new("reap");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s reap").expect("new-session");

    // A live pane refuses the restart without -k.
    let refused = client.send("respawn-pane -t %0").expect("refuse").join("");
    assert!(
        refused.contains("still running"),
        "a live pane must refuse respawn-pane without -k: {refused}"
    );
    // With -k it restarts in place — the respawned cue arrives.
    client.send("respawn-pane -t %0 -k").expect("restart");
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneRespawned { pane_id }
                if pane_id == "%0"
        )
    });
    // Flush the restarted shell's own output before the death wait.
    while client
        .notifications()
        .recv_timeout(Duration::from_millis(200))
        .is_ok()
    {}

    // Exit the shell with a code: the pane is HELD and announced.
    client.send("send-keys -t %0 'exit 7' Enter").expect("exit");
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneExited {
                pane_id,
                exit_code: Some(7),
            } if pane_id == "%0"
        )
    });
    let listed = client.send("list-panes").expect("list").join("\n");
    assert!(
        listed.contains("%0"),
        "the dead pane stays in the tree for respawn: {listed}"
    );

    // A dead pane restarts with no flag; the new shell answers.
    client.send("respawn-pane -t %0").expect("respawn");
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneRespawned { pane_id }
                if pane_id == "%0"
        )
    });
    // Case-folded marker: the typed command's echo carries the uppercase
    // form only, so only real output satisfies the wait.
    client
        .send("send-keys -t %0 'echo RESPAWNED-OUT | tr A-Z a-z' Enter")
        .expect("send-keys");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw = false;
    while Instant::now() < deadline && !saw {
        if let Ok(par_term_emu_core::tmux_control::TmuxNotification::Output { pane_id, data }) =
            client
                .notifications()
                .recv_timeout(Duration::from_millis(250))
        {
            saw = pane_id == "%0" && String::from_utf8_lossy(&data).contains("respawned-out");
        }
    }
    assert!(
        saw,
        "the restarted pane's process must produce output again"
    );
}

/// remain-on-exit off (the daemon config default): a pane whose child exits
/// is auto-REMOVED through the kill-pane contract — `%pane-exited` queues
/// ahead of the geometry, the pane leaves `list-panes`, the window (and the
/// session, emptied) cascade, and `respawn-pane` answers "no such pane"
/// (the pane is gone, not held).
#[test]
fn the_auto_remove_default_deletes_a_dead_pane() {
    let fixture = MuxFixture::new("autoremove");
    let _daemon = common::spawn_daemon_auto_remove(&fixture);
    wait_listening(fixture.socket());
    let stream = connect_local_stream(fixture.socket()).expect("connect");
    let mut writer = stream.try_clone().expect("clone for writing");
    let mut reader = BufReader::new(stream);

    command(&mut writer, &mut reader, "new-session -s auto");
    // The deterministic death: `-k` replaces the shell with `exit 3`, which
    // is gone before the next reap pass.
    command(&mut writer, &mut reader, "respawn-pane -t %0 -k exit 3");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = command(&mut writer, &mut reader, "list-panes").join("");
        if !listed.contains("%0") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the dead pane was never auto-removed: {listed}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // The cascade: the window went with the pane, and the emptied session
    // with it.
    let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
    assert!(
        !sessions.contains("auto"),
        "the emptied session cascaded away: {sessions}"
    );
    // The pane is gone, not held: respawn answers no-such-pane.
    let respawn = command(&mut writer, &mut reader, "respawn-pane -t %0").join("");
    assert!(
        respawn.contains("no such pane"),
        "a removed pane is gone, not held: {respawn}"
    );
}

/// remain-on-exit is the one LIVE daemon setting: a daemon started with the
/// hold on honors it, a `reload-config` after the config flips to
/// auto-remove applies the change at once (the `applied:` report line),
/// and the NEXT death is auto-removed with no restart.
#[test]
fn reload_config_flips_remain_on_exit_live() {
    let fixture = MuxFixture::new("relivereap");
    let _daemon = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let stream = connect_local_stream(fixture.socket()).expect("connect");
    let mut writer = stream.try_clone().expect("clone for writing");
    let mut reader = BufReader::new(stream);

    command(&mut writer, &mut reader, "new-session -s live");
    command(&mut writer, &mut reader, "respawn-pane -t %0 -k exit 3");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = command(&mut writer, &mut reader, "pane-info -t %0").join("");
        if info.contains(" exited=3") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the death was never recorded as held: {info}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // Flip the config to auto-remove and reload: the report names the
    // live apply.
    std::fs::write(fixture.config_path(), "[daemon]\nremain-on-exit = false\n")
        .expect("rewrite fixture config");
    let report = command(&mut writer, &mut reader, "reload-config").join("");
    assert!(
        report.contains("applied: daemon.remain-on-exit"),
        "the flip applies live: {report}"
    );

    // The next death is auto-removed with no restart.
    command(&mut writer, &mut reader, "respawn-pane -t %0");
    command(&mut writer, &mut reader, "respawn-pane -t %0 -k exit 4");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = command(&mut writer, &mut reader, "list-panes").join("");
        if !listed.contains("%0") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the post-reload death was never auto-removed: {listed}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// SEC-126: `respawn-pane` flags stop at the first command word, so a
/// `-k` or `-c` inside the command neither kills a live pane nor becomes
/// the start directory.
#[cfg(unix)]
#[test]
fn respawn_pane_ignores_flags_inside_the_command() {
    let fixture = MuxFixture::new("flags");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s flags").expect("new-session");

    // `sort -k` is the command's own flag: the live pane must be refused.
    let refused = client
        .send("respawn-pane -t %0 sh -c 'echo X; sort -k 1 /dev/null; sleep 600'")
        .expect("refuse")
        .join("");
    assert!(
        refused.contains("still running"),
        "a -k inside the command must not kill a live pane: {refused}"
    );

    // With a leading -k the restart runs the WHOLE command: `sh -c` keeps
    // its script instead of the inner `-c` being taken as a start dir.
    client
        .send("respawn-pane -t %0 -k sh -c 'echo RESP-$((6*7))'")
        .expect("restart");
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut respawned, mut output) = (false, false);
    while Instant::now() < deadline && !(respawned && output) {
        match client
            .notifications()
            .recv_timeout(Duration::from_millis(250))
        {
            Ok(par_term_emu_core::tmux_control::TmuxNotification::PaneRespawned { pane_id })
                if pane_id == "%0" =>
            {
                respawned = true
            }
            Ok(par_term_emu_core::tmux_control::TmuxNotification::Output { pane_id, data })
                if pane_id == "%0" && String::from_utf8_lossy(&data).contains("RESP-42") =>
            {
                output = true
            }
            _ => {}
        }
    }
    assert!(respawned, "the -k restart must be announced");
    assert!(output, "the respawned pane must run the whole sh -c script");
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
    let state_path = par_mux::mux::persist::state_file_in(state_dir.path(), &path);
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

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
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
    // form only, so only real output satisfies the wait. cmd.exe has no tr;
    // powershell lowercases instead.
    #[cfg(unix)]
    let marker_cmd = "echo SPLIT-OUT | tr A-Z a-z".to_string();
    #[cfg(windows)]
    let marker_cmd =
        r#"powershell -NoProfile -Command "'SPLIT-OUT'.ToLower()"#.replace('\'', r"'\''");
    client
        .send(&format!("send-keys -t {new_pane} '{marker_cmd}' Enter"))
        .expect("send-keys");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw = false;
    while Instant::now() < deadline && !saw {
        if let Ok(par_term_emu_core::tmux_control::TmuxNotification::Output { pane_id, data }) =
            client
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

/// Await the next `%layout-change` whose raw flags are `want_flags` and
/// whose layout tree is (`true`) or is not (`false`) a split — a split
/// renders with a `{…}` group (side-by-side) or a `[…]` group
/// (stacked). The filter picks a window shape out of the stream: the
/// split's two-pane frame over new-session's single-pane one, the zoom
/// frame (whose `window_layout` stays the true tree), and so on.
/// Returns `(window_layout, window_visible_layout)`.
fn next_layout_change(
    client: &mut par_mux::mux::MuxClient,
    want_flags: &str,
    want_split: bool,
) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(par_term_emu_core::tmux_control::TmuxNotification::LayoutChange {
            window_layout,
            window_visible_layout,
            window_raw_flags,
            ..
        }) = client
            .notifications()
            .recv_timeout(Duration::from_millis(250))
        {
            let split = window_layout.contains('{') || window_layout.contains('[');
            if window_raw_flags == want_flags && split == want_split {
                return (window_layout, window_visible_layout);
            }
        }
    }
    panic!("no %layout-change with flags {want_flags:?} (split={want_split}) arrived within 15s");
}

/// `resize-pane -Z` over the wire: the zoom's `%layout-change` keeps the
/// true tree in `window_layout` — that is what makes unzoom restore
/// exact — and marks the zoom in the other two fields instead: the
/// visible layout is the zoomed pane alone at the window's full extent,
/// and the raw flags carry tmux's `Z`. Unzooming pushes the tree back
/// into both layout fields with empty flags.
#[test]
fn a_zoom_marks_layout_change_and_unzoom_restores_it() {
    let fixture = MuxFixture::new("zoomwire");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s zoomwire").expect("new-session");
    client.send("split-window -h -t %0").expect("split");
    let (tree, visible) = next_layout_change(&mut client, "", true);
    assert_eq!(visible, tree, "unzoomed frames show the tree as-is");
    let extent = tree.split(',').nth(1).expect("window extent field");

    client.send("resize-pane -t %0 -Z").expect("zoom");
    let (zoom_tree, zoom_visible) = next_layout_change(&mut client, "Z", true);
    assert_eq!(zoom_tree, tree, "the zoom never edits the layout tree");
    assert!(
        !zoom_visible.contains('{'),
        "the visible layout is the zoomed pane alone: {zoom_visible}"
    );
    assert_eq!(
        zoom_visible.split(',').nth(1),
        Some(extent),
        "the zoomed pane renders at the window's full extent: {zoom_visible}"
    );
    assert!(
        zoom_visible.ends_with(",0"),
        "the visible pane is the zoom target %0: {zoom_visible}"
    );

    client.send("resize-pane -t %0 -Z").expect("unzoom");
    let (tree_again, visible_again) = next_layout_change(&mut client, "", true);
    assert_eq!(tree_again, tree, "unzoom restores the exact prior layout");
    assert_eq!(
        visible_again, tree,
        "unzoom restores the visible layout too"
    );
}

/// Await the next notification matching `pred`, skipping unrelated ones —
/// the interleaved `%output`/`%layout-change` frames a busy daemon pushes
/// must not satisfy a lifecycle wait.
fn next_notification(
    client: &mut par_mux::mux::MuxClient,
    mut pred: impl FnMut(&par_term_emu_core::tmux_control::TmuxNotification) -> bool,
) -> par_term_emu_core::tmux_control::TmuxNotification {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(note) = client
            .notifications()
            .recv_timeout(Duration::from_millis(250))
        {
            if pred(&note) {
                return note;
            }
        }
    }
    panic!("no matching notification arrived within 15s");
}

/// `break-pane`, `join-pane`, and window reorder over the wire:
/// break-pane replies the new window's id and announces it with
/// `%window-add` plus a `%layout-change` for the source; join-pane moves
/// the pane back and closes the emptied window (`%window-close`), with
/// layout changes for both windows; `swap-window` reorders, and
/// `list-windows` reflects the new order.
#[test]
fn panes_break_join_and_windows_reorder_over_the_wire() {
    let fixture = MuxFixture::new("bjr");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s bjr").expect("new-session");
    let moved = client
        .send("split-window -h -t %0")
        .expect("split")
        .join("")
        .trim()
        .to_string();
    let window_two = client
        .send("new-window -n two")
        .expect("new-window")
        .join("")
        .trim()
        .to_string();
    assert!(moved.starts_with('%'), "split replies the pane id: {moved}");
    assert!(
        window_two.starts_with('@'),
        "new-window replies the window id: {window_two}"
    );
    // Flush the setup's own notifications so every wait below matches
    // only the operation it follows.
    while client
        .notifications()
        .recv_timeout(Duration::from_millis(200))
        .is_ok()
    {}

    // break-pane: the pane leaves @0 (its layout drops to one pane) and
    // its new window is announced.
    let broken = client
        .send(&format!("break-pane -s {moved} -n promoted"))
        .expect("break")
        .join("")
        .trim()
        .to_string();
    assert!(
        broken.starts_with('@'),
        "break replies the window id: {broken}"
    );
    let (layout, _) = next_layout_change(&mut client, "", false);
    assert!(
        !(layout.contains('{') || layout.contains('[')),
        "the source window's layout dropped to one pane: {layout}"
    );
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::WindowAdd { window_id, .. }
                if window_id == &broken
        )
    });

    // join-pane: the pane lands back beside %0 (two panes again) and the
    // emptied break window closes.
    let join_reply = client
        .send(&format!("join-pane -s {moved} -t %0"))
        .expect("join")
        .join("");
    assert!(
        !join_reply.contains("error:"),
        "join-pane must succeed: {join_reply}"
    );
    let (layout, _) = next_layout_change(&mut client, "", true);
    assert!(
        layout.contains('{') || layout.contains('['),
        "the destination window holds both panes again: {layout}"
    );
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::WindowClose { window_id }
                if window_id == &broken
        )
    });

    // swap-window: the reorder cue lands and list-windows reflects it.
    client
        .send(&format!("swap-window -s @0 -t {window_two}"))
        .expect("swap");
    next_notification(&mut client, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::SessionsChanged
        )
    });
    let listing = client.send("list-windows").expect("list").join("\n");
    let two_at = listing
        .lines()
        .position(|l| l.starts_with(&window_two))
        .expect("window two is listed");
    let zero_at = listing
        .lines()
        .position(|l| l.starts_with("@0"))
        .expect("window zero is listed");
    assert!(
        two_at < zero_at,
        "the swap moved window two first: {listing}"
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

    let expected = par_mux::mux::build_stamp();
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
    use par_mux::mux::MuxClient;

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
    use par_mux::mux::MuxClient;
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
/// serving other clients afterwards. This is the complete-line case; the
/// unterminated stream is `an_unterminated_stream_over_budget_closes_the_connection`.
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

/// SEC-127: a client streaming bytes with no newline at all is cut off near
/// the 1 MiB budget. The old loop only checked the budget after `read_line`
/// returned, and `read_line` never returns without a newline, so the daemon
/// kept consuming (measured: 256 MiB in, RSS 12 → 279 MiB) and never
/// answered.
#[test]
fn an_unterminated_stream_over_budget_closes_the_connection() {
    use interprocess::local_socket::traits::Stream as _;
    use std::io::{BufRead, Write as _};

    const TOTAL: usize = 8 * 1024 * 1024;
    const CHUNK: usize = 64 * 1024;

    let fixture = MuxFixture::new("sec127");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let handle = std::thread::spawn(move || server.run());
    wait_listening(path);

    {
        let stream = connect_local_stream(path).expect("connect");
        // Unix only (named pipes reject I/O timeouts): keeps a daemon that
        // never answers from parking this test in `read_line` forever.
        let _ = stream.set_recv_timeout(Some(Duration::from_millis(250)));
        let mut writer = stream.try_clone().expect("clone");
        let _ = writer.set_send_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream);

        let pushed = std::thread::spawn(move || {
            let mut total = 0usize;
            if writer.write_all(b"send-keys -l ").is_err() {
                return total;
            }
            let chunk = vec![b'x'; CHUNK];
            while total < TOTAL {
                if writer.write_all(&chunk).is_err() {
                    break;
                }
                total += CHUNK;
            }
            total
        });

        let mut reply = String::new();
        let mut closed = false;
        // Outside the loop: a recv-timeout wake mid-line leaves the partial
        // in `line` for the next read to finish.
        let mut line = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match reader.read_line(&mut line) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(_) => {
                    reply.push_str(&line);
                    line.clear();
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                // A reset after the %error block is the close, too.
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
        assert!(
            reply.contains("%error") && reply.contains("1 MiB"),
            "an unterminated over-budget stream must be answered with the budget %error: {reply:?}"
        );
        assert!(closed, "the connection must close after the %error block");
        let total = pushed.join().expect("writer thread");
        assert!(
            total < 4 * 1024 * 1024,
            "the daemon must stop consuming near the budget, but it took {total} bytes"
        );
    }

    wait_listening(path);
    let stream = connect_local_stream(path).expect("second connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    let listed = command(&mut writer, &mut reader, "list-panes").join("");
    assert!(
        listed.contains("%end"),
        "the daemon must keep serving other clients: {listed}"
    );

    drop(writer);
    drop(reader);
    let _ = handle;
    drop(fixture);
}

/// `split-window -b` (card 01a0ea74): the new pane is placed BEFORE the
/// target — left of it under `-h`, above it in the default direction.
/// `list-panes` walks the layout's leaf order, so its line order is the
/// on-screen left-to-right / top-to-bottom order.
#[test]
fn split_window_b_places_the_new_pane_before_its_target() {
    let fixture = MuxFixture::new("splitb");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s splitb").expect("new-session");

    // Side by side, new pane LEFT of the target.
    let first = client
        .send("split-window -b -h -t %0")
        .expect("split")
        .join("")
        .trim()
        .to_string();
    assert_eq!(first, "%1", "the reply is the new pane id");
    let listed = client.send("list-panes").expect("list").join("\n");
    let order: Vec<&str> = listed.lines().map(str::trim).collect();
    assert_eq!(
        order,
        vec!["%1", "%0"],
        "-b -h puts the new pane left: {listed}"
    );

    // Default direction, new pane ABOVE its target.
    let second = client
        .send("split-window -b -t %1")
        .expect("split")
        .join("")
        .trim()
        .to_string();
    assert_eq!(second, "%2");
    let listed = client.send("list-panes").expect("list").join("\n");
    let order: Vec<&str> = listed.lines().map(str::trim).collect();
    assert_eq!(
        order,
        vec!["%2", "%1", "%0"],
        "-b stacks the second new pane above: {listed}"
    );
}

/// The `cmd=` token of a `pane-info` reply (card 01a0ea74): the pane's
/// foreground command — the deepest descendant of its child process, so a
/// pane running a job under the shell reports the job while an idle pane
/// reports the shell itself.
///
/// Unix-only: the process table snapshot is `None` on Windows, so the
/// token is legitimately absent there and the poll below could only fail.
#[cfg(unix)]
#[test]
fn pane_info_reports_the_panes_foreground_command() {
    use base64::Engine as _;

    fn cmd_token(reply: &str) -> Option<String> {
        let token = reply.split_whitespace().find(|t| t.starts_with("cmd="))?;
        let encoded = token.strip_prefix("cmd=")?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    let fixture = MuxFixture::new("fgcmd");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s fgcmd").expect("new-session");

    // Idle: the token names the pane's own shell. The shell may take a
    // moment to appear in the process table, so poll for a non-empty name.
    let deadline = Instant::now() + Duration::from_secs(10);
    let idle_name = loop {
        let reply = client.send("pane-info -t %0").expect("info").join("");
        if let Some(name) = cmd_token(&reply).filter(|n| !n.is_empty()) {
            break name;
        }
        assert!(
            Instant::now() < deadline,
            "an idle pane must name its shell: last reply {reply}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_ne!(
        idle_name, "sleep",
        "the idle pane is not running the job yet"
    );

    // A child process takes the foreground: sleep runs under the shell.
    client
        .send("send-keys -t %0 'sleep 15' Enter")
        .expect("send");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = client.send("pane-info -t %0").expect("info").join("");
        if cmd_token(&reply).as_deref() == Some("sleep") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cmd= must report the running job: last reply {reply}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// ARC-095: a held-dead pane's exit is a query, not only a push. A client
/// that connects after `%pane-exited` reads the code from `pane-info`'s
/// `exited=` token.
#[cfg(unix)]
#[test]
fn pane_info_reports_exit_for_held_pane() {
    let fixture = MuxFixture::new("paneexit");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut first = par_mux::mux::MuxClient::connect(path).expect("connect");
    first.send("new-session -s paneexit").expect("new-session");
    first
        .send("respawn-pane -t %0 -k exit 3")
        .expect("respawn into exit 3");
    next_notification(&mut first, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneExited {
                pane_id,
                exit_code: Some(3),
            } if pane_id == "%0"
        )
    });

    // A client that missed the push.
    let mut late = par_mux::mux::MuxClient::connect(path).expect("connect late");
    let reply = late.send("pane-info -t %0").expect("info").join("");
    assert!(
        reply.trim_end().ends_with(" exited=3"),
        "a held pane's pane-info carries its exit code: {reply:?}"
    );
    assert!(
        !reply.contains("cmd="),
        "a reaped child has no foreground command: {reply:?}"
    );
}

/// ARC-095: a live pane's `pane-info` carries no `exited=` token.
#[test]
fn pane_info_live_pane_has_no_exited_token() {
    let fixture = MuxFixture::new("panelive");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s panelive").expect("new-session");
    let reply = client.send("pane-info -t %0").expect("info").join("");
    assert!(
        reply.starts_with("%0 @0 "),
        "pane-info answers for the live pane: {reply:?}"
    );
    assert!(
        !reply.contains("exited="),
        "a live pane has no exited= token: {reply:?}"
    );
}

/// QA-230 (ENH-037): a registering client's held-state replay is queued
/// ahead of its first command's reply — on the raw wire, `%pane-exited` for
/// the held pane and the zoomed window's `%layout-change` (raw flags `Z`)
/// are the only lines before the `%begin` of the reply, and that reply is
/// the first command's (number 1).
#[test]
fn held_state_replay_precedes_the_first_commands_reply() {
    use std::io::{BufRead, Write as _};

    let fixture = MuxFixture::new("replayord");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    // Client A builds the state a late client must replay: pane %1 held
    // with exit 7 and window @0 zoomed. Both waits prove the daemon has
    // settled that state before B ever registers.
    let mut a = par_mux::mux::MuxClient::connect(path).expect("connect");
    a.send("new-session -s replayord").expect("new-session");
    a.send("split-window -h -t %0").expect("split");
    a.send("respawn-pane -t %1 -k exit 7").expect("hold %1");
    next_notification(&mut a, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneExited {
                pane_id,
                exit_code: Some(7),
            } if pane_id == "%1"
        )
    });
    a.send("resize-pane -t %0 -Z").expect("zoom");
    next_layout_change(&mut a, "Z", true);

    // Client B registers by sending its first command; every line before
    // the reply's %begin must be held-state replay.
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    writeln!(writer, "list-panes").expect("write first command");
    writer.flush().expect("flush");

    let (mut saw_exit, mut saw_zoom) = (false, false);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read wire");
        assert!(n > 0, "server closed before the reply's %begin");
        if line.starts_with("%begin") {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.starts_with("%pane-exited %1 7") {
            saw_exit = true;
        } else if trimmed.starts_with("%layout-change") && trimmed.ends_with(" Z") {
            saw_zoom = true;
        } else {
            panic!("a line before the reply's %begin is not held-state replay: {trimmed:?}");
        }
    }
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields[2], "1",
        "the replay precedes the FIRST command's reply block: {line:?}"
    );
    assert!(
        saw_exit,
        "the held pane's %pane-exited must precede the reply"
    );
    assert!(
        saw_zoom,
        "the zoomed window's %layout-change must precede the reply"
    );

    // Drain the reply block so teardown never races a full socket buffer.
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read wire");
        assert!(n > 0, "server closed before %end");
        if line.starts_with("%end") || line.starts_with("%error") {
            break;
        }
    }
}

/// QA-230 (ENH-037): on a fresh control connection the replay also precedes
/// the `%begin` framing itself — the bare replay lines are wrapped in
/// nothing, and the first `%begin` on the wire opens the first command's
/// block. The first line here is an unknown command, so registration runs
/// on the error-reply path and the numbered `%error` block is what the
/// replay precedes.
#[test]
fn replay_precedes_begin_framing_on_a_fresh_control_connection() {
    use std::io::{BufRead, Write as _};

    let fixture = MuxFixture::new("replayfrm");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut a = par_mux::mux::MuxClient::connect(path).expect("connect");
    a.send("new-session -s replayfrm").expect("new-session");
    a.send("respawn-pane -t %0 -k exit 3").expect("hold %0");
    next_notification(&mut a, |note| {
        matches!(
            note,
            par_term_emu_core::tmux_control::TmuxNotification::PaneExited {
                pane_id,
                exit_code: Some(3),
            } if pane_id == "%0"
        )
    });

    // A fresh connection whose first line is an unknown command.
    let stream = connect_local_stream(path).expect("connect");
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    writeln!(writer, "no-such-command").expect("write unknown command");
    writer.flush().expect("flush");

    let mut saw_exit = false;
    let begin;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read wire");
        assert!(n > 0, "server closed before any %begin");
        if line.starts_with("%begin") {
            begin = line.clone();
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.starts_with("%pane-exited %0 3") {
            saw_exit = true;
        } else {
            panic!("a line before the first %begin is not held-state replay: {trimmed:?}");
        }
    }
    assert!(
        saw_exit,
        "the held pane's %pane-exited must precede the framing"
    );
    let fields: Vec<&str> = begin.split_whitespace().collect();
    assert_eq!(
        fields[2], "1",
        "the first %begin opens the first command's block: {begin:?}"
    );
    let mut block = begin;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read wire");
        assert!(n > 0, "server closed before the block closed");
        block.push_str(&line);
        if line.starts_with("%end") || line.starts_with("%error") {
            break;
        }
    }
    assert!(
        block.contains("%error"),
        "the unknown command is rejected with an error block: {block:?}"
    );
}

/// ENH-042: `pane-exited-replay` pushes the held panes' `%pane-exited` lines
/// to the issuing client's stream only, with an empty reply block; the
/// other connected clients receive nothing new.
#[test]
fn pane_exited_replay_reaches_only_the_issuing_client() {
    use par_term_emu_core::tmux_control::TmuxNotification;

    let fixture = MuxFixture::new("exitreplay");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut a = par_mux::mux::MuxClient::connect(path).expect("connect a");
    a.send("new-session -s exitreplay").expect("new-session");
    a.send("split-window -h -t %0").expect("split");
    a.send("respawn-pane -t %1 -k exit 7").expect("hold %1");
    let held = |note: &TmuxNotification| {
        matches!(
            note,
            TmuxNotification::PaneExited { pane_id, exit_code: Some(7) } if pane_id == "%1"
        )
    };
    next_notification(&mut a, held);

    // B and C register on their first command; registration replay delivers
    // the held exit once to each. Consume it so the on-demand replay is the
    // next one B sees.
    let mut b = par_mux::mux::MuxClient::connect(path).expect("connect b");
    b.send("list-panes").expect("register b");
    next_notification(&mut b, held);
    let mut c = par_mux::mux::MuxClient::connect(path).expect("connect c");
    c.send("list-panes").expect("register c");
    next_notification(&mut c, held);

    let reply = b.send_checked("pane-exited-replay").expect("replay");
    assert!(reply.ok, "pane-exited-replay succeeds: {:?}", reply.body);
    assert!(
        reply.body.iter().all(|line| line.is_empty()),
        "the reply body is empty, the pushed lines are the payload: {:?}",
        reply.body
    );
    next_notification(&mut b, held);

    // Give any wrongly broadcast copy time to land, then check the other
    // clients saw no exit line.
    std::thread::sleep(Duration::from_millis(500));
    for (name, client) in [("a", &a), ("c", &c)] {
        while let Ok(note) = client.notifications().try_recv() {
            assert!(
                !matches!(note, TmuxNotification::PaneExited { .. }),
                "client {name} must not receive another client's replay: {note:?}"
            );
        }
    }
}

/// ENH-042: with no held panes `pane-exited-replay` succeeds with an empty
/// reply and pushes no `%pane-exited` line.
#[test]
fn pane_exited_replay_with_no_held_panes_is_empty() {
    use par_term_emu_core::tmux_control::TmuxNotification;

    let fixture = MuxFixture::new("exitnone");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _handle = std::thread::spawn(move || server.run());

    let mut client = par_mux::mux::MuxClient::connect(path).expect("connect");
    client.send("new-session -s exitnone").expect("new-session");
    let reply = client.send_checked("pane-exited-replay").expect("replay");
    assert!(reply.ok, "succeeds with nothing held: {:?}", reply.body);
    assert!(
        reply.body.iter().all(|line| line.is_empty()),
        "empty reply: {:?}",
        reply.body
    );
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(note) = client.notifications().try_recv() {
        assert!(
            !matches!(note, TmuxNotification::PaneExited { .. }),
            "no held pane, no %pane-exited: {note:?}"
        );
    }
}

/// The workspace commands over a live daemon socket: create, list, select,
/// rename, kill, the workspace-filtered list-sessions, and the
/// `%workspaces-changed` broadcast a second client receives.
#[test]
fn workspace_commands_over_the_wire() {
    let fixture = MuxFixture::new("wswire");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    let _server = std::thread::spawn(move || server.run());
    wait_listening(path);
    {
        let stream = connect_local_stream(path).expect("daemon accepts");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        // An observer client, connected before anything happens: everything
        // it sees is a broadcast. It joins the broadcast set on its first
        // control command.
        let mut observer = par_mux::mux::MuxClient::connect(path).expect("connect");
        let _ = observer.send("list-panes");

        let reply = command(&mut writer, &mut reader, "new-workspace -n dev").join("");
        assert!(
            reply.lines().any(|l| l.trim() == "+0"),
            "new-workspace replies with the id: {reply}"
        );

        let listed = command(&mut writer, &mut reader, "list-workspaces").join("");
        assert!(
            listed.lines().any(|l| l.trim() == "+0: dev active"),
            "the new workspace is active: {listed}"
        );

        // new-workspace spawns the workspace's FIRST session/window (the
        // manual-pass report: a created workspace highlighted in the side
        // panel while the view had nothing to land on), so the workspace
        // is landable the moment it exists; the session is named after
        // the workspace.
        let sessions = command(&mut writer, &mut reader, "list-sessions -t dev").join("");
        assert!(
            sessions.lines().any(|l| l.contains("$0: dev")),
            "the created workspace carries its first session: {sessions}"
        );

        // A further session lands in the active workspace; bare
        // list-sessions carries the workspace prefix.
        command(&mut writer, &mut reader, "new-session -s ws-svc");
        let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
        assert!(
            sessions.lines().any(|l| l.trim() == "+0: dev: $1: ws-svc"),
            "the workspace prefix rides the session line: {sessions}"
        );

        // The filter form.
        let filtered = command(&mut writer, &mut reader, "list-sessions -t dev").join("");
        assert!(filtered.lines().any(|l| l.contains("ws-svc")));

        // select + rename across two workspaces.
        command(&mut writer, &mut reader, "new-workspace -n beta");
        command(&mut writer, &mut reader, "select-workspace -t +0");
        let listed = command(&mut writer, &mut reader, "list-workspaces").join("");
        assert!(listed.lines().any(|l| l.trim() == "+0: dev active"));
        command(&mut writer, &mut reader, "rename-workspace -t +0 prod");
        let listed = command(&mut writer, &mut reader, "list-workspaces").join("");
        // prod is the active workspace, so its line carries the marker.
        assert!(listed.lines().any(|l| l.trim() == "+0: prod active"));

        // kill-workspace takes the workspace's session with it.
        command(&mut writer, &mut reader, "kill-workspace -t prod");
        let sessions = command(&mut writer, &mut reader, "list-sessions").join("");
        assert!(
            !sessions.contains("ws-svc"),
            "the killed workspace's session went with it: {sessions}"
        );

        // The observer saw the %workspaces-changed broadcasts (new-workspace,
        // select, rename, and kill all cue the roster).
        let mut saw = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && saw < 2 {
            match observer
                .notifications()
                .recv_timeout(Duration::from_millis(200))
            {
                Ok(par_term_emu_core::tmux_control::TmuxNotification::WorkspacesChanged) => {
                    saw += 1;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(saw >= 1, "the observer received %workspaces-changed");
    }
}

// ---- Smallest-attached-client sizing + shared-selection sync ----
//
// Two real client connections against one daemon: A drives, B observes.

/// The last `%layout-change` line's window extent, parsed from the layout
/// triple's `0000,COLSxROWS,0,0,N` window header field.
fn last_layout_size(text: &str) -> Option<(u32, u32)> {
    text.lines()
        .rfind(|l| l.starts_with("%layout-change "))
        .and_then(|line| {
            // `%layout-change @N <layout> <visible> <flags>` — the extent
            // is the window header field `0000,COLSxROWS,0,0,N`.
            line.split_whitespace()
                .nth(2)
                .and_then(|triple| triple.split(',').nth(1))
                .and_then(|extent| {
                    let (cols, rows) = extent.split_once('x')?;
                    Some((cols.parse().ok()?, rows.parse().ok()?))
                })
        })
}

/// Probe with `probe` until `needle` appears in the accumulated stream
/// (pushes ride the probes as body noise), returning everything read.
fn poll_until(
    writer: &mut impl std::io::Write,
    reader: &mut impl std::io::BufRead,
    probe: &str,
    needle: &str,
) -> String {
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !seen.contains(needle) {
        seen.push_str(&command(writer, reader, probe).join(""));
        assert!(
            Instant::now() < deadline,
            "never saw {needle:?}; read so far: {seen}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    seen
}

/// Poll `probes` times and return the accumulated stream (assertions about
/// what must NOT appear run over the whole accumulation).
fn poll_times(
    writer: &mut impl std::io::Write,
    reader: &mut impl std::io::BufRead,
    probe: &str,
    probes: usize,
) -> String {
    let mut seen = String::new();
    for _ in 0..probes {
        seen.push_str(&command(writer, reader, probe).join(""));
        std::thread::sleep(Duration::from_millis(25));
    }
    seen
}

#[test]
fn window_sizes_to_the_smallest_viewing_client_and_grows_on_disconnect() {
    let fixture = MuxFixture::new("minsize");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s min");
    let pane = pane_ids(&command(&mut writer_a, &mut reader_a, "list-panes").join("")).remove(0);

    // B registers and reports a smaller grid against the same window.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");
    let b_report = command(
        &mut writer_b,
        &mut reader_b,
        &format!("refresh-client -t {pane} -C 60x20"),
    );
    assert!(
        b_report.iter().any(|l| l.starts_with("%end")),
        "B's size report must succeed: {b_report:?}"
    );

    // A reports larger: the window takes the componentwise minimum (60x20).
    // The broadcast precedes the issuer's reply on the wire, so A's own
    // report read carries B's %layout-change line — the poll seeds from it.
    let report = command(
        &mut writer_a,
        &mut reader_a,
        &format!("refresh-client -t {pane} -C 100x30"),
    );
    assert!(
        report.iter().any(|l| l.starts_with("%end")),
        "A's size report must succeed: {report:?}"
    );
    let mut seen: Vec<String> = report;
    seen.extend(b_report);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !seen.join("").contains("%layout-change @0 0000,60x20") {
        seen.extend(command(&mut writer_a, &mut reader_a, "list-panes"));
        assert!(
            Instant::now() < deadline,
            "never saw the 60x20 minimum; read so far: {:?}",
            seen.join("")
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    // Repeated identical reports from B never move the grid again — the
    // no-feedback-loop property, observed live.
    for _ in 0..3 {
        command(
            &mut writer_b,
            &mut reader_b,
            &format!("refresh-client -t {pane} -C 60x20"),
        );
    }
    let stable = poll_times(&mut writer_a, &mut reader_a, "list-panes", 4);
    assert_eq!(
        last_layout_size(&stable),
        Some((60, 20)),
        "the minimum is stable under repeated reports; stream: {}",
        stable.escape_debug()
    );

    // B disconnects: its contribution is dropped and the window grows to
    // A's report.
    drop(writer_b);
    drop(reader_b);
    poll_until(
        &mut writer_a,
        &mut reader_a,
        "list-panes",
        "%layout-change @0 0000,100x30",
    );
}

#[test]
fn a_tab_switch_broadcasts_the_session_window_change_to_the_other_client() {
    let fixture = MuxFixture::new("tabsync");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s tabs");
    command(&mut writer_a, &mut reader_a, "new-window");

    // B registers BEFORE the switch so it is in the broadcast set.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");

    command(&mut writer_a, &mut reader_a, "select-window -t @1");
    let seen = poll_until(
        &mut writer_b,
        &mut reader_b,
        "list-panes",
        "%session-window-changed $0 @1",
    );
    assert!(
        seen.contains("%session-window-changed $0 @1"),
        "client B learns the session's window moved: {seen}"
    );
}

/// Zoom truth is per-window and broadcast: A's `resize-pane -Z` reaches a
/// second client as a `Z`-flagged `%layout-change` for the window, and A's
/// select-away (which unzooms) reaches it as an unflagged one — the cue
/// every attached render client sets its ` Z ` status from.
#[test]
fn a_zoom_and_its_select_away_unzoom_reach_the_other_client() {
    let fixture = MuxFixture::new("zoomsync");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s zoom");
    command(&mut writer_a, &mut reader_a, "split-window -h -t %0");

    // B registers BEFORE the zoom so it is in the broadcast set.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");

    command(&mut writer_a, &mut reader_a, "resize-pane -t %0 -Z");
    let zoomed = poll_until(&mut writer_b, &mut reader_b, "list-panes", " Z\n");
    assert!(
        zoomed
            .lines()
            .any(|l| l.starts_with("%layout-change @0 ") && l.ends_with(" Z")),
        "B sees the window's zoom flag: {zoomed}"
    );

    command(&mut writer_a, &mut reader_a, "select-pane -t %1");
    let unzoomed = poll_until(
        &mut writer_b,
        &mut reader_b,
        "list-panes",
        "%window-pane-changed @0 %1",
    );
    let after_zoom = unzoomed
        .lines()
        .rfind(|l| l.starts_with("%layout-change @0 "))
        .expect("the select-away unzoom broadcasts a layout change");
    assert!(
        after_zoom.ends_with(' '),
        "the select-away unzoom clears the flag: {after_zoom:?}"
    );
}

/// Client lifecycle: a second client's registration (its first control
/// command) reaches the already-registered client as
/// `%client-attached <id>`, and its socket drop — the same teardown an
/// eviction ends in — as `%client-left <id> $S @W`, naming the window it
/// was displaying. The joining client never sees its own attach line.
#[test]
fn client_attach_and_drop_broadcast_lifecycle_lines() {
    let fixture = MuxFixture::new("clientlife");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    // A registers FIRST so it is in the broadcast set for B's lifecycle.
    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s life");
    let pane = pane_ids(&command(&mut writer_a, &mut reader_a, "list-panes").join("")).remove(0);

    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    let b_first = command(&mut writer_b, &mut reader_b, "list-panes").join("");
    assert!(
        !b_first.contains("%client-attached"),
        "the joining client is not told about itself: {b_first}"
    );
    // Registered but silent about its size: not announced yet.
    let before_size = poll_times(&mut writer_a, &mut reader_a, "list-panes", 6);
    assert!(
        !before_size.contains("%client-attached"),
        "a client that has not reported a size is not announced: {before_size}"
    );
    command(
        &mut writer_b,
        &mut reader_b,
        &format!("refresh-client -t {pane} -C 60x20"),
    );

    let attached = poll_until(
        &mut writer_a,
        &mut reader_a,
        "list-panes",
        "%client-attached ",
    );
    let id = attached
        .lines()
        .find_map(|l| l.strip_prefix("%client-attached "))
        .expect("the attach line")
        .trim()
        .to_string();
    assert!(
        !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
        "the attach line carries a numeric client id: {id:?}"
    );

    drop(writer_b);
    drop(reader_b);
    let left = poll_until(&mut writer_a, &mut reader_a, "list-panes", "%client-left ");
    let line = left
        .lines()
        .find(|l| l.starts_with("%client-left "))
        .expect("the left line");
    assert_eq!(
        line,
        format!("%client-left {id} $0 @0"),
        "the left line names the same client and the window it displayed"
    );
}

#[test]
fn a_client_that_never_reports_a_size_causes_no_lifecycle_lines() {
    let fixture = MuxFixture::new("clientquiet");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s quiet");

    // A one-shot command client: registers, never reports a size, leaves.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");
    drop(writer_b);
    drop(reader_b);

    let seen = poll_times(&mut writer_a, &mut reader_a, "list-panes", 12);
    assert!(
        !seen.contains("%client-attached") && !seen.contains("%client-left"),
        "a client that never reported a size is neither announced nor missed: {seen}"
    );
}

#[test]
fn a_workspace_switch_broadcasts_the_display_move_to_the_other_client() {
    let fixture = MuxFixture::new("wssync");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s first");
    // A second workspace with its auto-spawned first session; the daemon's
    // displayed view now sits in +1.
    command(&mut writer_a, &mut reader_a, "new-workspace -n alt");

    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");

    // Switching back to the first workspace moves the shared display; the
    // notification's `client` field carries the workspace (+0), the rest
    // names the session the display landed on.
    command(&mut writer_a, &mut reader_a, "select-workspace -t +0");
    let seen = poll_until(
        &mut writer_b,
        &mut reader_b,
        "list-panes",
        "%client-session-changed +0 ",
    );
    assert!(
        seen.contains("%client-session-changed +0 $0"),
        "client B learns the displayed session moved: {seen}"
    );
}

#[test]
fn a_control_client_that_never_reports_a_size_never_constrains() {
    let fixture = MuxFixture::new("noconstrain");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s solo");

    // B registers (joins the broadcast set) but NEVER reports a size —
    // the plain control / roster-watcher shape.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");

    // Several polls: no %layout-change may appear from B's mere
    // registration.
    let quiet = poll_times(&mut writer_a, &mut reader_a, "list-panes", 4);
    assert!(
        !quiet.contains("%layout-change"),
        "an unreported control connection must not move any grid: {quiet}"
    );

    // A report still lands: the window takes A's own extent. The issuer's
    // own broadcast rides its report reply, so assert there.
    let pane = pane_ids(&command(&mut writer_a, &mut reader_a, "list-panes").join("")).remove(0);
    let report = command(
        &mut writer_a,
        &mut reader_a,
        &format!("refresh-client -t {pane} -C 90x26"),
    );
    assert!(
        report.join("").contains("%layout-change @0 0000,90x26"),
        "A's own report sizes the window to 90x26: {report:?}"
    );
}

#[test]
fn a_following_client_reconstructs_the_switched_window_screen() {
    let fixture = MuxFixture::new("rescreen");
    let path = fixture.socket();
    let server = MuxServer::bind(path).expect("bind");
    std::thread::spawn(move || server.run());
    wait_listening(path);

    let stream_a = connect_local_stream(path).expect("client A");
    let mut writer_a = stream_a.try_clone().expect("clone");
    let mut reader_a = BufReader::new(stream_a);
    command(&mut writer_a, &mut reader_a, "new-session -s rescreen");
    command(&mut writer_a, &mut reader_a, "new-window");
    let second_pane =
        pane_ids(&command(&mut writer_a, &mut reader_a, "list-panes -t @1").join("")).remove(0);
    // Distinctive output on the second window's pane.
    command(
        &mut writer_a,
        &mut reader_a,
        &format!("send-keys -t {second_pane} -l 'echo PARMUXSYNC_OK'"),
    );
    command(
        &mut writer_a,
        &mut reader_a,
        &format!("send-keys -t {second_pane} Enter"),
    );

    // B registers, then sees the shared selection land on @1.
    let stream_b = connect_local_stream(path).expect("client B");
    let mut writer_b = stream_b.try_clone().expect("clone");
    let mut reader_b = BufReader::new(stream_b);
    command(&mut writer_b, &mut reader_b, "list-panes");
    command(&mut writer_a, &mut reader_a, "select-window -t @1");
    poll_until(
        &mut writer_b,
        &mut reader_b,
        "list-panes",
        "%session-window-changed $0 @1",
    );

    // The pump's follow, spelled over the wire: a pane resync (no -C)
    // replays the pane's screen-restore stream, which a REAL terminal
    // emulator reconstructs into a screen — no raw needle matching against
    // escape soup.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut rendered_ok = false;
    while Instant::now() < deadline {
        let replay = command(
            &mut writer_b,
            &mut reader_b,
            &format!("refresh-client -t {second_pane}"),
        )
        .join("\n");
        let mut term = par_term_emu_core::terminal::Terminal::new(120, 40);
        term.process(replay.as_bytes());
        if term.grid().content_as_string().contains("PARMUXSYNC_OK") {
            rendered_ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        rendered_ok,
        "the resync replay must render the switched window's screen in a real emulator"
    );
}
