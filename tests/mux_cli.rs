//! `par-mux --cmd`: the one-shot client mode, driven through the real binary
//! against a real daemon.

#![cfg(feature = "mux")]

// ARC-106: cargo sets CARGO_BIN_EXE_par-mux even when the bin's
// required-features are unmet, so a plain-`mux` build would silently exec a
// stale target/debug/par-mux. Fail loudly instead.
#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{pane_ids, spawn_daemon, wait_listening, DaemonGuard, MuxFixture};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound on one CLI invocation. A client that never exits (a hung
/// pipe read) must fail the test, not wedge the run.
const CLI_DEADLINE: Duration = Duration::from_secs(15);

/// What one `par-mux` invocation produced.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run the par-mux binary with `args`, bounded by [`CLI_DEADLINE`].
fn par_mux(args: &[&str]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("par-mux spawns");
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

/// `par-mux --socket <path> --cmd <command>`.
fn cmd(socket: &Path, command: &str) -> Run {
    let socket = socket.to_str().expect("utf-8 socket path");
    par_mux(&["--socket", socket, "--cmd", command])
}

/// [`cmd`], requiring success.
fn cmd_ok(socket: &Path, command: &str) -> String {
    let run = cmd(socket, command);
    assert_eq!(
        run.code,
        Some(0),
        "{command:?} must succeed; stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
    run.stdout
}

/// A daemon on the fixture's socket with one session in it.
fn daemon_with_session(fixture: &MuxFixture, session: &str) -> DaemonGuard {
    let daemon = spawn_daemon(fixture);
    wait_listening(fixture.socket());
    cmd_ok(fixture.socket(), &format!("new-session -s {session}"));
    daemon
}

#[test]
fn send_keys_from_the_cli_drives_a_live_pane() {
    let fixture = MuxFixture::new("clikeys");
    let _daemon = daemon_with_session(&fixture, "clikeys");
    let socket = fixture.socket();
    let pane = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    // The shell echoes the typed line back with the quote/caret still in
    // it, so only the EXECUTED output contains the joined marker.
    #[cfg(unix)]
    let typed = r#"echo CLI""-MARK"#;
    #[cfg(windows)]
    let typed = "echo CLI^-MARK";
    let sent = cmd_ok(socket, &format!("send-keys -t {pane} '{typed}' Enter"));
    assert!(sent.is_empty(), "send-keys prints nothing: {sent:?}");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        if screen.lines().any(|l| l.trim() == "CLI-MARK") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never ran the CLI-sent command; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_multi_line_reply_prints_one_line_per_reply_line_without_framing() {
    let fixture = MuxFixture::new("clilist");
    let _daemon = daemon_with_session(&fixture, "clilist");
    let socket = fixture.socket();
    let first = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();
    cmd_ok(socket, &format!("split-window -h -t {first}"));

    let listed = cmd_ok(socket, "list-panes");
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 2, "two panes, two lines: {listed:?}");
    for line in &lines {
        assert!(
            !line.starts_with("%begin") && !line.starts_with("%end") && !line.starts_with("%error"),
            "reply framing leaked into stdout: {listed:?}"
        );
    }
    assert_eq!(
        pane_ids(&listed).len(),
        2,
        "each line is one pane id: {listed:?}"
    );
}

/// `split-window -c DIR` (card 01a0d9b2fb02): the new pane's shell actually
/// runs in DIR — `pwd` names it — and a `-c` naming a directory that does
/// not exist succeeds anyway, degrading to home with a visible note.
#[test]
#[cfg(unix)]
fn split_window_c_starts_the_pane_in_the_directory() {
    let fixture = MuxFixture::new("clicwd");
    let _daemon = daemon_with_session(&fixture, "clicwd");
    let socket = fixture.socket();
    let first = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    let dir = tempfile::tempdir().expect("temp dir");
    let target_dir = dir.path().canonicalize().unwrap();
    let split = cmd_ok(
        socket,
        &format!("split-window -t {first} -c {}", dir.path().display()),
    )
    .trim()
    .to_string();
    cmd_ok(socket, &format!("send-keys -t {split} 'pwd' Enter"));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {split}"));
        if screen
            .lines()
            .any(|l| l.trim() == target_dir.to_str().unwrap())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the split pane never reported the -c cwd; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // A gone directory degrades to home rather than failing the command;
    // the note is on the new pane's screen.
    let gone = cmd_ok(
        socket,
        &format!("split-window -t {first} -c /nonexistent-par-mux-c"),
    )
    .trim()
    .to_string();
    let noted = cmd_ok(socket, &format!("capture-pane -t {gone}"));
    assert!(
        noted.contains("is gone; pane started in"),
        "the fallback note is visible; screen:\n{noted}"
    );
}

#[test]
fn an_error_reply_exits_non_zero_with_the_message_on_stderr() {
    let fixture = MuxFixture::new("clierr");
    let _daemon = daemon_with_session(&fixture, "clierr");

    let run = cmd(fixture.socket(), "send-keys -t %999 x");
    assert_eq!(
        run.code,
        Some(1),
        "an %error reply exits 1: {:?}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "nothing on stdout: {:?}", run.stdout);
    assert!(
        run.stderr.contains("no such pane"),
        "the daemon's error text reaches stderr: {:?}",
        run.stderr
    );

    let run = cmd(fixture.socket(), "no-such-command");
    assert_ne!(
        run.code,
        Some(0),
        "an unknown command fails: {:?}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "nothing on stdout: {:?}", run.stdout);
    assert!(!run.stderr.is_empty(), "the failure is reported on stderr");
}

#[test]
fn no_daemon_fails_fast_and_does_not_start_one() {
    let fixture = MuxFixture::new("clinone");
    let started = Instant::now();
    let run = cmd(fixture.socket(), "list-sessions");
    assert_eq!(run.code, Some(1), "no daemon exits 1: {:?}", run.stderr);
    assert!(
        run.stderr.contains("no daemon running"),
        "the reason is on stderr: {:?}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "nothing on stdout: {:?}", run.stdout);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a one-shot command must not wait on a spawn: {:?}",
        started.elapsed()
    );
    assert!(
        par_term_emu_core_rust::mux::connect_local_stream(fixture.socket()).is_err(),
        "client mode must never start a daemon"
    );
}

/// The positional NAME resolves through `default_socket_path`, the same
/// helper the daemon binds with, on both transports.
#[test]
fn the_named_form_reaches_the_daemon_on_the_named_default_socket() {
    let fixture = MuxFixture::new("cliname");
    // Short and unique: macOS caps a Unix socket path at 104 bytes.
    let name = format!(
        "t{}",
        fixture
            .socket()
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('-').next())
            .expect("fixture name carries a unique suffix")
    );
    let socket = par_term_emu_core_rust::mux::default_socket_path(&name);
    let state_dir = fixture.state_dir();
    let daemon = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg(&name)
        .arg("--state-dir")
        .arg(&state_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon spawns");
    let daemon = NamedDaemon {
        child: daemon,
        socket,
    };
    wait_listening(&daemon.socket);

    let run = par_mux(&[&name, "--cmd", "version"]);
    assert_eq!(run.code, Some(0), "version succeeds: {:?}", run.stderr);
    assert!(
        run.stdout
            .contains(par_term_emu_core_rust::mux::build_stamp()),
        "the named form reached this daemon: {:?}",
        run.stdout
    );
}

/// A daemon bound to a named default socket outside the fixture dir: killed,
/// and its socket remnant removed, on drop.
struct NamedDaemon {
    child: std::process::Child,
    socket: std::path::PathBuf,
}

impl Drop for NamedDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// A pane runs `$PAR_MUX_BIN` in client mode against its own daemon with no
/// `par-mux` on PATH: the env contract names the daemon executable and the
/// socket, and the pane's identity vars match the session it lives in.
#[test]
fn a_pane_reaches_its_daemon_through_par_mux_bin() {
    let fixture = MuxFixture::new("clibin");
    let _daemon = daemon_with_session(&fixture, "clibin");
    let socket = fixture.socket();
    let pane = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    #[cfg(unix)]
    let typed = r#"echo "ID=$PAR_MUX_SESSION_ID/$PAR_MUX_SESSION/$PAR_MUX_WINDOW_ID" && "$PAR_MUX_BIN" --socket "$PAR_MUX_SOCKET" --cmd version | sed 's/^/BIN-/'"#;
    #[cfg(windows)]
    let typed = r#"echo ID=%PAR_MUX_SESSION_ID%/%PAR_MUX_SESSION%/%PAR_MUX_WINDOW_ID% & "%PAR_MUX_BIN%" --socket "%PAR_MUX_SOCKET%" --cmd version"#;
    cmd_ok(
        socket,
        &format!("send-keys -t {pane} -l '{}'", typed.replace('\'', r"'\''")),
    );
    cmd_ok(socket, &format!("send-keys -t {pane} Enter"));

    let stamp = par_term_emu_core_rust::mux::build_stamp();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        let ran_identity = screen.lines().any(|l| l.trim() == "ID=$0/clibin/@0");
        let ran_client = screen
            .lines()
            .any(|l| !l.contains("PAR_MUX_BIN") && l.contains(stamp));
        if ran_identity && ran_client {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never reached its daemon via PAR_MUX_BIN; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wait until `pane`'s screen has a line equal to `want`.
fn wait_line(socket: &Path, pane: &str, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        if screen.lines().any(|l| l.trim() == want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never printed {want:?}; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Type a line into `pane` that prints `VAR` as `<tag>=[value]`.
fn echo_var(socket: &Path, pane: &str, tag: &str, var: &str) {
    #[cfg(unix)]
    let typed = format!("echo {tag}=[\"${var}\"]");
    #[cfg(windows)]
    let typed = format!("echo {tag}=[%{var}%]");
    cmd_ok(
        socket,
        &format!("send-keys -t {pane} -l '{}'", typed.replace('\'', r"'\''")),
    );
    cmd_ok(socket, &format!("send-keys -t {pane} Enter"));
}

/// What [`echo_var`] prints for a var the pane does NOT have: sh expands an
/// unset var to empty; cmd.exe echoes the literal `%VAR%` (QA-147 family).
fn absent_line(tag: &str, var: &str) -> String {
    #[cfg(unix)]
    {
        let _ = var;
        format!("{tag}=[]")
    }
    #[cfg(windows)]
    {
        format!("{tag}=[%{var}%]")
    }
}

/// tmux `set-environment` semantics: a pane created after the call sees the
/// session value, a pane created before does not, and `new-session -e`
/// seeds the first pane.
#[test]
fn session_environment_reaches_new_panes_only() {
    let fixture = MuxFixture::new("clienv");
    let daemon = spawn_daemon(&fixture);
    wait_listening(fixture.socket());
    let socket = fixture.socket();
    let session = cmd_ok(socket, "new-session -s clienv -e 'SEEDED=from -e'");
    let session = session.trim();
    let before = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    cmd_ok(
        socket,
        &format!("set-environment -t {session} PMX_LATE 'late value'"),
    );
    let after = cmd_ok(socket, &format!("split-window -t {before}"));
    let after = after.trim();

    echo_var(socket, &before, "SEED", "SEEDED");
    wait_line(socket, &before, "SEED=[from -e]");
    echo_var(socket, &before, "OLD", "PMX_LATE");
    wait_line(socket, &before, &absent_line("OLD", "PMX_LATE"));
    echo_var(socket, after, "NEW", "PMX_LATE");
    wait_line(socket, after, "NEW=[late value]");

    cmd_ok(socket, &format!("set-environment -t {session} -u PMX_LATE"));
    let unset = cmd_ok(socket, &format!("split-window -t {after}"));
    let unset = unset.trim();
    echo_var(socket, unset, "GONE", "PMX_LATE");
    wait_line(socket, unset, &absent_line("GONE", "PMX_LATE"));

    let bad = cmd(socket, "set-environment -t $99 X y");
    assert_eq!(bad.code, Some(1), "unknown session is an error");
    drop(daemon);
}

/// Run the binary with `PAR_MUX_SOCKET` naming `socket` and no explicit
/// `--socket`/NAME: what a command typed inside a pane of that daemon sees.
/// `PAR_MUX_ENV` is stripped so the suite may itself run inside a pane.
fn par_mux_env(socket: &Path, args: &[&str]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .args(args)
        .env("PAR_MUX_SOCKET", socket)
        .env_remove("PAR_MUX_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("par-mux spawns");
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

/// A daemon a test holds no `Child` for (the `--restart` successor is a
/// detached grandchild): shutdown is asked over the socket on drop instead.
#[cfg(unix)]
struct EnvDaemonGuard {
    socket: std::path::PathBuf,
}

#[cfg(unix)]
impl Drop for EnvDaemonGuard {
    fn drop(&mut self) {
        if let Ok(mut stream) = par_term_emu_core_rust::mux::connect_local_stream(&self.socket) {
            use std::io::Write;
            let _ = writeln!(stream, "kill-server");
            let _ = stream.flush();
        }
    }
}

/// The pane env contract's socket var is the CLI's fallback target: with
/// `PAR_MUX_SOCKET` set and no `--socket`/NAME given, `--cmd` reaches the
/// daemon that variable names — the daemon that spawned the pane — instead
/// of the unnamed default socket.
#[test]
fn the_env_socket_default_targets_the_panes_daemon() {
    let fixture = MuxFixture::new("clienvsock");
    let _daemon = daemon_with_session(&fixture, "clienvsock");

    let run = par_mux_env(fixture.socket(), &["--cmd", "list-sessions"]);
    assert_eq!(
        run.code,
        Some(0),
        "env-defaulted --cmd must succeed; stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("clienvsock"),
        "the reply came from the env-named daemon: {:?}",
        run.stdout
    );
}

/// `--restart`/`--stop` honor the env socket too, the "rebuild the daemon
/// from inside its own pane" remedy: the restart stops the running daemon
/// (final save), its detached successor rebinds the same socket and serves
/// the saved tree back, and a later `--stop` through the same env cleanly
/// retires the successor. Unix-only: the successor is a forked, detached
/// process here; a Windows `--restart` serves in the foreground instead.
#[test]
#[cfg(unix)]
fn restart_and_stop_honor_the_env_socket() {
    let fixture = MuxFixture::new("envrestart");
    let mut first = daemon_with_session(&fixture, "envrestart");
    let socket = fixture.socket();

    // --state-dir mirrors the stopped daemon's, so the successor loads the
    // save the stop just wrote instead of the platform state dir's tree.
    let state_dir = fixture
        .state_dir()
        .to_str()
        .expect("utf-8 state dir")
        .to_string();
    let restart = par_mux_env(socket, &["--restart", "--state-dir", state_dir.as_str()]);
    assert_eq!(
        restart.code,
        Some(0),
        "--restart via the env socket succeeds: {:?}",
        restart.stderr
    );
    let status = first.wait().expect("the stopped daemon exits");
    assert!(
        status.success(),
        "a kill-server shutdown exits cleanly, got {status:?}"
    );

    wait_listening(socket);
    let _successor = EnvDaemonGuard {
        socket: socket.to_path_buf(),
    };
    let listed = par_mux_env(socket, &["--cmd", "list-sessions"]);
    assert_eq!(
        listed.code,
        Some(0),
        "the successor serves the env socket: {:?}",
        listed.stderr
    );
    assert!(
        listed.stdout.contains("envrestart"),
        "the saved tree was restored: {:?}",
        listed.stdout
    );

    let stop = par_mux_env(socket, &["--stop"]);
    assert_eq!(
        stop.code,
        Some(0),
        "--stop via the env socket: {:?}",
        stop.stderr
    );
    assert!(
        stop.stderr.contains("stopped the daemon"),
        "the stop report names the env socket: {:?}",
        stop.stderr
    );
    assert!(
        par_term_emu_core_rust::mux::connect_local_stream(socket).is_err(),
        "the socket is released after the stop"
    );
}

/// Explicit targeting wins over the env default: both the `--socket` flag
/// and the positional NAME reach their daemon while `PAR_MUX_SOCKET` points
/// at a path nothing serves.
#[test]
fn explicit_socket_and_name_beat_the_env_default() {
    let fixture = MuxFixture::new("envprec");
    // Short and unique: macOS caps a Unix socket path at 104 bytes.
    let name = format!(
        "t{}",
        fixture
            .socket()
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('-').next())
            .expect("fixture name carries a unique suffix")
    );
    let named_socket = par_term_emu_core_rust::mux::default_socket_path(&name);
    let daemon = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg(&name)
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon spawns");
    let _daemon = NamedDaemon {
        child: daemon,
        socket: named_socket.clone(),
    };
    wait_listening(&named_socket);

    let by_name = par_mux_env(fixture.socket(), &[&name, "--cmd", "version"]);
    assert_eq!(
        by_name.code,
        Some(0),
        "NAME overrides the env default: {:?}",
        by_name.stderr
    );
    assert!(
        by_name
            .stdout
            .contains(par_term_emu_core_rust::mux::build_stamp()),
        "the named daemon answered: {:?}",
        by_name.stdout
    );

    let by_flag = par_mux_env(
        fixture.socket(),
        &[
            "--socket",
            named_socket.to_str().expect("utf-8 socket path"),
            "--cmd",
            "version",
        ],
    );
    assert_eq!(
        by_flag.code,
        Some(0),
        "--socket overrides the env default: {:?}",
        by_flag.stderr
    );
}

// ---- ENH-039: the pane env contract's PAR_MUX_* set and the in-pane
// `--cmd` reachability across the daemon's endpoint modes (card
// 01a0f633, criterion 4). Unix-only: the probes type POSIX shell into a
// pane.

/// The variable names a default daemon's env contract exports into a pane —
/// the pre-ENH-039 set. A contract change fails here first, so it lands
/// deliberately.
#[cfg(unix)]
const DEFAULT_PANE_ENV: [&str; 7] = [
    "PAR_MUX_BIN",
    "PAR_MUX_ENV",
    "PAR_MUX_PANE_ID",
    "PAR_MUX_SESSION",
    "PAR_MUX_SESSION_ID",
    "PAR_MUX_SOCKET",
    "PAR_MUX_WINDOW_ID",
];

/// Spawn the daemon with extra flags on the standard socket/state wiring —
/// [`spawn_daemon`] extended for `--pane-endpoints`.
#[cfg(unix)]
fn spawn_daemon_with_args(fixture: &MuxFixture, extra: &[&str]) -> DaemonGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .args(extra)
        .env_remove("PAR_MUX_ENV")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon spawns");
    DaemonGuard::wrap(child)
}

/// Type one line into `pane` (literal, then Enter).
#[cfg(unix)]
fn type_line(socket: &Path, pane: &str, line: &str) {
    cmd_ok(
        socket,
        &format!("send-keys -t {pane} -l '{}'", line.replace('\'', r"'\''")),
    );
    cmd_ok(socket, &format!("send-keys -t {pane} Enter"));
}

/// The PAR_MUX_* variable names the pane's environment exports, sorted.
/// `cut -d= -f1` keeps every line under the 80-column wrap, and
/// `PAR_MUX_WINDOW_ID` sorts last, so its arrival means the sorted list is
/// complete.
#[cfg(unix)]
fn pane_env_names(socket: &Path, pane: &str) -> Vec<String> {
    type_line(socket, pane, "env | grep ^PAR_MUX | cut -d= -f1 | sort");
    let deadline = Instant::now() + Duration::from_secs(15);
    let screen = loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        if screen.lines().any(|l| l.trim() == "PAR_MUX_WINDOW_ID") {
            break screen;
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never printed its PAR_MUX env; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut names: Vec<String> = screen
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("PAR_MUX_") && l.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        })
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Assert the pane exports exactly `expected` — set equality, so an added
/// or removed variable fails loudly.
#[cfg(unix)]
fn assert_env_set(names: &[String], expected: &[&str]) {
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(names, &want, "the pane's PAR_MUX_* env set");
}

/// `{marker}=MATCH` iff the pane's `var` matches `pattern`: type a `case`
/// probe into the pane and wait for the verdict line. `pattern` is the raw
/// case-pattern text — quote it (`"path"`) for a literal comparison, leave
/// globs (`*.pane-*`) bare. Verdict lines are short, so a wrapped echo of
/// the long probe text can never equal one.
#[cfg(unix)]
fn pane_var_verdict(socket: &Path, pane: &str, var: &str, pattern: &str, marker: &str) -> String {
    type_line(
        socket,
        pane,
        &format!(
            "case \"${var}\" in {pattern}) echo {marker}=MATCH;; *) echo {marker}=DIFF;; esac"
        ),
    );
    let want_match = format!("{marker}=MATCH");
    let want_diff = format!("{marker}=DIFF");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        for line in screen.lines() {
            let line = line.trim();
            if line == want_match || line == want_diff {
                return line.to_string();
            }
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never printed {marker}=MATCH/DIFF; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Type `"$PAR_MUX_BIN" --cmd list-panes; echo LP=$?` into the pane — the
/// in-pane client form with no --socket and no NAME, whose target is
/// whatever the env contract exported — and return the `LP=<code>` verdict
/// line plus the screen it appeared on.
#[cfg(unix)]
fn pane_list_panes_exit(socket: &Path, pane: &str) -> (String, String) {
    type_line(
        socket,
        pane,
        r#""$PAR_MUX_BIN" --cmd list-panes; echo LP=$?"#,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let screen = cmd_ok(socket, &format!("capture-pane -t {pane}"));
        if let Some(line) = screen
            .lines()
            .map(str::trim)
            .find(|l| l.len() == 4 && l.starts_with("LP=") && l.as_bytes()[3].is_ascii_digit())
        {
            return (line.to_string(), screen);
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never printed LP=<code>; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ENH-039 criterion 4, default: a daemon started with neither new flag
/// exports exactly the pre-ENH-039 PAR_MUX_* set, with PAR_MUX_SOCKET
/// naming the control socket — and `list-panes` typed in the pane reaches
/// the daemon.
#[test]
#[cfg(unix)]
fn pane_env_contract_default_exports_the_historical_set() {
    let fixture = MuxFixture::new("clienvdef");
    let _daemon = daemon_with_session(&fixture, "clienvdef");
    let socket = fixture.socket();
    let pane = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    assert_env_set(&pane_env_names(socket, &pane), &DEFAULT_PANE_ENV);
    assert_eq!(
        pane_var_verdict(
            socket,
            &pane,
            "PAR_MUX_SOCKET",
            &format!("\"{}\"", socket.to_str().expect("utf-8 socket path")),
            "SS",
        ),
        "SS=MATCH",
        "the default contract's socket is the control socket"
    );

    let (exit_line, screen) = pane_list_panes_exit(socket, &pane);
    assert_eq!(
        exit_line, "LP=0",
        "list-panes typed in the pane succeeds; screen:\n{screen}"
    );
    assert!(
        screen.contains(pane.as_str()),
        "the reply names this daemon's pane: {screen}"
    );
}

/// ENH-039 criterion 4, exposed: with `--pane-endpoints
/// --expose-control-socket` the pane's PAR_MUX_SOCKET names its hook-only
/// endpoint, the full socket rides as PAR_MUX_CONTROL_SOCKET, and
/// `list-panes` typed in the pane still succeeds — through that fallback,
/// since the endpoint would have refused.
#[test]
#[cfg(unix)]
fn pane_env_contract_with_pane_endpoints_and_exposed_control_socket() {
    let fixture = MuxFixture::new("cliendexp");
    let daemon = spawn_daemon_with_args(&fixture, &["--pane-endpoints", "--expose-control-socket"]);
    wait_listening(fixture.socket());
    let socket = fixture.socket();
    cmd_ok(socket, "new-session -s cliendexp");
    let pane = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    let mut expected = DEFAULT_PANE_ENV.to_vec();
    expected.push("PAR_MUX_CONTROL_SOCKET");
    assert_env_set(&pane_env_names(socket, &pane), &expected);

    assert_eq!(
        pane_var_verdict(socket, &pane, "PAR_MUX_SOCKET", "*.pane-*", "SS"),
        "SS=MATCH",
        "the pane's socket is its own endpoint"
    );
    assert_eq!(
        pane_var_verdict(
            socket,
            &pane,
            "PAR_MUX_CONTROL_SOCKET",
            &format!("\"{}\"", socket.to_str().expect("utf-8 socket path")),
            "CC",
        ),
        "CC=MATCH",
        "the exposed control socket is the full socket"
    );

    let (exit_line, screen) = pane_list_panes_exit(socket, &pane);
    assert_eq!(
        exit_line, "LP=0",
        "list-panes via the exposed control socket succeeds; screen:\n{screen}"
    );
    assert!(
        screen.contains(pane.as_str()),
        "the reply names this daemon's pane: {screen}"
    );

    drop(daemon);
}

/// ENH-039 criterion 4, endpoint-only: `--pane-endpoints` alone keeps the
/// control socket out of the pane's env — exactly the historical variable
/// set, with PAR_MUX_SOCKET naming the endpoint — and `list-panes` typed
/// in the pane exits non-zero with the hook-only message. The daemon
/// itself keeps serving the control socket.
#[test]
#[cfg(unix)]
fn pane_env_contract_with_pane_endpoints_alone_refuses_control() {
    let fixture = MuxFixture::new("cliendonly");
    let daemon = spawn_daemon_with_args(&fixture, &["--pane-endpoints"]);
    wait_listening(fixture.socket());
    let socket = fixture.socket();
    cmd_ok(socket, "new-session -s cliendonly");
    let pane = pane_ids(&cmd_ok(socket, "list-panes"))
        .first()
        .expect("the session has a pane")
        .clone();

    assert_env_set(&pane_env_names(socket, &pane), &DEFAULT_PANE_ENV);
    assert_eq!(
        pane_var_verdict(socket, &pane, "PAR_MUX_SOCKET", "*.pane-*", "SS"),
        "SS=MATCH",
        "the pane's socket is its own endpoint"
    );

    let (exit_line, screen) = pane_list_panes_exit(socket, &pane);
    assert_eq!(
        exit_line, "LP=1",
        "list-panes exits non-zero through the hook-only endpoint; screen:\n{screen}"
    );
    // The refusal names the contract; newline-collapsed because the 80-column
    // wrap can split the message mid-word.
    assert!(
        screen.replace('\n', "").contains("hook-only"),
        "the refusal names the hook-only contract; screen:\n{screen}"
    );
    assert!(
        pane_ids(&cmd_ok(socket, "list-panes")).contains(&pane),
        "the daemon still serves the control socket"
    );

    drop(daemon);
}

/// `par-mux --cmd reload-config` against a live daemon: the re-read
/// reaches the running daemon (no restart) and the reply names each
/// daemon setting. The reply is computed against the DAEMON's process env
/// (`load_canonical` runs in the daemon), not the CLI child's — the
/// second half of the test proves that by pointing the child at a file
/// the daemon cannot see and asserting the daemon still reports
/// unchanged.
#[test]
fn reload_config_command_reaches_a_live_daemon() {
    let fixture = MuxFixture::new("clireload");
    let _daemon = daemon_with_session(&fixture, "clireload");
    let socket = fixture.socket();

    let socket_str = socket.to_str().expect("utf-8 socket");
    let run = par_mux(&["--socket", socket_str, "--cmd", "reload-config"]);
    // No canonical config file for either side: every setting unchanged
    // against the daemon's defaults — the baseline reply shape.
    assert_eq!(
        run.code,
        Some(0),
        "reload-config succeeds with no file: {}{}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("unchanged: daemon.socket"),
        "the per-setting report: {}",
        run.stdout
    );

    // A daemon THAT DID see the config (spawned with PAR_MUX_CONFIG
    // naming a file written AFTER the daemon started, whose state-dir
    // differs from what it applied) reports the diff on reload. This is
    // the live re-read reaching a running daemon, no restart.
    let fixture2 = MuxFixture::new("clireload2");
    let socket2 = fixture2.socket();
    let socket2_str = socket2.to_str().expect("utf-8");
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("config.toml");
    let daemon2 = {
        use std::process::Stdio;
        let child = Command::new(env!("CARGO_BIN_EXE_par-mux"))
            .arg("--socket")
            .arg(socket2)
            .arg("--state-dir")
            .arg(fixture2.state_dir())
            .env("PAR_MUX_CONFIG", &config)
            .env_remove("PAR_MUX_ENV")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("daemon spawns");
        common::DaemonGuard::wrap(child)
    };
    wait_listening(socket2);
    // The file states a state-dir the daemon did NOT start with (the
    // flag tier named fixture2's own dir; the file names a sibling), so
    // the reload's diff has something to report.
    let stated = fixture2.state_dir().join("moved").display().to_string();
    std::fs::write(&config, format!("[daemon]\nstate-dir = \"{stated}\"\n")).expect("write config");
    let run = par_mux(&["--socket", socket2_str, "--cmd", "reload-config"]);
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("restart-required: daemon.state-dir"),
        "the live re-read reports the moved state-dir: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("unchanged: daemon.socket"),
        "unchanged stays unchanged: {}",
        run.stdout
    );
    drop(daemon2);

    // The CLI child's own env cannot change the daemon's re-read: the
    // daemon resolves its config in its own process.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .args(["--socket", socket_str, "--cmd", "reload-config"])
        .env("PAR_MUX_CONFIG", "/nonexistent/nope.toml")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("par-mux spawns");
    let mut out = String::new();
    use std::io::Read as _;
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read");
    let _ = child.wait();
    assert!(
        out.contains("unchanged: daemon.socket"),
        "the daemon's env (not the client's) decides its config: {out}"
    );

    drop(_daemon);
}

/// `par-mux --gen-config` writes the effective config; the file parses
/// back; a second run refuses without --force and succeeds with it.
#[test]
fn gen_config_writes_effective_config_and_refuses_overwrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("config.toml");
    let config_str = config.to_str().expect("utf-8 config path");

    // The deterministic shape: PAR_MUX_CONFIG redirects the output path
    // (the tests never touch the user's real config dir).
    let run = par_mux_with(&["--gen-config"], &[("PAR_MUX_CONFIG", config_str)]);
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let written = std::fs::read_to_string(&config).expect("read generated config");
    assert!(written.contains("[client]"), "{written}");
    assert!(written.contains("prefix"), "{written}");
    assert!(written.contains("[daemon]"), "{written}");
    assert!(written.contains("socket"), "{written}");

    // A second run refuses to overwrite without --force...
    let run = par_mux_with(&["--gen-config"], &[("PAR_MUX_CONFIG", config_str)]);
    assert_eq!(run.code, Some(1), "refusal exits 1: {}", run.stderr);
    assert!(
        run.stderr.contains("--force"),
        "the refusal names the escape hatch: {}",
        run.stderr
    );
    let unchanged = std::fs::read_to_string(&config).expect("still there");

    // ...and --force rewrites.
    let run = par_mux_with(
        &["--gen-config", "--force"],
        &[("PAR_MUX_CONFIG", config_str)],
    );
    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    let rewritten = std::fs::read_to_string(&config).expect("rewritten");
    assert_eq!(unchanged, rewritten, "same effective values, same file");
}

/// [`par_mux`] with extra environment entries for the child.
fn par_mux_with(args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_par-mux"));
    command.args(args).stdin(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("par-mux spawns");
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
