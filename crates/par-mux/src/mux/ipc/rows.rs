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
/// `host_telemetry=`, `ws=<+N>`), kept in wire order — unknown keys are
/// kept, not rejected, so a newer daemon never breaks the parse.
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

    /// The owning workspace (`ws=` extra), `None` from a pre-`ws` daemon.
    pub fn workspace(&self) -> Option<&str> {
        self.extras
            .iter()
            .find(|(k, _)| k == "ws")
            .map(|(_, v)| v.as_str())
    }
}

/// Fuzz/test invariant (ENH-060): every parser either rejects `line` or
/// returns a row whose `Display` parses back to the same row.
#[cfg(any(test, fuzzing))]
pub fn assert_round_trip(line: &str) {
    if let Some(row) = WindowRow::parse(line) {
        assert_eq!(WindowRow::parse(&row.to_string()), Some(row), "window: {line:?}");
    }
    if let Some(row) = SessionRow::parse(line) {
        assert_eq!(SessionRow::parse(&row.to_string()), Some(row), "session: {line:?}");
    }
    if let Some(row) = WorkspaceRow::parse(line) {
        assert_eq!(
            WorkspaceRow::parse(&row.to_string()),
            Some(row),
            "workspace: {line:?}"
        );
    }
    if let Some(row) = AgentRow::parse(line) {
        assert!(
            row.extras.iter().all(|(k, _)| !k.contains('=')),
            "agent key kept '=': {line:?}"
        );
        assert_eq!(AgentRow::parse(&row.to_string()), Some(row), "agent: {line:?}");
    }
}

/// One `pane-info -t <pane>` reply line: `%N @W COLSxROWS`, then optional
/// whitespace-free `cmd=<base64>` / `exited=<code|?>` tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfoRow {
    /// The pane id, `%N`.
    pub pane: String,
    /// The owning window, `@W`.
    pub window: String,
    /// The pane terminal's grid, `(cols, rows)`.
    pub size: (usize, usize),
    /// The foreground command, still base64 as on the wire.
    pub cmd: Option<String>,
    /// `None` live; `Some(None)` held-dead with an unreadable code (`?`);
    /// `Some(Some(c))` exit code.
    pub exited: Option<Option<i32>>,
}

impl fmt::Display for PaneInfoRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}x{}",
            self.pane, self.window, self.size.0, self.size.1
        )?;
        if let Some(cmd) = &self.cmd {
            write!(f, " cmd={cmd}")?;
        }
        match self.exited {
            None => {}
            Some(Some(code)) => write!(f, " exited={code}")?,
            Some(None) => write!(f, " exited=?")?,
        }
        Ok(())
    }
}

impl PaneInfoRow {
    /// Parse one reply line; `None` when the pane is not `%N`, the window
    /// not `@W`, or the size not `COLSxROWS`. Unknown `key=value` tail
    /// tokens are ignored: the emitter promises older clients keep parsing
    /// the fixed prefix, so a new token must not break this parse.
    pub fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        let pane = fields.next()?;
        let window = fields.next()?;
        if !is_typed_id(pane, '%') || !is_typed_id(window, '@') {
            return None;
        }
        let (cols, rows) = fields.next()?.split_once('x')?;
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(cols) || !digits(rows) {
            return None;
        }
        let size = (cols.parse().ok()?, rows.parse().ok()?);
        let mut cmd = None;
        let mut exited = None;
        for token in fields {
            if let Some(value) = token.strip_prefix("cmd=") {
                cmd = Some(value.to_string());
            } else if let Some(code) = token.strip_prefix("exited=") {
                exited = Some(code.parse::<i32>().ok());
            }
        }
        Some(Self {
            pane: pane.to_string(),
            window: window.to_string(),
            size,
            cmd,
            exited,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `fuzz/corpus/ipc_rows` seed contents, as Rust literals (ENH-060).
    /// `include_str!` of the corpus would break `cargo package` (par-mux is
    /// published), so the seeds are mirrored here and kept in sync by the
    /// card's Verify-step diff check.
    const FUZZ_SEED_LINES: &[&str] = &[
        "%1 claude working hook",
        "%2 claude blocked hook reason=aGk=",
        "%3 pi idle scrape ws=+0",
        "%4 omp working hook reason=d2FpdA== telemetry=e30= host_telemetry=e30= ws=+12",
        "%5 pi idle hook future=1 ws=+1",
        "%6 pi idle hook ws=+0 ws=+1",
        "%7 pi idle hook == =x k=",
        "%8\tpi  working\thook ws=+2",
        "%10 pi idle hook\u{3000}ws=+0",
        "%9 pi idle hook stray ws=+0",
        "%x pi idle hook ws=+0",
        "@1 * two\n+0: main: $3: work\n+1: beta active\nagent %2 pi idle hook ws=+1",
    ];

    fn pane_info(cmd: Option<&str>, exited: Option<Option<i32>>) -> PaneInfoRow {
        PaneInfoRow {
            pane: "%3".to_string(),
            window: "@1".to_string(),
            size: (80, 24),
            cmd: cmd.map(str::to_string),
            exited,
        }
    }

    /// ARC-141: `PaneInfoRow` writes the emitter's exact bytes and reads
    /// them back.
    #[test]
    fn pane_info_row_formats_and_round_trips() {
        let cases = [
            (pane_info(None, None), "%3 @1 80x24"),
            (
                pane_info(Some("Y2xhdWRl"), None),
                "%3 @1 80x24 cmd=Y2xhdWRl",
            ),
            (pane_info(None, Some(Some(0))), "%3 @1 80x24 exited=0"),
            (pane_info(None, Some(None)), "%3 @1 80x24 exited=?"),
            (
                pane_info(Some("Y2xhdWRl"), Some(Some(2))),
                "%3 @1 80x24 cmd=Y2xhdWRl exited=2",
            ),
        ];
        for (row, golden) in cases {
            assert_eq!(row.to_string(), golden);
            assert_eq!(PaneInfoRow::parse(golden), Some(row));
        }
    }

    /// ARC-141: the fixed prefix is required; an unknown tail token is
    /// ignored so a newer daemon never breaks an older client.
    #[test]
    fn pane_info_row_rejects_bad_prefixes_and_ignores_unknown_tokens() {
        assert_eq!(PaneInfoRow::parse("@1 %3 80x24"), None, "swapped ids");
        assert_eq!(PaneInfoRow::parse("%3 @1"), None, "no size");
        assert_eq!(PaneInfoRow::parse("%3 @1 80by24"), None, "bad size");
        assert_eq!(PaneInfoRow::parse("%3 @1 x24"), None, "empty cols");
        assert_eq!(
            PaneInfoRow::parse("%3 @1 80x24 future=1"),
            Some(pane_info(None, None))
        );
    }

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

    /// `workspace()` reads the `ws=` extra; a row without it has none.
    #[test]
    fn agent_row_workspace_reads_the_ws_extra() {
        let row = AgentRow::parse("%1 pi working hook reason=x ws=+2").expect("parses");
        assert_eq!(row.workspace(), Some("+2"));
        let bare = AgentRow::parse("%1 pi working hook").expect("parses");
        assert_eq!(bare.workspace(), None);
    }

    /// ENH-060: the `fuzz/corpus/ipc_rows` seeds, replayed on stable.
    #[test]
    fn ipc_rows_fuzz_seeds_round_trip() {
        for seed in FUZZ_SEED_LINES {
            assert_round_trip(seed);
            for line in seed.split('\n') {
                assert_round_trip(line);
                if let Some((_, rest)) = line.split_once(' ') {
                    assert_round_trip(rest);
                }
            }
        }
    }

    /// The extras grammar behind the corpus (ENH-060): the `ws=` lookup,
    /// extras order, the empty-key/value shapes, and the two rejects.
    #[test]
    fn agent_row_extras_grammar() {
        let ws = AgentRow::parse("%3 pi idle scrape ws=+0").expect("parses");
        assert_eq!(ws.workspace(), Some("+0"));

        let all = AgentRow::parse(
            "%4 omp working hook reason=d2FpdA== telemetry=e30= host_telemetry=e30= ws=+12",
        )
        .expect("parses");
        assert_eq!(
            all.extras,
            vec![
                ("reason".to_string(), "d2FpdA==".to_string()),
                ("telemetry".to_string(), "e30=".to_string()),
                ("host_telemetry".to_string(), "e30=".to_string()),
                ("ws".to_string(), "+12".to_string()),
            ]
        );

        let empty = AgentRow::parse("%7 pi idle hook == =x k=").expect("parses");
        assert_eq!(
            empty.extras,
            vec![
                ("".to_string(), "=".to_string()),
                ("".to_string(), "x".to_string()),
                ("k".to_string(), "".to_string()),
            ]
        );

        assert_eq!(AgentRow::parse("%9 pi idle hook stray ws=+0"), None);
        assert_eq!(AgentRow::parse("%x pi idle hook ws=+0"), None);
    }
}
