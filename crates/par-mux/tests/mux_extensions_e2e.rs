//! End-to-end proof that the EMBEDDED pi/omp agent-state extensions drive a
//! live daemon. A bun driver loads the real asset file shipped in
//! `integrations/extensions/` (the same bytes `extensions.rs` embeds with
//! `include_str!`) and fires the agent's lifecycle events with the pane env
//! contract set. The accepted reports come back as `%agent-state-changed`
//! broadcasts and the quit release as `%agent-released`, proving asset and
//! hook endpoint agree on the wire.
//!
//! The broadcast count is exact because the assets serialize their sends:
//! the daemon orders reports by ARRIVAL against a per-source monotonic seq,
//! so a send that overtakes its predecessor is stale-dropped and the count
//! falls short. The driver repeats the racing session_start/agent_start pair
//! `BURSTS` times to turn a one-in-N race into a deterministic regression.
//!
//! Ported from par-term's `tests/mux_extensions_e2e.rs` (removed in par-term
//! d00b7e85) onto this repo's daemon binary and embedded assets.
//!
//! Skips (passing, with a note) where bun is absent: the assets are
//! TypeScript and cannot execute without a JS runtime. Unix-only, like the
//! sibling hook tests (SIGTERM stop, `exec -a` namesake process).

#![cfg(all(feature = "mux", unix))]

#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use common::{sigterm_clean, spawn_daemon, wait_listening, MuxFixture};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// A minimal fake pi runtime: records `pi.on(...)` handlers so the test can
/// fire the lifecycle events the real host would fire. `events.on`
/// subscriptions are accepted and ignored.
const DRIVER: &str = r#"
const target = process.argv[2];
const mod = await import(target);
const handlers = {};
const pi = {
  events: { on: (_name, _cb) => {} },
  on: (name, cb) => { (handlers[name] ??= []).push(cb); },
};
mod.default(pi);
const ctx = {
  mode: "tui",
  hasUI: true,
  isIdle: () => false,
  sessionManager: {
    getSessionFile: () => "/tmp/par-mux-e2e-session.json",
    getSessionId: () => "e2e-session-1",
  },
};
const bursts = Number(process.argv[3] ?? 1);
for (let i = 0; i < bursts; i++) {
  for (const cb of handlers["session_start"] ?? []) await cb({ reason: "startup" }, ctx);
  for (const cb of handlers["agent_start"] ?? []) await cb({}, ctx);
}
await new Promise((resolve) => setTimeout(resolve, 1500));
// A quit must release the pane's claim (pane.release_agent): the shutdown
// handler awaits the send, so the release is on the wire before exit.
for (const cb of handlers["session_shutdown"] ?? []) await cb({ type: "session_shutdown", reason: "quit" });
await new Promise((resolve) => setTimeout(resolve, 1500));
"#;

const BURSTS: usize = 25;
/// pi reports a session on BOTH session_start and agent_start: burst 1 gives
/// two broadcasts, later bursts three.
const PI_EXPECTED: usize = 3 * BURSTS - 1;
/// omp reports a session only on agent_start: exactly two per burst.
const OMP_EXPECTED: usize = 2 * BURSTS;

fn bun_available() -> bool {
    match Command::new("bun").arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => panic!("could not probe bun: {err}"),
    }
}

/// A control client: writer half and buffered reader half of one socket.
struct Control(UnixStream, BufReader<UnixStream>);

impl Control {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("daemon accepts control client");
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("timeout installs");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        Control(stream, reader)
    }

    /// Run one command and drain its `%begin`...`%end` block.
    fn command(&mut self, line: &str) -> Vec<String> {
        writeln!(self.0, "{line}").expect("write command");
        self.0.flush().expect("flush");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut out = Vec::new();
        loop {
            let mut buf = String::new();
            match self.1.read_line(&mut buf) {
                Ok(0) => panic!("server closed while answering {line:?}"),
                Ok(_) => {
                    let done = buf.starts_with("%end") || buf.starts_with("%error");
                    out.push(buf);
                    if done {
                        return out;
                    }
                }
                Err(err) if timed_out(&err) => {}
                Err(err) => panic!("read error: {err}"),
            }
            assert!(
                Instant::now() < deadline,
                "{line:?} never completed: {out:?}"
            );
        }
    }

    /// Lines readable within one short read slice; empty when idle.
    fn poll_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let mut buf = String::new();
            match self.1.read_line(&mut buf) {
                Ok(0) => panic!("server closed the control connection"),
                Ok(_) => lines.push(buf),
                Err(err) if timed_out(&err) => return lines,
                Err(err) => panic!("read error: {err}"),
            }
        }
    }
}

fn timed_out(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Drive the embedded asset `file` (under `integrations/extensions/`) with
/// bun against a live daemon and assert the exact broadcast count, the quit
/// release, and the pane leaving the roster.
fn embedded_extension_drives_the_daemon(agent: &str, file: &str, expected: usize) {
    if !bun_available() {
        eprintln!("skipping {agent} e2e: bun not found on PATH");
        return;
    }

    let asset = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("integrations/extensions")
        .join(file);
    assert!(
        asset.is_file(),
        "embedded asset missing: {}",
        asset.display()
    );

    let fixture = MuxFixture::new(agent);
    let socket = fixture.socket().to_path_buf();
    let mut daemon = spawn_daemon(&fixture);
    wait_listening(&socket);
    let mut control = Control::connect(&socket);
    control.command("new-session -s ext-e2e");
    let pane = control
        .command("list-panes")
        .iter()
        .map(|l| l.trim())
        .find(|l| l.starts_with('%') && l[1..].starts_with(|c: char| c.is_ascii_digit()))
        .expect("new-session created a pane")
        .to_string();

    // A claimed pane must host the agent's namesake process, as every real
    // claimed pane does: the daemon's liveness sweep clears a hook claim whose
    // pane tree provably lacks the claimed CLI for two 1s ticks, and the
    // driver runs bun in the TEST process, not the pane. `exec -a` renames
    // the sleeping process to the claimed label.
    control.command(&format!(
        "send-keys -t {pane} \"bash -c 'exec -a {agent} sleep 90'\" Enter"
    ));

    let root = tempfile::tempdir().expect("temp dir");
    let driver = root.path().join("driver.mjs");
    std::fs::write(&driver, DRIVER).expect("write driver");
    let run = Command::new("bun")
        .arg(&driver)
        .arg(&asset)
        .arg(BURSTS.to_string())
        .env("PAR_MUX_ENV", "1")
        .env("PAR_MUX_SOCKET", &socket)
        .env("PAR_MUX_PANE_ID", &pane)
        .env("PAR_MUX_SENDLOG", "1")
        .env_remove("OMPCODE")
        .current_dir(root.path())
        .output()
        .expect("bun runs the embedded extension");
    assert!(
        run.status.success(),
        "driver failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let state_line = format!("%agent-state-changed {pane} {agent} working source=hook");
    let released_line = format!("%agent-released {pane} {agent}");
    let mut broadcasts = 0usize;
    let mut released = false;
    let mut trace: Vec<String> = Vec::new();
    // Notifications queued on the control socket while the driver ran are
    // still buffered, so this loop reads them after the fact.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        for line in control.poll_lines() {
            let line = line.trim_end();
            if line.starts_with("%agent-") {
                trace.push(line.to_string());
            }
            if line == state_line {
                broadcasts += 1;
            } else if line == released_line {
                released = true;
            }
        }
        if broadcasts >= expected && released {
            break;
        }
    }
    assert!(
        broadcasts >= expected && released,
        "{agent} extension: expected {expected} broadcasts AND the quit release, saw \
         {broadcasts} broadcasts and released={released}; notifications in arrival \
         order: {trace:?}\ndriver stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        broadcasts, expected,
        "serialized sends broadcast exactly once per accepted report: {trace:?}"
    );

    // After the shutdown release the pane must be GONE from the roster.
    let roster = control.command("list-agents").join("");
    assert!(
        !roster.lines().any(|l| l.starts_with(&format!("{pane} "))),
        "{pane} must leave the roster after session_shutdown(quit): {roster}"
    );

    drop(control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut daemon);
}

#[test]
fn pi_extension_drives_a_live_daemon() {
    embedded_extension_drives_the_daemon("pi", "par-mux-agent-state.ts", PI_EXPECTED);
}

#[test]
fn omp_extension_drives_a_live_daemon() {
    embedded_extension_drives_the_daemon("omp", "par-mux-omp-agent-state.ts", OMP_EXPECTED);
}
