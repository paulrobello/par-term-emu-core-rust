//! The settings dialog's config writes: one `[client]` key set in the
//! user's `config.toml` in place (herdr's `config::write_edit`), every
//! other byte — comments, layout, unrelated keys — preserved. A file that
//! does not parse is never overwritten.

use std::path::Path;

/// One `[client]` value the dialog writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingValue {
    Bool(bool),
    Str(String),
    List(Vec<String>),
}

/// `text` with `[client] <key> = value` set: the key's existing entry is
/// replaced in place (its decor kept), else it is appended to the
/// `[client]` table, which is created when absent. `Err` names a parse
/// failure — the caller writes nothing then.
pub(crate) fn set_client_key(
    text: &str,
    key: &str,
    value: &SettingValue,
) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("config.toml does not parse: {err}"))?;
    if !doc.contains_key("client") {
        doc["client"] = toml_edit::table();
    }
    let client = doc["client"]
        .as_table_mut()
        .ok_or_else(|| "config.toml's `client` is not a table".to_string())?;
    let new = match value {
        SettingValue::Bool(b) => toml_edit::value(*b),
        SettingValue::Str(s) => toml_edit::value(s.as_str()),
        SettingValue::List(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(item.as_str());
            }
            toml_edit::value(array)
        }
    };
    match client.get_mut(key).and_then(|item| item.as_value_mut()) {
        // Keep the entry's own decor (an inline comment after it).
        Some(existing) => {
            let decor = existing.decor().clone();
            if let Ok(mut value) = new.into_value() {
                *value.decor_mut() = decor;
                *existing = value;
            }
        }
        None => client[key] = new,
    }
    Ok(doc.to_string())
}

/// Set `[client] <key>` in the file at `path` (created when absent),
/// through a sibling temp file and a rename.
pub(crate) fn write_client_key(path: &Path, key: &str, value: &SettingValue) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("{}: {err}", path.display())),
    };
    let updated = set_client_key(&text, key, value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".tmp-{}", std::process::id()));
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, updated).map_err(|err| format!("{}: {err}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {err}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key replaced in place keeps every comment and unrelated key; a
    /// new key lands in `[client]`; a missing table is created; a file
    /// that does not parse is refused.
    #[test]
    fn set_client_key_preserves_the_user_file() {
        let text = "# my config\n[client]\n# the prefix I like\nprefix = \"C-a\"\nnotify-delivery = \"terminal\" # loud\n\n[daemon]\nexit-empty = false\n";
        let out = set_client_key(
            text,
            "notify-delivery",
            &SettingValue::Str("system".to_string()),
        )
        .unwrap();
        assert_eq!(
            out,
            "# my config\n[client]\n# the prefix I like\nprefix = \"C-a\"\nnotify-delivery = \"system\" # loud\n\n[daemon]\nexit-empty = false\n"
        );
        let out = set_client_key(
            &out,
            "notify-states",
            &SettingValue::List(vec!["blocked".to_string()]),
        )
        .unwrap();
        assert!(out.contains("notify-states = [\"blocked\"]"), "{out}");
        assert!(out.contains("# the prefix I like"), "{out}");
        let parsed: crate::mux::config::ConfigFile = toml::from_str(&out).unwrap();
        assert_eq!(parsed.client.prefix.as_deref(), Some("C-a"));
        assert_eq!(parsed.daemon.exit_empty, Some(false));
        let fresh = set_client_key("", "sidebar-on-launch", &SettingValue::Bool(false)).unwrap();
        assert_eq!(fresh, "[client]\nsidebar-on-launch = false\n");
        assert!(set_client_key("[client\nbroken", "x", &SettingValue::Bool(true)).is_err());
    }

    #[test]
    fn write_client_key_creates_and_updates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("par-mux").join("config.toml");
        write_client_key(&path, "border-lines", &SettingValue::Str("heavy".into())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[client]\nborder-lines = \"heavy\"\n"
        );
        std::fs::write(&path, "[client\n").unwrap();
        assert!(write_client_key(&path, "border-lines", &SettingValue::Str("x".into())).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[client\n",
            "a broken file is left alone"
        );
    }
}
