//! Attach-client unit tests, split by topic. The scripted fake-daemon
//! helpers shared across suites live here.

use super::*;
use crate::mux::{emit_block, MuxServer};
// The Listener trait supplies `.accept()` on unix; the Windows named-pipe
// listener accepts via its own inherent impl, so this import is unused there.
#[cfg(unix)]
use interprocess::local_socket::traits::Listener as _;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

mod client_config_and_io;
mod handshake;
mod help_and_sidebar;
mod management_chords;
mod panels_and_reload;
mod passthrough_switching;
mod prefix_and_resync;
mod replay_fence;
mod status_snapshot_and_targets;
mod targets;

/// A temp-dir socket path unique to this test process.
fn test_socket(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-attach-{}-{tag}.sock", std::process::id()));
    path
}

/// A hand-rolled daemon substitute: accepts one connection, records
/// every control line it receives (in order), and answers each with a
/// scripted reply chosen by the command name. Replies use the real
/// wire framing (`emit_block`) so the client parser sees the protocol
/// exactly as the daemon writes it. The listener is consumed by the
/// accept thread; the socket path is what the test connects to.
struct FakeDaemon {
    /// (command name, full command line) per control line, in order.
    received: Receiver<(String, String)>,
}

impl FakeDaemon {
    fn bind(tag: &str) -> (Self, PathBuf) {
        let path = test_socket(tag);
        let _ = std::fs::remove_file(&path);
        let listener = crate::mux::bind_local_listener(&path).expect("bind fake daemon");
        let (tx, received) = channel();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                serve_one(stream, tx);
            }
        });
        (Self { received }, path)
    }
}

/// Serve one connection: record lines, answer per the scripted table.
fn serve_one(stream: crate::mux::LocalStream, tx: std::sync::mpsc::Sender<(String, String)>) {
    use interprocess::TryClone as _;
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    let mut number = 0u32;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
        let reply = match name.as_str() {
            "version" => "9.9.9+deadbeef".to_string(),
            "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
            "list-panes" => "%0\n".to_string(),
            "list-sessions" => "$0: default\n".to_string(),
            "list-windows" => "@0 * default\n".to_string(),
            // The resync's scripted restore body: absolute row
            // addressing like the real encoder's, ending with the
            // pane's real cursor CUP (row 2, col 9 — right after
            // "prompt> "). No trailing newline, like the real stream.
            "refresh-client" if trimmed.contains("-t %0") => {
                "\x1b[1;1Hfirst\x1b[2;1Hprompt> \x1b[2;9H".to_string()
            }
            _ => String::new(),
        };
        number += 1;
        tx.send((name, trimmed.to_owned())).ok();
        writer
            .write_all(emit_block(number, &reply, true).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// Waits for the recorded lines, tolerating the reader thread's delay.
fn recorded(rx: &Receiver<(String, String)>, n: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for _ in 0..n {
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(line) => out.push(line),
            Err(_) => break,
        }
    }
    out
}

/// The next recorded line whose command name is `name`, draining any
/// interleaved status re-queries first — the robust needle when the
/// chord's resync burst has a variable tail.
fn wait_for_line(rx: &Receiver<(String, String)>, name: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("no {name} line arrived within the bound");
        }
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok((n, line)) if n == name => return line,
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("no {name} line: the daemon side is gone");
            }
        }
    }
}

/// The scripted-reply daemon the management-chord tests share: like
/// [`serve_one`] but with a table covering split-window (reply = new
/// pane id), list-panes under both windows, select-pane,
/// new-window (reply = window id), and select-window.
fn serve_management(
    stream: crate::mux::LocalStream,
    tx: std::sync::mpsc::Sender<(String, String)>,
) {
    use interprocess::TryClone as _;
    let mut writer = stream.try_clone().expect("clone stream");
    let mut reader = BufReader::new(stream);
    let mut number = 0u32;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
        let reply = match trimmed {
            "version" => "9.9.9+deadbeef".to_string(),
            "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
            "list-panes" => "%0\n".to_string(),
            // The focused pane %0 splits: the new pane %1 arrives.
            "split-window -t %0 -h" => "%1\n".to_string(),
            "split-window -t %0" => "%1\n".to_string(),
            // After the split lands the pump on %1, the window @0
            // holds both panes with %1 active (the daemon focuses the
            // fresh split).
            "list-panes -t @0" => "%0 0 -\n%1 1 *".to_string(),
            "select-pane -t %1" => String::new(),
            "select-pane -t %0" => String::new(),
            // new-window in session $0: the fresh @1.
            "new-window -t $0" => "@1\n".to_string(),
            "select-window -t @1" => String::new(),
            "list-windows -t $0" => "@0 0 -\n@1 1 *".to_string(),
            // The new window's active pane: %1 (a distinct id keeps
            // the landing assertion honest — %0 is @0's pane).
            "list-panes -t @1" => "%1 0 *".to_string(),
            _ => String::new(),
        };
        number += 1;
        tx.send((name, trimmed.to_owned())).ok();
        writer
            .write_all(emit_block(number, &reply, true).as_bytes())
            .ok();
        writer.flush().ok();
    }
}

/// Bind the management fake and build a Session over it, focused on
/// %0 in window @0 of session $0.
fn management_session(tag: &str) -> (std::sync::mpsc::Receiver<(String, String)>, Session) {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = channel();
    let sender = tx.clone();
    std::thread::spawn(move || {
        if let Ok(stream) = listener.accept() {
            serve_management(stream, sender);
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    drop(tx);
    let mut session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
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
        output_fence: None,
    };
    // A status draw writes to the process stdout; in tests that is
    // the captured harness. Idle both flags so route_bytes's tail
    // has nothing to repaint and the wire sees only the chord.
    session.drawn_size = Some((80, 24));
    (rx, session)
}

/// A passthrough Session over a fake daemon answering each exact
/// command line from `replies` (everything else an ok empty reply),
/// focused on %0 in window @0 of session $0; the handshake's lines
/// are drained before it returns, so the receiver holds only what
/// the test drives.
fn scripted_passthrough(
    tag: &str,
    replies: &[(&str, &str)],
) -> (std::sync::mpsc::Receiver<String>, Session) {
    let path = test_socket(tag);
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let table: std::collections::HashMap<String, String> = replies
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let Ok(stream) = listener.accept() else {
            return;
        };
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone stream");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let reply = match trimmed {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                other => table.get(other).cloned().unwrap_or_default(),
            };
            number += 1;
            tx.send(trimmed.to_owned()).ok();
            // A scripted `!err` reply answers with an error block.
            let ok = reply != "!err";
            let reply = if ok { reply } else { String::new() };
            writer
                .write_all(emit_block(number, &reply, ok).as_bytes())
                .ok();
            writer.flush().ok();
        }
    });
    let conn = conn::AttachConn::connect(&path).expect("connect");
    let session = Session {
        conn,
        socket_path: path,
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: "@0".to_string(),
        session_id: Some("$0".to_string()),
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: Some((80, 24)),
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
        output_fence: None,
    };
    let _: Vec<String> = rx.try_iter().collect();
    (rx, session)
}

/// A passthrough Session literal bound to `conn`, showing `window`/`pane`.
fn passthrough_session(conn: conn::AttachConn, window: &str, pane: &str) -> Session {
    Session {
        conn,
        socket_path: PathBuf::from("/nonexistent"),
        pane: pane.to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: window.to_string(),
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
        output_fence: None,
    }
}
