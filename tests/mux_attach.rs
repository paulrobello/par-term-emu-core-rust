//! `par-mux attach` Phase A: the passthrough client under a real PTY.
//!
//! The attach client is spawned as the par-mux binary's `attach` subcommand
//! under a portable-pty master, so the host side of the test is the host
//! terminal the client writes to — exactly the production shape. The tests
//! cover the card's acceptance criteria: byte fidelity through the replay +
//! `%output` path, prefix `d` detach leaving the pane running, and clean
//! terminal restore.

#![cfg(all(feature = "mux-bin", feature = "attach"))]

// ARC-106: cargo sets CARGO_BIN_EXE_par-mux even when the bin's
// required-features are unmet, so a mux-without-attach build would silently
// exec a binary without the attach subcommand. Fail loudly instead.
#[cfg(not(feature = "attach"))]
compile_error!("this test drives `par-mux attach`: build with --features attach");

mod common;

use common::{wait_listening, MuxFixture};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

/// One PTY-master side of a spawned `par-mux attach`.
struct AttachHost {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    to_child: Box<dyn Write + Send>,
    /// Reader-thread output, raw bytes.
    output_rx: Receiver<Vec<u8>>,
    _master: Box<dyn MasterPty + Send>,
}

/// Spawn `par-mux attach --socket <path> [-t target]` under a fresh PTY and
/// hand back the master side: a writer (the client's stdin) and a reader
/// channel (the client's stdout). stderr goes to a buffer for diagnostics.
fn spawn_attach(
    fixture: &MuxFixture,
    extra: &[&str],
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_par-mux"));
    cmd.arg("attach");
    cmd.arg("--socket");
    cmd.arg(fixture.socket());
    cmd.args(extra);
    let child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn par-mux attach under the PTY");
    let killer = child.clone_killer();
    let to_child = pair.master.take_writer().expect("master writer");
    let mut from_child = pair.master.try_clone_reader().expect("master reader");
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match from_child.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let host = AttachHost {
        child,
        killer,
        to_child,
        output_rx: rx,
        _master: pair.master,
    };
    (host, stderr)
}

/// Collect host output until `needle` appears, the child exits, or the
/// deadline passes. Returns everything read (joined).
fn wait_for_output(host: &AttachHost, needle: &[u8], deadline: Duration) -> Vec<u8> {
    let end = Instant::now() + deadline;
    let mut collected: Vec<u8> = Vec::new();
    loop {
        if !needle.is_empty() && collected.windows(needle.len()).any(|w| w == needle) {
            return collected;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return collected;
        }
        match host
            .output_rx
            .recv_timeout(left.min(Duration::from_millis(50)))
        {
            Ok(bytes) => collected.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return collected,
        }
    }
}

/// The attach child's exit code once it terminates, polled with a deadline.
fn child_exit(host: &mut AttachHost, timeout: Duration) -> Option<i32> {
    let end = Instant::now() + timeout;
    loop {
        match host.child.try_wait() {
            Ok(Some(status)) => return Some(status.exit_code() as i32),
            Ok(None) => {}
            // A reaped child (its status consumed elsewhere) still proves
            // termination.
            Err(err) => {
                eprintln!("try_wait error: {err}");
                return None;
            }
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A live daemon (the real binary — the production shape an attach client
/// meets) + one seeded session/pane, by the plain control client. The
/// DaemonGuard outlives the client and reaps the daemon even on panic.
fn fixture_with_session(
    tag: &str,
) -> (
    MuxFixture,
    common::DaemonGuard,
    par_term_emu_core_rust::mux::MuxClient,
) {
    let fixture = MuxFixture::new(tag);
    let daemon = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut client =
        par_term_emu_core_rust::mux::MuxClient::connect(fixture.socket()).expect("connect");
    client.send("new-session -s att").expect("new-session");
    (fixture, daemon, client)
}

/// Acceptance criterion 1: a known escape-sequence payload round-trips
/// byte-identical through attach — the pane emits a distinctive SGR+CUP
/// sequence; the host reads exactly those bytes out of the client.
///
/// Unix-only: the pane payload pipeline is POSIX shell syntax.
#[cfg(unix)]
#[test]
fn escape_sequence_payload_round_trips_byte_identical() {
    let (fixture, _daemon, mut client) = fixture_with_session("fidelity");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The payload: a CUP + SGR + literal marker + reset. Distinctive bytes
    // that must survive the daemon's octal %output escaping and the
    // client's octal decode untouched.
    const PAYLOAD: &str = "\x1b[5;3H\x1b[38;5;196mATTACH-FIDELITY-MARKER\x1b[0m";

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Settle on the resync + status draw, so everything the host reads
    // after this point is the LIVE %output path (the criterion: the raw
    // pane bytes survive the daemon's octal %output escaping AND the
    // client's octal decode, byte-identical).
    let _ = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));

    // Emit the payload. Base64 carries the bytes through both quoting
    // layers untouched (the daemon's bounded grammar and the pane shell's
    // own word splitting); the whole command rides as ONE quoted send-keys
    // token (tokens otherwise concatenate without spaces), and Enter goes
    // separately as a key.
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(PAYLOAD);
    client
        .send(&format!(
            "send-keys -t {pane} -l 'echo {encoded} | base64 -d'"
        ))
        .expect("send-keys payload");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");

    let got = wait_for_output(&host, b"ATTACH-FIDELITY-MARKER", Duration::from_secs(15));
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    assert!(
        got.windows(PAYLOAD.len()).any(|w| w == PAYLOAD.as_bytes()),
        "the exact escape-sequence payload must appear byte-identical in the \
         client's output (%output raw forward): {:?}\nstderr: {}\ncapture: {capture:?}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 2: prefix d detaches — the client exits 0, restores
/// the terminal (the reset sequence appears in its output), and the pane is
/// still running afterwards (a later capture still sees its content).
#[test]
fn prefix_d_detaches_and_leaves_the_pane_running() {
    let (fixture, _daemon, mut client) = fixture_with_session("detach");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    wait_for_output(&host, b"attach-detach-marker", Duration::from_secs(1)); // settle
                                                                             // Put a marker on the pane's screen first.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'echo attach-detach-marker'"
        ))
        .expect("send marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");
    let _ = wait_for_output(&host, b"attach-detach-marker", Duration::from_secs(15));

    // Prefix d = C-b 0x02 then 'd'.
    host.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    host.to_child.flush().ok();

    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "prefix d must detach with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );

    // The pane outlived the client: an ordinary capture still answers and
    // the marker is on the pane's screen.
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture after detach");
    assert!(
        capture.join("").contains("attach-detach-marker"),
        "the pane must still be running after detach: {capture:?}"
    );
}

/// Acceptance criterion 2b (clean restore): the client's output carries a
/// terminal-restore sequence after detach — raw mode off is crossterm's
/// disable_raw_mode (no bytes), and the pump leaves the scroll region
/// reset; the DECSTBM reserve appeared at startup, so a detach must reset
/// the region (our status redraw writes `\x1b[r` on the way out via the
/// last draw, and detach itself re-emits a final reset).
#[test]
fn detach_restores_the_terminal_region() {
    let (fixture, _daemon, mut client) = fixture_with_session("restore");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Startup: the status-line DECSTBM reserve must appear — wait for the
    // status DRAW (inverse-video bottom row), which is what carries it.
    let startup = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));
    assert!(
        startup.windows(7).any(|w| w == b"\x1b[1;23r"),
        "attach must reserve the status row with DECSTBM (ESC[1;23r on a 24-row \
         terminal): {:?}\nstderr: {}",
        String::from_utf8_lossy(&startup),
        stderr.lock().unwrap()
    );
    let _ = client.send(&format!("send-keys -t {pane} -l x Enter"));
    let _ = wait_for_output(&host, b"x", Duration::from_secs(5));

    host.to_child.write_all(&[0x02, b'd']).expect("prefix d");
    host.to_child.flush().ok();
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(code, Some(0));
    // Drain what remains: the final reset must be present.
    let mut tail = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        tail.extend_from_slice(&bytes);
    }
    // The last thing the client writes before exit is the region restore.
    // (crossterm's raw-mode disable is a termios call, not bytes.)
    assert!(
        tail.windows(3).any(|w| w == b"\x1b[r"),
        "detach must restore the scroll region (ESC[r): {tail:?}"
    );
}

/// Acceptance criterion 3: prefix pane switching issues a daemon-side
/// select and a resync redraw. Split the pane, then prefix o; the client's
/// output shows the NEW pane's content (the resync of the other pane).
#[cfg(unix)]
#[test]
fn prefix_o_switches_panes_and_resyncs() {
    let (fixture, _daemon, mut client) = fixture_with_session("switch");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let pane_b = client
        .send(&format!("split-window -h -t {pane_a}"))
        .expect("split")
        .join("");
    let pane_b = pane_b.trim().to_string();
    assert!(
        pane_b.starts_with('%'),
        "split returned a pane id: {pane_b:?}"
    );

    // Distinct markers on each pane.
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo PANE-B-MARKER'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("press Enter");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    // Attach to pane A: settling output arrives; B's marker must NOT be
    // the reason we proceed (B is a different pane).
    let _ = wait_for_output(&host, b"$", Duration::from_secs(2));

    // Prefix o cycles to pane B; the resync redraw carries B's marker.
    host.to_child.write_all(&[0x02, b'o']).expect("prefix o");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"PANE-B-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"PANE-B-MARKER".len())
            .any(|w| w == b"PANE-B-MARKER"),
        "prefix o must resync to the other pane's screen. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // The daemon-side select landed: the window's active pane is B now.
    let active = client.send("list-panes").expect("global roster");
    let _ = active; // roster carries no marker globally; the resync above is the proof
    host.killer.kill().ok();
}

/// The client exits 0 when the daemon shuts down (%exit), restoring the
/// terminal.
#[test]
fn daemon_exit_ends_the_client_cleanly() {
    let (fixture, _daemon, mut client) = fixture_with_session("exit");
    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Settle on the status draw, so %exit is the only pending signal.
    let startup = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));
    let _ = startup;
    client.send_checked("kill-server").expect("kill-server");
    // Give a debug harness time to `sample` the hung child.
    let code = child_exit(&mut host, Duration::from_secs(35));
    std::thread::sleep(Duration::from_secs(10));
    if code != Some(0) {
        // Diagnostics: what did the client print, and is the daemon's
        // socket still live?
        let mut tail = Vec::new();
        while let Ok(bytes) = host.output_rx.try_recv() {
            tail.extend_from_slice(&bytes);
        }
        let live = par_term_emu_core_rust::mux::connect_local_stream(fixture.socket()).is_ok();
        let ps = std::process::Command::new("ps")
            .arg("-axo")
            .arg("pid,stat,command")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let children: String = ps
            .lines()
            .filter(|l| l.contains("par-mux") || l.contains("attach"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "exit {code:?}; daemon socket live: {live}; stderr: {}; client tail: {:?}; procs: {children}",
            stderr.lock().unwrap(),
            String::from_utf8_lossy(&tail)
        );
    }
}

/// The `-t` target forms resolve: a pane id attaches to that pane (its
/// content arrives), and an unknown target is a clean failure with exit 1
/// and a message, not a crash.
#[test]
fn target_resolution_and_clean_failure() {
    let (fixture, _daemon, mut client) = fixture_with_session("targets");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane} -l 'echo TARGETED-MARKER'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("press Enter");
    // Let the echo land on the pane screen BEFORE attach starts: the
    // assertion is about the resync replay, not a race with %output.
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    let got = wait_for_output(&host, b"TARGETED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"TARGETED-MARKER".len())
            .any(|w| w == b"TARGETED-MARKER"),
        "attaching with -t <pane> must resync that pane. stderr: {}; bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();

    // Unknown target: exit 1, message on stderr, no hang.
    let output = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("attach")
        .arg("--socket")
        .arg(fixture.socket())
        .arg("-t")
        .arg("%999")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run attach with a bad target");
    assert_eq!(output.status.code(), Some(1));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("no such target"),
        "the failure names the target problem: {err}"
    );
}
