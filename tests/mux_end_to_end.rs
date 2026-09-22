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
