//! `[daemon]`: the daemon section of the config file and its resolved
//! companions — the agent-append table, the exit/restore policies, and the
//! boot-time canonical reads.

use super::{load_canonical, resolve, Overrides};
use serde::{Deserialize, Serialize};

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
    /// The default exit action for workspaces whose `set-exit-action`
    /// has not chosen one: `remove` (the default), `hold`, or `shell`.
    /// When this key is absent, a `remain-on-exit = true` file maps to
    /// `hold`; any other spelling keeps `remove`.
    #[serde(
        default,
        rename = "exit-action",
        skip_serializing_if = "Option::is_none"
    )]
    pub exit_action: Option<String>,
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
    /// The continuation prompt a daemon restart passes to a resumed agent
    /// (`[daemon] agent-resume-prompt`). Empty disables the injection.
    #[serde(
        default,
        rename = "agent-resume-prompt",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_resume_prompt: Option<String>,
}

/// What happens when a pane's child process exits on its own — the
/// `[daemon] exit-action` default and, through `set-exit-action`, each
/// workspace's own override of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    /// The kill-pane cascade runs: the pane goes, its window closes when
    /// it was the last pane, an emptied session cascades with it (the
    /// default).
    Remove,
    /// The pane is held dead in the tree — the legacy `remain-on-exit`
    /// behavior, scoped to the workspace — so `respawn-pane` can restart
    /// it in place.
    Hold,
    /// The pane respawns the user's default shell in place once its
    /// process has lived at least a second (the reaper's crash-loop
    /// guard); a faster exit holds the pane dead instead.
    Shell,
}

impl ExitAction {
    /// The config-file and control-verb spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Remove => "remove",
            Self::Hold => "hold",
            Self::Shell => "shell",
        }
    }

    /// Parse the config-file and control-verb spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "remove" => Some(Self::Remove),
            "hold" => Some(Self::Hold),
            "shell" => Some(Self::Shell),
            _ => None,
        }
    }
}

/// What a restart does with a workspace's panes — the
/// `set-workspace-restore-policy` choice. The per-pane
/// `--restore start-command` create flag is the single-pane form of
/// [`Self::StartCommand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorePolicy {
    /// Re-run each pane's original start command, every flag; a
    /// hook-reported session id still rides to it as
    /// `PAR_MUX_RESUME_SESSION_ID`, so a wrapper can pass `--resume`
    /// itself.
    StartCommand,
    /// The historical behavior: rebuild each agent pane through its
    /// resume invocation (hook-reported argv first, per-agent table
    /// second).
    AgentResume,
    /// Restore nothing in this workspace: its sessions never rebuild and
    /// the workspace itself comes back empty (the state `new-workspace`
    /// creates). Also the no-restore opt-out asked for by the sibling
    /// card.
    NoRestore,
}

impl RestorePolicy {
    /// The config-file and control-verb spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::StartCommand => "start-command",
            Self::AgentResume => "agent-resume",
            Self::NoRestore => "none",
        }
    }

    /// Parse the config-file and control-verb spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "start-command" => Some(Self::StartCommand),
            "agent-resume" => Some(Self::AgentResume),
            "none" => Some(Self::NoRestore),
            _ => None,
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

/// The daemon's resolved `[daemon] agent-resume-prompt` from the canonical
/// config file (file tier only). Empty = restore injects no continuation
/// prompt; the boot restore path reads this alongside
/// [`daemon_agent_append`].
#[must_use]
pub fn daemon_agent_resume_prompt() -> String {
    resolve(&load_canonical(), &Overrides::default()).agent_resume_prompt
}
