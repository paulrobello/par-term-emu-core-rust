//! Per-agent resume invocations (Phase 6, task 6.2 — `par-mux.md` D6.2).
//!
//! The resume invocation is NOT persisted: a stored argv would freeze a
//! vendor's CLI shape into a state file that newer par-mux code later
//! restores. The facts (agent + session ref) persist; the invocation is
//! rebuilt at restore time from this table, which ships with the binary and
//! is therefore always as current as the running code.
//!
//! Two sources, in order (design criterion 6 — hook-first from the start):
//! the agent's own reported invocation
//! ([`PersistAgentSession::resume_argv`], stored verbatim as a JSON argv
//! string by the hook endpoint, contract settled by card
//! 01a0c766bc567c30bc429c4e380554d3) wins; the table is the fallback for
//! agents that cannot report one. Only pi/omp report today, so the table
//! covers claude/codex/grok in practice — but the order is encoded now so a
//! newly-reporting agent drops out of the table path with no restructuring.
//!
//! The five entries are cross-checked against herdr's `plan()`
//! (`~/Repos/herdr/src/agent_resume.rs:136`), a reference implementation to
//! check against rather than derive from:
//!
//! | agent | invocation | ref kinds |
//! |---|---|---|
//! | claude | `claude --resume <id>` | id |
//! | codex | `codex resume <id>` — a subcommand, not a flag | id |
//! | grok | `grok --resume <id>` | id |
//! | pi | `pi --session <value>` | id or path |
//! | omp | `omp --resume=<value>` — `=`-joined; omp has no `--session` | id or path |
//!
//! Deliberate divergences from herdr, recorded per the design's cross-check
//! criterion:
//!
//! - herdr's table carries 15+ agents; par-mux carries the owner's supported
//!   five (decided 2026-09-21) and returns `None` — a fresh spawn, not an
//!   error — for everything else.
//! - herdr keys each entry on `(source, agent)` through
//!   `is_official_agent_source`; par-mux keys on the agent label alone, with
//!   `source` informational.
//! - herdr's `plan()` returns an `AgentResumePlan` carrying a dedupe key (a
//!   roster concern); par-mux returns bare argv.
//! - When a pi/omp session holds BOTH an id and a path, the table prefers the
//!   path — matching what our own extensions report ("transcript path else
//!   session id"); herdr takes whichever single kind its ref carries.

use crate::mux::persist::PersistAgentSession;

/// The resume invocation for a persisted agent session: hook-reported first,
/// table second (design criterion 6).
///
/// `None` means "no invocation derivable" — task 6.3's restore chain falls
/// back to the pane's original `spawn_command`, never a dead pane.
pub fn resume_invocation(session: &PersistAgentSession) -> Option<Vec<String>> {
    // Hook-reported argv is used verbatim. The endpoint validates shape at
    // report time, so a value failing the same check here is a hand-edited
    // state file — degrade to the table rather than spawn a broken command.
    if let Some(argv) = session
        .resume_argv
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .filter(|argv| !argv.is_empty() && argv.iter().all(|part| !part.trim().is_empty()))
    {
        return Some(argv);
    }
    resume_argv(&session.agent, session)
}

/// The per-agent table alone, ignoring any hook-reported invocation.
///
/// `None` means "no entry for this agent", which is a fresh spawn, NOT an
/// error. The three argv shapes are irreconcilable on purpose — subcommand
/// (codex), separate argument (claude/grok/pi), `=`-joined (omp) — which is
/// the concrete reason this is a table and not a format string.
pub fn resume_argv(agent: &str, session: &PersistAgentSession) -> Option<Vec<String>> {
    match agent {
        // Id-only: a transcript path is not a usable ref for these three.
        "claude" => Some(vec!["claude".into(), "--resume".into(), id_ref(session)?]),
        "codex" => Some(vec!["codex".into(), "resume".into(), id_ref(session)?]),
        "grok" => Some(vec!["grok".into(), "--resume".into(), id_ref(session)?]),
        "pi" => Some(vec![
            "pi".into(),
            "--session".into(),
            id_or_path_ref(session)?,
        ]),
        "omp" => Some(vec![
            "omp".into(),
            format!("--resume={}", id_or_path_ref(session)?),
        ]),
        // No entry: a fresh spawn, not an error.
        _ => None,
    }
}

/// The session ref for the id-only agents; `None` when no id was persisted —
/// a transcript path is not a usable ref for claude/codex/grok.
fn id_ref(session: &PersistAgentSession) -> Option<String> {
    non_empty(session.session_id.as_deref())
}

/// Render an argv as a POSIX shell command line — the shape the factory's
/// `sh -c` spawn consumes (task 6.3). Every argument is single-quoted
/// unconditionally: the argv crosses from a validated structure into a
/// string a shell re-parses, and picking an "unquotable" alphabet is a
/// guess about vendor CLIs the table exists not to make.
pub fn render_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|part| format!("'{}'", part.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The spawn-command tail that keeps a failed resume from costing the pane
/// (unix only): when the invocation exits non-zero — an uninstalled binary,
/// a session id the CLI rejects — the pane falls through to a live shell
/// instead of exiting into the reaper, so the restored screen, scrollback,
/// and agent identity survive until an explicit `kill-pane`.
pub(crate) const SURVIVING_TAIL: &str = " || { printf 'par-mux: agent resume failed; pane kept on a shell\\n' >&2; exec \"${SHELL:-sh}\"; }";

/// Render a resume invocation for the pane factory's `sh -c` spawn, shaped
/// so a FAILED resume cannot delete the pane (D6.3's "never a dead or
/// half-initialized pane"): the invocation runs; only on a non-zero exit
/// does the pane drop into a live shell carrying the restored screen and
/// scrollback. On Windows the tail's POSIX syntax would reach cmd.exe
/// verbatim, so the invocation stays bare there — cmd.exe resume quoting
/// is tracked as its own gap.
pub fn render_surviving(argv: &[String]) -> String {
    let rendered = render_argv(argv);
    if cfg!(windows) {
        return rendered;
    }
    format!("{rendered}{SURVIVING_TAIL}")
}

/// The session ref for pi/omp, which accept a transcript path as well as an
/// id. Path-first when both are present, matching what our own extensions
/// report.
fn id_or_path_ref(session: &PersistAgentSession) -> Option<String> {
    non_empty(session.session_path.as_deref()).or_else(|| non_empty(session.session_id.as_deref()))
}

/// Empty strings read as absent — a hand-edited state file must not produce
/// an argv with an empty ref argument.
fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|value| !value.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(agent: &str, id: Option<&str>, path: Option<&str>) -> PersistAgentSession {
        PersistAgentSession {
            agent: agent.to_string(),
            session_id: id.map(str::to_string),
            session_path: path.map(str::to_string),
            source: Some("par-mux:test".to_string()),
            resume_argv: None,
        }
    }

    fn reported(session: PersistAgentSession, argv: &str) -> PersistAgentSession {
        PersistAgentSession {
            resume_argv: Some(argv.to_string()),
            ..session
        }
    }

    fn argv(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|part| part.to_string()).collect())
    }

    // --- the table: exact argv per agent, all three shapes (criteria 1-2) ---

    #[test]
    fn claude_resumes_with_a_separate_resume_argument() {
        assert_eq!(
            resume_argv("claude", &session("claude", Some("abc-123"), None)),
            argv(&["claude", "--resume", "abc-123"])
        );
    }

    #[test]
    fn codex_resumes_through_a_subcommand_not_a_flag() {
        assert_eq!(
            resume_argv("codex", &session("codex", Some("abc-123"), None)),
            argv(&["codex", "resume", "abc-123"])
        );
    }

    #[test]
    fn grok_resumes_with_a_separate_resume_argument() {
        assert_eq!(
            resume_argv("grok", &session("grok", Some("abc-123"), None)),
            argv(&["grok", "--resume", "abc-123"])
        );
    }

    #[test]
    fn pi_takes_a_separate_session_argument() {
        assert_eq!(
            resume_argv("pi", &session("pi", Some("s-1"), None)),
            argv(&["pi", "--session", "s-1"])
        );
    }

    #[test]
    fn omp_joins_the_resume_flag_with_an_equals_sign() {
        assert_eq!(
            resume_argv("omp", &session("omp", Some("s-1"), None)),
            argv(&["omp", "--resume=s-1"])
        );
    }

    #[test]
    fn unknown_agent_has_no_entry_rather_than_an_error() {
        assert_eq!(
            resume_argv("cursor", &session("cursor", Some("s-1"), None)),
            None
        );
        assert_eq!(resume_argv("", &session("", Some("s-1"), None)), None);
    }

    // --- ref kinds: which agents resolve from a path (criterion 4) ---

    #[test]
    fn pi_and_omp_resolve_from_a_path_as_well_as_an_id() {
        for agent in ["pi", "omp"] {
            assert!(
                resume_argv(agent, &session(agent, None, Some("/tmp/s.jsonl"))).is_some(),
                "{agent} resolves from a path alone"
            );
        }
    }

    #[test]
    fn pi_and_omp_prefer_the_path_when_both_refs_are_present() {
        assert_eq!(
            resume_argv("pi", &session("pi", Some("s-1"), Some("/tmp/s.jsonl"))),
            argv(&["pi", "--session", "/tmp/s.jsonl"])
        );
        assert_eq!(
            resume_argv("omp", &session("omp", Some("s-1"), Some("/tmp/s.jsonl"))),
            argv(&["omp", "--resume=/tmp/s.jsonl"])
        );
    }

    #[test]
    fn the_other_three_use_the_id_only() {
        for agent in ["claude", "codex", "grok"] {
            assert_eq!(
                resume_argv(agent, &session(agent, None, Some("/tmp/s.jsonl"))),
                None,
                "{agent} must not resolve from a path alone"
            );
        }
        assert_eq!(resume_argv("claude", &session("claude", None, None)), None);
    }

    #[test]
    fn an_empty_ref_string_reads_as_absent() {
        assert_eq!(
            resume_argv("claude", &session("claude", Some(""), None)),
            None
        );
        assert_eq!(resume_argv("pi", &session("pi", None, Some(""))), None);
    }

    // --- hook-first precedence (criterion 5) ---

    #[test]
    fn a_reported_invocation_wins_over_the_table_entry() {
        let session = reported(
            session("pi", Some("s-1"), Some("/tmp/table.jsonl")),
            r#"["pi","--session","/tmp/reported.jsonl"]"#,
        );
        assert_eq!(
            resume_invocation(&session),
            argv(&["pi", "--session", "/tmp/reported.jsonl"])
        );
    }

    #[test]
    fn a_reported_invocation_wins_even_with_no_table_entry() {
        let session = reported(session("kimi", None, None), r#"["kimi","--resume","k-9"]"#);
        assert_eq!(
            resume_invocation(&session),
            argv(&["kimi", "--resume", "k-9"])
        );
    }

    #[test]
    fn a_malformed_reported_invocation_falls_through_to_the_table() {
        let session = reported(session("claude", Some("abc-123"), None), "not json");
        assert_eq!(
            resume_invocation(&session),
            argv(&["claude", "--resume", "abc-123"])
        );
    }

    #[test]
    fn a_structurally_empty_reported_invocation_falls_through_to_the_table() {
        // The endpoint never stores these shapes; a state file that carries
        // one anyway must degrade to the table, not spawn a broken command.
        for bad in ["[]", r#"[""]"#] {
            let session = reported(session("claude", Some("abc-123"), None), bad);
            assert_eq!(
                resume_invocation(&session),
                argv(&["claude", "--resume", "abc-123"]),
                "reported argv {bad} must not be used verbatim"
            );
        }
    }

    #[test]
    fn no_report_and_no_entry_is_none() {
        assert_eq!(
            resume_invocation(&session("cursor", Some("s-1"), None)),
            None
        );
    }

    // --- rendering for the factory's `sh -c` spawn (task 6.3) ---

    #[test]
    fn render_argv_single_quotes_every_argument() {
        let argv: Vec<String> = ["omp", "--resume=/tmp/s.jsonl", ""]
            .iter()
            .map(|part| part.to_string())
            .collect();
        assert_eq!(render_argv(&argv), "'omp' '--resume=/tmp/s.jsonl' ''");
    }

    #[test]
    fn render_argv_quotes_spaces_and_embedded_quotes_safely() {
        let argv: Vec<String> = ["pi", "--session", "/tmp/my session's file.jsonl"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        assert_eq!(
            render_argv(&argv),
            "'pi' '--session' '/tmp/my session'\\''s file.jsonl'"
        );
    }

    // --- the surviving render: a failed resume must not cost the pane ---

    #[test]
    fn render_surviving_appends_a_shell_fallback_tail_on_unix() {
        let argv: Vec<String> = ["pi", "--session", "s-1"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        if cfg!(windows) {
            // cmd.exe cannot parse the POSIX tail; the invocation stays
            // bare there (its cmd.exe quoting is a separate open gap).
            assert_eq!(render_surviving(&argv), render_argv(&argv));
        } else {
            assert_eq!(
                render_surviving(&argv),
                format!("'pi' '--session' 's-1'{SURVIVING_TAIL}")
            );
        }
    }
}
