//! The hook endpoint over the wire (Phase 5, Task 5.2): herdr-shaped JSON
//! reports on the control socket are answered in place with one JSON reply,
//! accepted reports broadcast `%agent-state-changed` to control clients,
//! stale ones do not — and the port proof: herdr's own kimi integration
//! script, env-renamed, drives a live daemon end to end.
//!
//! Unix-only: the daemon is stopped with SIGTERM and the hook script is
//! POSIX sh. Clients here are plain `std` Unix streams (not
//! [`par_term_emu_core_rust::mux::connect_local_stream`]) because the
//! negative assertions need `set_read_timeout`, which the interprocess
//! wrapper does not expose.

#![cfg(all(feature = "mux", unix))]

// ARC-106: cargo sets CARGO_BIN_EXE_par-mux even when the bin's
// required-features are unmet, so a plain-`mux` build would silently exec a
// stale target/debug/par-mux. Fail loudly instead.
#[cfg(not(feature = "mux-bin"))]
compile_error!("this test drives the par-mux binary: build it with --features mux-bin");

mod common;

use base64::Engine as _;
use common::{sigterm_clean, wait_listening, MuxFixture};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// A control client: the writer half and a buffered reader half of one
/// socket, with a long read timeout so replies always land.
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

    /// Run one command and drain its `%begin`…`%end` block, collecting any
    /// interleaved `%output` pushes as body noise on the way past.
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

    /// The next line satisfying `ready`, skipping `%output` noise.
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

    /// Everything readable within `window` (short per-read timeouts), for
    /// negative assertions: what did NOT arrive.
    fn drain_within(&mut self, window: Duration) -> Vec<String> {
        self.0
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("timeout installs");
        let deadline = Instant::now() + window;
        let mut lines = Vec::new();
        while Instant::now() < deadline {
            let mut buf = String::new();
            match self.1.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => lines.push(buf),
                Err(err) if timed_out(&err) => break,
                Err(err) => panic!("read error while draining: {err}"),
            }
        }
        self.0
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout restores");
        lines
    }
}

fn timed_out(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Every pane-id-shaped line (`%` + digit) in a reply block.
fn pane_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            l.strip_prefix('%')
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
        .collect()
}

/// One daemon plus a registered control client and the id of its first
/// pane — the stage every hook test plays on.
struct Stage {
    daemon: common::DaemonGuard,
    path: std::path::PathBuf,
    control: Control,
    pane: String,
    // Last, so it drops after Drop::drop has stopped the daemon: removes the
    // socket and state dir even when the test panicked.
    _fixture: MuxFixture,
}

fn stage(tag: &str) -> Stage {
    stage_with(tag, &[])
}

/// [`stage`] on a daemon started with extra flags — `--pane-endpoints` and
/// friends (ENH-039). Everything else is the same stage.
fn stage_with(tag: &str, daemon_args: &[&str]) -> Stage {
    use std::process::Stdio;
    let fixture = MuxFixture::new(tag);
    let path = fixture.socket().to_path_buf();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_par-mux"))
        .arg("--socket")
        .arg(&path)
        .arg("--state-dir")
        .arg(fixture.state_dir())
        .args(daemon_args)
        // Test daemons are deliberate, not nested: strip the pane marker so
        // the nesting guard does not refuse them when the suite itself runs
        // inside a mux pane (why common::spawn_daemon strips it too).
        .env_remove("PAR_MUX_ENV")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("daemon binary spawns");
    let daemon = common::DaemonGuard::wrap(child);
    wait_listening(&path);
    let mut control = Control::connect(&path);
    control.command("new-session -s hooks");
    let pane = pane_ids(&control.command("list-panes").join(""))
        .first()
        .expect("new-session created a pane")
        .clone();
    Stage {
        daemon,
        path,
        control,
        pane,
        _fixture: fixture,
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Best-effort teardown; a panic mid-test still leaves the daemon to
        // be reaped by the explicit calls in each test's happy path.
        let _ = self.control.0.shutdown(Shutdown::Both);
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        let _ = signal::kill(Pid::from_raw(self.daemon.id() as i32), Signal::SIGTERM);
        let _ = self.daemon.wait();
    }
}

/// Send one hook report line on a fresh connection and read the one reply.
fn hook_round_trip(path: &std::path::Path, report: &str) -> String {
    let stream = UnixStream::connect(path).expect("daemon accepts hook connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout installs");
    let mut writer = stream.try_clone().expect("clone");
    writeln!(writer, "{report}").expect("send report");
    writer.flush().expect("flush");
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    let n = reader.read_line(&mut reply).expect("read reply");
    assert!(n > 0, "server closed before replying");
    reply
}

/// The endpoint path pane `pane`'s env exports as `PAR_MUX_SOCKET`. The
/// pane prints the variable's basename (a full path would wrap past the
/// 80-column screen and split); the test joins it with the fixture dir the
/// endpoints are bound beside. `refresh-client` returns the styled grid —
/// escapes wrap the text and row ends ride as escaped `\r\n` — so the
/// marker is searched anywhere in a line and the value is read only up to
/// the first character a socket file name cannot contain; the `.pane-`
/// requirement skips the echoed command, which also contains the marker.
fn pane_endpoint_of(stage: &mut Stage, pane: &str) -> std::path::PathBuf {
    stage.control.command(&format!(
        "send-keys -t {pane} -l '{}'",
        r"echo PS=${PAR_MUX_SOCKET##*/}"
    ));
    stage.control.command(&format!("send-keys -t {pane} Enter"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found: Option<String> = None;
    while found.is_none() {
        let screen = stage
            .control
            .command(&format!("refresh-client -t {pane}"))
            .join("");
        for line in screen.lines() {
            if let Some(idx) = line.find("PS=") {
                let rest = &line[idx + "PS=".len()..];
                let value: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '%'))
                    .collect();
                if value.contains(".pane-") {
                    found = Some(value);
                    break;
                }
            }
        }
        if found.is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pane {pane} never printed PS=<endpoint>; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let base = found.expect("checked above");
    stage
        .path
        .parent()
        .expect("the control socket has a parent dir")
        .join(base)
}

/// The kimi script's report shape, for hand-driven reports: seq large and
/// spaced so ordering is explicit.
fn report(pane: &str, state: &str, seq: u64) -> String {
    format!(
        r#"{{"id":"probe-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"kimi","state":"{state}","seq":{seq},"source":"par-mux:test"}}}}"#
    )
}

/// A pi/omp-shaped report: same grammar, plus the optional blocked-reason
/// `message` both agents send.
fn report_with_message(pane: &str, state: &str, message: &str, seq: u64) -> String {
    format!(
        r#"{{"id":"probe-{seq}","method":"pane.report_agent","params":{{"pane_id":"{pane}","agent":"pi","state":"{state}","message":"{message}","seq":{seq},"source":"par-mux:test"}}}}"#
    )
}

/// The decoded fields of one roster row (ARC-060 grammar): positions 1-4
/// fixed (`%N agent state source`), then zero or more whitespace-free
/// `key=value` tokens. This is the positional parse a roster consumer
/// writes — the shape par-term's fixed parser targets.
struct RosterRow {
    pane: String,
    agent: String,
    state: String,
    source: String,
    reason: Option<String>,
    telemetry: Option<Vec<u8>>,
    host_telemetry: Option<Vec<u8>>,
}

fn parse_roster_row(row: &str) -> RosterRow {
    let mut tokens = row.split_whitespace();
    let pane = tokens.next().expect("pane token").to_string();
    let agent = tokens.next().expect("agent token").to_string();
    let state = tokens.next().expect("state token").to_string();
    let source = tokens.next().expect("source token").to_string();
    let mut fields = RosterRow {
        pane,
        agent,
        state,
        source,
        reason: None,
        telemetry: None,
        host_telemetry: None,
    };
    for token in tokens {
        let (key, value) = token.split_once('=').unwrap_or_else(|| {
            panic!("every token after source is key=value, got {token:?} in {row:?}")
        });
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(value)
            .unwrap_or_else(|_| panic!("{key} token is standard base64 in {row:?}"));
        match key {
            "reason" => fields.reason = Some(String::from_utf8(decoded).expect("reason is UTF-8")),
            "telemetry" => fields.telemetry = Some(decoded),
            "host_telemetry" => fields.host_telemetry = Some(decoded),
            other => panic!("unknown roster token {other:?} in {row:?}"),
        }
    }
    fields
}

/// The pi session-report shape the shipped par-term asset sends: path-only
/// ref (`currentSessionRef` drops the id when a path exists) plus the
/// resume invocation the extension reports so Phase 6 needs no table entry
/// for it.
fn session_report_with_resume(pane: &str, session_path: &str, seq: u64) -> String {
    format!(
        r#"{{"id":"probe-{seq}","method":"pane.report_agent_session","params":{{"pane_id":"{pane}","agent":"pi","seq":{seq},"source":"par-mux:test","session_start_source":"startup","agent_session_path":"{session_path}","session_resume_argv":["pi","--session","{session_path}"]}}}}"#
    )
}

#[test]
fn a_blocked_reason_rides_the_roster_line_and_leaves_with_the_state() {
    let mut stage = stage("reason");

    // The reply block's body lines, with protocol framing stripped (the
    // roster test's local helper, repeated here).
    fn body_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| {
                !l.starts_with("%output")
                    && !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
            })
            .map(str::trim)
            .collect()
    }

    // The pi-shaped blocked report with a reason drives the live daemon.
    let reply = hook_round_trip(
        &stage.path,
        &report_with_message(
            &stage.pane,
            "blocked",
            "permission needed for rm -rf build/",
            1_000,
        ),
    );
    assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the blocked broadcast",
    );

    // The reason is readable through list-agents without waiting for a
    // broadcast — the reattaching-client case the roster serves. It rides
    // as one `reason=<base64>` token: parse positions 1-4, then every
    // remaining token on its first `=` (the ARC-060 grammar).
    let roster = stage.control.command("list-agents").join("");
    let lines = body_lines(&roster);
    let row = lines
        .iter()
        .find(|line| line.starts_with(&format!("{} pi blocked hook", stage.pane)))
        .expect("the rostered pane carries its blocked row");
    let fields = parse_roster_row(row);
    assert_eq!(
        fields.reason.as_deref(),
        Some("permission needed for rm -rf build/"),
        "the reason decodes from the roster line: {row}"
    );
    assert!(
        fields.telemetry.is_none() && fields.host_telemetry.is_none(),
        "no telemetry tokens on this row: {row}"
    );

    // A later report without a message (empty string = absent) clears it: a
    // stale blocked reason cannot survive into a working state.
    hook_round_trip(
        &stage.path,
        &report_with_message(&stage.pane, "working", "", 2_000),
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the working broadcast",
    );
    let roster = stage.control.command("list-agents").join("");
    let lines = body_lines(&roster);
    assert!(
        lines.contains(&format!("{} pi working hook", stage.pane).as_str()),
        "no reason residue on a message-less state: {lines:?}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

/// ARC-060 conformance: a blocked reason containing `telemetry=` and
/// ending in `hook` — the two shapes that made the old free-text column
/// ambiguous — rides the roster next to a fresh telemetry token, and the
/// positional parse a roster consumer writes (tokens 1-4, then each
/// remaining token split on its first `=`) recovers every field.
#[test]
fn roster_reason_and_telemetry_tokens_parse_positionally() {
    let mut stage = stage("reason-telemetry");

    // The reply block's body lines, with protocol framing stripped (the
    // roster test's local helper, repeated here).
    fn body_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| {
                !l.starts_with("%output")
                    && !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
            })
            .map(str::trim)
            .collect()
    }

    let reply = hook_round_trip(
        &stage.path,
        &report_with_message(&stage.pane, "blocked", "waiting on telemetry=x hook", 1_000),
    );
    assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the blocked broadcast",
    );

    let sampled_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock past epoch")
        .as_millis() as u64
        - 60_000;
    let reply = hook_round_trip(
        &stage.path,
        &format!(
            r#"{{"id":2,"method":"pane.report_agent_telemetry","params":{{"pane_id":"{}","agent":"pi","seq":1001,"source":"par-mux:test","telemetry":{{"version":1,"source":"claude_code","sampled_at_unix_ms":{},"model":"GLM 5.3"}}}}}}"#,
            stage.pane, sampled_at
        ),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "telemetry accepted: {reply}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-telemetry-changed"),
        "the telemetry broadcast",
    );

    let roster = stage.control.command("list-agents").join("");
    let lines = body_lines(&roster);
    assert_eq!(lines.len(), 1, "one rostered pane: {lines:?}");
    let fields = parse_roster_row(lines[0]);
    assert_eq!(fields.pane, stage.pane);
    assert_eq!(fields.agent, "pi");
    assert_eq!(fields.state, "blocked");
    assert_eq!(fields.source, "hook");
    assert_eq!(
        fields.reason.as_deref(),
        Some("waiting on telemetry=x hook"),
        "the reason decodes with both ambiguity shapes intact"
    );
    let telemetry = fields.telemetry.expect("the telemetry token rides the row");
    let parsed: serde_json::Value = serde_json::from_slice(&telemetry)
        .expect("the telemetry token decodes to the canonical JSON");
    assert_eq!(parsed["model"], "GLM 5.3");

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn a_pi_shaped_session_report_with_resume_argv_drives_the_daemon() {
    let mut stage = stage("resume-argv");

    // A state report first, so the session report has a state to
    // rebroadcast — acceptance over the wire is observable only through
    // the rebroadcast (error replies produce no notification). Same agent
    // as the session report below: a session report from a DIFFERENT agent
    // clears the previous claim instead of rebroadcasting it, so a
    // mismatched pair would have nothing to announce.
    let pi_state_report = format!(
        r#"{{"id":"probe-1000","method":"pane.report_agent","params":{{"pane_id":"{}","agent":"pi","state":"working","seq":1000,"source":"par-mux:test"}}}}"#,
        stage.pane
    );
    let reply = hook_round_trip(&stage.path, &pi_state_report);
    assert!(reply.contains(r#""result":"ok""#), "accepted: {reply}");
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the state broadcast",
    );

    // The exact shape the shipped pi asset sends: path-only ref plus its
    // resume invocation. Before the id-or-path contract this report was
    // error-replied (`missing agent_session_id`) — every path-carrying
    // pi/omp session report was silently dropped.
    let reply = hook_round_trip(
        &stage.path,
        &session_report_with_resume(&stage.pane, "/tmp/pi-session.jsonl", 1_001),
    );
    assert!(
        reply.contains(r#""result":"ok""#),
        "the path-only shape is accepted: {reply}"
    );
    // The rebroadcast is the wire-level proof the session report landed:
    // only an accepted session report re-announces the known state.
    let rebroadcast = stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the session report's rebroadcast",
    );
    assert!(
        rebroadcast.contains(format!("{} pi working source=hook", stage.pane).as_str()),
        "the rebroadcast carries the known state: {rebroadcast}"
    );

    // The roster still reads the pane — the report changed identity, not
    // state.
    let roster = stage.control.command("list-agents").join("");
    assert!(
        roster.contains(&format!("{} pi working hook", stage.pane)),
        "the roster still carries the pane: {roster}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn hook_report_replies_in_place_and_broadcasts_to_control_clients() {
    let mut stage = stage("report");
    let reply = hook_round_trip(&stage.path, &report(&stage.pane, "working", 1_000));
    assert!(
        reply.contains(r#""id":"probe-1000""#) && reply.contains(r#""result":"ok""#),
        "one-shot JSON reply echoes the id: {reply}"
    );

    let broadcast = stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the state broadcast",
    );
    assert_eq!(
        broadcast,
        format!(
            "%agent-state-changed {} kimi working source=hook\n",
            stage.pane
        ),
        "the broadcast carries pane, agent, state and provenance"
    );

    stage.control.command("list-sessions"); // daemon still serves commands
    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

/// T5.4: the roster query agrees with what hooks reported — and a hookless
/// pane never appears in it.
#[test]
fn list_agents_agrees_with_hook_reports_and_omits_hookless_panes() {
    let mut stage = stage("roster");

    // A second, hookless pane alongside the claimed one.
    stage
        .control
        .command(&format!("split-window -t {} -h", stage.pane));
    let claimed = stage.pane.clone();
    let hookless = "%1".to_string();

    // The reply block's body lines, with protocol framing and interleaved
    // %output pushes stripped.
    fn body_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| {
                !l.starts_with("%output")
                    && !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    }

    // No reports yet: the roster body is empty.
    let empty = stage.control.command("list-agents").join("");
    assert!(
        body_lines(&empty).is_empty(),
        "before any report the roster carries no lines: {:?}",
        body_lines(&empty)
    );

    // Claim the first pane through the hook path, exactly as a ported
    // script would.
    let reply = hook_round_trip(&stage.path, &report(&claimed, "blocked", 2_000));
    assert!(
        reply.contains(r#""result":"ok""#),
        "claim accepted: {reply}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the claim's broadcast",
    );

    let roster = stage.control.command("list-agents").join("");
    let lines = body_lines(&roster);
    assert!(
        lines.contains(&format!("{claimed} kimi blocked hook").as_str()),
        "the roster line matches the hook's claim, provenance included: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with(&hookless)),
        "the hookless pane is absent: {lines:?}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn stale_report_over_the_wire_writes_and_broadcasts_nothing() {
    let mut stage = stage("stale");

    let reply = hook_round_trip(&stage.path, &report(&stage.pane, "working", 2_000));
    assert!(
        reply.contains(r#""result":"ok""#),
        "in-order accepted: {reply}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the in-order broadcast",
    );

    // Older, then equal sequence: ok replies, but nothing must broadcast.
    for stale in [1_999_u64, 2_000] {
        let reply = hook_round_trip(&stage.path, &report(&stage.pane, "blocked", stale));
        assert!(
            reply.contains(r#""result":"ok""#),
            "dropped is not an error: {reply}"
        );
    }
    let drained = stage.control.drain_within(Duration::from_millis(700));
    let broadcasts = drained
        .iter()
        .filter(|line| line.starts_with("%agent-state-changed"))
        .count();
    assert_eq!(
        broadcasts, 0,
        "stale reports must not broadcast; drained: {drained:?}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn a_hook_connection_receives_no_broadcast_pushes() {
    let mut stage = stage("quiet");

    // Connect the hook FIRST, then trigger pushes from the control side:
    // a connection registered at accept time would already hold
    // `%layout-change`/`%output` lines by the time the split returns.
    let mut hook = UnixStream::connect(&stage.path).expect("daemon accepts hook connection");
    hook.set_read_timeout(Some(Duration::from_millis(400)))
        .expect("timeout installs");
    stage
        .control
        .command(&format!("split-window -t {} -h", stage.pane));

    let mut buf = [0u8; 1024];
    match hook.read(&mut buf) {
        Ok(n) => panic!(
            "the hook connection received a push before its reply: {:?}",
            String::from_utf8_lossy(&buf[..n])
        ),
        Err(err) if timed_out(&err) => {}
        Err(err) => panic!("unexpected read error on the hook connection: {err}"),
    }

    // Now report: the FIRST bytes this connection ever sees are its reply.
    let mut writer = hook.try_clone().expect("clone");
    writeln!(writer, "{}", report(&stage.pane, "idle", 3_000)).expect("send report");
    writer.flush().expect("flush");
    let mut reader = BufReader::new(hook);
    let mut reply = String::new();
    let n = reader.read_line(&mut reply).expect("read reply");
    assert!(n > 0, "server closed before replying");
    assert!(
        reply.contains(r#""result":"ok""#) && !reply.starts_with('%'),
        "the reply is JSON, not a push: {reply}"
    );
    drop((writer, reader));

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

/// The port proof, parameterized over `--pane-endpoints` (ENH-039
/// criterion 4): the asset script is untouched, and in endpoint mode it
/// runs with the pane's own endpoint — the socket the pane's env actually
/// exports — as its `PAR_MUX_SOCKET`, so its own-pane reports still land.
fn herdr_kimi_script_env_renamed_drives(pane_endpoints: bool) {
    let mut stage = stage_with(
        "port-proof",
        if pane_endpoints {
            &["--pane-endpoints"][..]
        } else {
            &[][..]
        },
    );
    let pane = stage.pane.clone();

    // The daemon seeds the env contract into every pane it spawns — that is
    // what lets a ported script run from INSIDE a pane. Endpoint mode keeps
    // the marker pair (`PAR_MUX_ENV`/`PAR_MUX_PANE_ID`) intact.
    stage.control.command(&format!(
        "send-keys -t {pane} 'echo PM=$PAR_MUX_ENV/$PAR_MUX_PANE_ID' Enter"
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = stage
            .control
            .command(&format!("refresh-client -t {pane}"))
            .join("");
        if screen.contains(&format!("PM=1/{pane}")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pane never reported its env: {screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The socket the script reports through: the control socket by default,
    // the pane's own hook-only endpoint under --pane-endpoints — read from
    // the pane's env rather than assumed.
    let script_socket: String = if pane_endpoints {
        let endpoint = pane_endpoint_of(&mut stage, &pane);
        endpoint.to_str().expect("utf-8 endpoint path").to_string()
    } else {
        stage.path.to_str().expect("utf-8 socket path").to_string()
    };

    // herdr's own script, env-renamed, driving the daemon from outside.
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/par-mux-agent-state.sh");
    for action in ["working", "blocked"] {
        let out = std::process::Command::new("sh")
            .arg(&script)
            .arg(action)
            .env("PAR_MUX_ENV", "1")
            .env("PAR_MUX_SOCKET", &script_socket)
            .env("PAR_MUX_PANE_ID", &pane)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("script runs");
        assert!(out.status.success(), "the ported script exits 0: {out:?}");
        let broadcast = stage.control.line_until(
            |line| line.starts_with("%agent-state-changed"),
            &format!("the {action} broadcast"),
        );
        assert_eq!(
            broadcast,
            format!("%agent-state-changed {pane} kimi {action} source=hook\n"),
            "the ported script drove the pane's state"
        );
    }

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

#[test]
fn herdr_kimi_script_env_renamed_drives_a_live_daemon() {
    herdr_kimi_script_env_renamed_drives(false);
}

/// The same port proof against a `--pane-endpoints` daemon: the script's
/// socket is the pane's hook-only endpoint and its own-pane reports still
/// land — the migration property ENH-039 promises (assets unchanged).
#[test]
fn herdr_kimi_script_env_renamed_drives_a_live_daemon_with_pane_endpoints() {
    herdr_kimi_script_env_renamed_drives(true);
}

/// ENH-039 (card 01a0f633, criterion 1): under `--pane-endpoints` a pane's
/// `PAR_MUX_SOCKET` accepts only hook reports. A pane connecting to it and
/// sending `capture-pane -t %0`, `send-keys -t %0 x` or `kill-server` gets
/// the hook-only refusal and a closed connection — and nothing changes:
/// no `x` typed, no server killed, the daemon still serving commands.
#[test]
fn a_pane_endpoint_refuses_control_commands_and_leaves_the_daemon_unchanged() {
    let mut stage = stage_with("endpoint-refuse", &["--pane-endpoints"]);
    let pane = stage.pane.clone();
    let endpoint = pane_endpoint_of(&mut stage, &pane);

    // The pane's visible content — non-blank lines, reply framing
    // (%begin/%end with per-reply counters) stripped. Blank rows churn as
    // the shell's line editor redraws, so only content is compared.
    fn content(text: &str) -> Vec<String> {
        text.lines()
            .filter(|l| {
                !l.starts_with("%begin")
                    && !l.starts_with("%end")
                    && !l.starts_with("%error")
                    && !l.starts_with("%output")
                    && !l.trim().is_empty()
            })
            .map(str::to_string)
            .collect()
    }
    // The pre-refusal content, once the pane is SETTLED: zsh draws the next
    // prompt some time after the echo's output lands, so capture until two
    // consecutive content snapshots agree.
    let mut before = content(
        &stage
            .control
            .command(&format!("capture-pane -t {pane}"))
            .join(""),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let next = content(
            &stage
                .control
                .command(&format!("capture-pane -t {pane}"))
                .join(""),
        );
        let settled = next == before;
        before = next;
        if settled || Instant::now() >= deadline {
            break;
        }
    }

    for refused in [
        format!("capture-pane -t {pane}"),
        format!("send-keys -t {pane} x"),
        "kill-server".to_string(),
    ] {
        let stream = UnixStream::connect(&endpoint).expect("the endpoint accepts");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout installs");
        let mut writer = stream.try_clone().expect("clone");
        writeln!(writer, "{refused}").expect("send command");
        writer.flush().expect("flush");
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        reader.read_line(&mut reply).expect("read the refusal");
        assert_eq!(
            reply.trim(),
            r#"{"error":"hook-only endpoint"}"#,
            "{refused:?} is refused with the hook-only error: {reply}"
        );
        // The endpoint closes after refusing — the next read is EOF, so a
        // control-mode client learns immediately instead of timing out.
        let n = reader
            .read_line(&mut String::new())
            .expect("read after the refusal");
        assert_eq!(n, 0, "the connection closes after the refusal: {refused:?}");
    }

    // No `x` typed (the content lists are equal and `before` has none) and
    // the daemon still serves list-panes (kill-server was refused).
    let after = content(
        &stage
            .control
            .command(&format!("capture-pane -t {pane}"))
            .join(""),
    );
    assert_eq!(
        before, after,
        "the pane's visible content is untouched by the refused commands"
    );
    assert!(
        pane_ids(&stage.control.command("list-panes").join("")).contains(&pane),
        "the daemon still serves commands after kill-server was refused"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}

/// ENH-039 (card 01a0f633, criterion 1): an endpoint is bound to its pane.
/// Over pane %1's endpoint a report naming `%0` is refused with "pane_id
/// does not match this endpoint" and nothing lands; the same report naming
/// `%1` — or omitting pane_id, which the endpoint fills in — is accepted,
/// broadcast, and visible in `list-agents` for `%1`.
#[test]
fn a_pane_endpoint_binds_hook_reports_to_its_own_pane() {
    let mut stage = stage_with("endpoint-bind", &["--pane-endpoints"]);
    stage
        .control
        .command(&format!("split-window -t {} -h", stage.pane));
    let bound = "%1".to_string();
    let endpoint = pane_endpoint_of(&mut stage, &bound);

    // A report for another pane over %1's endpoint is refused before
    // anything is looked up or written: %0 never reaches the roster.
    let cross = hook_round_trip(&endpoint, &report("%0", "working", 1_000));
    assert!(
        cross.contains("pane_id does not match this endpoint"),
        "a cross-pane report is refused: {cross}"
    );

    // The same report naming the bound pane is accepted and broadcast.
    let own = hook_round_trip(&endpoint, &report(&bound, "working", 1_000));
    assert!(
        own.contains(r#""result":"ok""#),
        "the bound pane's own report is accepted: {own}"
    );
    let broadcast = stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the bound pane's broadcast",
    );
    assert_eq!(
        broadcast,
        format!("%agent-state-changed {bound} kimi working source=hook\n"),
        "the accepted report broadcast names the bound pane"
    );

    // A report omitting pane_id is filed for the bound pane — the minimal
    // hook script needs no pane id at all.
    let omitted = hook_round_trip(
        &endpoint,
        r#"{"id":"probe-1001","method":"pane.report_agent","params":{"agent":"kimi","state":"blocked","seq":1001,"source":"par-mux:test"}}"#,
    );
    assert!(
        omitted.contains(r#""result":"ok""#),
        "a report without pane_id is accepted: {omitted}"
    );
    stage.control.line_until(
        |line| line.starts_with("%agent-state-changed"),
        "the pane-less report's broadcast",
    );

    // The roster carries %1's latest state and never %0's.
    let roster = stage.control.command("list-agents").join("");
    assert!(
        roster.contains(&format!("{bound} kimi blocked hook")),
        "the bound pane's latest state is rostered: {roster}"
    );
    assert!(
        !roster.contains("%0 kimi"),
        "no report ever landed for %0: {roster}"
    );

    drop(stage.control.0.shutdown(Shutdown::Both));
    sigterm_clean(&mut stage.daemon);
}
