//! Shared helpers for the mux integration tests (QA-102).
//!
//! Each integration test file is its own crate, so a helper copied per file
//! drifts; these live once here and are pulled in with `mod common;`. Not
//! every test binary uses every helper, hence the module-wide dead_code
//! allow — it is the price of one source of truth.

#![allow(dead_code)]

use par_mux::mux::connect_local_stream;
use par_mux::mux::persist::{state_file_in, state_file_path};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A socket path and state dir that no other test run can ever name,
/// removed even when the test panics.
///
/// A `process::id()`-derived path in the shared temp dir repeats once the OS
/// recycles the pid, so a remnant of an earlier run — or an orphaned daemon
/// still listening on it — collides with a later one. A trailing
/// `remove_file` never runs on an early return or a failed assertion, so the
/// remnants accumulate. `TempDir` answers both: its name carries OS-provided
/// randomness and its `Drop` removes the directory, socket and state included.
///
/// The socket's file NAME carries that randomness too, not just its
/// directory: the daemon keys its state file by the socket's stem, and a
/// daemon this test cannot pass `--state-dir` to (one started by
/// `MuxClient::connect_or_spawn_at`) writes into the shared platform state
/// dir, where a fixed stem would collide across runs. `Drop` removes that
/// platform file for this fixture's stem only.
pub struct MuxFixture {
    dir: tempfile::TempDir,
    socket: PathBuf,
}

impl MuxFixture {
    pub fn new(tag: &str) -> Self {
        // Short prefix and name: macOS caps a Unix socket path at 104 bytes
        // and `$TMPDIR` alone already spends ~49 of them.
        let dir = tempfile::Builder::new()
            .prefix("par-mux-")
            .tempdir()
            .expect("create fixture temp dir");
        let unique = dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("par-mux-"))
            .expect("tempdir name carries the prefix")
            .to_string();
        let socket = dir.path().join(format!("{tag}-{unique}"));
        Self { dir, socket }
    }

    /// The socket path to bind or connect.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The `--state-dir` a spawned daemon persists under — inside the
    /// fixture, so the platform state dir is never touched.
    pub fn state_dir(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// The config file this fixture's daemons run under: inside the
    /// fixture, so the suite never touches (or inherits) the developer's
    /// real config. Written by [`spawn_daemon`] /
    /// [`spawn_daemon_auto_remove`] before the daemon starts.
    pub fn config_path(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    /// Where a daemon started with [`Self::state_dir`] saves its tree.
    pub fn state_path(&self) -> PathBuf {
        state_file_in(&self.state_dir(), &self.socket)
    }
}

impl Drop for MuxFixture {
    fn drop(&mut self) {
        let platform = state_file_path(&self.socket);
        let mut tmp = platform.as_os_str().to_os_string();
        tmp.push(".tmp");
        let _ = std::fs::remove_file(&platform);
        let _ = std::fs::remove_file(PathBuf::from(tmp));
    }
}

/// One reply block must complete within this budget: a daemon that drains
/// forever (reply lines that never reach `%end`) fails the issuing test
/// here with the partial block in hand. A single read that never RETURNS
/// (a wedged daemon holding the socket open) is nextest's
/// `terminate-after` to kill, not this loop's.
const COMMAND_DEADLINE: Duration = Duration::from_secs(10);

/// Run one command and drain its `%begin`…`%end` block. Pushed `%output`
/// notifications may interleave with the reply; they are collected as body
/// noise, same as the daemon tests.
pub fn command(stream: &mut impl Write, reader: &mut impl BufRead, line: &str) -> Vec<String> {
    writeln!(stream, "{line}").expect("write command");
    stream.flush().expect("flush");
    let started = Instant::now();
    let mut out = Vec::new();
    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf).expect("read reply");
        assert!(n > 0, "server closed while answering {line:?}");
        let done = buf.starts_with("%end") || buf.starts_with("%error");
        out.push(buf);
        if done {
            return out;
        }
        assert!(
            started.elapsed() < COMMAND_DEADLINE,
            "command {line:?} never completed within {COMMAND_DEADLINE:?}: {out:?}"
        );
    }
}

/// Every pane-id-shaped line (`%` + digit) in a reply block.
pub fn pane_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            l.strip_prefix('%')
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
        .collect()
}

/// The pid printed after `marker` by an executed `echo <marker> $$`, if the
/// executed output has landed. The echoed command line carries a literal
/// `$$` there instead, so occurrences without following digits (the tty's
/// echo of the command itself) are skipped.
pub fn pid_after(marker: &str, text: &str) -> Option<u32> {
    let needle = format!("{marker} ");
    let mut from = 0;
    while let Some(found) = text[from..].find(&needle) {
        let rest = text[from + found + needle.len()..].trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(pid) = digits.parse() {
            return Some(pid);
        }
        from += found + needle.len();
    }
    None
}

/// Owns a spawned daemon `Child` and guarantees it is not left running.
///
/// The happy path reaps it explicitly (via [`sigterm_clean`], which takes
/// `&mut Child` and receives this through deref coercion). `Drop` is the
/// panic backstop: if a test panics between spawning the daemon and
/// reaching that cleanup, the daemon would otherwise be orphaned —
/// reparented to init and left running, exactly how the 7 orphaned
/// daemons found during the sibling-helpers fix were created. `try_wait`
/// first, because a child already reaped by the happy path's `wait()` may
/// have had its pid recycled by the OS; killing a *stale* pid rather than
/// this one's would be a different bug than the one this guards against.
/// `std::process::Child::kill` (not a raw signal by pid) is deliberately
/// used: it is a documented no-op once the child has already been waited
/// on, and unlike a SIGTERM-then-wait it cannot hang `Drop` during a panic
/// unwind if the daemon is wedged.
pub struct DaemonGuard(std::process::Child);

impl DaemonGuard {
    /// Wrap an already-spawned daemon the same way [`spawn_daemon`] does,
    /// for tests that need a non-default spawn (a lowered rlimit, a
    /// wrapper) but the same guaranteed cleanup.
    pub fn wrap(child: std::process::Child) -> Self {
        Self(child)
    }
}

impl std::ops::Deref for DaemonGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DaemonGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Spawn the daemon binary on the fixture's socket, persisting into the
/// fixture's state dir, with null stdio: a daemon that outlives a failed
/// assertion must not hold the test harness's output pipe open, or
/// `cargo test` hangs at exit instead of reporting the failure.
///
/// The daemon's config pins `remain-on-exit = true` — the held-dead
/// contract the death-path tests assert against (a dead pane is HELD with
/// its exit code). The auto-remove default is what
/// [`spawn_daemon_auto_remove`] is for; a fixture daemon must never read
/// the developer's real config.
pub fn spawn_daemon(fixture: &MuxFixture) -> DaemonGuard {
    write_fixture_config(fixture, true);
    spawn_daemon_on(
        fixture.socket(),
        Some(&fixture.config_path()),
        fixture.state_dir(),
    )
}

/// [`spawn_daemon`] under the product default: `remain-on-exit = false` —
/// a pane whose child exits is auto-removed through the kill-pane contract.
pub fn spawn_daemon_auto_remove(fixture: &MuxFixture) -> DaemonGuard {
    write_fixture_config(fixture, false);
    spawn_daemon_on(
        fixture.socket(),
        Some(&fixture.config_path()),
        fixture.state_dir(),
    )
}

/// Pin the fixture's config to the given remain-on-exit value.
fn write_fixture_config(fixture: &MuxFixture, remain_on_exit: bool) {
    std::fs::write(
        fixture.config_path(),
        format!("[daemon]\nremain-on-exit = {remain_on_exit}\n"),
    )
    .expect("write fixture config");
}

/// The spawn the two policy helpers share: `--socket`, `--state-dir`,
/// `PAR_MUX_CONFIG` when given, null stdio.
fn spawn_daemon_on(socket: &Path, config: Option<&Path>, state_dir: PathBuf) -> DaemonGuard {
    use std::process::Stdio;
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"));
    command
        .arg("--socket")
        .arg(socket)
        .arg("--state-dir")
        .arg(state_dir);
    if let Some(config) = config {
        command.env("PAR_MUX_CONFIG", config);
    } else {
        command.env_remove("PAR_MUX_CONFIG");
    }
    let child = command
        // Test daemons are deliberate, not nested: strip the pane marker so
        // the nesting guard does not refuse them when the suite itself runs
        // inside a mux pane.
        .env_remove("PAR_MUX_ENV")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon binary spawns");
    DaemonGuard(child)
}

/// Set in the child [`rerun_isolated`] starts, so the test body knows it is
/// the isolated copy.
pub const REEXEC_MARKER: &str = "PAR_TEST_REEXEC";

/// Upper bound on one isolated re-run. A wedged child must fail the parent
/// test, not hang the run.
const REEXEC_DEADLINE: Duration = Duration::from_secs(60);

/// Run `test` (exact name) in a fresh copy of this test binary with `set`
/// and `unset` applied to the CHILD's environment, and return whether it
/// passed. The child sees [`REEXEC_MARKER`].
///
/// This is how a test that needs a different process env gets one without
/// mutating this process's env, which every parallel test (and every daemon
/// they spawn) shares (QA-196). A test using it starts with
/// `if std::env::var_os(REEXEC_MARKER).is_none() { assert!(rerun_isolated(..)); return; }`.
/// The child's stdout and stderr go to a temp file, printed when it fails.
pub fn rerun_isolated(test: &str, set: &[(&str, &str)], unset: &[&str]) -> bool {
    use std::process::Stdio;
    let log = tempfile::NamedTempFile::new().expect("create re-exec log file");
    let mut command = std::process::Command::new(std::env::current_exe().expect("test exe"));
    command
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(REEXEC_MARKER, "1")
        .stdin(Stdio::null())
        .stdout(log.reopen().expect("reopen log for stdout"))
        .stderr(log.reopen().expect("reopen log for stderr"));
    for (key, value) in set {
        command.env(key, value);
    }
    for key in unset {
        command.env_remove(key);
    }
    let mut child = command.spawn().expect("re-exec the test binary");
    let deadline = Instant::now() + REEXEC_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the re-exec child") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let output = std::fs::read_to_string(log.path()).unwrap_or_default();
    // A name `--exact` matches nothing still exits 0 with zero tests run, so
    // require that the one test actually ran.
    let ran_one = output.contains("test result: ok. 1 passed");
    let passed = status.is_some_and(|s| s.success()) && ran_one;
    if !passed {
        eprintln!(
            "isolated re-run of {test} {}:\n{output}",
            match status {
                None => format!("timed out after {REEXEC_DEADLINE:?}"),
                Some(status) if !status.success() => format!("failed ({status})"),
                Some(_) => "ran no test (name mismatch?)".to_string(),
            }
        );
    }
    passed
}

/// Poll until the daemon's socket accepts connections (its listener is up).
/// Panics on timeout: a silent return used to push the failure into a far
/// later `connect`, away from the spawn that never came up.
pub fn wait_listening(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connect_local_stream(path) {
            Ok(_) => return,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => panic!(
                "daemon never listened on {} within 5s: {err}",
                path.display()
            ),
        }
    }
}

/// SIGTERM, then require the clean exit the handler guarantees (Task 3.5).
/// The daemon never exits on its own — forgetting this is an infinite wait.
///
/// Unix-only like its sole consumer (mux_restart): SIGTERM has no Windows
/// equivalent, and the ungated `nix` imports broke the mux_reattach build on
/// Windows (run 36048176346) — mux_reattach compiles this module without
/// using this helper.
#[cfg(unix)]
pub fn sigterm_clean(child: &mut std::process::Child) {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).expect("SIGTERM delivered");
    let status = child.wait().expect("daemon exits");
    assert!(
        status.success(),
        "a clean SIGTERM exits 0 after saving, got {status:?}"
    );
}

/// Poll `query` until `ready(&reply)` holds, then return the reply text —
/// the shell behind a pane is a real process, so its output arrives
/// asynchronously after send-keys returns.
pub fn wait_until(
    stream: &mut impl Write,
    reader: &mut impl BufRead,
    query: &str,
    ready: impl Fn(&str) -> bool,
    what: &str,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = command(stream, reader, query).join("");
        if ready(&out) {
            return out;
        }
        assert!(
            Instant::now() < deadline,
            "never saw {what} in reply to {query:?}: {out}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// [`wait_until`] on a plain marker.
pub fn wait_for(
    stream: &mut impl Write,
    reader: &mut impl BufRead,
    query: &str,
    marker: &str,
) -> String {
    wait_until(
        stream,
        reader,
        query,
        |text| text.contains(marker),
        &format!("marker {marker:?}"),
    )
}

/// [`wait_until`] on a shell having EXECUTED `echo <marker> $$` — the pid
/// in the output, not the literal `$$` in the echoed command line.
pub fn wait_for_pid(
    stream: &mut impl Write,
    reader: &mut impl BufRead,
    pane: &str,
    marker: &str,
) -> u32 {
    let text = wait_until(
        stream,
        reader,
        &format!("refresh-client -t {pane}"),
        |text| pid_after(marker, text).is_some(),
        &format!("pid output after {marker}"),
    );
    pid_after(marker, &text).expect("ready() verified it")
}
