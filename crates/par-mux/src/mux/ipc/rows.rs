//! Typed reply rows for the daemon's fixed-shape listings, shared by the
//! daemon emitters and the clients that parse them (ARC-130).
//!
//! Each row's `Display` is the exact wire line the daemon writes, and its
//! `parse` reads that line back. A row type owns its column grammar, so
//! the emitter and every parser cannot drift apart.

use std::fmt;

/// Whether `s` is `<prefix><digits>` (a typed id such as `$3`, `@0`, `+1`).
fn is_typed_id(s: &str, prefix: char) -> bool {
    s.strip_prefix(prefix)
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// One `list-windows -t <session>` row: `@N <marker> <name>`, marker `*`
/// for the session's active window and `-` otherwise. The name is the
/// line remainder, so a spaced name survives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRow {
    /// The window id, `@N`.
    pub id: String,
    /// Whether this is the session's active window.
    pub active: bool,
    /// The window name (may contain spaces; may be empty).
    pub name: String,
}

impl fmt::Display for WindowRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let marker = if self.active { '*' } else { '-' };
        write!(f, "{} {marker} {}", self.id, self.name)
    }
}

impl WindowRow {
    /// Parse one row; `None` for a line that is not `@N <marker> …`.
    pub fn parse(line: &str) -> Option<Self> {
        let (id, rest) = line.split_once(' ')?;
        if !is_typed_id(id, '@') {
            return None;
        }
        let (marker, name) = rest.split_once(' ').unwrap_or((rest, ""));
        let active = match marker {
            "*" => true,
            "-" => false,
            _ => return None,
        };
        Some(Self {
            id: id.to_string(),
            active,
            name: name.to_string(),
        })
    }
}

/// One `list-sessions` row: `+W: wname: $N: name`. The session id is the
/// LAST `$N:` marker, so a workspace name containing `: ` or `$` still
/// parses; the bare `$N: name` shape of a pre-workspaces daemon parses
/// with no workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// The owning workspace as `(id, name)`, when the daemon sent one.
    pub workspace: Option<(String, String)>,
    /// The session id, `$N`.
    pub id: String,
    /// The session name.
    pub name: String,
}

impl fmt::Display for SessionRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((ws_id, ws_name)) = &self.workspace {
            write!(f, "{ws_id}: {ws_name}: ")?;
        }
        write!(f, "{}: {}", self.id, self.name)
    }
}

impl SessionRow {
    /// Parse one row; `None` when the line carries no `$N: ` marker.
    pub fn parse(line: &str) -> Option<Self> {
        let idx = line.rfind('$')?;
        let (id, name) = line[idx..].split_once(": ")?;
        if !is_typed_id(id, '$') {
            return None;
        }
        let head = &line[..idx];
        let workspace = head
            .strip_suffix(": ")
            .and_then(|prefix| prefix.split_once(": "))
            .filter(|(ws_id, _)| is_typed_id(ws_id, '+'))
            .map(|(ws_id, ws_name)| (ws_id.to_string(), ws_name.to_string()));
        Some(Self {
            workspace,
            id: id.to_string(),
            name: name.to_string(),
        })
    }
}

/// One `list-workspaces` row: `+N: name`, with ` active` appended on the
/// daemon's active workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// The workspace id, `+N`.
    pub id: String,
    /// The workspace name.
    pub name: String,
    /// Whether this is the daemon's active workspace.
    pub active: bool,
}

impl fmt::Display for WorkspaceRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.id, self.name)?;
        if self.active {
            f.write_str(" active")?;
        }
        Ok(())
    }
}

impl WorkspaceRow {
    /// Parse one row; `None` for a line that is not `+N: …`.
    pub fn parse(line: &str) -> Option<Self> {
        let (id, rest) = line.split_once(": ")?;
        if !is_typed_id(id, '+') {
            return None;
        }
        let (name, active) = match rest.strip_suffix(" active") {
            Some(name) => (name, true),
            None => (rest, false),
        };
        Some(Self {
            id: id.to_string(),
            name: name.to_string(),
            active,
        })
    }
}

/// One `list-agents` roster row: `%N <agent> <state> <source>` then zero
/// or more whitespace-free `key=value` tokens (`reason=`, `telemetry=`,
/// `host_telemetry=`), kept in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    /// The pane id, `%N`.
    pub pane: String,
    /// The agent name the hook or rule reported.
    pub agent: String,
    /// The agent state.
    pub state: String,
    /// `hook` or `scrape`.
    pub source: String,
    /// The trailing `key=value` tokens, in wire order.
    pub extras: Vec<(String, String)>,
}

impl fmt::Display for AgentRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.pane, self.agent, self.state, self.source
        )?;
        for (key, value) in &self.extras {
            write!(f, " {key}={value}")?;
        }
        Ok(())
    }
}

impl AgentRow {
    /// Parse one row; `None` when the four positional columns are not
    /// all present, the pane is not `%N`, or a trailing token is not
    /// `key=value`.
    pub fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        let pane = fields.next()?;
        if !is_typed_id(pane, '%') {
            return None;
        }
        let agent = fields.next()?.to_string();
        let state = fields.next()?.to_string();
        let source = fields.next()?.to_string();
        let extras = fields
            .map(|token| {
                token
                    .split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            pane: pane.to_string(),
            agent,
            state,
            source,
            extras,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, active: bool, name: &str) -> WindowRow {
        WindowRow {
            id: id.to_string(),
            active,
            name: name.to_string(),
        }
    }

    /// Golden wire lines: the emitters' exact bytes before ARC-130.
    #[test]
    fn rows_format_the_golden_wire_lines() {
        assert_eq!(window("@1", true, "two").to_string(), "@1 * two");
        assert_eq!(window("@4", false, "b win").to_string(), "@4 - b win");
        let session = SessionRow {
            workspace: Some(("+0".to_string(), "main".to_string())),
            id: "$3".to_string(),
            name: "work".to_string(),
        };
        assert_eq!(session.to_string(), "+0: main: $3: work");
        let ws = WorkspaceRow {
            id: "+1".to_string(),
            name: "beta".to_string(),
            active: true,
        };
        assert_eq!(ws.to_string(), "+1: beta active");
        let agent = AgentRow {
            pane: "%2".to_string(),
            agent: "claude".to_string(),
            state: "blocked".to_string(),
            source: "hook".to_string(),
            extras: vec![("reason".to_string(), "aGk=".to_string())],
        };
        assert_eq!(agent.to_string(), "%2 claude blocked hook reason=aGk=");
    }

    #[test]
    fn window_row_round_trips() {
        for row in [
            window("@0", true, "main"),
            window("@12", false, "a b  c"),
            window("@3", false, ""),
        ] {
            assert_eq!(WindowRow::parse(&row.to_string()), Some(row));
        }
        assert_eq!(WindowRow::parse("@0: main"), None, "the bare form");
        assert_eq!(WindowRow::parse("$0 @1 * x"), None, "the -a form");
    }

    /// Workspace-prefixed rows and names containing `: ` and `$` (the
    /// `$0:` bug: a whitespace split kept the colon on the id).
    #[test]
    fn session_row_round_trips() {
        for row in [
            SessionRow {
                workspace: Some(("+0".to_string(), "main".to_string())),
                id: "$0".to_string(),
                name: "work".to_string(),
            },
            SessionRow {
                workspace: Some(("+7".to_string(), "a: b $x".to_string())),
                id: "$12".to_string(),
                name: "x: y".to_string(),
            },
            SessionRow {
                workspace: None,
                id: "$0".to_string(),
                name: "bare".to_string(),
            },
        ] {
            assert_eq!(SessionRow::parse(&row.to_string()), Some(row));
        }
        let parsed = SessionRow::parse("+0: main: $0: work").expect("parses");
        assert_eq!(parsed.id, "$0", "no trailing colon on the id");
        assert_eq!(SessionRow::parse("$x: nope"), None);
    }

    #[test]
    fn workspace_row_round_trips() {
        for (name, active) in [("main", true), ("a: b", false), ("x active y", false)] {
            let row = WorkspaceRow {
                id: "+3".to_string(),
                name: name.to_string(),
                active,
            };
            assert_eq!(WorkspaceRow::parse(&row.to_string()), Some(row));
        }
        assert_eq!(WorkspaceRow::parse("$0: work"), None);
    }

    #[test]
    fn agent_row_round_trips() {
        let row = AgentRow {
            pane: "%9".to_string(),
            agent: "pi".to_string(),
            state: "working".to_string(),
            source: "scrape".to_string(),
            extras: vec![
                ("telemetry".to_string(), "e30=".to_string()),
                ("host_telemetry".to_string(), "e30=".to_string()),
            ],
        };
        assert_eq!(AgentRow::parse(&row.to_string()), Some(row));
        assert_eq!(AgentRow::parse("%1 pi working"), None, "source missing");
        assert_eq!(AgentRow::parse("%1 pi working hook stray"), None);
    }
}
