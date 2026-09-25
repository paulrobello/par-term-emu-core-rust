//! `par-mux --cmd`: the one-shot client mode, driven through the real binary
//! against a real daemon.

#![cfg(feature = "mux")]

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
    wait_line(socket, &before, "OLD=[]");
    echo_var(socket, after, "NEW", "PMX_LATE");
    wait_line(socket, after, "NEW=[late value]");

    cmd_ok(socket, &format!("set-environment -t {session} -u PMX_LATE"));
    let unset = cmd_ok(socket, &format!("split-window -t {after}"));
    let unset = unset.trim();
    echo_var(socket, unset, "GONE", "PMX_LATE");
    wait_line(socket, unset, "GONE=[]");

    let bad = cmd(socket, "set-environment -t $99 X y");
    assert_eq!(bad.code, Some(1), "unknown session is an error");
    drop(daemon);
}
