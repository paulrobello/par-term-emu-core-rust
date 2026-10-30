//! Runtime tests for the shipped claude hook asset: the v4 nested-claude
//! guard, the identity stamp on state reports, and the release ownership
//! proof, executed against the real script over a real unix socket.
//!
//! Every hook run is detached with a double fork, so the hook's ancestry
//! ends at launchd: a gate run from inside an agent pane otherwise sits
//! under claude processes itself, and the guard would (correctly) silence
//! the top-level control run. Nested runs route the hook through two
//! claude-named wrapper scripts — two ancestry matches, so the walk must
//! classify them nested.

use super::claude_hook_asset;
use super::temp_root;
use serde_json::Value as JsonValue;
use std::fs;
use std::io::BufRead;
use std::path::Path;
use std::time::{Duration, Instant};

fn python3_or_skip() -> bool {
    std::process::Command::new("python3")
        .arg("-c")
        .arg("pass")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The claude-named wrapper chain: both wrappers live under a directory
/// named `claude`, so each wrapper's ps command line matches the hook's
/// component matcher (the daemon argv-matcher's rules).
fn write_claude_wrappers(root: &Path) {
    let dir = root.join("claude");
    fs::create_dir(&dir).expect("wrapper dir");
    for name in ["outer", "inner"] {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n\"$@\"\n").expect("wrapper script");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("exec bit");
    }
}

/// Bind the fake control socket the hook reports through.
fn bind_bench_socket(root: &Path) -> (std::os::unix::net::UnixListener, std::path::PathBuf) {
    let sock = root.join("control.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
    (listener, sock)
}

/// The double-fork launcher: forks, detaches its session, forks again, and
/// runs the hook in the grandchild (ancestry: launchd), with the pane env
/// contract set. The middle process exits immediately, so the walk never
/// sees the test harness; the launcher records the hook's exit code and
/// stderr in a result file the test polls for.
const LAUNCHER_PY: &str = r#"import json, os, subprocess, sys
hook, action, payload_path, sock, mode, result_path = sys.argv[1:7]
if os.fork() > 0:
    sys.exit(0)
os.setsid()
if os.fork() > 0:
    sys.exit(0)
env = dict(os.environ)
env["PAR_MUX_ENV"] = "1"
env["PAR_MUX_SOCKET"] = sock
env["PAR_MUX_PANE_ID"] = "%0"
with open(payload_path, "rb") as stdin_file:
    if mode == "nested":
        base = os.path.dirname(hook)
        cmd = [
            os.path.join(base, "claude", "outer"),
            os.path.join(base, "claude", "inner"),
            "sh", hook, action,
        ]
    else:
        cmd = ["sh", hook, action]
    proc = subprocess.run(cmd, stdin=stdin_file, env=env, capture_output=True, timeout=30)
tmp_path = result_path + ".part"
with open(tmp_path, "w") as out:
    out.write(json.dumps({"code": proc.returncode, "err": proc.stderr.decode()}))
os.replace(tmp_path, result_path)
"#;

/// Run the hook once, fully detached, and resolve with the run's exit code
/// and stderr once it has finished.
fn run_hook_detached(
    root: &Path,
    tag: &str,
    action: &str,
    payload: &str,
    sock: &Path,
    nested: bool,
) -> JsonValue {
    let payload_path = root.join(format!("{tag}.payload"));
    fs::write(&payload_path, payload).expect("payload");
    let result_path = root.join(format!("{tag}.result"));
    let launcher_path = root.join(format!("{tag}.launcher.py"));
    fs::write(&launcher_path, LAUNCHER_PY).expect("launcher");
    std::process::Command::new("python3")
        .arg(&launcher_path)
        .arg(root.join("par-mux-claude-session-hook.sh"))
        .arg(action)
        .arg(&payload_path)
        .arg(sock)
        .arg(if nested { "nested" } else { "top" })
        .arg(&result_path)
        .spawn()
        .expect("launcher spawns")
        // The launcher's top process exits as soon as it forks; the hook
        // itself runs in the detached grandchild.
        .wait()
        .expect("launcher wait");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !result_path.exists() {
        assert!(Instant::now() < deadline, "{tag}: the hook never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
    serde_json::from_str(&fs::read_to_string(&result_path).expect("result")).expect("result JSON")
}

/// Drain connections until `want` request lines arrive or `timeout` passes
/// — `want: 0` waits the whole window to prove nothing ever connected. The
/// accepted streams inherit the listener's nonblocking mode, so reads poll
/// with WouldBlock instead of `set_read_timeout` (EINVAL on macOS for a
/// socket accepted nonblocking).
fn collect_requests(
    listener: &std::os::unix::net::UnixListener,
    want: usize,
    timeout: Duration,
) -> Vec<String> {
    listener.set_nonblocking(true).expect("nonblocking");
    let deadline = Instant::now() + timeout;
    let mut requests = Vec::new();
    // The loop drains the FULL window when want is 0 — a satisfied-count
    // early exit would never poll, and a silence assertion would pass
    // without waiting.
    while Instant::now() < deadline {
        if want > 0 && requests.len() >= want {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                let read_deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    match reader.read_line(&mut line) {
                        Ok(0) | Ok(_) => break,
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            if Instant::now() >= read_deadline {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                if !line.trim().is_empty() {
                    requests.push(line);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("accept failed: {err}"),
        }
    }
    requests
}

/// The v4 nested-claude guard, executed for real: the shipped script speaks
/// for a top-level claude and stays silent when the claude it serves was
/// spawned by another claude — both the SessionStart report that used to
/// re-key the parent's claim and the SessionEnd release that used to wipe
/// it. Acceptance criteria 1 and 2, hook side: nothing leaves the nested
/// run, so the pane's claim, roster entry, and resume identity are untouched.
#[cfg(unix)]
#[test]
fn the_hook_speaks_at_top_level_and_stays_silent_for_a_nested_claude() {
    if !python3_or_skip() {
        eprintln!("skipping: python3 is not available");
        return;
    }
    let root = temp_root();
    let hook = root.path().join("par-mux-claude-session-hook.sh");
    fs::write(&hook, claude_hook_asset()).expect("hook asset");
    write_claude_wrappers(root.path());
    let (listener, sock) = bind_bench_socket(root.path());

    // Top-level: the SessionStart identity report goes out.
    let done = run_hook_detached(
        root.path(),
        "top-session",
        "session",
        r#"{"session_id":"s-parent","transcript_path":"/tmp/parent.jsonl","source":"startup"}"#,
        &sock,
        false,
    );
    assert_eq!(done["code"], 0, "the hook exits clean: {done}");
    let requests = collect_requests(&listener, 1, Duration::from_secs(3));
    assert_eq!(requests.len(), 1, "exactly one request: {requests:?}");
    let request: JsonValue = serde_json::from_str(requests[0].trim()).expect("JSON line");
    assert_eq!(request["method"], "pane.report_agent_session", "{request}");
    assert_eq!(
        request["params"]["agent_session_id"], "s-parent",
        "{request}"
    );
    assert_eq!(
        request["params"]["agent_session_path"], "/tmp/parent.jsonl",
        "{request}"
    );
    assert_eq!(
        request["params"]["session_start_source"], "startup",
        "{request}"
    );
    assert_eq!(
        request["params"]["session_resume_argv"][2], "s-parent",
        "the resume argv is intact: {request}"
    );

    // Nested: the child's SessionStart sends nothing (the report that used
    // to re-key the parent's claim to the child's session).
    run_hook_detached(
        root.path(),
        "nested-session",
        "session",
        r#"{"session_id":"s-child","transcript_path":"/tmp/child.jsonl","source":"startup"}"#,
        &sock,
        true,
    );
    let requests = collect_requests(&listener, 0, Duration::from_millis(800));
    assert!(
        requests.is_empty(),
        "a nested claude sends nothing: {requests:?}"
    );

    // Nested release: the child's SessionEnd sends nothing (the half that
    // used to wipe the parent's claim outright).
    run_hook_detached(
        root.path(),
        "nested-release",
        "release",
        r#"{"session_id":"s-child","reason":"clear"}"#,
        &sock,
        true,
    );
    let requests = collect_requests(&listener, 0, Duration::from_millis(800));
    assert!(
        requests.is_empty(),
        "a nested claude releases nothing: {requests:?}"
    );
}

/// Every v4 report carries the session id: a UserPromptSubmit state report
/// re-establishes a claim that lost its identity without waiting for a
/// SessionStart, and the SessionEnd release carries the ownership proof the
/// daemon's release guard checks.
#[cfg(unix)]
#[test]
fn the_hook_stamps_identity_on_state_reports_and_a_proof_on_its_release() {
    if !python3_or_skip() {
        eprintln!("skipping: python3 is not available");
        return;
    }
    let root = temp_root();
    let hook = root.path().join("par-mux-claude-session-hook.sh");
    fs::write(&hook, claude_hook_asset()).expect("hook asset");
    let (listener, sock) = bind_bench_socket(root.path());

    let done = run_hook_detached(
        root.path(),
        "top-prompt",
        "prompt",
        r#"{"session_id":"s-parent","transcript_path":"/tmp/parent.jsonl","prompt":"hi"}"#,
        &sock,
        false,
    );
    assert_eq!(done["code"], 0, "{done}");
    let requests = collect_requests(&listener, 1, Duration::from_secs(3));
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request: JsonValue = serde_json::from_str(requests[0].trim()).expect("JSON line");
    assert_eq!(request["method"], "pane.report_agent", "{request}");
    assert_eq!(request["params"]["state"], "working", "{request}");
    assert_eq!(
        request["params"]["agent_session_id"], "s-parent",
        "{request}"
    );
    assert_eq!(
        request["params"]["agent_session_path"], "/tmp/parent.jsonl",
        "{request}"
    );

    let done = run_hook_detached(
        root.path(),
        "top-release",
        "release",
        r#"{"session_id":"s-parent","reason":"clear"}"#,
        &sock,
        false,
    );
    assert_eq!(done["code"], 0, "{done}");
    let requests = collect_requests(&listener, 1, Duration::from_secs(3));
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request: JsonValue = serde_json::from_str(requests[0].trim()).expect("JSON line");
    assert_eq!(request["method"], "pane.release_agent", "{request}");
    assert_eq!(
        request["params"]["agent_session_id"], "s-parent",
        "{request}"
    );
}
