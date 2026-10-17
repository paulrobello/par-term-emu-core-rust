//! muse arm: muse reads hooks from the `hooks` object of its user settings
//! (`$XDG_CONFIG_HOME/muse/settings.json`, else `~/.config/muse/…`), so
//! par-mux merges a SessionStart entry there — the claude arm's
//! merge-discipline engine — and writes the reporter script under par-mux's
//! own config directory. muse requires `schema_version: 1` at the top of
//! its settings, so a fresh file bootstraps with it; an existing file is
//! the user's (muse already validated it).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    has_command, home_dir, hook_asset_path, hook_command_with_action, merge_hook_event,
    remove_marked_file, save_settings, unmerge_hook_entries, write_hook_asset,
};

const MUSE_HOOK_ASSET_POSIX: &str =
    include_str!("../../../integrations/hooks/par-mux-muse-session-hook.sh");
const MUSE_HOOK_ASSET_WINDOWS: &str =
    include_str!("../../../integrations/hooks/par-mux-muse-session-hook.ps1");

/// The asset file name for the current platform — the `.ps1` port on
/// Windows, the `.sh` reporter elsewhere. Runtime-selected (not cfg-split)
/// so both arms compile on every platform and the selection is assertable
/// in tests instead of invisible to the non-Windows build.
fn muse_hook_install_name() -> &'static str {
    if cfg!(windows) {
        "par-mux-muse-session-hook.ps1"
    } else {
        "par-mux-muse-session-hook.sh"
    }
}

/// The hook asset contents for the current platform.
fn muse_hook_asset() -> &'static str {
    if cfg!(windows) {
        MUSE_HOOK_ASSET_WINDOWS
    } else {
        MUSE_HOOK_ASSET_POSIX
    }
}

/// Marker identifying the installed script as ours regardless of which
/// build wrote it — the uninstall path refuses to remove a same-named file
/// without one.
pub const MUSE_HOOK_MARKER: &str = "PAR_MUX_INTEGRATION_ID=muse";

/// Outcome of a muse hook install.
#[derive(Debug)]
pub struct MuseHookInstall {
    /// The settings file that was merged into.
    pub settings_path: PathBuf,
    /// Where the hook script asset was written.
    pub hook_path: PathBuf,
    /// Whether the settings file changed (false = our entry was already
    /// present; the asset is still refreshed).
    pub settings_changed: bool,
}

/// Outcome of a muse hook uninstall.
#[derive(Debug)]
pub struct MuseHookUninstall {
    /// The settings file our entry was removed from.
    pub settings_path: PathBuf,
    /// Where the hook script asset lived.
    pub hook_path: PathBuf,
    /// Whether the settings file changed (false = our entry was not there).
    pub settings_changed: bool,
    /// Whether the hook script asset was actually ours to delete.
    pub hook_removed: bool,
}

/// Resolve the muse settings path: `$XDG_CONFIG_HOME/muse/settings.json`
/// when that env is set (muse's XDG layout), else `~/.config/muse/…`.
pub fn muse_settings_path() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home_dir()?.join(".config"),
    };
    Ok(base.join("muse").join("settings.json"))
}

/// Where the hook script asset lives: par-mux's own config directory.
fn muse_asset_path() -> io::Result<PathBuf> {
    hook_asset_path(muse_hook_install_name())
}

/// The installed hook asset's currency against the bundled copy —
/// the version-aware half of the status surfaces.
pub(crate) fn hook_asset_state() -> crate::mux::integrations::AssetState {
    match muse_asset_path() {
        Ok(path) => crate::mux::integrations::AssetState::at(&path, muse_hook_asset()),
        Err(_) => crate::mux::integrations::AssetState::absent(muse_hook_asset()),
    }
}

/// Install the muse session hook: write the script asset and merge the
/// SessionStart entry into the user's settings.
pub fn install_muse_hook() -> io::Result<MuseHookInstall> {
    install_muse_hook_into(&muse_settings_path()?, &muse_asset_path()?)
}

/// Install with explicit paths (test/install seam).
pub fn install_muse_hook_into(
    settings_path: &Path,
    hook_path: &Path,
) -> io::Result<MuseHookInstall> {
    // The agent must be installed: its config directory has to exist. A
    // missing settings FILE bootstraps (with muse's required
    // schema_version), a missing DIRECTORY is an install-muse-first
    // condition, not a silent skip.
    let agent_dir = settings_path.parent().ok_or_else(|| {
        io::Error::other(format!(
            "muse settings path has no parent: {settings_path:?}"
        ))
    })?;
    if !agent_dir.is_dir() {
        return Err(io::Error::other(format!(
            "muse settings directory not found at {}. install muse cli first",
            agent_dir.display()
        )));
    }

    let content = match fs::read_to_string(settings_path) {
        Ok(content) => content,
        // muse rejects user settings without a top-level schema_version, so
        // a fresh file starts with one instead of an empty object.
        Err(err) if err.kind() == io::ErrorKind::NotFound => "{\"schema_version\":1}\n".to_string(),
        Err(err) => return Err(err),
    };

    // Merge and verify BEFORE writing anything: a malformed user config
    // must fail with the file — and the asset — untouched. The entry is
    // matcher-less: whether muse honors claude's matcher space is not
    // documented, and the reporter self-filters on hook_event_name.
    let command = hook_command_with_action(hook_path, "session");
    let merged = merge_hook_event(&content, settings_path, &command, None, "SessionStart")?;
    let settings_changed = merged.is_some();

    write_hook_asset(hook_path, muse_hook_asset())?;

    if settings_changed {
        save_settings(settings_path, merged.as_deref().unwrap_or(&content))?;
    }

    Ok(MuseHookInstall {
        settings_path: settings_path.to_path_buf(),
        hook_path: hook_path.to_path_buf(),
        settings_changed,
    })
}

/// Uninstall the muse session hook: remove our entry from settings and
/// delete the script asset when it is ours.
pub fn uninstall_muse_hook() -> io::Result<MuseHookUninstall> {
    uninstall_muse_hook_into(&muse_settings_path()?, &muse_asset_path()?)
}

/// Uninstall with explicit paths (test/install seam).
pub fn uninstall_muse_hook_into(
    settings_path: &Path,
    hook_path: &Path,
) -> io::Result<MuseHookUninstall> {
    let mut settings_changed = false;
    if settings_path.is_file() {
        let content = fs::read_to_string(settings_path)?;
        let command = hook_command_with_action(hook_path, "session");
        if let Some(updated) = unmerge_hook_entries(
            &content,
            settings_path,
            &[("SessionStart", command.as_str())],
        )? {
            save_settings(settings_path, &updated)?;
            settings_changed = true;
        }
    }

    let hook_removed = remove_marked_file(hook_path, MUSE_HOOK_MARKER)?;

    Ok(MuseHookUninstall {
        settings_path: settings_path.to_path_buf(),
        hook_path: hook_path.to_path_buf(),
        settings_changed,
        hook_removed,
    })
}

/// Whether muse's settings carry par-mux's SessionStart entry (read-only).
pub(crate) fn muse_hook_installed() -> bool {
    let Ok(settings_path) = muse_settings_path() else {
        return false;
    };
    let Ok(content) = fs::read_to_string(&settings_path) else {
        return false;
    };
    let Ok(hook_path) = muse_asset_path() else {
        return false;
    };
    let command = hook_command_with_action(&hook_path, "session");
    has_command(&content, &settings_path, &command, "SessionStart").unwrap_or(false)
}

/// Startup self-heal: rewrite the script asset when the settings file still
/// carries our entry but the script is gone — the hooks directory has
/// repeatedly been deleted out from under live registrations while the
/// entries in the agent's own config survived.
pub fn heal_muse_hook_asset() {
    let Ok(settings_path) = muse_settings_path() else {
        return; // no home to resolve against — nothing to heal
    };
    let Ok(hook_path) = muse_asset_path() else {
        return;
    };
    if let Err(err) = heal_muse_hook_asset_into(&settings_path, &hook_path) {
        log::warn!("muse mux-hook heal skipped: {err}");
    }
}

/// Heal with explicit paths (test seam). `Ok(())` in every skip case:
/// script present, agent settings absent, or our entry not registered. A
/// malformed settings file surfaces as an error with nothing written. The
/// settings file itself is never touched.
pub(crate) fn heal_muse_hook_asset_into(settings_path: &Path, hook_path: &Path) -> io::Result<()> {
    if hook_path.exists() {
        return Ok(());
    }
    let Ok(content) = fs::read_to_string(settings_path) else {
        return Ok(()); // agent not installed — nothing registers the script
    };
    let command = hook_command_with_action(hook_path, "session");
    if has_command(&content, settings_path, &command, "SessionStart")? {
        write_hook_asset(hook_path, muse_hook_asset())?;
        // The deleter is unidentified; this line dates any recurrence in the debug log.
        log::info!("mux hook heal: restored {}", hook_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;
    use std::fs;

    use super::super::parse_serde;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn muse_install_bootstraps_a_fresh_settings_file_with_schema_version() {
        let root = temp_root();
        let muse_dir = root.path().join("config").join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let hook = root
            .path()
            .join("par-mux-hooks")
            .join("par-mux-muse-session-hook.sh");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();

        let result = install_muse_hook_into(&settings, &hook).unwrap();

        assert!(result.settings_changed);
        let parsed: JsonValue =
            serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(parsed["schema_version"], 1, "muse requires the field");
        let entry = &parsed["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["timeout"], super::super::HOOK_TIMEOUT_SECS);
        let command = entry["command"].as_str().unwrap();
        let runner = if cfg!(windows) { "powershell " } else { "sh " };
        assert!(command.starts_with(runner) && command.ends_with(" session"));
        assert!(command.contains(muse_hook_install_name()));
        assert!(
            parsed["hooks"]["SessionStart"][0].get("matcher").is_none(),
            "matcher-less: the reporter self-filters on hook_event_name"
        );

        // The asset is our script, verbatim, and executable.
        assert_eq!(fs::read_to_string(&hook).unwrap(), muse_hook_asset());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&hook).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "the hook must be executable");
        }
    }

    #[test]
    fn muse_install_merges_into_a_user_settings_file_without_touching_it() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let original = concat!(
            "{\n",
            "  // user's own settings\n",
            "  \"schema_version\": 1,\n",
            "  \"model\": \"muse-spark-1.2\",\n",
            "  \"hooks\": {\n",
            "    // existing user hooks\n",
            "    \"Stop\": [\n",
            "      { \"hooks\": [ { \"type\": \"command\", \"command\": \"user-notify\" } ] }\n",
            "    ]\n",
            "  }\n",
            "}\n"
        );
        fs::write(&settings, original).unwrap();
        let hook = root.path().join("hook.sh");

        let result = install_muse_hook_into(&settings, &hook).unwrap();
        assert!(result.settings_changed);

        // jsonc, not serde_json: the user's comments survive the merge and
        // must still parse.
        let parsed = parse_serde(&fs::read_to_string(&settings).unwrap(), &settings).unwrap();
        assert_eq!(parsed["model"], "muse-spark-1.2");
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            "user-notify"
        );
        assert_eq!(
            parsed["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .last(),
            Some("session")
        );
    }

    #[test]
    fn muse_reinstall_is_a_no_op_on_settings_but_refreshes_the_asset() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let hook = root.path().join("hook.sh");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();

        install_muse_hook_into(&settings, &hook).unwrap();
        let after_first = fs::read_to_string(&settings).unwrap();
        fs::write(&hook, "# user edit").unwrap();

        let second = install_muse_hook_into(&settings, &hook).unwrap();

        assert!(!second.settings_changed, "the entry is already present");
        assert_eq!(fs::read_to_string(&settings).unwrap(), after_first);
        assert_eq!(fs::read_to_string(&hook).unwrap(), muse_hook_asset());
    }

    #[test]
    fn muse_uninstall_removes_the_entry_and_the_marked_asset() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let hook = root.path().join("hook.sh");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();
        install_muse_hook_into(&settings, &hook).unwrap();

        let result = uninstall_muse_hook_into(&settings, &hook).unwrap();

        assert!(result.settings_changed && result.hook_removed);
        let parsed: JsonValue =
            serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(
            parsed.get("hooks").is_none() || parsed["hooks"].get("SessionStart").is_none(),
            "our entry is gone: {parsed}"
        );
        assert!(!hook.exists());

        // Idempotent.
        let again = uninstall_muse_hook_into(&settings, &hook).unwrap();
        assert!(!again.settings_changed && !again.hook_removed);
    }

    #[test]
    fn muse_uninstall_restores_a_previously_merged_file_byte_for_byte() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let original = "{\n  \"schema_version\": 1,\n  \"model\": \"muse-spark-1.2\"\n}\n";
        fs::write(&settings, original).unwrap();
        let hook = root.path().join("hook.sh");

        install_muse_hook_into(&settings, &hook).unwrap();
        assert!(
            uninstall_muse_hook_into(&settings, &hook)
                .unwrap()
                .settings_changed
        );

        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            original,
            "the splice machinery must restore the user's bytes exactly"
        );
    }

    #[test]
    fn muse_missing_settings_directory_is_an_actionable_error() {
        let root = temp_root();
        let settings = root.path().join("never").join("settings.json");
        let hook = root.path().join("hook.sh");

        let err = install_muse_hook_into(&settings, &hook)
            .unwrap_err()
            .to_string();

        assert!(
            err.contains("muse settings directory not found at"),
            "error must name the path: {err}"
        );
        assert!(
            err.contains("install muse cli first"),
            "error must say what to do: {err}"
        );
    }

    #[test]
    fn muse_malformed_user_settings_fail_with_nothing_written() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        fs::write(&settings, "{\"schema_version\":1,} duplicate").unwrap();
        let hook = root.path().join("hook.sh");

        assert!(install_muse_hook_into(&settings, &hook).is_err());

        assert!(
            !hook.exists(),
            "the asset must not be written when the merge fails"
        );
    }

    #[test]
    fn muse_heal_restores_a_missing_asset_only_when_registered() {
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        let hook = root.path().join("hook.sh");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();

        // Not installed: heal is a silent no-op.
        heal_muse_hook_asset_into(&settings, &hook).unwrap();
        assert!(!hook.exists());

        install_muse_hook_into(&settings, &hook).unwrap();
        fs::remove_file(&hook).unwrap();
        heal_muse_hook_asset_into(&settings, &hook).unwrap();
        assert_eq!(fs::read_to_string(&hook).unwrap(), muse_hook_asset());
    }

    #[test]
    fn muse_hook_installed_reads_the_real_settings_only() {
        // The read-only path must resolve through the same XDG rule and
        // return false everywhere when nothing is installed.
        // (The env-dependent real path is covered by the mux_cli suite;
        // here only the local seam is pinned.)
        let root = temp_root();
        let settings = root.path().join("muse").join("settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let hook = root.path().join("hook.sh");
        install_muse_hook_into(&settings, &hook).unwrap();
        let command = hook_command_with_action(&hook, "session");
        let content = fs::read_to_string(&settings).unwrap();
        assert!(has_command(&content, &settings, &command, "SessionStart").unwrap());
    }

    #[test]
    fn the_windows_ps1_asset_is_a_marked_reporter_port() {
        // The uninstall marker and the reporter contract, checked against the
        // asset contents directly so the .ps1 variant cannot drift from the
        // .sh reporter shape it ports (the file is never installed on this
        // platform, so nothing else would catch it).
        assert!(MUSE_HOOK_ASSET_WINDOWS.contains(MUSE_HOOK_MARKER));
        assert!(
            MUSE_HOOK_ASSET_WINDOWS.contains("pane.report_agent_session_cwd"),
            "speaks the cwd-resolved control-socket method"
        );
        assert!(MUSE_HOOK_ASSET_WINDOWS.contains("par-mux:muse"));
        assert!(MUSE_HOOK_ASSET_WINDOWS.contains("MUSE_SESSION_ID"));
        assert!(
            MUSE_HOOK_ASSET_WINDOWS.contains("par-mux-*.sock"),
            "discovers daemon marker files"
        );
        assert!(
            !MUSE_HOOK_ASSET_WINDOWS.contains("PAR_MUX_PANE_ID"),
            "muse clears the hook env: no pane id ever arrives"
        );
    }

    #[test]
    fn the_posix_asset_discovers_sockets_and_reports_by_cwd() {
        assert!(MUSE_HOOK_ASSET_POSIX.contains(MUSE_HOOK_MARKER));
        assert!(MUSE_HOOK_ASSET_POSIX.contains("pane.report_agent_session_cwd"));
        assert!(MUSE_HOOK_ASSET_POSIX.contains("MUSE_SESSION_ID"));
        assert!(MUSE_HOOK_ASSET_POSIX.contains("par-mux-*.sock"));
        assert!(MUSE_HOOK_ASSET_POSIX.contains("par-mux-{uid}"));
        assert!(
            !MUSE_HOOK_ASSET_POSIX.contains("$PAR_MUX_PANE_ID"),
            "no pane-id env guard: muse clears the hook env, it never arrives"
        );
        // The version marker parses through the status surfaces.
        assert_eq!(
            super::super::parse_integration_version(MUSE_HOOK_ASSET_POSIX),
            Some("1".to_string())
        );
        assert_eq!(
            super::super::parse_integration_version(MUSE_HOOK_ASSET_WINDOWS),
            Some("1".to_string())
        );
    }

    #[test]
    fn muse_parse_round_trip_through_the_engine_helpers() {
        // A settings file with comments and trailing commas — jsonc in
        // practice — merges and parses back.
        let root = temp_root();
        let muse_dir = root.path().join("muse");
        fs::create_dir_all(&muse_dir).unwrap();
        let settings = muse_dir.join("settings.json");
        fs::write(
            &settings,
            "{\n  // muse settings\n  \"schema_version\": 1\n}\n",
        )
        .unwrap();
        let hook = root.path().join("hook.sh");

        install_muse_hook_into(&settings, &hook).unwrap();

        let content = fs::read_to_string(&settings).unwrap();
        let parsed = parse_serde(&content, &settings).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert!(parsed["hooks"]["SessionStart"].is_array());
    }
}
