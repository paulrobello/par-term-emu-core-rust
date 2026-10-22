//! The settings dialog (herdr's settings overlay): tabbed sections of
//! `[client]` config keys — reached from the side panel's command menu.
//! Every TUI-facing `[client]` key has a row here (the CLAUDE.md rule):
//! toggles flip on Enter, small choice lists cycle on Enter, and
//! numbers, hex colors, and chords edit through the prompt modal seeded
//! with the value in force and validated before anything is written.
//! Arrows (or `j`/`k`) move the selection, Tab/←/→ (or `h`/`l`) switch
//! sections, Escape (or `q`) closes. The integrations section lists the
//! installs and its Enter (or `u`) reinstalls the stale ones; the states
//! section is a multi-select (space toggles).

use super::{Modal, WindowSession};
use crate::mux::attach::panels::{spell_key, HelpRow};
use crate::mux::attach::settings_write::{write_client_key, SettingValue};
use crate::mux::config::Management;
use par_term_emu_core::keyboard::TermKeyEvent;

/// The dialog's overlay title.
pub(super) const SETTINGS_TITLE: &str = " settings ";

/// The dialog's sections, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSection {
    Sidebar,
    Borders,
    General,
    Notify,
    States,
    Keys,
    Integrations,
}

/// The named `agent-rows` layouts the dialog cycles through: each is the
/// token spelling written to the config (herdr's default, a one-line
/// compact form, and everything the sidebar can show).
const AGENT_ROWS_PRESETS: &[(&str, &[&[&str]])] = &[
    (
        "default",
        &[&["state_icon", "workspace", "project"], &["agent"]],
    ),
    ("compact", &[&["state_icon", "agent"]]),
    (
        "detailed",
        &[
            &["state_icon", "state", "workspace", "project"],
            &["agent", "pane"],
        ],
    ),
    (
        "verbose",
        &[&["state_icon", "state", "agent"], &["message"]],
    ),
];

impl SettingsSection {
    pub(super) const ALL: [Self; 7] = [
        Self::Sidebar,
        Self::Borders,
        Self::General,
        Self::Notify,
        Self::States,
        Self::Keys,
        Self::Integrations,
    ];

    fn tab(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Borders => "borders",
            Self::General => "general",
            Self::Notify => "notify",
            Self::States => "states",
            Self::Keys => "keys",
            Self::Integrations => "integrations",
        }
    }

    /// The section's heading and one-line description.
    fn heading(self) -> (&'static str, &'static str) {
        match self {
            Self::Sidebar => ("side panel", "what the workspace side panel shows"),
            Self::Borders => ("borders", "the divider and border look, and the theme"),
            Self::General => ("general", "client behavior and the attach mode"),
            Self::Notify => ("notifications", "how agent state changes are announced"),
            Self::States => (
                "notify on",
                "the agent states whose arrival notifies (space toggles)",
            ),
            Self::Keys => ("keys", "the prefix and every management chord"),
            Self::Integrations => (
                "agent integrations",
                "agent hooks (par-mux --install-agent-hooks)",
            ),
        }
    }

    /// The multi-select states section's entries.
    fn choices(self) -> &'static [&'static str] {
        match self {
            Self::States => crate::mux::config::NOTIFY_STATES,
            _ => &[],
        }
    }

    /// The section's editable rows (the states and integrations sections
    /// keep their own shapes).
    fn rows(self) -> &'static [SettingSpec] {
        match self {
            Self::Sidebar => &[
                SettingSpec {
                    key: "sidebar-on-launch",
                    label: "show on attach",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "sidebar-width",
                    label: "width",
                    kind: SettingKind::Number { min: 6, max: 60 },
                },
                SettingSpec {
                    key: "agent-rows",
                    label: "agent lines",
                    kind: SettingKind::Choice(&["default", "compact", "detailed", "verbose"]),
                },
            ],
            Self::Borders => &[
                SettingSpec {
                    key: "border-lines",
                    label: "line style",
                    kind: SettingKind::Choice(&["herdr", "unicode", "double", "heavy", "ascii"]),
                },
                SettingSpec {
                    key: "pane-borders",
                    label: "pane boxes",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "show-label-in-border",
                    label: "labels in border",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "pane-gaps",
                    label: "pane gaps",
                    kind: SettingKind::Number { min: 0, max: 8 },
                },
                SettingSpec {
                    key: "border-active-color",
                    label: "focus color",
                    kind: SettingKind::Hex,
                },
                SettingSpec {
                    key: "border-color",
                    label: "border color",
                    kind: SettingKind::Hex,
                },
                SettingSpec {
                    key: "theme",
                    label: "theme",
                    kind: SettingKind::Choice(crate::mux::theme::THEME_NAMES),
                },
                SettingSpec {
                    key: "theme-dark",
                    label: "auto: dark host",
                    kind: SettingKind::Choice(crate::mux::theme::PRESET_NAMES),
                },
                SettingSpec {
                    key: "theme-light",
                    label: "auto: light host",
                    kind: SettingKind::Choice(crate::mux::theme::PRESET_NAMES),
                },
            ],
            Self::General => &[
                SettingSpec {
                    key: "mode",
                    label: "attach mode",
                    kind: SettingKind::Choice(&["render", "passthrough"]),
                },
                SettingSpec {
                    key: "scrollbar-gutter",
                    label: "scroll gutter",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "drag-cursor-shape",
                    label: "drag cursor shape",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "resize-step",
                    label: "resize step",
                    kind: SettingKind::Number { min: 1, max: 99 },
                },
                SettingSpec {
                    key: "kitty-graphics",
                    label: "kitty graphics",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "session-picker",
                    label: "session picker",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "git-status",
                    label: "git status badge",
                    kind: SettingKind::Toggle,
                },
            ],
            Self::Notify => &[
                SettingSpec {
                    key: "notify-delivery",
                    label: "delivery",
                    kind: SettingKind::Choice(crate::mux::config::NOTIFY_DELIVERIES),
                },
                SettingSpec {
                    key: "sound",
                    label: "sounds",
                    kind: SettingKind::Toggle,
                },
                SettingSpec {
                    key: "sound-done",
                    label: "done sound",
                    kind: SettingKind::Text,
                },
                SettingSpec {
                    key: "sound-blocked",
                    label: "blocked sound",
                    kind: SettingKind::Text,
                },
                SettingSpec {
                    key: "sound-mute",
                    label: "muted agents",
                    kind: SettingKind::List,
                },
                SettingSpec {
                    key: "sound-player",
                    label: "sound player",
                    kind: SettingKind::Text,
                },
            ],
            Self::Keys => &[
                SettingSpec {
                    key: "prefix",
                    label: "prefix",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "reload",
                    label: "reload config",
                    kind: SettingKind::ReloadChord,
                },
                SettingSpec {
                    key: "split-right",
                    label: "split right",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "split-down",
                    label: "split down",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "kill-pane",
                    label: "kill pane",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "break-pane",
                    label: "break pane out",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "join-pane",
                    label: "join pane into tab",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "palette",
                    label: "command palette",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "new-window",
                    label: "new window",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "resize",
                    label: "resize mode",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "swap-prev",
                    label: "swap prev pane",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "swap-next",
                    label: "swap next pane",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "workspace-next",
                    label: "next workspace",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "workspace-prev",
                    label: "prev workspace",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "help",
                    label: "keybinds",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "picker",
                    label: "window picker",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "zoom",
                    label: "zoom pane",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "rename-window",
                    label: "rename window",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "rename-pane",
                    label: "rename pane",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "border-cycle",
                    label: "cycle borders",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "label-toggle",
                    label: "toggle labels",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "workspace-picker",
                    label: "workspace picker",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "sidebar",
                    label: "toggle sidebar",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "status-bar",
                    label: "toggle status bar",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "notify-jump",
                    label: "jump to notification",
                    kind: SettingKind::Chord,
                },
                SettingSpec {
                    key: "respawn",
                    label: "respawn dead pane",
                    kind: SettingKind::Chord,
                },
            ],
            Self::States | Self::Integrations => &[],
        }
    }

    /// The section's selectable row count (its rows, or the states
    /// multi-select's entries).
    pub(super) fn row_count(self) -> usize {
        match self {
            Self::States => self.choices().len(),
            Self::Integrations => 0,
            _ => self.rows().len(),
        }
    }
}

/// What one row edits and how Enter applies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingKind {
    /// on/off — Enter flips it.
    Toggle,
    /// A fixed choice list — Enter cycles to the next (wrapping).
    Choice(&'static [&'static str]),
    /// A whole number in range — Enter edits it in the prompt.
    Number { min: u32, max: u32 },
    /// A `#rrggbb` hex color (an empty prompt clears to the built-in).
    Hex,
    /// A chord in the tmux spelling (one after-prefix key).
    Chord,
    /// The reload chord (prefix + key spelling; the last token binds).
    ReloadChord,
    /// Free text, e.g. a file path (an empty prompt clears to the
    /// built-in).
    Text,
    /// A comma-separated list (an empty prompt clears it).
    List,
    /// One `[[client.popup]]` entry by index (`usize::MAX` = a new one),
    /// spelled `KEY [WxH] COMMAND`; an empty prompt deletes it.
    Popup(usize),
}

/// One editable `[client]` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SettingSpec {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) kind: SettingKind,
}

/// A prompt-edited row handed to the prompt modal: what to validate and
/// write on commit, plus the dialog state to restore when the prompt
/// closes (commit or cancel both return to the dialog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingEdit {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) kind: SettingKind,
    pub(super) back: SettingsState,
}

/// The open dialog: its section, the selected row, the body's first
/// visible row (the keys section scrolls), the integration rows read
/// when it opened, and the file's `mode` (no live mirror exists — it is
/// chosen at launch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsState {
    pub section: SettingsSection,
    pub selected: usize,
    pub top: usize,
    pub integrations: Vec<(String, String)>,
    pub file_mode: String,
    /// The configured popups: the general section's trailing rows.
    pub popups: Vec<crate::mux::popup::PopupEntry>,
}

/// The current config values the dialog shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsValues {
    pub sidebar_on_launch: bool,
    pub sidebar_width: u16,
    pub agent_rows: Vec<Vec<String>>,
    pub border_lines: String,
    pub pane_borders: bool,
    pub show_label_in_border: bool,
    pub pane_gaps: u16,
    pub border_active_color: String,
    pub border_color: String,
    pub theme: crate::mux::theme::ThemeSelection,
    pub scrollbar_gutter: bool,
    pub drag_cursor_shape: bool,
    pub kitty_graphics: bool,
    pub session_picker: bool,
    pub git_status: bool,
    pub resize_step: u32,
    pub notify_delivery: String,
    pub notify_states: Vec<String>,
    pub sound: crate::mux::config::SoundSettings,
    pub prefix: u8,
    pub reload: u8,
    pub management: Management,
}

/// A theme key's canonical spelling (an alias like `tokyonight` must
/// match its choice-list entry, or Enter would cycle from the first
/// entry); an unknown name shows as spelled.
fn theme_spelling(name: &str) -> String {
    crate::mux::theme::canonical(name).map_or_else(|| name.to_string(), str::to_string)
}

/// One row's current value: what the row displays and what Enter acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldVal {
    Bool(bool),
    Num(u32),
    Text(String),
    /// A configured value plus the hint the row displays beside it — the
    /// theme's `auto` naming the preset it resolved to. Cycling and the
    /// edit seed read the raw value; only the paint adds the hint.
    TextWithHint(String, String),
}

impl SettingsValues {
    /// The value in force for `key` (`file_mode` covers `mode`, which
    /// has no live mirror).
    fn field(&self, key: &str, file_mode: &str) -> FieldVal {
        let m = &self.management;
        match key {
            "sidebar-on-launch" => FieldVal::Bool(self.sidebar_on_launch),
            "sidebar-width" => FieldVal::Num(self.sidebar_width as u32),
            "agent-rows" => FieldVal::Text(self.agent_rows_preset()),
            "border-lines" => FieldVal::Text(self.border_lines.clone()),
            "pane-borders" => FieldVal::Bool(self.pane_borders),
            "show-label-in-border" => FieldVal::Bool(self.show_label_in_border),
            "pane-gaps" => FieldVal::Num(self.pane_gaps as u32),
            "border-active-color" => FieldVal::Text(self.border_active_color.clone()),
            "border-color" => FieldVal::Text(self.border_color.clone()),
            "theme" => {
                let configured = theme_spelling(&self.theme.theme);
                if configured == "auto" {
                    FieldVal::TextWithHint(configured, format!("({})", self.theme.effective()))
                } else {
                    FieldVal::Text(configured)
                }
            }
            "theme-dark" => FieldVal::Text(theme_spelling(&self.theme.dark)),
            "theme-light" => FieldVal::Text(theme_spelling(&self.theme.light)),
            "mode" => FieldVal::Text(file_mode.to_string()),
            "scrollbar-gutter" => FieldVal::Bool(self.scrollbar_gutter),
            "drag-cursor-shape" => FieldVal::Bool(self.drag_cursor_shape),
            "kitty-graphics" => FieldVal::Bool(self.kitty_graphics),
            "session-picker" => FieldVal::Bool(self.session_picker),
            "git-status" => FieldVal::Bool(self.git_status),
            "resize-step" => FieldVal::Num(self.resize_step),
            "notify-delivery" => FieldVal::Text(self.notify_delivery.clone()),
            "sound" => FieldVal::Bool(self.sound.enabled),
            "sound-done" => FieldVal::Text(self.sound.done.clone()),
            "sound-blocked" => FieldVal::Text(self.sound.blocked.clone()),
            "sound-mute" => FieldVal::Text(self.sound.mute.join(", ")),
            "sound-player" => FieldVal::Text(self.sound.player.clone()),
            "prefix" => FieldVal::Text(spell_key(self.prefix)),
            "reload" => FieldVal::Text(format!(
                "{} {}",
                spell_key(self.prefix),
                spell_key(self.reload)
            )),
            "split-right" => FieldVal::Text(spell_key(m.split_right)),
            "split-down" => FieldVal::Text(spell_key(m.split_down)),
            "kill-pane" => FieldVal::Text(spell_key(m.kill_pane)),
            "break-pane" => FieldVal::Text(spell_key(m.break_pane)),
            "join-pane" => FieldVal::Text(spell_key(m.join_pane)),
            "palette" => FieldVal::Text(spell_key(m.palette)),
            "new-window" => FieldVal::Text(spell_key(m.new_window)),
            "resize" => FieldVal::Text(spell_key(m.resize)),
            "swap-prev" => FieldVal::Text(spell_key(m.swap_prev)),
            "swap-next" => FieldVal::Text(spell_key(m.swap_next)),
            "workspace-next" => FieldVal::Text(spell_key(m.workspace_next)),
            "workspace-prev" => FieldVal::Text(spell_key(m.workspace_prev)),
            "help" => FieldVal::Text(spell_key(m.help)),
            "picker" => FieldVal::Text(spell_key(m.picker)),
            "zoom" => FieldVal::Text(spell_key(m.zoom)),
            "rename-window" => FieldVal::Text(spell_key(m.rename_window)),
            "rename-pane" => FieldVal::Text(spell_key(m.rename_pane)),
            "border-cycle" => FieldVal::Text(spell_key(m.border_cycle)),
            "label-toggle" => FieldVal::Text(spell_key(m.label_toggle)),
            "workspace-picker" => FieldVal::Text(spell_key(m.workspace_picker)),
            "sidebar" => FieldVal::Text(spell_key(m.sidebar)),
            "status-bar" => FieldVal::Text(spell_key(m.status_bar)),
            "respawn" => FieldVal::Text(spell_key(m.respawn)),
            "notify-jump" => FieldVal::Text(spell_key(m.notify_jump)),
            _ => FieldVal::Text(String::new()),
        }
    }

    /// The preset name the live `agent-rows` matches (`custom` when it
    /// matches none — a hand-edited layout the dialog leaves alone
    /// until a preset is chosen).
    fn agent_rows_preset(&self) -> String {
        for (name, lines) in AGENT_ROWS_PRESETS {
            let rendered: Vec<Vec<String>> = lines
                .iter()
                .map(|line| line.iter().map(|token| token.to_string()).collect())
                .collect();
            if rendered == self.agent_rows {
                return name.to_string();
            }
        }
        "custom".to_string()
    }

    /// The preset's token lines, for writing.
    fn preset_rows(name: &str) -> Vec<Vec<String>> {
        for (preset, lines) in AGENT_ROWS_PRESETS {
            if *preset == name {
                return lines
                    .iter()
                    .map(|line| line.iter().map(|token| token.to_string()).collect())
                    .collect();
            }
        }
        Vec::new()
    }
}

/// The panel's fixed content width — every row pads to it so the box
/// keeps its size across sections.
const PANEL_WIDTH: usize = 64;

/// The body's fixed row count (the keys section scrolls inside it).
const BODY_ROWS: usize = 8;

/// The label column's width.
const LABEL_WIDTH: usize = 20;

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
        render_only: false,
    }
}

/// One row's display text: `on`/`off` for toggles, digits for numbers,
/// the text itself otherwise (an empty value reads `built-in`, `none`
/// for the mute list, `os default` for the player). Text wider than the
/// value column keeps its tail (a path's file name) behind `…`.
fn field_display(key: &str, field: &FieldVal) -> String {
    let text = match field {
        FieldVal::Bool(on) => if *on { "on" } else { "off" }.to_string(),
        FieldVal::Num(n) => n.to_string(),
        FieldVal::TextWithHint(text, hint) => format!("{text} {hint}"),
        FieldVal::Text(text) if text.is_empty() => match key {
            "sound-mute" => "none",
            "sound-player" => "os default",
            _ => "built-in",
        }
        .to_string(),
        FieldVal::Text(text) => text.clone(),
    };
    let width = PANEL_WIDTH - 4 - LABEL_WIDTH;
    let len = text.chars().count();
    if len <= width {
        return text;
    }
    let tail: String = text.chars().skip(len - (width - 1)).collect();
    format!("\u{2026}{tail}")
}

/// Compose the dialog (herdr's layout): the tab row with the current
/// section highlighted, a blank, the section heading (accent) and its
/// description, a blank, the body — one row per editable key with the
/// value in force right-aligned (`▸` on the selection), the states
/// multi-select (`[x]`/`[ ]`), or the integrations list — scrolled by
/// the dialog's `top`, padded to a fixed height, then the footer
/// controls. Pure — the unit-test surface.
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
            .map(|(agent, status)| {
                let mark = if status == "not installed" {
                    "-"
                } else {
                    "\u{2713}"
                };
                format!(" {mark} {agent:<10} {status}")
            })
            .collect(),
        SettingsSection::States => state
            .section
            .choices()
            .iter()
            .enumerate()
            .map(|(at, choice)| {
                let pointer = if at == state.selected {
                    "\u{25B8}"
                } else {
                    " "
                };
                let mark = if values.notify_states.iter().any(|s| s == choice) {
                    "[x]"
                } else {
                    "[ ]"
                };
                format!(" {pointer} {mark} {choice}")
            })
            .collect(),
        section => {
            let mut body: Vec<String> = section
                .rows()
                .iter()
                .enumerate()
                .map(|(at, spec)| {
                    let pointer = if at == state.selected {
                        "\u{25B8}"
                    } else {
                        " "
                    };
                    let value = field_display(spec.key, &values.field(spec.key, &state.file_mode));
                    format!(
                        " {} {:<width$} {:>vwidth$}",
                        pointer,
                        spec.label,
                        value,
                        width = LABEL_WIDTH,
                        vwidth = PANEL_WIDTH - 4 - LABEL_WIDTH
                    )
                })
                .collect();
            if section == SettingsSection::General {
                body.extend(state.popup_row_texts(section.rows().len(), LABEL_WIDTH, PANEL_WIDTH));
            }
            body
        }
    };
    // The selection's highlight rides the composed pointer row inside
    // the body's scrolled window.
    let selected_in_body = state
        .selected
        .saturating_sub(state.top)
        .min(BODY_ROWS.saturating_sub(1));
    for i in 0..BODY_ROWS {
        let at = state.top + i;
        let text = body.get(at).cloned().unwrap_or_default();
        if state.section != SettingsSection::Integrations && i == selected_in_body {
            highlights.push((rows.len(), 0, PANEL_WIDTH));
        }
        rows.push(row(text, false, false));
    }
    let controls = match state.section {
        SettingsSection::Integrations => {
            " \u{2190}\u{2192} section \u{b7} enter/u update outdated \u{b7} esc close "
        }
        SettingsSection::States => {
            " \u{2191}\u{2193} select \u{b7} space toggle \u{b7} tab section \u{b7} esc close "
        }
        _ => " \u{2191}\u{2193} select \u{b7} enter edit \u{b7} tab section \u{b7} esc close ",
    };
    rows.push(row(controls.to_string(), false, true));
    SettingsPanel {
        rows,
        highlights,
        body_start,
    }
}

/// The integrations rows as the dialog shows them: every hook, then the
/// extensions, then the bundled skill — name plus the version-aware
/// currency string.
fn integrations_rows() -> Vec<(String, String)> {
    let mut integrations: Vec<(String, String)> = crate::mux::integrations::hook_status()
        .into_iter()
        .map(|hook| (hook.agent, hook.asset.display(hook.installed)))
        .collect();
    integrations.extend(
        crate::mux::integrations::extension_status()
            .into_iter()
            .map(|ext| (ext.agent, ext.asset.display(ext.installed))),
    );
    if let Ok(skill) = crate::mux::skill_installer::skill_status() {
        integrations.push(("skill".to_string(), skill.asset.display(skill.installed)));
    }
    integrations
}

impl WindowSession {
    /// Open the settings dialog on its first section.
    pub(super) fn enter_settings(&mut self) {
        // Every bundled integration, each row's value the version-aware
        // currency (`v2`, `v1 → v2 outdated`, `asset missing`, `not
        // installed`) so a stale install is visible without leaving the
        // dialog.
        let integrations = integrations_rows();
        // `mode` is chosen at launch: the file is the only record of it.
        let file_mode = match &self.settings_path {
            Some(path) => crate::mux::config::load_file(path)
                .ok()
                .flatten()
                .and_then(|file| file.client.mode)
                .unwrap_or_else(|| "render".to_string()),
            None => "render".to_string(),
        };
        let state = SettingsState {
            section: SettingsSection::Sidebar,
            selected: 0,
            top: 0,
            integrations,
            file_mode,
            popups: self.popup_entries(),
        };
        self.open_modal(Modal::Settings(state));
        self.refresh_settings();
    }

    /// The values the dialog shows as in force.
    fn settings_values(&self) -> SettingsValues {
        let hex = |color: Option<ratatui::style::Color>| match color {
            Some(ratatui::style::Color::Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => String::new(),
        };
        SettingsValues {
            sidebar_on_launch: self.sidebar_on_launch,
            sidebar_width: self.render_opts.sidebar_width,
            agent_rows: self.agent_rows.clone(),
            border_lines: self.render_opts.glyphs.name().to_string(),
            pane_borders: self.render_opts.pane_borders,
            show_label_in_border: self.render_opts.show_label_in_border,
            pane_gaps: self.render_opts.pane_gaps,
            border_active_color: hex(self.render_opts.border_active),
            border_color: hex(self.render_opts.border_plain),
            theme: self.theme.clone(),
            scrollbar_gutter: self.render_opts.scrollbar_gutter,
            drag_cursor_shape: self.cursor.drag_shape,
            kitty_graphics: self.kitty.enabled,
            session_picker: self.session_picker,
            git_status: self.git_status,
            resize_step: self.resize_step,
            notify_delivery: self.notify.delivery.name().to_string(),
            notify_states: self.notify.states.clone(),
            sound: self.notify.sound.clone(),
            prefix: self.prefix,
            reload: self.reload_key,
            management: self.management,
        }
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

    /// The integrations tab's button: reinstall every integration graded
    /// stale (`v1 → v2 outdated`, `asset missing` under a surviving
    /// registration) — never-installed rows are left alone. The rows
    /// re-grade and the outcome flashes on the status row.
    fn settings_update_integrations(&mut self) {
        let (updated, failed) = crate::mux::integrations::update_outdated_integrations();
        if let Modal::Settings(state) = &mut self.modal {
            state.integrations = integrations_rows();
        }
        if !failed.is_empty() {
            self.chrome.flash(format!(
                "integrations: update failed — {}",
                failed.join(", ")
            ));
        } else if updated.is_empty() {
            self.chrome
                .flash("integrations: everything current".to_string());
        } else {
            self.chrome
                .flash(format!("integrations: updated {}", updated.join(", ")));
        }
        self.draw_status_row();
        self.refresh_settings();
    }

    /// Switch section by `delta` (wrapping).
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
        if let Modal::Settings(state) = &mut self.modal {
            state.section = next;
            state.selected = 0;
            state.top = 0;
        }
        self.refresh_settings();
    }

    /// Move the selection by `delta`, clamped to the section's rows, the
    /// body's window following the selection.
    fn settings_move_selection(&mut self, delta: isize) {
        if let Modal::Settings(state) = &mut self.modal {
            let count = state.section.row_count() + state.popup_rows();
            if count > 0 {
                state.selected =
                    (state.selected as isize + delta).clamp(0, count as isize - 1) as usize;
                if state.selected < state.top {
                    state.top = state.selected;
                } else if state.selected >= state.top + BODY_ROWS {
                    state.top = state.selected + 1 - BODY_ROWS;
                }
            }
        }
        self.refresh_settings();
    }

    /// Whether the selected row's value changes in place — a toggle or a
    /// choice — so ←/→ cycle it instead of switching sections.
    fn settings_selected_is_discrete(&self) -> bool {
        let Modal::Settings(state) = &self.modal else {
            return false;
        };
        state
            .section
            .rows()
            .get(state.selected)
            .is_some_and(|spec| matches!(spec.kind, SettingKind::Toggle | SettingKind::Choice(_)))
    }

    /// Apply the selection as a value change: a toggle flips, a choice
    /// cycles by `delta` (wrapping), and a number/hex/chord opens the
    /// prompt seeded with the value in force (the prompt's commit
    /// validates and writes). The states section toggles its entry; the
    /// integrations section runs the update button (reinstall the stale).
    pub(super) fn settings_apply(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        delta: isize,
    ) {
        let Modal::Settings(state) = &self.modal else {
            return;
        };
        let section = state.section;
        let selected = state.selected;
        let file_mode = state.file_mode.clone();
        let back = state.clone();
        match section {
            SettingsSection::States => {
                let Some(choice) = section.choices().get(selected).copied() else {
                    return;
                };
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
                self.settings_write_and_reload(conn, "notify-states", SettingValue::List(states));
            }
            SettingsSection::Integrations => self.settings_update_integrations(),
            section => {
                if section == SettingsSection::General && selected >= section.rows().len() {
                    self.edit_popup_row(selected - section.rows().len(), back);
                    return;
                }
                let Some(spec) = section.rows().get(selected) else {
                    return;
                };
                let values = self.settings_values();
                match spec.kind {
                    SettingKind::Toggle => {
                        let FieldVal::Bool(on) = values.field(spec.key, &file_mode) else {
                            return;
                        };
                        self.settings_write_and_reload(conn, spec.key, SettingValue::Bool(!on));
                    }
                    SettingKind::Choice(choices) => {
                        let current = match values.field(spec.key, &file_mode) {
                            FieldVal::Text(text) => text,
                            FieldVal::TextWithHint(text, _) => text,
                            _ => return,
                        };
                        let len = choices.len() as isize;
                        let next = match choices.iter().position(|choice| *choice == current) {
                            Some(at) => {
                                choices[(at as isize + delta).rem_euclid(len) as usize].to_string()
                            }
                            None => choices[delta.rem_euclid(len) as usize].to_string(),
                        };
                        let value = if spec.key == "agent-rows" {
                            SettingValue::Rows(SettingsValues::preset_rows(&next))
                        } else {
                            SettingValue::Str(next)
                        };
                        self.settings_write_and_reload(conn, spec.key, value);
                    }
                    kind @ (SettingKind::Number { .. }
                    | SettingKind::Hex
                    | SettingKind::Chord
                    | SettingKind::ReloadChord
                    | SettingKind::Text
                    | SettingKind::List
                    | SettingKind::Popup(_)) => {
                        let seed = match values.field(spec.key, &file_mode) {
                            FieldVal::Text(text) => text,
                            FieldVal::TextWithHint(text, _) => text,
                            FieldVal::Num(n) => n.to_string(),
                            FieldVal::Bool(on) => on.to_string(),
                        };
                        self.enter_setting_prompt(spec, kind, seed, back);
                    }
                }
            }
        }
        self.refresh_settings();
    }

    /// Open the prompt modal on one setting row, seeded with its value in
    /// force; the prompt's commit validates and writes, and closing it
    /// (either way) restores this dialog.
    pub(super) fn enter_setting_prompt(
        &mut self,
        spec: &SettingSpec,
        kind: SettingKind,
        seed: String,
        back: SettingsState,
    ) {
        let edit = SettingEdit {
            key: spec.key,
            label: spec.label,
            kind,
            back,
        };
        self.open_modal(Modal::Prompt(super::PromptState {
            text: seed,
            target: super::PromptTarget::Setting(Box::new(edit)),
        }));
        self.refresh_prompt();
    }

    /// Commit a setting edited in the prompt: validate `text` against the
    /// row's kind (a bad value flashes and the prompt stays up, nothing
    /// written), write it, reload, and restore the dialog. Returns
    /// whether the prompt closed.
    pub(super) fn setting_prompt_commit(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        edit: &SettingEdit,
        text: &str,
    ) -> bool {
        let text = text.trim();
        let value = match edit.kind {
            // An empty hex color clears the override (the built-in).
            SettingKind::Hex if text.is_empty() => Some(SettingValue::Str(String::new())),
            SettingKind::Hex => {
                if crate::mux::config::parse_hex_color(text).is_some() {
                    Some(SettingValue::Str(text.to_string()))
                } else {
                    self.chrome
                        .flash(format!("{}: expected #rrggbb", edit.label));
                    None
                }
            }
            SettingKind::Number { min, max } => match text.parse::<u32>() {
                Ok(n) if (min..=max).contains(&n) => Some(SettingValue::Int(n as i64)),
                _ => {
                    self.chrome
                        .flash(format!("{}: expected {min}..={max}", edit.label));
                    None
                }
            },
            SettingKind::Chord => {
                if crate::mux::config::parse_prefix(text).is_some() {
                    Some(SettingValue::Str(text.to_string()))
                } else {
                    self.chrome.flash(format!(
                        "{}: expected one key, tmux spelling (e.g. x or C-x)",
                        edit.label
                    ));
                    None
                }
            }
            SettingKind::ReloadChord => {
                if crate::mux::config::reload_chord_key(text).is_ok() {
                    Some(SettingValue::Str(text.to_string()))
                } else {
                    self.chrome
                        .flash(format!("{}: expected e.g. \"C-b C-r\"", edit.label));
                    None
                }
            }
            SettingKind::Text => Some(SettingValue::Str(text.to_string())),
            SettingKind::List => Some(SettingValue::List(
                text.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect(),
            )),
            SettingKind::Popup(index) => return self.popup_setting_commit(conn, edit, index, text),
            SettingKind::Toggle | SettingKind::Choice(_) => None,
        };
        let Some(value) = value else {
            self.draw_status_row();
            return false;
        };
        self.settings_write_and_reload(conn, edit.key, value);
        // Back to the dialog either way: a failed write already flashed
        // on the status row.
        self.open_modal(Modal::Settings(edit.back.clone()));
        self.refresh_settings();
        true
    }

    /// Write one `[client]` key and run the reload chord's path (the
    /// client rebinds from the file it just wrote, the daemon re-reads
    /// too). Returns whether the write happened.
    pub(super) fn settings_write_and_reload(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        key: &str,
        value: SettingValue,
    ) -> bool {
        let Some(path) = self.settings_path.clone() else {
            self.chrome
                .flash("settings: no config file location known".to_string());
            return false;
        };
        if let Err(err) = write_client_key(&path, key, &value) {
            self.chrome.flash(format!("settings not saved: {err}"));
            self.draw_status_row();
            return false;
        }
        self.reload_config(conn);
        true
    }

    /// One key while the dialog is up: the arrows select, and ←/→ CHANGE
    /// the selected row's value when it is discrete (a toggle flips, a
    /// choice cycles — the natural gesture on `delivery`; on the
    /// prompt-edited rows and the states/integrations sections they keep
    /// switching sections); Escape closes; everything else is consumed.
    pub(super) fn settings_key(
        &mut self,
        ev: &TermKeyEvent,
        conn: &mut crate::mux::attach::conn::AttachConn,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Escape => self.leave_settings(),
            TermKey::Up => self.settings_move_selection(-1),
            TermKey::Down => self.settings_move_selection(1),
            TermKey::Left | TermKey::Right => {
                let delta = if ev.key() == TermKey::Left { -1 } else { 1 };
                if self.settings_selected_is_discrete() {
                    self.settings_apply(conn, delta);
                } else {
                    self.settings_move_section(delta);
                }
            }
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
            b'\r' | b'\n' | b' ' => self.settings_apply(conn, 1),
            b'u' => {
                let on_integrations = matches!(
                    &self.modal,
                    Modal::Settings(state) if state.section == SettingsSection::Integrations
                );
                if on_integrations {
                    self.settings_update_integrations();
                }
            }
            _ => {}
        }
        matches!(self.modal, Modal::Settings(_))
    }

    /// A press at overlay row `row`, column `col` (inner-relative): a
    /// tab switches to its section, a row selects and applies.
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
        let count = state.section.row_count() + state.popup_rows();
        if index < BODY_ROWS && state.top + index < count {
            let selected = state.top + index;
            if let Modal::Settings(state) = &mut self.modal {
                state.selected = selected;
            }
            self.settings_apply(conn, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> SettingsValues {
        SettingsValues {
            sidebar_on_launch: true,
            sidebar_width: 31,
            agent_rows: SettingsValues::preset_rows("default"),
            border_lines: "herdr".to_string(),
            pane_borders: false,
            show_label_in_border: true,
            pane_gaps: 0,
            border_active_color: String::new(),
            border_color: "#00ffff".to_string(),
            theme: crate::mux::theme::ThemeSelection {
                theme: "Tokyo_Night".to_string(),
                ..crate::mux::theme::ThemeSelection::default()
            },
            scrollbar_gutter: false,
            drag_cursor_shape: false,
            kitty_graphics: true,
            session_picker: false,
            git_status: true,
            resize_step: 1,
            notify_delivery: "terminal".to_string(),
            notify_states: vec!["blocked".to_string(), "idle".to_string()],
            sound: crate::mux::config::SoundSettings::default(),
            prefix: 0x02,
            reload: 0x12,
            management: Management::default(),
        }
    }

    fn state(section: SettingsSection) -> SettingsState {
        SettingsState {
            section,
            selected: 0,
            top: 0,
            integrations: Vec::new(),
            file_mode: "render".to_string(),
            popups: Vec::new(),
        }
    }

    /// The integrations tab's footer names its button: Enter (or `u`)
    /// reinstalls the stale installs, and a stale row's status string
    /// renders under the mark.
    #[test]
    fn integrations_footer_offers_the_update_outdated_button() {
        let mut state = state(SettingsSection::Integrations);
        state.integrations = vec![("claude".to_string(), "v1 \u{2192} v2 outdated".to_string())];
        let composed = compose_settings(&state, &values());
        let footer = &composed.rows.last().unwrap().text;
        assert!(
            footer.contains("update outdated"),
            "the footer offers the button: {footer}"
        );
        let body: Vec<&String> = composed.rows.iter().map(|r| &r.text).collect();
        assert!(
            body.iter().any(|t| t.contains("v1 \u{2192} v2 outdated")),
            "the status string is visible: {body:?}"
        );
    }

    /// The tabs row keeps every section's tab inside the fixed panel
    /// width, and every composed row pads to that width.
    #[test]
    fn tabs_and_rows_fit_the_panel() {
        for section in SettingsSection::ALL {
            let composed = compose_settings(&state(section), &values());
            assert!(
                composed.rows[0].text.chars().count() <= PANEL_WIDTH,
                "{section:?}: tabs overflow: {:?}",
                composed.rows[0].text
            );
            for row in &composed.rows {
                assert_eq!(
                    row.text.chars().count(),
                    PANEL_WIDTH,
                    "{section:?}: every row pads to the fixed width: {:?}",
                    row.text
                );
            }
            assert!(composed.rows.last().unwrap().footer);
        }
    }

    /// A row section shows each key's label with its value in force
    /// right-aligned (an empty hex reads `built-in`), the pointer on the
    /// selection.
    #[test]
    fn row_sections_show_labels_and_values() {
        let mut st = state(SettingsSection::Borders);
        st.selected = 4; // focus color
        let composed = compose_settings(&st, &values());
        let rows: Vec<&str> = composed.rows[5..13]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert!(rows[0].starts_with("   line style"), "{rows:?}");
        assert!(rows[0].trim_end().ends_with("herdr"), "{rows:?}");
        assert!(rows[4].starts_with(" \u{25B8} focus color"), "{rows:?}");
        assert!(rows[1].trim_end().ends_with("off"), "{rows:?}");
        assert!(rows[4].trim_end().ends_with("built-in"), "{rows:?}");
        assert!(rows[5].trim_end().ends_with("#00ffff"), "{rows:?}");
    }

    /// The borders section carries the theme rows (`theme`, `theme-dark`,
    /// `theme-light`) as choice lists, scrolled into view; an alias in
    /// force shows its canonical spelling so Enter cycles from it.
    #[test]
    fn borders_section_lists_the_theme_rows() {
        let keys: Vec<&str> = SettingsSection::Borders
            .rows()
            .iter()
            .map(|spec| spec.key)
            .collect();
        assert_eq!(
            &keys[keys.len() - 3..],
            ["theme", "theme-dark", "theme-light"]
        );
        for spec in &SettingsSection::Borders.rows()[keys.len() - 3..] {
            assert!(matches!(spec.kind, SettingKind::Choice(_)), "{}", spec.key);
        }
        let mut st = state(SettingsSection::Borders);
        st.selected = keys.len() - 3;
        st.top = keys.len() - BODY_ROWS;
        let composed = compose_settings(&st, &values());
        let rows: Vec<&str> = composed.rows[5..5 + BODY_ROWS]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        let theme = rows
            .iter()
            .find(|r| r.starts_with(" \u{25B8} theme"))
            .expect("theme row visible");
        assert!(theme.trim_end().ends_with("tokyo-night"), "{rows:?}");
        assert!(rows
            .iter()
            .any(|r| r.contains("auto: dark host") && r.trim_end().ends_with("default")));
        assert!(rows
            .iter()
            .any(|r| r.contains("auto: light host") && r.trim_end().ends_with("catppuccin-latte")));
    }

    /// The notify section lists every sound key with its value in force:
    /// empty files read `built-in`, an empty mute list `none`, an empty
    /// player `os default`; a long path keeps its tail inside the panel.
    #[test]
    fn notify_section_lists_the_sound_keys() {
        let keys: Vec<&str> = SettingsSection::Notify
            .rows()
            .iter()
            .map(|spec| spec.key)
            .collect();
        assert_eq!(
            keys,
            [
                "notify-delivery",
                "sound",
                "sound-done",
                "sound-blocked",
                "sound-mute",
                "sound-player"
            ]
        );
        let st = state(SettingsSection::Notify);
        let rows = |v: &SettingsValues| -> Vec<String> {
            compose_settings(&st, v).rows[5..11]
                .iter()
                .map(|r| r.text.trim_end().to_string())
                .collect()
        };
        let shown = rows(&values());
        assert!(shown[1].ends_with("off"), "{shown:?}");
        assert!(shown[2].ends_with("built-in"), "{shown:?}");
        assert!(shown[3].ends_with("built-in"), "{shown:?}");
        assert!(shown[4].ends_with("none"), "{shown:?}");
        assert!(shown[5].ends_with("os default"), "{shown:?}");
        let mut v = values();
        v.sound = crate::mux::config::SoundSettings {
            enabled: true,
            player: "mpv --no-video".to_string(),
            done: format!("/very/long/{}/done.mp3", "x".repeat(80)),
            blocked: "b.wav".to_string(),
            mute: vec!["droid".to_string(), "codex".to_string()],
        };
        let shown = rows(&v);
        assert!(shown[1].ends_with("on"), "{shown:?}");
        assert!(shown[2].ends_with("xx/done.mp3"), "{shown:?}");
        assert!(shown[2].contains('\u{2026}'), "{shown:?}");
        assert!(shown[4].ends_with("droid, codex"), "{shown:?}");
        assert!(shown[5].ends_with("mpv --no-video"), "{shown:?}");
        for row in compose_settings(&st, &v).rows {
            assert_eq!(row.text.chars().count(), PANEL_WIDTH, "{:?}", row.text);
        }
    }

    /// The keys section scrolls its window with the selection; the
    /// reload row shows the documented prefix+key spelling.
    #[test]
    fn keys_section_scrolls_and_spells_chords() {
        let mut st = state(SettingsSection::Keys);
        let count = st.section.row_count();
        // The reload chord's spelling (row 1, top of the window).
        st.selected = 1;
        let composed = compose_settings(&st, &values());
        let reload = composed.rows[5 + 1].text.clone();
        assert!(reload.starts_with(" \u{25B8} reload config"), "{reload}");
        assert!(reload.trim_end().ends_with("C-b C-r"), "{reload}");
        // The window's tail: the last row stays visible.
        st.selected = count - 1;
        st.top = count - BODY_ROWS;
        let composed = compose_settings(&st, &values());
        let last = composed.rows[5 + BODY_ROWS - 1].text.clone();
        assert!(last.contains("respawn dead pane"), "{last}");
        assert!(last.trim_end().ends_with(" r"), "{last}");
    }

    /// The preset mapping round-trips: a live default reads `default`,
    /// an unknown layout reads `custom`, and preset_rows renders the
    /// token lines for writing.
    #[test]
    fn agent_rows_presets_round_trip() {
        let mut v = values();
        assert_eq!(v.agent_rows_preset(), "default");
        v.agent_rows = vec![vec!["agent".to_string()]];
        assert_eq!(v.agent_rows_preset(), "custom");
        assert_eq!(
            SettingsValues::preset_rows("compact"),
            vec![vec!["state_icon".to_string(), "agent".to_string()]]
        );
        assert!(SettingsValues::preset_rows("custom").is_empty());
    }
}
