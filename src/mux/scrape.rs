//! Pane scrape: the fallback state tier for agents without state hooks.
//!
//! Phase 5's owner amendment (card `01a0c76308c476918806db673626bf80`):
//! hooks stay primary and authoritative, and a pane whose agent cannot
//! report state through the T5.2 endpoint has its state read from rendered
//! content instead — title and screen text matched against per-agent
//! pattern rules ported from herdr. The honesty property the original D1
//! ruling protected survives mechanically: a scrape that matches no rule
//! produces NO state (a prior scrape's guess is cleared, never coerced to
//! `idle`), and every surfaced value carries `scrape` provenance beside
//! the `hook` provenance of a claim.
//!
//! Patterns ship bundled (`include_str!`, one TOML per agent under
//! `patterns/`) and a local override file shadows the bundled set for its
//! agent — precedence bundled < override, no remote catalog (the T1+T3
//! ruling). An override that fails to parse or validate falls back to
//! bundled with a warning, never silently disabling the agent.

use crate::mux::foreground::{Liveness, ProcessTable};
use crate::mux::hooks::AGENT_CLAIM_KEYS;
use crate::mux::ids::PaneId;
use crate::mux::pane::MuxPane;
use crate::mux::tree::MuxTree;
use crate::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The bundled pattern sets, one per scrape-supported agent. Ownership of
/// the claude/codex/grok scope is the card's settled ruling; pi and omp
/// need none (they report state through hooks).
const BUNDLED_PATTERNS: &[(&str, &str)] = &[
    ("claude", include_str!("patterns/claude.toml")),
    ("codex", include_str!("patterns/codex.toml")),
    ("grok", include_str!("patterns/grok.toml")),
];

/// The states a rule may assert. `unknown` is not among them: it is the
/// absence of a match, produced by no rule at all.
const VALID_STATES: &[&str] = &["working", "blocked", "idle"];

/// The metadata keys the scrape tier writes — and clears together when a
/// pattern stops matching.
const SCRAPE_KEYS: &[&str] = &["agent_state", "agent_state_source", "agent_state_rule"];

/// One agent's pattern file, exactly as written. `updated_at` stays in the
/// TOML as human-readable audit data; only `version` is surfaced (in
/// shadow logs), so only that is deserialized.
#[derive(Debug, Deserialize)]
struct PatternSet {
    id: String,
    version: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    rules: Vec<Rule>,
}

/// One rule: a state, a priority, a region, and a condition tree.
#[derive(Debug, Deserialize)]
struct Rule {
    id: String,
    state: String,
    priority: i64,
    region: String,
    #[serde(flatten)]
    cond: Cond,
}

/// A condition tree — herdr's matcher vocabulary with the AND/OR structure
/// made explicit: present clauses AND together, every list is ALL
/// (`contains`, `regex`, `line_regex`), and the group clauses compose.
#[derive(Debug, Default, Deserialize)]
struct Cond {
    /// ALL of these substrings must appear in the region text.
    #[serde(default)]
    contains: Vec<String>,
    /// ALL of these regexes must match the region text (multiline allowed).
    #[serde(default)]
    regex: Vec<String>,
    /// ALL of these regexes must match, each at least one line of the region.
    #[serde(default)]
    line_regex: Vec<String>,
    /// ANY sub-condition matches.
    #[serde(default)]
    any: Vec<Cond>,
    /// ALL sub-conditions match.
    #[serde(default)]
    all: Vec<Cond>,
    /// NO sub-condition matches.
    #[serde(default)]
    not: Vec<Cond>,
}

/// A compiled condition tree.
#[derive(Clone)]
struct CompiledCond {
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
    any: Vec<CompiledCond>,
    all: Vec<CompiledCond>,
    not: Vec<CompiledCond>,
}

/// A compiled rule, its region parsed and regexes built.
#[derive(Clone)]
struct CompiledRule {
    id: String,
    state: String,
    priority: i64,
    region: Region,
    cond: CompiledCond,
}

/// One agent's compiled rules, ordered highest priority first (file order
/// breaks ties, matching herdr). Cloned per alias key so lookups stay
/// borrows. `version` rides along so shadow logging can name what was
/// shadowed — a stale bundled set is auditable from the daemon log.
#[derive(Clone)]
pub struct CompiledSet {
    rules: Vec<CompiledRule>,
    version: String,
}

/// The regions a v1 rule can look at. herdr's engine-v3 regions (prompt
/// markers, horizontal rules, the prompt box) are deliberately absent —
/// rules needing them were not ported.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Region {
    /// The pane terminal's OSC 0/2 title.
    OscTitle,
    /// The whole visible screen.
    Whole,
    /// The last N non-empty screen lines.
    BottomNonEmptyLines(usize),
    /// The first N non-empty screen lines.
    TopNonEmptyLines(usize),
}

/// Parse a region as written in a pattern file.
fn parse_region(raw: &str) -> Result<Region, String> {
    if raw == "osc_title" {
        return Ok(Region::OscTitle);
    }
    if raw == "whole" {
        return Ok(Region::Whole);
    }
    for (prefix, bottom) in [
        ("bottom_non_empty_lines(", true),
        ("top_non_empty_lines(", false),
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            let count = rest
                .strip_suffix(')')
                .ok_or_else(|| format!("unterminated region `{raw}`"))?;
            let count: usize = count
                .parse()
                .map_err(|_| format!("bad region count in `{raw}`"))?;
            return Ok(if bottom {
                Region::BottomNonEmptyLines(count)
            } else {
                Region::TopNonEmptyLines(count)
            });
        }
    }
    Err(format!("unknown region `{raw}`"))
}

impl CompiledCond {
    /// Compile one condition tree, building every regex up front so a
    /// malformed pattern fails at load, never mid-scrape.
    fn compile(cond: &Cond) -> Result<CompiledCond, String> {
        let build = |patterns: &[String]| -> Result<Vec<Regex>, String> {
            patterns
                .iter()
                .map(|pattern| {
                    Regex::new(pattern).map_err(|err| format!("bad regex `{pattern}`: {err}"))
                })
                .collect()
        };
        let compiled = CompiledCond {
            // herdr parity: contains needles lowercase at compile and the
            // region text lowercases at match (herdr detect/manifest.rs
            // compile_gate/compiled_rule_matches), so a real-cased agent
            // screen ("Do you want to proceed?") hits rules authored in
            // lowercase. regex/line_regex stay case-sensitive as written.
            contains: cond
                .contains
                .iter()
                .map(|needle| needle.to_lowercase())
                .collect(),
            regex: build(&cond.regex)?,
            line_regex: build(&cond.line_regex)?,
            any: cond
                .any
                .iter()
                .map(CompiledCond::compile)
                .collect::<Result<Vec<_>, String>>()?,
            all: cond
                .all
                .iter()
                .map(CompiledCond::compile)
                .collect::<Result<Vec<_>, String>>()?,
            not: cond
                .not
                .iter()
                .map(CompiledCond::compile)
                .collect::<Result<Vec<_>, String>>()?,
        };
        if compiled.is_vacuous() {
            return Err("condition matches everything (no clause present)".to_string());
        }
        Ok(compiled)
    }

    /// A condition with every clause empty would match anything; load
    /// rejects it rather than shipping an always-true rule.
    fn is_vacuous(&self) -> bool {
        self.contains.is_empty()
            && self.regex.is_empty()
            && self.line_regex.is_empty()
            && self.any.is_empty()
            && self.all.is_empty()
            && self.not.is_empty()
    }

    /// Whether this condition holds for a region. Absent clauses do not
    /// constrain: an empty `regex` list is not "nothing matched", it is
    /// "no regex clause" — the difference between AND-of-present-clauses
    /// and a chain of vacuous-any failures.
    fn matches(&self, text: &str, lines: &[&str]) -> bool {
        if !self.contains.is_empty() {
            let lower_text = text.to_lowercase();
            if !self.contains.iter().all(|n| lower_text.contains(n)) {
                return false;
            }
        }
        if !self.regex.is_empty() && !self.regex.iter().all(|re| re.is_match(text)) {
            return false;
        }
        if !self.line_regex.is_empty()
            && !self
                .line_regex
                .iter()
                .all(|re| lines.iter().any(|line| re.is_match(line)))
        {
            return false;
        }
        if !self.any.is_empty() && !self.any.iter().any(|cond| cond.matches(text, lines)) {
            return false;
        }
        if !self.all.is_empty() && !self.all.iter().all(|cond| cond.matches(text, lines)) {
            return false;
        }
        if !self.not.is_empty() && self.not.iter().any(|cond| cond.matches(text, lines)) {
            return false;
        }
        true
    }
}

/// What one pane looks like to the pattern engine — the only inputs v1
/// regions need, gathered under one read of the pane's terminal.
pub struct PaneSnapshot {
    /// The pane terminal's current OSC 0/2 title.
    pub title: String,
    /// The pane terminal's visible screen, one rtrimmed line per row.
    pub screen: String,
}

/// The region text a rule runs against.
fn region_text<'a>(region: &Region, snapshot: &'a PaneSnapshot) -> std::borrow::Cow<'a, str> {
    match region {
        Region::OscTitle => std::borrow::Cow::Borrowed(&snapshot.title),
        Region::Whole => std::borrow::Cow::Borrowed(&snapshot.screen),
        Region::BottomNonEmptyLines(count) => {
            let non_empty: Vec<&str> = snapshot
                .screen
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            let start = non_empty.len().saturating_sub(*count);
            std::borrow::Cow::Owned(non_empty[start..].join("\n"))
        }
        Region::TopNonEmptyLines(count) => {
            let non_empty: Vec<&str> = snapshot
                .screen
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            std::borrow::Cow::Owned(
                non_empty
                    .iter()
                    .take(*count)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
    }
}

impl CompiledSet {
    /// Compile one pattern file: parse, validate states/regions/regexes,
    /// and order highest priority first with file order breaking ties.
    /// Returns the compiled set plus the id and aliases to key it under.
    fn compile(text: &str) -> Result<(CompiledSet, String, Vec<String>), String> {
        let set: PatternSet =
            toml::from_str(text).map_err(|err| format!("TOML parse failed: {err}"))?;
        let mut rules = Vec::with_capacity(set.rules.len());
        for rule in &set.rules {
            if !VALID_STATES.contains(&rule.state.as_str()) {
                return Err(format!(
                    "rule `{}` has state `{}` (want one of {VALID_STATES:?})",
                    rule.id, rule.state
                ));
            }
            rules.push(CompiledRule {
                id: rule.id.clone(),
                state: rule.state.clone(),
                priority: rule.priority,
                region: parse_region(&rule.region)?,
                cond: CompiledCond::compile(&rule.cond)
                    .map_err(|err| format!("rule `{}`: {err}", rule.id))?,
            });
        }
        // Stable sort: equal priorities keep file order, herdr's tie-break.
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.priority));
        let id = set.id;
        let aliases = set.aliases;
        Ok((
            CompiledSet {
                rules,
                version: set.version,
            },
            id,
            aliases,
        ))
    }

    /// The highest-priority matching rule's state and id, or None when no
    /// rule matches — which IS the `unknown` the honesty rule requires.
    fn evaluate(&self, snapshot: &PaneSnapshot) -> Option<(&str, &str)> {
        for rule in &self.rules {
            let text = region_text(&rule.region, snapshot);
            let lines: Vec<&str> = text.lines().collect();
            if rule.cond.matches(&text, &lines) {
                return Some((rule.state.as_str(), rule.id.as_str()));
            }
        }
        None
    }
}

/// The loaded pattern sets, keyed by agent id and alias.
pub struct ScrapeEngine {
    sets: HashMap<String, CompiledSet>,
}

impl ScrapeEngine {
    /// Load the bundled sets, shadowed per agent by `<override_dir>/<agent>.toml`.
    ///
    /// An override that fails to parse or validate falls back to the
    /// bundled set with a warning on stderr (the daemon's status channel);
    /// a file for an agent with no bundled set adds that agent. `None`
    /// loads bundled only — the in-process shape tests use.
    pub fn load(override_dir: Option<&Path>) -> ScrapeEngine {
        let mut engine = ScrapeEngine {
            sets: HashMap::new(),
        };
        for (agent, text) in BUNDLED_PATTERNS {
            let Ok((mut set, mut id, mut aliases)) = CompiledSet::compile(text) else {
                // A bundled set that fails to compile is a build bug; say
                // so loudly and keep serving the other agents.
                log::warn!("par-mux: bundled `{agent}` patterns failed to compile");
                continue;
            };
            if let Some(dir) = override_dir {
                let path = dir.join(format!("{agent}.toml"));
                if path.exists() {
                    match std::fs::read_to_string(&path) {
                        Ok(text) => match CompiledSet::compile(&text) {
                            Ok((shadow, shadow_id, shadow_aliases)) => {
                                log::warn!(
                                    "par-mux: agent patterns for `{agent}` overridden by {} (v{} shadows bundled v{})",
                                    path.display(),
                                    shadow.version,
                                    set.version
                                );
                                set = shadow;
                                id = shadow_id;
                                aliases = shadow_aliases;
                            }
                            Err(err) => {
                                log::warn!(
                                    "par-mux: override {} failed ({err}); using bundled v{}",
                                    path.display(),
                                    set.version
                                );
                            }
                        },
                        Err(err) => {
                            log::warn!(
                                "par-mux: override {} unreadable ({err}); using bundled",
                                path.display()
                            );
                        }
                    }
                }
            }
            engine.index(set, &id, &aliases);
        }
        // Overrides for agents with no bundled set add them — the same-day
        // escape hatch the T3 tier exists for, not limited to the bundled
        // three.
        if let Some(dir) = override_dir {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => return engine,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(agent) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(agent) = agent.strip_suffix(".toml") else {
                    continue;
                };
                if engine.sets.contains_key(agent) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok((set, id, aliases)) = CompiledSet::compile(&text) {
                        log::warn!(
                            "par-mux: agent patterns for `{agent}` loaded from {} (no bundled set)",
                            path.display()
                        );
                        engine.index(set, &id, &aliases);
                    } else {
                        log::warn!(
                            "par-mux: pattern file {} failed to compile; ignored",
                            path.display()
                        );
                    }
                }
            }
        }
        engine
    }

    /// The compiled set for an agent label (id or alias), if any.
    pub fn set_for(&self, agent: &str) -> Option<&CompiledSet> {
        self.sets.get(agent)
    }

    /// Key one compiled set under its id and every alias.
    fn index(&mut self, set: CompiledSet, id: &str, aliases: &[String]) {
        for alias in aliases {
            self.sets.insert(alias.clone(), set.clone());
        }
        self.sets.insert(id.to_string(), set);
    }
}

/// One heartbeat pass over the tree: scrape every eligible pane, write
/// transitions into metadata, and return the notifications to broadcast.
///
/// The caller owns the broadcast and sends it after this returns — the
/// tree lock is released before any notification leaves, the discipline
/// `hooks.rs` set. Eligibility is claim-based membership plus structural
/// precedence: the pane must carry an `agent` label (hook report or
/// factory tag), must not be hook-authoritative (`agent_state_source` =
/// `hook` — permanent once any hook state report is accepted), and must
/// have patterns for its agent. Hook-authoritative panes are still visited
/// by the liveness sweep: a claim whose agent provably left the pane's
/// process tree is cleared and released (see `foreground.rs`).
pub fn scrape_tick(tree: &Arc<Mutex<MuxTree>>, engine: &ScrapeEngine) -> Vec<TmuxNotification> {
    scrape_tick_with(tree, engine, ProcessTable::snapshot().as_ref())
}

/// Mismatching ticks (not interrupted by a proven match) before a hook
/// claim is cleared. One never clears: the probe runs while a pane's
/// process tree is in ordinary flux (a shell pipeline between execs, an
/// agent restarting itself), and provable absence held across two
/// 1-second ticks is the death signal.
const LIVENESS_MISSES_TO_CLEAR: u8 = 2;

/// The keys the liveness sweep keeps its miss count under. The count
/// belongs to one agent label, so it lives and dies with the claim
/// (`AGENT_CLAIM_KEYS` clears it) and a relabel starts a fresh count.
const LIVENESS_MISS_KEYS: &[&str] = &["agent_liveness_misses", "agent_liveness_misses_agent"];

/// A tick that saw the agent alive drops the miss count: only unbroken
/// mismatches clear a claim. `Unknown` deliberately does NOT reset — a
/// measurement failure (argv unreadable mid-`exec`, the fresh-spawn
/// window) is not liveness, and erasing proof because the probe went
/// blind for a tick would let a dead agent's claim survive alternation.
fn reset_liveness_misses(pane: &mut MuxPane) {
    pane.clear_metadata(LIVENESS_MISS_KEYS);
}

/// Record one mismatching tick and return the count after the increment.
/// A count carried under a different agent label is the previous agent's;
/// restart from one under the current label.
fn bump_liveness_misses(pane: &mut MuxPane, agent: &str) -> u8 {
    let prior = if pane
        .metadata()
        .get("agent_liveness_misses_agent")
        .map(String::as_str)
        == Some(agent)
    {
        pane.metadata()
            .get("agent_liveness_misses")
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let misses = prior.saturating_add(1);
    pane.set_metadata("agent_liveness_misses", &misses.to_string());
    pane.set_metadata("agent_liveness_misses_agent", agent);
    misses
}

/// The tick with the process table injected — the liveness sweep's test
/// seam; [`scrape_tick`] reads the real one.
pub(crate) fn scrape_tick_with(
    tree: &Arc<Mutex<MuxTree>>,
    engine: &ScrapeEngine,
    table: Option<&ProcessTable>,
) -> Vec<TmuxNotification> {
    let mut notifications = Vec::new();
    let mut guard = tree.lock();
    let panes: Vec<PaneId> = guard
        .sessions()
        .iter()
        .filter_map(|session| guard.session(*session))
        .flat_map(|session| session.windows.clone())
        .filter_map(|window| guard.window(window))
        .flat_map(|window| window.panes())
        .collect();

    for pane_id in panes {
        let Some(pane) = guard.pane_mut(pane_id) else {
            continue;
        };
        // A dead pane holds its last state the way a quiet hook does: the
        // frozen screen would only re-derive it, at regex cost.
        if !pane.is_running() {
            continue;
        }
        let Some(agent) = pane.metadata().get("agent").cloned() else {
            continue;
        };
        let prior_source = pane.metadata().get("agent_state_source").cloned();
        if prior_source.as_deref() == Some("hook") {
            // Hook authority is never scraped — but a claim whose agent
            // provably left the pane's process tree is cleared here. A
            // crashed agent's hook never sends `pane.release_agent`; see
            // `foreground.rs` for why the sweep asks the descendant tree,
            // not the foreground process.
            if let Some(table) = table {
                if let Some(child_pid) = pane.child_pid() {
                    match table.agent_alive(child_pid, &agent) {
                        Liveness::Matches => reset_liveness_misses(pane),
                        // Unknown keeps the current count: the probe went
                        // blind, which is neither liveness nor death.
                        Liveness::Unknown => {}
                        Liveness::Mismatch => {
                            if bump_liveness_misses(pane, &agent) >= LIVENESS_MISSES_TO_CLEAR {
                                pane.clear_metadata(AGENT_CLAIM_KEYS);
                                notifications.push(TmuxNotification::AgentReleased {
                                    pane_id: pane_id.to_string(),
                                    agent: agent.clone(),
                                });
                            }
                        }
                    }
                }
            }
            continue;
        }
        let Some(set) = engine.set_for(&agent) else {
            continue;
        };
        let snapshot = {
            let terminal = pane.terminal();
            let term = terminal.read();
            PaneSnapshot {
                title: term.title().to_string(),
                screen: term.content(),
            }
        };
        match set.evaluate(&snapshot) {
            Some((state, rule)) => {
                let unchanged = prior_source.as_deref() == Some("scrape")
                    && pane.metadata().get("agent_state").map(String::as_str) == Some(state);
                if !unchanged {
                    pane.set_metadata("agent_state", state);
                    pane.set_metadata("agent_state_source", "scrape");
                    pane.set_metadata("agent_state_rule", rule);
                    notifications.push(TmuxNotification::AgentStateChanged {
                        pane_id: pane_id.to_string(),
                        agent,
                        state: state.to_string(),
                        source: "scrape".to_string(),
                    });
                }
            }
            None => {
                // No match is unknown, never idle: clear an earlier guess,
                // keep the push channel claim-only.
                if prior_source.as_deref() == Some("scrape") {
                    pane.clear_metadata(SCRAPE_KEYS);
                }
            }
        }
    }
    notifications
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::pane::ShellPaneFactory;

    /// A minimal pattern set with one title rule, the shape override
    /// tests need.
    const OVERRIDE_SET: &str = r#"
id = "claude"
version = "test"
updated_at = "2026-09-22T00:00:00Z"

[[rules]]
id = "override_idle"
state = "idle"
priority = 100
region = "osc_title"
contains = ["Override Idle"]
"#;

    fn tree_with_pane() -> (Arc<Mutex<MuxTree>>, PaneId) {
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
        let session = tree.new_session("scrape", 80, 24).expect("session spawns");
        let pane_id = tree
            .session(session)
            .expect("session exists")
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .expect("a new session has a pane");
        (Arc::new(Mutex::new(tree)), pane_id)
    }

    /// Feed bytes into the pane's terminal the way the PTY reader would.
    fn feed(tree: &Arc<Mutex<MuxTree>>, pane_id: PaneId, bytes: &[u8]) {
        let terminal = tree.lock().pane(pane_id).expect("pane exists").terminal();
        terminal.write().process(bytes);
    }

    #[test]
    fn bundled_sets_compile_with_aliases() {
        let engine = ScrapeEngine::load(None);
        for agent in ["claude", "codex", "grok"] {
            assert!(engine.set_for(agent).is_some(), "{agent} bundled");
        }
        assert!(engine.set_for("claude-code").is_some(), "claude alias");
        assert!(engine.set_for("grok-build").is_some(), "grok alias");
        assert!(engine.set_for("pi").is_none(), "pi reports by hook");
        assert!(engine.set_for("omp").is_none(), "omp reports by hook");
    }

    #[test]
    fn claude_title_rules_read_working_idle_and_unknown() {
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("claude").expect("claude bundled");

        // The braille spinner prefix is claude's working title.
        let working = PaneSnapshot {
            title: "⠋ Thinking hard".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&working)
                .map(|(state, rule)| (state.to_string(), rule.to_string())),
            Some(("working".to_string(), "osc_title_working".to_string()))
        );

        // "✳ " is claude's idle title.
        let idle = PaneSnapshot {
            title: "✳ ready".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&idle).map(|(s, _)| s.to_string()),
            Some("idle".to_string())
        );

        // A plain title matches nothing: unknown, by absence.
        let unknown = PaneSnapshot {
            title: "zsh".to_string(),
            screen: String::new(),
        };
        assert_eq!(set.evaluate(&unknown), None, "no match is no state");
    }

    #[test]
    fn claude_blocked_rules_match_mixed_case_prompts() {
        // herdr parity: the ported needles are authored lowercase, but real
        // claude chrome renders mixed case ("Bash command", "Do you want to
        // proceed?"), so contains matching must fold case on both sides or
        // the blocked rules never fire on a real screen (card
        // 01a0d9b38b0c7780a67f3a14e73e4bcd).
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("claude").expect("claude bundled");
        let snapshot = PaneSnapshot {
            title: "✳ ready".to_string(),
            screen: concat!(
                "╭─ Bash command ──────────────────────────────╮\n",
                "│ cat /etc/hosts                              │\n",
                "╰─────────────────────────────────────────────╯\n",
                "  Do you want to proceed?\n",
                "  ❯ 1. Yes\n",
                "    2. Yes, and don't ask again this session\n",
                "    3. No, and tell Claude what to do differently\n",
                "  Esc to cancel\n",
            )
            .to_string(),
        };
        assert_eq!(
            set.evaluate(&snapshot)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("blocked".to_string(), "bash_permission_prompt".to_string()))
        );
    }

    #[test]
    fn claude_blocked_rules_match_a_captured_real_prompt() {
        // Captured 2026-09-25 from a live Claude Code v2.1.282 session
        // (PTY 100x30, /tmp/claude-capture, bash permission prompt for
        // `cat /etc/hosts`) via this crate's own PtyTerminal — original
        // casing preserved. Against the pre-fix case-sensitive contains,
        // this screen matched no blocked rule and the pane read idle.
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("claude").expect("claude bundled");
        let snapshot = PaneSnapshot {
            title: "✳ ready".to_string(),
            screen: concat!(
                "\n",
                " ▐▛███▛█   Claude Code v2.1.282\n",
                "▝▜██████▀  GLM-5.3-flash with high effort · API Usage Billing\n",
                " ▝▝   ▝▝   /private/tmp/claude-capture\n",
                "\n",
                "\n",
                "❯ Run this exact bash command: cat /etc/hosts\n",
                "\n",
                "  Displaying contents of /etc/hosts\n",
                "  ⎿  $ cat /etc/hosts\n",
                "\n",
                "────────────────────────────────────────────────────────────────────────────────────────────────────\n",
                " Bash command\n",
                " Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below\n",
                "\n",
                "   cat /etc/hosts\n",
                "   Display contents of /etc/hosts\n",
                "\n",
                " Do you want to proceed?\n",
                " ❯ 1. Yes\n",
                "   2. Yes, allow reading from /private/etc from this project\n",
                "   3. Yes, and switch to auto mode · auto mode handles these prompts for you\n",
                "   4. No\n",
                "\n",
                " Esc to cancel · Tab to amend\n",
            )
            .to_string(),
        };
        assert_eq!(
            set.evaluate(&snapshot)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("blocked".to_string(), "bash_permission_prompt".to_string()))
        );
    }

    #[test]
    fn btw_overlay_needs_every_pattern_not_any() {
        // herdr parity (card 01a0da96d5c57ac08001102bf9234145): herdr's
        // compiled_gate_matches requires EVERY regex/line_regex pattern to
        // match; the port fired on ANY, so a bare "/btw" line without the
        // esc-to-close footer read "working" where herdr reads nothing.
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("claude").expect("claude bundled");

        let one_of_two = PaneSnapshot {
            title: "zsh".to_string(),
            screen: "  /btw what does this do\n".to_string(),
        };
        assert_eq!(
            set.evaluate(&one_of_two),
            None,
            "one line_regex pattern of two is not a match"
        );

        let both = PaneSnapshot {
            title: "zsh".to_string(),
            screen: "  /btw what does this do\nhere is the answer\n  esc to close".to_string(),
        };
        assert_eq!(
            set.evaluate(&both)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("working".to_string(), "btw_overlay_working".to_string()))
        );
    }

    #[test]
    fn codex_startup_update_needs_every_regex() {
        // Same all-of contract on the top-level regex list: startup_update
        // carries two patterns and must need both. With one present the
        // pane's honest reading is the idle title rule, not blocked.
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("codex").expect("codex bundled");

        let one_of_two = PaneSnapshot {
            title: "codex".to_string(),
            screen: concat!(
                "Update available!\n",
                "Update now\n",
                "Skip until next version\n",
            )
            .to_string(),
        };
        assert_eq!(
            set.evaluate(&one_of_two)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("idle".to_string(), "osc_title_idle".to_string())),
            "one regex pattern of two must not fire startup_update"
        );

        let both = PaneSnapshot {
            title: "codex".to_string(),
            screen: concat!(
                "Update available!\n",
                "Update now\n",
                "Skip until next version\n",
                "Press enter to continue\n",
            )
            .to_string(),
        };
        assert_eq!(
            set.evaluate(&both)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("blocked".to_string(), "startup_update".to_string()))
        );
    }

    #[test]
    fn claude_content_rule_beats_a_stale_title() {
        // Blocked permission chrome outranks an idle title (priority 980
        // over 250), exercised through the bottom-lines region.
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("claude").expect("claude bundled");
        let snapshot = PaneSnapshot {
            title: "✳ ready".to_string(),
            screen: "❯ ls\n  do you want to proceed?\n  ❯ 1. Yes\n  2. No\nesc to cancel\n"
                .to_string(),
        };
        assert_eq!(
            set.evaluate(&snapshot).map(|(s, _)| s.to_string()),
            Some("blocked".to_string())
        );
    }

    #[test]
    fn codex_weak_blocker_ignores_text_typed_at_the_prompt() {
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("codex").expect("codex bundled");
        // "[y/n]" sitting on the trailing prompt line is the user typing,
        // not a dialog — the port's added `not` guard. The pane may read
        // idle (the user IS at an idle prompt, the title rule's honest
        // reading); it must not read blocked.
        let typed = PaneSnapshot {
            title: "codex".to_string(),
            screen: "› should I run [y/n]\n".to_string(),
        };
        if let Some((state, _)) = set.evaluate(&typed) {
            assert_ne!(state, "blocked", "prompt typing is not a dialog");
        }
        // The same text above a live (sparkled) prompt marker is stale
        // prompt residue in herdr's reading — also never blocked.
        let stale = PaneSnapshot {
            title: "codex".to_string(),
            screen: "[y/n]\n›⠐\n".to_string(),
        };
        if let Some((state, _)) = set.evaluate(&stale) {
            assert_ne!(state, "blocked", "stale prompt is not a dialog");
        }
    }

    #[test]
    fn grok_footer_and_title_rules_rank() {
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("grok").expect("grok bundled");

        // Permission footer hints read as blocked at priority 1190 even
        // against a busy-looking title (spinner rule is 1000).
        let blocked = PaneSnapshot {
            title: "⠧ building".to_string(),
            screen: "┃  1 (●) Yes, proceed\n1/3:select │ Ctrl+o:yolo │ Ctrl+c:cancel\n".to_string(),
        };
        assert_eq!(
            set.evaluate(&blocked).map(|(s, _)| s.to_string()),
            Some("blocked".to_string())
        );

        // The [stop] chip marks a live turn.
        let working = PaneSnapshot {
            title: "my session - grok".to_string(),
            screen: "⠴ Explore /tmp + 1 more… 5.6s   19s ⇣29.7k [stop]\n".to_string(),
        };
        assert_eq!(
            set.evaluate(&working).map(|(s, _)| s.to_string()),
            Some("working".to_string())
        );

        // The resting title reads idle.
        let idle = PaneSnapshot {
            title: "grok".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&idle).map(|(s, _)| s.to_string()),
            Some("idle".to_string())
        );
    }

    /// A fresh override directory per test: its name carries OS-provided
    /// randomness, so no other test run can collide with it (a
    /// `process::id()`-derived name repeats once the OS recycles the pid and
    /// would load a stale override left by an earlier run), and its `Drop`
    /// removes it even when an assertion fails.
    fn override_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("par-mux-scrape-")
            .tempdir()
            .expect("create override temp dir")
    }

    #[test]
    fn override_shadows_bundled_for_its_agent_only() {
        let tmp = override_dir();
        let dir = tmp.path();
        std::fs::write(dir.join("claude.toml"), OVERRIDE_SET).expect("write override");
        let engine = ScrapeEngine::load(Some(dir));

        let set = engine.set_for("claude").expect("claude present");
        let snapshot = PaneSnapshot {
            title: "Override Idle".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&snapshot)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("idle".to_string(), "override_idle".to_string())),
            "the override's rule fired, not the bundled set"
        );
        // The bundled working rule is gone under the shadow.
        let working = PaneSnapshot {
            title: "⠋ Thinking hard".to_string(),
            screen: String::new(),
        };
        assert_eq!(set.evaluate(&working), None, "bundled rules are shadowed");

        // Other agents keep their bundled sets.
        let codex = engine.set_for("codex").expect("codex untouched");
        let snapshot = PaneSnapshot {
            title: "Action Required: trust".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            codex.evaluate(&snapshot).map(|(s, _)| s.to_string()),
            Some("blocked".to_string())
        );
    }

    #[test]
    fn a_broken_override_falls_back_to_bundled() {
        let tmp = override_dir();
        let dir = tmp.path();
        std::fs::write(dir.join("grok.toml"), "this is not toml [[[").expect("write junk");
        let engine = ScrapeEngine::load(Some(dir));

        let set = engine.set_for("grok").expect("grok still served");
        let idle = PaneSnapshot {
            title: "grok".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&idle).map(|(s, _)| s.to_string()),
            Some("idle".to_string()),
            "bundled rules survived the broken override"
        );
    }

    #[test]
    fn an_override_can_add_an_agent_with_no_bundled_set() {
        let tmp = override_dir();
        let dir = tmp.path();
        std::fs::write(
            dir.join("kimi.toml"),
            OVERRIDE_SET.replace("claude", "kimi"),
        )
        .expect("write");
        let engine = ScrapeEngine::load(Some(dir));
        let set = engine.set_for("kimi").expect("override-only agent loads");
        let snapshot = PaneSnapshot {
            title: "Override Idle".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&snapshot).map(|(s, _)| s.to_string()),
            Some("idle".to_string())
        );
    }

    #[test]
    fn tick_reads_title_state_and_clears_it_when_nothing_matches() {
        let (tree, pane_id) = tree_with_pane();
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "claude");
        let engine = ScrapeEngine::load(None);

        // A claude working title through the real OSC 0 path.
        feed(&tree, pane_id, b"\x1b]0;\xe2\xa0\x8b Thinking hard\x07");
        let notifications = scrape_tick(&tree, &engine);
        assert_eq!(notifications.len(), 1, "one transition broadcast");
        assert_eq!(
            notifications[0],
            TmuxNotification::AgentStateChanged {
                pane_id: pane_id.to_string(),
                agent: "claude".to_string(),
                state: "working".to_string(),
                source: "scrape".to_string(),
            }
        );
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert_eq!(
                pane.metadata().get("agent_state").map(String::as_str),
                Some("working")
            );
            assert_eq!(
                pane.metadata()
                    .get("agent_state_source")
                    .map(String::as_str),
                Some("scrape")
            );
            assert_eq!(
                pane.metadata().get("agent_state_rule").map(String::as_str),
                Some("osc_title_working")
            );
        }

        // A second identical tick is quiet — transitions only.
        assert!(
            scrape_tick(&tree, &engine).is_empty(),
            "unchanged state does not re-broadcast"
        );

        // A title that matches nothing clears the guess and stays off the
        // push channel: unknown, never idle.
        feed(&tree, pane_id, b"\x1b]0;plain shell\x07");
        let notifications = scrape_tick(&tree, &engine);
        assert!(
            notifications.is_empty(),
            "a cleared guess is not broadcast (claim-only push)"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        for key in SCRAPE_KEYS {
            assert!(
                !pane.metadata().contains_key(*key),
                "{key} cleared: {:?}",
                pane.metadata()
            );
        }
    }

    #[test]
    fn a_hook_report_wins_and_the_tick_never_overwrites_it() {
        let (tree, pane_id) = tree_with_pane();
        {
            let mut guard = tree.lock();
            let pane = guard.pane_mut(pane_id).expect("pane exists");
            pane.set_metadata("agent", "claude");
            // The state a hook claimed, with hook provenance.
            pane.set_metadata("agent_state", "blocked");
            pane.set_metadata("agent_state_source", "hook");
        }
        let engine = ScrapeEngine::load(None);

        // The screen screams "working" by scrape rules; the claim stands.
        feed(&tree, pane_id, b"\x1b]0;\xe2\xa0\x8b Thinking hard\x07");
        assert!(
            scrape_tick(&tree, &engine).is_empty(),
            "hook authority is never overwritten"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert_eq!(
            pane.metadata().get("agent_state").map(String::as_str),
            Some("blocked"),
            "the hook's state survives the scrape"
        );
        assert_eq!(
            pane.metadata()
                .get("agent_state_source")
                .map(String::as_str),
            Some("hook")
        );
    }

    #[test]
    fn panes_without_an_agent_label_are_never_scraped() {
        let (tree, pane_id) = tree_with_pane();
        let engine = ScrapeEngine::load(None);
        // No metadata at all — a plain shell pane.
        feed(&tree, pane_id, b"\x1b]0;\xe2\xa0\x8b Thinking hard\x07");
        assert!(
            scrape_tick(&tree, &engine).is_empty(),
            "claim-based membership: no label, no scrape"
        );
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        assert!(!pane.metadata().contains_key("agent_state"));
    }

    // ---- the liveness sweep (foreground.rs) over hook claims ----

    fn argv(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|p| p.to_string()).collect())
    }

    /// A hook-authoritative claim carrying the full identity the sweep
    /// must clear: label, state, session ref, and resume argv.
    fn hook_claim(tree: &Arc<Mutex<MuxTree>>, pane_id: PaneId, agent: &str) {
        let mut guard = tree.lock();
        let pane = guard.pane_mut(pane_id).expect("pane exists");
        pane.set_metadata("agent", agent);
        pane.set_metadata("agent_state", "working");
        pane.set_metadata("agent_state_source", "hook");
        pane.set_metadata("agent_session_id", "sess-1");
        pane.set_metadata("agent_resume_argv", r#"["pi","--session","/tmp/s.json"]"#);
    }

    fn claim_intact(tree: &Arc<Mutex<MuxTree>>, pane_id: PaneId) -> bool {
        let guard = tree.lock();
        let pane = guard.pane(pane_id).expect("pane exists");
        pane.metadata()
            .get("agent_state_source")
            .map(String::as_str)
            == Some("hook")
    }

    #[test]
    fn a_dead_agent_loses_its_hook_claim_after_two_mismatching_ticks() {
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "pi");
        let engine = ScrapeEngine::load(None);
        let child = tree
            .lock()
            .pane(pane_id)
            .expect("pane exists")
            .child_pid()
            .expect("spawned pane has a child");
        let agent_pid = child as i32 + 8000; // synthetic; no OS lookup happens
        let alive = ProcessTable::with_fixed_argv(&[
            (child as i32, 1, argv(&["-bash"])),
            (
                agent_pid,
                child as i32,
                argv(&["/usr/local/bin/pi", "--session", "/tmp/s.json"]),
            ),
        ]);
        let dead = ProcessTable::with_fixed_argv(&[(child as i32, 1, argv(&["-bash"]))]);

        // Alive: the claim stands and no miss count accumulates.
        assert!(scrape_tick_with(&tree, &engine, Some(&alive)).is_empty());
        assert!(claim_intact(&tree, pane_id));
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            assert!(
                !pane.metadata().contains_key("agent_liveness_misses"),
                "a matching tick stores no miss count"
            );
        }

        // Dead, tick one of two: recorded, not yet acted on.
        assert!(scrape_tick_with(&tree, &engine, Some(&dead)).is_empty());
        assert!(claim_intact(&tree, pane_id), "one mismatch never clears");

        // Dead, tick two: the claim — label, state, session identity,
        // resume argv, and the miss count — is cleared and the release
        // broadcast, exactly as a `pane.release_agent` report would.
        let notifications = scrape_tick_with(&tree, &engine, Some(&dead));
        assert_eq!(
            notifications,
            vec![TmuxNotification::AgentReleased {
                pane_id: pane_id.to_string(),
                agent: "pi".to_string(),
            }],
            "the sweep broadcasts the release"
        );
        {
            let guard = tree.lock();
            let pane = guard.pane(pane_id).expect("pane exists");
            for key in AGENT_CLAIM_KEYS {
                assert!(
                    !pane.metadata().contains_key(*key),
                    "{key} cleared: {:?}",
                    pane.metadata()
                );
            }
        }

        // A cleared claim is nobody's pane: further ticks are quiet.
        assert!(scrape_tick_with(&tree, &engine, Some(&dead)).is_empty());
    }

    #[test]
    fn a_live_agent_keeps_its_claim_and_a_transient_miss_resets() {
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "pi");
        let engine = ScrapeEngine::load(None);
        let child = tree
            .lock()
            .pane(pane_id)
            .expect("pane exists")
            .child_pid()
            .expect("spawned pane has a child");
        let agent_pid = child as i32 + 8000;
        let alive = ProcessTable::with_fixed_argv(&[
            (child as i32, 1, argv(&["-bash"])),
            (agent_pid, child as i32, argv(&["pi"])),
        ]);
        let dead = ProcessTable::with_fixed_argv(&[(child as i32, 1, argv(&["-bash"]))]);

        // Alive across ticks: no clear ever.
        for _ in 0..3 {
            assert!(scrape_tick_with(&tree, &engine, Some(&alive)).is_empty());
        }
        assert!(claim_intact(&tree, pane_id));

        // One mismatch, then alive again: the count resets, so a LATER
        // run of mismatches must start from one again.
        scrape_tick_with(&tree, &engine, Some(&dead));
        scrape_tick_with(&tree, &engine, Some(&alive));
        assert!(
            scrape_tick_with(&tree, &engine, Some(&dead)).is_empty(),
            "first mismatch after a reset does not clear"
        );
        assert!(claim_intact(&tree, pane_id));
    }

    #[test]
    fn an_unreadable_descendant_keeps_the_claim() {
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "pi");
        let engine = ScrapeEngine::load(None);
        let child = tree
            .lock()
            .pane(pane_id)
            .expect("pane exists")
            .child_pid()
            .expect("spawned pane has a child");
        let table = ProcessTable::with_fixed_argv(&[
            (child as i32, 1, argv(&["-bash"])),
            (child as i32 + 8000, child as i32, None), // present, argv unreadable
        ]);
        for _ in 0..3 {
            assert!(scrape_tick_with(&tree, &engine, Some(&table)).is_empty());
        }
        assert!(
            claim_intact(&tree, pane_id),
            "Unknown keeps the claim — the sweep only acts on proof"
        );
    }

    #[test]
    fn a_relabelled_claim_starts_a_fresh_miss_count() {
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "pi");
        let engine = ScrapeEngine::load(None);
        let child = tree
            .lock()
            .pane(pane_id)
            .expect("pane exists")
            .child_pid()
            .expect("spawned pane has a child");
        let dead_for_pi = ProcessTable::with_fixed_argv(&[(child as i32, 1, argv(&["-bash"]))]);

        // One miss under pi, then the hook relabels to claude.
        scrape_tick_with(&tree, &engine, Some(&dead_for_pi));
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "claude");

        // The new label must not inherit pi's miss: first claude mismatch
        // does not clear…
        assert!(scrape_tick_with(&tree, &engine, Some(&dead_for_pi)).is_empty());
        assert!(claim_intact(&tree, pane_id));
        // …the second does, naming claude.
        let notifications = scrape_tick_with(&tree, &engine, Some(&dead_for_pi));
        assert_eq!(
            notifications,
            vec![TmuxNotification::AgentReleased {
                pane_id: pane_id.to_string(),
                agent: "claude".to_string(),
            }]
        );
    }

    #[test]
    fn a_probe_blind_tick_preserves_the_miss_count() {
        // Unknown is a measurement failure, not liveness: a mismatch must
        // not be erased by an interleaved unreadable-argv tick (the
        // fresh-spawn window where macOS cannot read a mid-exec argv).
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "pi");
        let engine = ScrapeEngine::load(None);
        let child = tree
            .lock()
            .pane(pane_id)
            .expect("pane exists")
            .child_pid()
            .expect("spawned pane has a child");
        let dead = ProcessTable::with_fixed_argv(&[(child as i32, 1, argv(&["-bash"]))]);
        let blind = ProcessTable::with_fixed_argv(&[
            (child as i32, 1, argv(&["-bash"])),
            (child as i32 + 8000, child as i32, None),
        ]);

        scrape_tick_with(&tree, &engine, Some(&dead)); // miss 1
        scrape_tick_with(&tree, &engine, Some(&blind)); // Unknown: count kept
        let notifications = scrape_tick_with(&tree, &engine, Some(&dead)); // miss 2
        assert_eq!(
            notifications,
            vec![TmuxNotification::AgentReleased {
                pane_id: pane_id.to_string(),
                agent: "pi".to_string(),
            }],
            "the blind tick did not reset the miss count"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_real_table_clears_a_claim_no_process_can_match() {
        // End-to-end over the REAL process table: a hook claim whose agent
        // is provably not in the pane's tree (the pane's shell is the only
        // descendant) is cleared. The real table intermittently returns
        // Unknown right after spawn (argv unreadable mid-exec), which keeps
        // — not resets — the count, so poll a bounded number of ticks
        // instead of asserting an exact tick count.
        let (tree, pane_id) = tree_with_pane();
        hook_claim(&tree, pane_id, "zz-no-such-agent-cli");
        let engine = ScrapeEngine::load(None);
        assert!(
            scrape_tick(&tree, &engine).is_empty(),
            "the first mismatching tick never clears"
        );
        let mut released = Vec::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            released = scrape_tick(&tree, &engine);
            if !released.is_empty() {
                break;
            }
        }
        assert_eq!(
            released,
            vec![TmuxNotification::AgentReleased {
                pane_id: pane_id.to_string(),
                agent: "zz-no-such-agent-cli".to_string(),
            }],
            "the real probe proves absence and the sweep clears"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_real_table_keeps_a_claim_for_a_process_in_the_pane_tree() {
        // A pane whose child tree really does contain the claimed "agent"
        // — here a sleep command standing in for the CLI — keeps its claim
        // across the real table's ticks.
        let mut tree = MuxTree::new(Box::new(ShellPaneFactory::default()));
        let session = tree.new_session("real", 80, 24).expect("session spawns");
        let root = tree
            .session(session)
            .expect("session exists")
            .windows
            .iter()
            .filter_map(|window| tree.window(*window))
            .flat_map(|window| window.panes())
            .next()
            .expect("a new session has a pane");
        let sleeper = split_pane_with_command(&mut tree, root, "sleep 300");
        let tree = Arc::new(Mutex::new(tree));
        hook_claim(&tree, sleeper, "sleep");
        let engine = ScrapeEngine::load(None);
        for _ in 0..5 {
            assert!(
                scrape_tick(&tree, &engine).is_empty(),
                "a live stand-in agent is never released"
            );
        }
        assert!(claim_intact(&tree, sleeper));
    }

    #[cfg(unix)]
    fn split_pane_with_command(tree: &mut MuxTree, root: PaneId, command: &str) -> PaneId {
        use crate::mux::layout::SplitDirection;
        tree.split_pane(root, SplitDirection::Vertical, 0.5, Some(command))
            .expect("split spawns")
    }
}
