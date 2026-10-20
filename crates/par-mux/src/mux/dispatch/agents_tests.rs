//! agent-start / agent-prompt / agent-explain over live shell panes.
//! Detection is driven by hook reports or the scrape tier run from a test
//! thread (the dispatch harness has no daemon loop); no real agent CLI is
//! ever launched — `-x` substitutes a harmless program.

use super::super::{dispatch_command, Ctx};
use crate::mux::command::parse_command;
use crate::mux::ids::PaneId;
use crate::mux::pane::ShellPaneFactory;
use crate::mux::tree::MuxTree;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `agent-start -T` / `agent-prompt -T` for a start or turn the test DRIVES
/// to completion: a hang ceiling, not a latency budget (the 10 s these
/// used timed out at host load ~200 before the 3 s settle and the scrape
/// tick could confirm the start).
const DRIVEN_MS: u64 = crate::mux::test_wait::CEILING.as_secs() * 1000;

struct Fixture {
    tree: Arc<Mutex<MuxTree>>,
    clients: crate::mux::server::Clients,
}

const PANE: PaneId = PaneId(0);

impl Fixture {
    /// A tree with one live shell pane.
    fn new() -> Self {
        let f = Self {
            tree: Arc::new(Mutex::new(MuxTree::new(Box::new(
                ShellPaneFactory::default(),
            )))),
            clients: Arc::new(Mutex::new(Vec::new())),
        };
        let reply = f.run("new-session -s agents");
        assert!(reply.contains("%end"), "{reply}");
        f
    }

    fn run(&self, line: &str) -> String {
        let ctx = Ctx {
            tree: &self.tree,
            clients: &self.clients,
            command_number: 1,
            shutdown: None,
            config: None,
            client_id: None,
        };
        dispatch_command(parse_command(line).unwrap(), &ctx, None, None)
    }

    fn report(&self, agent: &str, state: &str, seq: u64) {
        let (reply, _) = crate::mux::hooks::handle_report(
            &format!(
                r#"{{"id":1,"method":"pane.report_agent","params":{{"pane_id":"{PANE}","agent":"{agent}","state":"{state}","seq":{seq},"source":"test:hook"}}}}"#
            ),
            &self.tree,
        );
        assert!(reply.contains("\"ok\""), "{reply}");
    }

    fn metadata(&self, key: &str) -> Option<String> {
        self.tree
            .lock()
            .pane(PANE)
            .unwrap()
            .metadata()
            .get(key)
            .cloned()
    }

    fn activity(&self) -> u64 {
        self.tree.lock().pane(PANE).unwrap().agent_activity
    }

    /// Run the scrape tier every 100 ms until `stop` — the daemon's
    /// heartbeat, which the dispatch harness has no loop for.
    fn scrape_until(&self, stop: &AtomicBool) {
        let engine = crate::mux::scrape::ScrapeEngine::bundled();
        while !stop.load(Ordering::Relaxed) {
            crate::mux::scrape::scrape_tick_with(&self.tree, &engine, None);
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// [`Self::scrape_until`] with the real process table, which the idle
    /// fallback needs to prove the agent runs in the pane.
    fn scrape_live_until(&self, stop: &AtomicBool) {
        let engine = crate::mux::scrape::ScrapeEngine::bundled();
        while !stop.load(Ordering::Relaxed) {
            let table = crate::mux::foreground::ProcessTable::snapshot();
            crate::mux::scrape::scrape_tick_with(&self.tree, &engine, table.as_ref());
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Block until the pane's capture shows a line equal to `want`.
    fn wait_for_line(&self, want: &str) {
        let deadline = Instant::now() + crate::mux::test_wait::CEILING;
        loop {
            let screen = self.run(&format!("capture-pane -t {PANE}"));
            if screen.lines().any(|l| l.trim() == want) {
                return;
            }
            assert!(Instant::now() < deadline, "never saw {want:?}:\n{screen}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The shell reached its prompt and runs typed lines.
    fn wait_for_shell(&self) {
        let reply = self.run(&format!("send-keys -t {PANE} 'echo SHELL\"\"-UP' Enter"));
        assert!(reply.contains("%end"), "{reply}");
        self.wait_for_line("SHELL-UP");
    }

    /// Block until the in-flight `start` has tagged the pane `kind`.
    ///
    /// agent-start tags only after its shell-readiness gate: a process-table
    /// snapshot (1.7 s cold at host load ~400) re-checked for up to
    /// `SHELL_READY_RETRY` while a non-shell runs in the foreground. A hook
    /// report filed before the tag claims the pane itself, and the start
    /// refuses with "already hosts agent" — so a test drives its reports
    /// from the tag, never from a fixed sleep. A start that replies before
    /// tagging fails here with its reply instead of waiting out the ceiling.
    fn tagged<'s>(
        &self,
        kind: &str,
        start: std::thread::ScopedJoinHandle<'s, String>,
    ) -> std::thread::ScopedJoinHandle<'s, String> {
        let deadline = Instant::now() + crate::mux::test_wait::CEILING;
        while self.metadata("agent").as_deref() != Some(kind) {
            if start.is_finished() {
                panic!(
                    "agent-start replied before tagging {kind}: {}",
                    start.join().unwrap()
                );
            }
            assert!(Instant::now() < deadline, "agent-start never tagged {kind}");
            std::thread::sleep(crate::mux::test_wait::POLL);
        }
        start
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(pane) = self.tree.lock().pane_mut(PANE) {
            let _ = pane.kill();
        }
    }
}

fn body(reply: &str) -> (Vec<String>, bool) {
    let lines: Vec<&str> = reply.lines().collect();
    let ok = lines.last().is_some_and(|l| l.starts_with("%end"));
    (
        lines[1..lines.len() - 1]
            .iter()
            .map(|l| (*l).to_string())
            .collect(),
        ok,
    )
}

/// AC1: agent-start types the program into the ready pane, tags the label
/// at once, and replies only after detection — here a hook claim filed
/// 600 ms in. The reply names the detected state and its source.
#[test]
fn agent_start_replies_only_after_a_hook_claim() {
    let f = Fixture::new();
    f.wait_for_shell();
    std::thread::scope(|scope| {
        let started = Instant::now();
        let start = scope.spawn(|| {
            f.run(&format!(
                "agent-start -t %0 -T {DRIVEN_MS} -x cat testagent"
            ))
        });
        let start = f.tagged("testagent", start);
        std::thread::sleep(Duration::from_millis(600));
        assert!(!start.is_finished(), "no reply before detection");
        assert_eq!(f.metadata("agent").as_deref(), Some("testagent"));
        f.report("testagent", "idle", 1);
        let reply = start.join().unwrap();
        let (lines, ok) = body(&reply);
        assert!(ok, "{reply}");
        assert_eq!(lines, vec!["%0 testagent idle hook"]);
        assert!(started.elapsed() >= Duration::from_millis(600));
    });
}

/// A `working` startup (a spinner) is not readiness, as in herdr: the start
/// keeps waiting until the agent settles to idle, and a deadline hit while
/// still working says so.
#[test]
fn agent_start_waits_through_a_working_startup() {
    let f = Fixture::new();
    f.wait_for_shell();
    std::thread::scope(|scope| {
        let start = scope.spawn(|| {
            f.run(&format!(
                "agent-start -t %0 -T {DRIVEN_MS} -x cat testagent"
            ))
        });
        let start = f.tagged("testagent", start);
        f.report("testagent", "working", 1);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!start.is_finished(), "working is not ready");
        f.report("testagent", "idle", 2);
        let (lines, ok) = body(&start.join().unwrap());
        assert!(ok, "{lines:?}");
        // working -> idle with nothing viewing the pane is stored as done.
        assert_eq!(lines, vec!["%0 testagent done hook"]);
    });
}

/// -T counts from the command, readiness gate included, so it leaves the
/// report seconds of room after the tag.
#[test]
fn agent_start_timeout_names_a_working_agent() {
    let f = Fixture::new();
    f.wait_for_shell();
    std::thread::scope(|scope| {
        let start = scope.spawn(|| f.run("agent-start -t %0 -T 4000 -x cat testagent"));
        let start = f.tagged("testagent", start);
        f.report("testagent", "working", 1);
        let (lines, ok) = body(&start.join().unwrap());
        assert!(!ok);
        assert_eq!(
            lines,
            vec!["agent-start: timed out after 4000ms waiting for testagent on %0 (still working)"]
        );
    });
    assert_eq!(
        f.metadata("agent").as_deref(),
        Some("testagent"),
        "a hook claimed the pane, so the label stays"
    );
}

/// AC1 via the scrape tier: the launched program sets claude's idle OSC
/// title and stays in the foreground (so no shell prompt hook retitles the
/// pane), and the bundled `osc_title_idle` rule confirms the start.
#[test]
fn agent_start_is_confirmed_by_a_scrape_rule() {
    let f = Fixture::new();
    f.wait_for_shell();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| f.scrape_until(&stop));
        let reply = f.run(&format!(
            r#"agent-start -t %0 -T {DRIVEN_MS} -x sh claude -c 'printf "\033]0;\342\234\263 ready\007"; exec sleep 30'"#
        ));
        stop.store(true, Ordering::Relaxed);
        let (lines, ok) = body(&reply);
        assert!(ok, "{reply}");
        assert_eq!(lines, vec!["%0 claude idle scrape"]);
    });
}

/// The stand-in for droid/copilot/cursor: an `sh` whose argv carries the
/// agent's name (`$0`) and which stays alive (the trailing `:` keeps sh
/// from exec'ing `sleep`), drawing nothing a pattern rule matches.
fn idle_fallback_start(kind: &str, timeout_ms: u64) -> String {
    format!("agent-start -t %0 -T {timeout_ms} -x sh {kind} -c 'sleep 30; :' {kind}")
}

/// droid, copilot, and cursor draw no idle marker: their start is
/// confirmed by the idle fallback once the agent's process is in the pane,
/// and not before the start settle.
#[test]
fn agent_start_confirms_droid_copilot_and_cursor_by_the_idle_fallback() {
    for kind in ["droid", "copilot", "cursor"] {
        let f = Fixture::new();
        f.wait_for_shell();
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| f.scrape_live_until(&stop));
            let started = Instant::now();
            let reply = f.run(&idle_fallback_start(kind, DRIVEN_MS));
            stop.store(true, Ordering::Relaxed);
            let (lines, ok) = body(&reply);
            assert!(ok, "{kind}: {reply}");
            assert_eq!(lines, vec![format!("%0 {kind} idle scrape")]);
            let waited = started.elapsed();
            assert!(
                waited >= Duration::from_secs(3),
                "{kind}: settled, {waited:?}"
            );
        });
        assert_eq!(
            f.metadata("agent_state_rule").as_deref(),
            Some(crate::mux::scrape::IDLE_FALLBACK_RULE)
        );
    }
}

/// The fallback needs the agent's process: a program that is not droid
/// never reads as droid idle, so the start times out and untags.
#[test]
fn agent_start_idle_fallback_needs_the_agents_process() {
    let f = Fixture::new();
    f.wait_for_shell();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| f.scrape_live_until(&stop));
        let reply = f.run("agent-start -t %0 -T 1500 -x cat droid");
        stop.store(true, Ordering::Relaxed);
        let (lines, ok) = body(&reply);
        assert!(!ok);
        assert_eq!(
            lines,
            vec!["agent-start: timed out after 1500ms waiting for droid on %0"]
        );
    });
    assert_eq!(f.metadata("agent"), None, "the failed start's tag is gone");
}

/// A deadline inside the settle window cannot be confirmed by the
/// fallback, and the timeout says the agent was seen but still settling.
#[test]
fn agent_start_timeout_inside_the_settle_names_it() {
    let f = Fixture::new();
    f.wait_for_shell();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| f.scrape_live_until(&stop));
        let reply = f.run(&idle_fallback_start("droid", 2000));
        stop.store(true, Ordering::Relaxed);
        let (lines, ok) = body(&reply);
        assert!(!ok);
        assert_eq!(
            lines,
            vec!["agent-start: timed out after 2000ms waiting for droid on %0 (still settling)"]
        );
    });
}

/// No detection inside -T: an error naming the timeout, and the tag
/// agent-start wrote is removed so the label does not linger.
#[test]
fn agent_start_times_out_and_untags() {
    let f = Fixture::new();
    f.wait_for_shell();
    let (lines, ok) = body(&f.run("agent-start -t %0 -T 700 -x cat testagent"));
    assert!(!ok);
    assert_eq!(
        lines,
        vec!["agent-start: timed out after 700ms waiting for testagent on %0"]
    );
    assert_eq!(f.metadata("agent"), None, "the failed start's tag is gone");
}

/// The test-side contract behind [`Fixture::tagged`]: a non-shell in the
/// foreground holds agent-start's readiness gate, so the tag lands late.
/// The fixed-sleep version (report 400 ms in) collided with the start
/// here: "agent-start: pane %0 already hosts agent testagent".
#[test]
fn agent_start_reports_wait_for_the_tag_through_the_readiness_gate() {
    let f = Fixture::new();
    f.wait_for_shell();
    let pid = f.tree.lock().pane(PANE).unwrap().child_pid().unwrap();
    let reply = f.run(&format!("send-keys -t {PANE} 'sleep 1' Enter"));
    assert!(reply.contains("%end"), "{reply}");
    crate::mux::test_wait::until(
        "sleep in the foreground",
        crate::mux::test_wait::CEILING,
        || {
            crate::mux::foreground::ProcessTable::snapshot()
                .and_then(|table| table.foreground_command(pid))
                .filter(|name| name == "sleep")
        },
        String::new,
    );
    std::thread::scope(|scope| {
        let start = scope.spawn(|| {
            f.run(&format!(
                "agent-start -t %0 -T {DRIVEN_MS} -x cat testagent"
            ))
        });
        let start = f.tagged("testagent", start);
        f.report("testagent", "idle", 1);
        let (lines, ok) = body(&start.join().unwrap());
        assert!(ok, "{lines:?}");
        assert_eq!(lines, vec!["%0 testagent idle hook"]);
    });
}

/// A blocked startup is an error, not a ready agent, and a pane already
/// carrying an agent label is refused.
#[test]
fn agent_start_refuses_a_blocked_start_and_a_claimed_pane() {
    let f = Fixture::new();
    f.wait_for_shell();
    std::thread::scope(|scope| {
        let start = scope.spawn(|| {
            f.run(&format!(
                "agent-start -t %0 -T {DRIVEN_MS} -x cat testagent"
            ))
        });
        let start = f.tagged("testagent", start);
        f.report("testagent", "blocked", 1);
        let (lines, ok) = body(&start.join().unwrap());
        assert!(!ok);
        assert!(lines[0].contains("is blocked during startup"), "{lines:?}");
    });
    let (lines, ok) = body(&f.run("agent-start -t %0 -x cat other"));
    assert!(!ok);
    assert_eq!(
        lines,
        vec!["agent-start: pane %0 already hosts agent testagent"]
    );
}

/// AC2: -w blocks through working until the agent settles, and the reply
/// carries the settled state.
#[test]
fn agent_prompt_waits_for_working_then_idle() {
    let f = Fixture::new();
    f.report("testagent", "idle", 1);
    std::thread::scope(|scope| {
        let started = Instant::now();
        let prompt = scope.spawn(|| {
            f.run(&format!(
                "agent-prompt -t %0 -w -T {DRIVEN_MS} -- do the thing"
            ))
        });
        std::thread::sleep(Duration::from_millis(500));
        f.report("testagent", "working", 2);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!prompt.is_finished(), "still working");
        f.report("testagent", "idle", 3);
        let reply = prompt.join().unwrap();
        let (lines, ok) = body(&reply);
        assert!(ok, "{reply}");
        // Nothing views the pane, so the finished turn is `done`.
        assert_eq!(lines, vec!["%0 testagent done hook"]);
        assert!(started.elapsed() >= Duration::from_millis(1000));
    });
}

/// AC2 anti-stall: a prompt nothing reacts to fails as stalled after the
/// effect window instead of reporting the unchanged idle state as a
/// finished turn.
#[test]
fn agent_prompt_reports_a_stall() {
    let f = Fixture::new();
    f.report("testagent", "idle", 1);
    let started = Instant::now();
    let (lines, ok) = body(&f.run("agent-prompt -t %0 -w -T 20000 -- hello"));
    assert!(!ok);
    assert_eq!(
        lines,
        vec![
            "agent-prompt: stalled: no working or blocked state on %0 within 5000ms of the prompt; current state is idle"
        ]
    );
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(5) && waited < Duration::from_secs(10),
        "{waited:?}"
    );
}

/// The seen flip (`done` → `idle`, written when a client views the pane)
/// is not activity: a prompt whose only state change is that flip still
/// reports the stall.
#[test]
fn the_seen_flip_does_not_pass_the_gate() {
    let f = Fixture::new();
    f.report("testagent", "working", 1);
    f.report("testagent", "idle", 2);
    assert_eq!(f.metadata("agent_state").as_deref(), Some("done"));
    std::thread::scope(|scope| {
        let prompt = scope.spawn(|| f.run("agent-prompt -t %0 -w -T 7000 -- hi"));
        std::thread::sleep(Duration::from_millis(300));
        f.tree
            .lock()
            .pane_mut(PANE)
            .unwrap()
            .set_metadata("agent_state", "idle");
        let (lines, ok) = body(&prompt.join().unwrap());
        assert!(!ok);
        assert!(lines[0].starts_with("agent-prompt: stalled"), "{lines:?}");
    });
}

/// A working→idle bounce between two polls still counts as the prompt's
/// effect; -T bounds the whole wait.
#[test]
fn a_fast_bounce_passes_the_gate_and_minus_t_bounds_the_wait() {
    let f = Fixture::new();
    f.report("testagent", "idle", 1);
    std::thread::scope(|scope| {
        let prompt = scope.spawn(|| f.run("agent-prompt -t %0 -w -u done -T 8000 -- go"));
        std::thread::sleep(Duration::from_millis(400));
        f.report("testagent", "working", 2);
        f.report("testagent", "idle", 3);
        let (lines, ok) = body(&prompt.join().unwrap());
        assert!(ok, "{lines:?}");
        assert_eq!(lines, vec!["%0 testagent done hook"]);
    });
    f.report("testagent", "working", 4);
    let started = Instant::now();
    let (lines, ok) = body(&f.run("agent-prompt -t %0 -w -T 600 -- more"));
    assert!(!ok);
    assert_eq!(
        lines,
        vec!["agent-prompt: timed out after 600ms on %0; current state is working"]
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

/// The prompt reaches the pane's PTY, then Enter submits it (the shell
/// runs it as a command line).
#[test]
fn agent_prompt_sends_text_then_enter() {
    let f = Fixture::new();
    f.wait_for_shell();
    f.report("testagent", "idle", 1);
    let (lines, ok) = body(&f.run(r#"agent-prompt -t %0 echo PROMPT""-LANDED"#));
    assert!(ok, "{lines:?}");
    f.wait_for_line("PROMPT-LANDED");
}

/// Refusals: no agent, a blocked agent.
#[test]
fn agent_prompt_refuses_unclaimed_and_blocked_panes() {
    let f = Fixture::new();
    let (lines, ok) = body(&f.run("agent-prompt -t %0 hi"));
    assert!(!ok);
    assert_eq!(lines, vec!["agent-prompt: pane %0 has no agent"]);
    f.report("testagent", "blocked", 1);
    let (lines, ok) = body(&f.run("agent-prompt -t %0 hi"));
    assert!(!ok);
    assert_eq!(
        lines,
        vec!["agent-prompt: testagent on %0 is blocked and requires interactive input"]
    );
}

/// AC3: a hook-authoritative pane explains as `hook` with its reporter and
/// says the scrape tier is skipped; a scraped pane names the stored rule,
/// the rule matching the live screen, and the bundled pattern file.
#[test]
fn agent_explain_names_hook_and_scrape_sources() {
    let f = Fixture::new();
    let (lines, ok) = body(&f.run("agent-explain -t %0"));
    assert!(!ok);
    assert_eq!(lines, vec!["agent-explain: pane %0 has no agent label"]);

    f.report("claude", "working", 1);
    let (lines, ok) = body(&f.run("agent-explain -t %0"));
    assert!(ok);
    let version = lines
        .iter()
        .find(|l| l.starts_with("patterns-version "))
        .cloned()
        .expect("the set's version is named");
    assert_eq!(
        lines,
        vec![
            "pane %0".to_string(),
            "agent claude".to_string(),
            "state working".to_string(),
            "source hook".to_string(),
            "hook-source test:hook".to_string(),
            "hook-seq 1".to_string(),
            "patterns bundled:patterns/claude.toml".to_string(),
            version,
            "scrape skipped hook-authoritative".to_string(),
        ]
    );

    {
        let mut guard = f.tree.lock();
        let pane = guard.pane_mut(PANE).unwrap();
        crate::mux::hooks::clear_agent_claim(pane);
        pane.set_metadata("agent", "claude");
        pane.terminal()
            .write()
            .process("\x1b]0;\u{2733} ready\x07".as_bytes());
    }
    crate::mux::scrape::scrape_tick_with(
        &f.tree,
        &crate::mux::scrape::ScrapeEngine::bundled(),
        None,
    );
    let (lines, ok) = body(&f.run("agent-explain -t %0"));
    assert!(ok, "{lines:?}");
    for want in [
        "state idle",
        "source scrape",
        "rule osc_title_idle",
        "patterns bundled:patterns/claude.toml",
        "live-rule osc_title_idle state=idle priority=250 region=osc_title",
    ] {
        assert!(lines.iter().any(|l| l == want), "{want}: {lines:?}");
    }
    assert!(
        lines.iter().any(|l| l.starts_with("rules-evaluated ")),
        "{lines:?}"
    );
}

/// The activity counter moves only on entries into working/blocked.
#[test]
fn agent_activity_counts_only_working_and_blocked_entries() {
    let f = Fixture::new();
    f.report("testagent", "idle", 1);
    assert_eq!(f.activity(), 0);
    f.report("testagent", "working", 2);
    f.report("testagent", "working", 3);
    assert_eq!(f.activity(), 1, "a repeat is not a new entry");
    f.report("testagent", "blocked", 4);
    f.report("testagent", "idle", 5);
    assert_eq!(f.activity(), 2);
}
