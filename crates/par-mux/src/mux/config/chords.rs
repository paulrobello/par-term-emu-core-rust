//! The client chord set: the resolved prefix and reload keys, the display
//! options, and the notification/sound settings a reload can move live.

use super::{parse_prefix, Management};

/// The reload chord's key byte: the LAST token of the chord spelling,
/// parsed with the shared prefix parser (`parse_prefix`). All tokens must
/// parse — a chord whose head is garbage is a config error, not a silent
/// no-op.
pub fn reload_chord_key(chord: &str) -> Result<u8, String> {
    let key = chord
        .split_whitespace()
        .last()
        .ok_or_else(|| "reload chord is empty (expected e.g. \"C-b C-r\")".to_string())?;
    for token in chord.split_whitespace() {
        if parse_prefix(token).is_none() {
            return Err(format!(
                "reload chord key {token:?} does not parse (expected the tmux spelling, e.g. C-b)"
            ));
        }
    }
    parse_prefix(key).ok_or_else(|| {
        format!("reload chord key {key:?} does not parse (expected the tmux spelling, e.g. C-r)")
    })
}

/// The client chords a reload can move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chords {
    /// The detach prefix byte.
    pub prefix: u8,
    /// The reload chord's key byte.
    pub reload: u8,
    /// The window/pane management chords (split/kill/new-window).
    pub management: Management,
    /// Cells per resize step: each arrow press in resize mode (and each
    /// cell of a divider drag) moves the pane's edge this many cells.
    /// A file value below 1 clamps to 1; the built-in default is 1.
    pub resize_step: u32,
    /// Per-pane border boxes in render mode (config `pane-borders`).
    /// Default off.
    pub pane_borders: bool,
    /// The pane's user title embedded in its top border (config
    /// `show-label-in-border`). Default on.
    pub show_label_in_border: bool,
    /// Visible gap bands between panes in render mode (config
    /// `pane-gaps`). Default 0.
    pub pane_gaps: u16,
    /// The tab strip's per-tab minimum cell width (config
    /// `tab-width-min`). 0 = no floor; when both bounds are set and min
    /// exceeds max, max wins. Default 0.
    pub tab_width_min: u16,
    /// The tab strip's per-tab maximum cell width (config
    /// `tab-width-max`). 0 = no cap. Default 0.
    pub tab_width_max: u16,
    /// The reserved right-edge gutter column (config
    /// `scrollbar-gutter`). Default off.
    pub scrollbar_gutter: bool,
    /// Whether a successful yank leaves copy mode (config
    /// `copy-mode-auto-exit`). Default on.
    pub copy_mode_auto_exit: bool,
    /// The cursor/pointer shaping (config `change-cursor-shape`): the
    /// resize pointer (OSC 22) over divider hovers/drags plus the
    /// DECSCUSR steady block during a live drag. Default on.
    pub change_cursor_shape: bool,
    /// Kitty graphics drawn on a kitty-capable host (config
    /// `kitty-graphics`). Default on.
    pub kitty_graphics: bool,
    /// The target-less multi-session picker (config `session-picker`).
    /// Default off. Read at launch: a reload changes the next attach.
    pub session_picker: bool,
    /// The target-less multi-workspace landing (config
    /// `prefer-default-workspace`): several workspaces resolve to the
    /// one named `default`, else the first listed. Default off. Read at
    /// launch, like the picker.
    pub prefer_default_workspace: bool,
    /// The sidebar's git badge (config `git-status`). Default on. The
    /// daemon reads it to emit the snapshot's `git` row; the client to
    /// render the badge and run its refresh poll.
    pub git_status: bool,
    /// The badge's snapshot re-query interval in seconds (config
    /// `git-poll-secs`); 0 disables polling, otherwise clamped 3-600;
    /// default 10. Live on reload.
    pub git_poll_secs: u64,
    /// The divider/border glyph set (config `border-lines`): `herdr`,
    /// `unicode`, `double`, `heavy`, or `ascii`. Default `herdr`.
    pub border_lines: String,
    /// The agents area's roster ordering (config `agents-sort`): herdr's
    /// `agent_panel_sort` — `grouped` (workspace groups, status priority
    /// beneath) or `priority` (status first). Default `grouped`.
    pub agents_sort: String,
    /// The session strip's visibility (config `session-strip`): `auto`
    /// (shown while the workspace holds two or more sessions), `always`,
    /// or `off`. Default `auto`.
    pub session_strip: String,
    /// The agent-state dots on the session strip and the window tabs
    /// (config `agent-state-dots`): `off` or `on`. Default `off`.
    pub agent_state_dots: String,
    /// The workspace side panel's width in columns when shown.
    pub sidebar_width: u16,
    /// Whether the side panel shows when a render client attaches
    /// (config `sidebar-on-launch`). Default on. Launch-only: the reload
    /// path resolves it but never applies it to a live session.
    pub sidebar_on_launch: bool,
    /// The side panel's agent entry lines (config `agent-rows`): token
    /// spellings per line, validated by the renderer.
    pub agent_rows: Vec<Vec<String>>,
    /// The agent-state notification delivery (config `notify-delivery`):
    /// one of [`NOTIFY_DELIVERIES`].
    pub notify_delivery: String,
    /// The agent states that notify (config `notify-states`): a subset of
    /// [`NOTIFY_STATES`].
    pub notify_states: Vec<String>,
    /// The agent state-change sounds (config `sound*`).
    pub sound: SoundSettings,
    /// The color theme (config `theme`): `default`, `auto`, or a preset
    /// name, kept as spelled (an unknown one warns and uses `default`).
    pub theme: String,
    /// The `auto` theme's dark-host preset (config `theme-dark`).
    pub theme_dark: String,
    /// The `auto` theme's light-host preset (config `theme-light`).
    pub theme_light: String,
    /// The validated popup command windows (config `[[client.popup]]`).
    pub popups: Vec<crate::mux::popup::Popup>,
}

/// The notification sounds (config `sound`, `sound-player`,
/// `sound-done`, `sound-blocked`, `sound-mute`). Default: off, the OS
/// player, the built-in OS sounds, nothing muted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoundSettings {
    /// Whether transitions sound at all (config `sound`).
    pub enabled: bool,
    /// The player command; empty = the OS default.
    pub player: String,
    /// The `done` sound file; empty = built-in.
    pub done: String,
    /// The `blocked` sound file; empty = built-in.
    pub blocked: String,
    /// Agent labels that never sound (matched case-insensitively).
    pub mute: Vec<String>,
}

/// The `notify-delivery` spellings.
pub const NOTIFY_DELIVERIES: &[&str] = &["off", "inside", "terminal", "system"];

/// One warning per display-option value the client does not know: an
/// `agent-rows` token (skipped when rendering), a `notify-delivery`
/// (the default `terminal` applies), a `notify-states` entry (ignored).
/// Pure — what the launch prints and the reload flashes.
pub fn display_option_warnings(chords: &Chords) -> Vec<String> {
    let mut warnings = Vec::new();
    for token in chords.agent_rows.iter().flatten() {
        if !AGENT_ROW_TOKENS.contains(&token.as_str()) {
            warnings.push(format!(
                "[client] agent-rows token {token:?} unknown — skipped (valid: {})",
                AGENT_ROW_TOKENS.join(", ")
            ));
        }
    }
    if !NOTIFY_DELIVERIES.contains(&chords.notify_delivery.as_str()) {
        warnings.push(format!(
            "[client] notify-delivery {:?} unknown — using terminal (valid: {})",
            chords.notify_delivery,
            NOTIFY_DELIVERIES.join(", ")
        ));
    }
    for state in &chords.notify_states {
        if !NOTIFY_STATES.contains(&state.as_str()) {
            warnings.push(format!(
                "[client] notify-states {state:?} unknown — ignored (valid: {})",
                NOTIFY_STATES.join(", ")
            ));
        }
    }
    warnings.extend(crate::mux::theme::theme_warnings(chords));
    warnings.extend(crate::mux::popup::shadow_warnings(chords));
    warnings
}

/// The agent states `notify-states` may select.
pub const NOTIFY_STATES: &[&str] = &["working", "blocked", "question", "done", "idle"];

/// The agent entry's tokens a line may name (config `agent-rows`).
pub const AGENT_ROW_TOKENS: &[&str] = &[
    "state_icon",
    "state",
    "workspace",
    "project",
    "message",
    "agent",
    "pane",
];

/// The default agent entry: the state icon, workspace, and project on
/// the first line, the agent on the second (herdr's two-line entry).
pub fn default_agent_rows() -> Vec<Vec<String>> {
    vec![
        vec!["state_icon".into(), "workspace".into(), "project".into()],
        vec!["agent".into()],
    ]
}

impl Chords {
    /// The built-in chord set: every default chord at its tmux spelling
    /// and the 1-cell resize step.
    pub fn with_defaults() -> Self {
        Self {
            prefix: 0x02,
            reload: 0x12,
            management: Management::default(),
            resize_step: 1,
            pane_borders: false,
            show_label_in_border: true,
            pane_gaps: 0,
            scrollbar_gutter: false,
            copy_mode_auto_exit: true,
            change_cursor_shape: true,
            kitty_graphics: true,
            session_picker: false,
            prefer_default_workspace: false,
            git_status: true,
            git_poll_secs: 10,
            tab_width_min: 0,
            tab_width_max: 0,
            border_lines: "herdr".to_string(),
            agents_sort: "grouped".to_string(),
            session_strip: "auto".to_string(),
            agent_state_dots: "off".to_string(),
            sidebar_width: 20,
            sidebar_on_launch: true,
            agent_rows: default_agent_rows(),
            notify_delivery: "terminal".to_string(),
            notify_states: vec!["blocked".to_string(), "done".to_string()],
            sound: SoundSettings::default(),
            theme: crate::mux::theme::DEFAULT.to_string(),
            theme_dark: crate::mux::theme::DEFAULT.to_string(),
            theme_light: crate::mux::theme::DEFAULT_LIGHT.to_string(),
            popups: Vec::new(),
        }
    }
}
