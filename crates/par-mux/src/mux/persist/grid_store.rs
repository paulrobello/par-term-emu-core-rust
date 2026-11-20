//! The grid sidecar store (card 01a1fa94ce7a): each pane's terminal
//! snapshot lives in its own content-addressed file beside the state
//! file, never inside it. The state file is pane metadata only — a
//! reader that wants cwd or session identity parses O(panes) bytes, not
//! the 200 MB of grid mass one tree can carry — and a save rewrites only
//! the panes whose content changed (the revision token IS the snapshot
//! cache's validity key, so an unchanged pane's file already exists and
//! the write is skipped).
//!
//! Layout beside `<target>`: `<stem>.panes/pane-<id>-<rev>.grid.json`.
//! A revision file is written once per (daemon life, content key) and
//! referenced by name from the metadata; the last-good and snapshot
//! tiers copy only the small metadata file, and the GC pass keeps every
//! revision those tiers still reference, so a recovery restore finds its
//! grids alive.

use super::format::{PersistError, PersistState};
use par_term_emu_core::terminal::replay_snapshot::TerminalSnapshot;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One pane's grid, split out of the metadata state on its way to its
/// own file.
pub(crate) struct SplitGrid {
    /// The pane's `%N` number.
    pub pane_id: u32,
    /// The revision token the metadata file references.
    pub rev: String,
    /// The snapshot itself.
    pub snapshot: TerminalSnapshot,
}

/// Where `target`'s grid files live: the `<stem>.panes` directory beside
/// it (`par-mux-default.state.json` → `par-mux-default.panes`).
pub(super) fn grid_store_dir(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let stem = name
        .strip_suffix(".state.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(name);
    let dir = format!("{stem}.panes");
    match target.parent() {
        Some(parent) => parent.join(dir),
        None => PathBuf::from(dir),
    }
}

/// The grid file name for one pane revision.
fn grid_file_name(pane_id: u32, rev: &str) -> String {
    format!("pane-{pane_id}-{rev}.grid.json")
}

/// Strip every pane terminal out of `state` (in place), returning what
/// was removed. The metadata's pane records keep their `grid_rev`
/// reference — the serialized file carries no grid bytes.
pub(crate) fn split_grids_out_of(state: &mut PersistState) -> Vec<SplitGrid> {
    let mut grids = Vec::new();
    for session in &mut state.sessions {
        for window in &mut session.windows {
            for pane in &mut window.panes {
                if let Some(snapshot) = pane.terminal.take() {
                    if let Some(rev) = pane.grid_rev.clone() {
                        grids.push(SplitGrid {
                            pane_id: pane.id,
                            rev,
                            snapshot,
                        });
                    }
                }
            }
        }
    }
    grids
}

/// Land the split grids as files, skipping any whose exact revision file
/// already exists (same revision ⇒ same snapshot-cache key ⇒ same
/// bytes within one daemon life). Returns the file names the current
/// metadata owns, for the GC pass.
pub(crate) fn write_grid_files(
    target: &Path,
    grids: &[SplitGrid],
) -> Result<HashSet<String>, PersistError> {
    let mut keep = HashSet::new();
    if grids.is_empty() {
        return Ok(keep);
    }
    let dir = grid_store_dir(target);
    fs::create_dir_all(&dir)?;
    for grid in grids {
        let name = grid_file_name(grid.pane_id, &grid.rev);
        let path = dir.join(&name);
        if !path.exists() {
            write_grid_file(&path, &grid.snapshot)?;
        }
        keep.insert(name);
    }
    Ok(keep)
}

/// One grid file's atomic write: serialize to `<file>.tmp` (`0600` from
/// creation), fsync, rename — the same discipline as the state file
/// itself. Grid files hold pane content, so they keep its owner-only
/// posture.
fn write_grid_file(path: &Path, snapshot: &TerminalSnapshot) -> Result<(), PersistError> {
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
    serde_json::to_writer(&mut writer, snapshot)?;
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

/// Remove grid files no recovery tier references anymore: the current
/// metadata's own revisions plus every revision named by the last-good
/// snapshot and the kept history snapshots (each a small metadata file
/// now, cheap to read for its references). Best-effort — a failure warns
/// and leaves the file; disk hygiene, not correctness (a stale file is
/// unreachable: nothing references it by name).
pub(crate) fn gc_grid_files(target: &Path, keep: &HashSet<String>) {
    let mut referenced = keep.clone();
    let mut tiers: Vec<PathBuf> = vec![super::save::lastgood_path(target)];
    tiers.extend(super::save::kept_snapshots(target));
    for tier in tiers {
        let Ok(text) = fs::read_to_string(&tier) else {
            continue;
        };
        if !collect_grid_refs(&text, &mut referenced) {
            // An unparseable recovery tier: keeping every grid file this
            // pass is cheaper than guessing what it might restore.
            return;
        }
    }
    let dir = grid_store_dir(target);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut remaining = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        if name.ends_with(".grid.json") && !referenced.contains(&name) {
            if let Err(err) = fs::remove_file(entry.path()) {
                log::warn!(
                    "par-mux: stale grid file {} could not be removed: {err}",
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

/// Pull every `pane-<id>-<rev>.grid.json` name a tier metadata JSON
/// references into `into`. Returns false when the tier fails to parse —
/// the caller then skips the whole GC pass rather than dropping files an
/// unreadable recovery tier might still restore.
fn collect_grid_refs(metadata_json: &str, into: &mut HashSet<String>) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(metadata_json) else {
        return false;
    };
    let Some(sessions) = value["sessions"].as_array() else {
        return true;
    };
    for session in sessions {
        let Some(windows) = session["windows"].as_array() else {
            continue;
        };
        for window in windows {
            let Some(panes) = window["panes"].as_array() else {
                continue;
            };
            for pane in panes {
                let (Some(id), Some(rev)) = (pane["id"].as_u64(), pane["grid_rev"].as_str()) else {
                    continue;
                };
                into.insert(grid_file_name(u32::try_from(id).unwrap_or(u32::MAX), rev));
            }
        }
    }
    true
}

/// Merge `target`'s grid files back into `state` at load: the inverse of
/// [`split_grids_out_of`]. A missing or unreadable grid file leaves that
/// pane's terminal `None` — the restore rebuilds its structure and
/// process without the saved screen, never a failed boot.
pub(crate) fn attach_grids(state: &mut PersistState, target: &Path) {
    let dir = grid_store_dir(target);
    for session in &mut state.sessions {
        for window in &mut session.windows {
            for pane in &mut window.panes {
                let (Some(rev), true) = (&pane.grid_rev, pane.terminal.is_none()) else {
                    continue;
                };
                let path = dir.join(grid_file_name(pane.id, rev));
                if let Some(snapshot) = read_grid_file(&path) {
                    pane.terminal = Some(snapshot);
                } else {
                    log::warn!(
                        "par-mux: pane {}'s grid file {} is missing or unreadable; restoring \
                         the pane without its saved screen",
                        pane.id,
                        path.display()
                    );
                }
            }
        }
    }
}

/// Read and parse one grid file; `None` on any read or parse failure.
fn read_grid_file(path: &Path) -> Option<TerminalSnapshot> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
