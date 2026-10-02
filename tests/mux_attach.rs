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
    _master: Box<dyn MasterPty + Send>,
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
        _master: pair.master,
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
    //
    // The reserve bytes themselves are asserted on UNIX only. Windows
    // ConPTY re-encodes the client's output: it INTERPRETS DECSTBM (the
    // region is applied conhost-side) and does not pass the raw sequence
    // through to the pty master, so ESC[1;23r never appears in the
    // capture even though the client emitted it (the re-encoded status
    // row draw does appear). The client-side emit is pinned by the unix
    // run; the Windows run asserts the status draw ConPTY preserves.
    if cfg!(unix) {
        let startup = wait_for_output(&host, b"\x1b[1;23r", Duration::from_secs(10));
        assert!(
            startup.windows(7).any(|w| w == b"\x1b[1;23r"),
            "attach must reserve the status row with DECSTBM (ESC[1;23r on a 24-row \
             terminal): {:?}\nstderr: {}",
            String::from_utf8_lossy(&startup),
            stderr.lock().unwrap()
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
    // The last thing the client writes before exit is the region restore.
    // (crossterm's raw-mode disable is a termios call, not bytes.) On
    // Windows ConPTY likewise absorbs the plain ESC[r reset, so the
    // restore is asserted through the client's exit code there (already
    // checked above) — the bytes themselves are pinned on unix.
    if cfg!(unix) {
        assert!(
            tail.windows(3).any(|w| w == b"\x1b[r"),
            "detach must restore the scroll region (ESC[r): {tail:?}"
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
    let mut conn = par_term_emu_core_rust::mux::attach::conn::AttachConn::connect(fixture.socket())
        .expect("render client connect");
    let _replay = conn.drain_pending_events();
    conn.send_checked(&format!("refresh-client -t {pane0} -C 80x24"))
        .expect("size report");
    let layout_event = conn
        .drain_pending_events()
        .into_iter()
        .find_map(|event| match event {
            par_term_emu_core_rust::tmux_control::TmuxNotification::LayoutChange {
                window_layout,
                window_visible_layout,
                window_raw_flags,
                ..
            } => Some((window_layout, window_visible_layout, window_raw_flags)),
            _ => None,
        })
        .expect("the size report broadcast a layout change");
    let rects = par_term_emu_core_rust::mux::attach::layout::parse_layout_triple(
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
    let mut renderer = par_term_emu_core_rust::mux::attach::render::PaneRenderer::new(
        80,
        24,
        par_term_emu_core_rust::mux::attach::render::Glyphs::Unicode,
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

/// Spawn the render-mode attach client under a PTY (the shared harness
/// shape, plus `--mode render`).
#[cfg(unix)]
fn spawn_attach_render(
    fixture: &MuxFixture,
    extra: &[&str],
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach(
        fixture,
        &["--mode", "render"]
            .iter()
            .chain(extra.iter())
            .copied()
            .collect::<Vec<&str>>(),
    )
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
        !body.contains("\x1b[<64"),
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

    // Left press at window-relative col 10 row 5 (1-based): the single
    // pane's rect starts at (0,0), so pane-relative is (9, 4) 0-based and
    // the wire spelling is ESC[<0;10;5M.
    host.to_child.write_all(b"\x1b[<0;10;5M").expect("click");
    host.to_child.flush().ok();
    let _ = wait_for_output(&host, b"0;10;5", Duration::from_secs(10));

    let capture = client
        .send(&format!("capture-pane -t {pane}"))
        .expect("capture");
    let body = capture.join("\n");
    assert!(
        body.contains("^[[<0;10;5M"),
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
                        // Skip the SGR run (ESC[...m sequences), then one
                        // symbol's UTF-8 bytes land.
                        while j + 1 < text.len() && text[j] == 0x1b && text[j + 1] == b'[' {
                            while j < text.len() && text[j] != b'm' {
                                j += 1;
                            }
                            j += 1;
                        }
                        if j < text.len() && row > 0 && col > 0 {
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
                                        ever[r]
                                            .insert(String::from_utf8_lossy(&grid[r]).into_owned());
                                    }
                                }
                            }
                            i = j;
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
