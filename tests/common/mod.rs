//! Shared helpers for the mux integration tests (QA-102).
//!
//! Each integration test file is its own crate, so a helper copied per file
//! drifts; these live once here and are pulled in with `mod common;`. Not
//! every test binary uses every helper, hence the module-wide dead_code
//! allow — it is the price of one source of truth.

#![allow(dead_code)]

use par_term_emu_core_rust::mux::connect_local_stream;
use par_term_emu_core_rust::mux::persist::{state_file_in, state_file_path};
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

/// Run one command and drain its `%begin`…`%end` block. Pushed `%output`
/// notifications may interleave with the reply; they are collected as body
/// noise, same as the daemon tests.
pub fn command(stream: &mut impl Write, reader: &mut impl BufRead, line: &str) -> Vec<String> {
    writeln!(stream, "{line}").expect("write command");
    stream.flush().expect("flush");
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
pub fn spawn_daemon(fixture: &MuxFixture) -> DaemonGuard {
    use std::process::Stdio;
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(fixture.socket())
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon binary spawns");
    DaemonGuard(child)
}

/// Poll until the daemon's socket accepts connections (its listener is up).
pub fn wait_listening(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while connect_local_stream(path).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
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
