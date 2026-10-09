//! `pane.release_agent`: the claiming agent's teardown report.

use super::{clear_agent_claim, error_reply, is_stale, ok_reply, parse_header};
use crate::mux::tree::MuxTree;
use par_term_emu_core::tmux_control::TmuxNotification;
use parking_lot::Mutex;
use std::sync::Arc;

/// `pane.release_agent`: the claiming agent announces it is gone (herdr's
/// SessionEnd shape). The claim — label, state, hook authority, blocked
/// reason, sequence stamps, and session identity — is cleared and the
/// removal broadcast, so the roster drops the pane instead of showing a
/// dead agent "working" until the pane dies, and a restart respawns the
/// pane's original command rather than a resume invocation for a session
/// that no longer has a live agent.
///
/// Guards: the releasing agent must match the pane's current label (a
/// stale hook from a different agent cannot wipe a live claim), and the
/// report must clear the same monotonic-`seq` rule as every other report.
/// Both failing guards are silent ok no-ops, exactly like a stale report.
pub(super) fn handle_release_report(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    tree: &Arc<Mutex<MuxTree>>,
) -> (String, Option<TmuxNotification>) {
    let header = match parse_header(params) {
        Ok(header) => header,
        Err(message) => return (error_reply(id, &message), None),
    };
    let notification = {
        let mut guard = tree.lock();
        let Some(pane) = guard.pane_mut(header.pane_id) else {
            return (
                error_reply(id, &format!("no such pane: {}", header.pane_id)),
                None,
            );
        };
        if is_stale(pane, header.source.as_deref(), header.seq) {
            return (ok_reply(id), None);
        }
        if pane.metadata().get("agent").map(String::as_str) != Some(header.agent.as_str()) {
            return (ok_reply(id), None);
        }
        clear_agent_claim(pane);
        Some(TmuxNotification::AgentReleased {
            pane_id: header.pane_id.to_string(),
            agent: header.agent.clone(),
        })
    };
    (ok_reply(id), notification)
}
