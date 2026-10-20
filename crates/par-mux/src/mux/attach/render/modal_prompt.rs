//! The rename/create prompt: enter/leave, the seeded text, its
//! refresh, keys and bytes, and the commit paths per target.

use super::modal_settings::SettingKind;
use super::{Modal, PromptState, PromptTarget, WindowSession};
use crate::mux::attach::panels::{
    compose_prompt_panel, next_window_name, next_workspace_name, wire_quote, NEW_PROMPT_FOOTER,
    PROMPT_FOOTER, PROMPT_NEW_WINDOW_OVERLAY_TITLE, PROMPT_NEW_WORKSPACE_OVERLAY_TITLE,
    PROMPT_PANE_OVERLAY_TITLE, PROMPT_SETTING_OVERLAY_TITLE, PROMPT_WINDOW_OVERLAY_TITLE,
    PROMPT_WORKSPACE_OVERLAY_TITLE,
};
use par_term_emu_core::keyboard::TermKeyEvent;

impl WindowSession {
    /// Open the prompt modal for `target`: the rename flows seed the
    /// target's current name — the window's or workspace's own name (a
    /// context menu may target a window/workspace the view is not
    /// showing) — and the create flows seed the next free index.
    pub(super) fn enter_prompt(&mut self, target: PromptTarget) {
        let seed = match &target {
            PromptTarget::Pane => self.status.pane_title().to_string(),
            PromptTarget::Window(id) => self
                .status
                .windows()
                .iter()
                .find(|(wid, _)| wid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWindow => next_window_name(self.status.windows()),
            PromptTarget::Workspace(id) => self
                .status
                .workspaces()
                .iter()
                .find(|(wsid, _)| wsid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWorkspace => next_workspace_name(self.status.workspaces()),
            // The settings rows open through `enter_setting_prompt`
            // (which owns the seed); this arm is unreachable.
            PromptTarget::Setting(_) => String::new(),
        };
        self.open_modal(Modal::Prompt(PromptState { text: seed, target }));
        self.refresh_prompt();
    }
    /// Dismiss the prompt: the next frame's pane repaint restores the
    /// covered cells. A settings-row prompt returns to the settings
    /// dialog instead of dismissing it.
    pub(super) fn leave_prompt(&mut self) {
        if let Modal::Prompt(prompt) = &self.modal {
            if let PromptTarget::Setting(edit) = &prompt.target {
                let back = edit.back.clone();
                self.modal = Modal::Settings(back);
                self.refresh_settings();
                return;
            }
        }
        if matches!(self.modal, Modal::Prompt(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }
    /// Re-compose the prompt overlay from the edit buffer.
    pub(super) fn refresh_prompt(&mut self) {
        let Modal::Prompt(prompt) = &self.modal else {
            return;
        };
        let title = match &prompt.target {
            PromptTarget::Pane => PROMPT_PANE_OVERLAY_TITLE,
            PromptTarget::Window(_) => PROMPT_WINDOW_OVERLAY_TITLE,
            PromptTarget::NewWindow => PROMPT_NEW_WINDOW_OVERLAY_TITLE,
            PromptTarget::Workspace(_) => PROMPT_WORKSPACE_OVERLAY_TITLE,
            PromptTarget::NewWorkspace => PROMPT_NEW_WORKSPACE_OVERLAY_TITLE,
            PromptTarget::Setting(_) => PROMPT_SETTING_OVERLAY_TITLE,
        };
        let footer = match prompt.target {
            PromptTarget::NewWindow | PromptTarget::NewWorkspace => NEW_PROMPT_FOOTER,
            PromptTarget::Setting(_) => NEW_PROMPT_FOOTER,
            _ => PROMPT_FOOTER,
        };
        self.renderer.set_overlay(Some((
            title,
            compose_prompt_panel(&prompt.text, footer),
            None,
        )));
    }
    /// One plain byte while the prompt is up: printable bytes and
    /// spaces append, Backspace pops, ^C clears the input (herdr's
    /// footer), Enter commits (the daemon command below), Escape
    /// cancels. Returns whether the mode is still up.
    pub(super) fn prompt_byte(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        byte: u8,
    ) -> bool {
        match byte {
            0x1b => self.leave_prompt(),
            b'\r' => self.commit_prompt(conn),
            0x03 => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.clear();
                }
                self.refresh_prompt();
            }
            0x7f => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.pop();
                }
                self.refresh_prompt();
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.push(b as char);
                }
                self.refresh_prompt();
            }
            b if b >= 0x80 => {
                if let (Some(ch), Modal::Prompt(prompt)) =
                    (self.text_utf8.push_printable(b), &mut self.modal)
                {
                    prompt.text.push(ch);
                    self.refresh_prompt();
                }
            }
            _ => {}
        }
        matches!(self.modal, Modal::Prompt(_))
    }
    /// One key event while the prompt is up: characters append, Escape
    /// cancels, everything else is consumed (the byte path carries the
    /// controls).
    pub(super) fn prompt_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        let _ = conn;
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Char => {
                if let (Some(ch), Modal::Prompt(prompt)) =
                    (char::from_u32(ev.codepoint), &mut self.modal)
                {
                    prompt.text.push(ch);
                }
            }
            TermKey::Escape => {
                self.leave_prompt();
                return;
            }
            _ => {}
        }
        self.refresh_prompt();
    }
    /// Commit the prompt: the pane spelling sets the sticky user title
    /// (`select-pane -T`), the window and workspace spellings rename
    /// their target (`rename-window`/`rename-workspace`, quoted), the
    /// create flows make the window/workspace and land the view on it;
    /// an empty input cancels (an empty name is not expressible on the
    /// wire — the parses require one).
    pub(super) fn commit_prompt(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Prompt(prompt) = &self.modal else {
            return;
        };
        let text = prompt.text.trim().to_string();
        let target = prompt.target.clone();
        // An empty commit clears a hex color override (the built-in
        // look); for every other target it cancels.
        let empty_clears = matches!(
            &target,
            PromptTarget::Setting(edit)
                if matches!(
                    edit.kind,
                    SettingKind::Hex | SettingKind::Text | SettingKind::List | SettingKind::Popup(_)
                )
        );
        if text.is_empty() && !empty_clears {
            self.leave_prompt();
            return;
        }
        match target {
            PromptTarget::Setting(edit) => {
                // Validates first: a bad value flashes and the prompt
                // stays up; a good one writes, reloads, and restores the
                // dialog (the same restore as leave_prompt).
                self.setting_prompt_commit(conn, &edit, &text);
            }
            PromptTarget::Pane => {
                let command = format!(
                    "select-pane -t {} -T {}",
                    self.focused_pane(),
                    wire_quote(&text)
                );
                let _ = conn.send_forget(&command);
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Window(id) => {
                // rename-window takes the REST OF THE LINE verbatim (the
                // daemon grammar's documented exception) — quoting would
                // become part of the name (the manual-pass round-8
                // report: renamed tabs wore literal quotes).
                let _ = conn.send_forget(&format!("rename-window -t {id} {text}"));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Workspace(id) => {
                let _ =
                    conn.send_forget(&format!("rename-workspace -t {id} {}", wire_quote(&text)));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    self.leave_prompt();
                    return;
                };
                self.send_then(
                    conn,
                    &format!("new-window -t {session} -n {}", wire_quote(&text)),
                    |s, c, reply| {
                        if !reply.ok {
                            return;
                        }
                        if let Some(window) = reply.body.first().map(|w| w.trim().to_string()) {
                            s.land_window(c, &window);
                        }
                    },
                );
                self.leave_prompt();
            }
            PromptTarget::NewWorkspace => {
                self.send_then(
                    conn,
                    &format!("new-workspace -n {}", wire_quote(&text)),
                    |s, c, reply| {
                        if !reply.ok {
                            return;
                        }
                        if let Some(workspace) = reply.body.first().map(|w| w.trim().to_string()) {
                            // In-order daemon: the roster queries inside the
                            // landing observe the select.
                            if c.send_forget(&format!("select-workspace -t {workspace}"))
                                .is_err()
                            {
                                return;
                            }
                            s.land_on_workspace(c, &workspace);
                        }
                    },
                );
                self.leave_prompt();
            }
        }
    }
}
