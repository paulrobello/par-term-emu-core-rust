//! claude mod arm: the agent-state hooks module (a Claude Code "mod" — the
//! plugin API), the state-truth complement to the `.sh` session reporter.
//!
//! The session hook owns identity and release (`pane.report_agent_session`,
//! `pane.release_agent`); the mod owns live state: question while an
//! AskUserQuestion dialog is up (cleared the moment the user answers),
//! parked-working while `Stop` reports in-flight background tasks or
//! scheduled session crons that will wake the session, idle otherwise.
//! Reports ride the control CLI — one bounded argv, no shell:
//! `par-mux --cmd "report-state -t <pane> -s <state> [-m '<msg>']"` with
//! `$PAR_MUX_SOCKET` deciding the daemon, so the wire shape is the same
//! `report-state` verb the .sh reporter's `pane.report_agent` uses.
//!
//! Install = write the mod files under par-mux's own config directory
//! (`<config>/mods/agent-state/`) and merge the folder into claude's
//! `env.CLAUDE_CODE_PLUGIN_DIRS` (the settings.json `env` block is one of
//! the two supported sources, so sessions launched after see the mod). The
//! merge follows the engine's discipline: parse and verify before any
//! write, splice at AST offsets so untouched bytes never move, reinstall is
//! a byte-exact no-op, and uninstall removes only our path segment — a
//! user's own plugin dirs survive.
//!
//! Fresh sessions read `env.CLAUDE_CODE_PLUGIN_DIRS` from settings; a
//! session already running when the mod installs sees it on restart. The
//! folder is hot-reload-watched, so asset updates land without a restart.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jsonc_parser::ast::Value as AstValue;
use jsonc_parser::common::Ranged;

use super::{
    append_object_property, apply_cuts, parse_root_object, parse_serde, prop_name_text,
    property_cut, reject_duplicate_keys, save_settings, string_value, verify_update, AssetState,
};

const MOD_NAME: &str = "agent-state";
const PLUGIN_DIRS_KEY: &str = "CLAUDE_CODE_PLUGIN_DIRS";

const MOD_PLUGIN_JSON: &str =
    include_str!("../../../integrations/mods/agent-state/.claude-plugin/plugin.json");
const MOD_HOOKS_JSON: &str =
    include_str!("../../../integrations/mods/agent-state/hooks/hooks.json");
const MOD_REGISTER: &str =
    include_str!("../../../integrations/mods/agent-state/hooks/register.tsx");
const MOD_TYPES: &str = include_str!("../../../integrations/mods/agent-state/types/index.d.ts");

/// Marker identifying the installed register.tsx as ours — the uninstall
/// path refuses to remove a folder whose register.tsx lacks one, exactly
/// like the script arms' file markers. Also the asset the status surfaces
/// byte-grade, so it carries the version marker too.
pub const CLAUDE_MOD_MARKER: &str = "PAR_MUX_INTEGRATION_ID=agent-state";

/// Where the mod folder lives: `<par-mux config>/mods/agent-state`, beside
/// the hooks directory. Derived from the hook asset path so the existing
/// `_into` test seam stays the single seam. The config root is the hook's
/// parent directory, one level up when that directory is named `hooks` —
/// walking two levels unconditionally would escape a root-level test hook's
/// tempdir to the shared TMPDIR, which is exactly the cross-test race this
/// fix removed (measured: parallel runs failed with ENOTEMPTY on a single
/// shared `$TMPDIR/mods/agent-state`, serial runs never did).
pub(crate) fn mod_dir_for(hook_path: &Path) -> PathBuf {
    let hooks_dir = hook_path.parent().unwrap_or_else(|| Path::new("."));
    let config = match hooks_dir.file_name() {
        Some(name) if name == "hooks" => hooks_dir.parent().unwrap_or(hooks_dir),
        _ => hooks_dir,
    };
    config.join("mods").join(MOD_NAME)
}

fn write_mod_files(mod_dir: &Path) -> io::Result<()> {
    for (rel, contents) in [
        (".claude-plugin/plugin.json", MOD_PLUGIN_JSON),
        ("hooks/hooks.json", MOD_HOOKS_JSON),
        ("hooks/register.tsx", MOD_REGISTER),
        ("types/index.d.ts", MOD_TYPES),
    ] {
        let path = mod_dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
    }
    Ok(())
}

/// Whether `content`'s `env.CLAUDE_CODE_PLUGIN_DIRS` names `dir`.
fn dirs_contains(content: &str, settings_path: &Path, dir: &str) -> io::Result<bool> {
    let value = parse_serde(content, settings_path)?;
    Ok(plugin_dirs_value(&value).is_some_and(|dirs| dirs.split(':').any(|entry| entry == dir)))
}

/// The `env.CLAUDE_CODE_PLUGIN_DIRS` string value, or None when env or key
/// is absent or not a string.
fn plugin_dirs_value(value: &serde_json::Value) -> Option<&str> {
    value
        .get("env")
        .and_then(|env| env.get(PLUGIN_DIRS_KEY))
        .and_then(serde_json::Value::as_str)
}

/// Merge `dir` into `content`'s `env.CLAUDE_CODE_PLUGIN_DIRS`.
/// `Ok(None)` = already present, byte-exact no-op. Refuses a non-string
/// value; splices at AST offsets so untouched bytes never move.
fn merge_plugin_dirs(content: &str, settings_path: &Path, dir: &str) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if dirs_contains(content, settings_path, dir)? {
        return Ok(None);
    }

    let updated = match root.get_object("env") {
        None => append_object_property(
            content,
            &root,
            "env",
            &format!("{{\"{PLUGIN_DIRS_KEY}\":\"{dir}\"}}"),
        ),
        Some(env_object) => match env_object
            .properties
            .iter()
            .find(|property| prop_name_text(&property.name) == Some(PLUGIN_DIRS_KEY))
        {
            None => {
                append_object_property(content, env_object, PLUGIN_DIRS_KEY, &format!("\"{dir}\""))
            }
            Some(property) => {
                let AstValue::StringLit(_) = property.value else {
                    return Err(io::Error::other(format!(
                        "{PLUGIN_DIRS_KEY} at {} must be a string",
                        settings_path.display()
                    )));
                };
                let existing = string_value(&property.value).unwrap_or_default();
                let mut entries: Vec<&str> =
                    existing.split(':').filter(|e| !e.is_empty()).collect();
                entries.push(dir);
                let joined = entries.join(":");
                let range = property.value.range();
                let escaped = serde_json::to_string(&joined).expect("joined dirs are serializable");
                let mut text = String::with_capacity(content.len() + escaped.len());
                text.push_str(&content[..range.start]);
                text.push_str(&escaped);
                text.push_str(&content[range.end..]);
                text
            }
        },
    };

    let mut desired = parse_serde(content, settings_path)?;
    if let Some(env) = desired
        .as_object_mut()
        .and_then(|root| root.get_mut("env"))
        .and_then(serde_json::Value::as_object_mut)
    {
        match env.get(PLUGIN_DIRS_KEY).and_then(serde_json::Value::as_str) {
            Some(existing) => {
                let mut entries: Vec<&str> =
                    existing.split(':').filter(|e| !e.is_empty()).collect();
                entries.push(dir);
                env.insert(
                    PLUGIN_DIRS_KEY.to_string(),
                    serde_json::Value::String(entries.join(":")),
                );
            }
            None => {
                env.insert(
                    PLUGIN_DIRS_KEY.to_string(),
                    serde_json::Value::String(dir.to_string()),
                );
            }
        }
    } else if let Some(root_object) = desired.as_object_mut() {
        root_object.insert(
            "env".to_string(),
            serde_json::json!({ PLUGIN_DIRS_KEY: dir }),
        );
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

/// Install: write the mod files, then merge the folder into the settings'
/// `env.CLAUDE_CODE_PLUGIN_DIRS` (settings saved only when the merge
/// changed them). Settings text is verified before anything is written.
pub fn install_claude_mod_into(settings_path: &Path, mod_dir: &Path) -> io::Result<bool> {
    let content = match fs::read_to_string(settings_path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => "{}\n".to_string(),
        Err(err) => return Err(err),
    };
    let merged = merge_plugin_dirs(&content, settings_path, &mod_dir.display().to_string())?;

    write_mod_files(mod_dir)?;
    if let Some(updated) = merged {
        save_settings(settings_path, &updated)?;
        return Ok(true);
    }
    Ok(false)
}

/// Install with the real paths resolved (`~/.claude/settings.json` and
/// `<par-mux config>/mods/agent-state`).
pub fn install_claude_mod() -> io::Result<bool> {
    install_claude_mod_into(&super::claude_settings_path()?, &claude_mod_dir()?)
}

/// Outcome of a claude mod uninstall.
#[derive(Debug)]
pub struct ClaudeModUninstall {
    /// The settings file our env entry was removed from.
    pub settings_path: PathBuf,
    /// The mod folder removed (marker-checked), if any.
    pub mod_dir: PathBuf,
    /// Whether the settings changed (false = our entry was not there).
    pub settings_changed: bool,
    /// Whether the mod folder was actually ours to delete.
    pub mod_removed: bool,
}

/// Remove `dir` from `content`'s `env.CLAUDE_CODE_PLUGIN_DIRS`.
/// `Ok(None)` = not present. Empties the key when our removal leaves no
/// other paths, prunes `env` when our removals emptied it.
fn unmerge_plugin_dirs(
    content: &str,
    settings_path: &Path,
    dir: &str,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if !dirs_contains(content, settings_path, dir)? {
        return Ok(None);
    }

    let env_object = root
        .get_object("env")
        .ok_or_else(|| io::Error::other("env vanished between parse and use"))?;
    let property = env_object
        .properties
        .iter()
        .find(|property| prop_name_text(&property.name) == Some(PLUGIN_DIRS_KEY))
        .ok_or_else(|| io::Error::other("plugin dirs key vanished between parse and use"))?;
    let AstValue::StringLit(_) = property.value else {
        return Err(io::Error::other(format!(
            "{PLUGIN_DIRS_KEY} at {} must be a string",
            settings_path.display()
        )));
    };
    let existing = string_value(&property.value).unwrap_or_default();
    let remaining: Vec<&str> = existing
        .split(':')
        .filter(|entry| !entry.is_empty() && *entry != dir)
        .collect();

    let updated = if remaining.is_empty() {
        // Cut the whole key, then prune env when our cut emptied it — each
        // pass re-parsed, same as the hook-entry uninstall.
        let (cut, _) = apply_cuts(content, |text| {
            let root = parse_root_object(text, settings_path)?;
            let Some(env) = root.get_object("env") else {
                return Ok(None);
            };
            // property_cut answers a zero range for a missing property, and
            // a zero cut marks apply_cuts changed without removing a byte —
            // so the key's presence is checked here, or the pass never ends.
            let present = env
                .properties
                .iter()
                .any(|prop| prop_name_text(&prop.name) == Some(PLUGIN_DIRS_KEY));
            if !present {
                return Ok(None);
            }
            Ok(Some(property_cut(&env.properties, PLUGIN_DIRS_KEY)))
        })?;
        let (cut, _) = apply_cuts(&cut, |text| {
            let root = parse_root_object(text, settings_path)?;
            let empty = root
                .get_object("env")
                .is_some_and(|env| env.properties.is_empty());
            Ok(empty.then(|| property_cut(&root.properties, "env")))
        })?;
        cut
    } else {
        let joined = remaining.join(":");
        let escaped = serde_json::to_string(&joined).expect("joined dirs are serializable");
        let range = property.value.range();
        let mut text = String::with_capacity(content.len());
        text.push_str(&content[..range.start]);
        text.push_str(&escaped);
        text.push_str(&content[range.end..]);
        text
    };

    let mut desired = parse_serde(content, settings_path)?;
    if let Some(env) = desired
        .as_object_mut()
        .and_then(|root| root.get_mut("env"))
        .and_then(serde_json::Value::as_object_mut)
    {
        if remaining.is_empty() {
            env.remove(PLUGIN_DIRS_KEY);
        } else {
            env.insert(
                PLUGIN_DIRS_KEY.to_string(),
                serde_json::Value::String(remaining.join(":")),
            );
        }
        let now_empty = desired
            .get("env")
            .and_then(serde_json::Value::as_object)
            .is_some_and(serde_json::Map::is_empty);
        if now_empty {
            desired
                .as_object_mut()
                .ok_or_else(|| io::Error::other("root must be an object"))?
                .remove("env");
        }
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

/// Uninstall: remove our env entry from the settings (only our path
/// segment; a user's own plugin dirs survive), then delete the mod folder
/// only when its register.tsx carries our marker.
pub fn uninstall_claude_mod_into(
    settings_path: &Path,
    mod_dir: &Path,
) -> io::Result<ClaudeModUninstall> {
    let mut settings_changed = false;
    if settings_path.is_file() {
        let content = fs::read_to_string(settings_path)?;
        if let Some(updated) =
            unmerge_plugin_dirs(&content, settings_path, &mod_dir.display().to_string())?
        {
            save_settings(settings_path, &updated)?;
            settings_changed = true;
        }
    }

    let register_path = mod_dir.join("hooks").join("register.tsx");
    let mod_removed = register_path.is_file()
        && fs::read_to_string(&register_path)?.contains(CLAUDE_MOD_MARKER)
        && {
            fs::remove_dir_all(mod_dir)?;
            true
        };

    Ok(ClaudeModUninstall {
        settings_path: settings_path.to_path_buf(),
        mod_dir: mod_dir.to_path_buf(),
        settings_changed,
        mod_removed,
    })
}

/// Startup self-heal: rewrite the mod files when the settings env entry
/// survived but the folder did not (or drifted from the bundled copy) —
/// the same external-deletion pattern the script assets healed twice.
pub fn heal_claude_mod() {
    let (Ok(settings_path), Ok(mod_dir)) = (claude_settings_path(), claude_mod_dir()) else {
        return;
    };
    if let Err(err) = heal_claude_mod_into(&settings_path, &mod_dir) {
        log::warn!("claude agent-state mod heal skipped: {err}");
    }
}

/// Heal with explicit paths (test seam). `Ok(())` in every skip case: the
/// folder current, our env entry not registered, or the settings absent.
pub(crate) fn heal_claude_mod_into(settings_path: &Path, mod_dir: &Path) -> io::Result<()> {
    if fs::read_to_string(mod_dir.join("hooks").join("register.tsx"))
        .map(|content| content == MOD_REGISTER)
        .unwrap_or(false)
    {
        return Ok(()); // folder present and current
    }
    let Ok(content) = fs::read_to_string(settings_path) else {
        return Ok(()); // agent not installed — nothing registers the folder
    };
    if dirs_contains(&content, settings_path, &mod_dir.display().to_string())? {
        write_mod_files(mod_dir)?;
        log::info!("claude mod heal: restored {}", mod_dir.display());
    }
    Ok(())
}

/// The real settings path (re-exported here so the mod arm's public API
/// stays one import).
pub(crate) fn claude_settings_path() -> io::Result<PathBuf> {
    super::claude_settings_path()
}

/// The real mod folder: `<par-mux config>/mods/agent-state`.
pub(crate) fn claude_mod_dir() -> io::Result<PathBuf> {
    let hook_path = super::hook_asset_path("par-mux-claude-session-hook.sh");
    hook_path.map(|path| mod_dir_for(&path))
}

/// Whether the settings' `env.CLAUDE_CODE_PLUGIN_DIRS` names our folder
/// (read-only).
pub(crate) fn claude_mod_installed() -> bool {
    let (Ok(settings_path), Ok(mod_dir)) = (claude_settings_path(), claude_mod_dir()) else {
        return false;
    };
    let Ok(content) = fs::read_to_string(&settings_path) else {
        return false;
    };
    dirs_contains(&content, &settings_path, &mod_dir.display().to_string()).unwrap_or(false)
}

/// The installed register.tsx's currency against the bundled copy — the
/// version-aware half of the status surfaces.
pub(crate) fn claude_mod_asset_state() -> AssetState {
    match claude_mod_dir() {
        Ok(dir) => AssetState::at(&dir.join("hooks").join("register.tsx"), MOD_REGISTER),
        Err(_) => AssetState::absent(MOD_REGISTER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// A user settings file with its own env block and an existing
    /// CLAUDE_CODE_PLUGIN_DIRS value naming a directory we must preserve.
    const USER_SETTINGS: &str = r#"{
  // personal settings
  "model": "opus",
  "env": {
    // user's own plugin dirs, and a var of their own
    "CLAUDE_CODE_PLUGIN_DIRS": "/opt/my-mods",
    "MY_FLAG": "1"
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "/usr/local/bin/lint-hook" }]
      }
    ]
  }
}
"#;

    fn read(p: &Path) -> String {
        fs::read_to_string(p).expect("reads")
    }

    #[test]
    fn install_writes_mod_files_and_merges_env_dir() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");

        let changed = install_claude_mod_into(&settings, &mod_dir).expect("installs");

        assert!(changed, "a fresh settings file is a change");
        for rel in [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/register.tsx",
            "types/index.d.ts",
        ] {
            let path = mod_dir.join(rel);
            assert!(path.is_file(), "{rel} written");
        }
        let register = read(&mod_dir.join("hooks").join("register.tsx"));
        assert!(register.contains(CLAUDE_MOD_MARKER));
        assert!(register.contains("report-state"));

        let content = read(&settings);
        assert!(dirs_contains(&content, &settings, &mod_dir.display().to_string()).unwrap());
    }

    #[test]
    fn install_appends_to_an_existing_dirs_value_and_preserves_the_rest() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        fs::write(&settings, USER_SETTINGS).expect("writes user settings");

        install_claude_mod_into(&settings, &mod_dir).expect("installs");

        let updated = read(&settings);
        for line in USER_SETTINGS.lines() {
            // The dirs value is the one line the install necessarily
            // rewrites (our path appended inside it); everything else
            // survives byte-exact.
            if line.contains(PLUGIN_DIRS_KEY) {
                continue;
            }
            assert!(updated.contains(line), "user line survives: {line}");
        }
        let dirs = plugin_dirs_value(&parse_serde(&updated, &settings).expect("parses"))
            .expect("dirs present")
            .to_string();
        let entries: Vec<&str> = dirs.split(':').collect();
        assert_eq!(
            entries,
            vec!["/opt/my-mods", mod_dir.display().to_string().as_str()],
            "our path appended, the user's kept first"
        );
    }

    #[test]
    fn reinstall_is_a_byte_exact_noop() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");

        install_claude_mod_into(&settings, &mod_dir).expect("first install");
        let before = read(&settings);
        let changed = install_claude_mod_into(&settings, &mod_dir).expect("second install");

        assert!(!changed, "already present is a no-op");
        assert_eq!(read(&settings), before, "settings untouched");
    }

    #[test]
    fn uninstall_keeps_user_dirs_and_removes_ours() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        fs::write(&settings, USER_SETTINGS).expect("writes user settings");
        install_claude_mod_into(&settings, &mod_dir).expect("installs");

        let r = uninstall_claude_mod_into(&settings, &mod_dir).expect("uninstalls");

        assert!(r.settings_changed);
        assert!(r.mod_removed);
        let updated = read(&settings);
        assert_eq!(
            plugin_dirs_value(&parse_serde(&updated, &settings).expect("parses")),
            Some("/opt/my-mods"),
            "the user's own dir survives"
        );
        assert!(!mod_dir.exists(), "our folder is gone");
    }

    #[test]
    fn uninstall_prunes_the_key_and_env_when_our_removal_empties_them() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        install_claude_mod_into(&settings, &mod_dir).expect("installs");

        let r = uninstall_claude_mod_into(&settings, &mod_dir).expect("uninstalls");

        assert!(r.settings_changed);
        assert!(r.mod_removed);
        let value = parse_serde(&read(&settings), &settings).expect("parses");
        assert!(value.get("env").is_none(), "env pruned when emptied");
        assert!(plugin_dirs_value(&value).is_none());
    }

    #[test]
    fn round_trip_restores_the_original_settings_bytes() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        fs::write(&settings, USER_SETTINGS).expect("writes user settings");
        let original = read(&settings);

        install_claude_mod_into(&settings, &mod_dir).expect("installs");
        uninstall_claude_mod_into(&settings, &mod_dir).expect("uninstalls");

        assert_eq!(read(&settings), original, "byte-exact round trip");
    }

    #[test]
    fn uninstall_never_touches_a_foreign_mod_folder() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        install_claude_mod_into(&settings, &mod_dir).expect("installs");
        // The user replaced our register.tsx with their own module.
        let register = mod_dir.join("hooks").join("register.tsx");
        fs::write(&register, "// my own module, not par-mux's\n").expect("overwrites");
        fs::write(&settings, "{}\n").expect("settings no longer name us");

        let r = uninstall_claude_mod_into(&settings, &mod_dir).expect("uninstalls");

        assert!(!r.mod_removed, "a folder without our marker survives");
        assert!(mod_dir.exists(), "the foreign folder stays");
    }

    #[test]
    fn heal_restores_a_deleted_folder_under_a_surviving_registration() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        install_claude_mod_into(&settings, &mod_dir).expect("installs");
        fs::remove_dir_all(&mod_dir).expect("external deletion simulated");

        heal_claude_mod_into(&settings, &mod_dir).expect("heals");

        let register = read(&mod_dir.join("hooks").join("register.tsx"));
        assert!(register.contains(CLAUDE_MOD_MARKER));
    }

    #[test]
    fn heal_skips_a_current_folder_and_an_unregistered_install() {
        let root = temp_root();
        let settings = root.path().join("settings.json");
        let mod_dir = root.path().join("mods").join("agent-state");
        install_claude_mod_into(&settings, &mod_dir).expect("installs");
        let before = read(&mod_dir.join("hooks").join("register.tsx"));

        heal_claude_mod_into(&settings, &mod_dir).expect("current folder is a skip");

        assert_eq!(read(&mod_dir.join("hooks").join("register.tsx")), before);

        // No registration anywhere: nothing to heal.
        let bare = temp_root();
        heal_claude_mod_into(&bare.path().join("none.json"), &mod_dir).expect("skip");
    }

    #[test]
    fn mod_dir_for_derives_the_mod_folder_from_the_hook_path() {
        let hook = Path::new("/cfg/par-mux/hooks/par-mux-claude-session-hook.sh");
        assert_eq!(
            mod_dir_for(hook),
            PathBuf::from("/cfg/par-mux/mods/agent-state")
        );
    }

    /// A root-level hook (a test seam's tempdir, no `hooks` component) keeps
    /// the derivation INSIDE the hook's own directory — the two-level walk
    /// escaped a tempdir to the shared TMPDIR and every parallel test that
    /// derived it raced on one `$TMPDIR/mods/agent-state` (measured).
    #[test]
    fn mod_dir_for_stays_inside_a_root_level_hook_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let hook = dir.path().join("par-mux-claude-session-hook.sh");
        let derived = mod_dir_for(&hook);
        assert_eq!(
            derived,
            dir.path().join("mods").join("agent-state"),
            "the config root is the hook's own directory"
        );
        assert!(
            derived.starts_with(dir.path()),
            "the derivation must not escape the caller's tree: {derived:?}"
        );
    }
}
