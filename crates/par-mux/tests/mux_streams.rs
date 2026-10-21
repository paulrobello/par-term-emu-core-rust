//! Raw pane streams over the control socket (H-CLI direct terminal
//! attach parity): `observe-pane` narrows a connection to one pane's
//! output for any number of read-only observers, and `control-pane` adds
//! input and a connection-scoped size report on the same connection.
//!
//! Unix-only: plain `std` Unix streams with read timeouts for the negative
//! assertions, as in `mux_hooks.rs`.

#![cfg(all(feature = "mux", unix))]

#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{pane_ids, spawn_daemon, wait_listening, MuxFixture};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// One connection: the writer half and a buffered reader.
struct Conn(UnixStream, BufReader<UnixStream>);

impl Conn {
    fn connect(path: &std::path::Path) -> Self {
        let stream = UnixStream::connect(path).expect("daemon accepts a client");
        stream
            .set_read_timeout(Some(common::CEILING))
            .expect("timeout installs");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        Conn(stream, reader)
    }

    fn send(&mut self, line: &str) {
        writeln!(self.0, "{line}").expect("write");
        self.0.flush().expect("flush");
    }

    /// Send `line` and collect everything through its `%end`/`%error`.
    fn command(&mut self, line: &str) -> Vec<String> {
        self.send(line);
        let mut out = Vec::new();
        loop {
            let mut buf = String::new();
            let n = self.1.read_line(&mut buf).expect("read reply");
            assert!(n > 0, "server closed while answering {line:?}: {out:?}");
            let done = buf.starts_with("%end") || buf.starts_with("%error");
            out.push(buf);
            if done {
                return out;
            }
        }
    }

    /// Lines until one satisfies `ready` (inclusive), within the hang ceiling.
    fn until(&mut self, ready: impl Fn(&str) -> bool, what: &str) -> Vec<String> {
        let deadline = Instant::now() + common::CEILING;
        let mut lines = Vec::new();
        loop {
            let mut buf = String::new();
            let n = self.1.read_line(&mut buf).expect("read line");
            assert!(n > 0, "server closed while waiting for {what}: {lines:?}");
            let hit = ready(&buf);
            lines.push(buf);
            if hit {
                return lines;
            }
            assert!(Instant::now() < deadline, "never saw {what}: {lines:?}");
        }
    }

    /// Everything readable within `window`, then whether the socket hit EOF.
    fn drain(&mut self, window: Duration) -> (Vec<String>, bool) {
        self.0
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("timeout installs");
        let deadline = Instant::now() + window;
        let mut lines = Vec::new();
        let mut eof = false;
        while Instant::now() < deadline {
            let mut buf = String::new();
            match self.1.read_line(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(_) => lines.push(buf),
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(err) => panic!("read error while draining: {err}"),
            }
        }
        if !eof {
            self.0
                .set_read_timeout(Some(common::CEILING))
                .expect("timeout restores");
        }
        (lines, eof)
    }
}

/// A daemon with one session whose window is split: panes A and B.
fn two_panes(tag: &str) -> (MuxFixture, common::DaemonGuard, Conn, String, String) {
    let fixture = MuxFixture::new(tag);
    let daemon = spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut ctl = Conn::connect(fixture.socket());
    ctl.command("new-session -s streams -x 80 -y 24");
    ctl.command("split-window -t %0 -h");
    let panes = pane_ids(&ctl.command("list-panes").join(""));
    assert_eq!(panes.len(), 2, "two panes: {panes:?}");
    let (a, b) = (panes[0].clone(), panes[1].clone());
    (fixture, daemon, ctl, a, b)
}

/// The `%output` payload text of `lines` for `pane`, octal escapes left in.
fn output_of(lines: &[String], pane: &str) -> String {
    let prefix = format!("%output {pane} ");
    lines
        .iter()
        .filter_map(|l| l.strip_prefix(&prefix))
        .collect()
}

#[test]
fn several_observers_each_get_only_their_panes_output() {
    let (fixture, _daemon, mut ctl, a, b) = two_panes("obs-multi");

    let mut first = Conn::connect(fixture.socket());
    let mut second = Conn::connect(fixture.socket());
    for observer in [&mut first, &mut second] {
        let reply = observer.command(&format!("observe-pane -t {a}"));
        assert!(
            reply.last().is_some_and(|l| l.starts_with("%end")),
            "observe-pane accepted: {reply:?}"
        );
        let header = reply
            .iter()
            .find(|l| l.starts_with("pane "))
            .expect("the reply body opens with the pane header");
        assert!(
            header.starts_with(&format!("pane {a} ")) && header.contains('x'),
            "header names the pane and its grid: {header:?}"
        );
    }

    ctl.command(&format!("send-keys -t {b} -l 'echo BMARK''B'"));
    ctl.command(&format!("send-keys -t {b} Enter"));
    ctl.command(&format!("send-keys -t {a} -l 'echo AMARK''A'"));
    ctl.command(&format!("send-keys -t {a} Enter"));

    for observer in [&mut first, &mut second] {
        let lines = observer.until(|l| l.contains("AMARKA"), "pane A's marker on its observer");
        assert!(
            !lines.iter().any(|l| l.contains("BMARKB")),
            "pane B's output leaked into an observer of A: {lines:?}"
        );
        assert!(
            lines.iter().all(|l| l.starts_with("%output ")),
            "an observer only carries its pane's output: {lines:?}"
        );
        assert!(output_of(&lines, &a).contains("AMARKA"));
    }
    // The control client is not narrowed: it saw both panes.
    let (rest, _) = ctl.drain(Duration::from_millis(600));
    let _ = rest;
}

#[test]
fn an_observer_is_read_only_and_its_stream_ends_with_the_pane() {
    let (fixture, _daemon, mut ctl, a, _b) = two_panes("obs-ro");
    let mut observer = Conn::connect(fixture.socket());
    observer.command(&format!("observe-pane -t {a}"));

    let refused = observer.command(&format!("send-keys -t {a} -l nope"));
    assert!(
        refused.last().is_some_and(|l| l.starts_with("%error")),
        "a later command on an observer is refused: {refused:?}"
    );
    assert!(
        refused.iter().any(|l| l.contains("read-only pane stream")),
        "with the read-only reason: {refused:?}"
    );

    ctl.command(&format!("kill-pane -t {a}"));
    let (lines, eof) = observer.drain(Duration::from_secs(5));
    assert!(
        lines.iter().any(|l| l == "%exit\n"),
        "the stream ends with %exit when its pane closes: {lines:?}"
    );
    assert!(eof, "and the daemon closes the connection: {lines:?}");
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("%layout-change") || l.starts_with("%window-")),
        "other broadcasts never reach a stream: {lines:?}"
    );
}

#[test]
fn an_unknown_pane_is_refused_and_the_connection_stays_a_control_client() {
    let (fixture, _daemon, _ctl, _a, _b) = two_panes("obs-missing");
    let mut conn = Conn::connect(fixture.socket());
    let reply = conn.command("observe-pane -t %99");
    assert!(
        reply.last().is_some_and(|l| l.starts_with("%error")),
        "unknown pane refused: {reply:?}"
    );
    let listed = conn.command("list-panes");
    assert!(
        listed.last().is_some_and(|l| l.starts_with("%end")),
        "a refused stream leaves the connection usable: {listed:?}"
    );
}

/// The CLI pair end to end: `par-mux control` forwards its stdin to the
/// pane until EOF, and a `par-mux observe` started first sees the result
/// on its stdout as raw bytes (no `%` framing).
#[test]
fn the_observe_and_control_subcommands_stream_raw_bytes() {
    use std::process::{Command, Stdio};
    let (fixture, _daemon, mut ctl, a, _b) = two_panes("cli");
    let bin = env!("CARGO_BIN_EXE_par-mux");
    let socket = fixture.socket().to_str().expect("utf-8 socket path");

    let mut observer = Command::new(bin)
        .args(["observe", "-t", &a, "--no-seed", "--socket", socket])
        .env_remove("PAR_MUX_SOCKET")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("observe spawns");
    // The observer's stream must be registered before the input lands.
    std::thread::sleep(Duration::from_millis(500));

    let mut controller = Command::new(bin)
        .args(["control", "-t", &a, "--size", "80x24", "--socket", socket])
        .env_remove("PAR_MUX_SOCKET")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("control spawns");
    controller
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"echo CLI'M'ARK\r")
        .expect("stdin accepts input");
    let status = controller.wait().expect("control exits at stdin EOF");
    assert!(status.success(), "control exit: {status:?}");

    let mut stdout = BufReader::new(observer.stdout.take().expect("piped stdout"));
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let mut chunk = Vec::new();
        if stdout.read_until(b'\n', &mut chunk).unwrap_or(0) == 0 {
            break;
        }
        seen.extend_from_slice(&chunk);
        if seen.windows(7).any(|w| w == b"CLIMARK") {
            break;
        }
    }
    let _ = observer.kill();
    let _ = observer.wait();
    let text = String::from_utf8_lossy(&seen);
    assert!(text.contains("CLIMARK"), "observe stdout: {text:?}");
    assert!(
        !text.contains("%output"),
        "observe writes raw bytes, not control-mode lines: {text:?}"
    );

    // A refused stream exits 1; a stream ended by %exit (its pane killed)
    // exits 0.
    let refused = Command::new(bin)
        .args(["observe", "-t", "%99", "--socket", socket])
        .env_remove("PAR_MUX_SOCKET")
        .output()
        .expect("observe runs");
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let mut ended = Command::new(bin)
        .args(["observe", "-t", &a, "--socket", socket])
        .env_remove("PAR_MUX_SOCKET")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("observe spawns");
    std::thread::sleep(Duration::from_millis(500));
    ctl.command(&format!("kill-pane -t {a}"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = ended.try_wait().expect("observe status") {
            break status;
        }
        assert!(Instant::now() < deadline, "observe outlived its pane");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0), "{status:?}");
}

#[test]
fn a_controller_writes_input_and_its_size_report_is_withdrawn_on_disconnect() {
    let (fixture, _daemon, mut ctl, a, _b) = two_panes("ctl");
    let size_of = |ctl: &mut Conn, pane: &str| -> String {
        let info = ctl.command(&format!("pane-info -t {pane}"));
        info.iter()
            .find(|l| l.starts_with(pane))
            .and_then(|l| l.split_whitespace().nth(2))
            .expect("pane-info reports a grid")
            .to_string()
    };
    // A standing viewer at the session's size: when the controller's
    // smaller contribution is withdrawn, the window re-fits back to it.
    ctl.command(&format!("refresh-client -t {a} -C 80x24"));
    let before = size_of(&mut ctl, &a);

    let mut controller = Conn::connect(fixture.socket());
    let reply = controller.command(&format!("control-pane -t {a}"));
    assert!(reply.last().is_some_and(|l| l.starts_with("%end")));

    // Input through the same connection, the bytes `par-mux control`
    // forwards: "echo CTLMARK" + CR as hex.
    let hex: Vec<String> = b"echo CTL'M'ARK\r"
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let sent = controller.command(&format!("send-keys -t {a} -H {}", hex.join(" ")));
    assert!(sent.last().is_some_and(|l| l.starts_with("%end")));
    let lines = controller.until(|l| l.contains("CTLMARK"), "the controller's echo");
    assert!(lines.iter().all(|l| l.starts_with("%output ")));

    // A smaller size report shrinks the pane's window while connected.
    let resized = controller.command(&format!("refresh-client -t {a} -C 50x12"));
    assert!(resized.last().is_some_and(|l| l.starts_with("%end")));
    assert_ne!(size_of(&mut ctl, &a), before, "the report re-fit the pane");

    // A second stream on the same connection is refused.
    let again = controller.command(&format!("control-pane -t {a}"));
    assert!(again.last().is_some_and(|l| l.starts_with("%error")));

    drop(controller);
    let deadline = Instant::now() + common::CEILING;
    while size_of(&mut ctl, &a) != before {
        assert!(
            Instant::now() < deadline,
            "the controller's size contribution outlived its connection"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Hang detection survives the load-independent waits: a WEDGED daemon (one
/// SIGSTOPped after it answered once) still fails a positive wait at its
/// ceiling, naming what it waited for. The tests pass `test_wait::CEILING`
/// (90 s); this one passes a short ceiling so the demonstration is quick,
/// through the same `until` helper and the same std socket read the suite's
/// `Conn` uses.
#[test]
fn a_wedged_daemon_still_fails_a_positive_wait_at_its_ceiling() {
    let (_fixture, daemon, mut ctl, a, _b) = two_panes("wedge");
    // A healthy daemon answers the very query the wait will poll.
    assert!(ctl
        .command(&format!("pane-info -t {a}"))
        .last()
        .is_some_and(|l| l.starts_with("%end")));
    ctl.0
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("timeout installs");

    let pid = daemon.id() as libc::pid_t;
    // SAFETY: signalling the daemon child this test spawned and owns.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0, "SIGSTOP");
    let ceiling = Duration::from_secs(2);
    let started = Instant::now();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctl.send(&format!("pane-info -t {a}"));
        common::test_wait::until(
            "the wedged daemon's pane-info reply",
            ceiling,
            || {
                let mut line = String::new();
                match ctl.1.read_line(&mut line) {
                    Ok(n) if n > 0 && line.starts_with("%end") => Some(()),
                    _ => None,
                }
            },
            || "no reply".to_string(),
        )
    }));
    let elapsed = started.elapsed();
    // SAFETY: as above; the guard's SIGKILL ends it either way.
    unsafe { libc::kill(pid, libc::SIGCONT) };
    let message = caught
        .expect_err("a wedged daemon fails the wait")
        .downcast::<String>()
        .map(|s| *s)
        .unwrap_or_default();
    assert!(
        message.contains("the wedged daemon's pane-info reply"),
        "the failure names what it waited for: {message}"
    );
    assert!(
        elapsed >= ceiling && elapsed < ceiling + Duration::from_secs(10),
        "the wait failed at its ceiling, not before and not long after: {elapsed:?}"
    );
}

/// The seed export runs OFF the clients lock (a deep-scrollback pane
/// would otherwise pause every pane's `%output` fan-out for the whole
/// export), with the output epoch closing the ordering race: every
/// `%output` that reaches an observer BEFORE the seed must already be
/// reflected in it. A hammering connection whose send-keys the tty
/// echoes continuously makes the race window all but certain, so a fast
/// path that enqueued a stale seed ahead of racing output fails here.
#[test]
fn seed_export_racing_output_stays_ordered() {
    let fixture = MuxFixture::new("obs-race");
    let _daemon = spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let mut ctl = Conn::connect(fixture.socket());
    ctl.command("new-session -s race -x 200 -y 24");
    // Deep scrollback: 8000 wide lines is a multi-megabyte restore export.
    ctl.command(
        "send-keys -t %0 -l 'for i in $(seq 1 20000); do printf \"SEED-%06d\\n\" $i; done'",
    );
    ctl.command("send-keys -t %0 Enter");
    let deadline = Instant::now() + common::CEILING;
    loop {
        let screen = ctl.command("capture-pane -t %0 -S -5").join("");
        if screen.contains("SEED-020000") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the scrollback fill never landed: {screen:?}"
        );
    }

    // The racing writer: a hammering connection whose send-keys the tty
    // echoes continuously, so %output is in flight the whole time the
    // seed export runs.
    let hammer_path = fixture.socket().to_path_buf();
    let hammer = std::thread::spawn(move || {
        let mut hammer = Conn::connect(&hammer_path);
        for i in 0..300 {
            // Default mode joins the payload tokens with nothing and the
            // trailing Enter KEY executes the line, so the tty keeps
            // echoing — a canonical-mode line stops echoing past its
            // ~1 KiB buffer, which would silence the hammer mid-run.
            hammer.command(&format!("send-keys -t %0 RACEH{i:03} MARKER Enter"));
        }
    });
    // Let the hammer reach full flow before opening the stream.
    std::thread::sleep(Duration::from_millis(50));
    let mut observer = Conn::connect(fixture.socket());
    observer.send("observe-pane -t %0");

    // The seed block: everything through its %end. %output lines that
    // slipped in ahead of it land between the stream marker and %begin.
    let reply = observer.until(|l| l.starts_with("%end"), "the seed block");
    let begin = reply
        .iter()
        .position(|l| l.starts_with("%begin"))
        .expect("the seed block opens with %begin");
    let seed: String = reply[begin..].concat();
    let pre_seed: String = reply[..begin].concat();
    let raced_markers: Vec<&str> = pre_seed
        .split_whitespace()
        .filter(|w| w.starts_with("RACEH"))
        .collect();
    // Positive control: the hammer ran before the stream opened, so the
    // export raced real output and its seed reflects it.
    assert!(
        seed.contains("RACEH"),
        "the seed does not reflect the hammering output at all"
    );
    // A marker that DID slip in ahead of the seed (only possible when
    // its output was produced during the export window) must be in the
    // seed — with the epoch check the slow path re-exports, so this list
    // stays empty-or-reflected; without it, the fast path loses racing
    // output and fails here.
    for marker in &raced_markers {
        assert!(
            seed.contains(marker),
            "{marker} reached the observer before the seed but the seed \
             does not reflect it — the fast path lost racing output"
        );
    }

    // The stream stays live: the hammer's last marker streams through
    // (the hammer joins only after its final reply).
    hammer.join().expect("hammer finished");
    let tail = observer.until(|l| l.contains("RACEH299"), "the last racing marker");
    assert!(
        tail.iter().any(|l| l.starts_with("%output")),
        "the marker arrives as pane output: {tail:?}"
    );
}
