//! Trigger system for automated pattern matching on terminal output
//!
//! Provides regex-based triggers that scan terminal lines for patterns and execute
//! configurable actions (highlight, notify, bookmark, set variable, etc.).
//! Uses `RegexSet` for efficient multi-pattern matching in a single pass.

use std::collections::{HashMap, HashSet};

/// Unique trigger identifier
pub type TriggerId = u64;

/// Split direction for SplitPane trigger actions (mirrors par-term-config::TriggerSplitDirection)
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerSplitDirection {
    /// Split side by side.
    Horizontal,
    /// Split stacked.
    Vertical,
}

/// Which pane to split (mirrors par-term-config::TriggerSplitTarget)
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TriggerSplitTarget {
    /// Split the active pane.
    #[default]
    Active,
    /// Split the pane that produced the match.
    Source,
}

/// Command to run in the new pane (mirrors par-term-config::SplitPaneCommand)
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerSplitCommand {
    /// Send text to the new pane's shell.
    SendText {
        /// Text to send.
        text: String,
        /// Delay before sending, in milliseconds.
        delay_ms: u64,
    },
    /// Run a command as the new pane's initial process.
    InitialCommand {
        /// Executable to run.
        command: String,
        /// Command arguments.
        args: Vec<String>,
    },
}

/// Action to execute when a trigger matches
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerAction {
    /// Highlight matched text with specified colors and optional duration
    Highlight {
        /// Foreground color (RGB), if any.
        fg: Option<(u8, u8, u8)>,
        /// Background color (RGB), if any.
        bg: Option<(u8, u8, u8)>,
        /// Duration in milliseconds (0 = permanent)
        duration_ms: u64,
    },
    /// Send a notification (reuses existing notification system)
    Notify {
        /// Notification title.
        title: String,
        /// Notification body.
        message: String,
    },
    /// Add a bookmark/mark on the matched line
    MarkLine {
        /// Optional label for the mark.
        label: Option<String>,
        /// Optional mark color (RGB).
        color: Option<(u8, u8, u8)>,
    },
    /// Set a session variable (reuses existing badge session_variables)
    SetVariable {
        /// Variable name.
        name: String,
        /// Value to assign.
        value: String,
    },
    /// Run an external command (emitted as event for frontend)
    RunCommand {
        /// Executable to run.
        command: String,
        /// Command arguments.
        args: Vec<String>,
    },
    /// Play a sound (emitted as event for frontend)
    PlaySound {
        /// Sound identifier.
        sound_id: String,
        /// Playback volume.
        volume: u8,
    },
    /// Send text to the terminal (emitted as event for frontend)
    SendText {
        /// Text to send.
        text: String,
        /// Delay before sending, in milliseconds.
        delay_ms: u64,
    },
    /// Open a new pane (frontend-handled, emitted as ActionResult::SplitPane)
    SplitPane {
        /// Split direction.
        direction: TriggerSplitDirection,
        /// Command to run in the new pane.
        command: Option<TriggerSplitCommand>,
        /// Whether the new pane takes focus.
        focus_new_pane: bool,
        /// Which pane to split.
        target: TriggerSplitTarget,
    },
    /// Stop processing remaining actions for this trigger
    StopPropagation,
}

/// A registered trigger with its pattern and actions
#[derive(Debug, Clone)]
pub struct Trigger {
    /// Unique ID
    pub id: TriggerId,
    /// Human-readable name
    pub name: String,
    /// Raw regex pattern string
    pub pattern: String,
    /// Compiled regex (private, rebuilt as needed)
    regex: regex::Regex,
    /// Whether this trigger is active
    pub enabled: bool,
    /// Only match once per line (prevent duplicate matches on same line)
    pub fire_once_per_line: bool,
    /// Actions to execute on match
    pub actions: Vec<TriggerAction>,
    /// Creation timestamp (unix millis)
    pub created: u64,
    /// Number of times this trigger has matched
    pub match_count: usize,
}

/// Result of a trigger match on a line
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerMatch {
    /// ID of the trigger that matched
    pub trigger_id: TriggerId,
    /// Visible row the matched text occupied when it was scanned. A row
    /// scanned as it scrolled off the screen (ARC-091) reports the row it
    /// occupied just before leaving, so a `MarkLine` bookmark or highlight
    /// for it points at that pre-scroll index.
    pub row: usize,
    /// Column where match starts
    pub col: usize,
    /// Column where match ends (exclusive)
    pub end_col: usize,
    /// Matched text
    pub text: String,
    /// Capture groups (group 0 = full match, then numbered groups)
    pub captures: Vec<String>,
    /// Timestamp when match occurred (unix millis)
    pub timestamp: u64,
}

/// A highlight overlay created by a trigger action
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerHighlight {
    /// Row of the highlight
    pub row: usize,
    /// Start column (inclusive)
    pub col_start: usize,
    /// End column (exclusive)
    pub col_end: usize,
    /// Foreground color override
    pub fg: Option<(u8, u8, u8)>,
    /// Background color override
    pub bg: Option<(u8, u8, u8)>,
    /// Expiry timestamp (u64::MAX = permanent, unix millis otherwise)
    pub expiry: u64,
}

/// Trigger registry, highlights, action results, and pending scan rows
/// (Feature 18: Triggers & Automation).
pub(crate) struct TriggerState {
    trigger_registry: TriggerRegistry,
    trigger_highlights: Vec<TriggerHighlight>,
    trigger_action_results: Vec<ActionResult>,
    max_action_results: usize,
    pending_trigger_rows: HashSet<usize>,
}

impl Default for TriggerState {
    fn default() -> Self {
        Self {
            trigger_registry: TriggerRegistry::default(),
            trigger_highlights: Vec::new(),
            trigger_action_results: Vec::new(),
            max_action_results: 100,
            pending_trigger_rows: HashSet::new(),
        }
    }
}

impl TriggerState {
    /// Queue a row for trigger scanning (ARC-064).
    ///
    /// Triggers observe written text, not incidental render damage: only
    /// the printable-write path calls this, so erases, scrolls, attribute
    /// changes and screen switches cannot re-fire a trigger that already
    /// matched. Render damage still goes through `mark_row_dirty`.
    pub(crate) fn mark_written(&mut self, row: usize) {
        if self.trigger_registry.has_active_triggers() {
            self.pending_trigger_rows.insert(row);
        }
    }

    /// Move pending trigger rows with their content after a region scroll.
    ///
    /// Rows outside the scrolled region are untouched. A row whose content
    /// left the region was already scanned via [`TriggerState::take_departing`];
    /// any still listed is dropped here.
    pub(crate) fn shift_pending(&mut self, down: bool, n: usize, top: usize, bottom: usize) {
        if self.pending_trigger_rows.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_trigger_rows);
        self.pending_trigger_rows = pending
            .into_iter()
            .filter_map(|r| {
                if r < top || r > bottom {
                    Some(r)
                } else if down {
                    (r + n <= bottom).then_some(r + n)
                } else {
                    r.checked_sub(n).filter(|nr| *nr >= top)
                }
            })
            .collect();
    }

    /// Remove and return, sorted, the pending rows a region scroll is about
    /// to push off the screen of a `grid_rows`-row grid (ARC-091).
    ///
    /// `down`/`n`/`top`/`bottom` mean what they mean to
    /// [`TriggerState::shift_pending`]: content moving up leaves at `top`,
    /// content moving down leaves at `bottom`. Only rows already pending
    /// are returned (the ARC-064 written-text rule), and each is removed
    /// from the set, so it is scanned exactly once. Empty when no trigger
    /// is active.
    pub(crate) fn take_departing(
        &mut self,
        down: bool,
        n: usize,
        top: usize,
        bottom: usize,
        grid_rows: usize,
    ) -> Vec<usize> {
        if self.pending_trigger_rows.is_empty() || !self.trigger_registry.has_active_triggers() {
            return Vec::new();
        }
        let bottom = bottom.min(grid_rows.saturating_sub(1));
        if n == 0 || top > bottom {
            return Vec::new();
        }
        let n = n.min(bottom - top + 1);
        let (first, last) = if down {
            (bottom + 1 - n, bottom)
        } else {
            (top, top + n - 1)
        };
        let mut departing: Vec<usize> = self
            .pending_trigger_rows
            .iter()
            .copied()
            .filter(|r| (first..=last).contains(r))
            .collect();
        if departing.is_empty() {
            return departing;
        }
        departing.sort_unstable();
        for r in &departing {
            self.pending_trigger_rows.remove(r);
        }
        departing
    }

    /// Whether any written row is waiting to be scanned.
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending_trigger_rows.is_empty()
    }

    /// Remove and return every pending row.
    pub(crate) fn take_pending(&mut self) -> Vec<usize> {
        self.pending_trigger_rows.drain().collect()
    }

    /// Drop every pending row without scanning it.
    pub(crate) fn clear_pending(&mut self) {
        self.pending_trigger_rows.clear();
    }

    /// Queue an ActionResult for frontend polling under the
    /// `max_action_results` cap (QA-197). When the buffer is full the new
    /// result is dropped, matching the RunCommand/PlaySound/SendText arms
    /// that capped inline before this helper existed; Notify/MarkLine/
    /// SplitPane previously grew the buffer without bound.
    pub(crate) fn push_action_result(&mut self, result: ActionResult) {
        if self.trigger_action_results.len() < self.max_action_results {
            self.trigger_action_results.push(result);
        }
    }

    /// Carry the registry across RIS: `self` is the fresh state, `old` the
    /// state being replaced. Highlights, action results, and pending scan
    /// rows reset.
    pub(crate) fn carry_registry_from(&mut self, old: &mut TriggerState) {
        std::mem::swap(&mut self.trigger_registry, &mut old.trigger_registry);
    }
}

use crate::terminal::Terminal;

/// Trigger & automation operations on a [`Terminal`] (ARC-039).
///
/// Stateless by construction — call as `TriggerEngine::add_trigger(&mut term, ...)`.
pub struct TriggerEngine;

impl TriggerEngine {
    /// Add a new trigger with the given name, regex pattern, and actions
    ///
    /// Returns the trigger ID on success, or an error if the regex is invalid.
    pub fn add_trigger(
        term: &mut Terminal,
        name: String,
        pattern: String,
        actions: Vec<TriggerAction>,
    ) -> Result<TriggerId, String> {
        term.triggers.trigger_registry.add(name, pattern, actions)
    }

    /// Remove a trigger by ID
    pub fn remove_trigger(term: &mut Terminal, id: TriggerId) -> bool {
        term.triggers.trigger_registry.remove(id)
    }

    /// Enable or disable a trigger
    pub fn set_trigger_enabled(term: &mut Terminal, id: TriggerId, enabled: bool) -> bool {
        term.triggers.trigger_registry.set_enabled(id, enabled)
    }

    /// List all registered triggers
    pub fn list_triggers(term: &Terminal) -> Vec<&Trigger> {
        term.triggers.trigger_registry.list()
    }

    /// Get a trigger by ID
    pub fn get_trigger(term: &Terminal, id: TriggerId) -> Option<&Trigger> {
        term.triggers.trigger_registry.get(id)
    }

    /// Drain all pending trigger match events
    pub fn poll_trigger_matches(term: &mut Terminal) -> Vec<TriggerMatch> {
        term.triggers.trigger_registry.poll_matches()
    }

    /// Process trigger scans on pending dirty rows
    ///
    /// Called automatically in the PTY reader thread after `process()`.
    /// Can also be called manually for non-PTY terminals. Rows that left
    /// the screen during `process()` were already scanned as they left
    /// (ARC-091), so this covers the rows still visible.
    pub fn process_trigger_scans(term: &mut Terminal) {
        if !term.triggers.trigger_registry.has_active_triggers() {
            term.triggers.clear_pending();
            return;
        }

        let rows_to_scan: Vec<usize> = term.triggers.take_pending();
        Self::scan_rows(term, rows_to_scan);
    }

    /// Scan the given visible rows of the active grid and run the actions
    /// of every match. Callers remove the rows from the pending set first,
    /// so each written row is scanned exactly once (ARC-064).
    pub(crate) fn scan_rows(term: &mut Terminal, rows_to_scan: Vec<usize>) {
        let grid = term.active_grid();
        let cols = grid.cols();

        // Collect row texts and their char-to-grid-col mappings
        let mut row_data: Vec<(usize, String, Vec<usize>)> = Vec::new();
        for row in &rows_to_scan {
            let text = grid.row_text(*row);
            if !text.trim().is_empty() {
                let r = *row;
                let mapping = build_char_to_grid_col_map(
                    cols,
                    |col| {
                        grid.get(col, r)
                            .is_some_and(|cell| cell.flags.wide_char_spacer())
                    },
                    |col| {
                        grid.get(col, r)
                            .map(|cell| 1 + cell.combining().len())
                            .unwrap_or(1)
                    },
                );
                row_data.push((*row, text, mapping));
            }
        }

        // Scan each row
        for (row, text, mapping) in &row_data {
            let matches =
                term.triggers
                    .trigger_registry
                    .scan_line(*row, text, Some(mapping.as_slice()));

            for trigger_match in &matches {
                // Execute actions for this match
                Self::execute_trigger_actions(term, trigger_match);

                // Emit event
                term.events
                    .push(crate::terminal::TerminalEvent::TriggerMatched(
                        trigger_match.clone(),
                    ));
            }
        }
    }

    /// Execute trigger actions for a match
    fn execute_trigger_actions(term: &mut Terminal, trigger_match: &TriggerMatch) {
        let trigger_id = trigger_match.trigger_id;
        let actions = match term.triggers.trigger_registry.get(trigger_id) {
            Some(t) => t.actions.clone(),
            None => return,
        };

        let now = crate::terminal::unix_millis();

        for action in &actions {
            match action {
                TriggerAction::Highlight {
                    fg,
                    bg,
                    duration_ms,
                } => {
                    let expiry = if *duration_ms == 0 {
                        u64::MAX
                    } else {
                        now.saturating_add(*duration_ms)
                    };
                    term.triggers.trigger_highlights.push(TriggerHighlight {
                        row: trigger_match.row,
                        col_start: trigger_match.col,
                        col_end: trigger_match.end_col,
                        fg: *fg,
                        bg: *bg,
                        expiry,
                    });
                    // Permanent highlights (duration_ms == 0, expiry u64::MAX)
                    // never age out, so the buffer is bounded by evicting the
                    // oldest instead (QA-197).
                    while term.triggers.trigger_highlights.len() > term.triggers.max_action_results
                    {
                        term.triggers.trigger_highlights.remove(0);
                    }
                }
                TriggerAction::Notify { title, message } => {
                    let title = substitute_captures(title, &trigger_match.captures);
                    let message = substitute_captures(message, &trigger_match.captures);
                    // Apply immediately to terminal notification state so `notifications()`
                    // reflects trigger-driven alerts, consistent with SetVariable/Highlight.
                    // The ActionResult below is retained for host-side polling.
                    term.enqueue_notification(crate::terminal::notification::Notification::new(
                        title.clone(),
                        message.clone(),
                    ));
                    term.triggers.push_action_result(ActionResult::Notify {
                        trigger_id: trigger_match.trigger_id,
                        title,
                        message,
                    });
                }
                TriggerAction::MarkLine { label, color } => {
                    let label = label
                        .as_ref()
                        .map(|l| substitute_captures(l, &trigger_match.captures));
                    // Apply immediately to terminal bookmark state so `get_bookmarks()`
                    // reflects trigger-driven marks, consistent with SetVariable/Highlight.
                    // The ActionResult below is retained for host-side polling.
                    term.add_bookmark(trigger_match.row as isize, label.clone());
                    term.triggers.push_action_result(ActionResult::MarkLine {
                        trigger_id: trigger_match.trigger_id,
                        row: trigger_match.row,
                        label,
                        color: *color,
                    });
                }
                TriggerAction::SetVariable { name, value } => {
                    let name = substitute_captures(name, &trigger_match.captures);
                    let value = substitute_captures(value, &trigger_match.captures);
                    term.badge_state
                        .session_variables
                        .custom
                        .insert(name, value);
                }
                TriggerAction::RunCommand { command, args } => {
                    let command = substitute_captures(command, &trigger_match.captures);
                    let args: Vec<String> = args
                        .iter()
                        .map(|a| substitute_captures(a, &trigger_match.captures))
                        .collect();
                    term.triggers.push_action_result(ActionResult::RunCommand {
                        trigger_id,
                        command,
                        args,
                    });
                }
                TriggerAction::PlaySound { sound_id, volume } => {
                    let sound_id = substitute_captures(sound_id, &trigger_match.captures);
                    term.triggers.push_action_result(ActionResult::PlaySound {
                        trigger_id,
                        sound_id,
                        volume: *volume,
                    });
                }
                TriggerAction::SendText { text, delay_ms } => {
                    let text = substitute_captures(text, &trigger_match.captures);
                    term.triggers.push_action_result(ActionResult::SendText {
                        trigger_id,
                        text,
                        delay_ms: *delay_ms,
                    });
                }
                TriggerAction::SplitPane {
                    direction,
                    command,
                    focus_new_pane,
                    target,
                } => {
                    term.triggers.push_action_result(ActionResult::SplitPane {
                        trigger_id,
                        direction: direction.clone(),
                        command: command.clone(),
                        focus_new_pane: *focus_new_pane,
                        target: target.clone(),
                        source_pane_id: None, // per-pane polling not yet wired
                    });
                }
                TriggerAction::StopPropagation => {
                    break;
                }
            }
        }
    }

    /// Get active trigger highlights (filters expired ones)
    pub fn get_trigger_highlights(term: &Terminal) -> Vec<TriggerHighlight> {
        let now = crate::terminal::unix_millis();
        term.triggers
            .trigger_highlights
            .iter()
            .filter(|h| h.expiry > now)
            .cloned()
            .collect()
    }

    /// Clear all trigger highlights
    pub fn clear_trigger_highlights(term: &mut Terminal) {
        term.triggers.trigger_highlights.clear();
    }

    /// Remove expired trigger highlights
    pub fn clear_expired_highlights(term: &mut Terminal) {
        let now = crate::terminal::unix_millis();
        term.triggers.trigger_highlights.retain(|h| h.expiry > now);
    }

    /// Drain pending action results for frontend consumption
    pub fn poll_action_results(term: &mut Terminal) -> Vec<ActionResult> {
        std::mem::take(&mut term.triggers.trigger_action_results)
    }

    /// Get access to the trigger registry
    pub fn trigger_registry(term: &Terminal) -> &TriggerRegistry {
        &term.triggers.trigger_registry
    }

    /// Get mutable access to the trigger registry
    pub fn trigger_registry_mut(term: &mut Terminal) -> &mut TriggerRegistry {
        &mut term.triggers.trigger_registry
    }
}

/// Result of executing trigger actions (for frontend-handled actions)
#[derive(Debug, Clone, PartialEq)]
pub enum ActionResult {
    /// Frontend should run this command
    RunCommand {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Command to run.
        command: String,
        /// Command arguments.
        args: Vec<String>,
    },
    /// Frontend should play this sound
    PlaySound {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Sound identifier.
        sound_id: String,
        /// Playback volume.
        volume: u8,
    },
    /// Frontend should send this text to the terminal
    SendText {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Text to send.
        text: String,
        /// Delay before sending, in milliseconds.
        delay_ms: u64,
    },
    /// Frontend should display a notification
    Notify {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Notification title.
        title: String,
        /// Notification body.
        message: String,
    },
    /// Frontend should add a scrollbar mark at the given row
    MarkLine {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Row to mark.
        row: usize,
        /// Optional label for the mark.
        label: Option<String>,
        /// Optional mark color (RGB).
        color: Option<(u8, u8, u8)>,
    },
    /// Frontend should open a new split pane and optionally run a command
    SplitPane {
        /// Trigger that produced this result.
        trigger_id: TriggerId,
        /// Split direction.
        direction: TriggerSplitDirection,
        /// Command to run in the new pane.
        command: Option<TriggerSplitCommand>,
        /// Whether the new pane takes focus.
        focus_new_pane: bool,
        /// Which pane to split.
        target: TriggerSplitTarget,
        /// Pane ID that generated the match. None = per-pane polling not yet available.
        source_pane_id: Option<u64>,
    },
}

/// Registry managing all triggers with RegexSet-based multi-pattern matching
pub struct TriggerRegistry {
    triggers: HashMap<TriggerId, Trigger>,
    next_id: TriggerId,
    /// Compiled RegexSet for efficient multi-pattern matching
    regex_set: Option<regex::RegexSet>,
    /// Maps RegexSet index -> TriggerId (aligned with RegexSet pattern order)
    pattern_to_id: Vec<TriggerId>,
    /// Pending match events (drained by poll)
    matches: Vec<TriggerMatch>,
    /// Maximum pending matches to retain
    max_matches: usize,
}

impl std::fmt::Debug for TriggerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerRegistry")
            .field("trigger_count", &self.triggers.len())
            .field("next_id", &self.next_id)
            .field("pending_matches", &self.matches.len())
            .finish()
    }
}

impl Default for TriggerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TriggerRegistry {
    /// Create a new empty trigger registry
    pub fn new() -> Self {
        Self {
            triggers: HashMap::new(),
            next_id: 1,
            regex_set: None,
            pattern_to_id: Vec::new(),
            matches: Vec::new(),
            max_matches: 1000,
        }
    }

    /// Add a new trigger with the given name, pattern, and actions
    ///
    /// Returns the trigger ID on success, or an error if the regex is invalid.
    pub fn add(
        &mut self,
        name: String,
        pattern: String,
        actions: Vec<TriggerAction>,
    ) -> Result<TriggerId, String> {
        let regex =
            regex::Regex::new(&pattern).map_err(|e| format!("Invalid regex pattern: {}", e))?;

        let id = self.next_id;
        self.next_id += 1;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let trigger = Trigger {
            id,
            name,
            pattern,
            regex,
            enabled: true,
            fire_once_per_line: true,
            actions,
            created: now,
            match_count: 0,
        };

        self.triggers.insert(id, trigger);
        self.rebuild_regex_set();
        Ok(id)
    }

    /// Remove a trigger by ID
    pub fn remove(&mut self, id: TriggerId) -> bool {
        if self.triggers.remove(&id).is_some() {
            self.rebuild_regex_set();
            true
        } else {
            false
        }
    }

    /// Enable or disable a trigger
    pub fn set_enabled(&mut self, id: TriggerId, enabled: bool) -> bool {
        if let Some(trigger) = self.triggers.get_mut(&id) {
            trigger.enabled = enabled;
            self.rebuild_regex_set();
            true
        } else {
            false
        }
    }

    /// Get a trigger by ID
    pub fn get(&self, id: TriggerId) -> Option<&Trigger> {
        self.triggers.get(&id)
    }

    /// Get a mutable trigger by ID
    pub fn get_mut(&mut self, id: TriggerId) -> Option<&mut Trigger> {
        self.triggers.get_mut(&id)
    }

    /// List all triggers
    pub fn list(&self) -> Vec<&Trigger> {
        let mut triggers: Vec<&Trigger> = self.triggers.values().collect();
        triggers.sort_by_key(|t| t.id);
        triggers
    }

    /// Check if any triggers are registered and enabled
    pub fn has_active_triggers(&self) -> bool {
        self.regex_set.is_some()
    }

    /// Scan a line of text for trigger matches
    ///
    /// Uses RegexSet for efficient multi-pattern matching, then runs individual
    /// regexes only on patterns that matched to extract positions and captures.
    ///
    /// `char_to_grid_col` maps each character index in `text` to its grid column
    /// index. When `None`, character indices are used directly (assumes ASCII-only,
    /// no wide characters). Callers should provide this mapping when the text may
    /// contain multi-byte UTF-8 characters or was built from grid cells with
    /// wide character spacers filtered out.
    pub fn scan_line(
        &mut self,
        row: usize,
        text: &str,
        char_to_grid_col: Option<&[usize]>,
    ) -> Vec<TriggerMatch> {
        let regex_set = match &self.regex_set {
            Some(rs) => rs,
            None => return Vec::new(),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut results = Vec::new();

        // Phase 1: RegexSet multi-pattern match (single pass over text)
        let set_matches: Vec<usize> = regex_set.matches(text).into_iter().collect();

        // Phase 2: For each matched pattern, run individual regex for captures/positions
        for set_idx in set_matches {
            let trigger_id = self.pattern_to_id[set_idx];
            let trigger = match self.triggers.get_mut(&trigger_id) {
                Some(t) => t,
                None => continue,
            };

            let regex = trigger.regex.clone();
            let fire_once = trigger.fire_once_per_line;

            // Find all matches in the line
            for caps in regex.captures_iter(text) {
                let full_match = match caps.get(0) {
                    Some(m) => m,
                    None => continue,
                };

                let mut captures = Vec::new();
                for i in 0..caps.len() {
                    captures.push(
                        caps.get(i)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }

                let (col, end_col) = byte_offsets_to_grid_cols(
                    text,
                    full_match.start(),
                    full_match.end(),
                    char_to_grid_col,
                );

                let trigger_match = TriggerMatch {
                    trigger_id,
                    row,
                    col,
                    end_col,
                    text: full_match.as_str().to_string(),
                    captures,
                    timestamp: now,
                };

                results.push(trigger_match);

                // Increment match count
                if let Some(t) = self.triggers.get_mut(&trigger_id) {
                    t.match_count += 1;
                }

                if fire_once {
                    break;
                }
            }
        }

        // Buffer matches (with capacity limit)
        for m in &results {
            if self.matches.len() >= self.max_matches {
                self.matches.remove(0);
            }
            self.matches.push(m.clone());
        }

        results
    }

    /// Drain all pending match events
    pub fn poll_matches(&mut self) -> Vec<TriggerMatch> {
        std::mem::take(&mut self.matches)
    }

    /// Set the maximum number of pending matches to retain
    pub fn set_max_matches(&mut self, max: usize) {
        self.max_matches = max;
        if self.matches.len() > max {
            let excess = self.matches.len() - max;
            self.matches.drain(0..excess);
        }
    }

    /// Clear all triggers and pending matches
    pub fn clear(&mut self) {
        self.triggers.clear();
        self.regex_set = None;
        self.pattern_to_id.clear();
        self.matches.clear();
    }

    /// Rebuild the RegexSet from all enabled triggers
    fn rebuild_regex_set(&mut self) {
        let mut patterns = Vec::new();
        let mut ids = Vec::new();

        // Collect enabled triggers in ID order for deterministic ordering
        let mut enabled: Vec<(&TriggerId, &Trigger)> =
            self.triggers.iter().filter(|(_, t)| t.enabled).collect();
        enabled.sort_by_key(|(id, _)| **id);

        for (id, trigger) in enabled {
            patterns.push(trigger.pattern.as_str());
            ids.push(*id);
        }

        if patterns.is_empty() {
            self.regex_set = None;
            self.pattern_to_id.clear();
            return;
        }

        match regex::RegexSet::new(&patterns) {
            Ok(set) => {
                self.regex_set = Some(set);
                self.pattern_to_id = ids;
            }
            Err(_) => {
                // Should not happen since individual patterns were already validated
                self.regex_set = None;
                self.pattern_to_id.clear();
            }
        }
    }
}

/// Convert regex byte offsets to grid column indices
///
/// Regex `Match::start()` and `Match::end()` return byte offsets in the UTF-8
/// string, not character or column indices. When the text contains multi-byte
/// UTF-8 characters (e.g., `❯` = 3 bytes) or was built from grid cells with
/// wide character spacers filtered out, byte offsets will not correspond to
/// grid column positions.
///
/// This function converts a (start_byte, end_byte) pair to (start_col, end_col)
/// grid column indices using an optional `char_to_grid_col` mapping.
///
/// When `char_to_grid_col` is `None`, it falls back to counting characters up to
/// each byte offset (handles multi-byte UTF-8 but not wide character spacers).
fn byte_offsets_to_grid_cols(
    text: &str,
    start_byte: usize,
    end_byte: usize,
    char_to_grid_col: Option<&[usize]>,
) -> (usize, usize) {
    match char_to_grid_col {
        Some(mapping) => {
            // Convert byte offsets to character indices, then look up grid columns
            let start_char_idx = text[..start_byte].chars().count();
            let end_char_idx = text[..end_byte].chars().count();

            let start_col = mapping
                .get(start_char_idx)
                .copied()
                .unwrap_or(start_char_idx);
            // For end_col (exclusive), we need the column *after* the last matched char.
            // If end_char_idx points past the last char, use one past the last mapped column.
            let end_col = if end_char_idx > 0 && end_char_idx <= mapping.len() {
                if end_char_idx < mapping.len() {
                    // Next character's grid column = exclusive end
                    mapping[end_char_idx]
                } else {
                    // Past the last character: use last mapped column + 1
                    mapping[end_char_idx - 1] + 1
                }
            } else if end_char_idx == 0 {
                mapping.first().copied().unwrap_or(0)
            } else {
                end_char_idx
            };

            (start_col, end_col)
        }
        None => {
            // No mapping provided: convert byte offsets to character indices
            // This handles multi-byte UTF-8 but not wide character spacers
            let start_col = text[..start_byte].chars().count();
            let end_col = text[..end_byte].chars().count();
            (start_col, end_col)
        }
    }
}

/// Build a mapping from character index (in row_text output) to grid column index
///
/// `row_text()` filters out `wide_char_spacer` cells and concatenates graphemes
/// from each remaining cell. Each cell may produce multiple characters (base char
/// plus combining characters). This function mirrors that logic to build a
/// `Vec<usize>` where index `i` is the grid column of the cell that produced the
/// `i`-th character in the resulting text.
///
/// - `is_wide_char_spacer(col)` should return `true` for spacer cells
/// - `grapheme_char_count(col)` should return the number of `char`s in the cell's
///   grapheme cluster (1 + number of combining characters)
pub fn build_char_to_grid_col_map<F, G>(
    num_cols: usize,
    is_wide_char_spacer: F,
    grapheme_char_count: G,
) -> Vec<usize>
where
    F: Fn(usize) -> bool,
    G: Fn(usize) -> usize,
{
    let mut mapping = Vec::with_capacity(num_cols);
    for col in 0..num_cols {
        if !is_wide_char_spacer(col) {
            let char_count = grapheme_char_count(col);
            for _ in 0..char_count {
                mapping.push(col);
            }
        }
    }
    mapping
}

/// Substitute capture groups in a template string
///
/// Replaces `$0`, `$1`, `$2`, etc. with the corresponding capture group values.
pub fn substitute_captures(template: &str, captures: &[String]) -> String {
    let mut result = template.to_string();
    // Replace in reverse order so $10 doesn't get partially replaced by $1
    for (i, cap) in captures.iter().enumerate().rev() {
        let placeholder = format!("${}", i);
        result = result.replace(&placeholder, cap);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_add_remove() {
        let mut registry = TriggerRegistry::new();
        let id = registry.add("test".into(), "ERROR".into(), vec![]).unwrap();
        assert!(registry.get(id).is_some());
        assert_eq!(registry.list().len(), 1);
        assert!(registry.remove(id));
        assert!(registry.get(id).is_none());
        assert_eq!(registry.list().len(), 0);
    }

    #[test]
    fn test_trigger_invalid_regex() {
        let mut registry = TriggerRegistry::new();
        let result = registry.add("bad".into(), "[invalid".into(), vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_trigger_scan_match() {
        let mut registry = TriggerRegistry::new();
        registry
            .add("error".into(), r"ERROR:\s+(.+)".into(), vec![])
            .unwrap();

        let matches = registry.scan_line(5, "prefix ERROR: something went wrong", None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].row, 5);
        assert_eq!(matches[0].text, "ERROR: something went wrong");
        assert_eq!(matches[0].captures.len(), 2); // group 0 + group 1
        assert_eq!(matches[0].captures[1], "something went wrong");
    }

    #[test]
    fn test_trigger_multi_pattern() {
        let mut registry = TriggerRegistry::new();
        registry
            .add("error".into(), "ERROR".into(), vec![])
            .unwrap();
        registry.add("warn".into(), "WARN".into(), vec![]).unwrap();

        let matches = registry.scan_line(0, "ERROR and WARN on same line", None);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_trigger_fire_once_per_line() {
        let mut registry = TriggerRegistry::new();
        registry
            .add("word".into(), r"\b\w+\b".into(), vec![])
            .unwrap();

        // fire_once_per_line is true by default, so only first word match
        let matches = registry.scan_line(0, "hello world foo", None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "hello");
    }

    #[test]
    fn test_trigger_enable_disable() {
        let mut registry = TriggerRegistry::new();
        let id = registry.add("test".into(), "MATCH".into(), vec![]).unwrap();

        // Enabled by default
        let matches = registry.scan_line(0, "MATCH here", None);
        assert_eq!(matches.len(), 1);

        // Disable
        registry.set_enabled(id, false);
        let matches = registry.scan_line(0, "MATCH here", None);
        assert_eq!(matches.len(), 0);

        // Re-enable
        registry.set_enabled(id, true);
        let matches = registry.scan_line(0, "MATCH here", None);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_trigger_capture_groups() {
        let mut registry = TriggerRegistry::new();
        registry
            .add(
                "ip".into(),
                r"(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})".into(),
                vec![],
            )
            .unwrap();

        let matches = registry.scan_line(0, "IP: 192.168.1.100", None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].captures.len(), 5); // full match + 4 groups
        assert_eq!(matches[0].captures[0], "192.168.1.100");
        assert_eq!(matches[0].captures[1], "192");
        assert_eq!(matches[0].captures[2], "168");
        assert_eq!(matches[0].captures[3], "1");
        assert_eq!(matches[0].captures[4], "100");
    }

    #[test]
    fn test_substitute_captures() {
        let captures = vec![
            "full match".to_string(),
            "group1".to_string(),
            "group2".to_string(),
        ];
        assert_eq!(
            substitute_captures("got $1 and $2", &captures),
            "got group1 and group2"
        );
        assert_eq!(substitute_captures("all: $0", &captures), "all: full match");
    }

    #[test]
    fn test_poll_matches() {
        let mut registry = TriggerRegistry::new();
        registry.add("test".into(), "MATCH".into(), vec![]).unwrap();

        registry.scan_line(0, "MATCH", None);
        registry.scan_line(1, "MATCH", None);

        let matches = registry.poll_matches();
        assert_eq!(matches.len(), 2);

        // After poll, matches are drained
        let matches = registry.poll_matches();
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_max_matches() {
        let mut registry = TriggerRegistry::new();
        registry.set_max_matches(2);
        registry.add("test".into(), "X".into(), vec![]).unwrap();

        registry.scan_line(0, "X", None);
        registry.scan_line(1, "X", None);
        registry.scan_line(2, "X", None);

        let matches = registry.poll_matches();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].row, 1);
        assert_eq!(matches[1].row, 2);
    }

    #[test]
    fn test_no_active_triggers() {
        let registry = TriggerRegistry::new();
        assert!(!registry.has_active_triggers());
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_no_mapping() {
        // Without a mapping, byte offsets are converted to char indices
        // ASCII: byte offset == char index
        let text = "hello world";
        let (start, end) = byte_offsets_to_grid_cols(text, 6, 11, None);
        assert_eq!(start, 6);
        assert_eq!(end, 11);
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_multibyte_no_mapping() {
        // Multi-byte UTF-8 without mapping: byte offsets converted to char indices
        // "❯ hello" where ❯ is 3 bytes (U+276F)
        let text = "❯ hello";
        // Byte layout: ❯(3 bytes) + ' '(1) + h(1) + e(1) + l(1) + l(1) + o(1) = 9 bytes
        // "hello" starts at byte 4, ends at byte 9
        let (start, end) = byte_offsets_to_grid_cols(text, 4, 9, None);
        // char indices: ❯=0, ' '=1, h=2, e=3, l=4, l=5, o=6
        // "hello" starts at char 2, ends at char 7
        assert_eq!(start, 2);
        assert_eq!(end, 7);
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_with_mapping_ascii() {
        // ASCII with simple 1:1 mapping (no wide chars)
        let text = "hello world";
        // mapping: each char maps to its own grid column
        let mapping: Vec<usize> = (0..11).collect();
        let (start, end) = byte_offsets_to_grid_cols(text, 6, 11, Some(&mapping));
        assert_eq!(start, 6);
        assert_eq!(end, 11);
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_with_wide_chars() {
        // Simulate: grid has cells [W][spacer][o][r][l][d]
        // row_text produces "World" (W from wide char + "orld")
        // But W is a wide character at grid column 0, spacer at column 1
        // 'o' is at grid column 2, 'r' at 3, 'l' at 4, 'd' at 5
        let text = "World";
        // mapping: char_idx -> grid_col
        // W(char 0) -> col 0 (wide char, spacer at col 1 filtered)
        // o(char 1) -> col 2
        // r(char 2) -> col 3
        // l(char 3) -> col 4
        // d(char 4) -> col 5
        let mapping = vec![0, 2, 3, 4, 5];

        // Match "orld" at bytes 1..5 (ASCII, so byte == char index here)
        let (start, end) = byte_offsets_to_grid_cols(text, 1, 5, Some(&mapping));
        // char 1 -> grid col 2, char 5 is past end -> last col + 1 = 5 + 1 = 6
        assert_eq!(start, 2);
        assert_eq!(end, 6);
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_multibyte_with_mapping() {
        // "❯ hello" where ❯ is 3 bytes, occupies 1 grid column
        let text = "❯ hello";
        // Grid: [❯][space][h][e][l][l][o]
        // No wide char spacers, but ❯ is multi-byte
        let mapping: Vec<usize> = (0..7).collect();

        // Match "hello" - starts at byte 5 (after ❯(3) + space(1) + h at byte 4... wait)
        // ❯ = 3 bytes, space = 1 byte, so "hello" starts at byte 4
        // Actually: ❯(3 bytes) + ' '(1 byte) = 4 bytes, then 'h' at byte 4
        let (start, end) = byte_offsets_to_grid_cols(text, 4, 9, Some(&mapping));
        // chars: ❯=0, ' '=1, h=2, e=3, l=4, l=5, o=6
        // char 2 -> grid col 2, char 7 is past end -> last + 1 = 7
        assert_eq!(start, 2);
        assert_eq!(end, 7);
    }

    #[test]
    fn test_byte_offsets_to_grid_cols_multibyte_with_wide_and_mapping() {
        // Simulate prompt: "❯ " followed by "test"
        // Grid: [❯(wide)][spacer][ ][t][e][s][t]
        // row_text filters spacer: "❯ test"
        // ❯ is 3-byte UTF-8 and a wide character occupying 2 grid columns
        let text = "❯ test";
        // mapping: char_idx -> grid_col
        // ❯(char 0) -> col 0 (wide char, spacer at col 1 filtered)
        // ' '(char 1) -> col 2
        // t(char 2) -> col 3
        // e(char 3) -> col 4
        // s(char 4) -> col 5
        // t(char 5) -> col 6
        let mapping = vec![0, 2, 3, 4, 5, 6];

        // Match "test": ❯ is 3 bytes, ' ' is 1 byte = 4 bytes, so "test" at bytes 4..8
        let (start, end) = byte_offsets_to_grid_cols(text, 4, 8, Some(&mapping));
        // char 2 -> grid col 3, char 6 is past end -> last + 1 = 7
        assert_eq!(start, 3);
        assert_eq!(end, 7);
    }

    #[test]
    fn test_build_char_to_grid_col_map_no_spacers() {
        // Simple case: 5 columns, no spacers, 1 char per cell
        let mapping = build_char_to_grid_col_map(5, |_| false, |_| 1);
        assert_eq!(mapping, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_build_char_to_grid_col_map_with_spacer() {
        // Grid: [W][spacer][a][b][c]
        // W is wide at col 0, spacer at col 1
        let mapping = build_char_to_grid_col_map(5, |col| col == 1, |_| 1);
        assert_eq!(mapping, vec![0, 2, 3, 4]);
    }

    #[test]
    fn test_build_char_to_grid_col_map_with_combining() {
        // Grid: [e\u{0301}][a][b] where cell 0 has a combining accent (2 chars)
        let mapping = build_char_to_grid_col_map(3, |_| false, |col| if col == 0 { 2 } else { 1 });
        // Cell 0 produces 2 chars, both map to col 0
        // Cell 1 produces 1 char mapping to col 1
        // Cell 2 produces 1 char mapping to col 2
        assert_eq!(mapping, vec![0, 0, 1, 2]);
    }

    #[test]
    fn test_build_char_to_grid_col_map_wide_and_combining() {
        // Grid: [W+combining][spacer][a][b]
        // Cell 0: wide char with 1 combining char (2 chars total), grid col 0
        // Cell 1: spacer, grid col 1 (filtered out)
        // Cell 2: 'a', grid col 2
        // Cell 3: 'b', grid col 3
        let mapping =
            build_char_to_grid_col_map(4, |col| col == 1, |col| if col == 0 { 2 } else { 1 });
        assert_eq!(mapping, vec![0, 0, 2, 3]);
    }

    #[test]
    fn test_scan_line_with_multibyte_chars_returns_grid_cols() {
        // Verify that scan_line with a mapping returns grid column indices
        let mut registry = TriggerRegistry::new();
        registry.add("test".into(), "ERROR".into(), vec![]).unwrap();

        // "❯ ERROR" where ❯ is 3 bytes
        let text = "❯ ERROR";
        // Mapping: ❯=col0, ' '=col1, E=col2, R=col3, R=col4, O=col5, R=col6
        let mapping: Vec<usize> = (0..7).collect();

        let matches = registry.scan_line(0, text, Some(&mapping));
        assert_eq!(matches.len(), 1);
        // "ERROR" in the string: byte 4 to 9 (❯ is 3 bytes + space is 1 byte)
        // char index 2 to 7 -> grid cols 2 to 7
        assert_eq!(matches[0].col, 2);
        assert_eq!(matches[0].end_col, 7);
    }

    #[test]
    fn test_scan_line_with_wide_char_mapping() {
        let mut registry = TriggerRegistry::new();
        registry.add("test".into(), "ERROR".into(), vec![]).unwrap();

        // Simulate: wide char at col 0 (spacer at col 1), then " ERROR"
        // row_text = "W ERROR" (W is 1 char from the wide cell)
        let text = "W ERROR";
        // mapping: W=col0 (spacer at col1 filtered), ' '=col2, E=col3, R=col4, R=col5, O=col6, R=col7
        let mapping = vec![0, 2, 3, 4, 5, 6, 7];

        let matches = registry.scan_line(0, text, Some(&mapping));
        assert_eq!(matches.len(), 1);
        // "ERROR" at bytes 2..7, chars 2..7 -> grid cols 3..8
        assert_eq!(matches[0].col, 3);
        assert_eq!(matches[0].end_col, 8);
    }
}
