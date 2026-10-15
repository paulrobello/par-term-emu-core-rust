//! `handle_event`'s payload-patch arms: events that carry their own fact
//! patch the cached status state in place instead of re-querying.

use super::WindowSession;

impl WindowSession {
    /// A pane's title changed.
    pub(super) fn on_pane_title_changed(&mut self, pane_id: &str, title: &str) {
        let Some(pane) = pane_id
            .strip_prefix('%')
            .and_then(|p| p.parse::<u32>().ok())
        else {
            return;
        };
        let title = title.trim();
        if title.is_empty() {
            // A cleared user title falls back to the program's OSC
            // title, which the payload does not carry: re-query
            // that pane on the next refresh.
            self.renderer.invalidate_title(pane);
            self.status_dirty = true;
            return;
        }
        // The payload IS the new effective title (a user title
        // wins over OSC): patch it without a query (ENH-043).
        if self.renderer.layout().iter().any(|r| r.pane == pane) {
            self.renderer.set_user_title(pane, title);
        }
        if self.renderer.focused() == Some(pane) {
            self.status.set_pane_title(title);
            self.draw_status_row();
        }
    }

    /// An agent's state changed (`state` is `Some`) or it was released
    /// (`None`): patch the cached roster and rebuild the side panel's
    /// agents section from it, with no re-query (ENH-043). The status row
    /// shows no agents.
    pub(super) fn on_agent_changed(
        &mut self,
        pane_id: &str,
        agent: &str,
        state: Option<&str>,
        ws: &str,
    ) {
        // The notification compares against the state BEFORE the patch.
        if let (Some(state), Some(pane)) = (
            state,
            pane_id
                .strip_prefix('%')
                .and_then(|p| p.parse::<u32>().ok()),
        ) {
            let previous = self
                .status
                .agents()
                .iter()
                .find(|entry| entry.pane == pane)
                .map(|entry| entry.state.clone());
            self.notify_agent_state(pane, agent, previous.as_deref(), state, ws);
        }
        let patched = self.status.patch_agent(pane_id, agent, state, ws);
        if state.is_some() && patched {
            self.status_dirty = true;
        }
        self.rebuild_agent_section();
        self.draw_status_row();
    }

    /// A pane's process exited and the daemon holds it (`%pane-exited`,
    /// live or from the registration replay): remember the code for the
    /// exit cue and the respawn chord. Only the shown window's panes
    /// change the status row.
    pub(super) fn on_pane_exited(&mut self, pane_id: &str, exit_code: Option<i32>) {
        let Some(pane) = pane_id
            .strip_prefix('%')
            .and_then(|p| p.parse::<u32>().ok())
        else {
            return;
        };
        self.held.insert(pane, exit_code);
        if self.renderer.layout().iter().any(|r| r.pane == pane) {
            self.draw_status_row();
        }
    }

    /// A held pane was restarted (`%pane-respawned`): drop its exit cue
    /// and park the emulator reset + replay for the pump (the replay
    /// needs the connection).
    pub(super) fn on_pane_respawned(&mut self, pane_id: &str) {
        let Some(pane) = pane_id
            .strip_prefix('%')
            .and_then(|p| p.parse::<u32>().ok())
        else {
            return;
        };
        self.held.remove(&pane);
        if self.renderer.layout().iter().any(|r| r.pane == pane) {
            if !self.pending.respawned.contains(&pane) {
                self.pending.respawned.push(pane);
            }
            self.draw_status_row();
        }
    }

    /// The status row's exit cue for the shown window: the focused pane's
    /// when it is held, else the first held pane in layout order —
    /// `exited N — <prefix> <respawn> respawns` (passthrough's wording).
    pub(super) fn exit_cue(&self) -> Option<String> {
        let focused = self.renderer.focused();
        let held = focused
            .filter(|pane| self.held.contains_key(pane))
            .or_else(|| {
                self.renderer
                    .layout()
                    .iter()
                    .map(|r| r.pane)
                    .find(|pane| self.held.contains_key(pane))
            })?;
        let code = match self.held.get(&held).copied().flatten() {
            Some(code) => code.to_string(),
            None => "?".to_string(),
        };
        let chord = format!(
            "{} {}",
            crate::mux::attach::panels::spell_key(self.prefix),
            crate::mux::attach::panels::spell_key(self.management.respawn)
        );
        if Some(held) == focused {
            Some(format!("(exited {code} — {chord} respawns)"))
        } else {
            Some(format!("(%{held} exited {code})"))
        }
    }

    /// A window was renamed.
    pub(super) fn on_window_renamed(&mut self, window_id: &str, name: &str) {
        self.status.patch_window_name(window_id, name);
        self.draw_status_row();
        self.draw_tab_strip();
    }
}
