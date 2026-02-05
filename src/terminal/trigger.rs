//! Trigger system for automated pattern matching on terminal output
//!
//! Provides regex-based triggers that scan terminal lines for patterns and execute
//! configurable actions (highlight, notify, bookmark, set variable, etc.).
//! Uses `RegexSet` for efficient multi-pattern matching in a single pass.

use std::collections::HashMap;

/// Unique trigger identifier
pub type TriggerId = u64;

/// Action to execute when a trigger matches
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerAction {
    /// Highlight matched text with specified colors and optional duration
    Highlight {
        fg: Option<(u8, u8, u8)>,
        bg: Option<(u8, u8, u8)>,
        /// Duration in milliseconds (0 = permanent)
        duration_ms: u64,
    },
    /// Send a notification (reuses existing notification system)
    Notify { title: String, message: String },
    /// Add a bookmark/mark on the matched line
    MarkLine { label: Option<String> },
    /// Set a session variable (reuses existing badge session_variables)
    SetVariable { name: String, value: String },
    /// Run an external command (emitted as event for frontend)
    RunCommand { command: String, args: Vec<String> },
    /// Play a sound (emitted as event for frontend)
    PlaySound { sound_id: String, volume: u8 },
    /// Send text to the terminal (emitted as event for frontend)
    SendText { text: String, delay_ms: u64 },
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
    /// Row where the match occurred
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

/// Result of executing trigger actions (for frontend-handled actions)
#[derive(Debug, Clone, PartialEq)]
pub enum ActionResult {
    /// Frontend should run this command
    RunCommand {
        trigger_id: TriggerId,
        command: String,
        args: Vec<String>,
    },
    /// Frontend should play this sound
    PlaySound {
        trigger_id: TriggerId,
        sound_id: String,
        volume: u8,
    },
    /// Frontend should send this text to the terminal
    SendText {
        trigger_id: TriggerId,
        text: String,
        delay_ms: u64,
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
    pub fn scan_line(&mut self, row: usize, text: &str) -> Vec<TriggerMatch> {
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

                let trigger_match = TriggerMatch {
                    trigger_id,
                    row,
                    col: full_match.start(),
                    end_col: full_match.end(),
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

        let matches = registry.scan_line(5, "prefix ERROR: something went wrong");
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

        let matches = registry.scan_line(0, "ERROR and WARN on same line");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_trigger_fire_once_per_line() {
        let mut registry = TriggerRegistry::new();
        registry
            .add("word".into(), r"\b\w+\b".into(), vec![])
            .unwrap();

        // fire_once_per_line is true by default, so only first word match
        let matches = registry.scan_line(0, "hello world foo");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "hello");
    }

    #[test]
    fn test_trigger_enable_disable() {
        let mut registry = TriggerRegistry::new();
        let id = registry.add("test".into(), "MATCH".into(), vec![]).unwrap();

        // Enabled by default
        let matches = registry.scan_line(0, "MATCH here");
        assert_eq!(matches.len(), 1);

        // Disable
        registry.set_enabled(id, false);
        let matches = registry.scan_line(0, "MATCH here");
        assert_eq!(matches.len(), 0);

        // Re-enable
        registry.set_enabled(id, true);
        let matches = registry.scan_line(0, "MATCH here");
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

        let matches = registry.scan_line(0, "IP: 192.168.1.100");
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

        registry.scan_line(0, "MATCH");
        registry.scan_line(1, "MATCH");

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

        registry.scan_line(0, "X");
        registry.scan_line(1, "X");
        registry.scan_line(2, "X");

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
}
