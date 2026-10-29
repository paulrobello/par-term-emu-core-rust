//! Named layouts: whole-tree captures saved under `<base>/par-mux/layouts`.

use super::format::{PersistError, PersistState, FORMAT_VERSION};
use super::save::{write_job, SaveOrigin};
use super::state_file::platform_state_dir;
use crate::mux::tree::MuxTree;
use std::fs;
use std::path::{Path, PathBuf};

/// Where named layouts live: `<base>/par-mux/layouts`. `base` is the same
/// test seam as [`state_file_in`](super::state_file_in).
#[must_use]
pub fn layouts_dir_in(base: &Path) -> PathBuf {
    base.join("par-mux").join("layouts")
}

/// [`layouts_dir_in`] under the platform state dir.
#[must_use]
pub fn layouts_dir() -> PathBuf {
    layouts_dir_in(&platform_state_dir())
}

/// Whether `name` is a safe layout file stem: non-empty, only
/// `[A-Za-z0-9._-]`, and not `.` or `..`.
#[must_use]
pub fn valid_layout_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The layouts file for `name` under an explicit layouts directory — the
/// seam a persisting daemon dispatches through, so a `--state-dir` daemon
/// keeps the store beside its state file. A rejected name never reaches
/// the filesystem, exactly like the base-based [`layout_file_in`].
pub fn layout_file_in_dir(dir: &Path, name: &str) -> Result<PathBuf, PersistError> {
    if !valid_layout_name(name) {
        return Err(PersistError::InvalidLayoutName {
            name: name.to_string(),
        });
    }
    Ok(dir.join(format!("{name}.json")))
}

/// The file for layout `name` under the base whose layouts dir is
/// `<base>/par-mux/layouts`; a rejected name never reaches the filesystem.
pub fn layout_file_in(base: &Path, name: &str) -> Result<PathBuf, PersistError> {
    layout_file_in_dir(&layouts_dir_in(base), name)
}

/// Capture `tree` and write it as layout `name`, overwriting any previous
/// layout of that name. This borrows `tree` across the file write, so a
/// caller holding the tree lock should capture under it and call
/// [`save_layout_state_in`] after dropping it.
pub fn save_layout_in(base: &Path, tree: &MuxTree, name: &str) -> Result<PathBuf, PersistError> {
    save_layout_state_in(base, &tree.to_persist_state(), name)
}

/// Write an already-captured `state` as layout `name` under an explicit
/// layouts directory. Touches no tree, so it is safe off-lock.
pub fn save_layout_state_in_dir(
    dir: &Path,
    state: &PersistState,
    name: &str,
) -> Result<PathBuf, PersistError> {
    let target = layout_file_in_dir(dir, name)?;
    // `Reap` skips the last-good and snapshot-history siblings, which exist
    // for the daemon's boot state and would litter the layouts dir.
    write_job(SaveOrigin::Reap, state, &target)?;
    Ok(target)
}

/// Write an already-captured `state` as layout `name`, overwriting any
/// previous layout of that name. Touches no tree, so it is safe off-lock.
pub fn save_layout_state_in(
    base: &Path,
    state: &PersistState,
    name: &str,
) -> Result<PathBuf, PersistError> {
    save_layout_state_in_dir(&layouts_dir_in(base), state, name)
}

/// Read layout `name` from an explicit layouts directory; unlike
/// [`load_or_quarantine`](super::load_or_quarantine), a bad file is reported and left in place: it is
/// not the daemon's boot state.
pub fn load_layout_in_dir(dir: &Path, name: &str) -> Result<PersistState, PersistError> {
    let text = fs::read_to_string(layout_file_in_dir(dir, name)?)?;
    let state: PersistState = serde_json::from_str(&text)?;
    if state.format_version != FORMAT_VERSION {
        return Err(PersistError::UnsupportedVersion {
            found: state.format_version,
            supported: FORMAT_VERSION,
        });
    }
    Ok(state)
}

/// Read layout `name` under the base whose layouts dir is
/// `<base>/par-mux/layouts`.
pub fn load_layout_in(base: &Path, name: &str) -> Result<PersistState, PersistError> {
    load_layout_in_dir(&layouts_dir_in(base), name)
}

/// Sorted names of the saved layouts under an explicit layouts directory;
/// a missing directory yields none.
#[must_use]
pub fn list_layouts_in_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            path.file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| valid_layout_name(s))
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Sorted names of the saved layouts under the base whose layouts dir is
/// `<base>/par-mux/layouts`; a missing directory yields none.
#[must_use]
pub fn list_layouts_in(base: &Path) -> Vec<String> {
    list_layouts_in_dir(&layouts_dir_in(base))
}

/// Delete layout `name` from an explicit layouts directory; a missing
/// layout is the I/O not-found error.
pub fn kill_layout_in_dir(dir: &Path, name: &str) -> Result<(), PersistError> {
    fs::remove_file(layout_file_in_dir(dir, name)?)?;
    Ok(())
}

/// Delete layout `name` under the base whose layouts dir is
/// `<base>/par-mux/layouts`; a missing layout is the I/O not-found error.
pub fn kill_layout_in(base: &Path, name: &str) -> Result<(), PersistError> {
    kill_layout_in_dir(&layouts_dir_in(base), name)
}
