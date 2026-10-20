//! Agent lifecycle handlers: agent-start, agent-prompt, agent-explain
//! (herdr `agent start` / `agent prompt --wait` / `agent explain`).
//!
//! The two blocking verbs park only the issuing connection's thread, the
//! `wait-output` discipline: agent state lives in pane metadata (hook
//! reports and scrape ticks write it without PTY output), so each poll
//! takes the tree lock briefly and no lock is held while sleeping or while
//! writing to the PTY.

use super::*;
use crate::mux::command::AGENT_START_DEFAULT_TIMEOUT_MS;
use crate::mux::scrape::{PaneSnapshot, ScrapeEngine};
use std::time::{Duration, Instant};

/// How often a blocked agent verb re-reads the pane's claim.
const AGENT_POLL: Duration = Duration::from_millis(100);

/// How long `agent-start` re-checks a pane whose foreground is not a shell
/// (herdr's `PANE_SHELL_READINESS_RETRY_TIMEOUT`).
const SHELL_READY_RETRY: Duration = Duration::from_secs(2);

/// How long after typing the command `agent-start` refuses a scrape `idle`
/// that only the idle fallback produced: a TUI still drawing its first
/// frame matches no rule either (herdr's `AGENT_START_SETTLE_DELAY`).
const START_SETTLE: Duration = Duration::from_secs(3);

/// The gap between a prompt's text and its Enter, so a TUI that reads a
/// fast paste as typing commits the text before the submit key (herdr's
/// `AGENT_PROMPT_SUBMIT_DELAY`).
const PROMPT_SUBMIT_DELAY: Duration = Duration::from_millis(300);

/// The anti-stall window: a prompt that produces no `working` or `blocked`
/// state within this long is reported stalled instead of waited on
/// (herdr's `AGENT_PROMPT_EFFECT_TIMEOUT_MS`).
pub(super) const PROMPT_EFFECT_TIMEOUT: Duration = Duration::from_millis(5_000);

/// The states `agent-prompt -w` ends on when `-u` names none.
const DEFAULT_PROMPT_UNTIL: &[&str] = &["idle", "done", "blocked"];

/// The agent group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_agent_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::AgentStart {
            pane,
            kind,
            program,
            args,
            timeout_ms,
        } => cmd_agent_start(ctx, pane, &kind, program, &args, timeout_ms),
        MuxCommand::AgentPrompt {
            pane,
            text,
            wait,
            until,
            timeout_ms,
        } => cmd_agent_prompt(ctx, pane, &text, wait, &until, timeout_ms),
        MuxCommand::AgentExplain { pane } => cmd_agent_explain(ctx, pane),
        other => unreachable!("route_command sent {other:?} to the agent group"),
    }
}

/// The engine the daemon's scrape tick runs, or the bundled set.
fn engine_of(tree: &MuxTree) -> Arc<ScrapeEngine> {
    tree.scrape_engine
        .clone()
        .unwrap_or_else(ScrapeEngine::bundled)
}

/// Whether a claim's label names the requested kind: equal, or both keys
/// of one pattern set (`claude` / `claude-code`).
fn same_agent(engine: &ScrapeEngine, kind: &str, label: &str) -> bool {
    kind == label
        || matches!(
            (engine.set_for(kind), engine.set_for(label)),
            (Some(a), Some(b)) if a.origin() == b.origin()
        )
}

/// Whether a process name is an interactive shell (herdr's
/// `is_pane_shell_process_name`, with the login-shell `-` stripped).
fn is_shell_name(name: &str) -> bool {
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_start_matches('-')
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "sh" | "bash"
            | "dash"
            | "zsh"
            | "fish"
            | "ksh"
            | "mksh"
            | "csh"
            | "tcsh"
            | "elvish"
            | "xonsh"
            | "nu"
            | "pwsh"
            | "powershell"
            | "cmd"
    )
}

/// One word quoted for the pane's shell. POSIX single quotes on Unix; on
/// Windows (cmd/PowerShell) double quotes — `cmd_agent_start` refuses a
/// word carrying `"` there, since no escape is portable across both.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        return word.to_string();
    }
    if cfg!(windows) {
        format!("\"{word}\"")
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The checks every parked agent verb repeats between polls: daemon
/// shutdown and the issuing client hanging up.
fn interrupted(ctx: &Ctx<'_>, verb: &str) -> Option<Outcome> {
    if ctx
        .shutdown
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
    {
        return Some(Outcome::err(
            ctx,
            &format!("{verb}: the daemon is shutting down"),
        ));
    }
    if ctx
        .client_id
        .is_some_and(|id| !ctx.clients.lock().iter().any(|(c, ..)| *c == id))
    {
        return Some(Outcome::err(ctx, &format!("{verb}: client disconnected")));
    }
    None
}

/// What a poll read about the pane's agent under one brief tree lock.
enum Observed {
    /// The pane is gone.
    Closed,
    /// The pane's process exited.
    Exited,
    /// The pane is live; its claim's label, state, source, and scrape rule
    /// (any may be absent), and its activity counter.
    Live {
        agent: Option<String>,
        state: Option<String>,
        source: Option<String>,
        rule: Option<String>,
        activity: u64,
    },
}

fn observe(tree: &Arc<Mutex<MuxTree>>, pane: PaneId) -> Observed {
    let guard = tree.lock();
    let Some(target) = guard.pane(pane) else {
        return Observed::Closed;
    };
    if !target.is_running() || target.dead() {
        return Observed::Exited;
    }
    let get = |key: &str| target.metadata().get(key).cloned();
    Observed::Live {
        agent: get("agent"),
        state: get("agent_state"),
        source: get("agent_state_source"),
        rule: get("agent_state_rule"),
        activity: target.agent_activity,
    }
}

/// `agent-start`: type the agent's command into a pane sitting at its
/// shell prompt, tag the pane's agent label so the scrape tier watches it,
/// and reply once a hook claim or a scrape rule reports the agent ready
/// (`idle` or `done`) under that label. A start that ends without
/// detection removes the tag again, unless something else claimed the
/// pane meanwhile.
pub(super) fn cmd_agent_start(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    kind: &str,
    program: Option<String>,
    args: &[String],
    timeout_ms: Option<u64>,
) -> Outcome {
    if cfg!(windows)
        && std::iter::once(program.as_deref().unwrap_or(kind))
            .chain(args.iter().map(String::as_str))
            .any(|word| word.contains('"'))
    {
        return Outcome::err(
            ctx,
            "agent-start: arguments must not contain '\"' on Windows",
        );
    }
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(AGENT_START_DEFAULT_TIMEOUT_MS));
    let deadline = Instant::now() + timeout;
    let (pane, child_pid, input, engine) = {
        let guard = ctx.tree.lock();
        let id = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let Some(target) = guard.pane(id) else {
            return Outcome::err(ctx, &format!("no such pane: {id}"));
        };
        if !target.is_running() || target.dead() {
            return Outcome::err(ctx, &format!("agent-start: pane {id} has exited"));
        }
        if let Some(agent) = target.metadata().get("agent") {
            return Outcome::err(
                ctx,
                &format!("agent-start: pane {id} already hosts agent {agent}"),
            );
        }
        let input = match target.input_handle() {
            Ok(input) => input,
            Err(err) => return Outcome::err(ctx, &format!("agent-start: {err}")),
        };
        (id, target.child_pid(), input, engine_of(&guard))
    };
    // The process table reads the whole OS process list: off the lock.
    // An unreadable table (Windows) cannot prove the pane busy. A shell
    // still initializing runs short-lived helpers, so a busy answer is
    // re-checked for up to SHELL_READY_RETRY (herdr's retry window).
    let retry_until = Instant::now() + SHELL_READY_RETRY.min(timeout);
    loop {
        let foreground = child_pid.and_then(|pid| {
            crate::mux::foreground::ProcessTable::snapshot()
                .and_then(|table| table.foreground_command(pid))
        });
        let Some(name) = foreground.filter(|name| !is_shell_name(name)) else {
            break;
        };
        if Instant::now() >= retry_until {
            return Outcome::err(
                ctx,
                &format!("agent-start: pane {pane} is not at a shell prompt (running {name})"),
            );
        }
        std::thread::sleep(AGENT_POLL);
    }
    // Tag under the lock, re-checking the pane did not change hands.
    {
        let mut guard = ctx.tree.lock();
        let Some(target) = guard.pane_mut(pane) else {
            return Outcome::err(ctx, &format!("agent-start: pane {pane} closed"));
        };
        if let Some(agent) = target.metadata().get("agent") {
            return Outcome::err(
                ctx,
                &format!("agent-start: pane {pane} already hosts agent {agent}"),
            );
        }
        target.set_metadata("agent", kind);
    }
    let mut line = std::iter::once(program.as_deref().unwrap_or(kind))
        .chain(args.iter().map(String::as_str))
        .map(shell_word)
        .collect::<Vec<_>>()
        .join(" ");
    line.push('\r');
    if let Err(err) = input.write(line.as_bytes()) {
        untag_undetected(ctx.tree, pane, kind);
        return Outcome::err(ctx, &format!("agent-start: {err}"));
    }
    let typed_at = Instant::now();
    let mut still_working = false;
    let mut settling = false;
    loop {
        match observe(ctx.tree, pane) {
            Observed::Closed => {
                return Outcome::err(ctx, &format!("agent-start: pane {pane} closed"));
            }
            Observed::Exited => {
                untag_undetected(ctx.tree, pane, kind);
                return Outcome::err(
                    ctx,
                    &format!("agent-start: pane {pane} exited before {kind} was detected"),
                );
            }
            Observed::Live {
                agent,
                state,
                source,
                rule,
                ..
            } => match (agent, state) {
                (None, _) => {
                    return Outcome::err(
                        ctx,
                        &format!("agent-start: {kind} released pane {pane} during startup"),
                    );
                }
                (Some(agent), _) if !same_agent(&engine, kind, &agent) => {
                    return Outcome::err(
                        ctx,
                        &format!("agent-start: expected {kind}, detected {agent} on {pane}"),
                    );
                }
                (Some(_), Some(state)) if state == "blocked" => {
                    return Outcome::err(
                        ctx,
                        &format!(
                            "agent-start: {kind} on {pane} is blocked during startup and is not ready for prompts"
                        ),
                    );
                }
                // herdr parity: `working` is startup still running (a
                // spinner), not readiness — replying then would let the
                // next `agent-prompt -w` mistake startup's end for its turn.
                (Some(_), Some(state)) if state == "working" => still_working = true,
                (Some(_), Some(_))
                    if source.as_deref() == Some("scrape")
                        && rule.as_deref() == Some(crate::mux::scrape::IDLE_FALLBACK_RULE)
                        && typed_at.elapsed() < START_SETTLE =>
                {
                    settling = true;
                }
                (Some(agent), Some(state)) => {
                    let source = source.unwrap_or_else(|| "hook".to_string());
                    return Outcome::ok(ctx, &format!("{pane} {agent} {state} {source}"));
                }
                (Some(_), None) => {}
            },
        }
        if Instant::now() >= deadline {
            untag_undetected(ctx.tree, pane, kind);
            let still = if still_working {
                " (still working)"
            } else if settling {
                " (still settling)"
            } else {
                ""
            };
            return Outcome::err(
                ctx,
                &format!(
                    "agent-start: timed out after {}ms waiting for {kind} on {pane}{still}",
                    timeout.as_millis()
                ),
            );
        }
        if let Some(outcome) = interrupted(ctx, "agent-start") {
            untag_undetected(ctx.tree, pane, kind);
            return outcome;
        }
        std::thread::sleep(AGENT_POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

/// Remove the `agent` tag `agent-start` wrote when no hook or scrape ever
/// claimed the pane under it, so a failed start does not linger in the
/// persisted label set.
fn untag_undetected(tree: &Arc<Mutex<MuxTree>>, pane: PaneId, kind: &str) {
    let mut guard = tree.lock();
    if let Some(target) = guard.pane_mut(pane) {
        let metadata = target.metadata();
        if metadata.get("agent").map(String::as_str) == Some(kind)
            && !metadata.contains_key("agent_state")
            && !metadata.contains_key("agent_seq")
        {
            crate::mux::hooks::clear_agent_claim(target);
        }
    }
}

/// `agent-prompt`: submit `text` to the pane's agent (a bracketed paste
/// when the agent enabled it, then Enter after a short gap), and with
/// `-w` block until the agent reaches an `until` state. The anti-stall
/// gate first requires evidence the prompt landed — an entry into
/// `working` or `blocked` after submission, or an agent already working —
/// within [`PROMPT_EFFECT_TIMEOUT`]; a prompt nothing reacted to is
/// reported stalled instead of being mistaken for a finished turn.
pub(super) fn cmd_agent_prompt(
    ctx: &Ctx<'_>,
    pane: Target<PaneId>,
    text: &str,
    wait: bool,
    until: &[String],
    timeout_ms: Option<u64>,
) -> Outcome {
    let started = Instant::now();
    let deadline = timeout_ms.and_then(|ms| started.checked_add(Duration::from_millis(ms)));
    let (pane, agent, prior_state, activity, input, paste, focus) = {
        let guard = ctx.tree.lock();
        let id = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let Some(target) = guard.pane(id) else {
            return Outcome::err(ctx, &format!("no such pane: {id}"));
        };
        if !target.is_running() || target.dead() {
            return Outcome::err(ctx, &format!("agent-prompt: pane {id} has exited"));
        }
        let metadata = target.metadata();
        let Some(agent) = metadata.get("agent").cloned() else {
            return Outcome::err(ctx, &format!("agent-prompt: pane {id} has no agent"));
        };
        let state = metadata.get("agent_state").cloned();
        if state.as_deref() == Some("blocked") {
            return Outcome::err(
                ctx,
                &format!("agent-prompt: {agent} on {id} is blocked and requires interactive input"),
            );
        }
        let input = match target.input_handle() {
            Ok(input) => input,
            Err(err) => return Outcome::err(ctx, &format!("agent-prompt: {err}")),
        };
        let terminal = target.terminal();
        let term = terminal.read();
        // Copilot ignores a synthetic Enter after a focus loss until it
        // sees focus gained (herdr's submit quirk).
        let focus = if agent == "copilot" {
            term.report_focus_in()
        } else {
            Vec::new()
        };
        (
            id,
            agent,
            state,
            target.agent_activity,
            input,
            term.bracketed_paste(),
            focus,
        )
    };
    let mut body = focus;
    if paste {
        // The paste end marker inside the text would end the paste early
        // and let the rest arrive as typed keys.
        body.extend_from_slice(b"\x1b[200~");
        body.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
        body.extend_from_slice(b"\x1b[201~");
    } else {
        body.extend_from_slice(text.as_bytes());
    }
    let written = input.write(&body).and_then(|()| {
        std::thread::sleep(PROMPT_SUBMIT_DELAY);
        input.write(b"\r")
    });
    if let Err(err) = written {
        return Outcome::err(ctx, &format!("agent-prompt: {err}"));
    }
    if !wait {
        return Outcome::ok(ctx, "");
    }
    let until: Vec<&str> = if until.is_empty() {
        DEFAULT_PROMPT_UNTIL.to_vec()
    } else {
        until.iter().map(String::as_str).collect()
    };
    // The gate: a working agent already proves the prompt has a turn to
    // join; otherwise wait for a fresh entry into working/blocked.
    let mut gated = prior_state.as_deref() == Some("working");
    let gate_deadline = started + PROMPT_EFFECT_TIMEOUT;
    loop {
        let (state, source, now_activity) = match observe(ctx.tree, pane) {
            Observed::Closed => {
                return Outcome::err(ctx, &format!("agent-prompt: pane {pane} closed"));
            }
            Observed::Exited => {
                return Outcome::err(
                    ctx,
                    &format!("agent-prompt: pane {pane} exited while waiting"),
                );
            }
            Observed::Live {
                agent: current,
                state,
                source,
                activity: now,
                ..
            } => {
                if current.as_deref() != Some(agent.as_str()) {
                    return Outcome::err(
                        ctx,
                        &format!("agent-prompt: {agent} is no longer running in pane {pane}"),
                    );
                }
                (state, source, now)
            }
        };
        gated |= now_activity != activity;
        let now = Instant::now();
        if gated {
            if let Some(state) = state.as_deref().filter(|state| until.contains(state)) {
                let source = source.as_deref().unwrap_or("hook");
                return Outcome::ok(ctx, &format!("{pane} {agent} {state} {source}"));
            }
        } else if now >= gate_deadline && deadline.is_none_or(|d| d > gate_deadline) {
            return Outcome::err(
                ctx,
                &format!(
                    "agent-prompt: stalled: no working or blocked state on {pane} within {}ms of the prompt; current state is {}",
                    PROMPT_EFFECT_TIMEOUT.as_millis(),
                    state.as_deref().unwrap_or("unknown")
                ),
            );
        }
        if deadline.is_some_and(|d| now >= d) {
            return Outcome::err(
                ctx,
                &format!(
                    "agent-prompt: timed out after {}ms on {pane}; current state is {}",
                    timeout_ms.unwrap_or_default(),
                    state.as_deref().unwrap_or("unknown")
                ),
            );
        }
        if let Some(outcome) = interrupted(ctx, "agent-prompt") {
            return outcome;
        }
        let park = deadline.map_or(AGENT_POLL, |d| {
            AGENT_POLL.min(d.saturating_duration_since(now))
        });
        std::thread::sleep(park);
    }
}

/// `agent-explain`: how the pane's agent state is detected — `key value`
/// lines naming the stored state and its source (`hook` claim, `scrape`
/// rule, or `none`), the claim's reporting script for a hook, and for the
/// scrape tier the rule matching the live screen now with its pattern
/// file. A hook-authoritative pane is never scraped, which the reply says.
pub(super) fn cmd_agent_explain(ctx: &Ctx<'_>, pane: Target<PaneId>) -> Outcome {
    let (pane, metadata, snapshot, engine) = {
        let guard = ctx.tree.lock();
        let id = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => return Outcome::err(ctx, &err.to_string()),
        };
        let Some(target) = guard.pane(id) else {
            return Outcome::err(ctx, &format!("no such pane: {id}"));
        };
        let terminal = target.terminal();
        let snapshot = {
            let term = terminal.read();
            PaneSnapshot {
                title: term.title().to_string(),
                screen: term.content(),
            }
        };
        (id, target.metadata().clone(), snapshot, engine_of(&guard))
    };
    let Some(agent) = metadata.get("agent") else {
        return Outcome::err(
            ctx,
            &format!("agent-explain: pane {pane} has no agent label"),
        );
    };
    let get = |key: &str| metadata.get(key).map(String::as_str);
    let source = get("agent_state_source").unwrap_or(if get("agent_state").is_some() {
        "hook"
    } else {
        "none"
    });
    let mut lines = vec![
        format!("pane {pane}"),
        format!("agent {agent}"),
        format!("state {}", get("agent_state").unwrap_or("unknown")),
        format!("source {source}"),
    ];
    if let Some(reporter) = get("agent_source") {
        lines.push(format!("hook-source {reporter}"));
    }
    if let Some(seq) = get("agent_seq") {
        lines.push(format!("hook-seq {seq}"));
    }
    if source == "scrape" {
        if let Some(rule) = get("agent_state_rule") {
            lines.push(format!("rule {rule}"));
        }
    }
    match engine.set_for(agent) {
        None => lines.push("patterns none".to_string()),
        Some(set) => {
            lines.push(format!("patterns {}", set.origin()));
            lines.push(format!("patterns-version {}", set.version()));
            if source == "hook" {
                lines.push("scrape skipped hook-authoritative".to_string());
            } else {
                let (matched, rules) = set.explain(&snapshot);
                lines.push(format!("rules-evaluated {rules}"));
                match matched {
                    Some(rule) => lines.push(format!(
                        "live-rule {} state={} priority={} region={}",
                        rule.id, rule.state, rule.priority, rule.region
                    )),
                    None => lines.push("live-rule none".to_string()),
                }
            }
        }
    }
    Outcome::ok(ctx, &lines.join("\n"))
}

#[cfg(all(test, unix))]
#[path = "agents_tests.rs"]
mod tests;
