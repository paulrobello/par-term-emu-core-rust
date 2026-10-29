//! par-mux session-hook installer for config-entry agents.
//!
//! pi and omp load extensions from a directory ([`extensions`]
//! drops a file in). claude, codex, muse, droid, copilot, and cursor have no
//! extension directory: their hooks are ENTRIES inside config files the user
//! owns and edits (grok merges a whole file par-mux owns), so
//! installing means MERGING into those files without disturbing anything
//! par-mux did not add — the user's entries, comments, and formatting
//! survive, proven by round-trip tests in the arm modules.
//!
//! Merge discipline (herdr's claude integration is the proven precedent):
//! - parse before touching anything; a file that fails the strict parse
//!   (duplicate keys, structurally invalid) is a clear error and NOTHING is
//!   written — not even the hook asset;
//! - insertion is a pure text splice at AST-computed offsets replicating the
//!   file's own delimiter style, so untouched bytes never move;
//! - the merged result is re-parsed and compared against the desired value
//!   before it is allowed near the disk (fail closed on any disagreement);
//! - the config file is rewritten through the atomic sibling-temp rename
//!   (the local `atomic` module), preserving its mode;
//! - reinstall is a no-op while our command is already present (matched by
//!   command string, whatever the user has done to the formatting around it),
//!   and uninstall removes only our command entries — each cut takes one
//!   adjacent separator comma so `[{ours}, {user}]` becomes `[{user}]`, never
//!   `[, {user}]` — pruning a container only when our removals emptied it.
//!
//! The hook assets are the kimi-port reporter shape (core
//! `tests/assets/par-mux-agent-state.sh`): on the agent's session-start event
//! each sends one `pane.report_agent_session` line over `$PAR_MUX_SOCKET`.
//! The scripts are inert outside a par-mux pane (env guards) and silent on
//! every failure.
//!
//! The engine is deliberately core-free (plain fs + jsonc parsing), so it
//! builds and lints without the `mux` feature. Each arm installs a
//! platform-selected asset — the `.sh` reporter on POSIX, a native
//! PowerShell `.ps1` port on Windows (herdr's `powershell -File` command
//! shape) — through the runtime-selected seam here, so both variants
//! compile and are asserted on every platform.

mod claude;
mod codex;
mod config_hooks;
pub mod extensions;
mod grok;
mod muse;

pub use config_hooks::{
    ConfigHookArm, ConfigHookInstall, ConfigHookUninstall, CONFIG_HOOK_ARMS, COPILOT, CURSOR,
    DEVIN, DROID, KIMI, MASTRACODE, QODERCLI, QWEN,
};

pub use claude::{
    claude_settings_path, heal_claude_hook_asset, install_claude_hook, install_claude_hook_into,
    uninstall_claude_hook, uninstall_claude_hook_into, ClaudeHookInstall, ClaudeHookUninstall,
    CLAUDE_HOOK_MARKER,
};
pub use codex::{
    codex_config_dir, heal_codex_hook_asset, install_codex_hook, install_codex_hook_into,
    uninstall_codex_hook, uninstall_codex_hook_into, CodexHookInstall, CodexHookUninstall,
    CODEX_HOOK_MARKER,
};
pub use extensions::{
    extension_status, install_omp_extension, install_omp_extension_into, install_pi_extension,
    install_pi_extension_into, omp_extension_dir, pi_extension_dir, uninstall_omp_extension,
    uninstall_pi_extension, ExtensionRemoval, ExtensionStatus,
};
pub use grok::{
    grok_config_dir, install_grok_hook, install_grok_hook_into, uninstall_grok_hook,
    uninstall_grok_hook_into, GrokHookInstall, GrokHookUninstall, GROK_HOOK_MARKER,
};
pub use muse::{
    heal_muse_hook_asset, install_muse_hook, install_muse_hook_into, muse_settings_path,
    uninstall_muse_hook, uninstall_muse_hook_into, MuseHookInstall, MuseHookUninstall,
    MUSE_HOOK_MARKER,
};

/// Startup self-heal for the agent-hook script assets under par-mux's own
/// config directory: rewrite a script whose registration in the agent's
/// config survived it. That directory has twice been deleted externally
/// while the settings entries lived on, leaving every agent session start
/// running a missing script. Best-effort, and only par-mux's own assets
/// are ever written — the agents' config files are never touched.
pub fn heal_hook_assets() {
    claude::heal_claude_hook_asset();
    codex::heal_codex_hook_asset();
    muse::heal_muse_hook_asset();
    for arm in CONFIG_HOOK_ARMS {
        arm.heal();
    }
}

/// Read-only install state of one agent session hook. Never writes anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHookStatus {
    /// The agent name (claude, codex, grok, muse, droid, copilot, cursor).
    pub agent: String,
    /// Whether the session hook is registered in the agent config.
    pub installed: bool,
    /// The installed asset file's currency against the bundled copy.
    pub asset: AssetState,
}

/// Byte-currency of an installed integration asset against the copy
/// bundled into this build — the version-aware half of the status
/// surfaces (`--agent-hooks-status`, `--integration-status`, the settings
/// dialog's integrations rows), so an installed file this build would
/// write differently reads as outdated instead of silently current. The
/// gap that let a stale codex hook keep failing daemon restores after a
/// par-mux upgrade (live, 2026-10-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCurrency {
    /// Nothing at the install path.
    Absent,
    /// Byte-identical to the bundled asset.
    Current,
    /// Present but different from the bundled asset — an older build's copy.
    Outdated,
}

/// What the status surfaces know about one bundled asset: its currency
/// and the `PAR_MUX_INTEGRATION_VERSION` markers on both sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetState {
    /// How the installed file compares to the bundled one.
    pub currency: AssetCurrency,
    /// The installed file's `PAR_MUX_INTEGRATION_VERSION`.
    pub installed_version: Option<String>,
    /// The bundled asset's `PAR_MUX_INTEGRATION_VERSION`.
    pub bundled_version: Option<String>,
}

impl AssetState {
    /// Read `path` and grade it against `bundled`. A file that cannot be
    /// read (missing, unreadable) is Absent; anything readable that
    /// differs by a byte is Outdated.
    pub(crate) fn at(path: &std::path::Path, bundled: &str) -> AssetState {
        let bundled_version = parse_integration_version(bundled);
        let Ok(content) = fs::read_to_string(path) else {
            return AssetState::absent(bundled);
        };
        let currency = if content == bundled {
            AssetCurrency::Current
        } else {
            AssetCurrency::Outdated
        };
        AssetState {
            currency,
            installed_version: parse_integration_version(&content),
            bundled_version,
        }
    }

    /// The Absent state for an asset that cannot be read.
    pub(crate) fn absent(bundled: &str) -> AssetState {
        AssetState {
            currency: AssetCurrency::Absent,
            installed_version: None,
            bundled_version: parse_integration_version(bundled),
        }
    }

    /// The value the status surfaces show for this asset, given whether
    /// its integration is registered/installed at all: `v2` when current,
    /// `v1 → v2 outdated` when a stale copy sits at the path, `not
    /// installed` when there is nothing to grade.
    pub fn display(&self, installed: bool) -> String {
        if !installed {
            return "not installed".to_string();
        }
        let bundled = self.bundled_version.as_deref().unwrap_or("?");
        match self.currency {
            AssetCurrency::Absent => {
                format!("asset missing (bundled v{bundled})")
            }
            AssetCurrency::Current => {
                format!("v{}", self.installed_version.as_deref().unwrap_or(bundled))
            }
            AssetCurrency::Outdated => format!(
                "v{} \u{2192} v{bundled} outdated",
                self.installed_version.as_deref().unwrap_or("?")
            ),
        }
    }
}

/// The `PAR_MUX_INTEGRATION_VERSION=<n>` marker's value, whatever comment
/// grammar wraps it (`# `, `// `, `<!-- ... -->`) — one parser for the
/// hooks, the extensions, and the skill.
pub fn parse_integration_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start_matches(['#', '/', ' ', '!', '<', '-']);
        let Some(rest) = trimmed.strip_prefix("PAR_MUX_INTEGRATION_VERSION=") else {
            continue;
        };
        let token = rest.split_whitespace().next().unwrap_or("");
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

/// Read-only install state of each agent session hook, the data behind
/// the --agent-hooks-status flag, the --integration-status flag, and the
/// attach settings dialog's integrations rows. Never writes anything.
pub fn hook_status() -> Vec<AgentHookStatus> {
    [
        (
            "claude",
            claude::claude_hook_installed(),
            claude::hook_asset_state(),
        ),
        (
            "codex",
            codex::codex_hook_installed(),
            codex::hook_asset_state(),
        ),
        (
            "grok",
            grok::grok_hook_installed(),
            grok::hook_asset_state(),
        ),
        (
            "muse",
            muse::muse_hook_installed(),
            muse::hook_asset_state(),
        ),
    ]
    .into_iter()
    .chain(
        CONFIG_HOOK_ARMS
            .iter()
            .map(|arm| (arm.agent, arm.installed(), arm.hook_asset_state())),
    )
    .map(|(agent, installed, asset)| AgentHookStatus {
        agent: agent.to_string(),
        installed,
        asset,
    })
    .collect()
}

/// Whether the settings dialog's update button reinstalls this
/// integration: it is installed and its asset is not the bundled copy
/// (`v1 → v2 outdated`, or `asset missing` under a surviving
/// registration). Never-installed rows are left alone — the button
/// updates, it does not adopt.
fn wants_update(installed: bool, asset: &AssetState) -> bool {
    installed && asset.currency != AssetCurrency::Current
}

/// Reinstall every integration the update button selects (the predicate
/// [`update_outdated_integrations`] documents), dispatching each
/// name through `install`. Returns `(updated, failed)` name lists, a
/// failure as `name: error`. Pure over its inputs, so tests drive it with
/// fake statuses and a recording closure.
pub fn update_outdated_with(
    hooks: Vec<AgentHookStatus>,
    extensions: Vec<ExtensionStatus>,
    skill: Option<crate::mux::skill_installer::SkillStatus>,
    install: &mut dyn FnMut(&str) -> io::Result<()>,
) -> (Vec<String>, Vec<String>) {
    let mut updated: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut run = |name: &str, stale: bool| {
        if !stale {
            return;
        }
        match install(name) {
            Ok(()) => updated.push(name.to_string()),
            Err(err) => failed.push(format!("{name}: {err}")),
        }
    };
    for hook in hooks {
        run(&hook.agent, wants_update(hook.installed, &hook.asset));
    }
    for ext in extensions {
        run(&ext.agent, wants_update(ext.installed, &ext.asset));
    }
    if let Some(skill) = skill {
        run("skill", wants_update(skill.installed, &skill.asset));
    }
    (updated, failed)
}

/// The settings dialog's update button: reinstall every integration the
/// status surfaces grade as stale (installed, and the asset is not the
/// bundled copy) through the real
/// installers. Returns `(updated, failed)` as [`update_outdated_with`].
pub fn update_outdated_integrations() -> (Vec<String>, Vec<String>) {
    let skill = crate::mux::skill_installer::skill_status().ok();
    update_outdated_with(
        hook_status(),
        extension_status(),
        skill,
        &mut |name| match name {
            "claude" => claude::install_claude_hook().map(|_| ()),
            "codex" => codex::install_codex_hook().map(|_| ()),
            "grok" => grok::install_grok_hook().map(|_| ()),
            "muse" => muse::install_muse_hook().map(|_| ()),
            "pi" => extensions::install_pi_extension().map(|_| ()),
            "omp" => extensions::install_omp_extension().map(|_| ()),
            "skill" => crate::mux::skill_installer::install_skill().map(|_| ()),
            other => match CONFIG_HOOK_ARMS.iter().find(|arm| arm.agent == other) {
                Some(arm) => arm.install().map(|_| ()),
                None => Ok(()),
            },
        },
    )
}

use jsonc_parser::ast::{
    Array as AstArray, Object as AstObject, ObjectPropName, Value as AstValue,
};
use jsonc_parser::common::{Range, Ranged};
use jsonc_parser::{parse_to_ast, parse_to_serde_value, ParseOptions};
use serde_json::{json, Value as JsonValue};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const HOOK_TIMEOUT_SECS: u64 = 10;

/// Where hook script assets are written: par-mux own config directory
/// (the config file directory), so every par-mux integration lives under
/// one roof. Fallible because the platform config dir is resolvable only
/// when the env allows it (the same rule as --gen-config).
pub(crate) fn hook_asset_path(install_name: &str) -> io::Result<PathBuf> {
    let dir = crate::mux::config::config_file_path()
        .and_then(|path| path.parent().map(|dir| dir.to_path_buf()))
        .map(|dir| dir.join("hooks"))
        .ok_or_else(|| {
            io::Error::other(
                "no par-mux config directory known (set PAR_MUX_CONFIG or XDG_CONFIG_HOME)",
            )
        })?;
    Ok(dir.join(install_name))
}

/// The settings entry's command string for the current platform (no action
/// argument — the claude arm's form).
pub(crate) fn hook_command_for(hook_path: &Path) -> String {
    platform_hook_command(hook_path, None, cfg!(windows))
}

/// The settings entry's command string with a trailing action argument (the
/// codex/grok arms' `… session` form).
pub(crate) fn hook_command_with_action(hook_path: &Path, action: &str) -> String {
    platform_hook_command(hook_path, Some(action), cfg!(windows))
}

/// Platform core (test seam — `windows` is injected so both arms compile and
/// are asserted on every platform, leaving no cfg(windows) blind spot):
///
/// - POSIX runs the `.sh` asset. The claude form leaves the path bare when
///   it needs no quoting so the common case stays readable; the action form
///   (`sh '<path>' session`) always quotes, the grok-arm heritage.
/// - Windows runs the `.ps1` asset through herdr's command shape —
///   `powershell -NoProfile -ExecutionPolicy Bypass -File "<path>"` — with
///   the path always double-quoted (spaces are the common case under
///   `%APPDATA%`) and embedded quotes backslash-escaped.
pub(crate) fn platform_hook_command(
    hook_path: &Path,
    action: Option<&str>,
    windows: bool,
) -> String {
    let text = hook_path.display().to_string();
    let mut command = if windows {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            text.replace('"', "\\\"")
        )
    } else if let Some(action) = action {
        format!("sh {} {action}", shell_single_quote(&text))
    } else {
        let safe = !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/.:=@%+".contains(c));
        if safe {
            text
        } else {
            shell_single_quote(&text)
        }
    };
    if let Some(action) = action {
        if windows {
            command.push(' ');
            command.push_str(action);
        }
    }
    command
}

pub(crate) fn shell_single_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub(crate) fn write_hook_asset(hook_path: &Path, asset: &str) -> io::Result<()> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(hook_path, asset)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Delete a hook asset only when it carries our marker: a same-named file
/// without one is a user file we must not touch.
pub(crate) fn remove_marked_file(path: &Path, marker: &str) -> io::Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    if !fs::read_to_string(path)?.contains(marker) {
        return Ok(false);
    }
    fs::remove_file(path)?;
    Ok(true)
}

pub(crate) fn save_settings(settings_path: &Path, contents: &str) -> io::Result<()> {
    atomic::save_string_atomic_preserving_mode(settings_path, contents).map_err(|err| {
        io::Error::other(format!(
            "failed to write {}: {err:#}",
            settings_path.display()
        ))
    })
}

pub(crate) fn home_dir() -> io::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| io::Error::other("Could not determine home directory"))
}

/// Settings files are jsonc in practice: comments and trailing commas are
/// part of what the agents accept, everything looser is not.
pub(crate) fn parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

pub(crate) fn parse_root_object<'a>(
    content: &'a str,
    settings_path: &Path,
) -> io::Result<AstObject<'a>> {
    let parsed = parse_to_ast(content, &Default::default(), &parse_options()).map_err(|err| {
        io::Error::other(format!(
            "failed to parse {}: {err}",
            settings_path.display()
        ))
    })?;
    match parsed.value {
        Some(AstValue::Object(object)) => Ok(object),
        _ => Err(io::Error::other(format!(
            "config at {} must be a JSON object",
            settings_path.display()
        ))),
    }
}

pub(crate) fn parse_serde(content: &str, settings_path: &Path) -> io::Result<JsonValue> {
    parse_to_serde_value(content, &parse_options()).map_err(|err| {
        io::Error::other(format!(
            "failed to parse {}: {err}",
            settings_path.display()
        ))
    })
}

/// Duplicate keys make the file ambiguous — which value survives a merge is
/// not ours to decide, so refuse.
pub(crate) fn reject_duplicate_keys(object: &AstObject, settings_path: &Path) -> io::Result<()> {
    let mut names = std::collections::HashSet::new();
    for property in &object.properties {
        let Some(name) = prop_name_text(&property.name) else {
            return Err(io::Error::other("JSON object property is missing a name"));
        };
        if !names.insert(name) {
            return Err(io::Error::other(format!(
                "duplicate key \"{name}\" in {} — fix the file before installing",
                settings_path.display()
            )));
        }
    }
    for property in &object.properties {
        match &property.value {
            AstValue::Object(inner) => reject_duplicate_keys(inner, settings_path)?,
            AstValue::Array(array) => {
                for element in &array.elements {
                    if let AstValue::Object(inner) = element {
                        reject_duplicate_keys(inner, settings_path)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn prop_name_text<'a>(name: &'a ObjectPropName<'a>) -> Option<&'a str> {
    match name {
        ObjectPropName::String(lit) => Some(lit.value.as_ref()),
        ObjectPropName::Word(word) => Some(word.value),
    }
}

pub(crate) fn string_value<'a>(value: &'a AstValue<'_>) -> Option<&'a str> {
    match value {
        AstValue::StringLit(lit) => Some(lit.value.as_ref()),
        _ => None,
    }
}

/// The canonical SessionStart entry for `command`, as compact JSON text (used
/// for the splice) and as a value (used for the presence check and verify).
/// `matcher` scopes the entry to claude's SessionStart sources; codex has no
/// matcher space and passes `None`.
pub(crate) fn canonical_entry_json(matcher: Option<&str>, command: &str) -> String {
    let command_json = serde_json::to_string(command).expect("command is serializable");
    let matcher_json = match matcher {
        Some(matcher) => format!(
            "\"matcher\":{},",
            serde_json::to_string(matcher).expect("matcher is serializable",)
        ),
        None => String::new(),
    };
    format!(
        "{{{matcher_json}\"hooks\":[{{\"type\":\"command\",\"command\":{command_json},\"timeout\":{HOOK_TIMEOUT_SECS}}}]}}"
    )
}

pub(crate) fn canonical_entry_value(matcher: Option<&str>, command: &str) -> JsonValue {
    let mut entry = json!({});
    if let Some(matcher) = matcher {
        entry["matcher"] = json!(matcher);
    }
    entry["hooks"] = json!([{
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_SECS,
    }]);
    entry
}

/// Whether any SessionStart group already carries our exact command.
pub(crate) fn has_command(
    content: &str,
    settings_path: &Path,
    command: &str,
    event: &str,
) -> io::Result<bool> {
    let value = parse_serde(content, settings_path)?;
    Ok(value
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(JsonValue::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(JsonValue::as_array)
                    .is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.get("command").and_then(JsonValue::as_str) == Some(command)
                        })
                    })
            })
        }))
}

/// Merge our entry for `event` into `content`. `Ok(None)` = already present,
/// byte-exact no-op.
pub(crate) fn merge_hook_event(
    content: &str,
    settings_path: &Path,
    command: &str,
    matcher: Option<&str>,
    event: &str,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if has_command(content, settings_path, command, event)? {
        return Ok(None);
    }

    splice_event_entry(
        content,
        &root,
        settings_path,
        event,
        &canonical_entry_json(matcher, command),
        canonical_entry_value(matcher, command),
    )
    .map(Some)
}

/// Append one entry (`canonical` text, `value` its parsed form) to
/// `hooks.<event>`, creating the array and the hooks object as needed, and
/// verify the spliced text parses back to exactly the desired value — the
/// shared tail of the nested-group and flat-entry merges.
fn splice_event_entry(
    content: &str,
    root: &AstObject<'_>,
    settings_path: &Path,
    event: &str,
    canonical: &str,
    value: JsonValue,
) -> io::Result<String> {
    let updated = match root.get_object("hooks") {
        Some(hooks) => match hooks.get_array(event) {
            Some(event_array) => append_array_element(content, event_array, canonical),
            None => append_object_property(content, hooks, event, &format!("[{canonical}]")),
        },
        None => append_object_property(
            content,
            root,
            "hooks",
            &format!("{{\"{event}\":[{canonical}]}}"),
        ),
    };

    // Fail closed: the spliced text must parse back to exactly the desired
    // value before the caller is allowed to write it. The desired value
    // mirrors the splice branch above.
    let mut desired = parse_serde(content, settings_path)?;
    match desired
        .get_mut("hooks")
        .and_then(|hooks| hooks.get_mut(event))
        .and_then(JsonValue::as_array_mut)
    {
        Some(entries) => entries.push(value),
        None => {
            let Some(root_object) = desired.as_object_mut() else {
                return Err(io::Error::other(format!(
                    "config at {} must be a JSON object",
                    settings_path.display()
                )));
            };
            match root_object.get_mut("hooks") {
                Some(hooks_value) => {
                    let Some(hooks_object) = hooks_value.as_object_mut() else {
                        return Err(io::Error::other(format!(
                            "hooks at {} must be a JSON object",
                            settings_path.display()
                        )));
                    };
                    hooks_object.insert(event.to_string(), json!([value]));
                }
                None => {
                    let mut fresh_hooks = serde_json::Map::new();
                    fresh_hooks.insert(event.to_string(), json!([value]));
                    root_object.insert("hooks".to_string(), JsonValue::Object(fresh_hooks));
                }
            }
        }
    }
    verify_update(updated, settings_path, &desired)
}

/// Whether `hooks.<event>` holds an entry `is_ours` accepts — the presence
/// check for the flat-entry shapes (copilot's `{type, bash}`, cursor's bare
/// `{command}`), where the command is not nested under a group.
pub(crate) fn has_flat_entry(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<bool> {
    let value = parse_serde(content, settings_path)?;
    Ok(any_entry(
        value.get("hooks").and_then(|hooks| hooks.get(event)),
        is_ours,
    ))
}

/// Whether `entries` is an array holding an element `is_ours` accepts.
fn any_entry(entries: Option<&JsonValue>, is_ours: &dyn Fn(&JsonValue) -> bool) -> bool {
    entries
        .and_then(JsonValue::as_array)
        .is_some_and(|entries| entries.iter().any(is_ours))
}

/// Merge one flat entry into `hooks.<event>` under the same discipline as
/// [`merge_hook_event`]. `Ok(None)` = an entry `is_ours` accepts is already
/// present, a byte-exact no-op.
pub(crate) fn merge_flat_entry(
    content: &str,
    settings_path: &Path,
    event: &str,
    entry: &JsonValue,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if has_flat_entry(content, settings_path, event, is_ours)? {
        return Ok(None);
    }
    let canonical = serde_json::to_string(entry).expect("hook entry is serializable");
    splice_event_entry(
        content,
        &root,
        settings_path,
        event,
        &canonical,
        entry.clone(),
    )
    .map(Some)
}

/// Remove every flat entry `is_ours` accepts from `hooks.<event>`, one
/// separator comma per cut, pruning the event array and the hooks object
/// only when our removals emptied them. `Ok(None)` = nothing to remove.
pub(crate) fn unmerge_flat_entries(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    let original = parse_serde(content, settings_path)?;

    let (updated, changed) = apply_cuts(content, |text| {
        find_next_flat_cut(text, settings_path, event, is_ours)
    })?;
    if !changed {
        return Ok(None);
    }
    let (updated, _) = apply_cuts(&updated, |text| {
        find_next_empty_container_cut(text, settings_path, &[event])
    })?;

    let mut desired = original;
    if let Some(root_object) = desired.as_object_mut() {
        if let Some(hooks) = root_object
            .get_mut("hooks")
            .and_then(JsonValue::as_object_mut)
        {
            if let Some(entries) = hooks.get_mut(event).and_then(JsonValue::as_array_mut) {
                let had_entries = !entries.is_empty();
                entries.retain(|entry| !is_ours(entry));
                if had_entries && entries.is_empty() {
                    hooks.remove(event);
                }
            }
            if hooks.is_empty() {
                root_object.remove("hooks");
            }
        }
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

// Root-level flat entry helpers (MastraCode's `hooks.json`): its event
// arrays live at the document root, not under a `hooks` object, so the
// same splice discipline applies one level up.

/// Whether the root-level `<event>` holds an entry `is_ours` accepts.
pub(crate) fn has_root_flat_entry(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<bool> {
    let value = parse_serde(content, settings_path)?;
    Ok(any_entry(value.get(event), is_ours))
}

/// Merge one flat entry into the root-level `<event>` array under the same
/// discipline as [`merge_flat_entry`]. `Ok(None)` = already present.
pub(crate) fn merge_root_flat_entry(
    content: &str,
    settings_path: &Path,
    event: &str,
    entry: &JsonValue,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if has_root_flat_entry(content, settings_path, event, is_ours)? {
        return Ok(None);
    }
    let canonical = serde_json::to_string(entry).expect("hook entry is serializable");
    let updated = match root.get_array(event) {
        Some(event_array) => append_array_element(content, event_array, &canonical),
        None => append_object_property(content, &root, event, &format!("[{canonical}]")),
    };

    let mut desired = parse_serde(content, settings_path)?;
    match desired.get_mut(event).and_then(JsonValue::as_array_mut) {
        Some(entries) => entries.push(entry.clone()),
        None => {
            let Some(root_object) = desired.as_object_mut() else {
                return Err(io::Error::other(format!(
                    "config at {} must be a JSON object",
                    settings_path.display()
                )));
            };
            root_object.insert(event.to_string(), json!([entry]));
        }
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

/// Remove every flat entry `is_ours` accepts from the root-level `<event>`,
/// pruning the event key when our removals emptied it. `Ok(None)` = nothing
/// to remove.
pub(crate) fn unmerge_root_flat_entry(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    let original = parse_serde(content, settings_path)?;

    let (updated, changed) = apply_cuts(content, |text| {
        find_next_root_flat_cut(text, settings_path, event, is_ours)
    })?;
    if !changed {
        return Ok(None);
    }
    // Prune the emptied event key only; the document itself stays.
    let (updated, _) = apply_cuts(&updated, |text| {
        find_next_empty_root_event_cut(text, settings_path, event)
    })?;

    let mut desired = original;
    if let Some(root_object) = desired.as_object_mut() {
        if let Some(entries) = root_object.get_mut(event).and_then(JsonValue::as_array_mut) {
            let had_entries = !entries.is_empty();
            entries.retain(|entry| !is_ours(entry));
            if had_entries && entries.is_empty() {
                root_object.remove(event);
            }
        }
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

/// The next flat entry of ours to cut from the root-level `<event>`.
fn find_next_root_flat_cut(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<Range>> {
    let root = parse_root_object(content, settings_path)?;
    find_flat_cut_in(content, settings_path, root.get_array(event), is_ours)
}

fn find_next_empty_root_event_cut(
    content: &str,
    settings_path: &Path,
    event: &str,
) -> io::Result<Option<Range>> {
    let root = parse_root_object(content, settings_path)?;
    if let Some(event_array) = root.get_array(event) {
        if event_array.elements.is_empty() {
            return Ok(Some(property_cut(&root.properties, event)));
        }
    }
    Ok(None)
}

/// The next flat entry of ours to cut from `hooks.<event>`.
fn find_next_flat_cut(
    content: &str,
    settings_path: &Path,
    event: &str,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<Range>> {
    let root = parse_root_object(content, settings_path)?;
    let event_array = root
        .get_object("hooks")
        .and_then(|hooks| hooks.get_array(event));
    find_flat_cut_in(content, settings_path, event_array, is_ours)
}

/// The cut for the first element of `event_array` that `is_ours` accepts,
/// each element judged by parsing its own text span: the shared walk of
/// the `hooks.<event>` and root-level `<event>` flat shapes (QA-707).
fn find_flat_cut_in(
    content: &str,
    settings_path: &Path,
    event_array: Option<&AstArray<'_>>,
    is_ours: &dyn Fn(&JsonValue) -> bool,
) -> io::Result<Option<Range>> {
    let Some(event_array) = event_array else {
        return Ok(None);
    };
    for (index, element) in event_array.elements.iter().enumerate() {
        let range = element.range();
        let value = parse_serde(&content[range.start..range.end], settings_path)?;
        if !is_ours(&value) {
            continue;
        }
        return Ok(Some(member_cut(
            index
                .checked_sub(1)
                .and_then(|prev| event_array.elements.get(prev))
                .map(|prev| prev.range().end),
            range.start,
            range.end,
            event_array
                .elements
                .get(index + 1)
                .map(|next| next.range().start),
        )));
    }
    Ok(None)
}

/// Apply `next_cut` until it finds nothing, one span per pass, each span
/// computed against the text it is cut from. Returns the cut text and
/// whether any cut happened.
fn apply_cuts(
    content: &str,
    mut next_cut: impl FnMut(&str) -> io::Result<Option<Range>>,
) -> io::Result<(String, bool)> {
    let mut updated = content.to_string();
    let mut changed = false;
    while let Some(cut) = next_cut(&updated)? {
        let mut next = String::with_capacity(updated.len());
        next.push_str(&updated[..cut.start]);
        next.push_str(&updated[cut.end..]);
        updated = next;
        changed = true;
    }
    Ok((updated, changed))
}

/// Add `"<key>": <value_json>` to the root object when the key is absent
/// (cursor's required `version`), as a splice that leaves every other byte
/// alone. `Ok(None)` = the key is already there.
pub(crate) fn ensure_root_key(
    content: &str,
    settings_path: &Path,
    key: &str,
    value_json: &str,
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    if root
        .properties
        .iter()
        .any(|property| prop_name_text(&property.name) == Some(key))
    {
        return Ok(None);
    }
    let updated = append_object_property(content, &root, key, value_json);
    let mut desired = parse_serde(content, settings_path)?;
    let value: JsonValue = serde_json::from_str(value_json).map_err(io::Error::other)?;
    if let Some(object) = desired.as_object_mut() {
        object.insert(key.to_string(), value);
    }
    verify_update(updated, settings_path, &desired).map(Some)
}

/// Remove our command entries from `content`. Each entry pairs the hook
/// event its command lives under with the command string; only those
/// events' arrays are touched, so a user's own hooks — including empty
/// arrays under events we never install — survive byte-exact.
/// `Ok(None)` = nothing to remove.
pub(crate) fn unmerge_hook_entries(
    content: &str,
    settings_path: &Path,
    entries: &[(&str, &str)],
) -> io::Result<Option<String>> {
    let root = parse_root_object(content, settings_path)?;
    reject_duplicate_keys(&root, settings_path)?;
    let original = parse_serde(content, settings_path)?;
    let events: Vec<&str> = entries.iter().map(|(event, _)| *event).collect();

    // One cut per pass, re-parsing between cuts so every span is computed
    // against the text it is applied to. Settings files are small.
    let (updated, changed) = apply_cuts(content, |text| {
        find_next_our_cut(text, settings_path, entries)
    })?;

    if !changed {
        return Ok(None);
    }

    // Prune containers our removals emptied: the events we cut from, then
    // hooks. Each prune is its own re-parsed pass for the same span-safety
    // reason.
    let (updated, _) = apply_cuts(&updated, |text| {
        find_next_empty_container_cut(text, settings_path, &events)
    })?;

    let desired = desired_after_removal(original, entries);
    verify_update(updated, settings_path, &desired).map(Some)
}

/// The next single removal to make: a whole group when the group holds only
/// our commands, else one of our command entries inside a mixed group.
fn find_next_our_cut(
    content: &str,
    settings_path: &Path,
    entries: &[(&str, &str)],
) -> io::Result<Option<Range>> {
    let root = parse_root_object(content, settings_path)?;
    let Some(hooks) = root.get_object("hooks") else {
        return Ok(None);
    };

    for (event, command) in entries {
        let Some(event_array) = hooks.get_array(event) else {
            continue;
        };
        for (group_index, group) in event_array.elements.iter().enumerate() {
            let AstValue::Object(group_object) = group else {
                continue;
            };
            let Some(group_hooks) = group_object.get_array("hooks") else {
                continue;
            };
            let Some(first_our_entry) = group_hooks
                .elements
                .iter()
                .position(|entry| entry_is_our_command(entry, command))
            else {
                continue;
            };
            if group_hooks
                .elements
                .iter()
                .all(|entry| entry_is_our_command(entry, command))
            {
                let group_range = group.range();
                let cut = member_cut(
                    group_index
                        .checked_sub(1)
                        .and_then(|prev| event_array.elements.get(prev))
                        .map(Ranged::range)
                        .map(|range| range.end),
                    group_range.start,
                    group_range.end,
                    event_array
                        .elements
                        .get(group_index + 1)
                        .map(Ranged::range)
                        .map(|range| range.start),
                );
                return Ok(Some(cut));
            }
            let entry_range = group_hooks.elements[first_our_entry].range();
            let cut = member_cut(
                first_our_entry
                    .checked_sub(1)
                    .and_then(|prev| group_hooks.elements.get(prev))
                    .map(Ranged::range)
                    .map(|range| range.end),
                entry_range.start,
                entry_range.end,
                group_hooks
                    .elements
                    .get(first_our_entry + 1)
                    .map(Ranged::range)
                    .map(|range| range.start),
            );
            return Ok(Some(cut));
        }
    }
    Ok(None)
}

/// The next empty-container prune: an event array we cut from with no
/// elements left (drop the property from hooks), or a hooks object with no
/// properties (drop the property from root). Events we never touched are
/// not candidates — a user's pre-existing empty array survives uninstall.
fn find_next_empty_container_cut(
    content: &str,
    settings_path: &Path,
    events: &[&str],
) -> io::Result<Option<Range>> {
    let root = parse_root_object(content, settings_path)?;
    let Some(hooks) = root.get_object("hooks") else {
        return Ok(None);
    };
    if hooks.properties.is_empty() {
        return Ok(Some(property_cut(&root.properties, "hooks")));
    }
    for event in events {
        if let Some(event_array) = hooks.get_array(event) {
            if event_array.elements.is_empty() {
                return Ok(Some(property_cut(&hooks.properties, event)));
            }
        }
    }
    Ok(None)
}

/// The cut span for the named property, including one adjacent separator
/// comma — same rule as element cuts.
fn property_cut(properties: &[jsonc_parser::ast::ObjectProp<'_>], name: &str) -> Range {
    let Some(index) = properties
        .iter()
        .position(|property| prop_name_text(&property.name) == Some(name))
    else {
        return Range { start: 0, end: 0 };
    };
    let property = &properties[index];
    member_cut(
        index
            .checked_sub(1)
            .map(|prev| &properties[prev])
            .map(|prev| prev.range.end),
        property.range.start,
        property.range.end,
        properties.get(index + 1).map(|next| next.range.start),
    )
}

/// The cut span for removing container member `[start, end)`: the separator
/// in front when there is a predecessor, else the one behind, so exactly one
/// comma goes with the member and the surviving text stays well-formed.
fn member_cut(
    prev_end: Option<usize>,
    start: usize,
    end: usize,
    next_start: Option<usize>,
) -> Range {
    match (prev_end, next_start) {
        (Some(prev), _) => Range { start: prev, end },
        (None, Some(next)) => Range { start, end: next },
        (None, None) => Range { start, end },
    }
}

fn entry_is_our_command(entry: &AstValue, command: &str) -> bool {
    let AstValue::Object(entry_object) = entry else {
        return false;
    };
    entry_object.properties.iter().any(|property| {
        prop_name_text(&property.name) == Some("command")
            && string_value(&property.value) == Some(command)
    })
}

/// The value the file must parse back to after uninstall: our commands gone,
/// emptied containers pruned. Only the events our entries were installed
/// under are pruned, and only when removing ours emptied them — a user's
/// pre-existing empty array under any event survives.
fn desired_after_removal(mut value: JsonValue, entries: &[(&str, &str)]) -> JsonValue {
    let Some(root) = value.as_object_mut() else {
        return value;
    };
    let Some(hooks) = root.get_mut("hooks").and_then(JsonValue::as_object_mut) else {
        return value;
    };
    for (event, command) in entries {
        let Some(event_array) = hooks.get_mut(*event).and_then(JsonValue::as_array_mut) else {
            continue;
        };
        let had_groups = !event_array.is_empty();
        let mut kept_groups = Vec::with_capacity(event_array.len());
        for group in event_array.drain(..) {
            let mut group = group;
            let pruned_empty = match group.get_mut("hooks").and_then(JsonValue::as_array_mut) {
                Some(entries) => {
                    entries.retain(|entry| {
                        entry.get("command").and_then(JsonValue::as_str) != Some(*command)
                    });
                    entries.is_empty()
                }
                None => false,
            };
            if !pruned_empty {
                kept_groups.push(group);
            }
        }
        *event_array = kept_groups;
        if had_groups && event_array.is_empty() {
            hooks.remove(*event);
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    value
}

fn verify_update(updated: String, settings_path: &Path, desired: &JsonValue) -> io::Result<String> {
    let actual = parse_serde(&updated, settings_path)?;
    if &actual != desired {
        return Err(io::Error::other(format!(
            "failed to safely update config at {}",
            settings_path.display()
        )));
    }
    Ok(updated)
}

// --- text-splice machinery (ported from herdr's claude integration) ---

fn append_object_property(
    content: &str,
    object: &AstObject<'_>,
    name: &str,
    value: &str,
) -> String {
    let key = serde_json::to_string(name).expect("JSON object keys are serializable");
    let key_value_separator = object
        .properties
        .first()
        .map(|property| &content[property.name.range().end..property.value.range().start])
        .unwrap_or(":");
    let insertion = format!("{key}{key_value_separator}{value}");
    let delimiter = object_delimiter(content, object);
    append_to_container(
        content,
        object.range,
        !object.properties.is_empty(),
        delimiter,
        &insertion,
    )
}

fn append_array_element(content: &str, array: &AstArray<'_>, value: &str) -> String {
    let delimiter = array_delimiter(content, array);
    append_to_container(
        content,
        array.range,
        !array.elements.is_empty(),
        delimiter,
        value,
    )
}

fn object_delimiter<'a>(content: &'a str, object: &AstObject<'_>) -> &'a str {
    match object.properties.as_slice() {
        [first, second, ..] => delimiter_suffix(&content[first.range.end..second.range.start]),
        [first] => &content[object.range.start + 1..first.range.start],
        [] => "",
    }
}

fn array_delimiter<'a>(content: &'a str, array: &AstArray<'_>) -> &'a str {
    match array.elements.as_slice() {
        [first, second, ..] => delimiter_suffix(&content[first.range().end..second.range().start]),
        [first] => &content[array.range.start + 1..first.range().start],
        [] => "",
    }
}

fn delimiter_suffix(delimiter: &str) -> &str {
    delimiter
        .split_once(',')
        .map(|(_, suffix)| suffix)
        .unwrap_or(delimiter)
}

fn append_to_container(
    content: &str,
    range: Range,
    has_elements: bool,
    delimiter: &str,
    value: &str,
) -> String {
    let closing = range.end - 1;
    // Insert right after the last non-whitespace member and leave the file's
    // own trailing gap (newline + indent before the closer) AFTER the new
    // member — that is what makes the uninstall cut (back to the previous
    // member's end) restore the original bytes exactly.
    let insertion_index = if has_elements {
        content[..closing]
            .trim_end_matches([' ', '\t', '\n', '\r'])
            .len()
    } else {
        closing
    };
    let mut updated = String::with_capacity(content.len() + delimiter.len() + value.len() + 1);
    updated.push_str(&content[..insertion_index]);
    if has_elements {
        updated.push(',');
        updated.push_str(delimiter);
    }
    updated.push_str(value);
    updated.push_str(&content[insertion_index..]);
    updated
}

/// Crash-safe atomic config writes, ported from par-term-config
/// `atomic_save` (the durability and permission contracts live there).
/// Only what the hook installers need: write bytes to a sibling temp
/// file, fsync, then rename over the target, preserving the target mode
/// when it exists (0o600 for a new file) and fsyncing the parent dir
/// best-effort so the rename survives a crash.
mod atomic {
    use std::io;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Atomically replace `path` with `contents`, keeping the target mode.
    pub(super) fn save_string_atomic_preserving_mode(
        path: &Path,
        contents: &str,
    ) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "par-mux-save".to_string());
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path =
            path.with_file_name(format!("{file_name}.tmp.{}.{unique}", std::process::id()));
        if let Err(err) = write_staged(&temp_path, path, contents) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }
        if let Err(err) = std::fs::rename(&temp_path, path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }
        sync_parent_dir(path);
        Ok(())
    }

    #[cfg_attr(not(unix), allow(unused_variables))]
    fn write_staged(temp_path: &Path, target: &Path, contents: &str) -> io::Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::File::create(temp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        preserve_mode(target, &file)?;
        Ok(())
    }

    /// Give the staged file the target current mode before the rename
    /// replaces it: `symlink_metadata` deliberately, so a symlink target
    /// keeps the link own mode, not the mode of what it pointed at.
    #[cfg(unix)]
    fn preserve_mode(target: &Path, staged: &std::fs::File) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::symlink_metadata(target)
            .ok()
            .map(|meta| meta.permissions().mode())
            .filter(|mode| *mode != 0o600);
        if let Some(mode) = mode {
            staged.set_permissions(std::fs::Permissions::from_mode(mode))?
        }
        Ok(())
    }

    fn sync_parent_dir(path: &Path) {
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_integration_version_reads_all_three_comment_grammars() {
        assert_eq!(
            parse_integration_version("# PAR_MUX_INTEGRATION_VERSION=3\n"),
            Some("3".to_string())
        );
        assert_eq!(
            parse_integration_version("// PAR_MUX_INTEGRATION_VERSION=12\n"),
            Some("12".to_string())
        );
        assert_eq!(
            parse_integration_version("<!-- PAR_MUX_INTEGRATION_VERSION=1 -->\n"),
            Some("1".to_string())
        );
        assert_eq!(parse_integration_version("no marker here\n"), None);
    }

    #[test]
    fn asset_state_grades_bytes_against_the_bundled_copy() {
        let bundled = "# header\n# PAR_MUX_INTEGRATION_VERSION=2\nbody\n";
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("asset");

        assert_eq!(
            AssetState::at(&path, bundled).currency,
            AssetCurrency::Absent,
            "a missing file is absent"
        );

        std::fs::write(&path, bundled).expect("writes the copy");
        let state = AssetState::at(&path, bundled);
        assert_eq!(state.currency, AssetCurrency::Current);
        assert_eq!(state.installed_version.as_deref(), Some("2"));
        assert_eq!(state.bundled_version.as_deref(), Some("2"));

        std::fs::write(&path, "# PAR_MUX_INTEGRATION_VERSION=1\nold body\n").expect("rewrites");
        let state = AssetState::at(&path, bundled);
        assert_eq!(state.currency, AssetCurrency::Outdated);
        assert_eq!(state.installed_version.as_deref(), Some("1"));
        assert_eq!(state.bundled_version.as_deref(), Some("2"));
    }

    #[test]
    fn display_names_the_version_and_the_drift() {
        let bundled = "# PAR_MUX_INTEGRATION_VERSION=2\n";
        let absent = AssetState::absent(bundled);
        assert_eq!(absent.display(false), "not installed");
        assert_eq!(absent.display(true), "asset missing (bundled v2)");

        let current = AssetState {
            currency: AssetCurrency::Current,
            installed_version: Some("2".to_string()),
            bundled_version: Some("2".to_string()),
        };
        assert_eq!(current.display(true), "v2");

        let outdated = AssetState {
            currency: AssetCurrency::Outdated,
            installed_version: Some("1".to_string()),
            bundled_version: Some("2".to_string()),
        };
        assert_eq!(outdated.display(true), "v1 \u{2192} v2 outdated");
    }

    /// The settings dialog's update button selects only INSTALLED
    /// integrations whose asset is not the bundled copy — outdated, or an
    /// asset gone missing under a surviving registration — and never
    /// adopts a never-installed row.
    #[test]
    fn update_outdated_reinstalls_only_stale_installed_integrations() {
        let stale = |installed_version: Option<&str>| AssetState {
            currency: if installed_version.is_some() {
                AssetCurrency::Outdated
            } else {
                AssetCurrency::Absent
            },
            installed_version: installed_version.map(String::from),
            bundled_version: Some("2".to_string()),
        };
        let hooks = vec![
            AgentHookStatus {
                agent: "stale".to_string(),
                installed: true,
                asset: stale(Some("1")),
            },
            AgentHookStatus {
                agent: "current".to_string(),
                installed: true,
                asset: AssetState {
                    currency: AssetCurrency::Current,
                    installed_version: Some("2".to_string()),
                    bundled_version: Some("2".to_string()),
                },
            },
            AgentHookStatus {
                agent: "gone".to_string(),
                installed: true,
                asset: stale(None),
            },
            AgentHookStatus {
                agent: "off".to_string(),
                installed: false,
                asset: stale(Some("1")),
            },
        ];
        let mut called: Vec<String> = Vec::new();
        let (updated, failed) = update_outdated_with(hooks, vec![], None, &mut |name| {
            called.push(name.to_string());
            Ok(())
        });
        assert_eq!(
            updated,
            vec!["stale".to_string(), "gone".to_string()],
            "installed-but-stale rows reinstall, current and never-installed do not"
        );
        assert!(failed.is_empty(), "no failures expected: {failed:?}");
        assert_eq!(called, updated);
    }

    /// An installer failure surfaces as `name: error` in the failed list,
    /// so the dialog's flash can name what did not move.
    #[test]
    fn update_outdated_reports_installer_failures_by_name() {
        let hooks = vec![AgentHookStatus {
            agent: "stale".to_string(),
            installed: true,
            asset: AssetState {
                currency: AssetCurrency::Outdated,
                installed_version: Some("1".to_string()),
                bundled_version: Some("2".to_string()),
            },
        }];
        let (updated, failed) = update_outdated_with(hooks, vec![], None, &mut |_name| {
            Err(std::io::Error::other("boom"))
        });
        assert!(updated.is_empty(), "nothing counts as updated: {updated:?}");
        assert_eq!(failed, vec!["stale: boom".to_string()]);
    }

    /// Every bundled asset must carry the marker the status surfaces parse
    /// — a hook, extension, or skill shipping without one would always read
    /// as versionless, and the byte-compare would silently carry the
    /// out-of-date detection alone.
    #[test]
    fn every_bundled_asset_carries_a_parseable_version() {
        let assets: [(&str, &str); 17] = [
            (
                "droid sh",
                include_str!("../../../integrations/hooks/par-mux-droid-session-hook.sh"),
            ),
            (
                "droid ps1",
                include_str!("../../../integrations/hooks/par-mux-droid-session-hook.ps1"),
            ),
            (
                "copilot sh",
                include_str!("../../../integrations/hooks/par-mux-copilot-session-hook.sh"),
            ),
            (
                "copilot ps1",
                include_str!("../../../integrations/hooks/par-mux-copilot-session-hook.ps1"),
            ),
            (
                "cursor sh",
                include_str!("../../../integrations/hooks/par-mux-cursor-session-hook.sh"),
            ),
            (
                "cursor ps1",
                include_str!("../../../integrations/hooks/par-mux-cursor-session-hook.ps1"),
            ),
            (
                "claude sh",
                include_str!("../../../integrations/hooks/par-mux-claude-session-hook.sh"),
            ),
            (
                "claude ps1",
                include_str!("../../../integrations/hooks/par-mux-claude-session-hook.ps1"),
            ),
            (
                "codex sh",
                include_str!("../../../integrations/hooks/par-mux-codex-session-hook.sh"),
            ),
            (
                "codex ps1",
                include_str!("../../../integrations/hooks/par-mux-codex-session-hook.ps1"),
            ),
            (
                "grok sh",
                include_str!("../../../integrations/hooks/par-mux-grok-session-hook.sh"),
            ),
            (
                "grok ps1",
                include_str!("../../../integrations/hooks/par-mux-grok-session-hook.ps1"),
            ),
            (
                "muse sh",
                include_str!("../../../integrations/hooks/par-mux-muse-session-hook.sh"),
            ),
            (
                "muse ps1",
                include_str!("../../../integrations/hooks/par-mux-muse-session-hook.ps1"),
            ),
            (
                "pi extension",
                include_str!("../../../integrations/extensions/par-mux-agent-state.ts"),
            ),
            (
                "omp extension",
                include_str!("../../../integrations/extensions/par-mux-omp-agent-state.ts"),
            ),
            ("skill", include_str!("../../../skills/par-mux/SKILL.md")),
        ];
        for (name, text) in assets {
            assert!(
                parse_integration_version(text).is_some(),
                "{name} lost its PAR_MUX_INTEGRATION_VERSION marker"
            );
        }
    }
}
