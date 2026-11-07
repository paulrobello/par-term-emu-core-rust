//! `[client]`: the attach-client section of the config file — every chord
//! and display option at its file spelling.

use serde::{Deserialize, Serialize};

/// `[client]`: what the attach client reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClientSection {
    /// The detach prefix, tmux spelling (e.g. `C-b`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Default attach mode: `render` (the built-in default) or
    /// `passthrough`. Startup-only — a running attach does not switch
    /// pipelines; `--mode` overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The reload chord, spelled as prefix + key (e.g. `C-b C-r`): the
    /// LAST token is the key matched after the prefix; earlier tokens are
    /// the chord's documented head and must parse as keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reload: Option<String>,
    /// The split-right chord (tmux's `%`): the key matched after the
    /// prefix that splits the focused pane and lands on the new pane.
    #[serde(
        default,
        rename = "split-right",
        skip_serializing_if = "Option::is_none"
    )]
    pub split_right: Option<String>,
    /// The `layout-even-horizontal` layout-preset chord (default `M-1`).
    #[serde(
        default,
        rename = "layout-even-horizontal",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_even_horizontal: Option<String>,
    /// The `layout-even-vertical` layout-preset chord (default `M-2`).
    #[serde(
        default,
        rename = "layout-even-vertical",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_even_vertical: Option<String>,
    /// The `layout-main-horizontal` layout-preset chord (default `M-3`).
    #[serde(
        default,
        rename = "layout-main-horizontal",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_main_horizontal: Option<String>,
    /// The `layout-main-vertical` layout-preset chord (default `M-4`).
    #[serde(
        default,
        rename = "layout-main-vertical",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_main_vertical: Option<String>,
    /// The `layout-tiled` layout-preset chord (default `M-5`).
    #[serde(
        default,
        rename = "layout-tiled",
        skip_serializing_if = "Option::is_none"
    )]
    pub layout_tiled: Option<String>,
    /// The split-down chord (tmux's `"`): the key matched after the
    /// prefix that splits the focused pane below and lands on the new pane.
    #[serde(
        default,
        rename = "split-down",
        skip_serializing_if = "Option::is_none"
    )]
    pub split_down: Option<String>,
    /// The kill-pane chord (tmux's `x`): the key matched after the prefix
    /// that kills the focused pane and lands on the successor.
    #[serde(default, rename = "kill-pane", skip_serializing_if = "Option::is_none")]
    pub kill_pane: Option<String>,
    /// The break-pane chord (`!`): the key matched after the prefix that
    /// breaks the focused pane out into a new window of its session and
    /// lands on it.
    #[serde(
        default,
        rename = "break-pane",
        skip_serializing_if = "Option::is_none"
    )]
    pub break_pane: Option<String>,
    /// The join-pane chord (`J`): the key matched after the prefix that
    /// opens the window picker in join mode — the picked window receives
    /// the focused pane.
    #[serde(default, rename = "join-pane", skip_serializing_if = "Option::is_none")]
    pub join_pane: Option<String>,
    /// The command-palette chord (`:`): the key matched after the prefix
    /// that opens the filterable palette over every management action.
    #[serde(default, rename = "palette", skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    /// The new-window chord (tmux's `c`): the key matched after the prefix
    /// that opens a window in the focused pane's session and lands on it.
    #[serde(
        default,
        rename = "new-window",
        skip_serializing_if = "Option::is_none"
    )]
    pub new_window: Option<String>,
    /// The resize chord: the key matched after the prefix that enters the
    /// sticky resize mode (arrows adjust the focused pane's edges; Enter,
    /// Escape, and `q` exit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<String>,
    /// Cells per resize step: each arrow press in resize mode (and each
    /// cell of divider drag) moves the pane's edge this many cells.
    #[serde(
        default,
        rename = "resize-step",
        skip_serializing_if = "Option::is_none"
    )]
    pub resize_step: Option<u32>,
    /// The swap-with-previous-pane chord (tmux's `{`).
    #[serde(default, rename = "swap-prev", skip_serializing_if = "Option::is_none")]
    pub swap_prev: Option<String>,
    /// The swap-with-next-pane chord (tmux's `}`).
    #[serde(default, rename = "swap-next", skip_serializing_if = "Option::is_none")]
    pub swap_next: Option<String>,
    /// The move-active-tab-left chord (tmux's `<`).
    #[serde(
        default,
        rename = "move-window-left",
        skip_serializing_if = "Option::is_none"
    )]
    pub move_window_left: Option<String>,
    /// The move-active-tab-right chord (tmux's `>`).
    #[serde(
        default,
        rename = "move-window-right",
        skip_serializing_if = "Option::is_none"
    )]
    pub move_window_right: Option<String>,
    /// The scroll-the-tab-strip-left chord.
    #[serde(
        default,
        rename = "tab-strip-left",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_strip_left: Option<String>,
    /// The scroll-the-tab-strip-right chord.
    #[serde(
        default,
        rename = "tab-strip-right",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_strip_right: Option<String>,
    /// The next-workspace chord: the key matched after the prefix that
    /// selects the next workspace (id order) and lands the view on it.
    #[serde(
        default,
        rename = "workspace-next",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_next: Option<String>,
    /// The previous-workspace chord: the key matched after the prefix
    /// that selects the previous workspace (id order) and lands the view
    /// on it.
    #[serde(
        default,
        rename = "workspace-prev",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_prev: Option<String>,
    /// The help chord: the key matched after the prefix that opens the
    /// bindings panel (every chord shown at its effective binding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// The picker chord: the key matched after the prefix that opens the
    /// window picker modal (every session's windows as one flat list,
    /// the shown one emphasized, a name two sessions share prefixed
    /// with its session's name, keyboard and mouse navigable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker: Option<String>,
    /// The zoom chord (tmux's `z`): the key matched after the prefix
    /// that toggles the focused pane between its layout rect and the
    /// full window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<String>,
    /// The rename-window chord (tmux's `,`): the key matched after the
    /// prefix that opens the rename prompt for the shown window.
    #[serde(
        default,
        rename = "rename-window",
        skip_serializing_if = "Option::is_none"
    )]
    pub rename_window: Option<String>,
    /// The rename-pane chord (tmux's `$`): the key matched after the
    /// prefix that opens the rename prompt for the focused pane's user
    /// title.
    #[serde(
        default,
        rename = "rename-pane",
        skip_serializing_if = "Option::is_none"
    )]
    pub rename_pane: Option<String>,
    /// The border-style chord: the key matched after the prefix that
    /// cycles the divider/border glyph set (unicode → double → heavy →
    /// ascii).
    #[serde(
        default,
        rename = "border-cycle",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_cycle: Option<String>,
    /// The pane-labels chord: the key matched after the prefix that
    /// toggles each pane's title embedded in its border (render mode).
    #[serde(
        default,
        rename = "label-toggle",
        skip_serializing_if = "Option::is_none"
    )]
    pub label_toggle: Option<String>,
    /// The workspace-picker chord: the key matched after the prefix that
    /// opens the workspace list modal (render mode).
    #[serde(
        default,
        rename = "workspace-picker",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_picker: Option<String>,
    /// The sidebar chord: the key matched after the prefix that toggles
    /// the workspace side panel (render mode).
    #[serde(default, rename = "sidebar", skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<String>,
    /// The status-bar chord: the key matched after the prefix that
    /// toggles the bottom status bar (render mode; shown by default).
    #[serde(
        default,
        rename = "status-bar",
        skip_serializing_if = "Option::is_none"
    )]
    pub status_bar: Option<String>,
    /// The respawn chord: the key matched after the prefix that restarts
    /// the focused pane when it is held dead (`respawn-pane`; a live pane
    /// is left alone).
    #[serde(default, rename = "respawn", skip_serializing_if = "Option::is_none")]
    pub respawn: Option<String>,
    /// The notification-jump chord: the key matched after the prefix that
    /// focuses the pane behind the most recent agent notification this
    /// client delivered (render mode).
    #[serde(
        default,
        rename = "notify-jump",
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_jump: Option<String>,
    /// The side panel's width in columns (render mode; clamped 6..=60).
    #[serde(
        default,
        rename = "sidebar-width",
        skip_serializing_if = "Option::is_none"
    )]
    pub sidebar_width: Option<u16>,
    /// Show the side panel when a render client attaches (default true).
    /// Read at launch only: the sidebar chord flips it at runtime and a
    /// config reload leaves the live panel as it is.
    #[serde(
        default,
        rename = "sidebar-on-launch",
        skip_serializing_if = "Option::is_none"
    )]
    pub sidebar_on_launch: Option<bool>,
    /// The side panel's agent entry, one inner array per line, each a
    /// list of tokens joined by ` · ` (herdr's `[ui.sidebar.agents]
    /// rows`): `state_icon`, `state`, `workspace`, `project`, `agent`,
    /// `pane`. A token with nothing to show drops out; an unknown token
    /// warns and is skipped. Default `[["state_icon", "workspace",
    /// "project"], ["agent"]]`.
    #[serde(
        default,
        rename = "agent-rows",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_rows: Option<Vec<Vec<String>>>,
    /// How an agent state change is announced (herdr's `[ui.toast]
    /// delivery`): `terminal` (default — the host terminal's
    /// notification escape, OSC 9 or kitty's OSC 99), `inside` (a toast
    /// card over the panes), `system` (the OS notifier), or `off`.
    #[serde(
        default,
        rename = "notify-delivery",
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_delivery: Option<String>,
    /// The agent states whose arrival notifies (`working`, `blocked`,
    /// `done`, `idle`). Default `["blocked", "done"]` — needs attention,
    /// and finished while out of view.
    #[serde(
        default,
        rename = "notify-states",
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_states: Option<Vec<String>>,
    /// Play a sound on agent state changes (herdr's `[ui.sound]
    /// enabled`): `blocked` when an agent needs input, `done` when one
    /// finishes out of view. Independent of `notify-delivery`. Default
    /// off.
    #[serde(default, rename = "sound", skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
    /// The player command a sound file is handed to, split on
    /// whitespace with the file appended as the last argument. Empty =
    /// the OS default (`afplay` on macOS, `paplay` elsewhere on Unix;
    /// none on Windows, where the terminal bell rings instead).
    #[serde(
        default,
        rename = "sound-player",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_player: Option<String>,
    /// The `done` sound's file. Empty = the built-in OS sound; a relative
    /// path resolves from the config file's directory.
    #[serde(
        default,
        rename = "sound-done",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_done: Option<String>,
    /// The `blocked` sound's file. Empty = the built-in OS sound; a
    /// relative path resolves from the config file's directory.
    #[serde(
        default,
        rename = "sound-blocked",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_blocked: Option<String>,
    /// Agent labels whose state changes never sound (herdr's
    /// `[ui.sound.agents] <agent> = "off"`). Default empty.
    #[serde(
        default,
        rename = "sound-mute",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_mute: Option<Vec<String>>,
    /// The divider/border line style: `herdr` (default), `unicode`,
    /// `double`, `heavy`, or `ascii`. An unknown value warns and uses
    /// the default.
    #[serde(
        default,
        rename = "border-lines",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_lines: Option<String>,
    /// Per-pane border boxes in render mode (herdr's look): each pane
    /// renders its own complete ring, focused accent vs dim, content
    /// inset by the border cells. Default off — the shared-divider look.
    #[serde(
        default,
        rename = "pane-borders",
        skip_serializing_if = "Option::is_none"
    )]
    pub pane_borders: Option<bool>,
    /// The pane's user title embedded in its top border (only meaningful
    /// with `pane-borders`). Default on.
    #[serde(
        default,
        rename = "show-label-in-border",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_label_in_border: Option<bool>,
    /// Visible gap bands between panes in render mode (herdr's look):
    /// each pane's rect insets by this many cells per side and the band
    /// fills with the theme background. Default 0.
    #[serde(default, rename = "pane-gaps", skip_serializing_if = "Option::is_none")]
    pub pane_gaps: Option<u16>,
    /// The tab strip's per-tab minimum cell width in columns (config
    /// `tab-width-min`): a tab whose name is shorter still gets a block
    /// this wide. 0 = no floor. Default 0.
    #[serde(
        default,
        rename = "tab-width-min",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_width_min: Option<u16>,
    /// The tab strip's per-tab maximum cell width in columns (config
    /// `tab-width-max`): a longer name truncates and ellipsizes to fit;
    /// 0 = no cap. When both are set and min exceeds max, max wins.
    /// Default 0.
    #[serde(
        default,
        rename = "tab-width-max",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_width_max: Option<u16>,
    /// Reserve a right-edge gutter column in every render-mode pane rect
    /// (content narrows by one; the gutter shows a minimal scroll
    /// position indicator while the pane is scrolled). Default off.
    #[serde(
        default,
        rename = "scrollbar-gutter",
        skip_serializing_if = "Option::is_none"
    )]
    pub scrollbar_gutter: Option<bool>,
    /// Leave copy mode when a yank succeeds (config
    /// `copy-mode-auto-exit`). Default on.
    #[serde(
        default,
        rename = "copy-mode-auto-exit",
        skip_serializing_if = "Option::is_none"
    )]
    pub copy_mode_auto_exit: Option<bool>,
    /// While the pointer is over a resize handle, and while a drag is
    /// live, shape the host cursor to the resize shape (best-effort
    /// DECSCUSR; restored on leave/drag end). Default on.
    #[serde(
        default,
        rename = "change-cursor-shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub change_cursor_shape: Option<bool>,
    /// Draw kitty graphics protocol images the panes emit on the host
    /// terminal (render mode; herdr's `terminal.kitty_graphics`). Only
    /// takes effect when the host answers the launch kitty query.
    /// Default on.
    #[serde(
        default,
        rename = "kitty-graphics",
        skip_serializing_if = "Option::is_none"
    )]
    pub kitty_graphics: Option<bool>,
    /// A target-less attach against several sessions opens an inline
    /// session picker instead of refusing (both modes; only when stdin
    /// and stdout are both terminals). Default off.
    #[serde(
        default,
        rename = "session-picker",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_picker: Option<bool>,
    /// A target-less attach against several WORKSPACES lands the
    /// workspace named `default`, else the first listed one, instead of
    /// refusing or picking (both modes). One workspace keeps the
    /// target-less rule. Default off.
    #[serde(
        default,
        rename = "prefer-default-workspace",
        skip_serializing_if = "Option::is_none"
    )]
    pub prefer_default_workspace: Option<bool>,
    /// The workspace side panel's git badge (` main ↑2` under the
    /// displayed session's workspace; render mode). Default on.
    #[serde(
        default,
        rename = "git-status",
        skip_serializing_if = "Option::is_none"
    )]
    pub git_status: Option<bool>,
    /// The git badge's snapshot re-query interval in seconds
    /// (`[client] git-poll-secs`); 0 disables polling, otherwise
    /// clamped 3-600; default 10. File-only.
    #[serde(
        default,
        rename = "git-poll-secs",
        skip_serializing_if = "Option::is_none"
    )]
    pub git_poll_secs: Option<u64>,
    /// The agents area's roster ordering (herdr's `agent_panel_sort`):
    /// `grouped` (workspace groups, status priority beneath) or
    /// `priority` (status first). Default `grouped`.
    #[serde(
        default,
        rename = "agents-sort",
        skip_serializing_if = "Option::is_none"
    )]
    pub agents_sort: Option<String>,
    /// The session strip's visibility (render mode): `auto` (the default
    /// — shown while the workspace holds two or more sessions), `always`,
    /// or `off`.
    #[serde(
        default,
        rename = "session-strip",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_strip: Option<String>,
    /// The agent-state dots on the session strip and the window tabs
    /// (render mode): `off` (the default) or `on`.
    #[serde(
        default,
        rename = "agent-state-dots",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_state_dots: Option<String>,
    /// The focused pane's border/divider highlight, `#rrggbb` hex. Empty
    /// = the built-in accent (bright cyan). Config-file only, like the
    /// other `[client]` display options.
    #[serde(
        default,
        rename = "border-active-color",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_active_color: Option<String>,
    /// Unfocused border/divider color, `#rrggbb` hex. Empty = the
    /// built-in dim look. Config-file only.
    #[serde(
        default,
        rename = "border-color",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_color: Option<String>,
    /// The render-mode color theme (herdr's presets): `default` (the
    /// built-in look), `auto` (follow the host's light/dark appearance
    /// through `theme-dark` / `theme-light`), or a preset name — see
    /// [`crate::mux::theme::THEME_NAMES`]. An unknown name warns and
    /// uses `default`.
    #[serde(default, rename = "theme", skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The preset `theme = "auto"` uses on a dark host. Default
    /// `default`.
    #[serde(
        default,
        rename = "theme-dark",
        skip_serializing_if = "Option::is_none"
    )]
    pub theme_dark: Option<String>,
    /// The preset `theme = "auto"` uses on a light host. Default
    /// `catppuccin-latte`.
    #[serde(
        default,
        rename = "theme-light",
        skip_serializing_if = "Option::is_none"
    )]
    pub theme_light: Option<String>,
    /// Popup command windows (herdr's `type = "popup"` custom commands):
    /// `[[client.popup]]` tables of `key`, `command`, and optional
    /// `width`/`height` (cells or `"N%"`) and `description`. See
    /// [`crate::mux::popup`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<Vec<crate::mux::popup::PopupEntry>>,
}
