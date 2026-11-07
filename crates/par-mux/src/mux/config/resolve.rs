//! Tier resolution: flags > env > file > built-in defaults, producing the
//! [`EffectiveConfig`] the daemon serves and `--gen-config` renders.

use super::{AgentAppend, ConfigFile, ExitAction};

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
    /// The daemon's default exit action: what a workspace whose
    /// `set-exit-action` never chose does when a pane's child exits.
    /// Folded from the file's `exit-action` key, falling back to `hold`
    /// while the legacy `remain-on-exit` boolean is set. The third live
    /// daemon setting: `reload-config` applies a changed value at once
    /// (it takes effect at the next observed death).
    pub exit_action: ExitAction,
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
    /// The continuation prompt a daemon restart passes to a resumed agent
    /// (the fifth live daemon setting: read at each restore, so
    /// `reload-config` applies a change at once). Empty = inject nothing.
    pub agent_resume_prompt: String,
}

/// The default continuation prompt: assess-first wording, so a resumed
/// agent re-grounds itself instead of blindly redoing whatever was in
/// flight when the daemon died.
pub const DEFAULT_AGENT_RESUME_PROMPT: &str = "You were interrupted by a daemon restart. Before doing anything else, re-ground yourself: check the par-mux roster, any background tasks, gates, and running processes for what survived, then continue the work you were doing when the session ended.";

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
            exit_action: ExitAction::Remove,
            exit_empty: true,
            border_active_color: String::new(),
            border_color: String::new(),
            git_status: true,
            git_poll_secs: 10,
            split_start_dir: "home-dir".to_string(),
            agent_append: AgentAppend::default(),
            agent_resume_prompt: DEFAULT_AGENT_RESUME_PROMPT.to_string(),
            config_path: None,
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
    // The exit-action default folds the legacy remain-on-exit boolean:
    // an explicit key wins, an absent key maps `remain-on-exit = true`
    // to `hold`, and an unknown spelling keeps `remove` (the
    // split-start-dir rule).
    eff.exit_action = match file.daemon.exit_action.as_deref() {
        Some(v) => ExitAction::parse(v).unwrap_or(ExitAction::Remove),
        None if eff.remain_on_exit => ExitAction::Hold,
        None => ExitAction::Remove,
    };
    if let Some(v) = file.daemon.exit_empty {
        eff.exit_empty = v;
    }
    // split-start-dir is file-only daemon behavior (no flag or env
    // tier); an unknown spelling keeps the home default.
    if let Some(v) = file.daemon.split_start_dir.as_ref() {
        eff.split_start_dir = v.clone();
    }
    // agent-resume-prompt is file-only daemon behavior like
    // split-start-dir; an empty value disables the injection entirely.
    if let Some(v) = file.daemon.agent_resume_prompt.as_ref() {
        eff.agent_resume_prompt = v.clone();
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
