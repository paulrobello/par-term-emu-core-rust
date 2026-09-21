//! Control-mode command parsing (client → server).

use crate::mux::ids::PaneId;

/// A command received from a control-mode client.
///
/// Phase 1 implements the four commands that prove the spine end to end.
/// Additional commands are new variants plus new dispatch arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxCommand {
    /// Create a session, optionally named.
    NewSession {
        /// Session name; a default is chosen when absent.
        name: Option<String>,
    },
    /// List every live pane.
    ListPanes,
    /// Send literal keys to a pane.
    SendKeys {
        /// Target pane.
        pane: PaneId,
        /// Literal text to write to the pane's PTY.
        keys: String,
    },
    /// Kill a pane.
    KillPane {
        /// Target pane.
        pane: PaneId,
    },
    /// Replay a pane's current screen to the requesting client.
    RefreshClient {
        /// Target pane.
        pane: PaneId,
    },
}

/// Parse one command line from a client.
///
/// Deliberately minimal: whitespace-split with a `-t`/`-s` flag scan. tmux's
/// real argument grammar (quoting, `--`, per-command option tables) is Phase 2
/// work, and pretending to implement it here would hide that.
pub fn parse_command(line: &str) -> Result<MuxCommand, String> {
    let parts: Vec<&str> = line.trim().split_whitespace().collect();
    let Some((name, args)) = parts.split_first() else {
        return Err("empty command".to_string());
    };

    let flag = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| *a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|v| (*v).to_string())
    };

    let target_pane = |flag_name: &str| -> Result<PaneId, String> {
        let raw = flag(flag_name).ok_or_else(|| format!("{name} requires {flag_name}"))?;
        raw.parse::<PaneId>()
            .map_err(|_| format!("invalid pane target: {raw}"))
    };

    match *name {
        "new-session" => Ok(MuxCommand::NewSession { name: flag("-s") }),
        "list-panes" => Ok(MuxCommand::ListPanes),
        "kill-pane" => Ok(MuxCommand::KillPane {
            pane: target_pane("-t")?,
        }),
        "refresh-client" => Ok(MuxCommand::RefreshClient {
            pane: target_pane("-t")?,
        }),
        "send-keys" => {
            let pane = target_pane("-t")?;
            // Everything after the `-t <target>` pair is the payload.
            let keys = args
                .iter()
                .skip_while(|a| **a != "-t")
                .skip(2)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            Ok(MuxCommand::SendKeys { pane, keys })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_session_with_a_name() {
        let cmd = parse_command("new-session -s work").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::NewSession {
                name: Some("work".into())
            }
        );
    }

    #[test]
    fn parses_new_session_without_a_name() {
        let cmd = parse_command("new-session").expect("parses");
        assert_eq!(cmd, MuxCommand::NewSession { name: None });
    }

    #[test]
    fn parses_send_keys_with_a_target() {
        let cmd = parse_command("send-keys -t %3 hello").expect("parses");
        assert_eq!(
            cmd,
            MuxCommand::SendKeys {
                pane: PaneId(3),
                keys: "hello".into()
            }
        );
    }

    #[test]
    fn parses_list_panes_and_kill_pane() {
        assert_eq!(parse_command("list-panes").unwrap(), MuxCommand::ListPanes);
        assert_eq!(
            parse_command("kill-pane -t %7").unwrap(),
            MuxCommand::KillPane { pane: PaneId(7) }
        );
    }

    #[test]
    fn rejects_an_unknown_command() {
        assert!(parse_command("frobnicate").is_err());
    }

    #[test]
    fn rejects_a_malformed_target() {
        assert!(parse_command("kill-pane -t notapane").is_err());
        assert!(
            parse_command("kill-pane").is_err(),
            "kill-pane needs a target"
        );
    }
}
