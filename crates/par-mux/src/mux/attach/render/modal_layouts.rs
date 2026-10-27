//! The layouts dialog (the command menu's `Layouts…`): save the live
//! tree under a name, or load a saved layout over it. Every overwrite
//! and every replace of live panes asks first; Escape backs out one
//! level at every state. The daemon verbs (`save-layout`, `list-layouts`,
//! `load-layout <name> -f`) ride the control socket; the dialog always
//! confirms before sending the forced load.

use super::{Modal, WindowSession};
use crate::mux::attach::conn::AttachConn;
use crate::mux::attach::panels::{compose_prompt_panel, HelpRow};
use crate::mux::attach::text_input::display_width;
use par_term_emu_core::keyboard::TermKeyEvent;

/// The dialog's overlay title.
pub(super) const LAYOUTS_TITLE: &str = " layouts ";
/// The saved-layout rows shown at once; the list scrolls with the cursor.
const PICK_ROWS: usize = 10;

const CHOOSE_FOOTER: &str = " navigate arrows/j/k · select enter · close esc/q ";
const SAVE_FOOTER: &str = " save enter · back esc ";
const CONFIRM_FOOTER: &str = " confirm enter · back esc ";
const PICK_FOOTER: &str = " navigate arrows/j/k · load enter · back esc ";

/// Where the dialog is in its save/load flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LayoutsState {
    /// The `Save…` / `Load…` rows.
    Choose { selected: usize },
    /// Typing the name to save under.
    SavePrompt { text: String },
    /// The name exists: overwrite it?
    SaveConfirmOverwrite { name: String },
    /// The saved layouts (empty: the `(no saved layouts)` row).
    LoadPick { names: Vec<String>, selected: usize },
    /// Replace the live panes with this layout?
    LoadConfirm { name: String },
}

fn row(text: String, accent: bool, footer: bool) -> HelpRow {
    HelpRow {
        text,
        accent,
        footer,
        render_only: false,
    }
}

/// The first visible list row for a cursor at `selected`.
fn pick_top(selected: usize) -> usize {
    (selected + 1).saturating_sub(PICK_ROWS)
}

/// The panel rows and the selected row's index (`None` when no row is
/// highlighted). Pure over the state — the unit-test surface.
fn compose_layouts(state: &LayoutsState) -> (Vec<HelpRow>, Option<usize>) {
    match state {
        LayoutsState::Choose { selected } => (
            vec![
                row(" Save… ".to_string(), false, false),
                row(" Load… ".to_string(), false, false),
                row(CHOOSE_FOOTER.to_string(), false, true),
            ],
            Some(*selected),
        ),
        LayoutsState::SavePrompt { text } => (compose_prompt_panel(text, SAVE_FOOTER), None),
        LayoutsState::SaveConfirmOverwrite { name } => (
            vec![
                row(format!(" Overwrite layout `{name}`? "), true, false),
                row(String::new(), false, false),
                row(CONFIRM_FOOTER.to_string(), false, true),
            ],
            None,
        ),
        LayoutsState::LoadPick { names, selected } => {
            let mut rows: Vec<HelpRow> = if names.is_empty() {
                vec![row(" (no saved layouts) ".to_string(), false, false)]
            } else {
                names
                    .iter()
                    .skip(pick_top(*selected))
                    .take(PICK_ROWS)
                    .map(|name| row(format!(" {name} "), false, false))
                    .collect()
            };
            rows.push(row(PICK_FOOTER.to_string(), false, true));
            let at = (!names.is_empty()).then(|| selected - pick_top(*selected));
            (rows, at)
        }
        LayoutsState::LoadConfirm { name } => (
            vec![
                row(
                    format!(" Replace current panes with layout `{name}`? "),
                    true,
                    false,
                ),
                row(String::new(), false, false),
                row(CONFIRM_FOOTER.to_string(), false, true),
            ],
            None,
        ),
    }
}

impl WindowSession {
    /// Open the dialog on its `Choose` state.
    pub(super) fn enter_layouts(&mut self) {
        self.open_modal(Modal::Layouts(LayoutsState::Choose { selected: 0 }));
        self.refresh_layouts();
    }

    /// Re-compose the overlay from the dialog's state.
    pub(super) fn refresh_layouts(&mut self) {
        let Modal::Layouts(state) = &self.modal else {
            return;
        };
        let (rows, selected) = compose_layouts(state);
        let highlights = selected
            .and_then(|at| rows.get(at).map(|r| (at, 0, display_width(&r.text))))
            .into_iter()
            .collect();
        self.renderer.set_overlay(Some((LAYOUTS_TITLE, rows, None)));
        self.renderer.set_overlay_highlights(highlights);
    }

    /// Close the dialog.
    pub(super) fn leave_layouts(&mut self) {
        if matches!(self.modal, Modal::Layouts(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    fn set_layouts_state(&mut self, state: LayoutsState) {
        self.modal = Modal::Layouts(state);
        self.refresh_layouts();
    }

    /// The saved layout names, via `list-layouts`. A failed query
    /// flashes and yields `None` so the flow stays where it is.
    fn query_layouts(&mut self, conn: &mut AttachConn) -> Option<Vec<String>> {
        match conn.send_interactive("list-layouts") {
            Ok(reply) if reply.ok => Some(
                reply
                    .body
                    .iter()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect(),
            ),
            Ok(reply) => {
                let why = reply.body.first().cloned().unwrap_or_default();
                self.chrome.flash(format!("layouts: {why}"));
                self.draw_status_row();
                None
            }
            Err(err) => {
                self.flash_if_busy(&err);
                None
            }
        }
    }

    /// Send `command` and flash its outcome on the status row.
    fn send_layout_command(&mut self, conn: &mut AttachConn, command: &str, done: String) {
        self.send_then(conn, command, move |s, _c, reply| {
            let text = if reply.ok {
                done
            } else {
                format!(
                    "layouts: {}",
                    reply.body.first().map(String::as_str).unwrap_or("failed")
                )
            };
            s.chrome.flash(text);
            s.draw_status_row();
        });
    }

    fn layouts_state(&self) -> Option<&LayoutsState> {
        match &self.modal {
            Modal::Layouts(state) => Some(state),
            _ => None,
        }
    }

    /// Move the list cursor by `delta` (wrapping) in `Choose`/`LoadPick`.
    fn layouts_move(&mut self, delta: isize) {
        let Some(state) = self.layouts_state().cloned() else {
            return;
        };
        let next = match state {
            LayoutsState::Choose { selected } => LayoutsState::Choose {
                selected: (selected as isize + delta).rem_euclid(2) as usize,
            },
            LayoutsState::LoadPick { names, selected } if !names.is_empty() => {
                let at = (selected as isize + delta).rem_euclid(names.len() as isize) as usize;
                LayoutsState::LoadPick {
                    names,
                    selected: at,
                }
            }
            _ => return,
        };
        self.set_layouts_state(next);
    }

    /// Enter on the current state.
    fn layouts_activate(&mut self, conn: &mut AttachConn) {
        let Some(state) = self.layouts_state().cloned() else {
            return;
        };
        match state {
            LayoutsState::Choose { selected: 0 } => {
                self.set_layouts_state(LayoutsState::SavePrompt {
                    text: String::new(),
                });
            }
            LayoutsState::Choose { .. } => {
                if let Some(names) = self.query_layouts(conn) {
                    self.set_layouts_state(LayoutsState::LoadPick { names, selected: 0 });
                }
            }
            LayoutsState::SavePrompt { text } => {
                let name = text.trim().to_string();
                if name.is_empty() {
                    return;
                }
                if !crate::mux::persist::valid_layout_name(&name) {
                    self.chrome
                        .flash("layout names use letters, digits, . _ -".to_string());
                    self.draw_status_row();
                    return;
                }
                let Some(existing) = self.query_layouts(conn) else {
                    return;
                };
                if existing.contains(&name) {
                    self.set_layouts_state(LayoutsState::SaveConfirmOverwrite { name });
                } else {
                    self.layouts_send_save(conn, &name);
                }
            }
            LayoutsState::SaveConfirmOverwrite { name } => self.layouts_send_save(conn, &name),
            LayoutsState::LoadPick { names, selected } => {
                if let Some(name) = names.get(selected) {
                    self.set_layouts_state(LayoutsState::LoadConfirm { name: name.clone() });
                } else {
                    self.leave_layouts();
                }
            }
            LayoutsState::LoadConfirm { name } => {
                // The dialog is the confirmation, so the load is forced.
                self.send_layout_command(
                    conn,
                    &format!("load-layout {name} -f"),
                    format!("layout loaded: {name}"),
                );
                self.leave_layouts();
            }
        }
    }

    fn layouts_send_save(&mut self, conn: &mut AttachConn, name: &str) {
        self.send_layout_command(
            conn,
            &format!("save-layout {name}"),
            format!("layout saved: {name}"),
        );
        self.leave_layouts();
    }

    /// Escape: back out one level; from `Choose` close.
    fn layouts_back(&mut self, conn: &mut AttachConn) {
        let Some(state) = self.layouts_state().cloned() else {
            return;
        };
        match state {
            LayoutsState::Choose { .. } => self.leave_layouts(),
            LayoutsState::SavePrompt { .. } | LayoutsState::LoadPick { .. } => {
                let selected = usize::from(matches!(state, LayoutsState::LoadPick { .. }));
                self.set_layouts_state(LayoutsState::Choose { selected });
            }
            LayoutsState::SaveConfirmOverwrite { name } => {
                self.set_layouts_state(LayoutsState::SavePrompt { text: name });
            }
            LayoutsState::LoadConfirm { name } => match self.query_layouts(conn) {
                Some(names) => {
                    let selected = names.iter().position(|n| *n == name).unwrap_or(0);
                    self.set_layouts_state(LayoutsState::LoadPick { names, selected });
                }
                None => self.set_layouts_state(LayoutsState::Choose { selected: 1 }),
            },
        }
    }

    fn layouts_text_mut(&mut self) -> Option<&mut String> {
        match &mut self.modal {
            Modal::Layouts(LayoutsState::SavePrompt { text }) => Some(text),
            _ => None,
        }
    }

    /// One plain byte while the dialog is up. Returns whether it is
    /// still up.
    pub(super) fn layouts_byte(&mut self, conn: &mut AttachConn, byte: u8) -> bool {
        let typing = self.layouts_text_mut().is_some();
        match byte {
            0x1b => self.layouts_back(conn),
            b'\r' => self.layouts_activate(conn),
            0x7f if typing => {
                if let Some(text) = self.layouts_text_mut() {
                    text.pop();
                }
                self.refresh_layouts();
            }
            0x03 if typing => {
                if let Some(text) = self.layouts_text_mut() {
                    text.clear();
                }
                self.refresh_layouts();
            }
            b if typing && (b.is_ascii_graphic() || b == b' ') => {
                if let Some(text) = self.layouts_text_mut() {
                    text.push(b as char);
                }
                self.refresh_layouts();
            }
            b if typing && b >= 0x80 => {
                if let Some(ch) = self.text_utf8.push_printable(b) {
                    if let Some(text) = self.layouts_text_mut() {
                        text.push(ch);
                    }
                    self.refresh_layouts();
                }
            }
            b'q' if !typing => match self.layouts_state() {
                Some(LayoutsState::Choose { .. }) => self.leave_layouts(),
                _ => self.layouts_back(conn),
            },
            b'j' if !typing => self.layouts_move(1),
            b'k' if !typing => self.layouts_move(-1),
            _ => {}
        }
        matches!(self.modal, Modal::Layouts(_))
    }

    /// One key event while the dialog is up (the CSI-u / functional
    /// spellings; plain bytes land in [`Self::layouts_byte`]).
    pub(super) fn layouts_key(&mut self, conn: &mut AttachConn, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        let typing = self.layouts_text_mut().is_some();
        match (ev.key(), ev.modifiers) {
            (TermKey::Escape, _) => self.layouts_back(conn),
            (TermKey::Enter, 0) => self.layouts_activate(conn),
            (TermKey::Up, 0) => self.layouts_move(-1),
            (TermKey::Down, 0) => self.layouts_move(1),
            (TermKey::Backspace, _) if typing => {
                if let Some(text) = self.layouts_text_mut() {
                    text.pop();
                }
                self.refresh_layouts();
            }
            (TermKey::Char, 0) if typing => {
                if let (Some(ch), Some(text)) =
                    (char::from_u32(ev.codepoint), self.layouts_text_mut())
                {
                    text.push(ch);
                    self.refresh_layouts();
                }
            }
            _ => {}
        }
    }

    /// A press on overlay row `row`: a `Choose`/`LoadPick` list row
    /// selects and activates; the other states consume it.
    pub(super) fn layouts_click(&mut self, conn: &mut AttachConn, row: usize) {
        let Some(state) = self.layouts_state().cloned() else {
            return;
        };
        match state {
            LayoutsState::Choose { .. } if row < 2 => {
                self.set_layouts_state(LayoutsState::Choose { selected: row });
                self.layouts_activate(conn);
            }
            LayoutsState::LoadPick { names, selected } => {
                // The footer is an overlay row too: only the visible
                // list rows select.
                let at = pick_top(selected) + row;
                if row < names.len().min(PICK_ROWS) {
                    self.set_layouts_state(LayoutsState::LoadPick {
                        names,
                        selected: at,
                    });
                    self.layouts_activate(conn);
                }
            }
            _ => {}
        }
    }
}
