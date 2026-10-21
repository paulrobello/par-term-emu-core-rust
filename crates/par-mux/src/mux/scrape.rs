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
//! the `hook` provenance of a claim. The one exception is opt-in per set:
//! `idle_fallback = true` ports herdr's known-agent idle default for CLIs
//! that draw no idle marker (droid, copilot, cursor), and fires only while
//! the process table proves the agent runs in the pane, stored under the
//! rule id [`IDLE_FALLBACK_RULE`].
//!
//! Patterns ship bundled (`include_str!`, one TOML per agent under
//! `patterns/`) and a local override file shadows the bundled set for its
//! agent — precedence bundled < override, no remote catalog (the T1+T3
//! ruling). An override that fails to parse or validate falls back to
//! bundled with a warning, never silently disabling the agent.

use crate::mux::foreground::{Liveness, ProcessTable};
use crate::mux::hooks::clear_agent_claim;
use crate::mux::ids::PaneId;
use crate::mux::pane::MuxPane;
use crate::mux::tree::MuxTree;
use par_term_emu_core::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The bundled pattern sets, one per scrape-supported agent. pi and omp
/// need none (they report state through hooks); muse joins with herdr's
/// proven captures — its hooks run with a cleared environment, so identity
/// arrives by hook and state by scrape, the claude shape. droid, copilot,
/// and cursor follow the same split: a session hook for identity, these
/// sets for state. Each key must equal its file's TOML `id`.
const BUNDLED_PATTERNS: &[(&str, &str)] = &[
    ("claude", include_str!("patterns/claude.toml")),
    ("codex", include_str!("patterns/codex.toml")),
    ("grok", include_str!("patterns/grok.toml")),
    ("muse", include_str!("patterns/muse.toml")),
    ("droid", include_str!("patterns/droid.toml")),
    ("copilot", include_str!("patterns/copilot.toml")),
    ("cursor", include_str!("patterns/cursor.toml")),
    ("kimi", include_str!("patterns/kimi.toml")),
    ("qwen", include_str!("patterns/qwen.toml")),
    ("qodercli", include_str!("patterns/qodercli.toml")),
    ("devin", include_str!("patterns/devin.toml")),
    ("gemini", include_str!("patterns/gemini.toml")),
    ("amp", include_str!("patterns/amp.toml")),
];

/// The states a rule may assert. `unknown` is not among them: it is the
/// absence of a match, produced by no rule at all.
const VALID_STATES: &[&str] = &["working", "blocked", "done", "idle"];

/// The rule id the scrape tier stores for a fallback `idle`: a set with
/// `idle_fallback = true`, no rule matching, the agent's process alive.
pub const IDLE_FALLBACK_RULE: &str = "idle_fallback";

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
    /// herdr's engine default for a known agent
    /// (`DEFAULT_KNOWN_AGENT_IDLE_FALLBACK`): no matching rule reads as
    /// `idle` — here only while the process table proves the agent runs in
    /// the pane. Opt-in per set, for agents whose CLIs draw no idle marker
    /// a rule could match.
    #[serde(default)]
    idle_fallback: bool,
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
    /// The region as written, for `agent-explain`.
    region_text: String,
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
    /// The set's `id`, which the fallback's liveness probe also accepts,
    /// so an alias label (`github-copilot`) finds a `copilot` process.
    id: String,
    /// See [`PatternSet::idle_fallback`].
    idle_fallback: bool,
    /// Where the set came from: `bundled:patterns/<id>.toml`, or the
    /// override file's path. `agent-explain` names it.
    origin: String,
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
                region_text: rule.region.clone(),
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
                id: id.clone(),
                idle_fallback: set.idle_fallback,
                origin: format!("bundled:patterns/{id}.toml"),
            },
            id,
            aliases,
        ))
    }

    /// Where this set was loaded from: `bundled:patterns/<id>.toml` or an
    /// override file's path.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The set's `version` field.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Whether the set opts into the idle fallback ([`IDLE_FALLBACK_RULE`]).
    pub fn idle_fallback(&self) -> bool {
        self.idle_fallback
    }

    /// The fallback verdict for a pane no rule matched: `idle` when the set
    /// opts in and the process table proves the agent (under its label or
    /// the set's id) runs in the pane's process tree. No table (Windows)
    /// proves nothing, so the pane stays unknown.
    fn fallback(
        &self,
        label: &str,
        child_pid: Option<u32>,
        table: Option<&ProcessTable>,
    ) -> Option<(&'static str, &'static str)> {
        if !self.idle_fallback {
            return None;
        }
        let (table, pid) = (table?, child_pid?);
        [label, self.id.as_str()]
            .iter()
            .any(|name| table.agent_alive(pid, name) == Liveness::Matches)
            .then_some(("idle", IDLE_FALLBACK_RULE))
    }

    /// The rule the scrape tick would pick, with what `agent-explain`
    /// reports about it, plus how many rules the set holds.
    pub fn explain(&self, snapshot: &PaneSnapshot) -> (Option<RuleMatch<'_>>, usize) {
        let matched = self.rules.iter().find_map(|rule| {
            let text = region_text(&rule.region, snapshot);
            let lines: Vec<&str> = text.lines().collect();
            rule.cond.matches(&text, &lines).then_some(RuleMatch {
                id: &rule.id,
                state: &rule.state,
                priority: rule.priority,
                region: &rule.region_text,
            })
        });
        (matched, self.rules.len())
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

/// The rule a scrape picks for a snapshot, as `agent-explain` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMatch<'a> {
    /// The rule's `id`.
    pub id: &'a str,
    /// The state it asserts.
    pub state: &'a str,
    /// Its priority.
    pub priority: i64,
    /// Its region, as written in the pattern file.
    pub region: &'a str,
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
                            Ok((mut shadow, shadow_id, shadow_aliases)) => {
                                shadow.origin = path.display().to_string();
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
                    if let Ok((mut set, id, aliases)) = CompiledSet::compile(&text) {
                        set.origin = path.display().to_string();
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

    /// The bundled-only engine, compiled once per process — what
    /// `agent-explain` reads when the tree carries no daemon engine.
    pub fn bundled() -> Arc<ScrapeEngine> {
        static BUNDLED: std::sync::OnceLock<Arc<ScrapeEngine>> = std::sync::OnceLock::new();
        Arc::clone(BUNDLED.get_or_init(|| Arc::new(ScrapeEngine::load(None))))
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

/// A tick that saw the agent alive drops the miss count: only unbroken
/// mismatches clear a claim. `Unknown` deliberately does NOT reset — a
/// measurement failure (argv unreadable mid-`exec`, the fresh-spawn
/// window) is not liveness, and erasing proof because the probe went
/// blind for a tick would let a dead agent's claim survive alternation.
fn reset_liveness_misses(pane: &mut MuxPane) {
    pane.update_agent_claim(|claim| {
        claim.liveness_misses = None;
        claim.liveness_misses_agent = None;
    });
}

/// Record one mismatching tick and return the count after the increment.
/// A count carried under a different agent label is the previous agent's;
/// restart from one under the current label.
fn bump_liveness_misses(pane: &mut MuxPane, agent: &str) -> u8 {
    pane.update_agent_claim(|claim| {
        let prior = if claim.liveness_misses_agent.as_deref() == Some(agent) {
            claim.liveness_misses.unwrap_or(0)
        } else {
            0
        };
        let misses = prior.saturating_add(1);
        claim.liveness_misses = Some(misses);
        claim.liveness_misses_agent = Some(agent.to_string());
        misses
    })
    .unwrap_or(0)
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
        // The owning workspace for the push, read before the pane_mut
        // borrow; the same map roster_rows reads, so push and list-agents
        // agree.
        let ws = guard
            .workspace_of_pane(pane_id)
            .map(|w| w.to_string())
            .unwrap_or_default();
        let in_view = crate::mux::hooks::pane_in_view(&guard, pane_id);
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
                                clear_agent_claim(pane);
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
        let matched = set
            .evaluate(&snapshot)
            .or_else(|| set.fallback(&agent, pane.child_pid(), table));
        match matched {
            Some((state, rule)) => {
                let prior_state = pane.metadata().get("agent_state").map(String::as_str);
                let state = crate::mux::hooks::derive_state(prior_state, state, in_view);
                let unchanged =
                    prior_source.as_deref() == Some("scrape") && prior_state == Some(state);
                if !unchanged {
                    pane.set_metadata("agent_state", state);
                    pane.set_metadata("agent_state_source", "scrape");
                    pane.set_metadata("agent_state_rule", rule);
                    notifications.push(TmuxNotification::AgentStateChanged {
                        pane_id: pane_id.to_string(),
                        agent,
                        state: state.to_string(),
                        source: "scrape".to_string(),
                        ws,
                        msg: String::new(),
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
        for agent in [
            "claude", "codex", "grok", "muse", "droid", "copilot", "cursor",
        ] {
            assert!(engine.set_for(agent).is_some(), "{agent} bundled");
        }
        assert!(engine.set_for("claude-code").is_some(), "claude alias");
        assert!(engine.set_for("grok-build").is_some(), "grok alias");
        assert!(engine.set_for("muse-code").is_some(), "muse alias");
        assert!(engine.set_for("github-copilot").is_some(), "copilot alias");
        assert!(engine.set_for("cursor-agent").is_some(), "cursor alias");
        // A bundled set that fails to compile is only logged and dropped,
        // so each file is compiled directly here; its id must also equal
        // the BUNDLED_PATTERNS key or the override lookup misses it.
        for (agent, text) in BUNDLED_PATTERNS {
            let (_, id, _) = CompiledSet::compile(text)
                .unwrap_or_else(|err| panic!("bundled {agent} fails to compile: {err}"));
            assert_eq!(&id, agent, "the TOML id is the bundled key");
        }
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

    /// A rule may assert `done` directly (herdr's derived state, as a
    /// pattern target).
    #[test]
    fn an_override_rule_can_match_done() {
        let tmp = override_dir();
        let dir = tmp.path();
        std::fs::write(
            dir.join("kimi.toml"),
            OVERRIDE_SET
                .replace("claude", "kimi")
                .replace("state = \"idle\"", "state = \"done\""),
        )
        .expect("write");
        let engine = ScrapeEngine::load(Some(dir));
        let set = engine.set_for("kimi").expect("a done rule validates");
        let snapshot = PaneSnapshot {
            title: "Override Idle".to_string(),
            screen: String::new(),
        };
        assert_eq!(
            set.evaluate(&snapshot).map(|(s, _)| s.to_string()),
            Some("done".to_string())
        );
    }

    /// A scraped finish (working -> idle) on a pane no client views is
    /// `done`, and later ticks that keep matching idle hold it quietly
    /// rather than re-broadcasting every tick.
    #[test]
    fn a_scraped_finish_out_of_view_is_done_and_holds_quietly() {
        let (tree, pane_id) = tree_with_pane();
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "claude");
        let engine = ScrapeEngine::load(None);
        feed(&tree, pane_id, b"\x1b]0;\xe2\xa0\x8b Thinking hard\x07");
        assert_eq!(scrape_tick(&tree, &engine).len(), 1, "working");
        feed(&tree, pane_id, "\x1b]0;\u{2733} ready\x07".as_bytes());
        let finished = scrape_tick(&tree, &engine);
        assert!(
            matches!(
                finished.as_slice(),
                [TmuxNotification::AgentStateChanged { state, source, .. }]
                    if state == "done" && source == "scrape"
            ),
            "{finished:?}"
        );
        assert!(
            scrape_tick(&tree, &engine).is_empty(),
            "a held done does not re-broadcast"
        );
        assert_eq!(
            tree.lock()
                .pane(pane_id)
                .expect("pane exists")
                .metadata()
                .get("agent_state")
                .map(String::as_str),
            Some("done")
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
                ws: tree
                    .lock()
                    .workspace_of_pane(pane_id)
                    .map(|w| w.to_string())
                    .unwrap_or_default(),
                msg: String::new(),
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
            for key in crate::mux::hooks::AGENT_CLAIM_KEYS {
                assert!(
                    !pane.metadata().contains_key(*key),
                    "{key} cleared: {:?}",
                    pane.metadata()
                );
            }
            assert!(
                pane.seq_by_source.is_empty()
                    && pane.telemetry.is_none()
                    && pane.host_telemetry.is_none(),
                "the typed half of the claim is cleared too"
            );
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
        // 10 s budget (200 x 50 ms): the release itself lands in a couple of
        // ticks; the bound only has to absorb parallel-load stretches.
        for _ in 0..200 {
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
        // The split's `sleep 300` is exec'd asynchronously: under parallel
        // load a real-table tick can run before exec lands, which reads as a
        // provable mismatch and would clear the claim before the stand-in is
        // ever visible. Wait for the probe to prove it first, bounded.
        let child = tree
            .lock()
            .pane(sleeper)
            .expect("pane exists")
            .child_pid()
            .expect("the split spawned a child");
        let visible_by = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match ProcessTable::snapshot().map(|table| table.agent_alive(child, "sleep")) {
                Some(Liveness::Matches) => break,
                _ if std::time::Instant::now() >= visible_by => {
                    panic!("the sleep stand-in never became visible to the real process table");
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
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

    #[test]
    fn muse_rules_read_trust_approval_working_and_idle() {
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for("muse").expect("muse bundled");

        // The trust screen is a real blocker (herdr's capture shape).
        let trust = PaneSnapshot {
            title: String::new(),
            screen: concat!(
                "Do you trust this workspace?\n",
                "Workspace: /tmp/proj\n",
                "⟩ 1  Trust and continue\n",
                "  2  Quit\n",
            )
            .to_string(),
        };
        assert_eq!(
            set.evaluate(&trust)
                .map(|(s, r)| (s.to_string(), r.to_string())),
            Some(("blocked".to_string(), "workspace_trust_blocked".to_string()))
        );

        // The paired picker controls distinguish a real question from
        // transcript text that happens to mention the phrases.
        let picker = PaneSnapshot {
            title: String::new(),
            screen: "Which tool?\nEnter to select · ↑/↓ to move · Tab for an optional note\n"
                .to_string(),
        };
        assert_eq!(
            set.evaluate(&picker).map(|(s, _)| s.to_string()),
            Some("blocked".to_string())
        );

        // A working turn names its interrupt hint.
        let working = PaneSnapshot {
            title: String::new(),
            screen: "◆ Working (designing · esc to interrupt)\n".to_string(),
        };
        assert_eq!(
            set.evaluate(&working).map(|(s, _)| s.to_string()),
            Some("working".to_string())
        );

        // The idle prompt is the `⟩` line; a bare shell prompt behind an
        // exited agent must NOT read idle for muse.
        let idle = PaneSnapshot {
            title: String::new(),
            screen: "probello@MacDaddy:~\n❯ \n⟩ \n".to_string(),
        };
        assert_eq!(
            set.evaluate(&idle).map(|(s, _)| s.to_string()),
            Some("idle".to_string())
        );
        let not_muse = PaneSnapshot {
            title: String::new(),
            screen: "probello@MacDaddy:~\n❯ \n".to_string(),
        };
        assert_eq!(set.evaluate(&not_muse), None, "no muse signal, no state");

        // The model · effort · cwd footer is the idle fallback.
        let footer = PaneSnapshot {
            title: String::new(),
            screen: "muse-spark-1.2 · medium · ~/proj\n".to_string(),
        };
        assert_eq!(
            set.evaluate(&footer).map(|(s, _)| s.to_string()),
            Some("idle".to_string())
        );
    }

    /// Evaluate `screen` against `agent`'s bundled set: (state, rule id).
    fn scrape_screen(agent: &str, screen: &str) -> Option<(String, String)> {
        let engine = ScrapeEngine::load(None);
        let set = engine.set_for(agent).expect("bundled");
        set.evaluate(&PaneSnapshot {
            title: String::new(),
            screen: screen.to_string(),
        })
        .map(|(state, rule)| (state.to_string(), rule.to_string()))
    }

    fn rule_of(agent: &str, screen: &str) -> Option<String> {
        scrape_screen(agent, screen).map(|(_, rule)| rule)
    }

    #[test]
    fn droid_rules_read_approval_menu_and_working() {
        let approval = concat!(
            "Execute command: rm -rf build\n",
            "> Yes, allow\n",
            "  No, cancel\n",
            "Use ↑↓ to navigate, Enter to select, Esc to cancel\n",
        );
        assert_eq!(
            scrape_screen("droid", approval),
            Some((
                "blocked".to_string(),
                "execute_selection_blocker".to_string()
            ))
        );
        assert_eq!(
            rule_of(
                "droid",
                "Pick a model\n↑↓ navigate · enter select · esc cancel\n"
            )
            .as_deref(),
            Some("selection_menu_blocker")
        );
        // The braille spinner outranks the bare stop hint.
        assert_eq!(
            rule_of("droid", "⠋ Thinking… (esc to stop)\n").as_deref(),
            Some("spinner_stop_working")
        );
        assert_eq!(
            rule_of("droid", "Running tool (esc to stop)\n").as_deref(),
            Some("stop_hint_working")
        );
        assert_eq!(scrape_screen("droid", "> \n"), None, "no signal, no state");
    }

    #[test]
    fn copilot_rules_read_selection_and_working() {
        let selection = concat!(
            "Allow this command?\n",
            "❯ 1. Yes\n",
            "  2. No\n",
            "↑↓ to navigate · Enter to select · Esc to cancel\n",
        );
        assert_eq!(
            scrape_screen("copilot", selection),
            Some(("blocked".to_string(), "selection_blocker".to_string()))
        );
        assert_eq!(
            rule_of("copilot", "◎ Waiting for background agents · 12s\n").as_deref(),
            Some("background_agents_working")
        );
        // The cancel hint without a select/confirm control is a turn.
        assert_eq!(
            scrape_screen("copilot", "∙ Thinking (Esc to cancel)\n"),
            Some(("working".to_string(), "working_cancel_hint".to_string()))
        );
        assert_eq!(scrape_screen("copilot", "> \n"), None);
        assert!(ScrapeEngine::load(None).set_for("ghcs").is_some());
    }

    #[test]
    fn cursor_rules_read_approvals_and_working() {
        let write = concat!(
            "src/main.rs\n",
            "Write to this file?\n",
            "→ Proceed (y)\n",
            "  Reject & propose changes (esc or n or p)\n",
        );
        assert_eq!(
            scrape_screen("cursor", write),
            Some(("blocked".to_string(), "write_file_approval".to_string()))
        );
        let run = concat!(
            "Waiting for approval...\n",
            "Run this command?\n",
            "→ Run (once) (y)\n",
            "  Skip (esc or n)\n",
        );
        assert_eq!(rule_of("cursor", run).as_deref(), Some("approval_prompt"));
        assert_eq!(
            rule_of("cursor", "⬢ Generating\n  ctrl+c to stop\n").as_deref(),
            Some("stop_hint_working")
        );
        assert_eq!(
            rule_of("cursor", "2 background tasks\n").as_deref(),
            Some("background_task_status_working")
        );
        assert_eq!(
            scrape_screen("cursor", "⠋ Reading files\n"),
            Some(("working".to_string(), "spinner_working".to_string()))
        );
        assert_eq!(scrape_screen("cursor", "→ Add a follow-up\n"), None);
    }

    /// The pane's labelled state after one tick against `table`:
    /// (agent_state, agent_state_rule).
    fn tick_state(
        tree: &Arc<Mutex<MuxTree>>,
        pane_id: PaneId,
        table: Option<&ProcessTable>,
    ) -> (Option<String>, Option<String>) {
        scrape_tick_with(tree, &ScrapeEngine::load(None), table);
        let guard = tree.lock();
        let metadata = guard.pane(pane_id).expect("pane exists").metadata();
        (
            metadata.get("agent_state").cloned(),
            metadata.get("agent_state_rule").cloned(),
        )
    }

    /// A table where the pane's shell runs `agent_argv` as its child.
    fn table_running(child: u32, agent_argv: &[&str]) -> ProcessTable {
        ProcessTable::with_fixed_argv(&[
            (child as i32, 1, argv(&["-bash"])),
            (child as i32 + 8000, child as i32, argv(agent_argv)),
        ])
    }

    #[test]
    fn idle_fallback_needs_the_agents_process_and_loses_to_a_rule() {
        let engine = ScrapeEngine::load(None);
        for agent in ["droid", "copilot", "cursor"] {
            assert!(engine.set_for(agent).unwrap().idle_fallback(), "{agent}");
        }
        for agent in ["claude", "codex", "grok", "muse"] {
            assert!(!engine.set_for(agent).unwrap().idle_fallback(), "{agent}");
        }

        let (tree, pane_id) = tree_with_pane();
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "droid");
        let child = tree.lock().pane(pane_id).unwrap().child_pid().unwrap();
        let alive = table_running(child, &["node", "/opt/factory/bin/droid"]);
        let shell_only = ProcessTable::with_fixed_argv(&[(child as i32, 1, argv(&["-bash"]))]);

        // No table (Windows) and a table without droid prove nothing.
        assert_eq!(tick_state(&tree, pane_id, None), (None, None));
        assert_eq!(tick_state(&tree, pane_id, Some(&shell_only)), (None, None));

        // droid alive, no rule matching: idle under the fallback rule id.
        assert_eq!(
            tick_state(&tree, pane_id, Some(&alive)),
            (Some("idle".into()), Some(IDLE_FALLBACK_RULE.into()))
        );

        // A matching rule outranks the fallback. The leading newline puts
        // the spinner glyph at column 0 of its own row: the pane is a live
        // spawned shell, whose prompt lands asynchronously and would
        // otherwise share the row (and break the rule's `^\s*[⠀-⣿]`
        // anchor) on hosts whose shell prints before this feed runs.
        feed(
            &tree,
            pane_id,
            b"\r\n\xE2\xA0\x8B Thinking\xE2\x80\xA6 (esc to stop)\r\n",
        );
        assert_eq!(
            tick_state(&tree, pane_id, Some(&alive)),
            (Some("working".into()), Some("spinner_stop_working".into()))
        );

        // droid gone: the fallback stops and the guess is cleared.
        feed(&tree, pane_id, b"\x1b[2J\x1b[H");
        assert_eq!(tick_state(&tree, pane_id, Some(&shell_only)), (None, None));
    }

    #[test]
    fn idle_fallback_probes_the_set_id_under_an_alias_label() {
        let (tree, pane_id) = tree_with_pane();
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "github-copilot");
        let child = tree.lock().pane(pane_id).unwrap().child_pid().unwrap();
        let alive = table_running(child, &["/usr/local/bin/copilot"]);
        assert_eq!(
            tick_state(&tree, pane_id, Some(&alive)).0.as_deref(),
            Some("idle")
        );
    }

    #[test]
    fn a_set_without_idle_fallback_stays_unknown_on_no_match() {
        let (tree, pane_id) = tree_with_pane();
        tree.lock()
            .pane_mut(pane_id)
            .expect("pane exists")
            .set_metadata("agent", "claude");
        let child = tree.lock().pane(pane_id).unwrap().child_pid().unwrap();
        let alive = table_running(child, &["claude"]);
        assert_eq!(tick_state(&tree, pane_id, Some(&alive)), (None, None));
    }
}
