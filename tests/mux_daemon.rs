//! The daemon requirement: panes outlive their clients, and two starts race safely.

#![cfg(feature = "mux")]

use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::{connect_local_stream, prepare_socket_path, MuxServer};
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

fn socket(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-daemon-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    path
}

/// Run one command and drain its `%begin`…`%end` block.
fn command(stream: &mut impl Write, reader: &mut impl BufRead, line: &str) -> Vec<String> {
    writeln!(stream, "{line}").expect("write command");
    stream.flush().expect("flush");
    let mut out = Vec::new();
    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf).expect("read reply");
        assert!(n > 0, "server closed while answering {line:?}");
        let done = buf.starts_with("%end") || buf.starts_with("%error");
        out.push(buf);
        if done {
            return out;
        }
    }
}

#[test]
fn panes_survive_every_client_disconnecting() {
    let path = socket("survive");
    let server = MuxServer::bind(&path).expect("bind");
    let handle = std::thread::spawn(move || server.run());

    // First client: create a session, then disconnect entirely.
    let pane_line = {
        let stream = connect_local_stream(&path).expect("first connect");
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        command(&mut writer, &mut reader, "new-session -s survive");
        let listed = command(&mut writer, &mut reader, "list-panes");
        listed.join("")
        // Both ends drop here — the client is gone.
    };
    assert!(pane_line.contains('%'), "a pane was created: {pane_line}");

    // Give the server a moment to notice the disconnect and (wrongly) exit.
    std::thread::sleep(Duration::from_millis(300));

    // Second client: the session must still be there.
    let stream = connect_local_stream(&path).expect(
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

    let _ = std::fs::remove_file(&path);
    drop(writer);
    let _ = handle;
}

#[test]
fn prepare_socket_path_makes_auto_spawn_race_safe() {
    let path = socket("race");
    let server = MuxServer::bind(&path).expect("first bind wins");
    let _handle = std::thread::spawn(move || server.run());

    // Wait for the listener to be reachable.
    let deadline = Instant::now() + Duration::from_secs(5);
    while connect_local_stream(&path).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }

    // A second would-be daemon must refuse rather than steal the path.
    let err =
        prepare_socket_path(&path).expect_err("a second daemon must not reclaim a live socket");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);

    let _ = std::fs::remove_file(&path);
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
    let target = std::env::temp_dir().join(format!(
        "par-mux-daemon-restore-{}.state.json",
        std::process::id()
    ));
    save_to(&tree, &target).expect("save");
    let state = match load_or_quarantine(&target) {
        Loaded::State(state) => state,
        other => panic!("expected a readable state file, got {other:?}"),
    };
    let _ = std::fs::remove_file(&target);
    let restored = MuxTree::from_persist_state(&state, Box::new(ShellPaneFactory::default()))
        .expect("state rebuilds");

    let path = socket("restore");
    let server = MuxServer::bind_with_tree(&path, restored).expect("bind");
    std::thread::spawn(move || server.run());

    let stream = connect_local_stream(&path).expect("connect");
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

    let _ = std::fs::remove_file(&path);
}
