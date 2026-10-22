//! The `done` state: an agent that finished while nobody was looking.
//!
//! herdr's model (`api_helpers::pane_agent_status`, `apply_pane_state_change`):
//! an agent returning to idle from working or blocked in a pane the user is
//! not viewing is `done` until the pane is viewed, then `idle`. Hooks never
//! report `done` in herdr; it is derived from the transition. par-mux
//! derives it the same way and additionally accepts an explicit `done` from
//! a hook or a scrape rule.
//!
//! "Viewed" is decided daemon-side so every client agrees: a pane is in
//! view when it is the pane some render client shows — the zoomed pane,
//! else the active pane, of the client's focused window. The focus is the
//! window the client last selected for itself (`select-window`,
//! `switch-client -t`, `select-workspace`), a shared-selection follow, or
//! its latest size report (`refresh-client -t <pane> -C`), whichever came
//! last — except that a report naming another window does not move a
//! focus the client's own selection placed: that report was sent for the
//! window it left ([`MuxTree::set_client_focus`]). The pane rule makes a
//! passthrough client, which reports a whole window while showing its
//! active pane, view that pane alone. Only connections holding a size
//! report are viewers; one-shot CLI and hook connections never report
//! one, so they never mark anything seen.

use crate::mux::ids::PaneId;
use crate::mux::tree::MuxTree;
use par_term_emu_core::tmux_control::TmuxNotification;

/// The panes the attached render clients show: per viewer, the zoomed
/// pane of its focused window, else that window's active pane.
fn viewed_panes(tree: &MuxTree) -> Vec<PaneId> {
    let mut viewed: Vec<PaneId> = tree
        .client_views
        .keys()
        .filter_map(|client_id| tree.client_focus_window(*client_id))
        .filter_map(|window_id| tree.window(window_id))
        .map(|window| window.zoomed.unwrap_or(window.active))
        .collect();
    viewed.sort_unstable();
    viewed.dedup();
    viewed
}

/// Whether any attached render client currently shows `pane`.
pub(crate) fn pane_in_view(tree: &MuxTree, pane: PaneId) -> bool {
    viewed_panes(tree).contains(&pane)
}

/// The state to store for an incoming `incoming` state given the pane's
/// `prior` stored state and whether it is in view.
pub(crate) fn derive_state<'a>(prior: Option<&str>, incoming: &'a str, in_view: bool) -> &'a str {
    match incoming {
        "idle"
            if !in_view && matches!(prior, Some("working" | "blocked" | "question" | "done")) =>
        {
            "done"
        }
        "done" if in_view => "idle",
        _ => incoming,
    }
}

/// Clear every viewed pane's `done` to `idle` and return the
/// `%agent-state-changed` pushes announcing it. Only `agent_state` is
/// written: provenance, sequence stamps, and the message survive.
pub(crate) fn mark_viewed_done_seen(tree: &mut MuxTree) -> Vec<TmuxNotification> {
    let mut notifications = Vec::new();
    for pane_id in viewed_panes(tree) {
        let ws = tree
            .workspace_of_pane(pane_id)
            .map(|w| w.to_string())
            .unwrap_or_default();
        let Some(pane) = tree.pane_mut(pane_id) else {
            continue;
        };
        let metadata = pane.metadata();
        if metadata.get("agent_state").map(String::as_str) != Some("done") {
            continue;
        }
        let Some(agent) = metadata.get("agent").cloned() else {
            continue;
        };
        let source = metadata
            .get("agent_state_source")
            .cloned()
            .unwrap_or_else(|| "hook".to_string());
        let msg = metadata.get("agent_message").cloned().unwrap_or_default();
        pane.set_metadata("agent_state", "idle");
        notifications.push(TmuxNotification::AgentStateChanged {
            pane_id: pane_id.to_string(),
            agent,
            state: "idle".to_string(),
            source,
            ws,
            msg,
        });
    }
    notifications
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_after_work_out_of_view_is_done_and_in_view_is_idle() {
        for prior in ["working", "blocked", "question", "done"] {
            assert_eq!(derive_state(Some(prior), "idle", false), "done", "{prior}");
            assert_eq!(derive_state(Some(prior), "idle", true), "idle", "{prior}");
        }
    }

    #[test]
    fn a_first_idle_or_idle_after_idle_never_becomes_done() {
        assert_eq!(derive_state(None, "idle", false), "idle");
        assert_eq!(derive_state(Some("idle"), "idle", false), "idle");
    }

    #[test]
    fn an_explicit_done_holds_out_of_view_and_clears_in_view() {
        assert_eq!(derive_state(None, "done", false), "done");
        assert_eq!(derive_state(Some("working"), "done", true), "idle");
    }

    #[test]
    fn working_and_blocked_pass_through() {
        for state in ["working", "blocked"] {
            assert_eq!(derive_state(Some("done"), state, false), state);
            assert_eq!(derive_state(Some("idle"), state, true), state);
        }
    }
}
