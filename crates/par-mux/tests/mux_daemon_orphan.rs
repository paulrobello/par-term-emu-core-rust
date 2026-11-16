//! Card 01a198178efc: test-spawned daemons die with their spawner. An
//! interrupted gate run kills nextest but not the foreground daemon child,
//! and an orphaned daemon holds its panes' PTYs — measured 2026-11-01,
//! 55 orphans exhausted the host's pty supply ("out of pty devices" on a
//! fresh openpty).

#![cfg(feature = "mux")]

#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

#[cfg(unix)]
use common::{wait_listening, MuxFixture};
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};

/// The spawner-shell wrapper: backgrounds the daemon, records its pid,
/// readies the test, then sleeps — alive on purpose so the test controls
/// the orphaning moment (kill it and the daemon's parent is gone, exactly
/// like a cancelled gate run killing nextest).
#[cfg(unix)]
fn spawner_script(bin: &str, sock: &std::path::Path, dir: &std::path::Path) -> String {
    let pidfile = dir.join("daemon.pid");
    let readyfile = dir.join("spawner.ready");
    let state = dir.join("state");
    format!(
        "env -u PAR_MUX_ENV PAR_MUX_DIE_WITH_PARENT=1 '{bin}' --foreground --socket '{s}' --state-dir '{st}' >/dev/null 2>&1 & d=$!; echo $d > '{pf}'; touch '{rf}'; sleep 300",
        s = sock.display(),
        st = state.display(),
        pf = pidfile.display(),
        rf = readyfile.display(),
    )
}

/// Poll `probe` until true or the bound.
#[cfg(unix)]
fn poll_until(bound: Duration, what: &str, mut probe: impl FnMut() -> bool) {
    let deadline = Instant::now() + bound;
    while !probe() {
        assert!(Instant::now() < deadline, "never observed: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `pgrep -f` for the first matching pid.
#[cfg(unix)]
fn pgrep_first(pattern: &str) -> Option<u32> {
    let out = Command::new("pgrep").arg("-f").arg(pattern).output().ok()?;
    let line = std::string::String::from_utf8_lossy(&out.stdout);
    line.lines().next()?.trim().parse().ok()
}

/// The pane child's findable marker, unique per fixture run: the test's
/// pgrep must never match a foreign `sleep 3600` left on the host by an
/// earlier run (card 01a1a7d2 — two leaked strays made this test fail
/// deterministically on every commit). The marker is a per-fixture
/// SCRIPT PATH (a single token): the pane command survives the daemon's
/// flatten-and-reparse spawn only as plain tokens, so any quoting-based
/// marker dies in the re-parse — a unique path cannot.
#[cfg(unix)]
fn pane_child_script(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let unique = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("orphan");
    let path = dir.join(format!("orphan-sleeper-{unique}.sh"));
    fs::write(&path, "#!/bin/sh\nsleep 3600\n").expect("sleeper script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("exec bit");
    path
}

/// `ps -p PID` liveness (exit status only).
#[cfg(unix)]
fn pid_alive(pid: &str) -> bool {
    Command::new("ps")
        .args(["-p", pid])
        .stdin(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
#[test]
fn a_die_with_parent_daemon_exits_and_frees_its_panes_when_orphaned() {
    let fixture = MuxFixture::new("die-with-parent");
    let sock = fixture.socket().to_path_buf();
    let dir = fixture.dir().to_path_buf();
    let bin = env!("CARGO_BIN_EXE_par-mux");
    let pidfile = dir.join("daemon.pid");
    let readyfile = dir.join("spawner.ready");

    let mut spawner = Command::new("/bin/bash")
        .arg("-c")
        .arg(spawner_script(bin, &sock, &dir))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawner shell");

    // The daemon is up before the pane work starts.
    poll_until(Duration::from_secs(15), "spawner readied", || {
        readyfile.exists()
    });
    wait_listening(&sock);

    // A pane whose child is a distinct, findable process — the pane's
    // PTY master lives in the daemon, so the child proves the tty is
    // held. The child is the per-fixture sleeper SCRIPT: its PATH is the
    // marker (unique per run), so every pgrep below is fixture-scoped
    // and a foreign `sleep 3600` left on the host by an earlier run
    // cannot read as this pane (card 01a1a7d2). The pane env vars are
    // stripped too, so the client cannot inherit this shell's PAR_MUX_*
    // contract even by accident.
    let sleeper = pane_child_script(&dir);
    let marker = sleeper.to_string_lossy().into_owned();
    let new_session = format!("new-session -s orphan {}", sleeper.display());
    let out = Command::new(bin)
        .args(["--socket"])
        .arg(&sock)
        .args(["--cmd"])
        .arg(&new_session)
        .env_remove("PAR_MUX_SOCKET")
        .env_remove("PAR_MUX_ENV")
        .env_remove("PAR_MUX_PANE_ID")
        .env_remove("PAR_MUX_SESSION_ID")
        .env_remove("PAR_MUX_WINDOW_ID")
        .stdin(Stdio::null())
        .output()
        .expect("new-session runs");
    assert!(
        out.status.success(),
        "new-session: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pane_pid = poll_until_find(
        Duration::from_secs(10),
        "pane sleep child observable",
        || pgrep_first(&marker),
    );
    assert!(pane_pid.is_some(), "the pane child was never observable");

    // Orphan the daemon exactly like a cancelled gate run: SIGKILL the
    // spawner. getppid() flips to 1; the watchdog requests shutdown.
    let _ = Command::new("kill")
        .args(["-9", &spawner.id().to_string()])
        .output();
    let _ = spawner.wait();

    // The daemon exits within the bound and takes the pane child with it.
    let daemon_pid = std::fs::read_to_string(&pidfile).expect("pidfile");
    let daemon_pid = daemon_pid.trim().to_string();
    poll_until(
        Duration::from_secs(15),
        "daemon exited after orphaning",
        || !pid_alive(&daemon_pid),
    );
    poll_until(
        Duration::from_secs(15),
        "pane child died with the daemon",
        || pgrep_first(&marker).is_none(),
    );
}

/// The `--socket` FLAG beats `$PAR_MUX_SOCKET` (card 01a1a7d2 criterion 2):
/// a client invoked from inside a pane — whose environment carries the
/// pane contract's socket — still reaches exactly the daemon its flag
/// names. The env var can never redirect a flagged client to the system
/// daemon.
#[cfg(unix)]
#[test]
fn the_socket_flag_beats_the_env_socket() {
    let fixture = MuxFixture::new("flag-beats-env");
    let sock = fixture.socket().to_path_buf();
    let dir = fixture.dir().to_path_buf();
    let bin = env!("CARGO_BIN_EXE_par-mux");
    let pidfile = dir.join("daemon.pid");
    let readyfile = dir.join("spawner.ready");

    let mut spawner = Command::new("/bin/bash")
        .arg("-c")
        .arg(spawner_script(bin, &sock, &dir))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawner shell");
    poll_until(Duration::from_secs(15), "spawner readied", || {
        readyfile.exists()
    });
    wait_listening(&sock);

    // A dead flag path while the env names the LIVE fixture socket: the
    // client must fail against the flag path, proving the env never
    // redirected it.
    let dead = dir.join("dead.sock");
    let out = Command::new(bin)
        .args(["--socket"])
        .arg(&dead)
        .args(["--cmd", "list-sessions"])
        .env("PAR_MUX_SOCKET", &sock)
        .stdin(Stdio::null())
        .output()
        .expect("client runs");
    assert!(
        !out.status.success(),
        "the client must fail: no daemon serves the flag path"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no daemon running on") && stderr.contains(dead.to_string_lossy().as_ref()),
        "the error names the FLAG path, not the env path: {stderr}"
    );

    // Clean up: orphan this fixture's daemon the same way.
    let _ = Command::new("kill")
        .args(["-9", &spawner.id().to_string()])
        .output();
    let _ = spawner.wait();
    let daemon_pid = fs::read_to_string(&pidfile).expect("pidfile");
    let daemon_pid = daemon_pid.trim().to_string();
    poll_until(
        Duration::from_secs(15),
        "daemon exited after orphaning",
        || !pid_alive(&daemon_pid),
    );
}

/// `poll_until` over an Option probe, returning the first found value.
#[cfg(unix)]
fn poll_until_find(
    bound: Duration,
    what: &str,
    mut probe: impl FnMut() -> Option<u32>,
) -> Option<u32> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(v) = probe() {
            return Some(v);
        }
        assert!(Instant::now() < deadline, "never observed: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
