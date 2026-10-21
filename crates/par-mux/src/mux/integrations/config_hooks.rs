//! droid, copilot, and cursor arms: session hooks whose registration is an
//! entry inside one config file the agent reads, ported from herdr's
//! integrations (Apache-2.0) onto par-mux's merge discipline. They differ
//! only in data, so one table drives all three:
//!
//! - droid: `~/.factory/settings.json`, claude-shape nested groups under
//!   `hooks.SessionStart` (`{hooks: [{type, command, timeout}]}`).
//! - copilot: `$COPILOT_HOME/settings.json` (else `~/.copilot`), flat
//!   entries under `hooks.SessionStart` —
//!   `{type: "command", bash|powershell: <cmd>, timeoutSec}`.
//! - cursor: `$CURSOR_CONFIG_DIR/hooks.json` (else `~/.cursor`), bare
//!   `{command}` entries under the camelCase `hooks.sessionStart`, in a file
//!   that must carry a top-level `version` (herdr adds `1` when missing).
//!
//! Every entry runs the same `<runner> '<asset>' session` command, and every
//! asset lives under par-mux's own config directory, as claude, codex, and
//! muse do.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value as JsonValue};

use super::{
    ensure_root_key, has_command, has_flat_entry, home_dir, hook_asset_path,
    hook_command_with_action, merge_flat_entry, merge_hook_event, remove_marked_file,
    save_settings, unmerge_flat_entries, unmerge_hook_entries, write_hook_asset, HOOK_TIMEOUT_SECS,
};

/// How an arm's config file spells one hook entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryShape {
    /// `{hooks: [{type: "command", command, timeout}]}` (droid).
    NestedGroup,
    /// `{type: "command", bash|powershell: <cmd>, timeoutSec}` (copilot).
    CopilotFlat,
    /// `{command}` (cursor).
    CursorFlat,
}

/// One config-entry agent: where its config lives, how an entry looks,
/// and the reporter assets it runs.
#[derive(Debug)]
pub struct ConfigHookArm {
    /// The agent label the reporter sends (and the status surfaces name).
    pub agent: &'static str,
    config_dir: fn() -> io::Result<PathBuf>,
    config_file: &'static str,
    event: &'static str,
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
    event: "SessionStart",
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
    event: "SessionStart",
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
    event: "sessionStart",
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
    event: "SessionStart",
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
    event: "SessionStart",
    shape: EntryShape::NestedGroup,
    bootstrap: "{}\n",
    required_root_key: None,
    install_hint: "install qoder cli first",
    asset_posix: include_str!("../../../integrations/hooks/par-mux-qodercli-session-hook.sh"),
    asset_windows: include_str!("../../../integrations/hooks/par-mux-qodercli-session-hook.ps1"),
};

/// The arms, in the order the status surfaces list them.
pub static CONFIG_HOOK_ARMS: [&ConfigHookArm; 5] = [&DROID, &COPILOT, &CURSOR, &QWEN, &QODERCLI];

const COPILOT_HOME_ENV_VAR: &str = "COPILOT_HOME";
const CURSOR_CONFIG_DIR_ENV_VAR: &str = "CURSOR_CONFIG_DIR";
const QWEN_HOME_ENV_VAR: &str = "QWEN_HOME";
const QODERCLI_CONFIG_DIR_ENV_VAR: &str = "QODERCLI_CONFIG_DIR";

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
            EntryShape::NestedGroup => super::canonical_entry_value(None, command),
            EntryShape::CopilotFlat => {
                let field = if windows { "powershell" } else { "bash" };
                json!({ "type": "command", field: command, "timeoutSec": HOOK_TIMEOUT_SECS })
            }
            EntryShape::CursorFlat => json!({ "command": command }),
        }
    }

    /// Whether a flat entry is ours: its command string, under whichever
    /// key the shape allows, is exactly `command`.
    fn flat_entry_is_ours(&self, entry: &JsonValue, command: &str) -> bool {
        let text = |key: &str| entry.get(key).and_then(JsonValue::as_str) == Some(command);
        match self.shape {
            EntryShape::NestedGroup => false,
            EntryShape::CopilotFlat => {
                entry.get("type").and_then(JsonValue::as_str) == Some("command")
                    && (text("bash") || text("powershell") || text("command"))
            }
            EntryShape::CursorFlat => text("command"),
        }
    }

    /// Whether `content` already registers `command`.
    fn registered(&self, content: &str, config_path: &Path, command: &str) -> io::Result<bool> {
        match self.shape {
            EntryShape::NestedGroup => has_command(content, config_path, command, self.event),
            _ => has_flat_entry(content, config_path, self.event, &|entry| {
                self.flat_entry_is_ours(entry, command)
            }),
        }
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
        let command = hook_command_with_action(&hook_path, "session");
        self.registered(&content, &config_path, &command)
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
        let command = hook_command_with_action(hook_path, "session");
        let merged = match self.shape {
            EntryShape::NestedGroup => {
                merge_hook_event(&content, config_path, &command, None, self.event)?
            }
            _ => merge_flat_entry(
                &content,
                config_path,
                self.event,
                &self.entry(&command, windows),
                &|entry| self.flat_entry_is_ours(entry, &command),
            )?,
        };
        if let Some(updated) = merged {
            content = updated;
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
            let content = fs::read_to_string(config_path)?;
            let command = hook_command_with_action(hook_path, "session");
            let removed = match self.shape {
                EntryShape::NestedGroup => {
                    unmerge_hook_entries(&content, config_path, &[(self.event, command.as_str())])?
                }
                _ => unmerge_flat_entries(&content, config_path, self.event, &|entry| {
                    self.flat_entry_is_ours(entry, &command)
                })?,
            };
            if let Some(updated) = removed {
                save_settings(config_path, &updated)?;
                config_changed = true;
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
        let command = hook_command_with_action(hook_path, "session");
        if self.registered(&content, config_path, &command)? {
            write_hook_asset(hook_path, self.asset())?;
            log::info!("mux hook heal: restored {}", hook_path.display());
        }
        Ok(())
    }
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

    /// Our registered entries under the arm's event, as parsed JSON.
    fn event_entries(arm: &ConfigHookArm, config: &Path) -> Vec<JsonValue> {
        let parsed = parse_serde(&fs::read_to_string(config).unwrap(), config).unwrap();
        parsed["hooks"][arm.event]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn droid_install_bootstraps_a_nested_session_start_group() {
        let (_root, config, hook) = stage(&DROID);

        let result = DROID.install_into(&config, &hook).unwrap();

        assert!(result.config_changed);
        let entries = event_entries(&DROID, &config);
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

        let entries = event_entries(&COPILOT, &config);
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
        let entries = event_entries(&CURSOR, &config);
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
        assert_eq!(event_entries(&CURSOR, &config).len(), 1);
    }

    /// The user's own file, with comments and their own entries, round
    /// trips byte-exact through install then uninstall for every arm.
    #[test]
    fn every_arm_restores_a_user_config_byte_for_byte() {
        let user_files: [(&ConfigHookArm, &str); 3] = [
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
            let parsed = parse_serde(&fs::read_to_string(&config).unwrap(), &config).unwrap();
            assert!(
                parsed.get("hooks").is_none(),
                "{}: hooks pruned: {parsed}",
                arm.agent
            );
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
            let command = hook_command_with_action(hook, "session");
            self.registered(&content, config, &command).unwrap()
        }
    }

    /// One run of `arm`'s POSIX asset against a local listener with the pane
    /// env set: the reported params, or `None` when the script refused.
    #[cfg(unix)]
    fn run_hook(arm: &ConfigHookArm, action: &str, payload: &str) -> Option<JsonValue> {
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
        assert_eq!(request["method"], "pane.report_agent_session");
        Some(request["params"].clone())
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
}
