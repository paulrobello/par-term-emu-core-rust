//! The env sidecar store (card 01a1fa94a26c): pane agent-env captures and
//! session env maps live OUTSIDE the state file. The agent capture sweeps
//! up every secret the pane's agent ran with (`PD_RTDB_TOKEN_*` and
//! friends), and the single JSON every downstream reader parses must not
//! carry them — the sidecars keep the same owner-only posture as the
//! state file while the state file itself stays env-free.
//!
//! Layout beside `<target>`: `<stem>.env/pane-<id>.json` and
//! `<stem>.env/session-<id>.json`, each written atomically `0600`. A pane
//! record carries the claim identity it belongs to, so a load pairs a
//! sidecar with the main file's claim only when both name the same agent
//! session — a stale sidecar from a relabeled pane never leaks in.

use super::format::{PersistError, PersistState};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One pane's external env record. `session` is the identity key the save
/// side's cache uses (`session_id`, else `session_path`); a load merges the
/// record only when the main file's claim names the same pair.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct PaneEnvRecord {
    /// The claim's agent label.
    pub agent: String,
    /// The claim's session identity (`session_id`, else `session_path`),
    /// when either exists.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub session: Option<String>,
    /// The captured, spawn-stripped environment.
    pub env: BTreeMap<String, String>,
}

/// One session's external env record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct SessionEnvRecord {
    /// The session environment.
    pub env: BTreeMap<String, String>,
}

/// What [`split_env_out_of`] pulled out of a state: the records the sidecar
/// writer lands, keyed by their owner ids.
#[derive(Debug, Default)]
pub(crate) struct ExtractedEnv {
    /// Pane id → the pane's agent-env record.
    pub panes: BTreeMap<u32, PaneEnvRecord>,
    /// Session id → the session's env record.
    pub sessions: BTreeMap<u32, SessionEnvRecord>,
}

/// Where `target`'s env sidecars live: the `<stem>.env` directory beside it
/// (`par-mux-default.state.json` → `par-mux-default.env`).
pub(super) fn env_store_dir(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let stem = name
        .strip_suffix(".state.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(name);
    let dir = format!("{stem}.env");
    match target.parent() {
        Some(parent) => parent.join(dir),
        None => PathBuf::from(dir),
    }
}

/// Strip every env map out of `state` (in place), returning what was
/// removed. `write_job` serializes the stripped state — the state file
/// itself never carries env — and lands the removed maps as sidecars.
pub(crate) fn split_env_out_of(state: &mut PersistState) -> ExtractedEnv {
    let mut extracted = ExtractedEnv::default();
    for session in &mut state.sessions {
        if !session.env.is_empty() {
            extracted.sessions.insert(
                session.id,
                SessionEnvRecord {
                    env: std::mem::take(&mut session.env),
                },
            );
        }
        for window in &mut session.windows {
            for pane in &mut window.panes {
                let Some(agent) = &mut pane.agent_session else {
                    continue;
                };
                if let Some(env) = agent.env.take() {
                    extracted.panes.insert(
                        pane.id,
                        PaneEnvRecord {
                            agent: agent.agent.clone(),
                            session: agent
                                .session_id
                                .clone()
                                .or_else(|| agent.session_path.clone()),
                            env,
                        },
                    );
                }
            }
        }
    }
    extracted
}

/// Land `extracted` as sidecars beside `target`, returning the file names
/// the current state owns — everything else in the store is stale by the
/// next GC pass. Panes and sessions whose env is absent get no file (an
/// unreadable capture must not resurrect stale env from an older sidecar).
pub(crate) fn write_env_records(
    target: &Path,
    extracted: &ExtractedEnv,
) -> Result<HashSet<String>, PersistError> {
    let mut keep = HashSet::new();
    if extracted.panes.is_empty() && extracted.sessions.is_empty() {
        return Ok(keep);
    }
    let dir = env_store_dir(target);
    fs::create_dir_all(&dir)?;
    for (pane_id, record) in &extracted.panes {
        let name = format!("pane-{pane_id}.json");
        write_sidecar(&dir.join(&name), record)?;
        keep.insert(name);
    }
    for (session_id, record) in &extracted.sessions {
        let name = format!("session-{session_id}.json");
        write_sidecar(&dir.join(&name), record)?;
        keep.insert(name);
    }
    Ok(keep)
}

/// One sidecar's atomic write: serialize to `<file>.tmp` (`0600` from
/// creation, same posture as the state file), fsync, rename.
fn write_sidecar<T: serde::Serialize>(path: &Path, record: &T) -> Result<(), PersistError> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let file = {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&tmp)?
    };
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, record)?;
    let file = writer
        .into_inner()
        .map_err(|err| PersistError::Io(err.into_error()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Remove sidecar files the current state no longer owns. Best-effort: a
/// failure warns and leaves the stale file — the load side's identity
/// check is the real guard, this is hygiene.
pub(crate) fn gc_env_sidecars(target: &Path, keep: &HashSet<String>) {
    let dir = env_store_dir(target);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut remaining = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        if name.ends_with(".json") && !keep.contains(&name) {
            if let Err(err) = fs::remove_file(entry.path()) {
                log::warn!(
                    "par-mux: stale env sidecar {} could not be removed: {err}",
                    entry.path().display()
                );
                remaining += 1;
            }
        } else if !name.ends_with(".tmp") {
            remaining += 1;
        }
    }
    if remaining == 0 {
        let _ = fs::remove_dir(&dir);
    }
}

/// Merge `target`'s env sidecars back into `state` at load: the inverse of
/// [`split_env_out_of`]. A pane record lands only when the main file's
/// claim names the same agent session; a session record replaces the
/// session's (empty) inline map. Best-effort — a missing or unreadable
/// store leaves the inline state exactly as parsed.
pub(crate) fn attach_env_sidecars(state: &mut PersistState, target: &Path) {
    let dir = env_store_dir(target);
    for session in &mut state.sessions {
        let path = dir.join(format!("session-{}.json", session.id));
        if let Some(record) = read_sidecar::<SessionEnvRecord>(&path) {
            session.env = record.env;
        }
        for window in &mut session.windows {
            for pane in &mut window.panes {
                let Some(agent) = &mut pane.agent_session else {
                    continue;
                };
                let path = dir.join(format!("pane-{}.json", pane.id));
                let Some(record) = read_sidecar::<PaneEnvRecord>(&path) else {
                    continue;
                };
                let identity = agent
                    .session_id
                    .clone()
                    .or_else(|| agent.session_path.clone());
                if record.agent == agent.agent && record.session == identity {
                    agent.env = Some(record.env);
                }
            }
        }
    }
}

/// Read and parse one sidecar; `None` on any read or parse failure (the
/// store is a derived cache, not evidence).
fn read_sidecar<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
