//! The plugin tier: manifests under `<state-dir>/par-mux/plugins/<name>/`
//! whose hooks run shell commands when the daemon emits matching control
//! notifications. Everything about a plugin is in its manifest and nothing
//! runs until `plugin install --trust` previews the source and every
//! command — the trust gate the CLI enforces before a file lands in the
//! plugins directory the daemon loads.
//!
//! Dispatch rides `crate::mux::server::broadcast::broadcast_notification`: the notification a
//! client sees is exactly the notification a plugin sees (same emitter,
//! same line), so a plugin cannot drift from the wire. Hook processes run
//! detached with a per-hook timeout and a daemon-wide concurrency cap; a
//! slow or stuck plugin drops its events, it never stalls the daemon.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// The manifest file every plugin directory carries.
pub const MANIFEST_NAME: &str = "par-mux-plugin.toml";

/// Hook events a manifest may name — the stable, meaningful slice of the
/// wire notifications (the emitter's own spellings).
pub const HOOK_EVENTS: &[&str] = &[
    "session-changed",
    "session-renamed",
    "sessions-changed",
    "window-add",
    "window-close",
    "window-renamed",
    "layout-change",
    "pane-exited",
    "pane-respawned",
    "pane-title-changed",
    "agent-state-changed",
    "agent-released",
    "notification",
    "config-changed",
];

/// The default and maximum hook timeout.
const HOOK_TIMEOUT_MS: u64 = 3_000;

/// At most this many hook processes run at once; beyond it the event drops
/// with a warning — a flooding plugin loses its own events, the daemon
/// keeps serving.
const MAX_RUNNING_HOOKS: usize = 8;

/// `par-mux-plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginManifest {
    /// The plugin's name — must equal its directory name.
    pub name: String,
    /// A version string, shown by `plugin list`.
    pub version: String,
    /// One line on what the plugin does, shown in the install preview.
    #[serde(default)]
    pub description: String,
    /// The hooks: which events run which commands.
    #[serde(rename = "hook")]
    pub hooks: Vec<Hook>,
}

/// One `[[hook]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Hook {
    /// An event from [`HOOK_EVENTS`].
    pub event: String,
    /// The shell command line the event runs. The emitted line is exposed
    /// as `$PAR_MUX_EVENT_LINE`, the event name as `$PAR_MUX_EVENT`.
    pub command: String,
    /// Kill the hook after this many milliseconds (1..=60000; default
    /// 3000).
    #[serde(
        rename = "timeout-ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,
}

impl Hook {
    /// The effective timeout, defaulted and clamped.
    pub fn effective_timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(HOOK_TIMEOUT_MS).clamp(1, 60_000)
    }
}

/// Parse and validate a manifest: the name is a safe directory name, every
/// hook names a known event, no command is empty, timeouts are in range.
pub fn parse_manifest(text: &str) -> Result<PluginManifest, String> {
    let manifest: PluginManifest =
        toml::from_str(text).map_err(|err| format!("manifest: {err}"))?;
    if manifest.name.is_empty()
        || !manifest
            .name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "plugin names are letters, digits, dashes, dots, and underscores: {:?}",
            manifest.name
        ));
    }
    if manifest.version.is_empty() {
        return Err("plugin version is required".to_string());
    }
    if manifest.hooks.is_empty() {
        return Err("a plugin declares at least one [[hook]]".to_string());
    }
    for hook in &manifest.hooks {
        if !HOOK_EVENTS.contains(&hook.event.as_str()) {
            return Err(format!(
                "unknown hook event {:?} (known: {})",
                hook.event,
                HOOK_EVENTS.join(", ")
            ));
        }
        if hook.command.trim().is_empty() {
            return Err(format!("the {:?} hook has an empty command", hook.event));
        }
        if let Some(timeout) = hook.timeout_ms {
            if !(1..=60_000).contains(&timeout) {
                return Err(format!(
                    "the {:?} hook's timeout-ms must be 1..=60000: {timeout}",
                    hook.event
                ));
            }
        }
    }
    Ok(manifest)
}

/// One installed plugin: its directory and parsed manifest.
#[derive(Debug)]
pub struct LoadedPlugin {
    /// The installed plugin's directory.
    pub dir: PathBuf,
    /// Its parsed manifest.
    pub manifest: PluginManifest,
}

/// The engine the daemon loads and the broadcaster consults.
#[derive(Debug, Default)]
pub struct PluginEngine {
    plugins: Vec<LoadedPlugin>,
    /// Running hook children with their kill deadlines. Guarded because
    /// dispatch fires from broadcast paths on many threads.
    running: Mutex<Vec<(std::process::Child, std::time::Instant)>>,
}

impl PluginEngine {
    /// Load every valid plugin under `dir` (missing dir = empty engine);
    /// an invalid manifest warns and skips, never failing the daemon.
    pub fn load(dir: &Path) -> Arc<Self> {
        let mut plugins = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return Arc::new(Self::default()),
        };
        for entry in entries.flatten() {
            let path = entry.path().join(MANIFEST_NAME);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match parse_manifest(&text) {
                Ok(manifest) if manifest.name == entry.file_name().to_string_lossy() => {
                    plugins.push(LoadedPlugin {
                        dir: entry.path(),
                        manifest,
                    });
                }
                Ok(manifest) => log::warn!(
                    "par-mux: plugin {} skipped: manifest name {:?} != directory name",
                    entry.path().display(),
                    manifest.name
                ),
                Err(err) => log::warn!("par-mux: plugin {} skipped: {err}", entry.path().display()),
            }
        }
        Arc::new(Self {
            plugins,
            running: Mutex::new(Vec::new()),
        })
    }

    /// The loaded plugins (`plugin list` reads this shape).
    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    /// Run the hooks matching `event_name` with `line` (the emitted wire
    /// line). Best-effort: spawn failures and a full concurrency budget
    /// drop the event with a warning.
    pub fn dispatch(&self, event_name: &str, line: &str) {
        let mut spawned = 0;
        for plugin in &self.plugins {
            for hook in &plugin.manifest.hooks {
                if hook.event != event_name {
                    continue;
                }
                if !self.spawn(plugin, hook, line) {
                    return;
                }
                spawned += 1;
            }
        }
        let _ = spawned;
    }

    fn spawn(&self, plugin: &LoadedPlugin, hook: &Hook, line: &str) -> bool {
        {
            let mut running = self
                .running
                .lock()
                .expect("the plugin running-list lock is only held briefly");
            // Reap finished children first so the budget counts live ones.
            running.retain_mut(|(child, _)| child.try_wait().ok().flatten().is_none());
            if running.len() >= MAX_RUNNING_HOOKS {
                log::warn!(
                    "par-mux: plugin {} dropped a {:?} event: {} hooks already running",
                    plugin.manifest.name,
                    hook.event,
                    running.len()
                );
                return false;
            }
            let mut command = hook_command(&hook.command);
            command
                .env("PAR_MUX_EVENT", &hook.event)
                .env("PAR_MUX_EVENT_LINE", line)
                .env_remove("PAR_MUX_SOCKET")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            match command.spawn() {
                Ok(child) => running.push((
                    child,
                    std::time::Instant::now()
                        + std::time::Duration::from_millis(hook.effective_timeout_ms()),
                )),
                Err(err) => log::warn!(
                    "par-mux: plugin {} hook {:?} failed to spawn: {err}",
                    plugin.manifest.name,
                    hook.event
                ),
            }
        }
        true
    }

    /// Kill every hook past its deadline. Cheap; called from the accept
    /// loop's idle tick.
    pub fn reap_expired(&self) {
        let mut running = self
            .running
            .lock()
            .expect("the plugin running-list lock is only held briefly");
        running.retain_mut(|(child, deadline)| {
            if std::time::Instant::now() >= *deadline {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            child.try_wait().ok().flatten().is_none()
        });
    }
}

/// The shell a hook command line runs through: POSIX sh, the daemon's
/// every other ported command's contract; cmd's `/d /c` on Windows.
fn hook_command(command_line: &str) -> std::process::Command {
    #[cfg(unix)]
    {
        let mut command = std::process::Command::new("/bin/sh");
        command.arg("-c").arg(command_line);
        command
    }
    #[cfg(windows)]
    {
        let shell =
            std::env::var_os("ComSpec").unwrap_or_else(|| r"C:\Windows\System32\cmd.exe".into());
        let mut command = std::process::Command::new(shell);
        command.arg("/d").arg("/c").arg(command_line);
        command
    }
}

/// The wire name of a notification — the emitter's own first token, so
/// plugin events can never drift from what clients see.
pub fn event_name(
    notification: &par_term_emu_core::tmux_control::TmuxNotification,
) -> Option<String> {
    let emitted = crate::mux::emit::emit(notification);
    let first = emitted.split_whitespace().next()?;
    let name = first.strip_prefix('%')?;
    HOOK_EVENTS.contains(&name).then(|| name.to_string())
}

/// The daemon's loaded engine. One per process — par-mux runs one daemon
/// per socket, and the broadcast paths reach this from every thread.
static ENGINE: OnceLock<RwLock<Option<Arc<PluginEngine>>>> = OnceLock::new();

fn engine_slot() -> &'static RwLock<Option<Arc<PluginEngine>>> {
    ENGINE.get_or_init(|| RwLock::new(None))
}

/// The directory the engine loaded from — remembered so `reload_engine`
/// can re-load without threading the state path through `Ctx`.
static LOADED_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn loaded_dir_slot() -> &'static Mutex<Option<PathBuf>> {
    LOADED_DIR.get_or_init(|| Mutex::new(None))
}

/// Load the engine from `dir` and install it (daemon startup).
pub fn set_engine_from(dir: PathBuf) {
    let engine = PluginEngine::load(&plugins_dir(&dir));
    *loaded_dir_slot().lock().expect("plugin dir lock") = Some(dir);
    *engine_slot().write().expect("plugin engine write lock") = Some(engine);
}

/// Re-load from the same directory (`reload-config`): an installed or
/// removed plugin takes effect without a daemon restart.
pub fn reload_engine() {
    let dir = loaded_dir_slot().lock().expect("plugin dir lock").clone();
    if let Some(dir) = dir {
        let engine = PluginEngine::load(&plugins_dir(&dir));
        *engine_slot().write().expect("plugin engine write lock") = Some(engine);
    }
}

/// Install `engine` directly (tests).
pub fn set_engine(engine: Arc<PluginEngine>) {
    *engine_slot().write().expect("plugin engine write lock") = Some(engine);
}

/// Swap in an engine and return the previous one (tests).
#[cfg(all(test, unix))]
pub(crate) fn swap_engine(engine: Option<Arc<PluginEngine>>) -> Option<Arc<PluginEngine>> {
    std::mem::replace(
        &mut *engine_slot().write().expect("plugin engine write lock"),
        engine,
    )
}

/// The broadcaster's dispatch: no engine or no matching hook is a no-op so
/// the un-plugged daemon pays one RwLock read per notification.
pub fn dispatch_notification(notification: &par_term_emu_core::tmux_control::TmuxNotification) {
    let Some(engine) = engine_slot()
        .read()
        .expect("plugin engine read lock")
        .clone()
    else {
        return;
    };
    let emitted = crate::mux::emit::emit(notification);
    let Some(first) = emitted.split_whitespace().next() else {
        return;
    };
    let Some(name) = first.strip_prefix('%') else {
        return;
    };
    if HOOK_EVENTS.contains(&name) {
        engine.dispatch(name, &emitted);
    }
}

/// Kill expired hooks across the loaded engine (the accept loop's idle
/// tick calls this; cheap when nothing runs).
pub fn reap_expired_hooks() {
    if let Some(engine) = engine_slot()
        .read()
        .expect("plugin engine read lock")
        .clone()
    {
        engine.reap_expired();
    }
}

/// Where the daemon's plugins live: `plugins/` beside the state file (the
/// same `par-mux` directory `agent-patterns/` lives in).
pub fn plugins_dir(par_mux_dir: &Path) -> PathBuf {
    par_mux_dir.join("plugins")
}

/// The `plugin install` registry entry, `plugins/installed.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct InstallationRecord {
    /// The installed plugin's name (its directory).
    pub name: String,
    /// The version recorded at install time.
    pub version: String,
    /// Where the plugin came from (a path or git URL, verbatim).
    pub source: String,
    /// Unix ms at install time.
    pub installed_unix_ms: u64,
}

/// Read `installed.json` under `plugins_dir` (absent = empty).
pub fn read_installations(plugins_dir: &Path) -> Result<Vec<InstallationRecord>, String> {
    let path = plugins_dir.join("installed.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Write `installed.json` (the installer owns every write).
pub fn write_installations(
    plugins_dir: &Path,
    records: &[InstallationRecord],
) -> Result<(), String> {
    std::fs::create_dir_all(plugins_dir)
        .map_err(|err| format!("{}: {err}", plugins_dir.display()))?;
    let path = plugins_dir.join("installed.json");
    let mut temp = path.clone().into_os_string();
    temp.push(format!(".tmp-{}", std::process::id()));
    let temp = PathBuf::from(temp);
    let body = serde_json::to_string_pretty(records).expect("installation records serialize");
    std::fs::write(&temp, body).map_err(|err| format!("{}: {err}", temp.display()))?;
    std::fs::rename(&temp, &path).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_text(name: &str, event: &str, command: &str) -> String {
        format!(
            "name = \"{name}\"\nversion = \"1\"\ndescription = \"test\"\n[[hook]]\nevent = \"{event}\"\ncommand = \"{command}\"\n"
        )
    }

    #[test]
    fn manifests_validate_events_and_timeouts() {
        assert!(parse_manifest(&manifest_text("ok", "pane-exited", "true")).is_ok());
        let missing_version = "name = \"x\"\n[[hook]]\nevent = \"exit\"\ncommand = \"true\"\n";
        assert!(parse_manifest(missing_version).is_err());
        assert!(parse_manifest(&manifest_text("x", "output", "true")).is_err());
        assert!(parse_manifest(&manifest_text("x", "pane-exited", "  ")).is_err());
        let timed = "name = \"x\"\nversion = \"1\"\n[[hook]]\nevent = \"pane-exited\"\ncommand = \"true\"\ntimeout-ms = 99999\n";
        assert!(parse_manifest(timed).is_err());
        assert_eq!(
            parse_manifest(&manifest_text("x", "pane-exited", "true"))
                .unwrap()
                .hooks[0]
                .effective_timeout_ms(),
            3_000
        );
    }

    #[test]
    fn event_names_come_from_the_emitter() {
        use par_term_emu_core::tmux_control::TmuxNotification;
        assert_eq!(
            event_name(&TmuxNotification::SessionsChanged).as_deref(),
            Some("sessions-changed")
        );
        // Not every notification is a plugin event (%output is not).
        assert_eq!(
            event_name(&TmuxNotification::PaneModeChanged {
                pane_id: "%1".into()
            }),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn dispatch_runs_matching_hooks_with_the_event_env() {
        let dir = tempfile::tempdir().expect("temp dir");
        let out = dir.path().join("hook-ran");
        let out_str = out
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('\"', "\\\"");
        let manifest = format!(
            "name = \"t\"\nversion = \"1\"\n[[hook]]\nevent = \"sessions-changed\"\ncommand = \"printf '%s' \\\"$PAR_MUX_EVENT\\\" > '{out_str}'\"\n"
        );
        let plugin_dir = dir.path().join("t");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join(MANIFEST_NAME), &manifest).unwrap();
        let engine = PluginEngine::load(dir.path());
        assert_eq!(engine.plugins().len(), 1);

        let previous = swap_engine(Some(engine.clone()));
        dispatch_notification(&par_term_emu_core::tmux_control::TmuxNotification::SessionsChanged);
        // The hook writes synchronously in the test command; wait bounded.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !out.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        swap_engine(previous);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "sessions-changed");
        engine.reap_expired();
    }

    #[test]
    fn an_invalid_plugin_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join(MANIFEST_NAME),
            "name = \"bad\"\nversion = \"1\"\n[[hook]]\nevent = \"nope\"\ncommand = \"true\"\n",
        )
        .unwrap();
        let engine = PluginEngine::load(dir.path());
        assert!(engine.plugins().is_empty());
    }

    #[test]
    fn installation_records_round_trip() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(read_installations(dir.path()).unwrap().is_empty());
        let record = InstallationRecord {
            name: "t".into(),
            version: "1".into(),
            source: "https://git.example/t".into(),
            installed_unix_ms: 42,
        };
        write_installations(dir.path(), std::slice::from_ref(&record)).unwrap();
        assert_eq!(read_installations(dir.path()).unwrap(), vec![record]);
    }
}
