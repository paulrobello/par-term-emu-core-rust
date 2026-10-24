//! The par-mux config file (`<config dir>/par-mux/config.toml`).
//!
//! One file carries client defaults (the attach prefix, the default attach
//! mode, the reload chord) and daemon defaults (socket, state dir, the
//! pane-endpoint switches). Resolution precedence everywhere is
//! **flags > env > file > built-in defaults**: a CLI flag wins, then the
//! relevant environment variable (`PAR_MUX_SOCKET` for the socket target —
//! the env var every pane spawns with), then the file, then the built-in.
//!
//! Reload semantics per setting (see docs/MUX.md "Configuration"):
//! - `client.prefix`, `client.reload`: LIVE — a running attach rebinds
//!   its chords immediately.
//! - everything else: startup-only. `daemon.state-dir` in particular is
//!   resolved once at daemon startup (`run_daemon` in the binary) and the
//!   persist worker writes to that fixed path, so a re-read can only
//!   report it restart-required.
//!
//! The reload entry points: the `reload-config` control command (daemon
//! side: re-read, report per-setting), the attach client's reload chord
//! (default `C-b C-r`, itself configurable as `[client] reload` — it
//! rebinds the client chords live and sends `reload-config` so the
//! daemon-side report rides the same keypress), and `--gen-config`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The config file's on-disk shape. Every field is optional: a missing
/// file, an empty file, and a file with only some keys all resolve against
/// the later tiers. Unknown keys are ignored (forward compatibility).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConfigFile {
    /// Attach-client defaults.
    #[serde(default)]
    pub client: ClientSection,
    /// Daemon defaults.
    #[serde(default)]
    pub daemon: DaemonSection,
    /// Saved remote hosts (`[machines.<label>]`): SSH thin-client profiles.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub machines: std::collections::BTreeMap<String, MachineProfile>,
}

/// `[machines.<label>]`: one saved remote par-mux host. The profile stores
/// no credentials — ssh (the user's own config, agent, and askpass) owns
/// authentication, exactly as herdr's machines do.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MachineProfile {
    /// The ssh target as openssh spells it (`user@host`, `host`, an alias
    /// the user's ssh config resolves; a `user@host:port` colon form is
    /// kept verbatim for ssh's `-p`-less spelling).
    pub target: String,
    /// The remote session this machine's attach lands on by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Disabled machines are listed but never attached without naming
    /// them explicitly.
    #[serde(
        default = "machine_enabled_by_default",
        skip_serializing_if = "machine_enabled"
    )]
    pub enabled: bool,
}

fn machine_enabled_by_default() -> bool {
    true
}

fn machine_enabled(enabled: &bool) -> bool {
    *enabled
}

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
    /// session/window picker modal (sessions with their windows nested,
    /// the current one emphasized, keyboard and mouse navigable).
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
    /// Reserve a right-edge gutter column in every render-mode pane rect
    /// (content narrows by one; the gutter shows a minimal scroll
    /// position indicator while the pane is scrolled). Default off.
    #[serde(
        default,
        rename = "scrollbar-gutter",
        skip_serializing_if = "Option::is_none"
    )]
    pub scrollbar_gutter: Option<bool>,
    /// While a divider drag is live, shape the host cursor to the resize
    /// shape (best-effort DECSCUSR; restored on drag end). Default off.
    #[serde(
        default,
        rename = "drag-cursor-shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub drag_cursor_shape: Option<bool>,
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

/// `[daemon.agent-append]`: extra command-line words the daemon appends
/// when `agent-start` (re)launches that agent in a pane — the resume
/// flags a shell alias would carry (`--continue` and friends). Words are
/// whitespace-split and every one is shell-quoted before typing, so a
/// value can never close the quoting and run shell syntax; `{name}` is
/// the one template var, replaced daemon-side with the pane cwd's
/// basename. Each key defaults to blank (the agent launches bare).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct AgentAppendFile {
    /// Extra words appended when `agent-start` launches claude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    /// Extra words appended when `agent-start` launches codex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
    /// Extra words appended when `agent-start` launches grok.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok: Option<String>,
    /// Extra words appended when `agent-start` launches muse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muse: Option<String>,
    /// Extra words appended when `agent-start` launches droid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub droid: Option<String>,
    /// Extra words appended when `agent-start` launches copilot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot: Option<String>,
    /// Extra words appended when `agent-start` launches cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Extra words appended when `agent-start` launches kimi.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kimi: Option<String>,
    /// Extra words appended when `agent-start` launches qwen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qwen: Option<String>,
    /// Extra words appended when `agent-start` launches qodercli.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qodercli: Option<String>,
    /// Extra words appended when `agent-start` launches devin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devin: Option<String>,
    /// Extra words appended when `agent-start` launches gemini.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini: Option<String>,
    /// Extra words appended when `agent-start` launches amp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amp: Option<String>,
    /// Extra words appended when `agent-start` launches mastracode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mastracode: Option<String>,
}

fn deserialize_update_interval<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(deserializer)?;
    if !matches!(value.as_str(), "hourly" | "daily" | "weekly") {
        return Err(serde::de::Error::custom(
            "update-interval must be hourly, daily, or weekly",
        ));
    }
    Ok(Some(value))
}

/// `[daemon]`: what the daemon reads at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DaemonSection {
    /// Automatically apply stable releases. Off unless explicitly enabled.
    #[serde(default, rename = "auto-update")]
    pub auto_update: bool,
    /// Update cadence (hourly, daily, weekly), default daily.
    #[serde(
        default,
        rename = "update-interval",
        deserialize_with = "deserialize_update_interval",
        skip_serializing_if = "Option::is_none"
    )]
    pub update_interval: Option<String>,
    /// Named default socket (`default` or a name) or an absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    /// State directory override; empty = the OS default.
    #[serde(default, rename = "state-dir", skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    /// The `--pane-endpoints` default.
    #[serde(
        default,
        rename = "pane-endpoints",
        skip_serializing_if = "Option::is_none"
    )]
    pub pane_endpoints: Option<bool>,
    /// The `--expose-control-socket` default.
    #[serde(
        default,
        rename = "expose-control-socket",
        skip_serializing_if = "Option::is_none"
    )]
    pub expose_control_socket: Option<bool>,
    /// Hold a pane whose child exited instead of removing it. `false`
    /// (the default) auto-removes a dead pane through the kill-pane
    /// contract at the reaper's next pass; `true` preserves the
    /// held-dead behavior (`respawn-pane` can restart it in place).
    #[serde(
        default,
        rename = "remain-on-exit",
        skip_serializing_if = "Option::is_none"
    )]
    pub remain_on_exit: Option<bool>,
    /// Exit the daemon once it holds no sessions (or only dead panes)
    /// for the 5 s grace (tmux's `exit-empty`); attached clients do not
    /// hold it. `true` (the default)
    /// matches the built-in behavior; `false` keeps a persisting daemon
    /// alive however long it sits empty.
    #[serde(
        default,
        rename = "exit-empty",
        skip_serializing_if = "Option::is_none"
    )]
    pub exit_empty: Option<bool>,
    /// What a `split-window` pane starts in when no `-c` is given:
    /// `home-dir` (the default), `previous-pane-dir` (the target pane's
    /// live cwd), or `workspace-dir` (the workspace's launch folder — a
    /// plain workspace has none and keeps the home default). Any other
    /// spelling keeps the home default.
    #[serde(
        default,
        rename = "split-start-dir",
        skip_serializing_if = "Option::is_none"
    )]
    pub split_start_dir: Option<String>,
    /// Per-agent resume words appended at `agent-start` (see
    /// [`AgentAppendFile`]); a key the file omits stays blank.
    #[serde(default, rename = "agent-append")]
    pub agent_append: AgentAppendFile,
}

/// The fully resolved settings after flags > env > file > defaults.
/// `gen-config` renders exactly this; `reload-config` compares a re-read
/// file's daemon section against the daemon's startup copy of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    /// Startup-only automatic update opt-in.
    pub auto_update: bool,
    /// Startup-only update cadence, default daily.
    pub update_interval: String,
    /// The attach detach prefix, tmux spelling.
    pub prefix: String,
    /// The default attach mode: `render` (the pane renderer) or
    /// `passthrough` (the byte pump). RENDER IS THE DEFAULT — passthrough
    /// is the opt-out.
    pub mode: String,
    /// The client reload chord, prefix + key (e.g. `C-b C-r`).
    pub reload: String,
    /// The socket target: `default`, a name, or an absolute path.
    pub socket: String,
    /// The state directory override; empty = OS default.
    pub state_dir: String,
    /// The pane-endpoints default.
    pub pane_endpoints: bool,
    /// The expose-control-socket default.
    pub expose_control_socket: bool,
    /// Hold a pane whose child exited instead of removing it. The one
    /// live daemon setting: `reload-config` applies a changed value at
    /// once (it takes effect at the next observed death), everything
    /// else is restart-required.
    pub remain_on_exit: bool,
    /// Exit the daemon once it holds no sessions (or only dead panes)
    /// for the 5 s grace; attached clients do not hold it. The second
    /// live daemon setting: the accept
    /// loop re-reads the applied copy every tick, so `reload-config`
    /// applies a changed value without a restart.
    pub exit_empty: bool,
    /// The focused pane's border/divider highlight (`[client]
    /// border-active-color`, `#rrggbb` hex); empty = the built-in accent
    /// (bright cyan). File-only: no attach flag sets it.
    pub border_active_color: String,
    /// Unfocused border/divider color (`[client] border-color`,
    /// `#rrggbb` hex); empty = the built-in dim look. File-only: no
    /// attach flag sets it.
    pub border_color: String,
    /// The sidebar git badge (`[client] git-status`); file-only, no
    /// attach flag sets it. The daemon-side gate for the snapshot's
    /// `git` row.
    pub git_status: bool,
    /// The git badge's snapshot re-query interval in seconds
    /// (`[client] git-poll-secs`); 0 disables polling, otherwise
    /// clamped 3-600; default 10. Live on
    /// reload.
    pub git_poll_secs: u64,
    /// What a `split-window` pane starts in when no `-c` is given
    /// (`[daemon] split-start-dir`): `home-dir` | `previous-pane-dir` |
    /// `workspace-dir`; any other spelling keeps the home default. The
    /// third live daemon setting: read at each split, so `reload-config`
    /// applies a change at once.
    pub split_start_dir: String,
    /// The config file this copy was resolved from, stamped by the daemon
    /// before serving (its `effective()` read resolves the same path).
    /// `reload-config` re-reads THIS path, so an env change mid-run cannot
    /// move the file under a running daemon. `None` = a hand-built copy
    /// (embedders, tests): the dispatch falls back to the canonical
    /// lookup. Not a setting: `render` never emits it and the reload diff
    /// never compares it.
    pub config_path: Option<std::path::PathBuf>,
    /// The resolved per-agent resume words (`[daemon.agent-append]`);
    /// blank = the agent launches bare. The fourth live daemon setting:
    /// read at each `agent-start`, so `reload-config` applies a change at
    /// once.
    pub agent_append: AgentAppend,
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
            auto_update: false,
            update_interval: "daily".into(),
            prefix: "C-b".to_string(),
            mode: "render".to_string(),
            reload: "C-b C-r".to_string(),
            socket: "default".to_string(),
            state_dir: String::new(),
            pane_endpoints: false,
            expose_control_socket: false,
            remain_on_exit: false,
            exit_empty: true,
            border_active_color: String::new(),
            border_color: String::new(),
            git_status: true,
            git_poll_secs: 10,
            split_start_dir: "home-dir".to_string(),
            agent_append: AgentAppend::default(),
            config_path: None,
        }
    }
}

/// The resolved `[daemon.agent-append]` table: one entry per supported
/// agent, blank = launch bare.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentAppend {
    /// The resolved claude entry; blank = launch bare.
    pub claude: String,
    /// The resolved codex entry; blank = launch bare.
    pub codex: String,
    /// The resolved grok entry; blank = launch bare.
    pub grok: String,
    /// The resolved muse entry; blank = launch bare.
    pub muse: String,
    /// The resolved droid entry; blank = launch bare.
    pub droid: String,
    /// The resolved copilot entry; blank = launch bare.
    pub copilot: String,
    /// The resolved cursor entry; blank = launch bare.
    pub cursor: String,
    /// The resolved kimi entry; blank = launch bare.
    pub kimi: String,
    /// The resolved qwen entry; blank = launch bare.
    pub qwen: String,
    /// The resolved qodercli entry; blank = launch bare.
    pub qodercli: String,
    /// The resolved devin entry; blank = launch bare.
    pub devin: String,
    /// The resolved gemini entry; blank = launch bare.
    pub gemini: String,
    /// The resolved amp entry; blank = launch bare.
    pub amp: String,
    /// The resolved mastracode entry; blank = launch bare.
    pub mastracode: String,
}

impl AgentAppend {
    /// The entry for the `agent-start` kind: one of the supported kinds,
    /// or blank for any other (an unknown kind still launches, bare).
    #[must_use]
    pub fn entry(&self, kind: &str) -> &str {
        match kind {
            "claude" => &self.claude,
            "codex" => &self.codex,
            "grok" => &self.grok,
            "muse" => &self.muse,
            "droid" => &self.droid,
            "copilot" => &self.copilot,
            "cursor" => &self.cursor,
            "kimi" => &self.kimi,
            "qwen" => &self.qwen,
            "qodercli" => &self.qodercli,
            "devin" => &self.devin,
            "gemini" => &self.gemini,
            "amp" => &self.amp,
            "mastracode" => &self.mastracode,
            _ => "",
        }
    }
}

/// The flag- and env-tier inputs to [`resolve`]. `None` = that tier did
/// not speak for the setting, so the file (then the built-in default)
/// decides. Bool flags are `Some(true)` only when the flag was passed —
/// they are bare on/off switches and can only vote the setting on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `--prefix` (attach).
    pub prefix: Option<String>,
    /// `--mode` (attach).
    pub mode: Option<String>,
    /// `--socket` / positional NAME resolved to its value; the flag tier
    /// of the socket target.
    pub socket: Option<String>,
    /// `--state-dir`.
    pub state_dir: Option<String>,
    /// `--pane-endpoints` passed.
    pub pane_endpoints: Option<bool>,
    /// `--expose-control-socket` passed.
    pub expose_control_socket: Option<bool>,
    /// `$PAR_MUX_SOCKET` (non-empty), the env tier of the socket target.
    pub env_socket: Option<String>,
}

/// Merge the tiers: each setting takes the first tier that speaks, ending
/// at [`EffectiveConfig::default`].
#[must_use]
pub fn resolve(file: &ConfigFile, o: &Overrides) -> EffectiveConfig {
    let mut eff = EffectiveConfig {
        auto_update: file.daemon.auto_update,
        update_interval: file
            .daemon
            .update_interval
            .clone()
            .unwrap_or_else(|| "daily".into()),
        ..EffectiveConfig::default()
    };
    if let Some(v) = o.prefix.as_ref().or(file.client.prefix.as_ref()) {
        eff.prefix = v.clone();
    }
    if let Some(v) = o.mode.as_ref().or(file.client.mode.as_ref()) {
        eff.mode = v.clone();
    }
    if let Some(v) = file.client.reload.as_ref() {
        eff.reload = v.clone();
    }
    // Socket target: flag tier, then env, then file, then the default
    // name. An empty env value counts as unset (the pane-expansion rule),
    // and so does an empty file value.
    let socket = o
        .socket
        .clone()
        .or_else(|| o.env_socket.clone().filter(|v| !v.is_empty()))
        .or_else(|| file.daemon.socket.clone().filter(|v| !v.is_empty()));
    if let Some(v) = socket {
        eff.socket = v;
    }
    if let Some(v) = o.state_dir.as_ref().or(file.daemon.state_dir.as_ref()) {
        eff.state_dir = v.clone();
    }
    if let Some(v) = o.pane_endpoints.or(file.daemon.pane_endpoints) {
        eff.pane_endpoints = v;
    }
    if let Some(v) = o
        .expose_control_socket
        .or(file.daemon.expose_control_socket)
    {
        eff.expose_control_socket = v;
    }
    if let Some(v) = file.daemon.remain_on_exit {
        eff.remain_on_exit = v;
    }
    if let Some(v) = file.daemon.exit_empty {
        eff.exit_empty = v;
    }
    // split-start-dir is file-only daemon behavior (no flag or env
    // tier); an unknown spelling keeps the home default.
    if let Some(v) = file.daemon.split_start_dir.as_ref() {
        eff.split_start_dir = v.clone();
    }
    // agent-append is file-only daemon behavior like split-start-dir; a
    // stated key replaces the blank entry, an omitted key keeps it.
    let stated_append = &file.daemon.agent_append;
    for (stated, resolved) in [
        (stated_append.claude.as_ref(), &mut eff.agent_append.claude),
        (stated_append.codex.as_ref(), &mut eff.agent_append.codex),
        (stated_append.grok.as_ref(), &mut eff.agent_append.grok),
        (stated_append.muse.as_ref(), &mut eff.agent_append.muse),
        (stated_append.droid.as_ref(), &mut eff.agent_append.droid),
        (
            stated_append.copilot.as_ref(),
            &mut eff.agent_append.copilot,
        ),
        (stated_append.cursor.as_ref(), &mut eff.agent_append.cursor),
        (stated_append.kimi.as_ref(), &mut eff.agent_append.kimi),
        (stated_append.qwen.as_ref(), &mut eff.agent_append.qwen),
        (
            stated_append.qodercli.as_ref(),
            &mut eff.agent_append.qodercli,
        ),
        (stated_append.devin.as_ref(), &mut eff.agent_append.devin),
        (stated_append.gemini.as_ref(), &mut eff.agent_append.gemini),
        (stated_append.amp.as_ref(), &mut eff.agent_append.amp),
        (
            stated_append.mastracode.as_ref(),
            &mut eff.agent_append.mastracode,
        ),
    ] {
        if let Some(v) = stated {
            *resolved = v.clone();
        }
    }
    // The border colors are file-only `[client]` options, like every
    // other client display option.
    if let Some(v) = file.client.border_active_color.as_ref() {
        eff.border_active_color = v.clone();
    }
    if let Some(v) = file.client.border_color.as_ref() {
        eff.border_color = v.clone();
    }
    if file.client.git_status == Some(false) {
        eff.git_status = false;
    }
    if let Some(v) = file.client.git_poll_secs {
        eff.git_poll_secs = if v == 0 { 0 } else { v.clamp(3, 600) };
    }
    eff
}

/// Environment override for the config file location (`PAR_MUX_CONFIG`),
/// else `<config dir>/par-mux/config.toml` via `dirs::config_dir()`.
/// `None` when the platform has no config dir and no override is set.
pub fn config_file_path() -> Option<PathBuf> {
    // `var_os` reads are allowed (QA-196 targets test WRITES); this is
    // the feature's one production env read.
    if let Some(path) = std::env::var_os("PAR_MUX_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    dirs::config_dir().map(|dir| dir.join("par-mux").join("config.toml"))
}

/// Read and parse the config file at `path`. `Ok(None)` = no file
/// (defaults apply); `Err` = a file exists but does not parse — callers
/// surface that rather than silently ignoring the user's file.
pub fn load_file(path: &Path) -> Result<Option<ConfigFile>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|err| format!("{}: TOML parse failed: {err}", path.display()))
}

/// Load the canonical config file (or `Ok(None)` when absent/mislocated),
/// logging a warning and falling back to defaults when it exists but does
/// not parse — unreadable config must not brick daemon startup, the same
/// rule the state file follows (D3.2).
pub fn load_canonical() -> ConfigFile {
    let Some(path) = config_file_path() else {
        return ConfigFile::default();
    };
    match load_file(&path) {
        Ok(file) => file.unwrap_or_default(),
        Err(err) => {
            log::warn!("par-mux: ignoring config: {err}");
            ConfigFile::default()
        }
    }
}

/// A reload's honest variant: absent stays `Ok(None)`-shaped (defaults /
/// current values apply), but a PRESENT file that does not parse is an
/// `Err` — a user editing their config mid-attach must see the mistake
/// on the status row, not have it silently ignored.
pub fn load_canonical_checked() -> Result<ConfigFile, String> {
    let Some(path) = config_file_path() else {
        return Ok(ConfigFile::default());
    };
    Ok(load_file(&path)?.unwrap_or_default())
}

/// Map a resolved `[daemon] socket` value to a socket path: an absolute
/// path stands as-is; anything else (including `default` and empty) names
/// a default socket path.
#[must_use]
pub fn socket_value_to_path(value: &str) -> PathBuf {
    if value.is_empty() {
        return crate::mux::default_socket_path("default");
    }
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::mux::default_socket_path(value)
    }
}

/// Render `eff` as config-file TOML: every setting spelled, with a short
/// header. Round-trips through [`load_file`].
#[must_use]
pub fn render(eff: &EffectiveConfig) -> String {
    // The display defaults come from the one canonical chord set, so the
    // generated file cannot drift from what the client applies (ARC-131).
    let defaults = Chords::with_defaults();
    let file = ConfigFile {
        client: ClientSection {
            prefix: Some(eff.prefix.clone()),
            mode: Some(eff.mode.clone()),
            reload: Some(eff.reload.clone()),
            git_poll_secs: Some(eff.git_poll_secs),
            agents_sort: Some(defaults.agents_sort.clone()),
            split_right: None,
            split_down: None,
            kill_pane: None,
            break_pane: None,
            join_pane: None,
            palette: None,
            new_window: None,
            resize: None,
            resize_step: None,
            swap_prev: None,
            move_window_left: None,
            move_window_right: None,
            tab_strip_left: None,
            tab_strip_right: None,
            swap_next: None,
            workspace_next: None,
            workspace_prev: None,
            help: None,
            picker: Some(char::from(defaults.management.picker).to_string()),
            zoom: None,
            rename_window: None,
            rename_pane: None,
            border_cycle: None,
            label_toggle: None,
            workspace_picker: None,
            sidebar: None,
            status_bar: None,
            respawn: None,
            notify_jump: None,
            sidebar_width: Some(defaults.sidebar_width),
            sidebar_on_launch: Some(defaults.sidebar_on_launch),
            agent_rows: Some(defaults.agent_rows.clone()),
            notify_delivery: Some(defaults.notify_delivery.clone()),
            notify_states: Some(defaults.notify_states.clone()),
            sound: Some(defaults.sound.enabled),
            sound_player: Some(defaults.sound.player.clone()),
            sound_done: Some(defaults.sound.done.clone()),
            sound_blocked: Some(defaults.sound.blocked.clone()),
            sound_mute: Some(defaults.sound.mute.clone()),
            border_lines: Some(defaults.border_lines.clone()),
            pane_borders: None,
            show_label_in_border: Some(defaults.show_label_in_border),
            pane_gaps: Some(defaults.pane_gaps),
            scrollbar_gutter: Some(defaults.scrollbar_gutter),
            drag_cursor_shape: Some(defaults.drag_cursor_shape),
            kitty_graphics: Some(defaults.kitty_graphics),
            session_picker: Some(defaults.session_picker),
            git_status: Some(defaults.git_status),
            border_active_color: Some(eff.border_active_color.clone()),
            border_color: Some(eff.border_color.clone()),
            theme: Some(defaults.theme.clone()),
            theme_dark: Some(defaults.theme_dark.clone()),
            theme_light: Some(defaults.theme_light.clone()),
            popup: None,
        },
        daemon: DaemonSection {
            auto_update: eff.auto_update,
            update_interval: Some(eff.update_interval.clone()),
            socket: Some(eff.socket.clone()),
            state_dir: Some(eff.state_dir.clone()),
            pane_endpoints: Some(eff.pane_endpoints),
            expose_control_socket: Some(eff.expose_control_socket),
            remain_on_exit: Some(eff.remain_on_exit),
            exit_empty: Some(eff.exit_empty),
            split_start_dir: Some(eff.split_start_dir.clone()),
            agent_append: AgentAppendFile {
                claude: Some(eff.agent_append.claude.clone()),
                codex: Some(eff.agent_append.codex.clone()),
                grok: Some(eff.agent_append.grok.clone()),
                muse: Some(eff.agent_append.muse.clone()),
                droid: Some(eff.agent_append.droid.clone()),
                copilot: Some(eff.agent_append.copilot.clone()),
                cursor: Some(eff.agent_append.cursor.clone()),
                kimi: Some(eff.agent_append.kimi.clone()),
                qwen: Some(eff.agent_append.qwen.clone()),
                qodercli: Some(eff.agent_append.qodercli.clone()),
                devin: Some(eff.agent_append.devin.clone()),
                gemini: Some(eff.agent_append.gemini.clone()),
                amp: Some(eff.agent_append.amp.clone()),
                mastracode: Some(eff.agent_append.mastracode.clone()),
            },
        },
        machines: Default::default(),
    };
    let mut out = String::from(
        "# par-mux configuration.\n\
         # Precedence: CLI flags > environment > this file > built-in defaults.\n\
         # Regenerate with: par-mux --gen-config [--force]\n",
    );
    let body = toml::to_string_pretty(&file).unwrap_or_default();
    out.push_str(&body);
    out
}

/// Write `eff` as the config file at `path`, refusing to overwrite an
/// existing file unless `force`. Returns an error message on refusal or
/// I/O failure.
pub fn write_file(path: &Path, eff: &EffectiveConfig, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to overwrite",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    std::fs::write(path, render(eff)).map_err(|err| format!("{}: {err}", path.display()))
}

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
    /// The reserved right-edge gutter column (config
    /// `scrollbar-gutter`). Default off.
    pub scrollbar_gutter: bool,
    /// The drag cursor shaping (config `drag-cursor-shape`): the
    /// resize pointer (OSC 22) over divider hovers/drags plus the
    /// DECSCUSR steady block during a live drag. Default off.
    pub drag_cursor_shape: bool,
    /// Kitty graphics drawn on a kitty-capable host (config
    /// `kitty-graphics`). Default on.
    pub kitty_graphics: bool,
    /// The target-less multi-session picker (config `session-picker`).
    /// Default off. Read at launch: a reload changes the next attach.
    pub session_picker: bool,
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
            drag_cursor_shape: false,
            kitty_graphics: true,
            session_picker: false,
            git_status: true,
            git_poll_secs: 10,
            border_lines: "herdr".to_string(),
            agents_sort: "grouped".to_string(),
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

/// The window/pane management chords: the keys matched after the prefix
/// that act on the focused pane/window. Each defaults to its tmux
/// spelling (`%`, `"`, `x`, `c`); the resize mode's entry key defaults to
/// `R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Management {
    /// Split the focused pane right — `split-window -t <pane> -h`.
    pub split_right: u8,
    /// Split the focused pane down — `split-window -t <pane>`.
    pub split_down: u8,
    /// Kill the focused pane — `kill-pane -t <pane>`.
    pub kill_pane: u8,
    /// Break the focused pane out into a new window of its session —
    /// `break-pane -s <pane>` — and land on the fresh window.
    pub break_pane: u8,
    /// Open the window picker in join mode: the picked window receives
    /// the focused pane (`join-pane -s <pane> -t <pane>`).
    pub join_pane: u8,
    /// Open the command palette: every management action (and
    /// settings/keybinds/reload/detach) as one filterable list.
    pub palette: u8,
    /// New window in the focused pane's session — `new-window -t <session>`.
    pub new_window: u8,
    /// Enter the sticky resize mode — arrows adjust the focused pane's
    /// edges by `Chords::resize_step` per press until Enter/Escape/`q`.
    pub resize: u8,
    /// Swap the focused pane with the previous pane in layout order —
    /// `swap-pane -s <focused> -t <prev>`.
    pub swap_prev: u8,
    /// Swap the focused pane with the next pane in layout order —
    /// `swap-pane -s <focused> -t <next>`.
    pub swap_next: u8,
    /// Move the shown window one tab left in its session's order —
    /// `swap-window -s <shown> -t <prev>` (tmux's `<`).
    pub move_window_left: u8,
    /// Move the shown window one tab right in its session's order —
    /// `swap-window -s <shown> -t <next>` (tmux's `>`).
    pub move_window_right: u8,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// left — toward the earlier windows — without switching (render
    /// mode).
    pub tab_strip_left: u8,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// right — toward the later windows — without switching (render
    /// mode).
    pub tab_strip_right: u8,
    /// Select the next workspace in id order — `select-workspace -t +N`
    /// — and land the view on the workspace's session. Prefix `W` by
    /// default (prefix `w` is reserved for the workspace picker).
    pub workspace_next: u8,
    /// Select the previous workspace in id order —
    /// `select-workspace -t +N` — and land the view on the workspace's
    /// session. Prefix `C-w` by default.
    pub workspace_prev: u8,
    /// Open the bindings help panel — every chord at its effective
    /// binding, from the live config.
    pub help: u8,
    /// Open the session/window picker modal — sessions with their
    /// windows nested, the current one emphasized, keyboard and mouse
    /// navigable.
    pub picker: u8,
    /// Toggle the focused pane between its layout rect and the full
    /// window — tmux's zoom.
    pub zoom: u8,
    /// Open the rename prompt for the shown window (its `name`).
    pub rename_window: u8,
    /// Open the rename prompt for the focused pane's user title.
    pub rename_pane: u8,
    /// Cycle the divider/border glyph set.
    pub border_cycle: u8,
    /// Toggle pane titles embedded in the pane borders (render mode).
    pub label_toggle: u8,
    /// Open the workspace picker modal — the session/window picker's
    /// workspace sibling: the daemon's workspace roster, the current one
    /// marked, keyboard and mouse navigable, Enter lands on it.
    pub workspace_picker: u8,
    /// Toggle the workspace side panel (render mode): a left strip with
    /// the workspace list, click-to-land; more sections will follow.
    pub sidebar: u8,
    /// Toggle the bottom status bar (render mode): shown by default;
    /// hiding it hands the row back to the pane grid.
    pub status_bar: u8,
    /// Respawn the focused pane when it is held dead — `respawn-pane -t
    /// <pane>`. A live pane is left alone.
    pub respawn: u8,
    /// Focus the pane behind the most recent agent notification this
    /// client delivered (herdr's `open_notification_target`; render mode).
    pub notify_jump: u8,
}

impl Default for Management {
    fn default() -> Self {
        Self {
            split_right: b'%',
            split_down: b'"',
            kill_pane: b'x',
            break_pane: b'!',
            join_pane: b'J',
            palette: b':',
            new_window: b'c',
            resize: b'R',
            swap_prev: b'{',
            swap_next: b'}',
            move_window_left: b'<',
            move_window_right: b'>',
            tab_strip_left: b'H',
            tab_strip_right: b'L',
            workspace_next: b'W',
            workspace_prev: 0x17, // C-w
            help: b'?',
            picker: b'w',
            zoom: b'z',
            rename_window: b',',
            rename_pane: b'$',
            border_cycle: b'B',
            label_toggle: b'l',
            workspace_picker: b'g',
            sidebar: b's',
            status_bar: b'S',
            respawn: b'r',
            notify_jump: b'j',
        }
    }
}

/// One after-prefix management key: a single token parsed with the shared
/// prefix parser. A management chord is ONE key matched by byte after the
/// prefix (the reload chord's multi-token spelling exists for
/// documentation only; a management chord keeps the single-key shape).
/// Public for the settings dialog's chord validation, which must accept
/// exactly what a reload would.
pub fn management_key(chord: &str, name: &str) -> Result<u8, String> {
    let token = chord
        .split_whitespace()
        .last()
        .ok_or_else(|| format!("[client] {name} chord is empty"))?;
    for token in chord.split_whitespace() {
        if parse_prefix(token).is_none() {
            return Err(format!(
                "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b or a single key)"
            ));
        }
    }
    parse_prefix(token).ok_or_else(|| {
        format!(
            "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b or a single key)"
        )
    })
}

/// The client-side half of a reload, PURE over the parsed file: re-derive
/// the prefix, reload key, and management chords from `file`. `current` is
/// the fallback for settings the (possibly partial) file does not name — a
/// reload only ever moves a chord when the file actually changed it.
/// Errors on a malformed chord (the caller surfaces it on the status row).
pub fn reload_client_chords(file: &ConfigFile, current: &Chords) -> Result<Chords, String> {
    let prefix = match file.client.prefix.as_deref() {
        Some(spec) => parse_prefix(spec).ok_or_else(|| {
            format!(
                "[client] prefix {spec:?} does not parse (expected the tmux spelling, e.g. C-b)"
            )
        })?,
        None => current.prefix,
    };
    let reload = match file.client.reload.as_deref() {
        Some(chord) => reload_chord_key(chord)?,
        None => current.reload,
    };
    // Each management chord: the file's spelling when present (an error
    // names the chord), else the current binding.
    macro_rules! chord {
        ($field:ident, $label:literal) => {
            match file.client.$field.as_deref() {
                Some(chord) => management_key(chord, $label)?,
                None => current.management.$field,
            }
        };
    }
    let management = Management {
        split_right: chord!(split_right, "split-right"),
        split_down: chord!(split_down, "split-down"),
        kill_pane: chord!(kill_pane, "kill-pane"),
        break_pane: chord!(break_pane, "break-pane"),
        join_pane: chord!(join_pane, "join-pane"),
        palette: chord!(palette, "palette"),
        new_window: chord!(new_window, "new-window"),
        resize: chord!(resize, "resize"),
        swap_prev: chord!(swap_prev, "swap-prev"),
        swap_next: chord!(swap_next, "swap-next"),
        move_window_left: chord!(move_window_left, "move-window-left"),
        move_window_right: chord!(move_window_right, "move-window-right"),
        tab_strip_left: chord!(tab_strip_left, "tab-strip-left"),
        tab_strip_right: chord!(tab_strip_right, "tab-strip-right"),
        workspace_next: chord!(workspace_next, "workspace-next"),
        workspace_prev: chord!(workspace_prev, "workspace-prev"),
        help: chord!(help, "help"),
        picker: chord!(picker, "picker"),
        zoom: chord!(zoom, "zoom"),
        rename_window: chord!(rename_window, "rename-window"),
        rename_pane: chord!(rename_pane, "rename-pane"),
        border_cycle: chord!(border_cycle, "border-cycle"),
        label_toggle: chord!(label_toggle, "label-toggle"),
        workspace_picker: chord!(workspace_picker, "workspace-picker"),
        sidebar: chord!(sidebar, "sidebar"),
        status_bar: chord!(status_bar, "status-bar"),
        respawn: chord!(respawn, "respawn"),
        notify_jump: chord!(notify_jump, "notify-jump"),
    };
    // A step the file names floors at 1 — a zero/negative step would make
    // every resize a no-op by construction.
    let resize_step = file
        .client
        .resize_step
        .map(|step| step.max(1))
        .unwrap_or(current.resize_step);
    let sidebar_width = file
        .client
        .sidebar_width
        .map(|width| width.clamp(6, 60))
        .unwrap_or(current.sidebar_width);
    let sidebar_on_launch = file
        .client
        .sidebar_on_launch
        .unwrap_or(current.sidebar_on_launch);
    // Display options keep their spelling: an unknown token or value is
    // the renderer's warn-and-default (the border-lines rule, and
    // [`display_option_warnings`]), never a reload error. An empty
    // agent-rows table falls back to the default entry.
    let agent_rows = match file.client.agent_rows.as_ref() {
        Some(rows) => {
            let rows: Vec<Vec<String>> = rows.iter().filter(|r| !r.is_empty()).cloned().collect();
            if rows.is_empty() {
                default_agent_rows()
            } else {
                rows
            }
        }
        None => current.agent_rows.clone(),
    };
    let notify_delivery = file
        .client
        .notify_delivery
        .clone()
        .unwrap_or_else(|| current.notify_delivery.clone());
    let notify_states = file
        .client
        .notify_states
        .clone()
        .unwrap_or_else(|| current.notify_states.clone());
    let client = &file.client;
    let sound = SoundSettings {
        enabled: client.sound.unwrap_or(current.sound.enabled),
        player: client
            .sound_player
            .clone()
            .unwrap_or_else(|| current.sound.player.clone()),
        done: client
            .sound_done
            .clone()
            .unwrap_or_else(|| current.sound.done.clone()),
        blocked: client
            .sound_blocked
            .clone()
            .unwrap_or_else(|| current.sound.blocked.clone()),
        mute: client
            .sound_mute
            .clone()
            .unwrap_or_else(|| current.sound.mute.clone()),
    };
    // Display options: default OFF — absent keys preserve the shared-
    // divider, label-free look exactly.
    let pane_borders = file.client.pane_borders.unwrap_or(current.pane_borders);
    let show_label_in_border = file
        .client
        .show_label_in_border
        .unwrap_or(current.show_label_in_border);
    // Herdr-parity display options: default OFF/0 — absent keys preserve
    // today's edge-to-edge, gutter-free, drag-inert look exactly.
    let pane_gaps = file.client.pane_gaps.unwrap_or(current.pane_gaps);
    let scrollbar_gutter = file
        .client
        .scrollbar_gutter
        .unwrap_or(current.scrollbar_gutter);
    let drag_cursor_shape = file
        .client
        .drag_cursor_shape
        .unwrap_or(current.drag_cursor_shape);
    let kitty_graphics = file.client.kitty_graphics.unwrap_or(current.kitty_graphics);
    let session_picker = file.client.session_picker.unwrap_or(current.session_picker);
    let git_status = file.client.git_status.unwrap_or(current.git_status);
    // The poll interval clamps instead of erroring: a bad number is the
    // file philosophy's warn-and-default, and 0 would spin the poll.
    let git_poll_secs = file
        .client
        .git_poll_secs
        .map(|v| if v == 0 { 0 } else { v.clamp(3, 600) })
        .unwrap_or(current.git_poll_secs);
    // The border line style rides as its spelling; an unknown value is
    // the renderer's warn-and-default (the file philosophy), not a
    // reload error.
    let border_lines = file
        .client
        .border_lines
        .clone()
        .unwrap_or_else(|| current.border_lines.clone());
    // The roster's ordering word: an unknown spelling keeps the current
    // (the file philosophy's warn-and-default; the header toggle and the
    // settings row are the only writers, so divergence is not a user
    // hazard).
    let agents_sort = match file.client.agents_sort.as_deref() {
        Some("priority") => "priority".to_string(),
        Some("grouped") => "grouped".to_string(),
        _ => current.agents_sort.clone(),
    };
    let theme = client
        .theme
        .clone()
        .unwrap_or_else(|| current.theme.clone());
    let theme_dark = client
        .theme_dark
        .clone()
        .unwrap_or_else(|| current.theme_dark.clone());
    let theme_light = client
        .theme_light
        .clone()
        .unwrap_or_else(|| current.theme_light.clone());
    let popups = match client.popup.as_deref() {
        Some(entries) => crate::mux::popup::validate(entries)?,
        None => current.popups.clone(),
    };
    Ok(Chords {
        prefix,
        reload,
        management,
        resize_step,
        pane_borders,
        show_label_in_border,
        pane_gaps,
        scrollbar_gutter,
        drag_cursor_shape,
        kitty_graphics,
        session_picker,
        git_status,
        git_poll_secs,
        border_lines,
        agents_sort,
        sidebar_width,
        sidebar_on_launch,
        agent_rows,
        notify_delivery,
        notify_states,
        sound,
        theme,
        theme_dark,
        theme_light,
        popups,
    })
}

/// The supported `agent-start` kinds in table order: the bundled scrape
/// patterns plus the hook-only mastracode arm. The reload report emits a
/// line per kind and `[daemon.agent-append]` carries a key per kind.
pub const AGENT_APPEND_KINDS: [&str; 14] = [
    "claude",
    "codex",
    "grok",
    "muse",
    "droid",
    "copilot",
    "cursor",
    "kimi",
    "qwen",
    "qodercli",
    "devin",
    "gemini",
    "amp",
    "mastracode",
];

/// The daemon-side `reload-config` report, over the re-read file and the
/// applied copy: one `unchanged:`/`restart-required:`/`applied:` line per
/// `[daemon]` setting. The file speaks only for settings it states — a
/// setting absent from the file is `unchanged`, which is what keeps a
/// daemon started with one-shot `--socket`/`--state-dir` flags quiet (a
/// flag is an override the file cannot express, so it never reads back
/// as a change).
///
/// `daemon.remain-on-exit`, `daemon.exit-empty`, `daemon.split-start-dir`,
/// and every `daemon.agent-append.<kind>` are the LIVE settings: a stated
/// difference is applied to `applied` and reported `applied:`; every
/// other stated difference is `restart-required:` — the socket and the
/// persist path are fixed at bind, and the boot-tier bools (pane
/// endpoints, expose-control-socket) decide how the daemon was built.
pub fn reload_report(applied: &mut EffectiveConfig, file: Option<&ConfigFile>) -> String {
    let Some(file) = file else {
        // Absent file: nothing changed.
        let mut lines: Vec<String> = [
            "unchanged: daemon.socket",
            "unchanged: daemon.state-dir",
            "unchanged: daemon.pane-endpoints",
            "unchanged: daemon.expose-control-socket",
            "unchanged: daemon.remain-on-exit",
            "unchanged: daemon.exit-empty",
            "unchanged: daemon.split-start-dir",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for kind in AGENT_APPEND_KINDS {
            lines.push(format!("unchanged: daemon.agent-append.{kind}"));
        }
        return lines.join("\n");
    };
    let mut lines = Vec::new();
    for (name, stated, current) in [
        (
            "daemon.socket",
            file.daemon.socket.as_deref(),
            applied.socket.as_str(),
        ),
        (
            "daemon.state-dir",
            file.daemon.state_dir.as_deref(),
            applied.state_dir.as_str(),
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => format!("restart-required: {name}"),
            _ => format!("unchanged: {name}"),
        });
    }
    for (name, stated, current) in [
        (
            "daemon.pane-endpoints",
            file.daemon.pane_endpoints,
            applied.pane_endpoints,
        ),
        (
            "daemon.expose-control-socket",
            file.daemon.expose_control_socket,
            applied.expose_control_socket,
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => format!("restart-required: {name}"),
            _ => format!("unchanged: {name}"),
        });
    }
    lines.push(match file.daemon.remain_on_exit {
        Some(v) if v != applied.remain_on_exit => {
            applied.remain_on_exit = v;
            "applied: daemon.remain-on-exit".to_string()
        }
        _ => "unchanged: daemon.remain-on-exit".to_string(),
    });
    lines.push(match file.daemon.exit_empty {
        Some(v) if v != applied.exit_empty => {
            applied.exit_empty = v;
            "applied: daemon.exit-empty".to_string()
        }
        _ => "unchanged: daemon.exit-empty".to_string(),
    });
    lines.push(match file.daemon.split_start_dir.as_deref() {
        Some(v) if v != applied.split_start_dir => {
            applied.split_start_dir = v.to_string();
            "applied: daemon.split-start-dir".to_string()
        }
        _ => "unchanged: daemon.split-start-dir".to_string(),
    });
    let stated_append = &file.daemon.agent_append;
    for (kind, stated, current) in [
        (
            "claude",
            stated_append.claude.as_deref(),
            &mut applied.agent_append.claude,
        ),
        (
            "codex",
            stated_append.codex.as_deref(),
            &mut applied.agent_append.codex,
        ),
        (
            "grok",
            stated_append.grok.as_deref(),
            &mut applied.agent_append.grok,
        ),
        (
            "muse",
            stated_append.muse.as_deref(),
            &mut applied.agent_append.muse,
        ),
        (
            "droid",
            stated_append.droid.as_deref(),
            &mut applied.agent_append.droid,
        ),
        (
            "copilot",
            stated_append.copilot.as_deref(),
            &mut applied.agent_append.copilot,
        ),
        (
            "cursor",
            stated_append.cursor.as_deref(),
            &mut applied.agent_append.cursor,
        ),
        (
            "kimi",
            stated_append.kimi.as_deref(),
            &mut applied.agent_append.kimi,
        ),
        (
            "qwen",
            stated_append.qwen.as_deref(),
            &mut applied.agent_append.qwen,
        ),
        (
            "qodercli",
            stated_append.qodercli.as_deref(),
            &mut applied.agent_append.qodercli,
        ),
        (
            "devin",
            stated_append.devin.as_deref(),
            &mut applied.agent_append.devin,
        ),
        (
            "gemini",
            stated_append.gemini.as_deref(),
            &mut applied.agent_append.gemini,
        ),
        (
            "amp",
            stated_append.amp.as_deref(),
            &mut applied.agent_append.amp,
        ),
        (
            "mastracode",
            stated_append.mastracode.as_deref(),
            &mut applied.agent_append.mastracode,
        ),
    ] {
        lines.push(match stated {
            Some(v) if v != current => {
                *current = v.to_string();
                format!("applied: daemon.agent-append.{kind}")
            }
            _ => format!("unchanged: daemon.agent-append.{kind}"),
        });
    }
    lines.join("\n")
}

/// The daemon's resolved `remain-on-exit` from the canonical config file
/// (file tier only — the setting has no flag or env tier). `false` when
/// the file is absent, unreadable, or silent: the auto-remove default.
/// The restore path reads this — it runs before the server exists to
/// publish an applied copy.
#[must_use]
pub fn daemon_remain_on_exit() -> bool {
    resolve(&load_canonical(), &Overrides::default()).remain_on_exit
}

/// The daemon's resolved `[daemon.agent-append]` from the canonical config
/// file (file tier only — the setting has no flag or env tier). The boot
/// restore path reads this alongside [`daemon_remain_on_exit`]: restore
/// runs before the server exists to publish an applied copy.
#[must_use]
pub fn daemon_agent_append() -> AgentAppend {
    resolve(&load_canonical(), &Overrides::default()).agent_append
}

/// Parse a `#rrggbb` hex color (the border-color config spellings).
/// `None` for anything else — callers keep their built-in default.
#[must_use]
pub fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

/// Parse the tmux prefix spelling (`C-b`, `C-a`, `C-Space`) or a literal
/// single character into its byte. The crate's ONE prefix grammar: the
/// attach client's `--prefix` and every chord in the config file are
/// parsed through this, so the spellings cannot drift apart.
pub fn parse_prefix(spec: &str) -> Option<u8> {
    let lower = spec.to_ascii_lowercase();
    if let Some(key) = lower.strip_prefix("c-") {
        return match key {
            "space" => Some(0x00),
            letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_lowercase() => {
                Some(letter.as_bytes()[0] - b'a' + 1)
            }
            _ => None,
        };
    }
    let bytes = spec.as_bytes();
    if bytes.len() == 1 {
        Some(bytes[0])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    /// `[client] agents-sort` resolves into the client chords (`grouped`
    /// default, `priority` respected, an unknown word keeping the
    /// current), and the generated file spells the setting.
    #[test]
    fn agents_sort_round_trips_through_the_file_and_chords() {
        use super::{reload_client_chords, render, resolve, Chords, ConfigFile, Overrides};
        let mut file = ConfigFile::default();
        file.client.agents_sort = Some("priority".to_string());
        let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("chords");
        assert_eq!(chords.agents_sort, "priority");

        // An unknown word keeps the current value.
        file.client.agents_sort = Some("newest".to_string());
        let chords = reload_client_chords(&file, &chords).expect("chords");
        assert_eq!(chords.agents_sort, "priority");

        // The generated file names the key.
        let eff = resolve(&file, &Overrides::default());
        assert!(render(&eff).contains("agents-sort"));
    }

    use super::*;

    #[test]
    fn automatic_updates_are_opt_in_with_daily_default() {
        let defaults: ConfigFile = toml::from_str("").unwrap();
        let effective = resolve(&defaults, &Overrides::default());
        assert!(!effective.auto_update);
        assert_eq!(effective.update_interval, "daily");
        for interval in ["hourly", "daily", "weekly"] {
            let config: ConfigFile = toml::from_str(&format!(
                "[daemon]\nauto-update = true\nupdate-interval = '{interval}'"
            ))
            .unwrap();
            let effective = resolve(&config, &Overrides::default());
            assert!(effective.auto_update);
            assert_eq!(effective.update_interval, interval);
            let rendered = render(&effective);
            let roundtrip: ConfigFile = toml::from_str(&rendered).unwrap();
            assert_eq!(
                resolve(&roundtrip, &Overrides::default()).update_interval,
                interval
            );
        }
        assert!(toml::from_str::<ConfigFile>("[daemon]\nupdate-interval = 'bad'").is_err());
    }

    #[test]
    fn full_file_parses_every_key() {
        let file: ConfigFile = toml::from_str(
            r#"
[client]
prefix = "C-a"
mode = "render"
reload = "C-a C-r"
[daemon]
socket = "/tmp/par-mux-test.sock"
state-dir = "/tmp/par-mux-state"
pane-endpoints = true
expose-control-socket = true
remain-on-exit = true
"#,
        )
        .expect("parse");
        assert_eq!(file.client.prefix.as_deref(), Some("C-a"));
        assert_eq!(file.client.mode.as_deref(), Some("render"));
        assert_eq!(file.client.reload.as_deref(), Some("C-a C-r"));
        assert_eq!(
            file.daemon.socket.as_deref(),
            Some("/tmp/par-mux-test.sock")
        );
        assert_eq!(file.daemon.pane_endpoints, Some(true));
        assert_eq!(file.daemon.expose_control_socket, Some(true));
        assert_eq!(file.daemon.remain_on_exit, Some(true));
    }

    /// Unknown keys and missing sections are tolerated — the file is
    /// forward-compatible and partially spellable.
    #[test]
    fn partial_and_unknown_keys_parse_to_defaults() {
        let file: ConfigFile =
            toml::from_str("[client]\nprefix = 'C-a'\nsome-future-key = 3").expect("parse");
        assert_eq!(file.client.prefix.as_deref(), Some("C-a"));
        assert_eq!(file.daemon, DaemonSection::default());
        let empty: ConfigFile = toml::from_str("").expect("parse");
        assert_eq!(empty, ConfigFile::default());
    }

    /// The precedence chain: file over built-in defaults, env over file,
    /// flags over env — per setting, with the other tiers silent.
    #[test]
    fn resolution_is_flags_over_env_over_file_over_defaults() {
        // Defaults only.
        assert_eq!(
            resolve(&ConfigFile::default(), &Overrides::default()),
            EffectiveConfig::default()
        );

        // File speaks.
        let file = ConfigFile {
            client: ClientSection {
                prefix: Some("C-a".into()),
                ..Default::default()
            },
            daemon: DaemonSection {
                pane_endpoints: Some(true),
                ..Default::default()
            },
            machines: Default::default(),
        };
        let eff = resolve(&file, &Overrides::default());
        assert_eq!(eff.prefix, "C-a");
        assert!(eff.pane_endpoints);
        assert_eq!(eff.mode, "render", "unset settings keep the default");

        // Env beats file (socket only — the one setting with an env tier).
        let eff = resolve(
            &file,
            &Overrides {
                env_socket: Some("/tmp/from-env.sock".into()),
                ..Default::default()
            },
        );
        assert_eq!(eff.socket, "/tmp/from-env.sock");
        assert_eq!(eff.prefix, "C-a", "env does not speak for other settings");

        // Flags beat env and file.
        let eff = resolve(
            &file,
            &Overrides {
                prefix: Some("C-s".into()),
                socket: Some("/tmp/from-flag.sock".into()),
                pane_endpoints: Some(false),
                env_socket: Some("/tmp/from-env.sock".into()),
                ..Default::default()
            },
        );
        assert_eq!(eff.prefix, "C-s");
        assert_eq!(eff.socket, "/tmp/from-flag.sock");
        assert!(!eff.pane_endpoints, "the flag tier wins over the file");
    }

    /// The attach mode tiers: unspecified = render; a file `passthrough`
    /// beats that default; the `--mode` flag beats the file.
    #[test]
    fn attach_mode_defaults_to_render_and_honors_file_then_flag() {
        assert_eq!(
            resolve(&ConfigFile::default(), &Overrides::default()).mode,
            "render"
        );
        let file = ConfigFile {
            client: ClientSection {
                mode: Some("passthrough".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(resolve(&file, &Overrides::default()).mode, "passthrough");
        let flagged = Overrides {
            mode: Some("render".into()),
            ..Default::default()
        };
        assert_eq!(resolve(&file, &flagged).mode, "render");
    }

    /// An empty env value counts as unset (a pane script expanding an
    /// unset variable produces the empty string), so the file tier speaks.
    #[test]
    fn empty_env_socket_falls_through_to_the_file() {
        let file = ConfigFile {
            daemon: DaemonSection {
                socket: Some("/tmp/from-file.sock".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let eff = resolve(
            &file,
            &Overrides {
                env_socket: Some(String::new()),
                ..Default::default()
            },
        );
        assert_eq!(eff.socket, "/tmp/from-file.sock");
    }

    /// gen-config's output parses back to the same effective settings.
    #[test]
    fn render_round_trips_through_the_parser() {
        let eff = EffectiveConfig {
            auto_update: true,
            update_interval: "weekly".into(),
            prefix: "C-a".into(),
            mode: "render".into(),
            reload: "C-a C-r".into(),
            socket: "/tmp/rt.sock".into(),
            state_dir: "/tmp/rt-state".into(),
            pane_endpoints: true,
            expose_control_socket: true,
            remain_on_exit: true,
            exit_empty: true,
            border_active_color: "#00ff00".into(),
            border_color: "#202020".into(),
            git_status: true,
            git_poll_secs: 30,
            split_start_dir: "previous-pane-dir".into(),
            agent_append: AgentAppend {
                claude: "--continue".into(),
                ..AgentAppend::default()
            },
            config_path: None,
        };
        let file: ConfigFile = toml::from_str(&render(&eff)).expect("round-trip parse");
        assert_eq!(resolve(&file, &Overrides::default()), eff);
    }

    /// remain-on-exit and exit-empty are the LIVE daemon settings: a stated
    /// difference is applied to the applied copy and reported `applied:`;
    /// the same value or an absent key stays `unchanged:`.
    #[test]
    fn reload_applies_remain_on_exit_live() {
        let mut applied = EffectiveConfig::default();
        let file: ConfigFile = toml::from_str("[daemon]\nremain-on-exit = true").unwrap();
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("applied: daemon.remain-on-exit"),
            "a stated difference applies live: {report}"
        );
        assert!(applied.remain_on_exit, "the applied copy moved");
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("unchanged: daemon.remain-on-exit"),
            "the same value is unchanged: {report}"
        );
        let file: ConfigFile = toml::from_str("[daemon]\nremain-on-exit = false").unwrap();
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("applied: daemon.remain-on-exit"),
            "flipping back applies too: {report}"
        );
        assert!(!applied.remain_on_exit, "the applied copy moved back");
    }

    /// exit-empty behaves identically: a stated difference applies live
    /// (`applied:`), the same value stays `unchanged:`.
    #[test]
    fn reload_applies_exit_empty_live() {
        let mut applied = EffectiveConfig::default();
        let file: ConfigFile = toml::from_str("[daemon]\nexit-empty = false").unwrap();
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("applied: daemon.exit-empty"),
            "a stated difference applies live: {report}"
        );
        assert!(!applied.exit_empty, "the applied copy moved");
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("unchanged: daemon.exit-empty"),
            "the same value is unchanged: {report}"
        );
    }

    /// The agent-append keys are LIVE like the other boot-surviving
    /// settings: a stated difference applies to the applied copy and
    /// reports `applied:`; the same value, an omitted key, and the
    /// absent-file report all stay `unchanged:`.
    #[test]
    fn reload_applies_agent_append_live() {
        let mut applied = EffectiveConfig::default();
        let file: ConfigFile =
            toml::from_str("[daemon.agent-append]\nclaude = \"--continue\"\n").unwrap();
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("applied: daemon.agent-append.claude"),
            "a stated difference applies live: {report}"
        );
        assert_eq!(applied.agent_append.claude, "--continue");
        let report = reload_report(&mut applied, Some(&file));
        assert!(
            report.contains("unchanged: daemon.agent-append.claude"),
            "the same value is unchanged: {report}"
        );
        let absent = reload_report(&mut applied, None);
        for kind in AGENT_APPEND_KINDS {
            assert!(
                absent.contains(&format!("unchanged: daemon.agent-append.{kind}")),
                "the absent file keeps every kind unchanged: {report}"
            );
        }
    }

    /// Every supported kind has a table key: the sentinel round-trip
    /// fails for any kind whose key the struct lacks (the parser drops
    /// unknown TOML keys silently). Unknown kinds and the blank default
    /// read empty.
    #[test]
    fn agent_append_covers_every_supported_kind() {
        let mut text = String::from("[daemon.agent-append]\n");
        for kind in AGENT_APPEND_KINDS {
            text.push_str(&format!("{kind} = \"--{kind}-flag\"\n"));
        }
        let eff = resolve(
            &toml::from_str::<ConfigFile>(&text).unwrap(),
            &Overrides::default(),
        );
        for kind in AGENT_APPEND_KINDS {
            let want = format!("--{kind}-flag");
            assert_eq!(eff.agent_append.entry(kind), want, "{kind} keeps its entry");
        }
        assert_eq!(
            eff.agent_append.entry("unknown-agent"),
            "",
            "an unknown kind reads blank"
        );
        assert_eq!(
            EffectiveConfig::default().agent_append.entry("claude"),
            "",
            "the default table is blank"
        );
    }

    /// exit-empty's default is ON — exit-when-empty ships enabled, as
    /// before the knob existed.
    #[test]
    fn exit_empty_defaults_to_on() {
        assert!(EffectiveConfig::default().exit_empty);
        let eff = resolve(&ConfigFile::default(), &Overrides::default());
        assert!(eff.exit_empty);
    }

    /// The product default: remain-on-exit is OFF (auto-remove) — both the
    /// built-in default and a file silent on the key.
    #[test]
    fn remain_on_exit_defaults_to_auto_remove() {
        assert!(!EffectiveConfig::default().remain_on_exit);
        let eff = resolve(&ConfigFile::default(), &Overrides::default());
        assert!(!eff.remain_on_exit);
    }

    /// The default rendering carries every key a user can start from.
    #[test]
    fn default_render_spells_every_setting() {
        let text = render(&EffectiveConfig::default());
        for key in [
            "prefix",
            "mode",
            "reload",
            "socket",
            "state-dir",
            "pane-endpoints",
            "expose-control-socket",
            "remain-on-exit",
            "exit-empty",
            "agent-append",
        ] {
            assert!(text.contains(key), "default render names {key}: {text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn socket_value_maps_names_and_paths() {
        assert_eq!(
            socket_value_to_path("work"),
            crate::mux::default_socket_path("work")
        );
        assert_eq!(
            socket_value_to_path("default"),
            crate::mux::default_socket_path("default")
        );
        assert_eq!(
            socket_value_to_path(""),
            crate::mux::default_socket_path("default")
        );
        assert_eq!(
            socket_value_to_path("/tmp/explicit.sock"),
            PathBuf::from("/tmp/explicit.sock")
        );
    }

    /// The management chord overrides parse through the same prefix
    /// grammar and rebind only the stated keys; a malformed spelling
    /// errors with the key's name in the message.
    /// The herdr-parity display options: absent keys keep the defaults
    /// (all OFF — the byte-identical contract), stated keys resolve, and
    /// gen-config emits all three at their defaults.
    #[test]
    fn session_picker_defaults_off_resolves_and_reload_keeps_it() {
        let defaults = Chords::with_defaults();
        assert!(!defaults.session_picker, "off unless the user opts in");
        let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
        assert!(
            !reload_client_chords(&absent, &defaults)
                .unwrap()
                .session_picker
        );
        let on: ConfigFile = toml::from_str("[client]\nsession-picker = true\n").unwrap();
        let chords = reload_client_chords(&on, &defaults).unwrap();
        assert!(chords.session_picker);
        assert!(
            reload_client_chords(&absent, &chords)
                .unwrap()
                .session_picker,
            "an absent key keeps the value in force"
        );
        let text = render(&EffectiveConfig::default());
        assert!(text.contains("session-picker = false"), "{text}");
    }

    /// The git badge key: default ON (unlike `session-picker`), a stated
    /// key resolves, an absent key keeps the value in force, gen-config
    /// emits it, and the daemon-side `EffectiveConfig` gate follows the
    /// file.
    #[test]
    fn git_status_defaults_on_resolves_and_reload_keeps_it() {
        let defaults = Chords::with_defaults();
        assert!(defaults.git_status, "on by default");
        let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
        assert!(reload_client_chords(&absent, &defaults).unwrap().git_status);
        let off: ConfigFile = toml::from_str("[client]\ngit-status = false\n").unwrap();
        let chords = reload_client_chords(&off, &defaults).unwrap();
        assert!(!chords.git_status);
        assert!(
            !reload_client_chords(&absent, &chords).unwrap().git_status,
            "an absent key keeps the value in force (still the false above)"
        );
        // The daemon-side EffectiveConfig gate: default on, a false key
        // turns it off, and gen-config emits the default.
        let eff = resolve(&ConfigFile::default(), &Overrides::default());
        assert!(eff.git_status);
        let off_file: ConfigFile = toml::from_str("[client]\ngit-status = false\n").unwrap();
        assert!(!resolve(&off_file, &Overrides::default()).git_status);
        let text = render(&EffectiveConfig::default());
        assert!(text.contains("git-status = true"), "{text}");
    }

    /// `git-poll-secs`: default 10, 0 disables, otherwise clamped
    /// 3-600 on resolve and reload,
    /// and gen-config emits the default.
    #[test]
    fn git_poll_secs_defaults_clamps_and_emits() {
        let defaults = Chords::with_defaults();
        assert_eq!(defaults.git_poll_secs, 10);
        let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
        assert_eq!(
            reload_client_chords(&absent, &defaults)
                .unwrap()
                .git_poll_secs,
            10
        );
        let zero: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 0\n").unwrap();
        assert_eq!(
            reload_client_chords(&zero, &defaults)
                .unwrap()
                .git_poll_secs,
            0,
            "0 disables the poll"
        );
        let huge: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 99999\n").unwrap();
        assert_eq!(
            reload_client_chords(&huge, &defaults)
                .unwrap()
                .git_poll_secs,
            600,
            "above the ceiling clamps to 600"
        );
        let one: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 1\n").unwrap();
        assert_eq!(
            reload_client_chords(&one, &defaults).unwrap().git_poll_secs,
            3,
            "1-2 clamps to the 3s floor (never below the 2s git run)"
        );
        let eff = resolve(&ConfigFile::default(), &Overrides::default());
        assert_eq!(eff.git_poll_secs, 10);
        let file: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 45\n").unwrap();
        assert_eq!(resolve(&file, &Overrides::default()).git_poll_secs, 45);
        let off: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 0\n").unwrap();
        assert_eq!(
            resolve(&off, &Overrides::default()).git_poll_secs,
            0,
            "0 disables the poll on the resolve path too"
        );
        let text = render(&EffectiveConfig::default());
        assert!(text.contains("git-poll-secs = 10"), "{text}");
    }

    #[test]
    fn herdr_parity_display_options_resolve_and_gen_config_emits() {
        let defaults = Chords::with_defaults();
        let empty: ConfigFile = toml::from_str("[client]\n").unwrap();
        let kept = reload_client_chords(&empty, &defaults).unwrap();
        assert_eq!(
            (
                kept.pane_gaps,
                kept.scrollbar_gutter,
                kept.drag_cursor_shape
            ),
            (0, false, false),
            "absent keys keep the defaults off"
        );
        let file: ConfigFile = toml::from_str(
            "[client]\npane-gaps = 2\nscrollbar-gutter = true\ndrag-cursor-shape = true\n",
        )
        .unwrap();
        let moved = reload_client_chords(&file, &defaults).unwrap();
        assert_eq!(
            (
                moved.pane_gaps,
                moved.scrollbar_gutter,
                moved.drag_cursor_shape
            ),
            (2, true, true),
            "stated keys resolve"
        );
        // gen-config emits all three; the output parses back.
        let text = render(&EffectiveConfig::default());
        let file: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(file.client.pane_gaps, Some(0), "gen-config emits pane-gaps");
        assert_eq!(
            file.client.scrollbar_gutter,
            Some(false),
            "gen-config emits scrollbar-gutter"
        );
        assert_eq!(
            file.client.drag_cursor_shape,
            Some(false),
            "gen-config emits drag-cursor-shape"
        );
    }

    #[test]
    fn management_chords_parse_route_and_error() {
        let defaults = Chords {
            prefix: 0x02,
            reload: 0x12,
            ..Chords::with_defaults()
        };
        // Defaults.
        let file: ConfigFile = toml::from_str("[client]\n").unwrap();
        assert_eq!(
            reload_client_chords(&file, &defaults).unwrap().management,
            Management {
                split_right: b'%',
                split_down: b'"',
                kill_pane: b'x',
                break_pane: b'!',
                join_pane: b'J',
                palette: b':',
                new_window: b'c',
                resize: b'R',
                swap_prev: b'{',
                swap_next: b'}',
                move_window_left: b'<',
                move_window_right: b'>',
                tab_strip_left: b'H',
                tab_strip_right: b'L',
                workspace_next: b'W',
                workspace_prev: 0x17,
                help: b'?',
                picker: b'w',
                zoom: b'z',
                rename_window: b',',
                rename_pane: b'$',
                border_cycle: b'B',
                label_toggle: b'l',
                workspace_picker: b'g',
                sidebar: b's',
                status_bar: b'S',
                respawn: b'r',
                notify_jump: b'j',
            }
        );
        // Full override, tmux spellings and literals alike.
        let file: ConfigFile = toml::from_str(
            "[client]\nsplit-right = \"C-s\"\nsplit-down = \"v\"\nkill-pane = \"C-x\"\nnew-window = \"W\"\n",
        )
        .unwrap();
        let m = reload_client_chords(&file, &defaults).unwrap().management;
        assert_eq!(m.split_right, 0x13);
        assert_eq!(m.split_down, b'v');
        assert_eq!(m.kill_pane, 0x18);
        assert_eq!(m.new_window, b'W');
        // A partial override keeps the untouched keys.
        let partial: ConfigFile = toml::from_str("[client]\nnew-window = \"n\"\n").unwrap();
        let m = reload_client_chords(&partial, &defaults)
            .unwrap()
            .management;
        assert_eq!(m.split_right, b'%', "unstated keeps the current/default");
        assert_eq!(m.new_window, b'n');
        // The workspace chords parse through the same grammar; unstated
        // keeps the defaults (W / C-w).
        let file: ConfigFile =
            toml::from_str("[client]\nworkspace-next = \"C-n\"\nworkspace-prev = \"C-p\"\n")
                .unwrap();
        let m = reload_client_chords(&file, &defaults).unwrap().management;
        assert_eq!(m.workspace_next, 0x0e, "C-n overrides the W default");
        assert_eq!(m.workspace_prev, 0x10, "C-p overrides the C-w default");
        let empty: ConfigFile = toml::from_str("[client]\n").unwrap();
        let m = reload_client_chords(&empty, &defaults).unwrap().management;
        assert_eq!(m.workspace_next, b'W');
        assert_eq!(m.workspace_prev, 0x17, "C-w");
        // Malformed spellings error, naming the key.
        for (key, spec) in [
            ("split-right", "C-"),
            ("split-down", "C-1"),
            ("kill-pane", "xy"),
            ("new-window", "C-bb"),
        ] {
            let body = format!("[client]\n{key} = \"{spec}\"\n");
            let file: ConfigFile = toml::from_str(&body).unwrap();
            let err = reload_client_chords(&file, &defaults).unwrap_err();
            assert!(err.contains(key), "the error names the key: {err}");
        }
    }

    /// The tab-strip scroll chords: default H / L, rebindable with the
    /// tmux spelling, a partial override keeping the sibling's default.
    #[test]
    fn tab_strip_chords_parse_and_rebind() {
        use super::{reload_client_chords, Chords, ConfigFile};
        let defaults = Chords::with_defaults();
        // Defaults.
        let file: ConfigFile = toml::from_str("[client]\n").unwrap();
        let m = reload_client_chords(&file, &defaults).unwrap().management;
        assert_eq!(m.tab_strip_left, b'H');
        assert_eq!(m.tab_strip_right, b'L');
        // A rebind parses with the tmux spelling.
        let file: ConfigFile =
            toml::from_str("[client]\ntab-strip-left = \"C-j\"\ntab-strip-right = \"C-l\"\n")
                .unwrap();
        let m = reload_client_chords(&file, &defaults).unwrap().management;
        assert_eq!(m.tab_strip_left, 0x0a, "C-j");
        assert_eq!(m.tab_strip_right, 0x0c, "C-l");
        // A partial override keeps the sibling's default.
        let file: ConfigFile = toml::from_str("[client]\ntab-strip-left = \"h\"\n").unwrap();
        let m = reload_client_chords(&file, &defaults).unwrap().management;
        assert_eq!(m.tab_strip_left, b'h');
        assert_eq!(m.tab_strip_right, b'L', "unstated keeps the default");
        // Malformed spellings error, naming the key.
        let file: ConfigFile = toml::from_str("[client]\ntab-strip-left = \"C-]\"\n").unwrap();
        let err = reload_client_chords(&file, &defaults).unwrap_err();
        assert!(
            err.contains("tab-strip-left"),
            "the error names the key: {err}"
        );
    }

    /// The reload chord resolves through the shared prefix parser: the
    /// last token is the key; a malformed token anywhere is an error.
    #[test]
    fn reload_chord_key_is_the_last_token_and_validates_all_tokens() {
        assert_eq!(reload_chord_key("C-b C-r").unwrap(), 0x12);
        assert_eq!(reload_chord_key("C-b C-a").unwrap(), 0x01);
        // A single token is a chord of just that key.
        assert_eq!(reload_chord_key("C-r").unwrap(), 0x12);
        // The literal spellings the prefix parser accepts.
        assert_eq!(reload_chord_key("C-b C-Space").unwrap(), 0x00);
        assert!(reload_chord_key("").is_err());
        assert!(reload_chord_key("C-b C-1").is_err());
        assert!(
            reload_chord_key("C- C-r").is_err(),
            "a bad head is an error too"
        );
    }

    /// The resize affordances: the resize chord rebinds like the other
    /// management keys, and the step floors at 1 — a zero step would make
    /// every resize a no-op by construction.
    #[test]
    fn resize_chord_and_step_parse_with_the_step_floored() {
        let defaults = Chords::with_defaults();
        // Unstated keys keep their current values.
        let file: ConfigFile = toml::from_str("[client]\n").unwrap();
        let chords = reload_client_chords(&file, &defaults).unwrap();
        assert_eq!(chords.management.resize, b'R');
        assert_eq!(chords.resize_step, 1);
        // Full override: the chord through the shared grammar, the step
        // through the file tier.
        let file: ConfigFile =
            toml::from_str("[client]\nresize = \"C-z\"\nresize-step = 3\n").unwrap();
        let chords = reload_client_chords(&file, &defaults).unwrap();
        assert_eq!(chords.management.resize, 0x1a);
        assert_eq!(chords.resize_step, 3);
        // Zero and unstated-file floors: 0 clamps to 1.
        let file: ConfigFile = toml::from_str("[client]\nresize-step = 0\n").unwrap();
        let chords = reload_client_chords(&file, &defaults).unwrap();
        assert_eq!(chords.resize_step, 1, "a zero step floors at 1");
        // A malformed resize chord errors naming the key.
        let file: ConfigFile = toml::from_str("[client]\nresize = \"C-1\"\n").unwrap();
        let err = reload_client_chords(&file, &defaults).unwrap_err();
        assert!(err.contains("resize"), "the error names the key: {err}");
    }

    /// load_file: absent = None, present parses, garbage errors with the
    /// path in the message.
    #[test]
    fn load_file_classifies_absent_parsed_and_broken() {
        let dir = tempfile::tempdir().expect("tempdir");
        let absent = dir.path().join("absent.toml");
        assert_eq!(load_file(&absent), Ok(None));

        let good = dir.path().join("good.toml");
        std::fs::write(&good, "[client]\nprefix = 'C-a'").unwrap();
        let parsed = load_file(&good).unwrap().expect("some file");
        assert_eq!(parsed.client.prefix.as_deref(), Some("C-a"));

        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "[client").unwrap();
        let err = load_file(&bad).unwrap_err();
        assert!(err.contains("bad.toml"), "the error names the file: {err}");
    }

    /// write_file refuses to overwrite without force, and force writes.
    #[test]
    fn write_file_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sub").join("config.toml");
        write_file(&path, &EffectiveConfig::default(), false).expect("first write");
        let first = std::fs::read_to_string(&path).unwrap();

        let changed = EffectiveConfig {
            prefix: "C-z".into(),
            ..EffectiveConfig::default()
        };
        let err = write_file(&path, &changed, false).unwrap_err();
        assert!(
            err.contains("--force"),
            "refusal names the escape hatch: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            first,
            "the existing file survived the refusal"
        );

        write_file(&path, &changed, true).expect("forced rewrite");
        assert!(std::fs::read_to_string(&path).unwrap().contains("C-z"));
    }

    /// Hostile files never panic the loader: wrong-typed values, unknown
    /// keys, a 1MB string value, invalid UTF-8 (rejected, not lossily
    /// decoded), duplicate keys, and a directory-shaped path all come back
    /// as classified errors or clean parses per the documented contract.
    #[test]
    fn load_file_survives_hostile_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = |name: &str| dir.path().join(name);

        // Wrong-typed values: a string field given an integer / bool.
        std::fs::write(path("wrong_type.toml"), "[client]\nprefix = 5\n").expect("write");
        assert!(
            load_file(&path("wrong_type.toml")).is_err(),
            "int for a string field is a parse error"
        );
        std::fs::write(
            path("wrong_type2.toml"),
            "[daemon]\npane-endpoints = 'yes'\n",
        )
        .expect("write");
        assert!(
            load_file(&path("wrong_type2.toml")).is_err(),
            "string for a bool field is a parse error"
        );

        // Unknown keys are forward-compat ignored and resolve to defaults.
        std::fs::write(
            path("unknown.toml"),
            "[client]\nbogus = 'x'\n[daemon]\nnope = 1\n",
        )
        .expect("write");
        let file = load_file(&path("unknown.toml"))
            .expect("no io error")
            .expect("parses");
        assert_eq!(
            resolve(&file, &Overrides::default()),
            EffectiveConfig::default()
        );

        // A 1MB string value parses (no size cliff in the reader).
        let big = format!("[client]\nbogus = '{}'\n", "a".repeat(1 << 20));
        std::fs::write(path("big.toml"), big).expect("write");
        assert!(load_file(&path("big.toml")).is_ok());

        // Invalid UTF-8: rejected (read_to_string), never a panic.
        std::fs::write(path("utf8.toml"), b"[client]\nprefix = '\xff\xfe'\n").expect("write");
        assert!(load_file(&path("utf8.toml")).is_err(), "rejected");

        // Duplicate keys are a TOML error.
        std::fs::write(
            path("dup.toml"),
            "[client]\nprefix = 'C-b'\nprefix = 'C-a'\n",
        )
        .expect("write");
        assert!(load_file(&path("dup.toml")).is_err(), "duplicate key");

        // A directory where the file should be: an io error naming the
        // path, not a panic and not Ok(None).
        let err = load_file(dir.path()).expect_err("directory read fails");
        assert!(
            err.contains(dir.path().to_string_lossy().as_ref()),
            "the error names the path: {err}"
        );
    }

    /// gen-config output round-trips: write_file → load_file → resolve
    /// reproduces the effective config it was generated from, for both
    /// the defaults and a mutated prefix.
    #[test]
    fn gen_config_round_trips_through_load_and_resolve() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        write_file(&path, &EffectiveConfig::default(), false).expect("write");
        let loaded = load_file(&path)
            .expect("no io error")
            .expect("written file parses");
        assert_eq!(
            resolve(&loaded, &Overrides::default()),
            EffectiveConfig::default()
        );

        let mutated = EffectiveConfig {
            prefix: "C-a".into(),
            mode: "render".into(),
            ..EffectiveConfig::default()
        };
        let path2 = dir.path().join("mutated.toml");
        write_file(&path2, &mutated, false).expect("write");
        let loaded = load_file(&path2)
            .expect("no io error")
            .expect("written file parses");
        assert_eq!(resolve(&loaded, &Overrides::default()), mutated);
    }

    /// ARC-131: every display default the generated config spells equals
    /// `Chords::with_defaults()`, and the generated file resolves back to
    /// exactly that chord set.
    #[test]
    fn gen_config_display_defaults_equal_with_defaults() {
        let defaults = Chords::with_defaults();
        let text = render(&EffectiveConfig::default());
        let file: ConfigFile = toml::from_str(&text).expect("parses");
        let c = &file.client;
        assert_eq!(c.sidebar_width, Some(defaults.sidebar_width));
        assert_eq!(c.sidebar_on_launch, Some(defaults.sidebar_on_launch));
        assert_eq!(
            c.border_lines.as_deref(),
            Some(defaults.border_lines.as_str())
        );
        assert_eq!(c.show_label_in_border, Some(defaults.show_label_in_border));
        assert_eq!(c.pane_gaps, Some(defaults.pane_gaps));
        assert_eq!(c.scrollbar_gutter, Some(defaults.scrollbar_gutter));
        assert_eq!(c.drag_cursor_shape, Some(defaults.drag_cursor_shape));
        assert_eq!(c.kitty_graphics, Some(defaults.kitty_graphics));
        assert_eq!(c.session_picker, Some(defaults.session_picker));
        assert_eq!(
            reload_client_chords(&file, &Chords::with_defaults()).expect("resolves"),
            defaults
        );
    }

    /// `agent-rows` resolves to the two-line default when absent, follows
    /// an explicit table, drops empty lines, and keeps an unknown token
    /// for the renderer's warning (never a reload error); the generated
    /// config spells the default.
    #[test]
    fn agent_rows_resolve_default_explicit_and_warn_on_unknown_tokens() {
        let resolve_with = |text: &str| {
            let file: ConfigFile = toml::from_str(text).expect("parses");
            reload_client_chords(&file, &Chords::with_defaults()).map(|c| c.agent_rows)
        };
        assert_eq!(resolve_with("").unwrap(), default_agent_rows());
        assert_eq!(
            resolve_with("[client]\nagent-rows = [[\"agent\", \"state\"], [], [\"pane\"]]\n")
                .unwrap(),
            vec![
                vec!["agent".to_string(), "state".to_string()],
                vec!["pane".to_string()]
            ]
        );
        assert_eq!(
            resolve_with("[client]\nagent-rows = []\n").unwrap(),
            default_agent_rows(),
            "an empty table is the default"
        );
        let file: ConfigFile =
            toml::from_str("[client]\nagent-rows = [[\"agent\", \"bogus\"]]\n").unwrap();
        let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("never an error");
        let warnings = display_option_warnings(&chords);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"bogus\""), "{warnings:?}");
        let generated = render(&EffectiveConfig::default());
        let file: ConfigFile = toml::from_str(&generated).expect("parses");
        assert_eq!(file.client.agent_rows, Some(default_agent_rows()));
    }

    /// The notification keys resolve to `terminal` + blocked/done when
    /// absent, follow explicit values (an empty state list notifies
    /// nothing), and warn on unknown spellings without failing.
    #[test]
    fn notify_keys_resolve_defaults_explicit_and_warn_on_unknowns() {
        let resolve_with = |text: &str| {
            let file: ConfigFile = toml::from_str(text).expect("parses");
            reload_client_chords(&file, &Chords::with_defaults())
                .map(|c| (c.notify_delivery, c.notify_states))
        };
        assert_eq!(
            resolve_with("").unwrap(),
            (
                "terminal".to_string(),
                vec!["blocked".to_string(), "done".to_string()]
            )
        );
        assert_eq!(
            resolve_with("[client]\nnotify-delivery = \"system\"\nnotify-states = []\n").unwrap(),
            ("system".to_string(), Vec::new())
        );
        let file: ConfigFile = toml::from_str(
            "[client]\nnotify-delivery = \"toast\"\nnotify-states = [\"finished\", \"done\"]\n",
        )
        .unwrap();
        let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("never an error");
        let warnings = display_option_warnings(&chords);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("\"toast\""), "{warnings:?}");
        assert!(warnings[1].contains("\"finished\""), "{warnings:?}");
        assert!(display_option_warnings(&Chords::with_defaults()).is_empty());
        let generated = render(&EffectiveConfig::default());
        assert!(
            generated.contains("notify-delivery = \"terminal\""),
            "{generated}"
        );
    }

    /// The sound keys resolve to off / OS player / built-ins / no mutes
    /// when absent, follow explicit values, keep the current value for
    /// an absent key on reload, and the generated config spells them.
    #[test]
    fn sound_keys_resolve_defaults_and_explicit_values() {
        let file: ConfigFile = toml::from_str("").unwrap();
        let chords = reload_client_chords(&file, &Chords::with_defaults()).unwrap();
        assert_eq!(chords.sound, SoundSettings::default());
        assert!(!chords.sound.enabled);
        let file: ConfigFile = toml::from_str(
            "[client]\nsound = true\nsound-player = \"mpv --no-video\"\n\
             sound-done = \"sounds/done.mp3\"\nsound-blocked = \"/tmp/b.wav\"\n\
             sound-mute = [\"droid\", \"codex\"]\n",
        )
        .unwrap();
        let chords = reload_client_chords(&file, &Chords::with_defaults()).unwrap();
        assert_eq!(
            chords.sound,
            SoundSettings {
                enabled: true,
                player: "mpv --no-video".to_string(),
                done: "sounds/done.mp3".to_string(),
                blocked: "/tmp/b.wav".to_string(),
                mute: vec!["droid".to_string(), "codex".to_string()],
            }
        );
        // An absent key keeps what is in force.
        let partial: ConfigFile = toml::from_str("[client]\nsound = false\n").unwrap();
        let kept = reload_client_chords(&partial, &chords).unwrap();
        assert!(!kept.sound.enabled);
        assert_eq!(kept.sound.mute, chords.sound.mute);
        let generated = render(&EffectiveConfig::default());
        for key in [
            "sound = false",
            "sound-player = \"\"",
            "sound-done = \"\"",
            "sound-blocked = \"\"",
            "sound-mute = []",
        ] {
            assert!(generated.contains(key), "{key}: {generated}");
        }
    }

    /// `sidebar-on-launch` resolves to true when absent, and follows an
    /// explicit value either way; the generated config spells it.
    #[test]
    fn sidebar_on_launch_resolves_default_true_and_explicit_values() {
        let resolve_with = |text: &str| {
            let file: ConfigFile = toml::from_str(text).expect("parses");
            reload_client_chords(&file, &Chords::with_defaults())
                .expect("resolves")
                .sidebar_on_launch
        };
        assert!(resolve_with(""), "absent = default true");
        assert!(resolve_with("[client]\n"), "absent key = default true");
        assert!(!resolve_with("[client]\nsidebar-on-launch = false\n"));
        assert!(resolve_with("[client]\nsidebar-on-launch = true\n"));
        assert!(
            render(&EffectiveConfig::default()).contains("sidebar-on-launch = true"),
            "--gen-config documents the key"
        );
    }
}
