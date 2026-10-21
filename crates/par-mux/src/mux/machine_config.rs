//! Saved machines (herdr's machine parity, the thin slice): SSH profiles
//! under `[machines.<label>]` and the ssh invocation a `--machine` relay
//! runs. Credentials never enter the file — the user's ssh config, agent,
//! and askpass own authentication.
//!
//! The config edits are surgical (`toml_edit`), the same comment-preserving
//! discipline the attach settings writer uses: a machine add/remove
//! replaces only its own table and leaves every other line byte-equal.

use std::collections::BTreeMap;

use crate::mux::config::MachineProfile;

/// The machines `text` declares, label → profile. `Err` names a parse
/// failure in the `[machines]` table itself.
pub fn machines_from(text: &str) -> Result<BTreeMap<String, MachineProfile>, String> {
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("config.toml does not parse: {err}"))?;
    let Some(table) = doc.get("machines") else {
        return Ok(BTreeMap::new());
    };
    let table = table
        .as_table_like()
        .ok_or_else(|| "config.toml's `machines` is not a table".to_string())?;
    let mut machines = BTreeMap::new();
    for (label, value) in table.iter() {
        let item = value
            .as_table_like()
            .ok_or_else(|| format!("machine `{label}` is not a table"))?;
        let field = |name: &str| -> Option<String> {
            item.get(name)
                .and_then(|value| value.as_str())
                .map(str::to_string)
        };
        let target = field("target").ok_or_else(|| format!("machine `{label}` has no target"))?;
        let session = field("session");
        let enabled = item
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        machines.insert(
            label.to_string(),
            MachineProfile {
                target,
                session,
                enabled,
            },
        );
    }
    Ok(machines)
}

/// `text` with `[machines.<label>]` upserted to `profile`: the table's own
/// lines are replaced, everything else — comments included — stays
/// byte-equal. `Err` names a parse failure; the caller writes nothing then.
pub fn set_machine(text: &str, label: &str, profile: &MachineProfile) -> Result<String, String> {
    if label.is_empty() || !label.chars().all(is_label_char) {
        return Err(format!(
            "machine labels are letters, digits, dashes, dots, and underscores: {label:?}"
        ));
    }
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("config.toml does not parse: {err}"))?;
    let mut table = toml_edit::Table::new();
    table["target"] = toml_edit::value(profile.target.as_str());
    if let Some(session) = profile.session.as_deref() {
        table["session"] = toml_edit::value(session);
    }
    if !profile.enabled {
        table["enabled"] = toml_edit::value(false);
    }
    let machines = doc
        .entry("machines")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_like_mut()
        .ok_or_else(|| "config.toml's `machines` is not a table".to_string())?;
    machines.insert(label, toml_edit::Item::Table(table));
    Ok(doc.to_string())
}

/// `text` with `[machines.<label>]` removed; unchanged (`Ok(text)`) when
/// the machine never existed.
pub fn remove_machine(text: &str, label: &str) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("config.toml does not parse: {err}"))?;
    if let Some(machines) = doc
        .get_mut("machines")
        .and_then(|item| item.as_table_like_mut())
    {
        machines.remove(label);
        if machines.is_empty() {
            doc.remove("machines");
        }
    }
    Ok(doc.to_string())
}

fn is_label_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')
}

/// Quote one word for a POSIX remote shell — openssh concatenates the
/// remote command and runs it through the login shell, so every relayed
/// argument must survive that shell verbatim.
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=,%@+^".contains(ch))
    {
        return word.to_string();
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for ch in word.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// The ssh invocation a `--machine <label>` relay runs: `ssh` with
/// non-interactive auth for one-shot commands, a forced pty for attach
/// (the remote passthrough client needs a terminal to raw; ssh then owns
/// the local terminal's raw mode), the profile's target, and the par-mux
/// command line rebuilt from the relayed arguments.
pub fn ssh_relay_command(target: &str, relayed_args: &[String], interactive: bool) -> Vec<String> {
    let mut command = vec!["ssh".to_string()];
    if interactive {
        command.push("-tt".to_string());
    } else {
        // A scripted relay must fail fast rather than hang on a prompt:
        // credentials live with the user's ssh agent/config, and a
        // missing key is an error, not a password question.
        command.extend(["-o".to_string(), "BatchMode=yes".to_string()]);
    }
    command.push(target.to_string());
    let mut remote = String::from("par-mux");
    for arg in relayed_args {
        remote.push(' ');
        remote.push_str(&shell_quote(arg));
    }
    command.push(remote);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_round_trip_preserves_comments_and_neighbors() {
        let original = "# paul's config\n[client]\nprefix = \"C-a\"\n\n# the lab box\n[machines]\n[machines.lab]\ntarget = \"paul@10.0.0.4\"\n";
        let updated = set_machine(
            original,
            "lab",
            &MachineProfile {
                target: "paul@10.0.0.5".into(),
                session: Some("main".into()),
                enabled: true,
            },
        )
        .unwrap();
        assert!(updated.contains("# paul's config"));
        assert!(updated.contains("# the lab box"));
        assert!(updated.contains("prefix = \"C-a\""));
        assert!(updated.contains("target = \"paul@10.0.0.5\""));
        assert!(updated.contains("session = \"main\""));
        assert!(!updated.contains("enabled"));

        let machines = machines_from(&updated).unwrap();
        assert_eq!(
            machines.get("lab"),
            Some(&MachineProfile {
                target: "paul@10.0.0.5".into(),
                session: Some("main".into()),
                enabled: true,
            })
        );
    }

    #[test]
    fn a_disabled_machine_round_trips_and_lists() {
        let text = set_machine(
            "",
            "off",
            &MachineProfile {
                target: "host".into(),
                session: None,
                enabled: false,
            },
        )
        .unwrap();
        let machines = machines_from(&text).unwrap();
        assert_eq!(machines.get("off").map(|m| m.enabled), Some(false));
    }

    #[test]
    fn remove_prunes_the_machine_and_the_empty_table() {
        let text = set_machine(
            "",
            "gone",
            &MachineProfile {
                target: "h".into(),
                session: None,
                enabled: true,
            },
        )
        .unwrap();
        let removed = remove_machine(&text, "gone").unwrap();
        assert!(!removed.contains("machines"));
        assert!(machines_from(&removed).unwrap().is_empty());
        // Removing an absent machine is a no-op, not an error.
        assert_eq!(remove_machine(&removed, "gone").unwrap(), removed);
    }

    #[test]
    fn odd_labels_are_refused() {
        assert!(set_machine(
            "",
            "bad label",
            &MachineProfile {
                target: "h".into(),
                session: None,
                enabled: true
            }
        )
        .is_err());
    }

    #[test]
    fn relay_commands_quote_for_the_remote_shell() {
        assert_eq!(
            ssh_relay_command(
                "paul@host",
                &["--cmd".into(), "send-keys -t %0 hi there".into()],
                false
            ),
            vec![
                "ssh".to_string(),
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                "paul@host".to_string(),
                "par-mux --cmd 'send-keys -t %0 hi there'".to_string(),
            ]
        );
        let interactive =
            ssh_relay_command("lab", &["attach".into(), "-t".into(), "a b".into()], true);
        assert_eq!(interactive[1], "-tt");
        assert_eq!(interactive[3], "par-mux attach -t 'a b'");
        assert_eq!(shell_quote("plain-arg.txt"), "plain-arg.txt");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
