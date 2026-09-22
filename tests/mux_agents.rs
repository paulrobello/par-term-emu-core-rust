//! The Phase 5 agent-layer arc (Task 5.5), end to end over a live daemon:
//! a pane exists, herdr's own integration script (env-renamed) drives it
//! working then blocked, a second client observes both
//! `%agent-state-changed` broadcasts, `list-agents` agrees — and a restart
//! keeps NONE of it, because metadata is deliberately absent from the save
//! format (seam S2's assigned persist gap, Phase 6's entry criterion).
//! This file exists to keep that deferral tested, not assumed.
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

/// Run the env-renamed herdr kimi script for one action.
fn script_reports(path: &std::path::Path, pane: &str, action: &str) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/par-mux-agent-state.sh");
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg(action)
        .env("PAR_MUX_ENV", "1")
        .env("PAR_MUX_SOCKET", path)
        .env("PAR_MUX_PANE_ID", pane)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("script runs");
    assert!(out.status.success(), "the ported script exits 0: {out:?}");
}

/// Whether any object in `value` carries a `metadata` key — the save
/// format's deliberate agent-state gap, proven on the actual bytes written.
fn any_metadata_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.keys().any(|key| key == "metadata") || map.values().any(any_metadata_key)
        }
        serde_json::Value::Array(items) => items.iter().any(any_metadata_key),
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

    for action in ["working", "blocked"] {
        script_reports(&path, &pane, action);
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
        vec![format!("{pane} kimi blocked")],
        "list-agents agrees with the last report"
    );

    drop(control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut first);

    // The deferral, tested: the save format carries NO metadata — seam S2's
    // persist gap is Phase 6's entry criterion, and this asserts the actual
    // bytes on disk rather than trusting the struct definition.
    let saved = std::fs::read_to_string(&state_path).expect("the clean stop saved state");
    let parsed: serde_json::Value = serde_json::from_str(&saved).expect("the save is JSON");
    assert!(
        !any_metadata_key(&parsed),
        "the save format must not carry metadata yet (Phase 6 adds it)"
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
