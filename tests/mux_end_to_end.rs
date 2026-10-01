//! Phase 1 exit criterion: a control-mode client attaches over the socket,
//! creates a session, sends input, and receives `%output` PUSHED as the shell
//! produces it — the requirement a snapshot API cannot satisfy.

#![cfg(feature = "mux")]

use interprocess::TryClone as _;
use par_term_emu_core_rust::mux::{connect_local_stream, MuxServer};
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

/// A socket path inside a fresh `TempDir`: the directory name carries
/// OS-provided randomness, so no other test run can name the same path (a
/// pid-derived name repeats once the OS recycles the pid), and dropping the
/// returned guard removes the socket even when the test panics.
fn socket_path(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("par-mux-")
        .tempdir()
        .expect("create socket temp dir");
    let path = dir.path().join(format!("{tag}.sock"));
    (dir, path)
}

fn spawn_server(path: &std::path::Path) -> std::thread::JoinHandle<()> {
    let server = MuxServer::bind(path).expect("server binds");
    std::thread::spawn(move || server.run())
}

fn connect(
    path: &std::path::Path,
) -> (
    interprocess::local_socket::Stream,
    BufReader<interprocess::local_socket::Stream>,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match connect_local_stream(path) {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("server never accepted a connection: {e}"),
        }
    };
    let writer = stream.try_clone().expect("clone for writing");
    let reader = BufReader::new(stream);
    (writer, reader)
}

/// Read from `reader` until a complete `%begin`/`%end` block has arrived,
/// failing on `%error`. Returns the body lines between the brackets.
fn read_reply_block(reader: &mut BufReader<interprocess::local_socket::Stream>) -> Vec<String> {
    let mut saw_begin = false;
    let mut body = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read reply");
        assert!(n > 0, "server closed the connection mid-reply");
        let trimmed = line.trim_end();
        if trimmed.starts_with("%begin") {
            saw_begin = true;
        } else if trimmed.starts_with("%end") {
            assert!(saw_begin, "reply block ended before it began");
            return body;
        } else if trimmed.starts_with("%error") {
            panic!("command failed: {trimmed}");
        } else if saw_begin {
            body.push(trimmed.to_string());
        }
    }
}

#[test]
fn client_creates_a_session_and_receives_pushed_output() {
    let (_dir, path) = socket_path("e2e");
    let handle = spawn_server(&path);

    let (mut writer, mut reader) = connect(&path);

    // 1. Create a session. Expect a %begin/%end block.
    writeln!(writer, "new-session -s test").expect("write new-session");
    writer.flush().expect("flush");
    let body = read_reply_block(&mut reader);
    assert!(!body.is_empty(), "new-session replies with the session id");

    // 2. Send a command into the shell and expect its output to be PUSHED
    //    back as %output without us polling for it.
    writeln!(writer, "send-keys -t %0 'echo par-mux-marker' Enter").expect("write send-keys");
    writer.flush().expect("flush");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_marker = false;
    let mut line = String::new();
    while Instant::now() < deadline && !saw_marker {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => panic!("server closed the connection while awaiting output"),
            Ok(_) => {
                if line.starts_with("%output") && line.contains("par-mux-marker") {
                    saw_marker = true;
                }
            }
            Err(e) => panic!("read failed while awaiting output: {e}"),
        }
    }

    assert!(
        saw_marker,
        "expected a pushed %output line carrying the marker within 10s"
    );

    drop(writer);
    // run() serves until the listener closes; Phase 1 has no shutdown path
    // (Task 8's daemon owns lifecycle), so the server thread is detached and
    // dies with the test process.
    drop(handle);
}

#[test]
fn a_disconnecting_client_does_not_stop_the_server_or_touch_the_tree() {
    let (_dir, path) = socket_path("disconnect");
    let handle = spawn_server(&path);

    // Client A creates a session, then disconnects abruptly.
    let (mut writer_a, mut reader_a) = connect(&path);
    writeln!(writer_a, "new-session -s doomed").expect("A: new-session");
    writer_a.flush().expect("A: flush");
    read_reply_block(&mut reader_a);
    drop(writer_a);
    drop(reader_a);

    // The accept loop keeps running: client B connects and is served.
    let (mut writer_b, mut reader_b) = connect(&path);
    writeln!(writer_b, "new-session -s survivor").expect("B: new-session");
    writer_b.flush().expect("B: flush");
    read_reply_block(&mut reader_b);

    // The tree is untouched by A's disconnect: A's pane is still listed.
    writeln!(writer_b, "list-panes").expect("B: list-panes");
    writer_b.flush().expect("B: flush");
    let panes = read_reply_block(&mut reader_b);
    assert_eq!(
        panes.len(),
        2,
        "both sessions' panes survive the disconnect: {panes:?}"
    );

    drop(handle);
}

/// Read one reply block, capturing whether it closed with `%error` (the
/// shared helper panics on `%error`; the UTF-8 test asserts on it).
fn read_block_verdict(
    reader: &mut BufReader<interprocess::local_socket::Stream>,
) -> (bool, Vec<String>) {
    let mut saw_begin = false;
    let mut body = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read reply");
        assert!(n > 0, "server closed the connection mid-reply");
        let trimmed = line.trim_end();
        if trimmed.starts_with("%begin") {
            saw_begin = true;
        } else if trimmed.starts_with("%error") {
            assert!(saw_begin, "error block began: {trimmed}");
            return (false, body);
        } else if trimmed.starts_with("%end") {
            assert!(saw_begin, "reply block ended before it began");
            return (true, body);
        } else if saw_begin {
            body.push(trimmed.to_string());
        }
    }
}

#[test]
fn a_non_utf8_line_gets_an_error_reply_and_the_connection_survives() {
    let (_dir, path) = socket_path("utf8");
    let _handle = spawn_server(&path);
    let (mut writer, mut reader) = connect(&path);

    // A line of non-UTF-8 bytes (the send-keys -l with a Latin-1 payload
    // shape): read_line reports InvalidData for it, and the connection
    // must answer %error rather than dropping the client.
    writer
        .write_all(b"\xff\xfe caf\xe9\n")
        .expect("write non-UTF-8 line");
    writer.flush().expect("flush");
    let (ok, _) = read_block_verdict(&mut reader);
    assert!(!ok, "the undecodable line is a %error block, not a success");

    // The same connection serves the next, valid command.
    writer.write_all(b"version\n").expect("write version");
    writer.flush().expect("flush");
    let (ok, body) = read_block_verdict(&mut reader);
    assert!(ok, "version succeeds after the undecodable line");
    assert_eq!(
        body,
        vec![par_term_emu_core_rust::mux::build_stamp().to_string()],
        "the reply is version's own, on the same connection"
    );
}

/// SEC-127: a multi-byte char split across a recv-timeout wake survives.
/// `read_line` truncates the bytes it appended when the call errors and
/// they are not valid UTF-8, after the `BufReader` already consumed them,
/// so the old loop dropped the partial and answered `%error` for the tail.
#[test]
fn a_multibyte_char_split_across_a_poll_wake_survives() {
    let (_dir, path) = socket_path("utf8split");
    let _handle = spawn_server(&path);
    let (mut writer, mut reader) = connect(&path);

    writer
        .write_all(b"set-buffer caf\xc3")
        .expect("write the head");
    writer.flush().expect("flush");
    // More than two EVICTION_POLL (200 ms) recv-timeout wakes.
    std::thread::sleep(Duration::from_millis(450));
    writer.write_all(b"\xa9\n").expect("write the tail");
    writer.flush().expect("flush");
    let (ok, _) = read_block_verdict(&mut reader);
    assert!(ok, "the split line is one valid command");

    writer
        .write_all(b"show-buffer\n")
        .expect("write show-buffer");
    writer.flush().expect("flush");
    let (ok, body) = read_block_verdict(&mut reader);
    assert!(ok, "show-buffer succeeds");
    assert_eq!(body, vec!["café".to_string()], "the char arrived whole");
}

/// ARC-022: the pane spawn (fork/exec + reader start) runs OFF the tree
/// lock. A factory that stalls mid-spawn proves it: while client A's
/// new-session is still inside the factory, client B's list-panes must
/// answer — under the single-lock flow it queued behind the spawn for the
/// whole stall, and so would every other command in the daemon.
#[test]
fn a_slow_spawn_does_not_stall_other_clients() {
    use par_term_emu_core_rust::mux::pane::{
        MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext,
    };
    use par_term_emu_core_rust::mux::{MuxServer, MuxTree, PaneId};

    /// The real factory behind a gate in `create_pane`: it PROVES the
    /// dispatcher reached the spawn by sending on `entered`, then parks
    /// until released. The bounded release wait turns a lost release into
    /// a factory error instead of a hung daemon worker.
    struct GatedFactory {
        inner: ShellPaneFactory,
        entered: std::sync::mpsc::Sender<()>,
        /// `Mutex` supplies the `Sync` `PaneFactory` demands; one waiter.
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl PaneFactory for GatedFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            let _ = self.entered.send(());
            let _ = self
                .release
                .lock()
                .expect("release gate mutex")
                .recv_timeout(Duration::from_secs(30));
            self.inner.create_pane(id, cols, rows, command, context)
        }

        fn create_dead_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            exit_code: Option<i32>,
        ) -> Result<MuxPane, MuxError> {
            self.inner
                .create_dead_pane(id, cols, rows, command, exit_code)
        }
    }

    let (_dir, path) = socket_path("slowspawn");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let tree = MuxTree::new(Box::new(GatedFactory {
        inner: ShellPaneFactory::default(),
        entered: entered_tx,
        release: std::sync::Mutex::new(release_rx),
    }));
    let server = MuxServer::bind_with_tree(&path, tree).expect("server binds");
    let handle = std::thread::spawn(move || server.run());

    // Client A starts a session whose pane spawn parks in the gate. Its
    // reply arrives only after the release, so drive A from its own thread.
    let (mut writer_a, mut reader_a) = connect(&path);
    let session_a = std::thread::spawn(move || {
        writeln!(writer_a, "new-session -s slow").expect("A: write new-session");
        writer_a.flush().expect("A: flush");
        read_reply_block(&mut reader_a)
    });

    // Deterministic readiness: the dispatcher has REACHED the factory —
    // no sleep, no clock.
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the dispatcher never reached the factory");

    // While the spawn is parked in the gate, client B's list-panes must
    // answer. Old behavior: B queues on the tree lock behind the spawn, so
    // B's reply can only exist once the gate opens — the ordering is the
    // assertion, no elapsed-time budget involved.
    let (mut writer_b, mut reader_b) = connect(&path);
    writeln!(writer_b, "list-panes").expect("B: write list-panes");
    writer_b.flush().expect("B: flush");
    let (body_tx, body_rx) = std::sync::mpsc::channel();
    let mut reader_bg = BufReader::new(
        reader_b
            .get_mut()
            .try_clone()
            .expect("clone B's reader for the gated check"),
    );
    std::thread::spawn(move || {
        let body = read_reply_block(&mut reader_bg);
        let _ = body_tx.send(body);
    });
    let body_b = body_rx.recv_timeout(Duration::from_secs(10)).expect(
        "list-panes did not answer while the spawn was parked in the gate; \
         the spawn must run off the tree lock",
    );
    // Two-phase semantics: the pane is invisible until the insert lands —
    // an empty body is the correct answer mid-spawn.
    assert!(
        body_b.is_empty(),
        "no pane exists yet while the spawn is in flight: {body_b:?}"
    );

    // The gated spawn still completes and becomes visible.
    release_tx
        .send(())
        .expect("the factory never arrived at its release wait");
    session_a.join().expect("A: session thread");
    writeln!(writer_b, "list-panes").expect("B: write list-panes again");
    writer_b.flush().expect("B: flush again");
    let body_after = read_reply_block(&mut reader_b);
    assert_eq!(
        body_after,
        vec!["%0".to_string()],
        "pane %0 exists after the spawn lands"
    );

    drop(handle);
}

/// ARC-103: a new pane's first output is pushed to clients even when the
/// dispatcher is still between the spawn and completing it. The sink rides
/// the spawn context, so it is live before the reader thread's first read;
/// before the fix it was installed only after the insert, and bytes the
/// pane printed in between reached the daemon grid but never a client.
#[cfg(unix)]
#[test]
fn a_new_panes_first_output_is_pushed_even_when_wiring_lags() {
    use par_term_emu_core_rust::mux::pane::{
        MuxError, MuxPane, PaneFactory, ShellPaneFactory, SpawnContext,
    };
    use par_term_emu_core_rust::mux::{MuxClient, MuxServer, MuxTree, PaneId};
    use par_term_emu_core_rust::tmux_control::TmuxNotification;

    /// Spawns a pane that prints a marker at once, then holds the
    /// dispatcher in the factory while the pane's output arrives.
    struct LaggingFactory {
        inner: ShellPaneFactory,
    }
    impl PaneFactory for LaggingFactory {
        fn create_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            _command: Option<&str>,
            context: &SpawnContext<'_>,
        ) -> Result<MuxPane, MuxError> {
            let pane = self.inner.create_pane(
                id,
                cols,
                rows,
                Some("printf EARLY-MARK; exec sleep 30"),
                context,
            )?;
            std::thread::sleep(Duration::from_millis(500));
            Ok(pane)
        }

        fn create_dead_pane(
            &self,
            id: PaneId,
            cols: u16,
            rows: u16,
            command: Option<&str>,
            exit_code: Option<i32>,
        ) -> Result<MuxPane, MuxError> {
            self.inner
                .create_dead_pane(id, cols, rows, command, exit_code)
        }
    }

    let (_dir, path) = socket_path("lagwire");
    let tree = MuxTree::new(Box::new(LaggingFactory {
        inner: ShellPaneFactory::default(),
    }));
    let server = MuxServer::bind_with_tree(&path, tree).expect("server binds");
    let _handle = std::thread::spawn(move || server.run());
    wait_for_socket(&path);

    // Clients join broadcasts on their first control command, so the
    // observer registers before the pane exists.
    let mut observer = MuxClient::connect(&path).expect("observer connects");
    observer.send("list-sessions").expect("observer registers");
    let mut issuer = MuxClient::connect(&path).expect("issuer connects");
    issuer.send("new-session -s lag").expect("new-session");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = String::new();
    while !seen.contains("EARLY-MARK") {
        let left = deadline.saturating_duration_since(Instant::now());
        match observer.notifications().recv_timeout(left) {
            Ok(TmuxNotification::Output { pane_id, data }) if pane_id == "%0" => {
                seen.push_str(&String::from_utf8_lossy(&data));
            }
            Ok(_) => {}
            Err(_) => panic!("the pane's first output was never pushed: {seen:?}"),
        }
    }
}

/// Wait until the server's socket accepts a connection.
#[cfg(unix)]
fn wait_for_socket(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while connect_local_stream(path).is_err() {
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(25));
    }
}
