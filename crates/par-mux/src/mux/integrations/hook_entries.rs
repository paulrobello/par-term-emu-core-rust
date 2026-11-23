//! The shared merge/unmerge loop behind the claude and codex hook
//! installers: each agent supplies its ordered entries as data, and these
//! helpers fold them into the agent's settings text one event at a time.

use std::io;
use std::path::Path;

use super::{merge_hook_event, unmerge_hook_entries};

/// One settings entry par-mux installs: the hook event it lives under, the
/// command string, and the optional matcher (claude's SessionStart sources).
pub(super) struct HookEntry {
    pub(super) event: &'static str,
    pub(super) command: String,
    pub(super) matcher: Option<&'static str>,
}

impl HookEntry {
    /// An entry with no matcher: it applies to every invocation of the event.
    pub(super) fn any(event: &'static str, command: String) -> Self {
        Self {
            event,
            command,
            matcher: None,
        }
    }
}

/// Merge every entry into `content` in order, each against the sequentially
/// updated text, so a fresh install (or an upgrade from an older entry set)
/// splices in one write. Returns the final text and whether any entry
/// changed it.
pub(super) fn merge_entries(
    content: String,
    settings_path: &Path,
    entries: &[HookEntry],
) -> io::Result<(String, bool)> {
    let mut current = content;
    let mut changed = false;
    for entry in entries {
        current = match merge_hook_event(
            &current,
            settings_path,
            &entry.command,
            entry.matcher,
            entry.event,
        )? {
            Some(updated) => {
                changed = true;
                updated
            }
            None => current,
        };
    }
    Ok((current, changed))
}

/// Remove every entry from `content`. `Ok(None)` = nothing to remove.
pub(super) fn unmerge_entries(
    content: &str,
    settings_path: &Path,
    entries: &[HookEntry],
) -> io::Result<Option<String>> {
    let pairs: Vec<(&str, &str)> = entries
        .iter()
        .map(|entry| (entry.event, entry.command.as_str()))
        .collect();
    unmerge_hook_entries(content, settings_path, &pairs)
}
