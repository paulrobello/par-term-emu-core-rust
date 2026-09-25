//! The nesting contract: a daemon refuses to start inside a par-mux pane
//! (and `MuxClient` refuses to auto-spawn one there) while every pane-side
//! escape hatch — `--cmd`, `--stop`, `--restart` — keeps working, and the
//! panes of a daemon that DOES start there do not inherit the outer pane's
//! `PAR_MUX_*` identity.

#![cfg(feature = "mux")]

mod common;

use common::{spawn_daemon, wait_listening, MuxFixture};
use par_term_emu_core_rust::mux::connect_local_stream;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound on one CLI invocation. A client that never exits must fail
/// the test, not wedge the run.
const CLI_DEADLINE: Duration = Duration::from_secs(15);

/// What one `par-mux` invocation produced.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run the par-mux binary with `args` and `envs` set (the parent
/// environment kept, as a pane-side invocation would see), bounded by
/// [`CLI_DEADLINE`].
fn par_mux_env(args: &[&str], envs: &[(&str, &str)]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_par-mux"));
    child
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        child.env(key, value);
    }
    let mut child = child.spawn().expect("par-mux spawns");
    let mut out_pipe = child.stdout.take().expect("stdout piped");
    let mut err_pipe = child.stderr.take().expect("stderr piped");
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out_pipe.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err_pipe.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + CLI_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll par-mux") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("par-mux {args:?} did not exit within {CLI_DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Run {
        code: status.code(),
        stdout: out.join().expect("stdout reader"),
        stderr: err.join().expect("stderr reader"),
    }
}

/// A daemon this test started itself (possibly with a nesting env), killed
/// and reaped on drop so a failed assertion cannot orphan it.
struct OwnedDaemon {
    child: Child,
}

impl Drop for OwnedDaemon {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Start the daemon binary on the fixture's socket with `envs`, detached.
fn daemon_with_env(fixture: &MuxFixture, envs: &[(&str, &str)]) -> OwnedDaemon {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_par-mux"));
    cmd.arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    OwnedDaemon {
        child: cmd.spawn().expect("daemon spawns"),
    }
}

/// Poll until the socket STOPS accepting (a stopped daemon).
fn wait_not_listening(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while connect_local_stream(path).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        connect_local_stream(path).is_err(),
        "the socket on {} still accepts after the daemon stopped",
        path.display()
    );
}

/// Serve mode inside a pane: non-zero exit, the override named on stderr,
/// and no socket bound.
#[test]
fn nested_serve_mode_is_refused() {
    let fixture = MuxFixture::new("nestref");
    let socket = fixture.socket().to_string_lossy().into_owned();
    let state = fixture.state_dir().to_string_lossy().into_owned();
    let started = Instant::now();
    let run = par_mux_env(
        &["--socket", &socket, "--state-dir", &state],
        &[("PAR_MUX_ENV", "1")],
    );
    assert_ne!(run.code, Some(0), "refused daemon exits non-zero");
    assert!(
        run.stderr.contains("nested daemon"),
        "the refusal names nesting: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("PAR_MUX_ALLOW_NESTED=1"),
        "the refusal names the override: {:?}",
        run.stderr
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a refusal is immediate, not a timeout: {:?}",
        started.elapsed()
    );
    assert!(
        connect_local_stream(fixture.socket()).is_err(),
        "the refused daemon bound no socket"
    );
}

/// The documented override lets a nested daemon start (criterion 3), and
/// its panes drop the outer identity they inherited (criterion 4): the
/// daemon process carries stale PAR_MUX_* values, its panes must see only
/// their own.
#[test]
fn the_override_starts_a_daemon_whose_panes_drop_the_outer_env() {
    let fixture = MuxFixture::new("nestown");
    let daemon = daemon_with_env(
        &fixture,
        &[
            ("PAR_MUX_ENV", "1"),
            ("PAR_MUX_ALLOW_NESTED", "1"),
            ("PAR_MUX_LEAK_PROBE", "stale-outer-value"),
            ("PAR_MUX_PANE_ID", "%outer"),
        ],
    );
    wait_listening(fixture.socket());
    let socket = fixture.socket().to_string_lossy().into_owned();
    let created = par_mux_env(&["--socket", &socket, "--cmd", "new-session -s own"], &[]);
    assert_eq!(
        created.code,
        Some(0),
        "new-session succeeds: {:?}",
        created.stderr
    );
    let listed = par_mux_env(&["--socket", &socket, "--cmd", "list-panes"], &[]);
    let pane = common::pane_ids(&listed.stdout)
        .first()
        .expect("the session has a pane")
        .clone();

    // Probe both halves of the env contract from inside the pane.
    #[cfg(unix)]
    let probes = [
        "echo AGENV=$PAR_MUX_ENV/$PAR_MUX_PANE_ID/$PAR_MUX_SOCKET".to_string(),
        "echo LEAK=[$PAR_MUX_LEAK_PROBE]".to_string(),
    ];
    #[cfg(windows)]
    let probes = [
        "echo AGENV=%PAR_MUX_ENV%/%PAR_MUX_PANE_ID%/%PAR_MUX_SOCKET%".to_string(),
        "if defined PAR_MUX_LEAK_PROBE (echo LEAK=set) else (echo LEAK=unset)".to_string(),
    ];
    for probe in &probes {
        let typed = probe.replace('\'', r"'\''");
        par_mux_env(
            &[
                "--socket",
                &socket,
                "--cmd",
                &format!("send-keys -t {pane} -l '{typed}'"),
            ],
            &[],
        );
        par_mux_env(
            &[
                "--socket",
                &socket,
                "--cmd",
                &format!("send-keys -t {pane} Enter"),
            ],
            &[],
        );
    }

    let want_env = format!("AGENV=1/{pane}/{socket}");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw_env = false;
    let mut saw_leak = false;
    loop {
        let screen = par_mux_env(
            &[
                "--socket",
                &socket,
                "--cmd",
                &format!("capture-pane -t {pane}"),
            ],
            &[],
        )
        .stdout;
        if !saw_env && screen.lines().any(|l| l.trim() == want_env) {
            saw_env = true;
        }
        #[cfg(unix)]
        let leaked = screen
            .lines()
            .any(|l| l.trim() == "LEAK=[stale-outer-value]");
        #[cfg(windows)]
        let leaked = screen.lines().any(|l| l.trim() == "LEAK=set");
        #[cfg(unix)]
        let dropped = screen.lines().any(|l| l.trim() == "LEAK=[]");
        #[cfg(windows)]
        let dropped = screen.lines().any(|l| l.trim() == "LEAK=unset");
        if !saw_leak && dropped {
            saw_leak = true;
        }
        assert!(
            !leaked,
            "the outer pane's PAR_MUX_* leaked into the pane: {screen}"
        );
        if saw_env && saw_leak {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pane never ran both probes; env={saw_env} leak={saw_leak}; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let run = par_mux_env(&["--socket", &socket, "--stop"], &[("PAR_MUX_ENV", "1")]);
    assert_eq!(
        run.code,
        Some(0),
        "--stop works inside a pane: {:?}",
        run.stderr
    );
    wait_not_listening(fixture.socket());
    drop(daemon);
}

/// The pane-side escape hatches: --cmd, --restart and --stop all work with
/// PAR_MUX_ENV=1 — a pane must keep reaching (and restarting) its daemon.
#[test]
fn cmd_restart_and_stop_keep_working_inside_a_pane() {
    let fixture = MuxFixture::new("nesthatch");
    let socket = fixture.socket().to_string_lossy().into_owned();
    let daemon = spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let inside: &[(&str, &str)] = &[("PAR_MUX_ENV", "1")];

    // Client mode never consults the nesting guard.
    let run = par_mux_env(&["--socket", &socket, "--cmd", "list-sessions"], inside);
    assert_eq!(
        run.code,
        Some(0),
        "--cmd works inside a pane: {:?}",
        run.stderr
    );

    // --restart stops the old daemon and serves again from this process,
    // exempt from the guard like tmux's kill-server.
    let replacement = OwnedDaemon {
        child: Command::new(env!("CARGO_BIN_EXE_par-mux"))
            .arg("--socket")
            .arg(fixture.socket())
            .arg("--state-dir")
            .arg(fixture.state_dir())
            .env("PAR_MUX_ENV", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("--restart spawns"),
    };
    wait_listening(fixture.socket());
    let version = par_mux_env(&["--socket", &socket, "--cmd", "version"], inside);
    assert_eq!(
        version.code,
        Some(0),
        "the restarted daemon serves: {:?}",
        version.stderr
    );
    assert!(
        version
            .stdout
            .contains(par_term_emu_core_rust::mux::build_stamp()),
        "version came from the restarted daemon: {:?}",
        version.stdout
    );

    let run = par_mux_env(&["--socket", &socket, "--stop"], inside);
    assert_eq!(
        run.code,
        Some(0),
        "--stop works inside a pane: {:?}",
        run.stderr
    );
    wait_not_listening(fixture.socket());
    drop(replacement);
    drop(daemon);
}

/// `connect_or_spawn` from inside a pane: attaching to a live daemon works,
/// auto-spawning when none owns the socket is refused fast instead of
/// burning the ten-second connect deadline on a daemon the guard kills.
#[test]
fn auto_spawn_inside_a_pane_is_refused_fast() {
    let fixture = MuxFixture::new("nestrsp");
    std::env::set_var("PAR_MUX_ENV", "1");
    let started = Instant::now();
    let err = match par_term_emu_core_rust::mux::MuxClient::connect_or_spawn_at(fixture.socket()) {
        Err(err) => err,
        Ok(_client) => panic!("auto-spawn inside a pane is refused"),
    };
    let elapsed = started.elapsed();
    std::env::remove_var("PAR_MUX_ENV");
    assert!(
        err.to_string().contains("nested daemon"),
        "the error names nesting: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the refusal is immediate, not a retry deadline: {elapsed:?}"
    );
    assert!(
        connect_local_stream(fixture.socket()).is_err(),
        "nothing was spawned on the socket"
    );
}
