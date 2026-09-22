//! The Phase 5 agent-layer arc (Task 5.5), end to end over a live daemon:
//! a pane exists, herdr's own integration script (env-renamed) drives it
//! working then blocked, a second client observes both
//! `%agent-state-changed` broadcasts, `list-agents` agrees — and a restart
//! keeps the IDENTITY but none of the state: task 6.1 persists the
//! hook-reported agent session (id/path/argv) while state, its provenance,
//! seq, message, and start source stay deliberately out of the save
//! format. This file exists to keep that boundary tested, not assumed.
//!
//! Unix-only: the daemon is stopped with SIGTERM and the ported script is
//! POSIX sh + python3.

#![cfg(all(feature = "mux", unix))]

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

fn socket(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-agents-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    path
}

fn spawn_daemon(path: &std::path::Path) -> std::process::Child {
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

/// SIGTERM, then require the clean exit the handler guarantees.
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

/// A control client: writer + buffered reader halves of one socket.
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

    /// Run one command and drain its `%begin`…`%end` block.
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

    /// The next line satisfying `ready`, skipping everything else.
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

    /// The reply block's body lines, with protocol framing and interleaved
    /// `%output` pushes stripped.
    fn body_lines(&mut self, line: &str) -> Vec<String> {
        self.command(line)
            .join("")
            .lines()
            .filter(|l| {
                !l.starts_with("%output")
                    && !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Run the env-renamed herdr kimi script for one action, optionally
/// feeding it a session id the way the real host would (stdin payload).
fn script_reports(path: &std::path::Path, pane: &str, action: &str, session_id: Option<&str>) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/par-mux-agent-state.sh");
    let payload = session_id
        .map(|id| format!("{{\"session_id\":\"{id}\"}}"))
        .unwrap_or_default();
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg(action)
        .env("PAR_MUX_ENV", "1")
        .env("PAR_MUX_SOCKET", path)
        .env("PAR_MUX_PANE_ID", pane)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child
                .stdin
                .as_mut()
                .expect("piped stdin")
                .write_all(payload.as_bytes())?;
            child.wait_with_output()
        })
        .expect("script runs");
    assert!(out.status.success(), "the ported script exits 0: {out:?}");
}

/// Whether any object in `value` carries one of the state-shaped keys the
/// save format still deliberately omits: task 6.1 persists agent-session
/// IDENTITY only — state, its provenance, the ordering seq, the blocked
/// reason, and the start source all stay out (a restored pane reports them
/// anew or holds none). Proven on the actual bytes written.
fn any_state_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.keys().any(|key| {
                key == "agent_state"
                    || key == "agent_state_source"
                    || key == "agent_seq"
                    || key == "agent_message"
                    || key == "agent_session_start_source"
            }) || map.values().any(any_state_key)
        }
        serde_json::Value::Array(items) => items.iter().any(any_state_key),
        _ => false,
    }
}

#[test]
fn the_agent_arc_lives_and_dies_with_the_daemon() {
    let path = socket("arc");
    let state_path = par_term_emu_core_rust::mux::persist::state_file_path(&path);
    let _ = std::fs::remove_file(&state_path);

    // First daemon: a pane, driven by the ported herdr script.
    let mut first = spawn_daemon(&path);
    wait_listening(&path);
    let mut control = Control::connect(&path);
    control.command("new-session -s agents");
    let pane = control
        .body_lines("list-panes")
        .first()
        .expect("new-session created a pane")
        .clone();

    for (action, session_id) in [("working", Some("kimi-arc-1")), ("blocked", None)] {
        script_reports(&path, &pane, action, session_id);
        let broadcast = control.line_until(
            |line| line.starts_with("%agent-state-changed"),
            &format!("the {action} broadcast"),
        );
        assert_eq!(
            broadcast,
            format!("%agent-state-changed {pane} kimi {action} source=hook\n"),
            "the second client observed the lifecycle, provenance included"
        );
    }

    let roster = control.body_lines("list-agents");
    assert_eq!(
        roster,
        vec![format!("{pane} kimi blocked hook")],
        "list-agents agrees with the last report, provenance included"
    );

    drop(control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut first);

    // The deferral after task 6.1: the save carries the pane's agent-session
    // IDENTITY — the resume path's data — but still nothing state-shaped.
    // Asserted on the actual bytes on disk, not the struct definition.
    let saved = std::fs::read_to_string(&state_path).expect("the clean stop saved state");
    let parsed: serde_json::Value = serde_json::from_str(&saved).expect("the save is JSON");
    assert!(
        saved.contains(r#""agent_session""#) && saved.contains(r#""kimi-arc-1""#),
        "the reported session identity travels in the save (task 6.1): {saved}"
    );
    assert!(
        !any_state_key(&parsed),
        "state, provenance, seq, message and start source must not travel: {saved}"
    );

    // The second daemon serves the layout back but an EMPTY roster.
    let mut second = spawn_daemon(&path);
    wait_listening(&path);
    let mut control = Control::connect(&path);
    let panes = control.body_lines("list-panes");
    assert!(
        panes.contains(&pane),
        "the pane itself survived the restart: {panes:?}"
    );
    let roster = control.body_lines("list-agents");
    assert!(
        roster.is_empty(),
        "agent state deliberately did not survive: {roster:?}"
    );

    drop(control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut second);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&state_path);
}
