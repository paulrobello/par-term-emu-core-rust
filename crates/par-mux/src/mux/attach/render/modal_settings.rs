//! The settings dialog (herdr's settings overlay): tabbed sections of
//! config-file choices — reached from the side panel's command menu.
//! Arrows (or `j`/`k`) move the selection, Tab/←/→ (or `h`/`l`) switch
//! sections, Enter (or space) applies the selection — the key is written
//! into `config.toml` with the user's comments kept, then the reload
//! chord's path rebinds the client and sends `reload-config` — and
//! Escape (or `q`) closes. The integrations section is read-only.

use super::{Modal, WindowSession};
use crate::mux::attach::panels::HelpRow;
use crate::mux::attach::settings_write::{write_client_key, SettingValue};
use par_term_emu_core::keyboard::TermKeyEvent;

/// The dialog's overlay title.
pub(super) const SETTINGS_TITLE: &str = " settings ";

/// The dialog's sections, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSection {
    Sidebar,
    Borders,
    Notify,
    States,
    Integrations,
}

impl SettingsSection {
    pub(super) const ALL: [Self; 5] = [
        Self::Sidebar,
        Self::Borders,
        Self::Notify,
        Self::States,
        Self::Integrations,
    ];

    fn tab(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Borders => "borders",
            Self::Notify => "notify",
            Self::States => "states",
            Self::Integrations => "integrations",
        }
    }

    /// The section's heading and one-line description.
    fn heading(self) -> (&'static str, &'static str) {
        match self {
            Self::Sidebar => ("side panel", "show the side panel when a client attaches"),
            Self::Borders => ("pane borders", "the divider and border line style"),
            Self::Notify => ("notifications", "how agent state changes are announced"),
            Self::States => (
                "notify on",
                "the agent states whose arrival notifies (space toggles)",
            ),
            Self::Integrations => (
                "agent integrations",
                "session hooks that report agent state (par-mux --install-agent-hooks)",
            ),
        }
    }

    /// The section's choices (the integrations section lists its hook
    /// rows instead).
    fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Sidebar => &["on", "off"],
            Self::Borders => &["herdr", "unicode", "double", "heavy", "ascii"],
            Self::Notify => crate::mux::config::NOTIFY_DELIVERIES,
            Self::States => crate::mux::config::NOTIFY_STATES,
            Self::Integrations => &[],
        }
    }
}

/// The open dialog: its section, the selected row, and the integration
/// rows read when it opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsState {
    pub section: SettingsSection,
    pub selected: usize,
    pub integrations: Vec<(String, bool)>,
}

/// The current config values the dialog marks: which choice each
/// section holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsValues {
    pub sidebar_on_launch: bool,
    pub border_lines: String,
    pub notify_delivery: String,
    pub notify_states: Vec<String>,
}

/// The panel's fixed content width — every row pads to it so the box
/// keeps its size across sections.
const PANEL_WIDTH: usize = 62;

/// The body's fixed row count (the integrations list's height).
const BODY_ROWS: usize = 6;

/// The rows of one composed dialog plus each highlighted span
/// `(row, from_col, to_col)` and the row index the body starts at.
pub(super) struct SettingsPanel {
    pub rows: Vec<HelpRow>,
    pub highlights: Vec<(usize, usize, usize)>,
    pub body_start: usize,
}

fn row(text: String, accent: bool, footer: bool) -> HelpRow {
    let pad = PANEL_WIDTH.saturating_sub(text.chars().count());
    HelpRow {
        text: format!("{text}{}", " ".repeat(pad)),
        accent,
        footer,
    }
}

/// Compose the dialog (herdr's layout): the tab row with the current
/// section highlighted, a blank, the section heading (accent) and its
/// description, a blank, the choices — `▸` on the selection, which is
/// highlighted, and `●` on the value in force (`[x]` / `[ ]` in the
/// multi-select states list) — padded to a fixed height, then the
/// footer controls. Pure — the unit-test surface.
pub(super) fn compose_settings(state: &SettingsState, values: &SettingsValues) -> SettingsPanel {
    let mut rows: Vec<HelpRow> = Vec::new();
    let mut highlights = Vec::new();
    // The tab row: each tab ` name `, the current one highlighted.
    let mut tabs = String::new();
    for section in SettingsSection::ALL {
        let from = tabs.chars().count();
        tabs.push_str(&format!(" {} ", section.tab()));
        if section == state.section {
            highlights.push((0, from, tabs.chars().count()));
        }
    }
    rows.push(row(tabs, false, false));
    rows.push(row(String::new(), false, false));
    let (heading, description) = state.section.heading();
    rows.push(row(format!(" {heading}"), true, false));
    rows.push(row(format!(" {description}"), false, false));
    rows.push(row(String::new(), false, false));
    let body_start = rows.len();
    let body: Vec<String> = match state.section {
        SettingsSection::Integrations => state
            .integrations
            .iter()
            .map(|(agent, installed)| {
                let (mark, status) = if *installed {
                    ("\u{2713}", "installed")
                } else {
                    ("-", "not installed")
                };
                format!(" {mark} {agent:<10} {status}")
            })
            .collect(),
        section => section
            .choices()
            .iter()
            .enumerate()
            .map(|(i, choice)| {
                let pointer = if i == state.selected { "\u{25B8}" } else { " " };
                let mark = match section {
                    SettingsSection::States => {
                        if values.notify_states.iter().any(|s| s == choice) {
                            "[x]"
                        } else {
                            "[ ]"
                        }
                    }
                    _ => {
                        let current = match section {
                            SettingsSection::Sidebar => {
                                if values.sidebar_on_launch {
                                    "on"
                                } else {
                                    "off"
                                }
                            }
                            SettingsSection::Borders => values.border_lines.as_str(),
                            _ => values.notify_delivery.as_str(),
                        };
                        if current == *choice {
                            "\u{25CF}"
                        } else {
                            " "
                        }
                    }
                };
                format!(" {pointer} {mark} {choice}")
            })
            .collect(),
    };
    for i in 0..BODY_ROWS {
        let text = body.get(i).cloned().unwrap_or_default();
        if i == state.selected && state.section != SettingsSection::Integrations {
            highlights.push((rows.len(), 0, PANEL_WIDTH));
        }
        rows.push(row(text, false, false));
    }
    let controls = match state.section {
        SettingsSection::Integrations => " \u{2190}\u{2192} section \u{b7} esc close ",
        SettingsSection::States => {
            " \u{2191}\u{2193} select \u{b7} space toggle \u{b7} tab section \u{b7} esc close "
        }
        _ => " \u{2191}\u{2193} select \u{b7} enter apply \u{b7} tab section \u{b7} esc close ",
    };
    rows.push(row(controls.to_string(), false, true));
    SettingsPanel {
        rows,
        highlights,
        body_start,
    }
}

impl WindowSession {
    /// Open the settings dialog on its first section, the selection on
    /// the value in force.
    pub(super) fn enter_settings(&mut self) {
        let integrations = crate::mux::integrations::hook_status()
            .into_iter()
            .map(|hook| (hook.agent, hook.installed))
            .collect();
        let mut state = SettingsState {
            section: SettingsSection::Sidebar,
            selected: 0,
            integrations,
        };
        state.selected = self.settings_current_index(state.section);
        self.open_modal(Modal::Settings(state));
        self.refresh_settings();
    }

    /// The values the dialog marks as in force.
    fn settings_values(&self) -> SettingsValues {
        SettingsValues {
            sidebar_on_launch: self.sidebar_on_launch,
            border_lines: self.render_opts.glyphs.name().to_string(),
            notify_delivery: self.notify.delivery.name().to_string(),
            notify_states: self.notify.states.clone(),
        }
    }

    /// The row of `section`'s value in force (0 when none matches).
    fn settings_current_index(&self, section: SettingsSection) -> usize {
        let values = self.settings_values();
        let current = match section {
            SettingsSection::Sidebar => {
                if values.sidebar_on_launch {
                    "on"
                } else {
                    "off"
                }
            }
            SettingsSection::Borders => values.border_lines.as_str(),
            SettingsSection::Notify => values.notify_delivery.as_str(),
            SettingsSection::States | SettingsSection::Integrations => return 0,
        };
        section
            .choices()
            .iter()
            .position(|choice| *choice == current)
            .unwrap_or(0)
    }

    /// Re-compose the overlay from the dialog's state.
    pub(super) fn refresh_settings(&mut self) {
        let Modal::Settings(state) = &self.modal else {
            return;
        };
        let panel = compose_settings(state, &self.settings_values());
        self.renderer
            .set_overlay(Some((SETTINGS_TITLE, panel.rows, None)));
        self.renderer.set_overlay_highlights(panel.highlights);
    }

    /// Close the dialog.
    pub(super) fn leave_settings(&mut self) {
        if matches!(self.modal, Modal::Settings(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    /// Switch section by `delta` (wrapping), the selection landing on the
    /// new section's value in force.
    fn settings_move_section(&mut self, delta: isize) {
        let Modal::Settings(state) = &self.modal else {
            return;
        };
        let at = SettingsSection::ALL
            .iter()
            .position(|s| *s == state.section)
            .unwrap_or(0) as isize;
        let next = SettingsSection::ALL
            [(at + delta).rem_euclid(SettingsSection::ALL.len() as isize) as usize];
        let selected = self.settings_current_index(next);
        if let Modal::Settings(state) = &mut self.modal {
            state.section = next;
            state.selected = selected;
        }
        self.refresh_settings();
    }

    /// Move the selection by `delta`, clamped to the section's choices.
    fn settings_move_selection(&mut self, delta: isize) {
        if let Modal::Settings(state) = &mut self.modal {
            let count = state.section.choices().len();
            if count > 0 {
                state.selected =
                    (state.selected as isize + delta).clamp(0, count as isize - 1) as usize;
            }
        }
        self.refresh_settings();
    }

    /// Apply the selection: write its key into the config file, then run
    /// the reload chord's path (the client rebinds from the file it just
    /// wrote, the daemon re-reads too). The integrations section applies
    /// nothing.
    pub(super) fn settings_apply(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Settings(state) = &self.modal else {
            return;
        };
        let section = state.section;
        let Some(choice) = section.choices().get(state.selected).copied() else {
            return;
        };
        let (key, value) = match section {
            SettingsSection::Sidebar => ("sidebar-on-launch", SettingValue::Bool(choice == "on")),
            SettingsSection::Borders => ("border-lines", SettingValue::Str(choice.to_string())),
            SettingsSection::Notify => ("notify-delivery", SettingValue::Str(choice.to_string())),
            SettingsSection::States => {
                let mut states = self.notify.states.clone();
                if let Some(at) = states.iter().position(|s| s == choice) {
                    states.remove(at);
                } else {
                    states.push(choice.to_string());
                }
                // Keep the config's list in the canonical state order.
                states.sort_by_key(|s| {
                    crate::mux::config::NOTIFY_STATES
                        .iter()
                        .position(|known| known == s)
                });
                ("notify-states", SettingValue::List(states))
            }
            SettingsSection::Integrations => return,
        };
        let Some(path) = self.settings_path.clone() else {
            self.chrome
                .flash("settings: no config file location known".to_string());
            return;
        };
        if let Err(err) = write_client_key(&path, key, &value) {
            self.chrome.flash(format!("settings not saved: {err}"));
            self.draw_status_row();
            return;
        }
        if let (SettingsSection::Sidebar, SettingValue::Bool(on)) = (section, &value) {
            self.sidebar_on_launch = *on;
        }
        self.reload_config(conn);
        self.refresh_settings();
    }

    /// One key while the dialog is up: the arrows select and switch
    /// sections, Escape closes; everything else is consumed.
    pub(super) fn settings_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Escape => self.leave_settings(),
            TermKey::Up => self.settings_move_selection(-1),
            TermKey::Down => self.settings_move_selection(1),
            TermKey::Left => self.settings_move_section(-1),
            TermKey::Right => self.settings_move_section(1),
            _ => {}
        }
    }

    /// One plain byte while the dialog is up: `j`/`k` select, Tab /
    /// `l` / `h` switch sections, Enter / space apply, Escape / `q`
    /// close. Returns whether the dialog is still up.
    pub(super) fn settings_byte(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        byte: u8,
    ) -> bool {
        match byte {
            b'q' | 0x1b => self.leave_settings(),
            b'k' => self.settings_move_selection(-1),
            b'j' => self.settings_move_selection(1),
            b'\t' | b'l' => self.settings_move_section(1),
            b'h' => self.settings_move_section(-1),
            b'\r' | b'\n' | b' ' => self.settings_apply(conn),
            _ => {}
        }
        matches!(self.modal, Modal::Settings(_))
    }

    /// A press at overlay row `row`, column `col` (inner-relative): a
    /// tab switches to its section, a choice selects and applies.
    pub(super) fn settings_click(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        row: usize,
        col: usize,
    ) {
        let Modal::Settings(state) = &self.modal else {
            return;
        };
        if row == 0 {
            let mut from = 0;
            for (i, section) in SettingsSection::ALL.iter().enumerate() {
                let to = from + section.tab().chars().count() + 2;
                if (from..to).contains(&col) {
                    let at = SettingsSection::ALL
                        .iter()
                        .position(|s| *s == state.section)
                        .unwrap_or(0);
                    self.settings_move_section(i as isize - at as isize);
                    return;
                }
                from = to;
            }
            return;
        }
        let panel = compose_settings(state, &self.settings_values());
        let Some(index) = row.checked_sub(panel.body_start) else {
            return;
        };
        if index < state.section.choices().len() {
            if let Modal::Settings(state) = &mut self.modal {
                state.selected = index;
            }
            self.settings_apply(conn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> SettingsValues {
        SettingsValues {
            sidebar_on_launch: true,
            border_lines: "herdr".to_string(),
            notify_delivery: "terminal".to_string(),
            notify_states: vec!["blocked".to_string(), "idle".to_string()],
        }
    }

    /// herdr's dialog shape: the tab row with the section highlighted,
    /// heading and description, the choices with the selection and the
    /// value in force marked, a fixed body height, and the footer.
    #[test]
    fn compose_settings_marks_the_tab_selection_and_value() {
        let state = SettingsState {
            section: SettingsSection::Notify,
            selected: 2,
            integrations: Vec::new(),
        };
        let panel = compose_settings(&state, &values());
        assert!(panel.rows[0]
            .text
            .starts_with(" sidebar  borders  notify  states  integrations "));
        assert_eq!(panel.highlights[0], (0, 18, 26), "the notify tab");
        assert_eq!(panel.rows[2].text.trim_end(), " notifications");
        assert!(panel.rows[2].accent);
        assert_eq!(panel.body_start, 5);
        assert_eq!(panel.rows[5].text.trim_end(), "     off");
        assert_eq!(panel.rows[6].text.trim_end(), "   \u{25CF} terminal");
        assert_eq!(panel.rows[7].text.trim_end(), " \u{25B8}   system");
        assert_eq!(panel.highlights[1], (7, 0, PANEL_WIDTH), "the selection");
        assert!(panel.rows.last().unwrap().footer);
        assert!(
            panel
                .rows
                .iter()
                .all(|r| r.text.chars().count() == PANEL_WIDTH),
            "every row pads to the fixed width"
        );
        let states = compose_settings(
            &SettingsState {
                section: SettingsSection::States,
                selected: 0,
                integrations: Vec::new(),
            },
            &values(),
        );
        assert_eq!(states.rows[5].text.trim_end(), " \u{25B8} [ ] working");
        assert_eq!(states.rows[6].text.trim_end(), "   [x] blocked");
        assert_eq!(
            states.rows.len(),
            panel.rows.len(),
            "the box keeps its height across sections"
        );
        let hooks = compose_settings(
            &SettingsState {
                section: SettingsSection::Integrations,
                selected: 0,
                integrations: vec![("claude".to_string(), true), ("grok".to_string(), false)],
            },
            &values(),
        );
        assert_eq!(
            hooks.rows[5].text.trim_end(),
            " \u{2713} claude     installed"
        );
        assert_eq!(hooks.rows[6].text.trim_end(), " - grok       not installed");
        assert_eq!(hooks.highlights.len(), 1, "read-only: no selection band");
    }
}
