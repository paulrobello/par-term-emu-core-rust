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

#[path = "mux_attach/client.rs"]
mod client;
mod common;
// Every test in this suite is unix-only.
#[cfg(unix)]
#[path = "mux_attach/render_agents.rs"]
mod render_agents;
#[path = "mux_attach/render_input.rs"]
mod render_input;
#[path = "mux_attach/render_layout.rs"]
mod render_layout;
// Every test in this suite is unix-only.
#[cfg(unix)]
#[path = "mux_attach/render_sessions.rs"]
mod render_sessions;

use common::{wait_listening, MuxFixture};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
#[cfg(unix)]
use unicode_width::UnicodeWidthChar as _;

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

/// The client config every spawn pins unless a test supplies its own: the
/// side panel starts hidden, the state the toggle/geometry tests were
/// written against (the product default shows it at launch).
const DEFAULT_CLIENT_CONFIG: &str = "[client]\nsidebar-on-launch = false\n";

fn spawn_attach_mode(
    fixture: &MuxFixture,
    extra: &[&str],
    mode: &str,
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach_with_config(fixture, extra, mode, DEFAULT_CLIENT_CONFIG)
}

/// [`spawn_attach_mode`] with the client's config file contents given:
/// `PAR_MUX_CONFIG` points the client at the fixture's client file, so it
/// never reads the developer's real config.
fn spawn_attach_with_config(
    fixture: &MuxFixture,
    extra: &[&str],
    mode: &str,
    client_config: &str,
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach_with_env(fixture, extra, mode, client_config, &[])
}

/// [`spawn_attach_with_config`] with extra environment for the attach
/// process — the host-identity variables (`TERM_PROGRAM`,
/// `KITTY_WINDOW_ID`, …) a test pins to model a specific host.
fn spawn_attach_with_env(
    fixture: &MuxFixture,
    extra: &[&str],
    mode: &str,
    client_config: &str,
    env: &[(&str, &str)],
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    spawn_attach_host(fixture, extra, mode, client_config, env, false)
}

/// The kitty graphics support query the render client's launch probe
/// writes (`a=q` for its image id 31), answered by a kitty-capable host.
const KITTY_QUERY_PREFIX: &[u8] = b"\x1b_Gi=31,";

/// [`spawn_attach_with_env`] whose fake host also models kitty graphics
/// support: with `kitty_host` it answers the probe's kitty query with
/// `OK` (then DA1, as a real kitty answers the query's DA1 sentinel);
/// without it the host answers DA1 alone, as a terminal without the
/// protocol does.
fn spawn_attach_host(
    fixture: &MuxFixture,
    extra: &[&str],
    mode: &str,
    client_config: &str,
    env: &[(&str, &str)],
    kitty_host: bool,
) -> (AttachHost, std::sync::Arc<std::sync::Mutex<String>>) {
    std::fs::write(fixture.client_config_path(), client_config).expect("write client config");
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
    cmd.env("PAR_MUX_CONFIG", fixture.client_config_path());
    for (key, value) in env {
        cmd.env(key, value);
    }
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
                    while let Some(pos) = pending
                        .windows(KITTY_QUERY_PREFIX.len())
                        .position(|w| w == KITTY_QUERY_PREFIX)
                    {
                        let reply: &[u8] = if kitty_host {
                            b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c"
                        } else {
                            b"\x1b[?62;22c"
                        };
                        let mut w = answer.0.lock().expect("writer lock");
                        let _ = w.write_all(reply);
                        let _ = w.flush();
                        pending.drain(..pos + KITTY_QUERY_PREFIX.len());
                    }
                    // Bound the carryover: only a trailing partial query
                    // prefix can legitimately wait here.
                    let keep = KITTY_QUERY_PREFIX.len() - 1;
                    if pending.len() > keep {
                        pending.drain(..pending.len() - keep);
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

/// Whether `needle` is in `bytes`: the raw bytes, the escape-stripped text,
/// or (Unix) the reconstructed 24x80 screen.
///
/// The render client diffs cell by cell and skips a cell whose glyph is
/// unchanged, so a space between two painted words can be a cursor jump on
/// the wire (`reload\x1b[9;29Hconfig`, captured at host load ~200): the
/// plain text then reads `reloadconfig`, and only the screen holds the
/// phrase. The screen is rebuilt only when both cheaper checks miss.
fn output_contains(bytes: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    if bytes.windows(needle.len()).any(|w| w == needle) {
        return true;
    }
    let text_needle = String::from_utf8_lossy(needle);
    if plain_text(bytes).contains(text_needle.as_ref()) {
        return true;
    }
    screen_contains(bytes, &text_needle)
}

#[cfg(unix)]
fn screen_contains(bytes: &[u8], text: &str) -> bool {
    // Only a skipped blank cell splits a phrase, so only a phrase with a
    // space needs the screen; escapes and drain sentinels (`\x00-never`)
    // never reach a cell. The walk covers the recent tail only: a 24x80
    // frame is a few KiB, and re-walking a passthrough flood's megabytes
    // per chunk made the flood tests quadratic (6 s -> 60 s).
    const TAIL: usize = 64 * 1024;
    let tail = &bytes[bytes.len().saturating_sub(TAIL)..];
    text.contains(' ')
        && !text.chars().any(char::is_control)
        && reconstructed_screen(tail, 24, 80)
            .0
            .iter()
            .any(|row| row.contains(text))
}

/// The screen walk ([`reconstructed_screen`]) is Unix-only, as are the
/// render-mode suites whose diffed frames need it.
#[cfg(not(unix))]
fn screen_contains(_bytes: &[u8], _text: &str) -> bool {
    false
}

/// Collect host output until `needle` appears, the child exits, or the
/// deadline passes. Returns everything read (joined).
///
/// Dual-use, so its deadline is the caller's: a drain (`b"\x00-never"`)
/// waits the whole window by design, and some callers wait for a needle
/// that legitimately never comes, then assert on what did. A positive
/// wait that must not fail early belongs in [`expect_output`].
fn wait_for_output(host: &AttachHost, needle: &[u8], deadline: Duration) -> Vec<u8> {
    let end = Instant::now() + deadline;
    let mut collected: Vec<u8> = Vec::new();
    let mut checked = usize::MAX;
    loop {
        if checked != collected.len() {
            checked = collected.len();
            if output_contains(&collected, needle) {
                return collected;
            }
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

/// [`wait_for_output`] as a barrier (QA-232): panics naming `what` when
/// `needle` never arrives, so a missed barrier fails at its own line rather
/// than as a confusing assertion further down.
///
/// The needle matches per [`output_contains`]: the raw bytes, the
/// escape-stripped text (render mode paints cell by cell, one CUP+SGR run
/// per cell), or the reconstructed screen.
///
/// A positive wait, so `deadline` is raised to the shared hang ceiling
/// ([`common::CEILING`]): the idle-calibrated budgets callers pass (5-15 s)
/// failed at host load 300-560 before the paint could land, and a healthy
/// run returns at the paint regardless.
fn expect_output(host: &AttachHost, needle: &[u8], deadline: Duration, what: &str) -> Vec<u8> {
    let deadline = deadline.max(common::CEILING);
    let text_needle = String::from_utf8_lossy(needle).into_owned();
    let end = Instant::now() + deadline;
    let mut collected: Vec<u8> = Vec::new();
    let mut checked = usize::MAX;
    loop {
        if checked != collected.len() {
            checked = collected.len();
            if output_contains(&collected, needle) {
                return collected;
            }
        }
        let left = end.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{what}: {text_needle:?} not seen within {deadline:?}; collected: {}",
            String::from_utf8_lossy(&collected)
        );
        match host
            .output_rx
            .recv_timeout(left.min(Duration::from_millis(50)))
        {
            Ok(bytes) => collected.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!(
                "{what}: the client exited before {text_needle:?}; collected: {}",
                String::from_utf8_lossy(&collected)
            ),
        }
    }
}

/// A pane's visible screen has the EXECUTED output of `echo <marker>`: the
/// typed command line carries the marker once, so the output is the second
/// occurrence.
fn echoed(marker: &str) -> impl Fn(&str) -> bool + '_ {
    move |screen: &str| screen.matches(marker).count() >= 2
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

/// Read host output into `all` until the reconstructed 24x80 screen
/// satisfies `ready`, within the shared hang ceiling; panics naming `what`
/// with the last screen. The paint barrier before injecting input at a
/// screen position: a fixed settle (the 600 ms sleeps this replaces)
/// races the paint under host load, and a click that lands before its
/// target is laid out is lost.
#[cfg(unix)]
fn expect_screen(
    host: &AttachHost,
    all: &mut Vec<u8>,
    ready: impl Fn(&[String]) -> bool,
    what: &str,
) {
    let end = Instant::now() + common::CEILING;
    loop {
        let screen = reconstructed_screen(all, 24, 80).0;
        if ready(&screen) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{what}: not on screen within {:?}; screen:\n{}",
            common::CEILING,
            screen.join("\n")
        );
        match host.output_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                all.extend_from_slice(&bytes);
                // Batch what is already queued before re-walking the
                // whole stream.
                for more in host.output_rx.try_iter() {
                    all.extend_from_slice(&more);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("{what}: the client exited; screen:\n{}", screen.join("\n"))
            }
        }
    }
}

/// `len` cells of `screen`'s 0-based `row` from 0-based column `col`.
/// [`reconstructed_screen`] keeps one byte per cell and a non-ASCII glyph
/// renders as U+FFFD, so a column is a char index, never a byte offset.
#[cfg(unix)]
fn cells(screen: &[String], row: usize, col: usize, len: usize) -> String {
    screen
        .get(row)
        .map(|r| r.chars().skip(col).take(len).collect())
        .unwrap_or_default()
}

/// The side panel's ` menu ` chip is laid out where the `\x1b[<0;17;12M`
/// press lands: host row 11 (0-based), cols 13..19.
#[cfg(unix)]
fn menu_chip_painted(screen: &[String]) -> bool {
    cells(screen, 11, 13, 6) == " menu "
}

/// A live daemon (the real binary — the production shape an attach client
/// meets) + one seeded session/pane, by the plain control client. The
/// DaemonGuard outlives the client and reaps the daemon even on panic.
/// Whether real htop exists (the frame-coalescing test's pane app; the
/// brew paths cover the dev machines, PATH the rest).
#[cfg(unix)]
fn htop_available() -> bool {
    std::path::Path::new("/opt/homebrew/bin/htop").exists()
        || std::path::Path::new("/usr/local/bin/htop").exists()
        || std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d.join("htop").exists()))
            .unwrap_or(false)
}

fn fixture_with_session(tag: &str) -> (MuxFixture, common::DaemonGuard, par_mux::mux::MuxClient) {
    let fixture = MuxFixture::new(tag);
    let daemon = common::spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut client = par_mux::mux::MuxClient::connect(fixture.socket()).expect("connect");
    common::send(&mut client, "new-session -s att").expect("new-session");
    (fixture, daemon, client)
}

/// Decode `pending` incrementally, invoking `place` for every complete
/// char (advancing `cur` by the char's width) and recovering from an
/// invalid byte by dropping it — a wedged pending buffer would otherwise
/// swallow the rest of the stream. An incomplete tail stays pending.
#[cfg(unix)]
fn place_pending(
    pending: &mut Vec<u8>,
    cur: &mut Option<(u16, u16)>,
    place: &mut dyn FnMut(char, (u16, u16)),
) {
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                let Some(ch) = text.chars().next() else {
                    return;
                };
                if let Some(pos) = *cur {
                    place(ch, pos);
                    *cur = Some((pos.0, pos.1.saturating_add(ch.width().unwrap_or(0) as u16)));
                }
                pending.drain(..ch.len_utf8());
                if pending.is_empty() {
                    return;
                }
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    let head = String::from_utf8_lossy(&pending[..valid]).into_owned();
                    pending.drain(..valid);
                    for ch in head.chars() {
                        if let Some(pos) = *cur {
                            place(ch, pos);
                            *cur =
                                Some((pos.0, pos.1.saturating_add(ch.width().unwrap_or(0) as u16)));
                        }
                    }
                    continue;
                }
                match err.error_len() {
                    Some(_) => {
                        pending.remove(0);
                        if pending.is_empty() {
                            return;
                        }
                    }
                    None => return, // incomplete tail: wait for more bytes
                }
            }
        }
    }
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
    // The live 1-based cursor a terminal keeps: a CUP sets it, every
    // glyph advances it by its width, and glyphs following a mid-run SGR
    // (a style change without a re-address — the run-length encoding's
    // normal shape) land where the cursor already is. The per-cell
    // spelling CUP'd every glyph; this walk reads both identically.
    let mut cur: Option<(u16, u16)> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != 0x1b {
            pending.push(b);
            place_pending(&mut pending, &mut cur, &mut |ch, pos| {
                let (r, c) = ((pos.0 - 1) as usize, (pos.1 - 1) as usize);
                if r < rows as usize && c < cols as usize {
                    let mut buf = [0u8; 4];
                    grid[r][c] = *ch.encode_utf8(&mut buf).as_bytes().first().unwrap_or(&b' ');
                    ever[r].insert(String::from_utf8_lossy(&grid[r]).into_owned());
                }
            });
            i += 1;
            continue;
        }
        pending.clear();
        if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // CSI: parameters and intermediates up to the final byte.
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() {
                let params = &bytes[i + 2..j];
                if bytes[j] == b'H' {
                    let text = String::from_utf8_lossy(params);
                    let mut parts = text.split(';');
                    let (row, col) = (
                        parts.next().and_then(|r| r.parse::<u16>().ok()),
                        parts.next().and_then(|c| c.parse::<u16>().ok()),
                    );
                    cur = match (row, col) {
                        (Some(r), Some(c)) => Some((r, c)),
                        _ => None,
                    };
                }
                // Any other CSI (SGR runs, DECSCUSR, DECTCEM) never moves
                // the cursor.
                i = j + 1;
                continue;
            }
            i = bytes.len();
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b']' {
            // OSC: swallow through BEL or ST (ESC \).
            let mut j = i + 2;
            while j < bytes.len() {
                if bytes[j] == 0x07 {
                    j += 1;
                    break;
                }
                if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                    j += 2;
                    break;
                }
                j += 1;
            }
            i = j;
            continue;
        }
        // A bare ESC (or an ESC-prefixed sequence the walk does not
        // model): step past it.
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
        .set_read_timeout(Some(common::CEILING))
        .expect("timeout installs");
    writeln!(stream, "{json}").expect("write report");
    stream.flush().expect("flush");
    let mut reply = String::new();
    BufReader::new(stream.try_clone().expect("clone"))
        .read_line(&mut reply)
        .expect("one reply line");
    reply
}

/// A fixture daemon holding two sessions, `att` and `second`, with their
/// shells started.
#[cfg(unix)]
fn fixture_with_two_sessions(
    tag: &str,
) -> (MuxFixture, common::DaemonGuard, par_mux::mux::MuxClient) {
    let (fixture, daemon, mut client) = fixture_with_session(tag);
    client
        .send("new-session -s second")
        .expect("second session");
    std::thread::sleep(Duration::from_millis(500));
    (fixture, daemon, client)
}

/// The refusal text for the daemon's current roster, spelled out here so
/// the test pins the bytes rather than re-deriving them from the code.
#[cfg(unix)]
fn expected_refusal(client: &mut par_mux::mux::MuxClient) -> String {
    let mut text = String::from("par-mux: attach failed: multiple sessions exist — choose one:\n");
    for line in client
        .send("list-sessions -F '#{session_id}: #{session_name}'")
        .expect("list-sessions")
    {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(
        "attach to one with `par-mux attach -t <session>` — with several servers \
         running, name the server too: `par-mux attach <server> -t <session>`\n",
    );
    text
}

/// `[client] session-picker = true` under a real terminal: the picker
/// lists both sessions; moving to `second` and pressing Enter attaches
/// that session (typed input reaches its pane).
#[cfg(unix)]
fn session_picker_pick_attaches(mode: &str, tag: &str, ready: &[u8]) {
    let (fixture, _daemon, mut client) = fixture_with_two_sessions(tag);
    let order: Vec<String> = client
        .send("list-sessions -F '#{session_name}'")
        .expect("list-sessions");
    let index = order
        .iter()
        .position(|name| name == "second")
        .expect("second listed");
    let (mut host, _stderr) = spawn_attach_with_config(
        &fixture,
        &[],
        mode,
        "[client]\nsidebar-on-launch = false\nsession-picker = true\n",
    );
    let shown = expect_output(
        &host,
        b"enter attach",
        Duration::from_secs(10),
        "the picker's footer",
    );
    let text = String::from_utf8_lossy(&shown);
    assert!(
        text.contains("choose one") && text.contains("att") && text.contains("second"),
        "the picker lists both sessions: {text}"
    );
    // Down past the first row (arrows and j both move), then back up
    // once with k, landing on `second`.
    for _ in 0..index {
        host.to_child.write_all(b"\x1b[B").expect("down");
        host.to_child.flush().ok();
        std::thread::sleep(Duration::from_millis(100));
    }
    host.to_child.write_all(b"j").expect("j");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(100));
    host.to_child.write_all(b"k").expect("k");
    host.to_child.flush().ok();
    std::thread::sleep(Duration::from_millis(100));
    host.to_child.write_all(b"\r").expect("enter");
    host.to_child.flush().ok();
    expect_output(
        &host,
        ready,
        Duration::from_secs(10),
        "the client attached after the pick",
    );
    let pane = session_first_pane(&mut client, "second");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        host.to_child
            .write_all(b"echo PICKED-MARKER\r")
            .expect("type");
        host.to_child.flush().ok();
        std::thread::sleep(Duration::from_millis(300));
        let capture = client
            .send(&format!("capture-pane -t {pane}"))
            .expect("capture");
        if capture.join("\n").contains("PICKED-MARKER") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "keys must reach the picked session's pane: {capture:?}"
        );
    }
    host.killer.kill().ok();
}

/// The first pane of `session` through its window — `list-panes -t`
/// takes a window or pane target, not a session id.
#[cfg(unix)]
fn session_first_pane(client: &mut par_mux::mux::MuxClient, session: &str) -> String {
    let window = common::send(client, &format!("list-windows -t {session}"))
        .expect("list-windows")
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .find(|t| t.starts_with('@'))
        .expect("a window")
        .to_string();
    common::send(client, &format!("list-panes -t {window}"))
        .expect("list-panes")
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .find(|t| t.starts_with('%'))
        .expect("a pane")
        .to_string()
}

/// The 1-based column of the CUP that positioned `needle`'s first
/// character. The frame flushes one CUP+SGR+char per cell, so text only
/// exists after escape stripping — walk the stream keeping the last CUP
/// column and attach it to every plain char, then locate the needle in
/// the stripped text. The geometry read for the sidebar-refit asserts.
#[cfg(unix)]
fn cup_col_of_text(bytes: &[u8], needle: &str) -> Option<u16> {
    // The live 1-based cursor: CUPs set it, glyphs advance it by their
    // width (the client's run-length encoding rides the auto-advance; the
    // per-cell spelling CUP'd every glyph, which the old inherited-column
    // walker assumed).
    let mut cur: Option<(u16, u16)> = None;
    let mut cells: Vec<(char, Option<u16>)> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b != 0x1b {
            pending.push(b);
            place_pending(&mut pending, &mut cur, &mut |ch, pos| {
                cells.push((ch, Some(pos.1)));
            });
            continue;
        }
        pending.clear();
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
                    let (row, col) = (
                        text.split(';').next().and_then(|r| r.parse::<u16>().ok()),
                        text.split(';').nth(1).and_then(|c| c.parse::<u16>().ok()),
                    );
                    cur = match (row, col) {
                        (Some(r), Some(c)) => Some((r, c)),
                        _ => None,
                    };
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
#[cfg(unix)]
fn cup_row_of_text(bytes: &[u8], needle: &str) -> Option<u16> {
    // Same live-cursor walk as [`cup_col_of_text`], recording each glyph's
    // row instead of its column.
    let mut cur: Option<(u16, u16)> = None;
    let mut cells: Vec<(char, Option<u16>)> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b != 0x1b {
            pending.push(b);
            place_pending(&mut pending, &mut cur, &mut |ch, pos| {
                cells.push((ch, Some(pos.0)));
            });
            continue;
        }
        pending.clear();
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
                    let (row, col) = (
                        text.split(';').next().and_then(|r| r.parse::<u16>().ok()),
                        text.split(';').nth(1).and_then(|c| c.parse::<u16>().ok()),
                    );
                    cur = match (row, col) {
                        (Some(r), Some(c)) => Some((r, c)),
                        _ => None,
                    };
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
/// Whether any `glyph` paints on host row 24 — the live-cursor walk
/// (CUPs move it, glyphs advance by width) that reads per-cell and
/// run-length encodings identically. The byte-window form
/// [`row24_paints`] assumed a CUP+SGR immediately before the glyph.
#[cfg(unix)]
fn row24_paints_char(bytes: &[u8], glyph: char) -> bool {
    let mut cur: Option<(u16, u16)> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != 0x1b {
            pending.push(b);
            let mut hit = false;
            place_pending(&mut pending, &mut cur, &mut |ch, pos| {
                if ch == glyph && pos.0 == 24 {
                    hit = true;
                }
            });
            if hit {
                return true;
            }
            i += 1;
            continue;
        }
        pending.clear();
        if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() {
                if bytes[j] == b'H' {
                    let text = String::from_utf8_lossy(&bytes[i + 2..j]);
                    let mut parts = text.split(';');
                    let (row, col) = (
                        parts.next().and_then(|r| r.parse::<u16>().ok()),
                        parts.next().and_then(|c| c.parse::<u16>().ok()),
                    );
                    cur = match (row, col) {
                        (Some(r), Some(c)) => Some((r, c)),
                        _ => None,
                    };
                }
                i = j + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    false
}

/// The furthest column painted on `row_want` (0 when the row never
/// paints) — a full-row repaint reaches the last column; a partial one
/// stops early.
#[cfg(unix)]
fn row24_furthest_col_on(bytes: &[u8], row_want: u16) -> usize {
    let mut furthest = 0usize;
    let mut cur: Option<(u16, u16)> = None;
    let mut pending: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != 0x1b {
            pending.push(b);
            place_pending(&mut pending, &mut cur, &mut |ch, pos| {
                if pos.0 == row_want {
                    furthest = furthest.max(pos.1 as usize + ch.width().unwrap_or(0));
                }
            });
            i += 1;
            continue;
        }
        pending.clear();
        if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() {
                if bytes[j] == b'H' {
                    let text = String::from_utf8_lossy(&bytes[i + 2..j]);
                    let mut parts = text.split(';');
                    let (row, col) = (
                        parts.next().and_then(|r| r.parse::<u16>().ok()),
                        parts.next().and_then(|c| c.parse::<u16>().ok()),
                    );
                    cur = match (row, col) {
                        (Some(r), Some(c)) => Some((r, c)),
                        _ => None,
                    };
                }
                i = j + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    furthest
}

/// Prints `marker` on a fresh pane line. The leading empty `echo` keeps the
/// output off any wrapped typed line, and the `""` split keeps the typed
/// literal from containing the marker: pane prompts vary by host (24 chars
/// on CI runners), and without both the CUP geometry reads report the wrap
/// continuation's column instead of the pane's content column.
#[cfg(unix)]
fn print_marker_line(client: &mut par_mux::mux::MuxClient, pane: &str, marker: &str) {
    let (head, tail) = marker.split_at(marker.len() / 2);
    common::send(
        client,
        &format!("send-keys -t {pane} -l 'echo; echo {head}\"\"{tail}'"),
    )
    .expect("marker");
    common::send(client, &format!("send-keys -t {pane} Enter")).expect("Enter");
}

/// A two-pane session whose right pane printed `marker`, for the
/// sidebar-on-launch geometry reads. Returns the left pane's id.
#[cfg(unix)]
fn split_with_right_marker(client: &mut par_mux::mux::MuxClient, marker: &str) -> String {
    let pane_a = common::send(client, "list-panes")
        .expect("list-panes")
        .join("")
        .split_whitespace()
        .next()
        .expect("a pane")
        .to_string();
    let pane_b = common::send(client, &format!("split-window -h -t {pane_a}"))
        .expect("split")
        .join("");
    let pane_b = pane_b.trim().to_string();
    print_marker_line(client, &pane_b, marker);
    wait_pane_echo(client, &pane_b, marker);
    pane_a
}

/// Poll `capture-pane` over `client` until `echo <marker>` executed on
/// `pane` (QA-232's capture barrier for helpers holding only the client).
#[cfg(unix)]
fn wait_pane_echo(client: &mut par_mux::mux::MuxClient, pane: &str, marker: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = common::send(client, &format!("capture-pane -t {pane}"))
            .expect("capture-pane")
            .join("\n");
        if screen.contains(marker) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{marker} never executed on {pane}: {screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The shown pane and a background window's pane of a fresh session —
/// the two-window shape the notify-delivery tests drive.
#[cfg(unix)]
fn shown_and_background_panes(client: &mut par_mux::mux::MuxClient) -> (String, String) {
    let first_pane = |client: &mut par_mux::mux::MuxClient, cmd: &str| {
        common::send(client, cmd)
            .expect(cmd)
            .join("")
            .split_whitespace()
            .next()
            .expect("a pane")
            .to_string()
    };
    let shown = first_pane(client, "list-panes");
    let background_window = common::send(client, "new-window -d")
        .expect("new-window")
        .join("")
        .trim()
        .to_string();
    let background = first_pane(client, &format!("list-panes -t {background_window}"));
    (shown, background)
}

/// Type `echo <marker>` + Enter into `pane`.
#[cfg(unix)]
fn echo_marker(client: &mut par_mux::mux::MuxClient, pane: &str, marker: &str) {
    client
        .send(&format!("send-keys -t {pane} -l 'echo {marker}'"))
        .expect("marker keys");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("marker Enter");
}

/// Split `session`'s first window into three panes, each echoing
/// `MULTI-<i>` — the multi-pane view the stale-view reports lost.
#[cfg(unix)]
fn seed_multi_pane(client: &mut par_mux::mux::MuxClient, session: &str) {
    let pane = session_first_pane(client, session);
    client
        .send(&format!("split-window -t {pane} -h"))
        .expect("split -h");
    client
        .send(&format!("split-window -t {pane} -v"))
        .expect("split -v");
    let window = client
        .send(&format!("list-windows -t {session}"))
        .expect("list-windows")[0]
        .split_whitespace()
        .next()
        .expect("a window")
        .to_string();
    let panes: Vec<String> = client
        .send(&format!("list-panes -t {window}"))
        .expect("list-panes")
        .iter()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    assert_eq!(panes.len(), 3, "three panes: {panes:?}");
    for (i, pane) in panes.iter().enumerate() {
        echo_marker(client, pane, &format!("MULTI-{i}"));
    }
}

/// Collect `host` output until the reconstructed final screen satisfies
/// `done` or the deadline passes; returns the final screen text.
#[cfg(unix)]
fn screen_until(
    host: &AttachHost,
    all: &mut Vec<u8>,
    deadline: Duration,
    done: impl Fn(&str) -> bool,
) -> String {
    let end = std::time::Instant::now() + deadline;
    loop {
        let (grid, _) = reconstructed_screen(all, 24, 80);
        let shown = grid.join("\n");
        if done(&shown) || std::time::Instant::now() >= end {
            return shown;
        }
        match host.output_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => all.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let (grid, _) = reconstructed_screen(all, 24, 80);
                return grid.join("\n");
            }
        }
    }
}

/// Poll `host` until some row of the left `width` columns (the side
/// panel) has contained `needle` in any painted snapshot, or the
/// deadline passes. Returns whether it was seen.
#[cfg(unix)]
fn panel_ever_contains(
    host: &AttachHost,
    all: &mut Vec<u8>,
    width: usize,
    needle: &str,
    deadline: Duration,
) -> bool {
    let end = Instant::now() + deadline;
    loop {
        let (_, ever) = reconstructed_screen(all, 24, 80);
        // Row 0 is the tab strip, not the panel.
        let seen = ever.iter().skip(1).any(|snapshots| {
            snapshots
                .iter()
                .any(|row| row.chars().take(width).collect::<String>().contains(needle))
        });
        if seen {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        match host.output_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => all.extend_from_slice(&bytes),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// Keep collecting `host` output until the reconstructed final grid has
/// not changed for `stable`, bounded by `deadline` (panics on it). Byte
/// silence is not the signal: the cursor heartbeat re-emits the cursor
/// placement every second, so the stream never goes quiet that long.
#[cfg(unix)]
fn screen_stable(host: &AttachHost, all: &mut Vec<u8>, stable: Duration, deadline: Duration) {
    // `deadline` only catches a screen that never settles: the shared
    // hang ceiling, not a latency budget.
    let deadline = deadline.max(common::CEILING);
    let end = Instant::now() + deadline;
    let mut last = reconstructed_screen(all, 24, 80).0;
    let mut since = Instant::now();
    loop {
        if since.elapsed() >= stable {
            return;
        }
        assert!(
            Instant::now() < end,
            "screen_stable quiesce: the screen kept changing for {deadline:?}"
        );
        match host.output_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                all.extend_from_slice(&bytes);
                let now = reconstructed_screen(all, 24, 80).0;
                if now != last {
                    last = now;
                    since = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("screen_stable quiesce: the client exited")
            }
        }
    }
}

#[cfg(unix)]
fn shows_multi(screen: &str) -> bool {
    (0..3).all(|i| screen.contains(&format!("MULTI-{i}")))
}

/// A workspace holding two sessions — `$1` (three panes) and the newer
/// `$2` (one pane, `SOLO-MARK`), which `new-session` leaves displayed.
#[cfg(unix)]
fn fixture_two_session_workspace(
    tag: &str,
) -> (MuxFixture, common::DaemonGuard, par_mux::mux::MuxClient) {
    let (fixture, daemon, mut client) = fixture_with_session(tag);
    client.send("new-workspace -n demo").expect("new-workspace");
    seed_multi_pane(&mut client, "$1");
    client
        .send("new-session -s solo -t demo")
        .expect("second session in demo");
    let solo = session_first_pane(&mut client, "$2");
    echo_marker(&mut client, &solo, "SOLO-MARK");
    common::wait_daemon(
        fixture.socket(),
        &format!("capture-pane -t {solo}"),
        echoed("SOLO-MARK"),
        Duration::from_secs(10),
        "SOLO-MARK executed on the pane screen",
    );
    (fixture, daemon, client)
}

/// The pane count of `window`, by the plain control client.
#[cfg(unix)]
fn pane_count(client: &mut par_mux::mux::MuxClient, window: &str) -> usize {
    client
        .send(&format!("list-panes -t {window}"))
        .map(|rows| rows.iter().filter(|l| l.starts_with('%')).count())
        .unwrap_or(0)
}

/// Wait until `client` sees a `%pane-exited` push or the deadline passes.
#[cfg(unix)]
fn saw_pane_exited(client: &par_mux::mux::MuxClient, deadline: Duration) -> bool {
    let end = Instant::now() + deadline;
    while Instant::now() < end {
        match client
            .notifications()
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(par_term_emu_core::tmux_control::TmuxNotification::PaneExited { .. }) => {
                return true
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

/// A deterministic rich-like payload: bursts of cursor-addressed (CUP),
/// truecolor-styled (SGR) chunks carrying multi-byte UTF-8 glyphs, the
/// shape `python -m rich` floods a terminal with. No `\n`, so the pane
/// tty's ONLCR cannot rewrite it on the way out: the bytes the pane
/// writes are exactly the bytes its PTY reader sees.
#[cfg(unix)]
fn rich_like_payload(chunks: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..chunks {
        let row = i % 20 + 1;
        let col = (i * 7) % 40 + 1;
        let (r, g, b) = ((i * 37) % 256, (i * 91) % 256, (i * 53) % 256);
        out.extend_from_slice(
            format!(
                "\x1b[{row};{col}H\x1b[1;38;2;{r};{g};{b}m\u{2501}\u{2501} Lorem ipsum dolor sit amet \
                 chunk-{i:06} \u{2022} consectetur adipiscing elit \x1b[3;48;5;{}m\u{1f600}\x1b[0m",
                i % 256
            )
            .as_bytes(),
        );
    }
    out
}

/// Strip the passthrough client's own status draws from a host capture:
/// each is one `ESC7 … ESC8 ESC[r;cH` run written between whole `%output`
/// chunks, so what remains is exactly the pane bytes the client forwarded.
#[cfg(unix)]
fn strip_status_draws(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"\x1b7\x1b[1;") {
            if let Some(close) = bytes[i..].windows(2).position(|w| w == b"\x1b8") {
                let mut j = i + close + 2;
                if bytes[j..].starts_with(b"\x1b[") {
                    while j < bytes.len() && bytes[j] != b'H' {
                        j += 1;
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// The bytes from the first `start` marker through the following `end`
/// marker (markers included), or `None` when either is missing.
#[cfg(unix)]
fn marked_region<'a>(bytes: &'a [u8], start: &[u8], end: &[u8]) -> Option<&'a [u8]> {
    let s = bytes.windows(start.len()).position(|w| w == start)?;
    let e = bytes[s..].windows(end.len()).position(|w| w == end)?;
    Some(&bytes[s..s + e + end.len()])
}

/// Every status draw's closing placement CUP (`ESC8 ESC[r;cH`), as the
/// 1-based row it parks the host cursor on.
#[cfg(unix)]
fn status_draw_placement_rows(bytes: &[u8]) -> Vec<u16> {
    let mut rows = Vec::new();
    let mut i = 0;
    while let Some(at) = bytes[i..].windows(4).position(|w| w == b"\x1b8\x1b[") {
        let start = i + at + 4;
        let digits: Vec<u8> = bytes[start..]
            .iter()
            .copied()
            .take_while(u8::is_ascii_digit)
            .collect();
        if let Ok(row) = std::str::from_utf8(&digits).unwrap_or("").parse::<u16>() {
            rows.push(row);
        }
        i = start;
    }
    rows
}

/// How the stand-in host terminal drains the attach client's output.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostShape {
    /// The suite's standard harness: 8 KiB reads as fast as they come.
    Fast,
    /// Small reads with a pause after each, so the client's tty output
    /// buffer fills and its host writes block.
    Slow,
}

/// Spawn the passthrough client under a PTY drained slowly: 512-byte
/// reads with a 1 ms pause after each.
#[cfg(unix)]
fn spawn_attach_slow_host(fixture: &MuxFixture, extra: &[&str]) -> AttachHost {
    let pair = native_pty_system()
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
    cmd.arg("--mode");
    cmd.arg("passthrough");
    cmd.args(extra);
    let child = pair.slave.spawn_command(cmd).expect("spawn shimmed attach");
    let killer = child.clone_killer();
    let writer = std::sync::Arc::new(std::sync::Mutex::new(
        pair.master.take_writer().expect("master writer"),
    ));
    let mut from_child = pair.master.try_clone_reader().expect("master reader");
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        loop {
            match from_child.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    });
    AttachHost {
        child,
        killer,
        to_child: SharedWriter(writer),
        output_rx: rx,
        master: pair.master,
    }
}

#[cfg(unix)]
fn passthrough_flood_round_trip(shape: HostShape) {
    let (fixture, _daemon, mut client) = fixture_with_session("ptflood");
    let pane = session_first_pane(&mut client, "$0");

    const START: &[u8] = b"<<PT-PAYLOAD-START>>";
    const END: &[u8] = b"<<PT-PAYLOAD-END>>";
    let mut payload = START.to_vec();
    payload.extend_from_slice(&rich_like_payload(3000));
    payload.extend_from_slice(END);
    let payload_path = fixture.state_dir().with_file_name("payload.bin");
    std::fs::write(&payload_path, &payload).expect("write payload");

    // The daemon-side recorder: a second control client whose `%output`
    // for the pane is the PTY reader's exact byte sequence. It registers
    // (joins the broadcast) on its first command.
    let mut observer = par_mux::mux::MuxClient::connect(fixture.socket()).expect("observer");
    observer.send("list-panes").expect("observer registers");

    let (mut host, stderr) = match shape {
        HostShape::Fast => spawn_attach(&fixture, &["-t", &pane]),
        HostShape::Slow => (
            spawn_attach_slow_host(&fixture, &["-t", &pane]),
            std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        ),
    };
    expect_output(
        &host,
        b"\x1b[1;23r",
        Duration::from_secs(10),
        "passthrough status-row reserve",
    );

    client
        .send(&format!(
            "send-keys -t {pane} -l 'seq 1 20000; cat {}'",
            payload_path.display()
        ))
        .expect("send-keys flood");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("Enter");

    let got = wait_for_output(&host, END, Duration::from_secs(60));
    // Let the tail (and any trailing status draw) land.
    // output settle: collect the trailing redraw burst before draining
    std::thread::sleep(Duration::from_millis(300));
    let mut host_bytes = got;
    while let Ok(bytes) = host.output_rx.try_recv() {
        host_bytes.extend_from_slice(&bytes);
    }

    let mut daemon_bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while marked_region(&daemon_bytes, START, END).is_none() && Instant::now() < deadline {
        if let Ok(par_term_emu_core::tmux_control::TmuxNotification::Output { pane_id, data }) =
            observer
                .notifications()
                .recv_timeout(Duration::from_millis(100))
        {
            if pane_id == pane {
                daemon_bytes.extend_from_slice(&data);
            }
        }
    }

    let daemon_region = marked_region(&daemon_bytes, START, END)
        .expect("the daemon side must carry the whole payload");
    assert_eq!(
        daemon_region,
        &payload[..],
        "the pane PTY reader must see the payload byte-exact (ONLCR-free)"
    );
    let host_clean = strip_status_draws(&host_bytes);
    let Some(host_region) = marked_region(&host_clean, START, END) else {
        panic!(
            "the host never received the whole payload: {} host bytes, start seen: {}, \
             end seen: {}\nstderr: {}",
            host_bytes.len(),
            host_clean.windows(START.len()).any(|w| w == START),
            host_clean.windows(END.len()).any(|w| w == END),
            stderr.lock().unwrap()
        );
    };
    if host_region != daemon_region {
        let first_diff = host_region
            .iter()
            .zip(daemon_region.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(host_region.len().min(daemon_region.len()));
        let window = |r: &[u8]| {
            String::from_utf8_lossy(
                &r[first_diff.saturating_sub(40)..(first_diff + 120).min(r.len())],
            )
            .into_owned()
        };
        panic!(
            "passthrough lost or altered pane bytes: daemon {} bytes, host {} bytes, \
             first divergence at {first_diff}\n daemon: {:?}\n host:   {:?}",
            daemon_region.len(),
            host_region.len(),
            window(daemon_region),
            window(host_region),
        );
    }
    host.killer.kill().ok();
}

/// Strip SGR (CSI …m) runs from a `capture-pane -e` row.
#[cfg(unix)]
fn strip_sgr(row: &str) -> String {
    let bytes = row.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            // Keep only SGR runs; any other CSI (none expected in a
            // capture row) is dropped too — the comparison is text-only.
            i = j + 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The scratch window a popup opened: the `list-windows -a` row named
/// `popup` (`$S @N <marker> popup`), as its `@N`.
#[cfg(unix)]
fn popup_window_id(reply: &str) -> Option<String> {
    reply
        .lines()
        .find(|l| l.trim_end().ends_with(" popup"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(str::to_string)
}

/// `ESC _ … ST` (APC) sequences removed, so [`reconstructed_screen`]
/// never reads a kitty upload's base64 body as glyphs.
#[cfg(unix)]
fn strip_apc(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'_') {
            match bytes[i + 2..].windows(2).position(|w| w == b"\x1b\\") {
                Some(end) => i += 2 + end + 2,
                None => break,
            }
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// A file the pane `cat`s: clear, park the cursor at pane cell (5, 3),
/// a 20x40 px RGBA kitty image (2x2 cells at the 10x20 cell size the
/// client reported) with no `c=`/`r=`/`C=1` — so it moves the cursor
/// below itself like a real `icat` — then a text marker.
#[cfg(unix)]
fn kitty_image_script(dir: &std::path::Path) -> std::path::PathBuf {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(vec![0x7fu8; 20 * 40 * 4]);
    let mut bytes = b"\x1b[2J\x1b[4;6H".to_vec();
    bytes.extend_from_slice(format!("\x1b_Ga=T,f=32,s=20,v=40;{b64}\x1b\\").as_bytes());
    bytes.extend_from_slice(b"KITTY-AFTER-MARKER");
    let path = dir.join("img.bin");
    std::fs::write(&path, bytes).expect("write image script");
    path
}

/// Run `cat <script>` in `pane` and collect the host output through
/// `needle`, with the boot and settle output before it.
#[cfg(unix)]
fn cat_into_pane(
    client: &mut par_mux::mux::MuxClient,
    host: &AttachHost,
    pane: &str,
    script: &std::path::Path,
    needle: &[u8],
    what: &str,
) -> Vec<u8> {
    let mut seen = expect_output(host, b"\x1b[?1003h", common::CEILING, "render client up");
    // The pane's seed frame is on the host (its content rows sit under
    // the tab strip) before the pane prints anything new.
    expect_screen(
        host,
        &mut seen,
        |s| s[1..].iter().any(|r| !r.trim().is_empty()),
        "the pane's seed frame",
    );
    client
        .send(&format!(
            "send-keys -t {pane} -l 'cat {}'",
            script.display()
        ))
        .expect("send-keys");
    client
        .send(&format!("send-keys -t {pane} Enter"))
        .expect("Enter");
    seen.extend(expect_output(host, needle, common::CEILING, what));
    seen
}

#[cfg(unix)]
const KITTY_CLIENT_CONFIG: &str =
    "[client]\nborder-lines = \"unicode\"\nsidebar-on-launch = false\n";
