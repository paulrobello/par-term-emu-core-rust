//! The attach client's persisted UI state (herdr's client chrome
//! preferences): what the user dragged or toggled, kept apart from the
//! config file the user edits. One JSON file beside `config.toml`, so a
//! `PAR_MUX_CONFIG` override (every test fixture sets one) relocates it
//! with the config.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The persisted side-panel state. Every field is optional: an absent
/// one leaves the config/default value in force.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub(crate) struct ClientPrefs {
    /// The expanded panel's width in columns (the right-edge drag).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<u16>,
    /// The workspace area's share of the panel's rows (the rule drag).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_split: Option<f32>,
    /// The panel is collapsed to the status rail (the chevron).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_collapsed: Option<bool>,
}

/// `<config dir>/par-mux/client-state.json`, beside the config file;
/// `None` when no config location is known.
pub(crate) fn prefs_path() -> Option<PathBuf> {
    let config = crate::mux::config::config_file_path()?;
    Some(config.parent()?.join("client-state.json"))
}

/// Read the prefs at `path`; a missing or unreadable file is the
/// defaults (UI state must never block an attach).
pub(crate) fn load(path: &Path) -> ClientPrefs {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

/// Write `prefs` to `path` through a sibling temp file and a rename, so
/// a concurrent reader (another attach client launching) never sees a
/// torn file.
pub(crate) fn store(path: &Path, prefs: &ClientPrefs) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    let body = serde_json::to_vec_pretty(prefs).map_err(|err| err.to_string())?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, body).map_err(|err| format!("{}: {err}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {err}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_tolerate_garbage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("client-state.json");
        assert_eq!(load(&path), ClientPrefs::default(), "missing file");
        let prefs = ClientPrefs {
            sidebar_width: Some(31),
            sidebar_split: Some(0.25),
            sidebar_collapsed: Some(true),
        };
        store(&path, &prefs).expect("store");
        assert_eq!(load(&path), prefs);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "the temp file renamed away");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), ClientPrefs::default(), "garbage is defaults");
        std::fs::write(&path, r#"{"sidebar_width":40}"#).unwrap();
        assert_eq!(
            load(&path),
            ClientPrefs {
                sidebar_width: Some(40),
                ..ClientPrefs::default()
            },
            "a partial file keeps the rest unset"
        );
    }
}
