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
}

/// `[client]`: what the attach client reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClientSection {
    /// The detach prefix, tmux spelling (e.g. `C-b`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Default attach mode: `passthrough` (the built-in default) or
    /// `render`. Startup-only — a running attach does not switch pipelines.
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
    /// with `pane-borders`). Default off.
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
    /// The focused pane's border/divider highlight, `#rrggbb` hex. Empty
    /// = the built-in accent (bright cyan).
    #[serde(
        default,
        rename = "border-active-color",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_active_color: Option<String>,
    /// Unfocused border/divider color, `#rrggbb` hex. Empty = the
    /// built-in dim look.
    #[serde(
        default,
        rename = "border-color",
        skip_serializing_if = "Option::is_none"
    )]
    pub border_color: Option<String>,
}

/// `[daemon]`: what the daemon reads at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DaemonSection {
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
    /// Exit the daemon when it holds zero sessions and zero clients,
    /// after a short grace (tmux's `exit-empty`). `true` (the default)
    /// matches the built-in behavior; `false` keeps a persisting daemon
    /// alive however long it sits empty.
    #[serde(
        default,
        rename = "exit-empty",
        skip_serializing_if = "Option::is_none"
    )]
    pub exit_empty: Option<bool>,
}

/// The fully resolved settings after flags > env > file > defaults.
/// `gen-config` renders exactly this; `reload-config` compares a re-read
/// file's daemon section against the daemon's startup copy of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
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
    /// Exit the daemon when it holds zero sessions and zero clients,
    /// after a short grace. The second live daemon setting: the accept
    /// loop re-reads the applied copy every tick, so `reload-config`
    /// applies a changed value without a restart.
    pub exit_empty: bool,
    /// The focused pane's border/divider highlight, `#rrggbb` hex;
    /// empty = the built-in accent (bright cyan).
    pub border_active_color: String,
    /// Unfocused border/divider color, `#rrggbb` hex; empty = the
    /// built-in dim look.
    pub border_color: String,
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
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
    /// `--border-active-color` (attach), the flag tier.
    pub border_active_color: Option<String>,
    /// `--border-color` (attach), the flag tier.
    pub border_color: Option<String>,
}

/// Merge the tiers: each setting takes the first tier that speaks, ending
/// at [`EffectiveConfig::default`].
#[must_use]
pub fn resolve(file: &ConfigFile, o: &Overrides) -> EffectiveConfig {
    let mut eff = EffectiveConfig::default();
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
    if let Some(v) = o
        .border_active_color
        .as_ref()
        .or(file.client.border_active_color.as_ref())
    {
        eff.border_active_color = v.clone();
    }
    if let Some(v) = o
        .border_color
        .as_ref()
        .or(file.client.border_color.as_ref())
    {
        eff.border_color = v.clone();
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
    let file = ConfigFile {
        client: ClientSection {
            prefix: Some(eff.prefix.clone()),
            mode: Some(eff.mode.clone()),
            reload: Some(eff.reload.clone()),
            split_right: None,
            split_down: None,
            kill_pane: None,
            new_window: None,
            resize: None,
            resize_step: None,
            swap_prev: None,
            swap_next: None,
            workspace_next: None,
            workspace_prev: None,
            help: None,
            picker: Some("w".to_string()),
            pane_borders: Some(false),
            show_label_in_border: Some(false),
            pane_gaps: Some(0),
            scrollbar_gutter: Some(false),
            drag_cursor_shape: Some(false),
            border_active_color: Some(eff.border_active_color.clone()),
            border_color: Some(eff.border_color.clone()),
        },
        daemon: DaemonSection {
            socket: Some(eff.socket.clone()),
            state_dir: Some(eff.state_dir.clone()),
            pane_endpoints: Some(eff.pane_endpoints),
            expose_control_socket: Some(eff.expose_control_socket),
            remain_on_exit: Some(eff.remain_on_exit),
            exit_empty: Some(eff.exit_empty),
        },
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
    /// `show-label-in-border`). Default off.
    pub show_label_in_border: bool,
    /// Visible gap bands between panes in render mode (config
    /// `pane-gaps`). Default 0.
    pub pane_gaps: u16,
    /// The reserved right-edge gutter column (config
    /// `scrollbar-gutter`). Default off.
    pub scrollbar_gutter: bool,
    /// The drag cursor shaping (config `drag-cursor-shape`). Default
    /// off.
    pub drag_cursor_shape: bool,
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
            show_label_in_border: false,
            pane_gaps: 0,
            scrollbar_gutter: false,
            drag_cursor_shape: false,
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
}

impl Default for Management {
    fn default() -> Self {
        Self {
            split_right: b'%',
            split_down: b'"',
            kill_pane: b'x',
            new_window: b'c',
            resize: b'R',
            swap_prev: b'{',
            swap_next: b'}',
            workspace_next: b'W',
            workspace_prev: 0x17, // C-w
            help: b'?',
            picker: b'w',
        }
    }
}

/// One after-prefix management key: a single token parsed with the shared
/// prefix parser. A management chord is ONE key matched by byte after the
/// prefix (the reload chord's multi-token spelling exists for
/// documentation only; a management chord keeps the single-key shape).
fn management_key(chord: &str, name: &str) -> Result<u8, String> {
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
    let management = Management {
        split_right: match file.client.split_right.as_deref() {
            Some(chord) => management_key(chord, "split-right")?,
            None => current.management.split_right,
        },
        split_down: match file.client.split_down.as_deref() {
            Some(chord) => management_key(chord, "split-down")?,
            None => current.management.split_down,
        },
        kill_pane: match file.client.kill_pane.as_deref() {
            Some(chord) => management_key(chord, "kill-pane")?,
            None => current.management.kill_pane,
        },
        new_window: match file.client.new_window.as_deref() {
            Some(chord) => management_key(chord, "new-window")?,
            None => current.management.new_window,
        },
        resize: match file.client.resize.as_deref() {
            Some(chord) => management_key(chord, "resize")?,
            None => current.management.resize,
        },
        swap_prev: match file.client.swap_prev.as_deref() {
            Some(chord) => management_key(chord, "swap-prev")?,
            None => current.management.swap_prev,
        },
        swap_next: match file.client.swap_next.as_deref() {
            Some(chord) => management_key(chord, "swap-next")?,
            None => current.management.swap_next,
        },
        workspace_next: match file.client.workspace_next.as_deref() {
            Some(chord) => management_key(chord, "workspace-next")?,
            None => current.management.workspace_next,
        },
        workspace_prev: match file.client.workspace_prev.as_deref() {
            Some(chord) => management_key(chord, "workspace-prev")?,
            None => current.management.workspace_prev,
        },
        help: match file.client.help.as_deref() {
            Some(chord) => management_key(chord, "help")?,
            None => current.management.help,
        },
        picker: match file.client.picker.as_deref() {
            Some(chord) => management_key(chord, "picker")?,
            None => current.management.picker,
        },
    };
    // A step the file names floors at 1 — a zero/negative step would make
    // every resize a no-op by construction.
    let resize_step = file
        .client
        .resize_step
        .map(|step| step.max(1))
        .unwrap_or(current.resize_step);
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
    })
}

/// The daemon-side `reload-config` report, over the re-read file and the
/// applied copy: one `unchanged:`/`restart-required:`/`applied:` line per
/// `[daemon]` setting. The file speaks only for settings it states — a
/// setting absent from the file is `unchanged`, which is what keeps a
/// daemon started with one-shot `--socket`/`--state-dir` flags quiet (a
/// flag is an override the file cannot express, so it never reads back
/// as a change).
///
/// `daemon.remain-on-exit` and `daemon.exit-empty` are the LIVE settings: a
/// stated difference is applied to `applied` and reported `applied:`;
/// every other stated difference is `restart-required:` — the socket and
/// the persist path are fixed at bind, and the boot-tier bools (pane
/// endpoints, expose-control-socket) decide how the daemon was built.
pub fn reload_report(applied: &mut EffectiveConfig, file: Option<&ConfigFile>) -> String {
    let Some(file) = file else {
        // Absent file: nothing changed.
        return [
            "unchanged: daemon.socket",
            "unchanged: daemon.state-dir",
            "unchanged: daemon.pane-endpoints",
            "unchanged: daemon.expose-control-socket",
            "unchanged: daemon.remain-on-exit",
            "unchanged: daemon.exit-empty",
        ]
        .join("\n");
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
    use super::*;

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
                new_window: b'c',
                resize: b'R',
                swap_prev: b'{',
                swap_next: b'}',
                workspace_next: b'W',
                workspace_prev: 0x17,
                help: b'?',
                picker: b'w',
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
}
