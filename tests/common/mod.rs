//! Shared helpers for the mux integration tests (QA-102).
//!
//! Each integration test file is its own crate, so a helper copied per file
//! drifts; these live once here and are pulled in with `mod common;`. Not
//! every test binary uses every helper, hence the module-wide dead_code
//! allow — it is the price of one source of truth.

#![allow(dead_code)]

use par_term_emu_core_rust::mux::connect_local_stream;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

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

/// Spawn the daemon binary on `path` with null stdio: a daemon that
/// outlives a failed assertion must not hold the test harness's output
/// pipe open, or `cargo test` hangs at exit instead of reporting the
/// failure.
pub fn spawn_daemon(path: &std::path::Path) -> std::process::Child {
    use std::process::Stdio;
    std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon binary spawns")
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
