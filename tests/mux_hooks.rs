//! The hook endpoint over the wire (Phase 5, Task 5.2): herdr-shaped JSON
//! reports on the control socket are answered in place with one JSON reply,
//! accepted reports broadcast `%agent-state-changed` to control clients,
//! stale ones do not — and the port proof: herdr's own kimi integration
//! script, env-renamed, drives a live daemon end to end.
//!
//! Unix-only: the daemon is stopped with SIGTERM and the hook script is
//! POSIX sh. Clients here are plain `std` Unix streams (not
//! [`par_term_emu_core_rust::mux::connect_local_stream`]) because the
//! negative assertions need `set_read_timeout`, which the interprocess
//! wrapper does not expose.

#![cfg(all(feature = "mux", unix))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn socket(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-hooks-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    path
}

fn spawn_daemon(path: &std::path::Path) -> std::process::Child {
    // Null stdio: a daemon that outlives a failed assertion must not hold
    // the test harness's output pipe open (the mux_restart.rs lesson).
    use std::process::Stdio;
    std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon binary spawns")
}

fn wait_listening(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while UnixStream::connect(path).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// SIGTERM, then require the clean exit the handler guarantees (Task 3.5).
/// The daemon never exits on its own — forgetting this is an infinite wait.
fn sigterm_clean(child: &mut std::process::Child) {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).expect("SIGTERM delivered");
    let status = child.wait().expect("daemon exits");
    assert!(
        status.success(),
        "a clean SIGTERM exits 0 after saving, got {status:?}"
    );
}

/// A control client: the writer half and a buffered reader half of one
/// socket, with a long read timeout so replies always land.
struct Control(UnixStream, BufReader<UnixStream>);

impl Control {
    fn connect(path: &std::path::Path) -> Self {
        let stream = UnixStream::connect(path).expect("daemon accepts control client");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout installs");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        Control(stream, reader)
    }

    /// Run one command and drain its `%begin`…`%end` block, collecting any
    /// interleaved `%output` pushes as body noise on the way past.
    fn command(&mut self, line: &str) -> Vec<String> {
        writeln!(self.0, "{line}").expect("write command");
        self.0.flush().expect("flush");
        let mut out = Vec::new();
        loop {
            let mut buf = String::new();
            let n = self.1.read_line(&mut buf).expect("read reply");
            assert!(n > 0, "server closed while answering {line:?}");
            let done = buf.starts_with("%end") || buf.starts_with("%error");
            out.push(buf);
            if done {
                return out;
            }
        }
    }

    /// The next line satisfying `ready`, skipping `%output` noise.
    fn line_until(&mut self, ready: impl Fn(&str) -> bool, what: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut buf = String::new();
            let n = self.1.read_line(&mut buf).expect("read line");
            assert!(n > 0, "server closed while waiting for {what}");
            if ready(buf.trim_end()) {
                return buf;
            }
            assert!(Instant::now() < deadline, "never saw {what}: {buf}");
        }
    }

    /// Everything readable within `window` (short per-read timeouts), for
    /// negative assertions: what did NOT arrive.
    fn drain_within(&mut self, window: Duration) -> Vec<String> {
        self.0
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("timeout installs");
        let deadline = Instant::now() + window;
        let mut lines = Vec::new();
        while Instant::now() < deadline {
            let mut buf = String::new();
            match self.1.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => lines.push(buf),
                Err(err) if timed_out(&err) => break,
                Err(err) => panic!("read error while draining: {err}"),
            }
        }
        self.0
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout restores");
        lines
    }
}

fn timed_out(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Every pane-id-shaped line (`%` + digit) in a reply block.
fn pane_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            l.strip_prefix('%')
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
        .collect()
}

/// One daemon plus a registered control client and the id of its first
/// pane — the stage every hook test plays on.
struct Stage {
    daemon: std::process::Child,
    path: std::path::PathBuf,
    state_path: std::path::PathBuf,
    control: Control,
    pane: String,
}

fn stage(tag: &str) -> Stage {
    let path = socket(tag);
    let state_path = par_term_emu_core_rust::mux::persist::state_file_path(&path);
    let _ = std::fs::remove_file(&state_path);
    let daemon = spawn_daemon(&path);
    wait_listening(&path);
    let mut control = Control::connect(&path);
    control.command("new-session -s hooks");
    let pane = pane_ids(&control.command("list-panes").join(""))
        .first()
        .expect("new-session created a pane")
        .clone();
    Stage {
        daemon,
        path,
        state_path,
        control,
        pane,
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Best-effort teardown; a panic mid-test still leaves the daemon to
        // be reaped by the explicit calls in each test's happy path.
        let _ = self.control.0.shutdown(Shutdown::Both);
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        let _ = signal::kill(Pid::from_raw(self.daemon.id() as i32), Signal::SIGTERM);
        let _ = self.daemon.wait();
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.state_path);
    }
}

/// Send one hook report line on a fresh connection and read the one reply.
fn hook_round_trip(path: &std::path::Path, report: &str) -> String {
    let stream = UnixStream::connect(path).expect("daemon accepts hook connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout installs");
    let mut writer = stream.try_clone().expect("clone");
    writeln!(writer, "{report}").expect("send report");
    writer.flush().expect("flush");
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    let n = reader.read_line(&mut reply).expect("read reply");
    assert!(n > 0, "server closed before replying");
    reply
}

/// The kimi script's report shape, for hand-driven reports: seq large and
/// spaced so ordering is explicit.
fn report(pane: &str, state: &str, seq: u64) -> String {
    format!(
        r#"{{"id":"probe-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","state":"{state}","seq":{seq},"source":"par-mux:test"}}}}"#
    )
}

#[test]
fn hook_report_replies_in_place_and_broadcasts_to_control_clients() {
    let mut stage = stage("report");
    let reply = hook_round_trip(&stage.path, &report(&stage.pane, "working", 1_000));
    assert!(
        reply.contains(r#""id":"probe-1000""#) && reply.contains(r#""result":"ok""#),
        "one-shot JSON reply echoes the id: {reply}"
    );

    let broadcast = stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the state broadcast",
    );
    assert_eq!(
        broadcast,
        format!(
            "%agent-state-changed {} kimi working source=hook\n",
            stage.pane
        ),
        "the broadcast carries pane, agent, state and provenance"
    );

    stage.control.command("list-sessions"); // daemon still serves commands
    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

/// T5.4: the roster query agrees with what hooks reported — and a hookless
/// pane never appears in it.
#[test]
fn list_agents_agrees_with_hook_reports_and_omits_hookless_panes() {
    let mut stage = stage("roster");

    // A second, hookless pane alongside the claimed one.
    stage
        .control
        .command(&format!("split-window -t {} -h", stage.pane));
    let claimed = stage.pane.clone();
    let hookless = "%1".to_string();

    // The reply block's body lines, with protocol framing and interleaved
    // %output pushes stripped.
    fn body_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| {
                !l.starts_with("%output")
                    && !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    }

    // No reports yet: the roster body is empty.
    let empty = stage.control.command("list-agents").join("");
    assert!(
        body_lines(&empty).is_empty(),
        "before any report the roster carries no lines: {:?}",
        body_lines(&empty)
    );

    // Claim the first pane through the hook path, exactly as a ported
    // script would.
    let reply = hook_round_trip(&stage.path, &report(&claimed, "blocked", 2_000));
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the claim's broadcast",
    );

    let roster = stage.control.command("list-agents").join("");
    let lines = body_lines(&roster);
    assert!(
        lines.contains(&format!("{claimed} kimi blocked hook").as_str()),
        "the roster line matches the hook's claim, provenance included: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with(&hookless)),
        "the hookless pane is absent: {lines:?}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn stale_report_over_the_wire_writes_and_broadcasts_nothing() {
    let mut stage = stage("stale");

    let reply = hook_round_trip(&stage.path, &report(&stage.pane, "working", 2_000));
    assert!(
        reply.contains(r#""result":"ok""#),
        "in-order accepted: {reply}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the in-order broadcast",
    );

    // Older, then equal sequence: ok replies, but nothing must broadcast.
    for stale in [1_999_u64, 2_000] {
        let reply = hook_round_trip(&stage.path, &report(&stage.pane, "blocked", stale));
        assert!(
            reply.contains(r#""result":"ok""#),
            "dropped is not an error: {reply}"
        );
    }
    let drained = stage.control.drain_within(Duration::from_millis(700));
    let broadcasts = drained
        .iter()
        .filter(|line| line.starts_with("%agent-state-changed"))
        .count();
    assert_eq!(
        broadcasts, 0,
        "stale reports must not broadcast; drained: {drained:?}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn a_hook_connection_receives_no_broadcast_pushes() {
    let mut stage = stage("quiet");

    // Connect the hook FIRST, then trigger pushes from the control side:
    // a connection registered at accept time would already hold
    // `%layout-change`/`%output` lines by the time the split returns.
    let mut hook = UnixStream::connect(&stage.path).expect("daemon accepts hook connection");
    hook.set_read_timeout(Some(Duration::from_millis(400)))
        .expect("timeout installs");
    stage
        .control
        .command(&format!("split-window -t {} -h", stage.pane));

    let mut buf = [0u8; 1024];
    match hook.read(&mut buf) {
        Ok(n) => panic!(
            "the hook connection received a push before its reply: {:?}",
            String::from_utf8_lossy(&buf[..n])
        ),
        Err(err) if timed_out(&err) => {}
        Err(err) => panic!("unexpected read error on the hook connection: {err}"),
    }

    // Now report: the FIRST bytes this connection ever sees are its reply.
    let mut writer = hook.try_clone().expect("clone");
    writeln!(writer, "{}", report(&stage.pane, "idle", 3_000)).expect("send report");
    writer.flush().expect("flush");
    let mut reader = BufReader::new(hook);
    let mut reply = String::new();
    let n = reader.read_line(&mut reply).expect("read reply");
    assert!(n > 0, "server closed before replying");
    assert!(
        reply.contains(r#""result":"ok""#) && !reply.starts_with('%'),
        "the reply is JSON, not a push: {reply}"
    );
    drop((writer, reader));

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn herdr_kimi_script_env_renamed_drives_a_live_daemon() {
    let mut stage = stage("port-proof");
    let pane = stage.pane.clone();

    // The daemon seeds the env contract into every pane it spawns — that is
    // what lets a ported script run from INSIDE a pane.
    stage.control.command(&format!(
        "send-keys -t {pane} 'echo PM=$PAR_MUX_ENV/$PAR_MUX_PANE_ID' Enter"
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = stage
            .control
            .command(&format!("refresh-client -t {pane}"))
            .join("");
        if screen.contains(&format!("PM=1/{pane}")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never reported its env: {screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // herdr's own script, env-renamed, driving the daemon from outside.
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/par-mux-agent-state.sh");
    for action in ["working", "blocked"] {
        let out = std::process::Command::new("sh")
            .arg(&script)
            .arg(action)
            .env("PAR_MUX_ENV", "1")
            .env("PAR_MUX_SOCKET", &stage.path)
            .env("PAR_MUX_PANE_ID", &pane)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("script runs");
        assert!(out.status.success(), "the ported script exits 0: {out:?}");
        let broadcast = stage.control.line_until(
            |line| line.starts_with("%agent-state-changed"),
            &format!("the {action} broadcast"),
        );
        assert_eq!(
            broadcast,
            format!("%agent-state-changed {pane} kimi {action} source=hook\n"),
            "the ported script drove the pane's state"
        );
    }

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}
