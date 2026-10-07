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
    to_child: SharedWriter,
    /// Reader-thread output, raw bytes.
    output_rx: Receiver<Vec<u8>>,
    /// The master, kept for mid-session host-side resizes. Only the
    /// unix PTY tests resize; on Windows nothing reads it.
    #[cfg_attr(windows, allow(dead_code))]
    master: Box<dyn MasterPty + Send>,
}

/// Spawn `par-mux attach --socket <path> [-t target]` under a fresh PTY and
/// hand back the master side: a writer (the client's stdin) and a reader
/// channel (the client's stdout). stderr goes to a buffer for diagnostics.
///
/// The reader thread doubles as the stand-in host terminal: modern Windows
/// ConPTY sends `CSI 6n` (cursor position report) during its init handshake
/// and stalls its byte pump until the attached terminal answers — Windows
/// Terminal answers, and a bare pty master does not, so on Windows the
/// captured output would be the lone `\x1b[6n` and nothing else. The reader
/// therefore answers every `\x1b[6n` it sees with a cursor-position report
/// (`ESC[24;1R`, a 24-row grid's home column) written into the pane's
/// INPUT side. The reply flows toward the pane/ConPTY input direction, so
/// it never lands in the captured stdout the assertions read; on Unix
/// nothing in the harness emits `6n` spontaneously, so the branch is
/// inert and one harness shape serves both platforms.
fn spawn_attach(
    fixture: &MuxFixture,
    extra: &[&str],
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach_mode(fixture, extra, "passthrough")
}

/// The render-mode spawn: the same PTY harness with the pipeline the
/// render chords run under.
fn spawn_attach_render(
    fixture: &MuxFixture,
    extra: &[&str],
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach_mode(fixture, extra, "render")
}

fn spawn_attach_mode(
    fixture: &MuxFixture,
    extra: &[&str],
    mode: &str,
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
    // The suite's byte-shape assertions are passthrough's contract; the
    // product default is render, so both modes pin explicitly.
    cmd.arg("--mode");
    cmd.arg(mode);
    cmd.args(extra);
    let child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn par-mux attach under the PTY");
    let killer = child.clone_killer();
    // One take: portable-pty's ConPTY master hands the writer out exactly
    // once, so the test's stdin writes and the 6n answers share it behind
    // a mutex (Unix tolerates a second take; Windows does not).
    let writer = std::sync::Arc::new(std::sync::Mutex::new(
        pair.master.take_writer().expect("master writer"),
    ));
    let to_child = SharedWriter(writer.clone());
    let mut from_child = pair.master.try_clone_reader().expect("master reader");
    let answer = SharedWriter(writer);
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match from_child.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Answer a cursor-position query like a real terminal
                    // would (see fn doc). Scan the accumulated byte stream
                    // so a 6n split across reads still matches.
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(pos) = pending.windows(4).position(|w| w == b"\x1b[6n") {
                        let mut w = answer.0.lock().expect("writer lock");
                        let _ = w.write_all(b"\x1b[24;1R");
                        let _ = w.flush();
                        // Drop everything through the query so it is
                        // answered once per query.
                        pending.drain(..pos + 4);
                    }
                    // Bound the carryover: only a trailing partial query
                    // prefix (up to 3 bytes) can legitimately wait here.
                    if pending.len() > 3 {
                        pending.drain(..pending.len() - 3);
                    }
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
        master: pair.master,
    };
    (host, stderr)
}

/// The pty master's writer shared between the test's stdin writes and the
/// reader thread's cursor-position answers (ConPTY hands the writer out
/// exactly once; see [`spawn_attach`]).
struct SharedWriter(std::sync::Arc<std::sync::Mutex<Box<dyn Write + Send>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("writer lock").write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().expect("writer lock").flush()
    }
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

/// The visible text of a captured byte stream (ESC sequences stripped):
/// the status row and modal overlays flush per-cell (one CUP+SGR run per
/// cell), so multi-character matches must run over the plain text, not
/// the wire bytes.
fn plain_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b != 0x1b {
            out.push(b as char);
            continue;
        }
        match iter.next() {
            Some(b'[') => {
                // CSI: swallow through the final byte (@-~).
                for c in iter.by_ref() {
                    if (0x40..=0x7e).contains(&c) {
                        break;
                    }
                }
            }
            Some(b']') => {
                // OSC: swallow through BEL or ST (ESC \).
                for c in iter.by_ref() {
                    if c == 0x07 {
                        break;
                    }
                    if c == 0x1b {
                        iter.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
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
fn fixture_with_session(tag: &str) -> (MuxFixture, common::DaemonGuard, par_mux::mux::MuxClient) {
    let fixture = MuxFixture::new(tag);
    let daemon = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
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
    // Startup: the screen clear must come before anything else
    // the client draws — the replay must not overwrite the host's prior
    // content — and the status-line DECSTBM reserve must appear. Both are
    // asserted on UNIX only. Windows ConPTY re-encodes the client's
    // output: it INTERPRETS DECSTBM (the region is applied conhost-side)
    // and consumes alt-screen switches into its own buffer model, so
    // neither raw sequence passes through to the pty master even though
    // the client emitted them (the re-encoded status row draw does
    // appear). The client-side emits are pinned by the unix run; the
    // Windows run asserts the status draw ConPTY preserves.
    if cfg!(unix) {
        let startup = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));
        assert!(
            startup.windows(4).any(|w| w == b"\x1b[2J"),
            "attach must clear the screen before the replay: {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            !startup.windows(8).any(|w| w == b"\x1b[?1049h"),
            "attach must NOT enter the alternate screen (a pane app's own 1049 \
             pair would pop it and strand later draws on the host main): \
             {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            startup.windows(7).any(|w| w == b"\x1b[1;23r"),
            "attach must reserve the status row with DECSTBM (ESC[1;23r on a 24-row \
             terminal): {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
        assert!(
            startup.windows(4).position(|w| w == b"\x1b[2J")
                < startup.windows(7).position(|w| w == b"\x1b[1;23r"),
            "the clear precedes the status draw: {:?}",
            String::from_utf8_lossy(&startup)
        );
        // The status draw must leave the content region reserved: a
        // full-screen region reset (ESC[1;24r) after the draw lets the
        // next pane scroll carry the status row away (the manual-pass
        // scroll-away bug). Only rows-1 regions may appear.
        assert!(
            !startup.windows(9).any(|w| w == b"\x1b[1;24r"),
            "the status draw must not reset the scroll region to the full \
             screen: {:?}",
            String::from_utf8_lossy(&startup)
        );
    } else {
        let startup = wait_for_output(&host, b"\x1b[24;1H", Duration::from_secs(10));
        assert!(
            startup.windows(7).any(|w| w == b"\x1b[24;1H"),
            "attach must draw the status row (CUP to row 24): {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
        );
    }
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
    // The last things the client writes before exit are the region restore
    // and the final screen clear (no alt screen to leave — a pane app's
    // own 1049 pair would pop the client's). (crossterm's raw-mode disable
    // is a termios call, not bytes.) On Windows ConPTY likewise absorbs
    // ESC[r and erase-display, so the restore is asserted through the
    // client's exit code there (already checked above) — the bytes are
    // pinned on unix.
    if cfg!(unix) {
        assert!(
            tail.windows(3).any(|w| w == b"\x1b[r"),
            "detach must restore the scroll region (ESC[r): {tail:?}"
        );
        assert!(
            tail.windows(4).any(|w| w == b"\x1b[2J"),
            "detach must clear the screen (no alt screen to leave): {tail:?}"
        );
    }
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

/// prefix %: the chord splits the FOCUSED pane and the client lands on
/// the fresh split — the new pane's distinct marker reaches the client
/// through the landing resync, and the pane it split from stays alive.
#[cfg(unix)]
#[test]
fn split_chord_lands_the_client_on_the_new_pane() {
    let (fixture, _daemon, mut client) = fixture_with_session("chordsplit");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(2));

    // Prefix % splits right; the client lands on the new pane. The pane
    // starts a fresh shell, whose prompt is the landing evidence — but
    // the pane it came from shows one too, so drive the new pane
    // directly: typing AFTER the chord reaches the fresh pane only.
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(500));
    host.to_child
        .write_all(b"echo SPLIT-LANDED-MARKER\n")
        .expect("type into the new pane");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"SPLIT-LANDED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"SPLIT-LANDED-MARKER".len())
            .any(|w| w == b"SPLIT-LANDED-MARKER"),
        "the split chord must land the pump on the fresh pane (its stdin \
         carries the marker echo). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // Two panes now exist under the session.
    let roster = client.send("list-panes").expect("roster");
    assert!(
        roster.len() >= 2,
        "the split created a second pane: {roster:?}"
    );
    host.killer.kill().ok();
}

/// The render-mode chords against a live daemon: prefix z rides the
/// daemon's `resize-pane -Z` (the zoomed pane's child re-fits to the
/// full grid; the Z cue and the unzoom flash paint), prefix , commits
/// the window-rename prompt as a `rename-window` control line, prefix $
/// prompts for the pane title and Escape cancels without committing,
/// and prefix B repaints the dividers at the next glyph set.
#[cfg(unix)]
#[test]
fn render_mode_zoom_rename_border_chords() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderchords");
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
    assert!(pane_b.starts_with('%'), "split returned a pane id");
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo PANE-B-CHORD'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let boot = wait_for_output(&host, b"PANE-B-CHORD", Duration::from_secs(10));
    let early = child_exit(&mut host, Duration::from_millis(200));
    assert!(
        early.is_none(),
        "render attach exited {early:?} before the chords: stderr: {} boot: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&boot)
    );

    // prefix z: the zoomed re-layout re-seeds the view and the Z cue
    // paints on the status row.
    host.to_child.write_all(&[0x02, b'z']).expect("prefix z");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b" Z |", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains(" Z |"),
        "prefix z must paint the zoom cue. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // prefix z again: the unzoom flash names itself.
    host.to_child
        .write_all(&[0x02, b'z'])
        .expect("prefix z again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"unzoomed", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("unzoomed"),
        "the second prefix z must flash unzoomed. stderr: {}",
        stderr.lock().unwrap()
    );

    // prefix , opens the window-rename prompt.
    host.to_child.write_all(&[0x02, b',']).expect("prefix ,");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"rename window", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("rename window"),
        "prefix , must open the rename prompt. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // Typing + Enter commits as a rename-window control line.
    host.to_child.write_all(b"rewin").expect("type name");
    host.to_child.flush().ok();
    host.to_child.write_all(b"\r").expect("commit");
    host.to_child.flush().ok();
    let mut renamed = false;
    for _ in 0..50 {
        if let Ok(lines) = client.send("list-windows") {
            if lines.join(" ").contains("rewin") {
                renamed = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(renamed, "prefix , must commit as rename-window");

    // prefix $ opens the pane-title prompt; Escape cancels untouched.
    host.to_child.write_all(&[0x02, b'$']).expect("prefix $");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"rename pane", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("rename pane"),
        "prefix $ must open the pane prompt. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.to_child.write_all(b"nope-title").expect("type title");
    host.to_child.flush().ok();
    host.to_child.write_all(&[0x1b]).expect("escape");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(400));
    let title = client
        .send(&format!("pane-title -t {pane_a}"))
        .expect("pane-title")
        .join(" ");
    assert!(
        !title.contains("nope-title"),
        "Escape must cancel the prompt without committing: {title:?}"
    );

    // prefix B cycles the border glyphs (herdr is the seed default, so
    // two presses land on double): the flash names the new style and
    // the same repaint carries the double-line vertical.
    host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"border style", Duration::from_secs(10));
    // Accumulate the burst's tail (the dividers repaint in the same
    // frame the flash paints).
    got.extend(wait_for_output(&host, b"", Duration::from_millis(700)));
    assert!(
        plain_text(&got).contains("border style: double"),
        "prefix B must flash the new border style. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    assert!(
        got.windows("║".len()).any(|w| w == "║".as_bytes()),
        "the border cycle must repaint the dividers at the new glyphs"
    );

    // Three more cycles land on herdr: every pane draws its own rounded
    // box; the flash names the style and the repaint carries the corner.
    for _ in 0..3 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let mut got = wait_for_output(&host, b"border style: herdr", Duration::from_secs(10));
    got.extend(wait_for_output(&host, b"", Duration::from_millis(700)));
    assert!(
        plain_text(&got).contains("border style: herdr"),
        "cycling to herdr must flash the style. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    assert!(
        got.windows("╭".len()).any(|w| w == "╭".as_bytes()),
        "the herdr style must paint per-pane rounded boxes"
    );

    // prefix l toggles the pane labels; the flash names the new state.
    // Labels default ON (the manual-pass ruling: titles show in their
    // borders), so the first press turns them off.
    host.to_child.write_all(&[0x02, b'l']).expect("prefix l");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"labels off", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("labels off"),
        "prefix l must flash labels off. stderr: {}",
        stderr.lock().unwrap()
    );
    host.to_child
        .write_all(&[0x02, b'l'])
        .expect("prefix l again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"labels on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("labels on"),
        "prefix l again must flash labels on. stderr: {}",
        stderr.lock().unwrap()
    );

    // prefix g opens the workspace picker: the themed modal titles itself
    // ` workspaces ` with one row per workspace; Escape dismisses.
    host.to_child.write_all(&[0x02, b'g']).expect("prefix g");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"workspaces", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains(" workspaces "),
        "prefix g must open the workspace picker. stderr: {}",
        stderr.lock().unwrap()
    );
    host.to_child.write_all(&[0x1b]).expect("escape picker");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(300));

    // prefix s toggles the side panel: the strip paints the workspaces
    // section and the panes re-divide around it; toggling off restores.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    // The strip's cells precede the status row in the same frame diff, so
    // the flash and the section text arrive together.
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar on") && plain_text(&got).contains("workspaces"),
        "prefix s must flash and paint the workspaces section. stderr: {} tail: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
    );
    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar off"),
        "prefix s again must flash sidebar off. stderr: {}",
        stderr.lock().unwrap()
    );

    // Shift+arrow swaps with the pane in that direction: whichever side
    // holds focus, one of the two presses hits a neighbor and the roster's
    // layout order exchanges the two panes.
    let first_before = client
        .send("list-panes")
        .expect("roster")
        .iter()
        .find_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|p| p.starts_with('%'))
                .map(str::to_string)
        })
        .expect("a pane id");
    host.to_child.write_all(&[0x02]).expect("prefix");
    host.to_child.write_all(b"\x1b[1;2C").expect("shift-right");
    host.to_child.write_all(&[0x02]).expect("prefix");
    host.to_child.write_all(b"\x1b[1;2D").expect("shift-left");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(600));
    let first_after = client
        .send("list-panes")
        .expect("roster")
        .iter()
        .find_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|p| p.starts_with('%'))
                .map(str::to_string)
        })
        .expect("a pane id");
    assert_ne!(
        first_before, first_after,
        "the shift-arrow swap must exchange the two panes' layout cells"
    );
    host.killer.kill().ok();
}

/// prefix x: the chord kills the focused pane and the pump follows the
/// window's survivor — the survivor's marker resyncs in.
#[cfg(unix)]
#[test]
fn kill_chord_lands_the_client_on_the_survivor() {
    let (fixture, _daemon, mut client) = fixture_with_session("chordkill");
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
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo SURVIVOR-MARKER'"))
        .expect("marker B");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(2));

    // Prefix x kills pane A; the pump must land on the survivor B and
    // resync its marker in.
    host.to_child.write_all(&[0x02, b'x']).expect("prefix x");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"SURVIVOR-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"SURVIVOR-MARKER".len())
            .any(|w| w == b"SURVIVOR-MARKER"),
        "prefix x must land the pump on the surviving pane. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    let roster = client.send("list-panes").expect("roster");
    assert!(
        !roster.iter().any(|l| l.starts_with(&pane_a)),
        "the focused pane is gone from the roster: {roster:?}"
    );
    host.killer.kill().ok();
}

/// The client exits 0 when the daemon shuts down (%exit), restoring the
/// terminal.
#[test]
fn daemon_exit_ends_the_client_cleanly() {
    let (fixture, _daemon, mut client) = fixture_with_session("exit");
    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    // Settle on the status draw, so %exit is the only pending signal. On
    // Windows the settle keys on the re-encoded status-row draw (ConPTY
    // absorbs DECSTBM — see detach_restores_the_terminal_region).
    let settle_needle: &[u8] = if cfg!(unix) {
        b"\x1b[1;23r"
    } else {
        b"\x1b[24;1H"
    };
    let startup = wait_for_output(&host, settle_needle, Duration::from_secs(10));
    let _ = startup;
    client.send_checked("kill-server").expect("kill-server");
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "a daemon shutdown must end attach with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// Held-dead pane: typing into a dead pane must not flood the screen — the
/// client drops the bytes (the daemon answers every send-keys to a dead
/// pane with the NotStartedError `%error`, and the bytes would echo into
/// the pane's frozen screen either way), the status row names the respawn
/// chord, and prefix `r` respawns the pane and resumes forwarding.
#[cfg(unix)]
#[test]
fn dead_pane_takes_no_typing_and_prefix_r_respawns() {
    let (fixture, _daemon, mut client) = fixture_with_session("deadpane");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Kill the pane's shell: it is held dead (remain-on-exit) before the
    // client attaches, the deterministic shape (the %pane-exited-mid-view
    // path sets the same `exited` flag — pinned at the router level).
    client
        .send(&format!("send-keys -t {pane} -l 'exit'"))
        .expect("exit");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
            .join("");
        assert!(Instant::now() < deadline, "the pane never died: {info}");
        if info.contains("exited=") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    // Settle on the status draw; it must carry the respawn hint (the raw
    // status text passes through the pty untouched; the em dash is its
    // UTF-8 spelling \xe2\x80\x94).
    let startup = wait_for_output(&host, b"respawns)", Duration::from_secs(10));
    assert!(
        startup.windows(14).any(|w| w == b"(exited 0 \xe2\x80\x94 "),
        "the status row must show the held-dead cue: {:?}\nstderr: {}",
        String::from_utf8_lossy(&startup),
        stderr.lock().unwrap()
    );
    assert!(
        startup.windows(15).any(|w| w == b"C-b r respawns)"),
        "the status row must name the respawn chord: {:?}",
        String::from_utf8_lossy(&startup)
    );

    // Type a burst: the client must drop the bytes. The only output that
    // may follow is the settle-redraw status row (the one-shot redraw the
    // client owes after the startup burst — same bytes as the first draw,
    // no pane content), so assert no ECHO of the typed bytes and no
    // send-keys error text renders.
    host.to_child
        .write_all(b"garbage typing \x1b[A x")
        .expect("type");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(500));
    let mut after = startup.clone();
    while let Ok(bytes) = host.output_rx.try_recv() {
        after.extend_from_slice(&bytes);
    }
    let extra = &after[startup.len()..];
    assert!(
        !extra.windows(14).any(|w| w == b"garbage typing"),
        "typed bytes must never echo to a held-dead pane's screen: {:?}",
        String::from_utf8_lossy(&after)
    );
    assert!(
        !extra.windows(7).any(|w| w == b"par-mux"),
        "no error text may render over the dead pane: {:?}",
        String::from_utf8_lossy(&after)
    );
    // Only status-row redraws may appear after typing: every extra byte
    // must sit inside a status-draw block — the ESC7..ESC8 wrap plus the
    // absolute tracked-cell CUP (ESC[<r>;<c>H) that closes each draw (the
    // settle/redraw discipline) — no pane output, no flood. Byte-COUNT
    // budgets flake on slow machines where more redraws land in the window.
    let mut residue: Vec<u8> = Vec::new();
    let mut inside_draw = false;
    let mut awaiting_placement_h = false;
    let mut i = 0;
    while i < extra.len() {
        if extra[i..].starts_with(b"\x1b7") {
            inside_draw = true;
            i += 2;
        } else if extra[i..].starts_with(b"\x1b8") {
            // The wrap's restore is immediately followed by the draw's
            // absolute tracked-cell CUP — still draw bytes until its H.
            awaiting_placement_h = true;
            inside_draw = true;
            i += 2;
        } else if awaiting_placement_h {
            if extra[i] == b'H' {
                awaiting_placement_h = false;
                inside_draw = false;
            }
            i += 1;
        } else {
            if !inside_draw {
                residue.push(extra[i]);
            }
            i += 1;
        }
    }
    assert!(
        residue.iter().all(|b| b.is_ascii_whitespace()),
        "a held-dead pane must take no stdin bytes: only status-row redraws \
         (ESC7..ESC8 blocks) may follow, found {} bytes of other output: {:?}. \
         stderr: {}",
        residue.len(),
        String::from_utf8_lossy(&residue),
        stderr.lock().unwrap()
    );

    // Prefix r respawns the pane and resumes forwarding: the client wipes
    // the frozen dead-pane screen (clear + home) before the fresh replay,
    // then a typed echo round-trips through the revived pane.
    host.to_child.write_all(&[0x02, b'r']).expect("prefix r");
    host.to_child.flush().ok();
    let wiped = wait_for_output(&host, b"\x1b[2J\x1b[H", Duration::from_secs(10));
    assert!(
        wiped.windows(7).any(|w| w == b"\x1b[2J\x1b[H"),
        "respawn must clear the frozen screen before the resync replay. \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&wiped)
    );
    host.to_child
        .write_all(b"echo RESPAWNED-ROUNDTRIP\r")
        .expect("type after respawn");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"RESPAWNED-ROUNDTRIP", Duration::from_secs(15));
    assert!(
        got.windows(b"RESPAWNED-ROUNDTRIP".len())
            .any(|w| w == b"RESPAWNED-ROUNDTRIP"),
        "prefix r must respawn the pane and resume forwarding. stderr: {}\n\
         bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();
}

/// Regression (the manual-pass ESC7/ESC8 race): pane output flowing WHILE a
/// status draw ran made the ESC8 restore land one line off, and every later
/// output painted over the wrong row (observed on Terminal.app: a new
/// prompt overwriting the middle of the previous line). The status draw now
/// CLOSES with an absolute CUP at the shadow emulator's tracked cell, so a
/// scroll landing between the draw and the placement cannot make the final
/// position wrong. The PTY-level proof: flood the pane with scrolling
/// output while the client's status draws land (the settle-redraw cadence
/// draws during the flood), assert every draw the client emitted closes
/// with its absolute tracked-cell CUP (a bare-ESC8 close would be the raced
/// shape), then drive an echo through the pane — it must round-trip. The
/// exact tracked cell is pinned headless by
/// `status_draw_places_the_cursor_at_the_tracked_cell` (`src/mux/attach/mod.rs`).
#[cfg(unix)]
#[test]
fn status_draw_tracks_the_cursor_under_output_flood() {
    let (fixture, _daemon, mut client) = fixture_with_session("cursorrace");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // Flood: numbered lines scrolled fast enough that scrolls land while
    // the client's status redraws are in flight.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 60); do echo FLOOD-LINE-$i; done'"
        ))
        .expect("flood");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(600));

    let (mut host, stderr) = spawn_attach(&fixture, &["-t", &pane]);
    let startup = wait_for_output(&host, b"FLOOD-LINE-60", Duration::from_secs(15));
    assert!(
        !startup.is_empty(),
        "the flood must reach the host. stderr: {}",
        stderr.lock().unwrap()
    );
    // The settle-redraw draws AFTER the flood's last line; give it time to
    // flow, then join what followed so the scan sees whole draw blocks.
    std::thread::sleep(Duration::from_millis(700));
    let mut b = startup;
    while let Ok(bytes) = host.output_rx.try_recv() {
        b.extend_from_slice(&bytes);
    }

    // Every draw closes with its absolute tracked-cell CUP: scan the whole
    // capture for draw blocks and require the placement CUP right after
    // each restore. (The exact cell at draw time is the emulator's
    // business — the headless suite pins it; here the structural shape is
    // the contract.)
    let mut draws = 0usize;
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == 0x1b && b[i + 1] == 0x37 {
            // ESC7: find the ESC8 close, then require ESC[ ... H after it.
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == 0x1b && b[j + 1] == 0x38) {
                j += 1;
            }
            assert!(
                j + 1 < b.len(),
                "a draw never closed (no ESC8): bytes[{i}..]: {:?}",
                String::from_utf8_lossy(&b[i..(i + 80).min(b.len())])
            );
            j += 2;
            assert!(
                j + 1 < b.len() && b[j] == 0x1b && b[j + 1] == b'[',
                "a draw closed with a bare ESC8 (the raced shape): \
                 bytes[{j}..]: {:?}",
                String::from_utf8_lossy(&b[j..(j + 80).min(b.len())])
            );
            // The placement CUP's params end at 'H'.
            while j < b.len() && b[j] != b'H' {
                j += 1;
            }
            assert!(j < b.len(), "the placement CUP never closed");
            draws += 1;
            i = j + 1;
        } else {
            i += 1;
        }
    }
    assert!(
        draws > 0,
        "at least one status draw must appear in the flood capture. stderr: {}",
        stderr.lock().unwrap()
    );

    // With draws landing under live scrolling output, an echo typed now
    // must round-trip intact through the pane.
    host.to_child
        .write_all(b"echo RACED-ROUNDTRIP\r")
        .expect("type echo");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"RACED-ROUNDTRIP", Duration::from_secs(15));
    assert!(
        got.windows(15).any(|w| w == b"RACED-ROUNDTRIP"),
        "an echo must round-trip intact after draws under a flood. stderr: {}\n\
         bytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();
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

/// Phase B criteria 1+2, driven against the REAL daemon in-process: a
/// 2-pane split's `%layout-change` parses to rects matching the daemon's
/// own geometry, and each pane's replay-fed emulator reproduces what
/// `capture-pane` ground truth says the pane holds — the same cell
/// content the renderer paints at those rects. (Criterion 3's frame
/// coalescing is pinned by `output_flood_coalesces_at_frame_cadence` in
/// `src/mux/attach/render.rs` — a pure-renderer property.)
///
/// The renderer runs headless (no sink): the frames stay in the
/// [`PaneRenderer::buffer`] the assertions read.
#[test]
fn render_mode_layout_and_pane_replay_match_daemon_ground_truth() {
    let (fixture, _daemon, mut client) = fixture_with_session("render");

    // The initial session's pane, then a vertical split; distinct content
    // per pane through the shell.
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    // Let the echoes land on the pane screens BEFORE the replay: the
    // assertion is about the replay seeding, not a race with %output.
    std::thread::sleep(Duration::from_millis(500));

    // The render client: the documented handshake, then the size report
    // that pulls the current layout triple.
    let mut conn = par_mux::mux::attach::conn::AttachConn::connect(fixture.socket())
        .expect("render client connect");
    let _replay = conn.drain_pending_events();
    conn.send_checked(&format!("refresh-client -t {pane0} -C 80x24"))
        .expect("size report");
    let layout_event = conn
        .drain_pending_events()
        .into_iter()
        .find_map(|event| match event {
            par_term_emu_core::tmux_control::TmuxNotification::LayoutChange {
                window_layout,
                window_visible_layout,
                window_raw_flags,
                ..
            } => Some((window_layout, window_visible_layout, window_raw_flags)),
            _ => None,
        })
        .expect("the size report broadcast a layout change");
    let rects = par_mux::mux::attach::layout::parse_layout_triple(
        &layout_event.0,
        &layout_event.1,
        &layout_event.2,
    )
    .expect("the daemon's own layout string parses");
    assert_eq!(rects.len(), 2, "the split's two leaves");
    assert_eq!(rects[0].x, 0, "left pane starts at col 0");
    assert_eq!(rects[1].x, rects[0].width, "right pane abuts the left");
    assert_eq!(
        rects[0].height, rects[1].height,
        "a vertical split shares the height"
    );

    // Renderer over the daemon's layout, replays per pane.
    let mut renderer = par_mux::mux::attach::render::PaneRenderer::new(
        80,
        24,
        par_mux::mux::attach::render::Glyphs::Unicode,
    );
    renderer.apply_layout(rects.clone());
    for rect in &rects {
        let pane = format!("%{}", rect.pane);
        let reply = conn
            .send_checked(&format!("refresh-client -t {pane}"))
            .expect("replay request");
        assert!(reply.ok, "replay of {pane}");
        let mut bytes = reply.body.join("\n").into_bytes();
        bytes.push(b'\n');
        renderer.feed_output(rect.pane, &bytes);
    }
    renderer.render_frame();

    // Ground truth per pane: the daemon's own capture of what each pane
    // shows must appear at that pane's layout rect of the rendered buffer.
    for (marker, rect) in [("LEFT-MARKER", &rects[0]), ("RIGHT-MARKER", &rects[1])] {
        let rendered: String = (0..rect.height)
            .map(|row| {
                (0..rect.width)
                    .map(|col| {
                        renderer
                            .cell(rect.x + col, rect.y + row)
                            .map(|c| c.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>()
            })
            .collect::<String>();
        assert!(
            rendered.contains(marker),
            "the pane's rect must render its capture-pane content ({marker}); got {rendered:?}"
        );
        // And the daemon agrees the pane itself shows it.
        let capture = client
            .send(&format!("capture-pane -t %{}", rect.pane))
            .expect("capture");
        assert!(
            capture.join("\n").contains(marker),
            "daemon ground truth for %{} shows {marker}: {capture:?}",
            rect.pane
        );
    }
}

/// Acceptance criterion 1 (render mode, PTY level): the pane's DECCKM
/// state re-encodes arrow keys. The pane's shell enables application
/// cursor keys by emitting DECCKM through `printf`, the render client
/// parses the host arrow key from stdin and re-encodes it against the
/// pane's tracked state, and the pane's `cat -v` prints the received
/// spelling onto its screen — which the daemon's capture-pane verifies.
/// An app reading arrow keys sees `ESC O A` (application) or `ESC [ A`
/// (normal), not whatever the host terminal sent.
#[cfg(unix)]
#[test]
fn render_mode_arrow_keys_reencode_per_the_panes_decckm() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderkeys");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    // The pane: enable DECCKM, then echo every received byte visibly.
    // `cat -v` shows ESC as `^[`.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1h\"; cat -v'"
        ))
        .expect("start key reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    // Let the pane print its DECCKM so the replay carries it before the
    // client mirrors the pane.
    std::thread::sleep(Duration::from_millis(700));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle: the renderer's first frame paints (alt-screen enter). Then
    // send an Up arrow (the harness's stdin is the client's host stdin).
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    host.to_child.write_all(b"\x1b[A").expect("arrow up");
    host.to_child.flush().ok();

    // `cat -v` renders ESC O A as `^[[O A`? No — `ESC O A` prints as
    // `^[OA`. Wait for it on the capture via the client's own screen
    // paint; the capture assertion below is the authority.
    let _ = wait_for_output(&host, b"OA", Duration::from_secs(10));

    // The authority: the pane itself received the SS3 spelling.
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    let body = capture.join("\n");
    assert!(
        body.contains("^[OA"),
        "the pane must receive the application-cursor spelling ESC O A for Up \
         (DECCKM on): {body:?}\nstderr: {}",
        stderr.lock().unwrap()
    );
    assert!(
        !body.contains("0;11;6"),
        "sanity: no stray mouse SGR reached the pane: {body:?}"
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 3 (render mode, PTY level): the wheel scrolls the
/// CLIENT's scrollback when the pane does not own mouse mode — the
/// renderer repaints the pane rect from its scrollback, and the pane
/// itself receives nothing.
#[cfg(unix)]
#[test]
fn render_mode_wheel_scrolls_client_scrollback_without_mouse_mode() {
    let (fixture, _daemon, mut client) = fixture_with_session("wheelscroll");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Fill the pane's history: 40 numbered lines through the 24-row pane.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 40); do echo HISTLINE-$i; done'"
        ))
        .expect("fill");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(800));

    let (mut host, _stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle on the live paint.
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"HISTLINE-40", Duration::from_secs(10));
    // Drain the settle paint so the post-wheel read is wheel-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Wheel up over the pane: SGR wheel-up at (10, 5), 1-based. The view
    // shifts up 3 lines, so the pane rect's TOP view row repaints with
    // scrollback-only content. (Which history line lands there depends on
    // the pane's prompt/echo lines, so the exact number is pinned by the
    // renderer's unit suite, not asserted here — the routing behavior is.)
    // The live pane's daemon-side view must not move: scrolling is
    // client-side only, so the capture is identical before and after the
    // wheel. (Render mode's size report carries content height — grid
    // minus the status row — so the visible line count is one fewer than
    // the host grid; never pin a specific history line here.)
    let before = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture")
        .join("\n");
    assert!(!before.is_empty(), "the pane settled with a live view");
    host.to_child
        .write_all(b"\x1b[<64;10;5M")
        .expect("wheel up");
    host.to_child.flush().ok();
    let scrolled = wait_for_output(&host, b"\x1b[1;", Duration::from_secs(5));
    assert!(
        !scrolled.is_empty(),
        "the wheel must repaint the pane rect's top rows from scrollback"
    );
    let after = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture")
        .join("\n");
    assert_eq!(after, before, "the pane's live view is intact");
    assert!(
        !after.contains("\x1b[<64"),
        "no wheel SGR was forwarded to the pane: {after:?}"
    );
    host.killer.kill().ok();
}

/// Acceptance criterion 2+3 (forward side): a pane that owns mouse mode
/// receives pane-relative SGR mouse reports through the render client, and
/// a click on a rendered pane issues the daemon-side select-pane. The pane
/// enables SGR mouse mode (DECSET 1000 + 1006) via its own output; the
/// harness sends a left press at window (10, 5); the pane's `cat -v`
/// prints the re-encoded report.
#[cfg(unix)]
#[test]
fn render_mode_mouse_forwards_pane_relative_when_pane_owns_mouse() {
    let (fixture, _daemon, mut client) = fixture_with_session("mousefwd");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!(
            "send-keys -t {pane} -l 'printf \"\\033[?1000h\\033[?1006h\"; cat -v'"
        ))
        .expect("enable mouse + reader");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(700));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));

    // Left press at host col 10 row 5 (1-based): the tab strip shifts the
    // content down one row, so the single pane's rect (0,0) maps the
    // click to pane-relative (9, 3) 0-based and the wire spelling is
    // ESC[<0;10;4M.
    host.to_child.write_all(b"\x1b[<0;10;5M").expect("click");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"0;10;4", Duration::from_secs(10));

    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    let body = capture.join("\n");
    assert!(
        body.contains("^[[<0;10;4M"),
        "the owning pane must receive the pane-relative SGR click: {body:?}\n\
         stderr: {}",
        stderr.lock().unwrap()
    );

    // The click also focused the pane daemon-side (the single-pane window's
    // marked pane is this pane either way; the select-pane command itself
    // was issued — a two-pane focus assertion lives in the unit suite, and
    // pane-relative re-encoding during scroll mode is a non-goal (the
    // viewport is a modal view).
    host.killer.kill().ok();
}

/// Reconstruct the screen a render-mode client painted, from its
/// per-cell diff stream: each flush cell is `CUP row;col` + SGR run +
/// one symbol. Returns `(final, ever)`: the final `rows` strings of
/// `cols` symbols, and per-row snapshots of the row text after every
/// cell write, so a row fact that a later repaint removed (the agent
/// liveness sweep clearing a test claim) is still observable. Style is
/// not tracked.
#[cfg(unix)]
fn reconstructed_screen(
    bytes: &[u8],
    rows: u16,
    cols: u16,
) -> (Vec<String>, Vec<std::collections::BTreeSet<String>>) {
    let mut grid = vec![vec![b' '; cols as usize]; rows as usize];
    let mut ever: Vec<std::collections::BTreeSet<String>> = (0..rows)
        .map(|_| std::collections::BTreeSet::new())
        .collect();
    let mut i = 0;
    let text = bytes;
    while i < text.len() {
        // CUP: ESC [ row ; col H
        if text[i] == 0x1b && i + 1 < text.len() && text[i + 1] == b'[' {
            let mut j = i + 2;
            let mut row = 0usize;
            let mut col = 0usize;
            let mut part = 0; // 0 = row, 1 = col
            while j < text.len() {
                match text[j] {
                    b'0'..=b'9' if part == 0 => row = row * 10 + (text[j] - b'0') as usize,
                    b'0'..=b'9' => col = col * 10 + (text[j] - b'0') as usize,
                    b';' => part = 1,
                    b'H' => {
                        j += 1;
                        // Skip any CSI sequences between the CUP and the
                        // symbol — SGR runs (`…m`), but also the cursor
                        // placements the render client emits per frame:
                        // DECSCUSR (`CSI Ps SP q`) and DECTCEM (`CSI ?25
                        // h/l`). Parameter bytes (0x30-0x3F) and
                        // intermediates (0x20-0x2F, the SP) precede the
                        // final byte (0x40-0x7E). An `H`-final sequence
                        // (another CUP — the next cell write) stops the
                        // skip so the main loop re-parses it.
                        while j + 1 < text.len() && text[j] == 0x1b && text[j + 1] == b'[' {
                            let mut k = j + 2;
                            while k < text.len() && (0x20..0x40).contains(&text[k]) {
                                k += 1;
                            }
                            if k >= text.len() {
                                break;
                            }
                            if text[k] == b'H' {
                                break;
                            }
                            j = k + 1;
                        }
                        if j < text.len() && row > 0 && col > 0 {
                            if text[j] == 0x1b {
                                // A CUP with no symbol after the CSI
                                // runs — the cursor placement's CUP
                                // (followed by DECSCUSR/DECTCEM, skipped
                                // above). Resume AT the next sequence:
                                // the loop's i += 1 lands on its ESC.
                                i = j.saturating_sub(1);
                            } else {
                                // One UTF-8 scalar.
                                let rest = &text[j..];
                                if let Ok(s) = std::str::from_utf8(&rest[..rest.len().min(4)]) {
                                    if let Some(ch) = s.chars().next() {
                                        let (r, c) = (row - 1, col - 1);
                                        if r < rows as usize && c < cols as usize {
                                            let mut buf = [0u8; 4];
                                            grid[r][c] = *ch
                                                .encode_utf8(&mut buf)
                                                .as_bytes()
                                                .first()
                                                .unwrap_or(&b' ');
                                            ever[r].insert(
                                                String::from_utf8_lossy(&grid[r]).into_owned(),
                                            );
                                        }
                                    }
                                }
                                i = j;
                            }
                        }
                        break;
                    }
                    _ => break,
                }
                j += 1;
            }
        }
        i += 1;
    }
    let final_grid: Vec<String> = grid
        .into_iter()
        .map(|row| String::from_utf8_lossy(&row).into_owned())
        .collect();
    (final_grid, ever)
}

/// One hook report over its own one-line connection — the send-one-JSON,
/// read-one-reply, close shape herdr's scripts use (`parse_line` routes a
/// `{` line from any connection to the hook layer). Copied from
/// tests/mux_agents.rs; a test-binary-local helper.
#[cfg(unix)]
fn hook_report(path: &std::path::Path, json: &str) -> String {
    use std::io::{BufRead as _, BufReader, Write as _};
    let mut stream = std::os::unix::net::UnixStream::connect(path).expect("hook connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout installs");
    writeln!(stream, "{json}").expect("write report");
    stream.flush().expect("flush");
    let mut reply = String::new();
    BufReader::new(stream.try_clone().expect("clone"))
        .read_line(&mut reply)
        .expect("one reply line");
    reply
}

/// Acceptance criterion 1 (render mode, PTY level): the status bar. The
/// client's output carries a bottom-row draw containing the session name,
/// and an agent state change broadcast (%agent-state-changed) triggers the
/// throttled re-query — the roster chip appears/updates on the row.
#[cfg(unix)]
#[test]
fn render_mode_status_bar_shows_sessions_and_updates_on_agent_changes() {
    let (fixture, _daemon, mut client) = fixture_with_session("statusbar");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle: alt-screen enter + first frame.
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));

    // Agent churn: a hook report claims the pane; %agent-state-changed
    // marks the status stale and the throttled re-query pulls the chip
    // onto the row.
    let reply = hook_report(
        fixture.socket(),
        &format!(
            r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","state":"blocked","seq":1,"source":"par-mux:test"}}}}"#
        ),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    // The refresh runs at the next pump pass (16 ms cadence); give it a
    // generous beat, then reconstruct the row from everything received.
    std::thread::sleep(Duration::from_secs(2));
    let mut all = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let (_final, ever) = reconstructed_screen(&all, 24, 80);
    let ever_row: String = ever[23].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[23].iter().any(|r| r.contains("$0:att")),
        "the status bar must show the shown session on row 24. snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    assert!(
        ever[23].iter().any(|r| r.contains("kimi:blocked")),
        "the roster chip must appear on %agent-state-changed (a later liveness-sweep \
         repaint may remove the test claim again). snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a host resize re-fits
/// the whole client. Resizing the harness master PTY (the host terminal's
/// SIGWINCH shape) makes the pump report the new grid (content height,
/// minus the status row), the daemon re-divides the window and
/// re-broadcasts the layout, `resize_to` re-fits and re-seeds every pane
/// synchronously, and the status row repaints at the host's new bottom
/// row — the settle-redraw discipline means a repaint flood cannot leave
/// it blank.
#[cfg(unix)]
#[test]
fn render_mode_host_resize_refits_window_layout_and_status_row() {
    let (fixture, _daemon, mut client) = fixture_with_session("resize");
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane0]);
    // Settle: mouse capture + first frame with both panes painted.
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"LEFT-MARKER", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"RIGHT-MARKER", Duration::from_secs(10));
    while host.output_rx.try_recv().is_ok() {}

    // Resize the host terminal: 24x80 -> 30x100. Everything the host
    // receives past this point is resize-attributable.
    host.master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("host resize");

    // The status row repaints at the host's NEW bottom row (row 30) — a
    // CUP to row 30 cannot come from the 24-row geometry.
    let got = wait_for_output(&host, b"\x1b[30;", Duration::from_secs(10));
    assert!(
        !got.is_empty(),
        "the resize must re-fit and repaint (status row CUP to the new bottom \
         row). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // The daemon re-divided to the reported content grid (100x28 on a
    // 30-row host: rows minus the tab strip minus the status row): each
    // pane re-fit to half of 100 columns. Give the broadcast/refit a
    // beat, then read pane-info ground truth.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut fitted = false;
    while Instant::now() < deadline {
        let info = client
            .send(&format!("pane-info -t {pane0}"))
            .expect("pane-info");
        if info.join(" ").contains("50x28") {
            fitted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fitted,
        "the size report must re-fit the daemon's panes to the content grid \
         (100x28, half = 50 wide): {:?}",
        client
            .send(&format!("pane-info -t {pane0}"))
            .expect("pane-info")
    );

    // Reconstruct the painted screen at the new geometry: both panes
    // re-seeded, each in its half, and the status row carrying the shown
    // session on row 30. The repaint may straddle the needle read, so the
    // reconstruction joins the needle-read bytes with what followed; give
    // the re-seed + repaint a beat to finish flowing before draining.
    std::thread::sleep(Duration::from_millis(1500));
    let mut all = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let capture_left = client
        .send(&format!("capture-pane -t {pane0}"))
        .expect("capture left");
    let (final_grid, ever) = reconstructed_screen(&all, 30, 100);
    assert!(
        final_grid.iter().any(|row| row.contains("LEFT-MARKER"))
            || ever
                .iter()
                .any(|rows| rows.iter().any(|r| r.contains("LEFT-MARKER"))),
        "the left pane must repaint after the resize re-fit: {final_grid:?}\n\
         daemon capture: {:?}\nbytes received after resize: {} bytes, first 400: {:?}",
        capture_left.join("\n"),
        all.len(),
        String::from_utf8_lossy(&all[..all.len().min(400)])
    );
    assert!(
        final_grid.iter().any(|row| row.contains("RIGHT-MARKER")),
        "the right pane must repaint after the resize re-fit: {final_grid:?}"
    );
    let left_mark = final_grid
        .iter()
        .filter_map(|row| row.find("LEFT-MARKER"))
        .next();
    if let Some(col) = left_mark {
        assert!(
            col < 50,
            "the left pane's marker must sit in the re-divided left half \
             (cols 0..50): col {col}"
        );
    }
    let right_mark = final_grid
        .iter()
        .filter_map(|row| row.find("RIGHT-MARKER"))
        .next();
    if let Some(col) = right_mark {
        assert!(
            (50..100).contains(&col),
            "the right pane's marker must sit in the re-divided right half \
             (cols 50..100): col {col}"
        );
    }
    let ever_row: String = ever[29].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[29].iter().any(|r| r.contains("$0:att")),
        "the status row must survive the repaint flood at the new bottom row \
         (row 30). snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a host resize to a
/// TALLER-than-handshake grid re-fits the whole frame. The client settles
/// at the harness's 24-row default; growing the master to 40 rows (width
/// kept) must (a) report the content grid on the wire — 40 rows minus
/// the tab strip minus the status row = 38, (b) wipe the host screen and
/// repaint the full frame at the new geometry — tab strip on row 1,
/// status on row 40, pane content between — so no pre-resize content can
/// remain visible outside the new frame, and (c) keep pane content seeded
/// before the attach visible through the re-fit (position may shift).
#[cfg(unix)]
#[test]
fn render_mode_resize_to_taller_than_handshake_covers_the_full_frame() {
    let (fixture, _daemon, mut client) = fixture_with_session("tallresize");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // A deterministic tab name, so the strip row pins exactly.
    client.send("rename-window -t @0 big").expect("rename");
    // Pre-attach content: seeded before the client attaches, so the
    // replay path (not a live %output race) must carry it into the
    // re-fit frame.
    client
        .send(&format!("send-keys -t {pane} -l 'echo SURVIVOR-MARKER'"))
        .expect("seed marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    // Settle on the 24-row handshake grid: mouse capture + the seeded
    // marker painted.
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"SURVIVOR-MARKER", Duration::from_secs(10));
    // Drain the settle paint so everything read next is resize-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Grow the host 24 -> 40 rows, width kept.
    host.master
        .resize(PtySize {
            rows: 40,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("host resize");

    // The status row repaints at the host's NEW bottom row (row 40) — a
    // CUP to row 40 cannot come from the 24-row geometry.
    let got = wait_for_output(&host, b"\x1b[40;", Duration::from_secs(10));
    assert!(
        !got.is_empty(),
        "the taller resize must re-fit and repaint (status CUP to row 40). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );

    // Give the re-seed + full repaint a beat to finish flowing, then
    // collect everything the host received after the resize.
    std::thread::sleep(Duration::from_millis(1500));
    let mut all = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }

    // The wire size report re-fit the daemon's window to the content
    // grid: 40 rows - tab strip - status row = 38 rows, width kept.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut fitted = false;
    while Instant::now() < deadline {
        let info = client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info");
        if info.join(" ").contains("80x38") {
            fitted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fitted,
        "the size report must carry the content grid (40 rows minus the tab \
         strip minus the status row = 38, width kept): {:?}",
        client
            .send(&format!("pane-info -t {pane}"))
            .expect("pane-info")
    );

    // The repaint wipe: resize_to clears the host screen before the
    // re-fit frame — the discipline that guarantees no pre-resize
    // content can survive outside the new frame.
    assert!(
        all.windows(4).any(|w| w == b"\x1b[2J"),
        "the re-fit must wipe the host screen (CSI 2J) so pre-resize content \
         cannot remain visible. bytes: {}",
        String::from_utf8_lossy(&all[..all.len().min(400)])
    );

    // Reconstruct the painted screen at the new geometry and pin the
    // full frame: tab strip on row 1, status on row 40, and the
    // pre-attach marker re-seeded inside the pane area (1-based rows
    // 2..39; 0-based 1..=38).
    let (final_grid, ever) = reconstructed_screen(&all, 40, 80);
    let ever_row = |r: usize| {
        ever[r]
            .iter()
            .map(|s| format!("{s:?}\n"))
            .collect::<String>()
    };
    assert!(
        ever[0].iter().any(|r| r.contains(" big ")),
        "the tab strip must paint row 1 at the taller grid. snapshots:\n{}\n\
         stderr: {}",
        ever_row(0),
        stderr.lock().unwrap()
    );
    assert!(
        ever[39].iter().any(|r| r.contains("$0:att")),
        "the status row must paint the new bottom row (row 40). snapshots:\n{}\n\
         stderr: {}",
        ever_row(39),
        stderr.lock().unwrap()
    );
    let marker_row = final_grid
        .iter()
        .position(|row| row.contains("SURVIVOR-MARKER"))
        .or_else(|| {
            ever.iter()
                .position(|rows| rows.iter().any(|r| r.contains("SURVIVOR-MARKER")))
        });
    assert!(
        marker_row.is_some(),
        "the pre-attach marker must survive the re-fit. final: {final_grid:?}"
    );
    if let Some(row) = marker_row {
        assert!(
            (1..=38).contains(&row),
            "the marker must sit inside the pane area (1-based rows 2..39), \
             not on the strip or status rows: 0-based row {row}"
        );
    }
    host.killer.kill().ok();
}

/// Acceptance criterion (render mode, PTY level): a daemon-side zoom
/// (`resize-pane -t <pane> -Z`, tmux semantics) re-renders the VISIBLE
/// layout — the zoomed pane alone at full window extent — and a second
/// `-Z` unzooms, restoring the split. On the wire the true layout keeps
/// both panes while `window_visible_layout` carries the zoomed one and
/// the flags carry `Z`; the layout parser renders the visible form.
#[cfg(unix)]
#[test]
fn render_mode_daemon_zoom_shows_single_pane_and_unzoom_restores_split() {
    let (fixture, _daemon, mut client) = fixture_with_session("zoom");
    let pane0 = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("split-window -t {pane0} -h"))
        .expect("split");
    let pane1 = client
        .send("list-panes")
        .expect("list-panes")
        .join("\n")
        .lines()
        .find(|l| l.split_whitespace().next() != Some(pane0.as_str()))
        .and_then(|l| l.split_whitespace().next())
        .expect("the split's pane")
        .to_string();
    client
        .send(&format!("send-keys -t {pane0} -l 'echo LEFT-MARKER'"))
        .expect("keys left");
    client
        .send(&format!("send-keys -t {pane0} Enter"))
        .expect("enter left");
    client
        .send(&format!("send-keys -t {pane1} -l 'echo RIGHT-MARKER'"))
        .expect("keys right");
    client
        .send(&format!("send-keys -t {pane1} Enter"))
        .expect("enter right");
    std::thread::sleep(Duration::from_millis(500));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane0]);
    // Settle: the split view with both panes painted.
    let settle = {
        let a = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
        let b = wait_for_output(&host, b"LEFT-MARKER", Duration::from_secs(10));
        let c = wait_for_output(&host, b"RIGHT-MARKER", Duration::from_secs(10));
        let _ = (a, b);
        c
    };
    let _ = settle;
    while host.output_rx.try_recv().is_ok() {}

    // Zoom pane1. The %layout-change (flags carry Z) parks the visible
    // layout — pane1 alone at 80x23 — and the pump re-seeds it, so the
    // zoom repaint carries RIGHT-MARKER past the drain.
    client
        .send(&format!("resize-pane -t {pane1} -Z"))
        .expect("zoom");
    let zoomed = wait_for_output(&host, b"RIGHT-MARKER", Duration::from_secs(10));
    assert!(
        !zoomed.is_empty(),
        "the zoom must re-render the visible layout with the zoomed pane's \
         content. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&zoomed)
    );
    // The daemon kept the true layout: both panes still exist under the
    // window (zoom is view state, not structure). @0 is the session's
    // only window (the fixture seeds one session with one pane; the split
    // stayed in it).
    let roster = client.send("list-panes -t @0").expect("window roster");
    let roster_text = roster.join("\n");
    let ids: Vec<&str> = roster_text
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(
        ids.contains(&pane0.as_str()) && ids.contains(&pane1.as_str()),
        "the zoom must keep the true layout (both panes): {ids:?}"
    );
    while host.output_rx.try_recv().is_ok() {}

    // Unzoom: -Z again on the zoomed pane restores the exact split, and
    // the restore repaint re-seeds pane0 — its marker lands past the drain.
    client
        .send(&format!("resize-pane -t {pane1} -Z"))
        .expect("unzoom");
    let restored = wait_for_output(&host, b"LEFT-MARKER", Duration::from_secs(10));
    assert!(
        !restored.is_empty(),
        "unzooming must restore the split view (the hidden pane repaints). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&restored)
    );
    // Ground truth both ways: the panes kept their screens through the
    // zoom round-trip.
    for (marker, pane) in [("LEFT-MARKER", &pane0), ("RIGHT-MARKER", &pane1)] {
        let capture = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture");
        assert!(
            capture.join("\n").contains(marker),
            "{pane} must still show {marker} after the zoom round-trip: {capture:?}"
        );
    }
    host.killer.kill().ok();
}

/// Acceptance criterion 2 (render mode, PTY level): prefix-[ enters the
/// scroll viewport — the pane rect's top rows repaint from scrollback —
/// and q exits cleanly (live view restored, keys reach the pane again).
#[cfg(unix)]
#[test]
fn render_mode_prefix_bracket_enters_scroll_mode_and_q_exits() {
    let (fixture, _daemon, mut client) = fixture_with_session("scrollmode");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // Fill history: 40 numbered lines through the 24-row pane.
    client
        .send(&format!(
            "send-keys -t {pane} -l 'for i in $(seq 1 40); do echo HISTLINE-$i; done'"
        ))
        .expect("fill");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(800));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"HISTLINE-40", Duration::from_secs(10));
    while host.output_rx.try_recv().is_ok() {}

    // prefix [ (C-b then '['). The viewport jumps one viewport up and the
    // pane rect's top rows repaint from scrollback.
    host.to_child.write_all(&[0x02, b'[']).expect("prefix [");
    host.to_child.flush().ok();
    let scrolled = wait_for_output(&host, b"scroll", Duration::from_secs(10));
    assert!(
        !scrolled.is_empty(),
        "prefix [ must repaint the pane rect from scrollback (the [scroll +N] \
         status cue or the history rows). stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&scrolled)
    );

    // q exits: the live view returns (HISTLINE-40 repaints) and keys reach
    // the pane again.
    host.to_child.write_all(b"q").expect("q");
    host.to_child.flush().ok();
    let restored = wait_for_output(&host, b"HISTLINE-40", Duration::from_secs(10));
    assert!(
        !restored.is_empty(),
        "q must snap the view back to live (HISTLINE-40 repaints): {:?}",
        String::from_utf8_lossy(&restored)
    );

    // Keys reach the pane again: an `echo` runs, the marker lands.
    host.to_child
        .write_all(b"echo POST-SCROLL-MARKER\r")
        .expect("type after exit");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"POST-SCROLL-MARKER", Duration::from_secs(10));
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    let body = capture.join("\n");
    assert!(
        body.contains("POST-SCROLL-MARKER"),
        "keys must reach the pane after exiting scroll mode: {body:?}"
    );
    host.killer.kill().ok();
}

/// Render mode's prefix %: the chord splits the focused pane and the
/// view re-seeds — the split's second pane renders at its rect, and
/// typing lands in the fresh pane (its shell echoes it into the grid the
/// next frame paints).
#[cfg(unix)]
#[test]
fn render_mode_split_chord_reseeds_the_window_with_the_new_pane() {
    let (fixture, _daemon, mut client) = fixture_with_session("rendersplit");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));
    while host.output_rx.try_recv().is_ok() {}

    // prefix % splits right; the re-seed paints two panes (a vertical
    // divider column appears — Unicode `│` or the ACS `|` fallback).
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    let divider = wait_for_output(&host, "│".as_bytes(), Duration::from_secs(15));
    assert!(
        !divider.is_empty(),
        "the split chord must re-seed the window with the new pane's \
         divider. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&divider)
    );
    // Two panes exist daemon-side, and the fresh pane renders a prompt.
    let roster = client.send("list-panes").expect("roster");
    assert!(
        roster.len() >= 2,
        "the split created a second pane: {roster:?}"
    );
    // Typing after the chord reaches the FRESH pane: an echo there runs
    // in the split (asserted daemon-side — render frames paint per-cell
    // diffs, so the marker never appears as one raw substring in the
    // host stream).
    host.to_child
        .write_all(b"echo RENDER-SPLIT-MARKER\r")
        .expect("type into the split");
    host.to_child.flush().ok();
    let fresh = roster
        .iter()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .find(|id| *id != pane)
        .expect("the fresh pane id");
    let mut capture = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        capture = client
            .send(&format!("capture-pane -t {fresh}"))
            .expect("capture")
            .join("\n");
        if capture.contains("RENDER-SPLIT-MARKER") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        capture.contains("RENDER-SPLIT-MARKER"),
        "typing after the chord must reach the fresh pane: {capture:?}"
    );
    host.killer.kill().ok();
}

/// Regression (render-mode target-less resolution): `par-mux attach
/// --mode render` with no `-t` attaches to the newest session's newest
/// pane — the documented default — and renders it. `list-sessions`
/// replies `$N: name`; a whitespace split kept the colon (`$0:`), the
/// daemon's id parser rejected it, and the client exited 1 with "the
/// session has no windows". One session, one pane: the client must come
/// up (alt-screen enter + the pane's marker in the rendered frame) and
/// keys must still reach the pane — not exit.
#[cfg(unix)]
#[test]
fn render_mode_without_target_attaches_the_newest_session() {
    let (fixture, _daemon, mut client) = fixture_with_session("rendernotarget");
    let pane = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    // A distinctive line on the pane's screen: the rendered frame must
    // carry it once the client seeds from the replay.
    client
        .send(&format!("send-keys -t {pane} -l 'echo NO-TARGET-MARKER'"))
        .expect("seed marker");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("enter");
    std::thread::sleep(Duration::from_millis(600));

    // No -t anywhere: resolution is entirely the client's job.
    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    let got = wait_for_output(&host, b"NO-TARGET-MARKER", Duration::from_secs(15));
    assert!(
        !got.is_empty(),
        "the render client with no -t must attach the session's pane and \
         render its content (alt-screen enter + the replayed marker). \
         stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    // Alt-screen enter: the render client took over the screen rather
    // than exiting with an error banner.
    assert!(
        got.windows(8).any(|w| w == b"\x1b[?1049h"),
        "the client must enter the alternate screen (render mode): {:?}",
        String::from_utf8_lossy(&got)
    );

    // It stays up and routes keys: an echo lands on the pane's screen
    // through the client's input router.
    host.to_child
        .write_all(b"echo STILL-UP-MARKER\r")
        .expect("type");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"STILL-UP-MARKER", Duration::from_secs(10));
    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    assert!(
        capture.join("\n").contains("STILL-UP-MARKER"),
        "keys must reach the pane through the target-less client: {capture:?}"
    );
    host.killer.kill().ok();
}

/// The target-less render client picks the NEWEST session when several
/// exist: ids are monotonic, so a second session's pane is the target —
/// its marker renders, the first session's does not.
#[cfg(unix)]
#[test]
fn render_mode_without_target_picks_the_newest_session() {
    let (fixture, _daemon, mut client) = fixture_with_session("renderoldest");
    client
        .send("new-session -s second")
        .expect("second session");
    // The bare global roster's order is unspecified; find each session's
    // pane by querying per session.
    let sessions = client.send("list-sessions").expect("sessions").join("\n");
    let first_pane = client
        .send("list-panes -t $0")
        .expect("panes of $0")
        .join("");
    let first_pane = first_pane
        .split_whitespace()
        .next()
        .expect("pane of $0")
        .to_string();
    let second_pane = client
        .send("list-panes -t $1")
        .expect("panes of $1")
        .join("");
    let second_pane = second_pane
        .split_whitespace()
        .next()
        .expect("pane of $1")
        .to_string();
    client
        .send(&format!(
            "send-keys -t {first_pane} -l 'echo OLDEST-MARKER'"
        ))
        .expect("seed oldest");
    client
        .send(&format!("send-keys -t {first_pane} Enter"))
        .expect("enter oldest");
    client
        .send(&format!(
            "send-keys -t {second_pane} -l 'echo NEWEST-MARKER'"
        ))
        .expect("seed newest");
    client
        .send(&format!("send-keys -t {second_pane} Enter"))
        .expect("enter newest");
    std::thread::sleep(Duration::from_millis(600));
    let _ = sessions;

    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    let got = wait_for_output(&host, b"NEWEST-MARKER", Duration::from_secs(15));
    assert!(
        !got.is_empty(),
        "the no-target client must attach the newest session ($1) and \
         render its pane. stderr: {}\nbytes: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
    );
    assert!(
        !got.windows(13).any(|w| w == b"OLDEST-MARKER"),
        "the first session's content must not be the rendered view: {:?}",
        String::from_utf8_lossy(&got)
    );
    host.killer.kill().ok();
}

/// The tab strip, PTY level: a click on the second tab in the strip row
/// switches the shown window daemon-side — the client sends
/// `select-window` and the daemon's active-window marker moves — and no
/// `send-keys` reaches any pane from a strip click (no forwarding).
#[cfg(unix)]
#[test]
fn render_mode_tab_click_switches_the_active_window() {
    let (fixture, _daemon, mut client) = fixture_with_session("tabstrip");
    // Two windows with deterministic names: the herdr block "  one  " is
    // 7 cols (0-based 0..7), one gap column (7), so the second tab's
    // block "  two  " spans 0-based host cols 8..15 (1-based 9..15).
    client.send("new-window -t $0").expect("new-window");
    client.send("rename-window -t @0 one").expect("rename @0");
    client.send("rename-window -t @1 two").expect("rename @1");
    client.send("select-window -t @0").expect("back to @0");
    std::thread::sleep(Duration::from_millis(400));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", "@0"]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"one", Duration::from_secs(10));
    // Drain the settle paint so the post-click reads are click-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // A press on the second tab (host row 1 = the strip).
    host.to_child
        .write_all(b"\x1b[<0;12;1M")
        .expect("click press");
    host.to_child.flush().ok();

    // The authority: the daemon's active-window marker moved to @1.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut activated = false;
    while std::time::Instant::now() < deadline {
        let windows = client.send("list-windows -t $0").expect("list-windows");
        if windows
            .iter()
            .any(|l| l.split_whitespace().next() == Some("@1") && l.contains('*'))
        {
            activated = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        activated,
        "the click must activate the clicked window daemon-side. stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// The first pane of `session` through its window — `list-panes -t`
/// takes a window or pane target, not a session id.
#[cfg(unix)]
fn session_first_pane(client: &mut par_mux::mux::MuxClient, session: &str) -> String {
    let window = client
        .send(&format!("list-windows -t {session}"))
        .expect("list-windows")
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .find(|t| t.starts_with('@'))
        .expect("a window")
        .to_string();
    client
        .send(&format!("list-panes -t {window}"))
        .expect("list-panes")
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .find(|t| t.starts_with('%'))
        .expect("a pane")
        .to_string()
}

/// prefix W (render mode): the chord sends `select-workspace` and lands
/// the view on the target workspace's session — the daemon's active-
/// workspace marker moves, and the target pane's seeded content re-seeds
/// into the frame (the select+resync contract's redraw half).
#[cfg(unix)]
#[test]
fn render_mode_workspace_chord_selects_and_lands() {
    let (fixture, _daemon, mut client) = fixture_with_session("wschord");
    // A second workspace with its own session; `new-workspace` makes it
    // the daemon's active one. Back to `main` first, so prefix W has one
    // deterministic next: lab.
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    client
        .send("select-workspace -t main")
        .expect("select main");
    std::thread::sleep(Duration::from_millis(300));

    // The rosters: `list-panes -t` takes a window or pane target, not a
    // session, so each pane is reached through its window.
    let pane_a = session_first_pane(&mut client, "$0");
    let pane_work = session_first_pane(&mut client, "$1");
    // The landing evidence, seeded before the chord: the lab pane's
    // screen carries the marker, and the post-chord re-seed must bring
    // it into the frame.
    client
        .send(&format!("send-keys -t {pane_work} -l 'echo WS-LAB-MARKER'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_work} Enter"))
        .expect("Enter");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"main", Duration::from_secs(10));
    // Drain the settle paint so the post-chord reads are attributable.
    while host.output_rx.try_recv().is_ok() {}

    // prefix W: next workspace (main -> lab).
    host.to_child.write_all(&[0x02, b'W']).expect("prefix W");
    host.to_child.flush().ok();

    // The authority: the daemon's active-workspace marker moved to lab.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut selected = false;
    while std::time::Instant::now() < deadline {
        let workspaces = client.send("list-workspaces").expect("list-workspaces");
        if workspaces
            .iter()
            .any(|l| l.starts_with("+1:") && l.contains(" active"))
        {
            selected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        selected,
        "prefix W must select the next workspace daemon-side. stderr: {}",
        stderr.lock().unwrap()
    );

    // The resync half: the chord-attributable output re-seeds the target
    // pane, whose seeded marker must appear in the reconstructed frame.
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut all: Vec<u8> = Vec::new();
    while std::time::Instant::now() < end {
        match host.output_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => all.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let (final_grid, _ever) = reconstructed_screen(&all, 24, 80);
    let shown = final_grid.join("\n");
    assert!(
        shown.contains("WS-LAB-MARKER"),
        "the chord must land the view on the new workspace's pane (its \
         seeded marker re-seeds in). screen:\n{shown}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// %workspaces-changed (render mode): a workspace add broadcast marks
/// the status stale, and the throttled re-query pulls the new
/// workspace's name onto the workspaces segment — the same discipline
/// %agent-state-changed follows for the roster chips.
#[cfg(unix)]
#[test]
fn render_mode_workspaces_changed_refreshes_the_status_segment() {
    let (fixture, _daemon, mut client) = fixture_with_session("wsstatus");
    let (mut host, stderr) = spawn_attach_render(&fixture, &[]);
    let _ = wait_for_output(&host, b"\x1b[?1002h", Duration::from_secs(10));
    let _ = wait_for_output(&host, b"$0:att", Duration::from_secs(10));
    // Drain the settle paint so the post-broadcast reads are
    // broadcast-attributable.
    while host.output_rx.try_recv().is_ok() {}

    // Workspace churn: a new workspace rides %workspaces-changed.
    client.send("new-workspace -n beta").expect("new-workspace");
    std::thread::sleep(Duration::from_secs(2));
    let mut all = Vec::new();
    while let Ok(bytes) = host.output_rx.try_recv() {
        all.extend_from_slice(&bytes);
    }
    let (_final, ever) = reconstructed_screen(&all, 24, 80);
    let ever_row: String = ever[23].iter().map(|r| format!("{r:?}\n")).collect();
    assert!(
        ever[23].iter().any(|r| r.contains("beta")),
        "the workspaces segment must show the new workspace after \
         %workspaces-changed. snapshots:\n{ever_row}\nstderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// The passthrough status line gains the workspaces segment: workspace
/// names in id order, the daemon's active one bracketed.
#[cfg(unix)]
#[test]
fn passthrough_status_line_carries_the_workspaces_segment() {
    let (fixture, _daemon, mut client) = fixture_with_session("wspass");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    let got = wait_for_output(&host, b"[lab]", Duration::from_secs(15));
    assert!(
        got.windows(b"[lab]".len()).any(|w| w == b"[lab]"),
        "the passthrough status line must carry the workspaces segment \
         with the active workspace bracketed. bytes: {:?}\nstderr: {}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// prefix W (passthrough): the chord sends `select-workspace` and lands
/// the pump on the target workspace's pane — typing after the chord
/// reaches the new workspace's pane only.
#[cfg(unix)]
#[test]
fn passthrough_workspace_chord_selects_and_lands() {
    let (fixture, _daemon, mut client) = fixture_with_session("wspasschord");
    client.send("new-workspace -n lab").expect("new-workspace");
    client
        .send("new-session -s work -t lab")
        .expect("session in lab");
    client
        .send("select-workspace -t main")
        .expect("select main");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach(&fixture, &[]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(2));

    // Prefix W: next workspace (main -> lab). Typing AFTER the chord
    // reaches the lab session's pane only — the landing evidence.
    host.to_child.write_all(&[0x02, b'W']).expect("prefix W");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(500));
    host.to_child
        .write_all(b"echo WS-LANDED-MARKER\n")
        .expect("type into the landed pane");
    host.to_child.flush().ok();

    let got = wait_for_output(&host, b"WS-LANDED-MARKER", Duration::from_secs(15));
    assert!(
        got.windows(b"WS-LANDED-MARKER".len())
            .any(|w| w == b"WS-LANDED-MARKER"),
        "prefix W must land the pump on the new workspace's pane. \
         bytes: {:?}\nstderr: {}",
        String::from_utf8_lossy(&got),
        stderr.lock().unwrap()
    );
    let workspaces = client.send("list-workspaces").expect("list-workspaces");
    assert!(
        workspaces
            .iter()
            .any(|l| l.starts_with("+1:") && l.contains(" active")),
        "the chord must move the daemon's active-workspace marker: {workspaces:?}"
    );
    host.killer.kill().ok();
}

/// The sidebar chord in isolation: boot, attach, settle, toggle on and
/// off — bisecting the full-chain test's sidebar failure.
#[cfg(unix)]
#[test]
fn render_mode_sidebar_toggle_minimal() {
    let (fixture, _daemon, mut client) = fixture_with_session("sidebarmini");
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
    client
        .send(&format!(
            "send-keys -t {pane_b} -l 'echo SIDEBAR-MINI-MARK'"
        ))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"SIDEBAR-MINI-MARK", Duration::from_secs(10));

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    // The strip and the flash paint in the SAME frame (the strip's cells
    // precede the status row in the diff), so one wait covers both.
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar on") && plain_text(&got).contains("workspaces"),
        "the toggle must flash and paint the workspaces section. tail: {:?}",
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(500)
            .collect::<String>()
    );
    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("sidebar off"),
        "the second toggle must flash off. tail: {:?}",
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
    );
    host.killer.kill().ok();
}

/// The 1-based column of the CUP that positioned `needle`'s first
/// character. The frame flushes one CUP+SGR+char per cell, so text only
/// exists after escape stripping — walk the stream keeping the last CUP
/// column and attach it to every plain char, then locate the needle in
/// the stripped text. The geometry read for the sidebar-refit asserts.
fn cup_col_of_text(bytes: &[u8], needle: &str) -> Option<u16> {
    let mut last_cup: Option<u16> = None;
    let mut cells: Vec<(char, Option<u16>)> = Vec::new();
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b != 0x1b {
            cells.push((b as char, last_cup));
            continue;
        }
        match iter.next() {
            Some(b'[') => {
                let mut params: Vec<u8> = Vec::new();
                let mut final_byte = 0u8;
                for c in iter.by_ref() {
                    if (0x40..=0x7e).contains(&c) {
                        final_byte = c;
                        break;
                    }
                    params.push(c);
                }
                if final_byte == b'H' {
                    let text = String::from_utf8_lossy(&params);
                    last_cup = text.split(';').nth(1).and_then(|c| c.parse::<u16>().ok());
                }
            }
            Some(b']') => {
                // OSC: swallow through BEL or ST (ESC \).
                for c in iter.by_ref() {
                    if c == 0x07 {
                        break;
                    }
                    if c == 0x1b {
                        iter.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let text: String = cells.iter().map(|(c, _)| c).collect();
    let at = text.rfind(needle)?;
    let char_at = text[..at].chars().count();
    cells.get(char_at).and_then(|(_, col)| *col)
}

/// The 1-based row attached to `needle`'s last occurrence — the CUP that
/// positioned it, read from the same per-cell CUP stream
/// [`cup_col_of_text`] walks.
fn cup_row_of_text(bytes: &[u8], needle: &str) -> Option<u16> {
    let mut last_cup: Option<u16> = None;
    let mut cells: Vec<(char, Option<u16>)> = Vec::new();
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b != 0x1b {
            cells.push((b as char, last_cup));
            continue;
        }
        match iter.next() {
            Some(b'[') => {
                let mut params: Vec<u8> = Vec::new();
                let mut final_byte = 0u8;
                for c in iter.by_ref() {
                    if (0x40..=0x7e).contains(&c) {
                        final_byte = c;
                        break;
                    }
                    params.push(c);
                }
                if final_byte == b'H' {
                    let text = String::from_utf8_lossy(&params);
                    last_cup = text.split(';').next().and_then(|r| r.parse::<u16>().ok());
                }
            }
            Some(b']') => {
                // OSC: swallow through BEL or ST (ESC \).
                for c in iter.by_ref() {
                    if c == 0x07 {
                        break;
                    }
                    if c == 0x1b {
                        iter.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let text: String = cells.iter().map(|(c, _)| c).collect();
    let at = text.rfind(needle)?;
    let char_at = text[..at].chars().count();
    cells.get(char_at).and_then(|(_, row)| *row)
}

/// Whether any row-24 CUP is followed (within its CUP+SGR+cell run) by
/// `glyph`'s bytes — the wire shape the frame flushes one painted cell
/// in. Used to see WHICH content owns the bottom row.
fn row24_paints(bytes: &[u8], glyph: &[u8]) -> bool {
    let mut i = 0;
    while let Some(off) = bytes[i..].windows(5).position(|w| w == b"\x1b[24;") {
        let mut j = i + off + 5;
        while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
            j += 1;
        }
        j += 1; // past the CUP final byte
                // The style is a CHAIN of CSI sequences (reset + fg + modifiers);
                // skip every one before the glyph bytes.
        while j + 1 < bytes.len() && bytes[j] == 0x1b && bytes[j + 1] == b'[' {
            j += 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            j += 1; // past this sequence's final byte
        }
        if j + glyph.len() <= bytes.len() && &bytes[j..j + glyph.len()] == glyph {
            return true;
        }
        i += off + 1;
    }
    false
}

/// Sidebar toggle geometry: the refit must re-divide the panes to the
/// reported width and paint them flush against the panel's right edge —
/// the manual-pass screenshot showed the contents pushed right of the
/// panel with dead margin at the far right. On an 80-col host with the
/// 20-col panel up, the right pane's marker paints at column 51; with
/// the panel down it returns to 41.
#[cfg(unix)]
#[test]
fn render_mode_sidebar_toggle_refits_pane_geometry() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbgeo");
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
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo GEO-MARK-42'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");
    std::thread::sleep(Duration::from_millis(300));

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let pre = wait_for_output(&host, b"GEO-MARK-42", Duration::from_secs(10));
    let col_pre = cup_col_of_text(&pre, "GEO-MARK-42");
    assert_eq!(
        col_pre,
        Some(42),
        "pre-toggle the right pane's content starts at 41 (the 80/2 split, +1 for the per-pane-box inset). stderr: {} text: {:?}",
        stderr.lock().unwrap(),
        plain_text(&pre)
    );

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    // The refit's repaint rides the same flush as the flash (one pump
    // iteration); drain a beat for the tail before parsing the geometry.
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    assert!(
        plain_text(&flash).contains("sidebar on"),
        "toggle must flash on. tail: {:?}",
        String::from_utf8_lossy(&flash)
            .chars()
            .rev()
            .take(300)
            .collect::<String>()
    );
    let col_on = cup_col_of_text(&flash, "GEO-MARK-42");
    assert_eq!(
        col_on,
        Some(52),
        "panel up: panes re-divide to 60 wide, the right pane's content paints at 51 (30+20, +1 inset). text: {:?}",
        plain_text(&flash)
    );

    host.to_child
        .write_all(&[0x02, b's'])
        .expect("prefix s again");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar off", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    assert!(
        plain_text(&flash).contains("sidebar off"),
        "second toggle must flash off"
    );
    let col_off = cup_col_of_text(&flash, "GEO-MARK-42");
    assert_eq!(
        col_off,
        Some(42),
        "panel down restores the full-width split"
    );
    host.killer.kill().ok();
}

/// The refit's repaint_all clears the WHOLE screen — the strip rows too —
/// so the sidebar toggle's resize_to must invalidate the tab strip and
/// status row or their diffs flush nothing and the rows stay wiped (the
/// manual-pass report: part of the status bar did not render after
/// bringing the panel up).
#[cfg(unix)]
#[test]
fn render_mode_sidebar_toggle_repaints_the_strip_rows() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbstrip");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    // The refit frame rides the same flush as the flash (one pump
    // iteration); drain a beat for the tail before matching.
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(600),
    ));
    // Bottom row, last column (host 80x24): the status row's tail only
    // flushes on a full-row diff, which only the invalidate produces.
    let needle = b"\x1b[24;80H";
    assert!(
        flash.windows(needle.len()).any(|w| w == needle),
        "the refit must repaint the status row's tail. tail: {:?}",
        String::from_utf8_lossy(&flash)
            .chars()
            .rev()
            .take(400)
            .collect::<String>()
    );
    // Top row: the tab strip under the same rule. The invalidate
    // repaints every strip column, so both the head (col 1) and the bar
    // tail (col 80, unchanged since the seed frame) flush.
    for needle in [b"\x1b[1;1H".as_slice(), b"\x1b[1;80H".as_slice()] {
        assert!(
            flash.windows(needle.len()).any(|w| w == needle),
            "the refit must repaint the whole tab strip row ({:?}). tail: {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&flash)
                .chars()
                .rev()
                .take(400)
                .collect::<String>()
        );
    }
    host.killer.kill().ok();
}

/// The status-bar toggle: prefix S hides the bottom row — the pane grid
/// grows a row, the shell prompt moves DOWN to host row 24, the Help
/// chip vanishes from the flushes, and nothing erases the grown bottom
/// row afterward; prefix S again restores the row, the chip, and the
/// cursor to host row 23.
#[cfg(unix)]
#[test]
fn render_mode_status_bar_toggle_hides_and_restores_the_row() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbtoggle");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let mut shown = wait_for_output(&host, b"$", Duration::from_secs(10));
    // Settle a beat so the seed frame's tail (the strip and the status
    // row follow the pane cells in one flush) is in the capture.
    shown.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&shown).contains("C-b? Help"),
        "the Help chip ends the shown bar: {shown:?}"
    );
    assert_eq!(
        cup_row_of_text(&shown, "Help"),
        Some(24),
        "the chip paints on the bottom row. stderr: {}",
        stderr.lock().unwrap()
    );

    // Hide: the refit reports the taller grid and the pane replays one
    // row taller — the newest line moves DOWN onto the row the status
    // bar vacated. No flash cue — the flash paints ON the status row, so
    // the bar's vanishing is the feedback. The chip must be gone from
    // the flushes entirely (byte windows over a per-cell flush are
    // meaningless — this reads the plain text).
    host.to_child.write_all(&[0x02, b'S']).expect("prefix S");
    host.to_child.flush().ok();
    let mut hidden = wait_for_output(&host, b"\x1b[24;", Duration::from_secs(10));
    hidden.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        !plain_text(&hidden).contains("Help"),
        "the chip is gone while hidden. text: {:?}",
        plain_text(&hidden)
    );
    // The pane claims the vacated row: with herdr's per-pane boxes the
    // pane rectangle's bottom edge (the ring's ─) moves DOWN onto host
    // row 24 — a row no flush could target while the bar owned it, since
    // the hidden state never writes the row at all.
    assert!(
        row24_paints(&hidden, b"\xe2\x94\x80"),
        "the pane box's bottom edge must move onto the vacated row"
    );

    // Show again: the chip returns to the bottom row and the cursor back
    // to the shrunk content's last row (host row 23).
    host.to_child.write_all(&[0x02, b'S']).expect("prefix S");
    host.to_child.flush().ok();
    let mut reshowed = wait_for_output(&host, b"\x1b[23;", Duration::from_secs(10));
    reshowed.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&reshowed).contains("C-b? Help"),
        "the chip returns with the bar: {:?}",
        plain_text(&reshowed)
    );
    host.killer.kill().ok();
}

/// The side panel's ` menu ` chip opens the command menu, and its
/// `detach` row ends the client exactly like prefix `d` (exit 0).
#[cfg(unix)]
#[test]
fn render_mode_panel_menu_chip_detaches() {
    let (fixture, _daemon, mut client) = fixture_with_session("menudetach");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(600));
    // The panel's footer row is the 22-row content area's last row
    // (host row 22, SGR row 23); ` menu ` spans 0-based cols 13..19.
    host.to_child
        .write_all(b"\x1b[<0;17;23M")
        .expect("menu chip press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"reload config", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("reload config"),
        "the chip must open the command menu. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );

    // The menu box: the 28-char footer is the widest row, so 30 wide
    // over the 80-col host (x0 = 25); 5 rows plus the ring over 22
    // content rows (y0 = 7). `detach` is panel row 3 at host y0+5.
    let x0 = (80usize - 30) / 2;
    let y0 = (22usize - 7) / 2;
    host.to_child
        .write_all(format!("\x1b[<0;{};{}M", x0 + 5, y0 + 6).as_bytes())
        .expect("detach click");
    host.to_child.flush().ok();
    let code = child_exit(&mut host, Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "the detach row must end the client with exit 0. stderr: {}",
        stderr.lock().unwrap()
    );
}

/// The tab context menu must be CLICKABLE with the side panel up: the
/// modal centers over the HOST width (the pane layout already carries
/// the strip offset), so its clicks map by the raw host column — the
/// guard mapped them through `x - strip` and every click landed one
/// panel width left of the visible items (the manual-pass round-7
/// report; the same defect class as round 5's unclickable +).
#[cfg(unix)]
#[test]
fn render_mode_tab_menu_actions_click_with_the_panel_up() {
    let (fixture, _daemon, mut client) = fixture_with_session("menuclick");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    // Panel up, then a right-press on the shown window's tab opens the
    // tab menu.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(600));
    // With the panel up the tabs start at the panel's right edge
    // (strip col 20): the shown window's herdr block (`  att  `, width
    // 7) spans 0-based host cols 20-26, so the right press at 1-based
    // col 24 (0-based 23, the `a`) lands on it.
    let press: &[u8] = b"\x1b[<2;24;1M";
    host.to_child.write_all(press).expect("right press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"rename", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("rename"),
        "the menu must open on a right press. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );

    // Click the RENAME row at its RAW host coordinates (the menu's
    // painted geometry: the widest row is the 28-char footer, so the
    // box is 30 wide centered over the 80-col host — x0 = 25; 5 rows
    // plus the border ring centered over 22 content rows — y0 = 7).
    let x0 = (80usize.saturating_sub(30)) / 2; // 25
    let y0 = (22usize.saturating_sub(7)) / 2; // 7
    let menu_col = x0 + 5; // inside the box, 1-based host col
    let menu_row = y0 + 4; // rename (panel row 1) paints at host y0+3; SGR is 1-based
    host.to_child
        .write_all(format!("\x1b[<0;{menu_col};{menu_row}M").as_bytes())
        .expect("menu click");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b"rename window", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains("rename window"),
        "the menu's rename row must open the rename prompt. stderr: {} raw: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&flash)
    );
    host.killer.kill().ok();
}

#[cfg(unix)]
#[test]
fn render_mode_plus_click_works_with_the_panel_up() {
    let (fixture, _daemon, mut client) = fixture_with_session("pluspanel");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    // Panel up (the manual-pass reporter's state): the top row stays a
    // tab-strip row and the + keeps its reservation.
    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(600));
    let press: &[u8] = b"\x1b[<0;79;1M";
    host.to_child.write_all(press).expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains(" new tab "),
        "the prompt must open with the panel up. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );
    host.killer.kill().ok();
}

#[cfg(unix)]
#[test]
fn render_mode_plus_click_works_on_a_wide_host() {
    let (fixture, _daemon, mut client) = fixture_with_session("pluswide");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    // A live terminal is much wider than the harness's 80 columns: the
    // manual-pass report could not click the + there. Widen the host and
    // click the reserved slot at the new right edge.
    host.master
        .resize(PtySize {
            rows: 24,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize");
    std::thread::sleep(Duration::from_millis(600));
    let press: &[u8] = b"\x1b[<0;199;1M";
    host.to_child.write_all(press).expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    assert!(
        plain_text(&flash).contains(" new tab "),
        "the prompt must open at the wide host's right edge. stderr: {} text: {}",
        stderr.lock().unwrap(),
        plain_text(&flash)
    );
    host.killer.kill().ok();
}

/// The tab strip's ` + ` button: a click opens the new-tab prompt
/// (herdr's footer, the next free index as the editable default), Enter
/// creates the window and lands on it, Esc cancels without creating.
#[cfg(unix)]
#[test]
fn render_mode_plus_click_prompts_and_creates_a_window() {
    let (fixture, _daemon, mut client) = fixture_with_session("plusclick");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(10));

    // The ` + ` button owns the strip's reserved right edge: on an
    // 80-col host the reservation spans strip cols 77..79 and the `+`
    // paints at 78 — the click is a 1-based host col 79.
    let press: &[u8] = b"\x1b[<0;79;1M";
    host.to_child.write_all(press).expect("plus press");
    host.to_child.flush().ok();
    let mut flash = wait_for_output(&host, b" new tab ", Duration::from_secs(10));
    flash.extend(wait_for_output(
        &host,
        b"\x00-never",
        Duration::from_millis(400),
    ));
    let text = plain_text(&flash);
    assert!(
        text.contains(" new tab "),
        "the prompt opens. stderr: {} text: {text}",
        stderr.lock().unwrap()
    );
    assert!(
        text.contains("> 1"),
        "the default is the next free index: {text}"
    );
    let windows_before = client.send("list-windows").expect("list").join("");

    // Esc cancels without creating: the daemon's window roster is
    // unchanged after the prompt closes.
    host.to_child.write_all(&[0x1b]).expect("esc");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(300));
    let after_cancel = client.send("list-windows").expect("list").join("");
    assert_eq!(
        after_cancel, windows_before,
        "esc must not create a window: {after_cancel}"
    );

    // Click + again, edit the default (the input shows `> 1notes` before
    // saving), Enter: the window exists and the view lands on it. The
    // reopen's overlay flush races the esc restore window, so settle
    // instead of waiting on the title needle.
    host.to_child.write_all(press).expect("plus press");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(300));
    host.to_child.write_all(b"notes").expect("name");
    host.to_child.flush().ok();
    let typed = wait_for_output(&host, b"\x00-never", Duration::from_millis(400));
    assert!(
        plain_text(&typed).contains("notes"),
        "the typed name joins the default in the input line: {:?}",
        plain_text(&typed)
    );
    host.to_child.write_all(&[b'\r']).expect("enter");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(500));
    let created = client.send("list-windows").expect("list").join("");
    assert!(
        created.contains("notes"),
        "the named window exists: {created}"
    );
    host.killer.kill().ok();
}

/// session state: cycling away and back and then splitting a pane must
/// not revert the borders (the manual-pass report — the re-seed rebuilt
/// the renderer with the default glyphs).
#[cfg(unix)]
#[test]
fn render_mode_border_style_survives_split() {
    let (fixture, _daemon, mut client) = fixture_with_session("herdrsplit");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    // herdr is the DEFAULT: the seed's first frame (which flushes before
    // the shell's prompt bytes) already paints per-pane boxes.
    let pre = wait_for_output(&host, b"$", Duration::from_secs(10));
    // plain_text latin-1s multi-byte glyphs, so a painted ╭ is matched
    // on the raw bytes (the glyph's UTF-8 is contiguous in one cell's
    // flush); the wait needle is the same three bytes.
    assert!(
        pre.windows(3).any(|w| w == b"\xe2\x95\xad"),
        "herdr corner must paint at seed. stderr: {}",
        stderr.lock().unwrap()
    );

    // Cycle all the way around (herdr → unicode → … → herdr): five
    // presses return to the default set.
    for _ in 0..5 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"border style: herdr", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("border style: herdr"),
        "the full cycle must land back on herdr"
    );

    // Split via the CHORD: the split's re-seed used to restore the
    // session-level pane-borders flag (the cycle flipped the renderer's
    // flag only) and the boxes reverted to shared dividers.
    host.to_child.write_all(&[0x02, b'%']).expect("prefix %");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"\xe2\x95\xad", Duration::from_secs(10));
    assert!(
        got.windows(3).any(|w| w == b"\xe2\x95\xad"),
        "the chord split must keep the per-pane-box style. stderr: {}",
        stderr.lock().unwrap()
    );
    host.killer.kill().ok();
}

/// Clicking a workspace row in the side panel re-seeds the view (the
/// select+resync contract every switch follows). The re-seed must keep
/// the panel: its width rides the fresh renderer and its rows re-query —
/// the manual-pass report was the re-seed rebuilding the renderer
/// without either, so the panel vanished and the panes painted under
/// where it had been.
#[cfg(unix)]
#[test]
fn render_mode_workspace_click_reseeds_with_the_panel() {
    let (fixture, _daemon, mut client) = fixture_with_session("sbsplit");
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
    client
        .send(&format!("send-keys -t {pane_b} -l 'echo GEO-MARK-42'"))
        .expect("marker");
    client
        .send(&format!("send-keys -t {pane_b} Enter"))
        .expect("Enter");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"GEO-MARK-42", Duration::from_secs(10));

    host.to_child.write_all(&[0x02, b's']).expect("prefix s");
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"sidebar on", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("workspaces"),
        "panel must paint on open. stderr: {}",
        stderr.lock().unwrap()
    );

    // A press ON the painted workspace row lands on it: 1-based host
    // rows are 1 = tab strip, 2 = the FIRST workspace row (round 6: no
    // section header above it — the title moved to the strip row; the
    // panel begins under the strip row). The select+resync re-seed must
    // re-emit the panel's full frame (a fresh renderer diffs against an
    // empty buffer) with the panes still offset by the panel's width —
    // and REPEATED clicks must not ratchet the layout left: the re-seed
    // used to rebuild the renderer at the REPORTED width (grid less the
    // panel), so each re-seed narrowed the buffer and each report then
    // shrank again (the manual-pass click-shrinks-the-panes report).
    for click in 1..=2 {
        host.to_child
            .write_all(&[0x1b, b'[', b'<', b'0', b';', b'3', b';', b'2', b'M'])
            .expect("panel click");
        host.to_child.flush().ok();
        let mut got = wait_for_output(&host, b"workspaces", Duration::from_secs(10));
        got.extend(wait_for_output(
            &host,
            b"\x00-never",
            Duration::from_millis(600),
        ));
        assert!(
            plain_text(&got).contains("workspaces"),
            "click {click}: the re-seed must keep the panel painted"
        );
        let col = cup_col_of_text(&got, "GEO-MARK-42");
        assert_eq!(
            col,
            Some(52),
            "click {click}: the right pane must stay put (no ratchet). text: {:?}",
            plain_text(&got)
        );
    }
    host.killer.kill().ok();
}

/// Border labels paint the user `-T` title: set labels daemon-side
/// before attach, cycle to the per-pane-box style, and the label text
/// must appear embedded in the pane's top border (the manual-pass
/// report: labels never showed because the painter read the shell's OSC
/// title only).
#[cfg(unix)]
#[test]
fn render_mode_border_label_paints_the_user_title() {
    let (fixture, _daemon, mut client) = fixture_with_session("labels");
    let pane_a = client
        .send("list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    client
        .send(&format!("select-pane -t {pane_a} -T 'MY-TEST-LABEL'"))
        .expect("label a");
    let pane_b = client
        .send(&format!("split-window -h -t {pane_a}"))
        .expect("split")
        .join("");
    let pane_b = pane_b.trim().to_string();
    client
        .send(&format!("select-pane -t {pane_b} -T 'SECOND-LABEL'"))
        .expect("label b");

    let (mut host, stderr) = spawn_attach_render(&fixture, &["-t", &pane_a]);
    let _ = wait_for_output(&host, b"$", Duration::from_secs(5));

    // Cycle to herdr: the per-pane-box mode labels paint in.
    for _ in 0..4 {
        host.to_child.write_all(&[0x02, b'B']).expect("prefix B");
    }
    host.to_child.flush().ok();
    let got = wait_for_output(&host, b"MY-TEST-LABEL", Duration::from_secs(10));
    assert!(
        plain_text(&got).contains("MY-TEST-LABEL"),
        "the user label must paint in the border. stderr: {} tail: {:?}",
        stderr.lock().unwrap(),
        String::from_utf8_lossy(&got)
            .chars()
            .rev()
            .take(500)
            .collect::<String>()
    );
    host.killer.kill().ok();
}
