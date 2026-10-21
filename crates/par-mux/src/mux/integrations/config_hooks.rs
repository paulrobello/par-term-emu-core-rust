//! droid, copilot, cursor, qwen, qodercli, devin, kimi, and mastracode
//! arms: session hooks whose registration is an entry inside one config
//! file the agent reads, ported from herdr's integrations (Apache-2.0) onto
//! par-mux's merge discipline. They differ only in data, so one table
//! drives all of them:
//!
//! - droid/qwen/qodercli: claude-shape nested groups under
//!   `hooks.<Event>` (`{hooks: [{type, command, timeout}]}`).
//! - copilot: `$COPILOT_HOME/settings.json` (else `~/.copilot`), flat
//!   entries under `hooks.SessionStart` —
//!   `{type: "command", bash|powershell: <cmd>, timeoutSec}`.
//! - cursor: `$CURSOR_CONFIG_DIR/hooks.json` (else `~/.cursor`), bare
//!   `{command}` entries under the camelCase `hooks.sessionStart`, in a file
//!   that must carry a top-level `version` (herdr adds `1` when missing).
//! - devin: `config.json`, the claude-shape nested group under six
//!   lifecycle events, every command running the `session` action.
//! - kimi: `config.toml`, a marked block of `[[hooks]]` TOML tables
//!   appended after the user's content (herdr's block-marker discipline —
//!   kimi's config is TOML, so there is no JSON splice; the block is
//!   rebuilt whole on every install).
//! - mastracode: `hooks.json`, `{type, command, timeout, description}`
//!   entries under event keys at the DOCUMENT ROOT (no `hooks` wrapper).
//!
//! Every entry runs the same `<runner> '<asset>' <action>` command, and
//! every asset lives under par-mux's own config directory, as claude,
//! codex, and muse do.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as JsonValue};

use super::{
    ensure_root_key, has_command, has_flat_entry, has_root_flat_entry, home_dir, hook_asset_path,
    hook_command_with_action, merge_flat_entry, merge_hook_event, merge_root_flat_entry,
    remove_marked_file, save_settings, unmerge_flat_entries, unmerge_hook_entries,
    unmerge_root_flat_entry, write_hook_asset, HOOK_TIMEOUT_SECS,
};

/// How an arm's config file spells one hook entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryShape {
    /// `{hooks: [{type: "command", command, timeout}]}` under
    /// `hooks.<Event>` (droid, qwen, qodercli, devin).
    NestedGroup,
    /// `{type: "command", bash|powershell: <cmd>, timeoutSec}` (copilot).
    CopilotFlat,
    /// `{command}` (cursor).
    CursorFlat,
    /// `{type, command, timeout(ms), description}` under an event key at
    /// the document root (mastracode's hooks.json).
    MastracodeFlat,
    /// A marked block of `[[hooks]]` TOML tables in config.toml (kimi).
    KimiToml,
}

/// One registration an arm writes: the config-file event key, the action
/// argument its command runs, and (kimi's TOML tables only) the tool
/// matcher restricting the event.
#[derive(Debug)]
struct HookEvent {
    event: &'static str,
    action: &'static str,
    matcher: Option<&'static str>,
}

const fn session_start() -> HookEvent {
    HookEvent {
        event: "SessionStart",
        action: "session",
        matcher: None,
    }
}

static SINGLE_SESSION_EVENT: [HookEvent; 1] = [session_start()];

/// cursor's event key is camelCase.
static CURSOR_EVENT: [HookEvent; 1] = [HookEvent {
    event: "sessionStart",
    action: "session",
    matcher: None,
}];

/// One config-entry agent: where its config lives, how an entry looks,
/// and the reporter assets it runs.
#[derive(Debug)]
pub struct ConfigHookArm {
    /// The agent label the reporter sends (and the status surfaces name).
    pub agent: &'static str,
    config_dir: fn() -> io::Result<PathBuf>,
    config_file: &'static str,
    /// Every event the arm registers, in install order.
    events: &'static [HookEvent],
    shape: EntryShape,
    /// The config file's content when none exists yet.
    bootstrap: &'static str,
    /// The top-level key the agent requires, added when an existing file
    /// lacks it (cursor's `version`).
    required_root_key: Option<(&'static str, &'static str)>,
    install_hint: &'static str,
    asset_posix: &'static str,
    asset_windows: &'static str,
}

/// droid (Factory): `~/.factory/settings.json`. herdr reads no env
/// override for this directory, so neither does par-mux.
pub static DROID: ConfigHookArm = ConfigHookArm {
    agent: "droid",
    config_dir: droid_config_dir,
    config_file: "settings.json",
    events: &SINGLE_SESSION_EVENT,
    shape: EntryShape::NestedGroup,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install droid first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-droid-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-droid-session-hook.ps1"),
};

/// GitHub Copilot CLI: `$COPILOT_HOME/settings.json`, else `~/.copilot`.
pub static COPILOT: ConfigHookArm = ConfigHookArm {
    agent: "copilot",
    config_dir: copilot_config_dir,
    config_file: "settings.json",
    events: &SINGLE_SESSION_EVENT,
    shape: EntryShape::CopilotFlat,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install github copilot cli first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-copilot-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-copilot-session-hook.ps1"),
};

/// Cursor Agent CLI: `$CURSOR_CONFIG_DIR/hooks.json`, else `~/.cursor`.
pub static CURSOR: ConfigHookArm = ConfigHookArm {
    agent: "cursor",
    config_dir: cursor_config_dir,
    config_file: "hooks.json",
    events: &CURSOR_EVENT,
    shape: EntryShape::CursorFlat,
    bootstrap: "{\"version\":1}\n",
    required_root_key: Some(("version", "1")),
    install_hint: "install cursor agent cli first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-cursor-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-cursor-session-hook.ps1"),
};

/// Qwen Code: `$QWEN_HOME/settings.json`, else `~/.qwen` — the
/// claude-schema hooks table herdr registers into.
pub static QWEN: ConfigHookArm = ConfigHookArm {
    agent: "qwen",
    config_dir: qwen_config_dir,
    config_file: "settings.json",
    events: &SINGLE_SESSION_EVENT,
    shape: EntryShape::NestedGroup,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install qwen code first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-qwen-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-qwen-session-hook.ps1"),
};

/// Qoder CLI: `$QODERCLI_CONFIG_DIR/settings.json`, else `~/.qoder` —
/// qoder's schema mirrors claude's settings.json hooks.
pub static QODERCLI: ConfigHookArm = ConfigHookArm {
    agent: "qodercli",
    config_dir: qodercli_config_dir,
    config_file: "settings.json",
    events: &SINGLE_SESSION_EVENT,
    shape: EntryShape::NestedGroup,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install qoder cli first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-qodercli-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-qodercli-session-hook.ps1"),
};

/// Devin CLI: `config.json` under `$XDG_CONFIG_HOME/devin` (else
/// `~/.config/devin`; `%APPDATA%\devin` on Windows). herdr's devin
/// integration registers the `session` action across six lifecycle events
/// (its v3 shape; the state-action entries of herdr's v1/v2 were herdr's
/// own legacy, which par-mux never installed, so there is nothing to
/// sweep).
static DEVIN_EVENTS: [HookEvent; 6] = [
    session_event("SessionStart"),
    session_event("UserPromptSubmit"),
    session_event("PreToolUse"),
    session_event("PostToolUse"),
    session_event("PermissionRequest"),
    session_event("Stop"),
];

pub static DEVIN: ConfigHookArm = ConfigHookArm {
    agent: "devin",
    config_dir: devin_config_dir,
    config_file: "config.json",
    events: &DEVIN_EVENTS,
    shape: EntryShape::NestedGroup,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install devin cli first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-devin-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-devin-session-hook.ps1"),
};

/// Kimi Code: `$KIMI_CODE_HOME/config.toml`, else `~/.kimi-code`. The
/// hook tables carry tool matchers herdr derived for kimi's
/// AskUserQuestion flow (blocked on the question, working around it).
static KIMI_EVENTS: [HookEvent; 12] = [
    HookEvent {
        event: "SessionStart",
        action: "session",
        matcher: None,
    },
    HookEvent {
        event: "UserPromptSubmit",
        action: "working",
        matcher: None,
    },
    HookEvent {
        event: "PreToolUse",
        action: "working",
        matcher: Some("^(?!AskUserQuestion$).*$"),
    },
    HookEvent {
        event: "PreToolUse",
        action: "blocked",
        matcher: Some("^AskUserQuestion$"),
    },
    HookEvent {
        event: "PostToolUse",
        action: "working",
        matcher: Some("^AskUserQuestion$"),
    },
    HookEvent {
        event: "PostToolUseFailure",
        action: "working",
        matcher: Some("^AskUserQuestion$"),
    },
    HookEvent {
        event: "SubagentStart",
        action: "working",
        matcher: None,
    },
    HookEvent {
        event: "PreCompact",
        action: "working",
        matcher: None,
    },
    HookEvent {
        event: "PermissionRequest",
        action: "blocked",
        matcher: None,
    },
    HookEvent {
        event: "PermissionResult",
        action: "working",
        matcher: None,
    },
    HookEvent {
        event: "Stop",
        action: "idle",
        matcher: None,
    },
    HookEvent {
        event: "Interrupt",
        action: "idle",
        matcher: None,
    },
];

pub static KIMI: ConfigHookArm = ConfigHookArm {
    agent: "kimi",
    config_dir: kimi_config_dir,
    config_file: "config.toml",
    events: &KIMI_EVENTS,
    shape: EntryShape::KimiToml,
    bootstrap: "",
    required_root_key: None,
    install_hint: "install kimi code first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-kimi-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-kimi-session-hook.ps1"),
};

/// MastraCode: `~/.mastracode/hooks.json` — flat command entries under
/// event keys at the document root, timeouts in milliseconds, one action
/// per lifecycle event (herdr's table).
static MASTRACODE_EVENTS: [HookEvent; 11] = [
    session_event("SessionStart"),
    action_event("UserPromptSubmit", "working"),
    action_event("AgentStart", "working"),
    action_event("PreToolUse", "working"),
    action_event("PermissionRequest", "blocked"),
    action_event("PermissionResult", "working"),
    action_event("SubagentStart", "working"),
    action_event("SubagentEnd", "working"),
    action_event("Interrupt", "idle"),
    action_event("AgentEnd", "idle"),
    action_event("Stop", "idle"),
];

pub static MASTRACODE: ConfigHookArm = ConfigHookArm {
    agent: "mastracode",
    config_dir: mastracode_config_dir,
    config_file: "hooks.json",
    events: &MASTRACODE_EVENTS,
    shape: EntryShape::MastracodeFlat,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install mastracode first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-mastracode-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-mastracode-session-hook.ps1"),
};

/// MastraCode's hook timeout is milliseconds (herdr's
/// `MASTRACODE_HOOK_TIMEOUT_MS`), unlike every JSON arm's seconds.
const MASTRACODE_HOOK_TIMEOUT_MS: u64 = 10_000;

const fn session_event(event: &'static str) -> HookEvent {
    action_event(event, "session")
}

const fn action_event(event: &'static str, action: &'static str) -> HookEvent {
    HookEvent {
        event,
        action,
        matcher: None,
    }
}

/// The arms, in the order the status surfaces list them.
pub static CONFIG_HOOK_ARMS: [&ConfigHookArm; 8] = [
    &DROID,
    &COPILOT,
    &CURSOR,
    &QWEN,
    &QODERCLI,
    &DEVIN,
    &KIMI,
    &MASTRACODE,
];

const COPILOT_HOME_ENV_VAR: &str = "COPILOT_HOME";
const CURSOR_CONFIG_DIR_ENV_VAR: &str = "CURSOR_CONFIG_DIR";
const QWEN_HOME_ENV_VAR: &str = "QWEN_HOME";
const QODERCLI_CONFIG_DIR_ENV_VAR: &str = "QODERCLI_CONFIG_DIR";
const KIMI_CODE_HOME_ENV_VAR: &str = "KIMI_CODE_HOME";

fn droid_config_dir() -> io::Result<PathBuf> {
    Ok(home_dir()?.join(".factory"))
}

fn copilot_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(COPILOT_HOME_ENV_VAR).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".copilot"))
}

fn qwen_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(QWEN_HOME_ENV_VAR).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".qwen"))
}

fn qodercli_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) =
        std::env::var_os(QODERCLI_CONFIG_DIR_ENV_VAR).filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".qoder"))
}

fn cursor_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(CURSOR_CONFIG_DIR_ENV_VAR).filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".cursor"))
}

fn kimi_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(KIMI_CODE_HOME_ENV_VAR).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(home_dir()?.join(".kimi-code"))
}

/// herdr's devin_dir: `$XDG_CONFIG_HOME/devin` first, then the Windows
/// `APPDATA` split, else `~/.config/devin`.
fn devin_config_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir).join("devin"));
    }
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir).join("devin"));
    }
    Ok(home_dir()?.join(".config").join("devin"))
}

fn mastracode_config_dir() -> io::Result<PathBuf> {
    Ok(home_dir()?.join(".mastracode"))
}

/// Outcome of a config-entry hook install.
#[derive(Debug)]
pub struct ConfigHookInstall {
    /// The agent config file that was merged into.
    pub config_path: PathBuf,
    /// Where the hook script asset was written.
    pub hook_path: PathBuf,
    /// Whether the config file changed (false = our entry was already
    /// present; the asset is still refreshed).
    pub config_changed: bool,
}

/// Outcome of a config-entry hook uninstall.
#[derive(Debug)]
pub struct ConfigHookUninstall {
    /// The agent config file our entry was removed from.
    pub config_path: PathBuf,
    /// Where the hook script asset lived.
    pub hook_path: PathBuf,
    /// Whether the config file changed (false = our entry was not there).
    pub config_changed: bool,
    /// Whether the hook script asset was ours to delete.
    pub hook_removed: bool,
}

impl ConfigHookArm {
    /// The asset file name for the current platform. Runtime-selected (not
    /// cfg-split) so both arms compile and are asserted on every platform.
    fn install_name(&self) -> String {
        let ext = if cfg!(windows) { "ps1" } else { "sh" };
        format!("par-mux-{}-session-hook.{ext}", self.agent)
    }

    /// The hook asset contents for the current platform.
    fn asset(&self) -> &'static str {
        if cfg!(windows) {
            self.asset_windows
        } else {
            self.asset_posix
        }
    }

    /// The marker identifying an installed script as ours.
    fn marker(&self) -> String {
        format!("PAR_MUX_INTEGRATION_ID={}", self.agent)
    }

    /// Whether the agent looks installed: its config directory exists. The
    /// bulk `--install-agent-hooks` skips an absent agent instead of
    /// failing, so adding an arm never turns a working install red on a
    /// machine without that agent.
    pub fn agent_present(&self) -> bool {
        (self.config_dir)().is_ok_and(|dir| dir.is_dir())
    }

    /// The agent config file this arm merges into.
    pub fn config_path(&self) -> io::Result<PathBuf> {
        Ok((self.config_dir)()?.join(self.config_file))
    }

    fn asset_path(&self) -> io::Result<PathBuf> {
        hook_asset_path(&self.install_name())
    }

    /// Our entry for `command` in this arm's shape, for the platform
    /// `windows` names (copilot keys the command by shell).
    fn entry(&self, command: &str, windows: bool) -> JsonValue {
        match self.shape {
            EntryShape::NestedGroup | EntryShape::KimiToml => {
                super::canonical_entry_value(None, command)
            }
            EntryShape::CopilotFlat => {
                let field = if windows { "powershell" } else { "bash" };
                json!({ "type": "command", field: command, "timeoutSec": HOOK_TIMEOUT_SECS })
            }
            EntryShape::CursorFlat => json!({ "command": command }),
            EntryShape::MastracodeFlat => json!({
                "type": "command",
                "command": command,
                "timeout": MASTRACODE_HOOK_TIMEOUT_MS,
                "description": "Report MastraCode agent state to par-mux",
            }),
        }
    }

    /// Whether a flat entry is ours: its command string, under whichever
    /// key the shape allows, is exactly `command`.
    fn flat_entry_is_ours(&self, entry: &JsonValue, command: &str) -> bool {
        let text = |key: &str| entry.get(key).and_then(JsonValue::as_str) == Some(command);
        match self.shape {
            EntryShape::NestedGroup | EntryShape::KimiToml => false,
            EntryShape::CopilotFlat => {
                entry.get("type").and_then(JsonValue::as_str) == Some("command")
                    && (text("bash") || text("powershell") || text("command"))
            }
            EntryShape::CursorFlat => text("command"),
            EntryShape::MastracodeFlat => {
                entry.get("type").and_then(JsonValue::as_str) == Some("command") && text("command")
            }
        }
    }

    /// Whether `content` already registers every event's command.
    fn registered(&self, content: &str, config_path: &Path, hook_path: &Path) -> io::Result<bool> {
        if self.shape == EntryShape::KimiToml {
            // The block is rebuilt whole on install, so currency is the
            // block markers plus every current command line.
            return Ok(content.contains(KIMI_BLOCK_BEGIN)
                && self
                    .events
                    .iter()
                    .all(|event| content.contains(&kimi_command_line(hook_path, event))));
        }
        for event in self.events {
            let command = hook_command_with_action(hook_path, event.action);
            let present = match self.shape {
                EntryShape::NestedGroup => {
                    has_command(content, config_path, &command, event.event)?
                }
                EntryShape::CopilotFlat | EntryShape::CursorFlat => {
                    has_flat_entry(content, config_path, event.event, &|entry| {
                        self.flat_entry_is_ours(entry, &command)
                    })?
                }
                EntryShape::MastracodeFlat => {
                    has_root_flat_entry(content, config_path, event.event, &|entry| {
                        self.flat_entry_is_ours(entry, &command)
                    })?
                }
                EntryShape::KimiToml => unreachable!("handled above"),
            };
            if !present {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The install-time AND verify-time read of the asset's currency.
    pub(crate) fn hook_asset_state(&self) -> super::AssetState {
        match self.asset_path() {
            Ok(path) => super::AssetState::at(&path, self.asset()),
            Err(_) => super::AssetState::absent(self.asset()),
        }
    }

    /// Whether the agent config registers par-mux's entry (read-only).
    pub(crate) fn installed(&self) -> bool {
        let (Ok(config_path), Ok(hook_path)) = (self.config_path(), self.asset_path()) else {
            return false;
        };
        let Ok(content) = fs::read_to_string(&config_path) else {
            return false;
        };
        self.registered(&content, &config_path, &hook_path)
            .unwrap_or(false)
    }

    /// Install: merge our entry into the agent config and write the asset.
    pub fn install(&self) -> io::Result<ConfigHookInstall> {
        self.install_into(&self.config_path()?, &self.asset_path()?)
    }

    /// Install with explicit paths (test/install seam).
    pub fn install_into(
        &self,
        config_path: &Path,
        hook_path: &Path,
    ) -> io::Result<ConfigHookInstall> {
        self.install_into_for(config_path, hook_path, cfg!(windows))
    }

    fn install_into_for(
        &self,
        config_path: &Path,
        hook_path: &Path,
        windows: bool,
    ) -> io::Result<ConfigHookInstall> {
        let agent_dir = config_path.parent().ok_or_else(|| {
            io::Error::other(format!(
                "{} config path has no parent: {config_path:?}",
                self.agent
            ))
        })?;
        if !agent_dir.is_dir() {
            return Err(io::Error::other(format!(
                "{} config directory not found at {}. {}",
                self.agent,
                agent_dir.display(),
                self.install_hint
            )));
        }

        let original = match fs::read_to_string(config_path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => self.bootstrap.to_string(),
            Err(err) => return Err(err),
        };

        // Merge and verify BEFORE writing anything: a malformed user config
        // fails with the file, and the asset, untouched.
        let mut content = original.clone();
        if let Some((key, value)) = self.required_root_key {
            if let Some(updated) = ensure_root_key(&content, config_path, key, value)? {
                content = updated;
            }
        }
        match self.shape {
            EntryShape::KimiToml => {
                let rebuilt = build_kimi_config(&original, hook_path);
                if rebuilt != original {
                    content = rebuilt;
                }
            }
            _ => {
                for event in self.events {
                    let command = hook_command_with_action(hook_path, event.action);
                    let merged = match self.shape {
                        EntryShape::NestedGroup => merge_hook_event(
                            &content,
                            config_path,
                            &command,
                            event.matcher,
                            event.event,
                        )?,
                        EntryShape::CopilotFlat | EntryShape::CursorFlat => merge_flat_entry(
                            &content,
                            config_path,
                            event.event,
                            &self.entry(&command, windows),
                            &|entry| self.flat_entry_is_ours(entry, &command),
                        )?,
                        EntryShape::MastracodeFlat => merge_root_flat_entry(
                            &content,
                            config_path,
                            event.event,
                            &self.entry(&command, windows),
                            &|entry| self.flat_entry_is_ours(entry, &command),
                        )?,
                        EntryShape::KimiToml => unreachable!("handled above"),
                    };
                    if let Some(updated) = merged {
                        content = updated;
                    }
                }
            }
        }
        // A bootstrapped file is written even when our entry was all it got.
        let config_changed = content != original || !config_path.exists();

        write_hook_asset(hook_path, self.asset())?;
        if config_changed {
            save_settings(config_path, &content)?;
        }

        Ok(ConfigHookInstall {
            config_path: config_path.to_path_buf(),
            hook_path: hook_path.to_path_buf(),
            config_changed,
        })
    }

    /// Uninstall: remove our entry from the agent config and delete the
    /// asset when it carries our marker.
    pub fn uninstall(&self) -> io::Result<ConfigHookUninstall> {
        self.uninstall_into(&self.config_path()?, &self.asset_path()?)
    }

    /// Uninstall with explicit paths (test/install seam).
    pub fn uninstall_into(
        &self,
        config_path: &Path,
        hook_path: &Path,
    ) -> io::Result<ConfigHookUninstall> {
        let mut config_changed = false;
        if config_path.is_file() {
            let mut content = fs::read_to_string(config_path)?;
            match self.shape {
                EntryShape::KimiToml => {
                    let removed = remove_kimi_hook_block(&content);
                    if removed != content {
                        content = removed;
                        config_changed = true;
                    }
                }
                EntryShape::NestedGroup => {
                    let commands: Vec<String> = self
                        .events
                        .iter()
                        .map(|event| hook_command_with_action(hook_path, event.action))
                        .collect();
                    let pairs: Vec<(&str, &str)> = self
                        .events
                        .iter()
                        .zip(&commands)
                        .map(|(event, command)| (event.event, command.as_str()))
                        .collect();
                    if let Some(updated) = unmerge_hook_entries(&content, config_path, &pairs)? {
                        content = updated;
                        config_changed = true;
                    }
                }
                _ => {
                    for event in self.events {
                        let command = hook_command_with_action(hook_path, event.action);
                        let removed = match self.shape {
                            EntryShape::MastracodeFlat => unmerge_root_flat_entry(
                                &content,
                                config_path,
                                event.event,
                                &|entry| self.flat_entry_is_ours(entry, &command),
                            )?,
                            _ => unmerge_flat_entries(
                                &content,
                                config_path,
                                event.event,
                                &|entry| self.flat_entry_is_ours(entry, &command),
                            )?,
                        };
                        if let Some(updated) = removed {
                            content = updated;
                            config_changed = true;
                        }
                    }
                }
            }
            if config_changed {
                save_settings(config_path, &content)?;
            }
        }
        let hook_removed = remove_marked_file(hook_path, &self.marker())?;
        Ok(ConfigHookUninstall {
            config_path: config_path.to_path_buf(),
            hook_path: hook_path.to_path_buf(),
            config_changed,
            hook_removed,
        })
    }

    /// Startup self-heal: rewrite the asset when the agent config still
    /// registers it but the script is gone. Best-effort; never touches the
    /// agent config.
    pub(crate) fn heal(&self) {
        let (Ok(config_path), Ok(hook_path)) = (self.config_path(), self.asset_path()) else {
            return;
        };
        if let Err(err) = self.heal_into(&config_path, &hook_path) {
            log::warn!("{} mux-hook heal skipped: {err}", self.agent);
        }
    }

    fn heal_into(&self, config_path: &Path, hook_path: &Path) -> io::Result<()> {
        if hook_path.exists() {
            return Ok(());
        }
        let Ok(content) = fs::read_to_string(config_path) else {
            return Ok(());
        };
        if self.registered(&content, config_path, hook_path)? {
            write_hook_asset(hook_path, self.asset())?;
            log::info!("mux hook heal: restored {}", hook_path.display());
        }
        Ok(())
    }
}

// kimi's config.toml is TOML, so there is no JSON splice: herdr's
// block-marker discipline — the whole marked block is removed and re-appended
// on every install, and uninstall cuts the block (ported from herdr's
// config_edit.rs, Apache-2.0).

const KIMI_BLOCK_BEGIN: &str = "# >>> par-mux kimi integration";
const KIMI_BLOCK_END: &str = "# <<< par-mux kimi integration";
const KIMI_HOOK_TIMEOUT_SECS: u64 = 10;

/// The `command = "<cmd>"` line one kimi event registers — the block's
/// currency check.
fn kimi_command_line(hook_path: &Path, event: &HookEvent) -> String {
    format!(
        "command = {}",
        toml_basic_string(&hook_command_with_action(hook_path, event.action))
    )
}

/// The user's content with the current hook block appended after it.
fn build_kimi_config(content: &str, hook_path: &Path) -> String {
    let mut result = remove_kimi_hook_block(content)
        .trim_end_matches('\n')
        .to_string();
    if !result.is_empty() {
        result.push('\n');
        result.push('\n');
    }
    result.push_str(KIMI_BLOCK_BEGIN);
    result.push('\n');
    for event in KIMI_EVENTS.iter() {
        let command = hook_command_with_action(hook_path, event.action);
        let matcher = event
            .matcher
            .map(|matcher| format!("matcher = {}\n", toml_basic_string(matcher)))
            .unwrap_or_default();
        result.push_str(&format!(
            "[[hooks]]\nevent = {}\n{matcher}command = {}\ntimeout = {}\n\n",
            toml_basic_string(event.event),
            toml_basic_string(&command),
            KIMI_HOOK_TIMEOUT_SECS,
        ));
    }
    result.push_str(KIMI_BLOCK_END);
    result.push('\n');
    result
}

/// The content with our marked block cut; unchanged when no block exists.
fn remove_kimi_hook_block(content: &str) -> String {
    let trailing_newline = content.ends_with('\n');
    let mut lines = Vec::new();
    let mut in_block = false;
    let mut removed_block = false;

    for line in content.lines() {
        if line.trim() == KIMI_BLOCK_BEGIN {
            in_block = true;
            removed_block = true;
            continue;
        }
        if in_block {
            if line.trim() == KIMI_BLOCK_END {
                in_block = false;
            }
            continue;
        }
        lines.push(line.to_string());
    }

    if !removed_block {
        return content.to_string();
    }

    let mut result = lines.join("\n");
    if trailing_newline || result.is_empty() {
        result.push('\n');
    }
    while result.ends_with("\n\n") {
        result.pop();
    }
    if result == "\n" {
        String::new()
    } else {
        result
    }
}

/// A basic TOML string with herdr's escaping.
fn toml_basic_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\u{08}' => result.push_str("\\b"),
            '\t' => result.push_str("\\t"),
            '\n' => result.push_str("\\n"),
            '\u{0c}' => result.push_str("\\f"),
            '\r' => result.push_str("\\r"),
            ch if ch <= '\u{1f}' || ch == '\u{7f}' => {
                result.push_str(&format!("\\u{:04X}", ch as u32));
            }
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::super::parse_serde;
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// A temp agent config dir plus the config and asset paths in it.
    fn stage(arm: &ConfigHookArm) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = temp_root();
        let agent_dir = root.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let config = agent_dir.join(arm.config_file);
        let hook = root
            .path()
            .join("par-mux")
            .join("hooks")
            .join(arm.install_name());
        (root, config, hook)
    }

    /// Our registered entries under `event`, as parsed JSON.
    fn event_entries(config: &Path, event: &str) -> Vec<JsonValue> {
        let parsed = parse_serde(&fs::read_to_string(config).unwrap(), config).unwrap();
        parsed["hooks"][event]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn droid_install_bootstraps_a_nested_session_start_group() {
        let (_root, config, hook) = stage(&DROID);

        let result = DROID.install_into(&config, &hook).unwrap();

        assert!(result.config_changed);
        let entries = event_entries(&config, "SessionStart");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].get("matcher").is_none(), "matcher-less");
        let entry = &entries[0]["hooks"][0];
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["timeout"], HOOK_TIMEOUT_SECS);
        let command = entry["command"].as_str().unwrap();
        let runner = if cfg!(windows) { "powershell " } else { "sh " };
        assert!(command.starts_with(runner) && command.ends_with(" session"));
        assert_eq!(fs::read_to_string(&hook).unwrap(), DROID.asset());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&hook).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "the hook must be executable");
        }
    }

    #[test]
    fn copilot_install_writes_a_flat_shell_keyed_entry() {
        let (_root, config, hook) = stage(&COPILOT);

        COPILOT.install_into(&config, &hook).unwrap();

        let entries = event_entries(&config, "SessionStart");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["timeoutSec"], HOOK_TIMEOUT_SECS);
        let field = if cfg!(windows) { "powershell" } else { "bash" };
        let command = entry[field].as_str().expect("command under the shell key");
        assert!(command.ends_with(" session"));
        assert!(entry.get("hooks").is_none(), "flat, not a nested group");
    }

    #[test]
    fn copilot_entry_keys_the_command_by_platform_shell() {
        let posix = COPILOT.entry("cmd", false);
        assert_eq!(posix["bash"], "cmd");
        assert!(posix.get("powershell").is_none());
        let windows = COPILOT.entry("cmd", true);
        assert_eq!(windows["powershell"], "cmd");
        assert!(windows.get("bash").is_none());
        // Either spelling is recognized as ours, so uninstall finds an entry
        // whichever platform wrote it.
        assert!(COPILOT.flat_entry_is_ours(&posix, "cmd"));
        assert!(COPILOT.flat_entry_is_ours(&windows, "cmd"));
        assert!(!COPILOT.flat_entry_is_ours(&json!({"type": "command", "bash": "other"}), "cmd"));
    }

    #[test]
    fn cursor_install_bootstraps_version_and_a_bare_camel_case_entry() {
        let (_root, config, hook) = stage(&CURSOR);

        CURSOR.install_into(&config, &hook).unwrap();

        let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
        assert_eq!(parsed["version"], 1);
        assert!(parsed["hooks"].get("SessionStart").is_none());
        let entries = event_entries(&config, "sessionStart");
        assert_eq!(entries.len(), 1);
        let entry = entries[0].as_object().unwrap();
        assert_eq!(
            entry.len(),
            1,
            "cursor's minimal {{command}} shape: {entry:?}"
        );
        assert!(entry["command"].as_str().unwrap().ends_with(" session"));
    }

    #[test]
    fn cursor_install_adds_version_to_an_existing_file_that_lacks_it() {
        let (_root, config, hook) = stage(&CURSOR);
        fs::write(&config, "{\n  \"hooks\": {}\n}\n").unwrap();

        CURSOR.install_into(&config, &hook).unwrap();

        let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
        assert_eq!(parsed["version"], 1);
        assert_eq!(event_entries(&config, "sessionStart").len(), 1);
    }

    #[test]
    fn devin_installs_the_session_action_across_six_lifecycle_events() {
        let (_root, config, hook) = stage(&DEVIN);

        DEVIN.install_into(&config, &hook).unwrap();

        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "PermissionRequest",
            "Stop",
        ] {
            let entries = event_entries(&config, event);
            assert_eq!(entries.len(), 1, "{event}");
            let command = entries[0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("{event}"));
            assert!(
                command.ends_with(" session"),
                "{event}: every devin command runs the session action: {command}"
            );
        }
    }

    #[test]
    fn kimi_install_appends_a_marked_toml_block_with_matchers() {
        let (_root, config, hook) = stage(&KIMI);

        let result = KIMI.install_into(&config, &hook).unwrap();

        assert!(result.config_changed);
        let content = fs::read_to_string(&config).unwrap();
        assert!(
            content.starts_with(KIMI_BLOCK_BEGIN),
            "block first in a fresh file"
        );
        assert!(content.ends_with("# <<< par-mux kimi integration\n"));
        assert_eq!(
            content.matches("[[hooks]]").count(),
            12,
            "one table per event"
        );
        assert_eq!(content.matches("timeout = 10").count(), 12);
        assert_eq!(
            content.matches("matcher = \"^AskUserQuestion$\"").count(),
            3,
            "the question-matched trio"
        );
        assert_eq!(
            content
                .matches("matcher = \"^(?!AskUserQuestion$).*$\"")
                .count(),
            1
        );
        let session_command = hook_command_with_action(&hook, "session");
        assert!(content.contains(&format!(
            "command = {}",
            toml_basic_string(&session_command)
        )));
        assert!(KIMI.installed_at(&config, &hook));
    }

    #[test]
    fn kimi_install_replaces_a_stale_block_after_the_user_content() {
        let (_root, config, hook) = stage(&KIMI);
        fs::write(&config, "# user's kimi config\nmodel = \"k2\"\n").unwrap();

        KIMI.install_into(&config, &hook).unwrap();

        let content = fs::read_to_string(&config).unwrap();
        assert!(content.starts_with("# user's kimi config\n"));
        let user_end = content.find(KIMI_BLOCK_BEGIN).unwrap();
        assert_eq!(&content[user_end - 2..user_end], "\n\n");
    }

    #[test]
    fn mastracode_installs_root_level_entries_with_ms_timeouts() {
        let (_root, config, hook) = stage(&MASTRACODE);

        MASTRACODE.install_into(&config, &hook).unwrap();

        let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
        assert!(parsed.get("hooks").is_none(), "no hooks wrapper: {parsed}");
        for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
            let entries = parsed[event]
                .as_array()
                .unwrap_or_else(|| panic!("{event}"));
            assert_eq!(entries.len(), 1, "{event}");
            let entry = &entries[0];
            assert_eq!(entry["type"], "command");
            assert_eq!(entry["timeout"], 10_000, "{event}: milliseconds");
            assert_eq!(
                entry["description"],
                "Report MastraCode agent state to par-mux"
            );
        }
        assert!(parsed["SessionStart"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(" session"));
        assert!(parsed["Stop"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(" idle"));
    }

    /// The user's own file, with comments and their own entries, round
    /// trips byte-exact through install then uninstall for every arm.
    #[test]
    fn every_arm_restores_a_user_config_byte_for_byte() {
        let user_files: [(&ConfigHookArm, &str); 6] = [
            (
                &DROID,
                concat!(
                    "{\n",
                    "  // user's droid settings\n",
                    "  \"model\": \"claude-opus\",\n",
                    "  \"hooks\": {\n",
                    "    \"SessionStart\": [\n",
                    "      { \"hooks\": [ { \"type\": \"command\", \"command\": \"user-hook\" } ] }\n",
                    "    ]\n",
                    "  }\n",
                    "}\n"
                ),
            ),
            (
                &COPILOT,
                concat!(
                    "{\n",
                    "  \"theme\": \"dark\",\n",
                    "  \"hooks\": {\n",
                    "    \"SessionStart\": [\n",
                    "      { \"type\": \"command\", \"bash\": \"user-hook\", \"timeoutSec\": 5 }\n",
                    "    ]\n",
                    "  }\n",
                    "}\n"
                ),
            ),
            (
                &CURSOR,
                concat!(
                    "{\n",
                    "  \"version\": 1,\n",
                    "  \"hooks\": {\n",
                    "    \"stop\": [ { \"command\": \"user-hook\" } ]\n",
                    "  }\n",
                    "}\n"
                ),
            ),
            (
                &DEVIN,
                concat!(
                    "{\n",
                    "  // user's devin config\n",
                    "  \"theme\": \"dark\",\n",
                    "  \"hooks\": {\n",
                    "    \"SessionStart\": [\n",
                    "      { \"hooks\": [ { \"type\": \"command\", \"command\": \"user-hook\" } ] }\n",
                    "    ]\n",
                    "  }\n",
                    "}\n"
                ),
            ),
            // kimi's block editor appends after the user's lines and cuts
            // the block back out on uninstall; a trailing blank line would
            // not survive the append (herdr's trim discipline), so the
            // fixture ends on content.
            (
                &KIMI,
                concat!(
                    "# user's kimi config\n",
                    "model = \"user-hook\"\n",
                    "[features]\n",
                    "hooks = true\n"
                ),
            ),
            (
                &MASTRACODE,
                concat!(
                    "{\n",
                    "  // user's hooks\n",
                    "  \"SessionStart\": [\n",
                    "    { \"type\": \"command\", \"command\": \"user-hook\" }\n",
                    "  ]\n",
                    "}\n"
                ),
            ),
        ];
        for (arm, original) in user_files {
            let (_root, config, hook) = stage(arm);
            fs::write(&config, original).unwrap();

            let installed = arm.install_into(&config, &hook).unwrap();
            assert!(installed.config_changed, "{}: merged", arm.agent);
            let merged = fs::read_to_string(&config).unwrap();
            assert!(
                merged.contains("user-hook"),
                "{}: user entry kept",
                arm.agent
            );
            assert!(
                arm.installed_at(&config, &hook),
                "{}: registered",
                arm.agent
            );

            let removed = arm.uninstall_into(&config, &hook).unwrap();
            assert!(
                removed.config_changed && removed.hook_removed,
                "{}",
                arm.agent
            );
            assert_eq!(
                fs::read_to_string(&config).unwrap(),
                original,
                "{}: the splice must restore the user's bytes exactly",
                arm.agent
            );
        }
    }

    #[test]
    fn every_arm_reinstalls_as_a_config_no_op_and_refreshes_the_asset() {
        for arm in CONFIG_HOOK_ARMS {
            let (_root, config, hook) = stage(arm);
            arm.install_into(&config, &hook).unwrap();
            let after_first = fs::read_to_string(&config).unwrap();
            fs::write(&hook, "# user edit").unwrap();

            let second = arm.install_into(&config, &hook).unwrap();

            assert!(!second.config_changed, "{}: already present", arm.agent);
            assert_eq!(fs::read_to_string(&config).unwrap(), after_first);
            assert_eq!(fs::read_to_string(&hook).unwrap(), arm.asset());
        }
    }

    #[test]
    fn every_arm_uninstall_prunes_what_it_added_and_is_idempotent() {
        for arm in CONFIG_HOOK_ARMS {
            let (_root, config, hook) = stage(arm);
            arm.install_into(&config, &hook).unwrap();

            let result = arm.uninstall_into(&config, &hook).unwrap();

            assert!(
                result.config_changed && result.hook_removed,
                "{}",
                arm.agent
            );
            if arm.shape == EntryShape::KimiToml {
                let content = fs::read_to_string(&config).unwrap();
                assert!(
                    !content.contains(KIMI_BLOCK_BEGIN) && !content.contains(KIMI_BLOCK_END),
                    "{}: the block is cut: {content}",
                    arm.agent
                );
            } else if arm.shape == EntryShape::MastracodeFlat {
                let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
                assert!(
                    parsed.as_object().is_some_and(|root| root.is_empty()),
                    "{}: every root-level event key pruned: {parsed}",
                    arm.agent
                );
            } else {
                let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
                assert!(
                    parsed.get("hooks").is_none(),
                    "{}: hooks pruned: {parsed}",
                    arm.agent
                );
            }
            assert!(!hook.exists());
            let again = arm.uninstall_into(&config, &hook).unwrap();
            assert!(
                !again.config_changed && !again.hook_removed,
                "{}",
                arm.agent
            );
        }
    }

    #[test]
    fn uninstall_leaves_a_foreign_script_with_our_name_alone() {
        for arm in CONFIG_HOOK_ARMS {
            let (_root, config, hook) = stage(arm);
            fs::create_dir_all(hook.parent().unwrap()).unwrap();
            fs::write(&hook, "# user's own script, no marker\n").unwrap();

            let result = arm.uninstall_into(&config, &hook).unwrap();

            assert!(!result.hook_removed, "{}: not ours", arm.agent);
            assert!(hook.exists());
        }
    }

    #[test]
    fn a_missing_config_directory_is_an_actionable_error() {
        for arm in CONFIG_HOOK_ARMS {
            let root = temp_root();
            let config = root.path().join("never").join(arm.config_file);
            let hook = root.path().join("hook");

            let err = arm.install_into(&config, &hook).unwrap_err().to_string();

            assert!(
                err.contains(&format!("{} config directory not found at", arm.agent)),
                "error must name the path: {err}"
            );
            assert!(
                err.contains(arm.install_hint),
                "error must say what to do: {err}"
            );
            assert!(!hook.exists());
        }
    }

    #[test]
    fn a_malformed_config_fails_with_nothing_written() {
        for arm in CONFIG_HOOK_ARMS {
            if arm.shape == EntryShape::KimiToml {
                // kimi's block editor never parses the file (herdr's
                // discipline), so there is no malformed-file failure mode.
                continue;
            }
            let (_root, config, hook) = stage(arm);
            let broken = "{\"hooks\":{},} trailing";
            fs::write(&config, broken).unwrap();

            assert!(arm.install_into(&config, &hook).is_err(), "{}", arm.agent);

            assert_eq!(fs::read_to_string(&config).unwrap(), broken);
            assert!(
                !hook.exists(),
                "{}: no asset when the merge fails",
                arm.agent
            );
        }
    }

    #[test]
    fn heal_restores_a_missing_asset_only_when_registered() {
        for arm in CONFIG_HOOK_ARMS {
            let (_root, config, hook) = stage(arm);
            arm.heal_into(&config, &hook).unwrap();
            assert!(
                !hook.exists(),
                "{}: not installed, nothing healed",
                arm.agent
            );

            arm.install_into(&config, &hook).unwrap();
            fs::remove_file(&hook).unwrap();
            arm.heal_into(&config, &hook).unwrap();
            assert_eq!(fs::read_to_string(&hook).unwrap(), arm.asset());
        }
    }

    #[test]
    fn every_asset_is_a_marked_versioned_env_renamed_port() {
        for arm in CONFIG_HOOK_ARMS {
            for (platform, asset) in [("sh", arm.asset_posix), ("ps1", arm.asset_windows)] {
                let name = format!("{} {platform}", arm.agent);
                assert!(asset.contains(&arm.marker()), "{name}: marker");
                assert_eq!(
                    super::super::parse_integration_version(asset),
                    Some("1".to_string()),
                    "{name}: version"
                );
                assert!(asset.contains("pane.report_agent_session"), "{name}");
                assert!(asset.contains(&format!("par-mux:{}", arm.agent)), "{name}");
                assert!(!asset.contains("HERDR"), "{name}: fully env-renamed");
                assert!(asset.contains("PAR_MUX_PANE_ID"), "{name}");
                // Resume argv is per-agent: herdr's qwen/qodercli reports
                // carry none, so their ports do not invent one.
                let resumes = matches!(arm.agent, "droid" | "copilot" | "cursor");
                assert_eq!(
                    asset.contains("session_resume_argv"),
                    resumes,
                    "{name}: resume argv presence"
                );
            }
        }
        assert!(
            CURSOR.asset_windows.contains("Write-Output \"{}\""),
            "cursor's ps1 answers every exit with an empty hook response"
        );
    }

    impl ConfigHookArm {
        /// [`ConfigHookArm::installed`] against explicit paths.
        fn installed_at(&self, config: &Path, hook: &Path) -> bool {
            let content = fs::read_to_string(config).unwrap();
            self.registered(&content, config, hook).unwrap()
        }
    }

    /// One run of `arm`'s POSIX asset against a local listener with the pane
    /// env set: the reported (method, params), or `None` when the script
    /// refused.
    #[cfg(unix)]
    fn run_hook_request(
        arm: &ConfigHookArm,
        action: &str,
        payload: &str,
    ) -> Option<(String, JsonValue)> {
        use std::io::{BufRead as _, Write as _};
        use std::os::unix::net::UnixListener;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().expect("temp dir");
        let socket = dir.path().join("hook.sock");
        let listener = UnixListener::bind(&socket).expect("binds the test listener");
        let script = dir.path().join("hook.sh");
        fs::write(&script, arm.asset_posix).expect("writes the asset");
        let mut child = Command::new("sh")
            .arg(&script)
            .arg(action)
            .env("PAR_MUX_ENV", "1")
            .env("PAR_MUX_SOCKET", &socket)
            .env("PAR_MUX_PANE_ID", "pane-1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("runs the hook");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("feeds the payload");

        listener.set_nonblocking(true).expect("nonblocking accept");
        let started = Instant::now();
        let deadline = started + Duration::from_secs(10);
        let (stream, _) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if child.try_wait().ok().flatten().is_some()
                        && started.elapsed() > Duration::from_millis(300)
                    {
                        return None;
                    }
                    assert!(Instant::now() < deadline, "the hook never reported");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept failed: {err}"),
            }
        };
        // BSD/macOS accept() inherits the listener's O_NONBLOCK; a hook
        // that connected but has not written yet would read WouldBlock.
        stream.set_nonblocking(false).expect("blocking read");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("reads the report line");
        let _ = child.wait();
        let request: JsonValue = serde_json::from_str(line.trim()).expect("parses the report");
        let method = request["method"].as_str().unwrap_or_default().to_string();
        Some((method, request["params"].clone()))
    }

    /// [`run_hook_request`] for the session-reporting assets.
    #[cfg(unix)]
    fn run_hook(arm: &ConfigHookArm, action: &str, payload: &str) -> Option<JsonValue> {
        let (method, params) = run_hook_request(arm, action, payload)?;
        assert_eq!(method, "pane.report_agent_session");
        Some(params)
    }

    #[test]
    #[cfg(unix)]
    fn droid_asset_reports_its_session_and_refuses_other_events() {
        let params = run_hook(
            &DROID,
            "session",
            r#"{"hook_event_name":"SessionStart","session_id":"d-1","source":"startup"}"#,
        )
        .expect("a session start reports");
        assert_eq!(params["agent"], "droid");
        assert_eq!(params["pane_id"], "pane-1");
        assert_eq!(params["agent_session_id"], "d-1");
        assert_eq!(params["source"], "par-mux:droid");
        assert_eq!(
            params["session_resume_argv"],
            json!(["droid", "--resume", "d-1"])
        );
        assert_eq!(params["session_start_source"], "startup");

        assert!(run_hook(
            &DROID,
            "session",
            r#"{"hook_event_name":"Stop","session_id":"d-1"}"#
        )
        .is_none());
        assert!(run_hook(&DROID, "idle", r#"{"session_id":"d-1"}"#).is_none());
        assert!(run_hook(&DROID, "session", r#"{"hook_event_name":"SessionStart"}"#).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn copilot_asset_normalizes_the_event_and_rejects_other_payloads() {
        let params = run_hook(
            &COPILOT,
            "session",
            r#"{"hookEventName":"session_start","sessionId":"c-1"}"#,
        )
        .expect("a normalized session start reports");
        assert_eq!(params["agent"], "copilot");
        assert_eq!(params["agent_session_id"], "c-1");
        assert_eq!(
            params["session_resume_argv"],
            json!(["copilot", "--resume=c-1"])
        );

        // No event name: a bare session id reports, a prompt payload does not.
        assert!(run_hook(&COPILOT, "session", r#"{"session_id":"c-2"}"#).is_some());
        assert!(run_hook(&COPILOT, "session", r#"{"session_id":"c-2","prompt":"hi"}"#).is_none());
        assert!(run_hook(
            &COPILOT,
            "session",
            r#"{"hook_event_name":"PreToolUse","session_id":"c-2"}"#
        )
        .is_none());
    }

    #[test]
    #[cfg(unix)]
    fn cursor_asset_takes_the_conversation_id_and_skips_a_prefix() {
        let params = run_hook(
            &CURSOR,
            "session",
            r#"noise {"hook_event_name":"sessionStart","conversation_id":"k-1"}"#,
        )
        .expect("a session start reports");
        assert_eq!(params["agent"], "cursor");
        assert_eq!(params["agent_session_id"], "k-1");
        assert_eq!(
            params["session_resume_argv"],
            json!(["cursor-agent", "--resume", "k-1"])
        );

        assert!(run_hook(
            &CURSOR,
            "session",
            r#"{"hook_event_name":"stop","session_id":"k-1"}"#
        )
        .is_none());
    }

    #[test]
    #[cfg(unix)]
    fn kimi_asset_reports_session_and_states() {
        let (method, params) = run_hook_request(
            &KIMI,
            "session",
            r#"{"hook_event_name":"SessionStart","session_id":"km-1"}"#,
        )
        .expect("a session start reports");
        assert_eq!(method, "pane.report_agent_session");
        assert_eq!(params["agent"], "kimi");
        assert_eq!(params["agent_session_id"], "km-1");
        assert_eq!(params["session_start_source"], "startup");
        assert_eq!(params["source"], "par-mux:kimi");

        let (method, params) = run_hook_request(
            &KIMI,
            "blocked",
            r#"{"hook_event_name":"PermissionRequest","session_id":"km-1"}"#,
        )
        .expect("a state action reports");
        assert_eq!(method, "pane.report_agent");
        assert_eq!(params["state"], "blocked");
        assert_eq!(params["agent_session_id"], "km-1");

        // A session action without a session id refuses, as does an
        // unknown action.
        assert!(
            run_hook_request(&KIMI, "session", r#"{"hook_event_name":"SessionStart"}"#).is_none()
        );
        assert!(run_hook(&KIMI, "done", r#"{"session_id":"km-1"}"#).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn mastracode_asset_reports_session_and_states() {
        let (method, params) = run_hook_request(
            &MASTRACODE,
            "session",
            r#"{"hook_event_name":"SessionStart","session_id":"mc-1"}"#,
        )
        .expect("a session start reports");
        assert_eq!(method, "pane.report_agent_session");
        assert_eq!(params["agent"], "mastracode");
        assert_eq!(params["agent_session_id"], "mc-1");
        assert_eq!(params["session_start_source"], "startup");
        assert_eq!(params["source"], "par-mux:mastracode");

        let (method, params) = run_hook_request(
            &MASTRACODE,
            "working",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"mc-1"}"#,
        )
        .expect("a state action reports");
        assert_eq!(method, "pane.report_agent");
        assert_eq!(params["state"], "working");

        assert!(run_hook_request(&MASTRACODE, "session", r#"{}"#).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn devin_asset_reports_a_payload_session_id() {
        let params = run_hook(
            &DEVIN,
            "session",
            r#"{"hook_event_name":"SessionStart","sessionId":"dv-1"}"#,
        )
        .expect("a session id in the payload reports");
        assert_eq!(params["agent"], "devin");
        assert_eq!(params["agent_session_id"], "dv-1");
        assert_eq!(params["source"], "par-mux:devin");

        // No session id and no resolvable fallback (a startup SessionStart
        // never guesses): the script stays silent.
        assert!(run_hook(
            &DEVIN,
            "session",
            r#"{"hook_event_name":"SessionStart","source":"startup"}"#
        )
        .is_none());
    }
}
