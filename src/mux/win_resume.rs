//! Windows resume spawning: structured argv to a pane process without a
//! cmd.exe string re-parse (card 01a0d9b38c98: every Windows resume failed
//! because `render_argv` single-quotes for POSIX and cmd.exe treats `'`
//! literally, leaving `& | ^ %` live into the bargain).
//!
//! Why not one quoting rule: the spawn layer (portable-pty 0.9) quotes each
//! argument with CreateProcess/MSVCRT rules and offers no raw-argument
//! escape hatch. That transport is exact for a PE target but cannot carry a
//! pre-quoted line through `cmd.exe /C` — cmd re-parses the line with its
//! own (different) rules and mangles embedded quotes. So the transport is
//! chosen by what `argv[0]` resolves to:
//!
//! - a PE image (`.exe`/`.com`) → spawned directly; every argument arrives
//!   verbatim (spaces, `&`, `^`, quotes — all safe, no shell involved);
//! - anything else (`.cmd`/`.bat` shims — the shape npm-distributed agent
//!   CLIs take on Windows — and unresolved names) → spawned through a
//!   self-deleting bridge batch file we fully control, where double-quoted
//!   arguments carry spaces and `&` and cmd's own PATH+PATHEXT search
//!   resolves the program.
//!
//! Residuals, recorded rather than hidden: `%VAR%` expansion and embedded
//! double quotes cannot be represented on a cmd command line, so the bridge
//! passes them through live (bounded by the same-user socket trust, as
//! before); and a `%TEMP%` path containing `&` would defeat cmd's
//! two-quote rule for the bridge's own path.
//!
//! Everything here is production-reachable only under `cfg(windows)`; the
//! resolver and renderer stay compiled (and unit-tested) everywhere so the
//! logic cannot drift between platforms, which is why non-Windows builds
//! allow the dead-code lint for this module rather than gating it away.

#![cfg_attr(not(windows), allow(dead_code))]

use crate::pty_error::PtyError;
use crate::pty_session::PtySession;
use std::path::{Path, PathBuf};

/// How `argv[0]` resolves for the Windows resume spawn.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResumeTarget {
    /// A PE image CreateProcess can launch directly — the exact-args path.
    Executable(PathBuf),
    /// A cmd script (`.cmd`/`.bat`) or a name that only cmd can resolve —
    /// the bridge-batch path.
    ShellScript(PathBuf),
    /// Nothing on PATH matched; the bridge still lets cmd's own search try.
    Unresolved(String),
}

/// Resolve `argv[0]` against the daemon's `PATH`.
///
/// PE images win over scripts even when the script's directory comes first:
/// the direct transport is exact while any cmd re-parse is not, so a
/// `.cmd` shadowing an `.exe` earlier on PATH deliberately loses here (a
/// divergence from cmd's per-directory PATHEXT order, chosen for argument
/// fidelity). Extensionless files are skipped for the same reason — an
/// npm install puts a POSIX sh shim next to its `.cmd`/`.ps1` siblings, and
/// CreateProcess cannot execute the shim.
pub(crate) fn resolve_target(program: &str) -> ResumeTarget {
    resolve_target_in(program, std::env::var("PATH").ok().as_deref())
}

/// The resolver core, parameterized on the PATH value so tests can drive it
/// with a synthetic directory tree instead of the daemon's environment.
pub(crate) fn resolve_target_in(program: &str, path_var: Option<&str>) -> ResumeTarget {
    if program.contains(['/', '\\']) || Path::new(program).is_absolute() {
        // An explicitly located program: classify by what it names. A PE
        // extension is direct; anything else (including extensionless)
        // goes to the bridge, where cmd applies its own semantics — the
        // failure mode stays inside the pane, as before.
        return classify(Path::new(program));
    }
    let Some(path_var) = path_var else {
        return ResumeTarget::Unresolved(program.to_string());
    };
    // First script hit across all directories, kept as the fallback for
    // when no directory yields a PE image.
    let mut script: Option<PathBuf> = None;
    let extensionless = Path::new(program).extension().is_none();
    for dir in std::env::split_paths(path_var) {
        let direct = dir.join(program);
        if direct.is_file() {
            if is_pe_path(&direct) {
                return ResumeTarget::Executable(direct);
            }
            if script.is_none() && is_script_path(&direct) {
                script = Some(direct);
            }
            // Anything else here (the extensionless sh shim) is skipped.
        }
        if extensionless {
            for pe in PE_EXTENSIONS {
                let candidate = dir.join(format!("{program}.{pe}"));
                if candidate.is_file() {
                    return ResumeTarget::Executable(candidate);
                }
            }
            if script.is_none() {
                for script_ext in SCRIPT_EXTENSIONS {
                    let candidate = dir.join(format!("{program}.{script_ext}"));
                    if candidate.is_file() {
                        script = Some(candidate);
                        break;
                    }
                }
            }
        }
    }
    match script {
        Some(path) => ResumeTarget::ShellScript(path),
        None => ResumeTarget::Unresolved(program.to_string()),
    }
}

/// File extensions CreateProcess can execute directly.
const PE_EXTENSIONS: [&str; 2] = ["exe", "com"];
/// File extensions only cmd.exe can run — the npm shim shape.
const SCRIPT_EXTENSIONS: [&str; 2] = ["cmd", "bat"];

fn is_pe_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| PE_EXTENSIONS.iter().any(|pe| ext.eq_ignore_ascii_case(pe)))
}

fn is_script_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        SCRIPT_EXTENSIONS
            .iter()
            .any(|se| ext.eq_ignore_ascii_case(se))
    })
}

/// Classify an explicitly located program by its extension.
fn classify(path: &Path) -> ResumeTarget {
    if is_pe_path(path) {
        ResumeTarget::Executable(path.to_path_buf())
    } else {
        ResumeTarget::ShellScript(path.to_path_buf())
    }
}

/// The bridge batch: the invocation with every argument double-quoted
/// (spaces and `&` are literal inside cmd quotes), followed by a
/// self-delete so a restore leaves no file behind once the agent exits.
pub(crate) fn render_bridge_batch(argv: &[String], program: &str) -> String {
    let mut line = String::new();
    line.push('"');
    line.push_str(program);
    line.push('"');
    for arg in &argv[1..] {
        line.push_str(" \"");
        line.push_str(arg);
        line.push('"');
    }
    // The `(goto) 2>nul & del` idiom: a plain trailing `del` leaves cmd
    // reading a batch file that no longer exists, which prints "The batch
    // file cannot be found." into the pane; the no-label goto closes the
    // batch context first (observed on the Windows 11 VM, 2026-09-25).
    format!("@echo off\r\n{line}\r\n(goto) 2>nul & del \"%~f0\"\r\n")
}

/// Write the bridge batch to the temp directory under a unique name.
fn write_bridge_batch(argv: &[String], program: &str) -> Result<PathBuf, PtyError> {
    let unique = format!(
        "par-mux-resume-{}-{}.cmd",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::write(&path, render_bridge_batch(argv, program)).map_err(|err| {
        PtyError::ProcessSpawnError(format!(
            "cannot write resume bridge {}: {err}",
            path.display()
        ))
    })?;
    Ok(path)
}

/// Spawn a resume argv into a configured [`PtySession`], choosing the
/// transport by resolution. Env and cwd were already applied to the
/// session by the factory and carry through either arm.
pub(crate) fn spawn_resume_argv(session: &mut PtySession, argv: &[String]) -> Result<(), PtyError> {
    match resolve_target(&argv[0]) {
        ResumeTarget::Executable(path) => {
            let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
            session.spawn(&path.to_string_lossy(), &args)
        }
        ResumeTarget::ShellScript(path) => spawn_via_bridge(session, argv, &path.to_string_lossy()),
        ResumeTarget::Unresolved(program) => spawn_via_bridge(session, argv, &program),
    }
}

/// Run the invocation through a bridge batch under `%COMSPEC% /d /c`.
///
/// `/d` skips AutoRun registry hooks; the batch path is a single quoted
/// token, the one shape cmd's `/C` two-quote rule passes through the
/// MSVCRT-quoted spawn layer intact.
fn spawn_via_bridge(
    session: &mut PtySession,
    argv: &[String],
    program: &str,
) -> Result<(), PtyError> {
    let batch = write_bridge_batch(argv, program)?;
    let batch_str = batch.to_string_lossy().into_owned();
    let shell = PtySession::get_default_shell();
    session.spawn(&shell, &["/d", "/c", &batch_str])
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- resolution: PE wins, scripts fall back, shims are skipped ---

    #[test]
    fn a_pe_in_a_later_path_dir_wins_over_a_script_in_an_earlier_one() {
        let exe_dir = tempfile::tempdir().unwrap();
        let script_dir = tempfile::tempdir().unwrap();
        std::fs::write(script_dir.path().join("agent"), b"#!/bin/sh\n").unwrap();
        std::fs::write(script_dir.path().join("agent.cmd"), b"@echo off\r\n").unwrap();
        std::fs::write(exe_dir.path().join("agent.exe"), b"MZ").unwrap();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let joined = format!(
            "{}{sep}{}",
            script_dir.path().display(),
            exe_dir.path().display()
        );
        assert_eq!(
            resolve_target_in("agent", Some(&joined)),
            ResumeTarget::Executable(exe_dir.path().join("agent.exe")),
            "the exact-args transport outranks directory order"
        );
    }

    #[test]
    fn a_cmd_shim_without_a_pe_resolves_to_the_bridge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent"), b"#!/bin/sh\n").unwrap();
        std::fs::write(dir.path().join("agent.cmd"), b"@echo off\r\n").unwrap();
        let joined = dir.path().display().to_string();
        assert_eq!(
            resolve_target_in("agent", Some(&joined)),
            ResumeTarget::ShellScript(dir.path().join("agent.cmd")),
            "the extensionless sh shim is skipped; the .cmd is the target"
        );
    }

    #[test]
    fn an_explicit_exe_path_is_direct_and_an_explicit_cmd_path_is_the_bridge() {
        assert_eq!(
            resolve_target_in("C:\\tools\\agent.exe", None),
            ResumeTarget::Executable(PathBuf::from("C:\\tools\\agent.exe"))
        );
        assert_eq!(
            resolve_target_in("C:\\tools\\agent.cmd", None),
            ResumeTarget::ShellScript(PathBuf::from("C:\\tools\\agent.cmd"))
        );
        // Extensionless explicit paths (the npm sh shim) stay on the cmd
        // path — the failure lands in the pane, not the restore chain.
        assert_eq!(
            resolve_target_in("C:\\tools\\agent", None),
            ResumeTarget::ShellScript(PathBuf::from("C:\\tools\\agent"))
        );
    }

    #[test]
    fn a_name_with_an_extension_is_not_re_suffixed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool.cmd"), b"@echo off\r\n").unwrap();
        let joined = dir.path().display().to_string();
        assert_eq!(
            resolve_target_in("tool.cmd", Some(&joined)),
            ResumeTarget::ShellScript(dir.path().join("tool.cmd")),
            "agent.cmd must not also probe agent.cmd.exe"
        );
    }

    #[test]
    fn nothing_on_path_is_unresolved_not_an_error() {
        assert_eq!(
            resolve_target_in("par-mux-test-no-such-binary", Some("")),
            ResumeTarget::Unresolved("par-mux-test-no-such-binary".to_string())
        );
        assert_eq!(
            resolve_target_in("agent", None),
            ResumeTarget::Unresolved("agent".to_string())
        );
    }

    // --- the bridge render: quoting and self-cleanup ---

    #[test]
    fn the_bridge_quotes_every_argument_and_deletes_itself() {
        let argv: Vec<String> = ["agent.cmd", "--resume", "C:\\my dir\\s 1 & x"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        assert_eq!(
            render_bridge_batch(&argv, "agent.cmd"),
            "@echo off\r\n\"agent.cmd\" \"--resume\" \"C:\\my dir\\s 1 & x\"\r\n(goto) 2>nul & del \"%~f0\"\r\n"
        );
    }

    #[test]
    fn the_bridge_carries_the_program_but_not_as_an_argument() {
        let argv: Vec<String> = ["C:\\tools\\agent.cmd", "--flag"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        assert_eq!(
            render_bridge_batch(&argv, "C:\\tools\\agent.cmd"),
            "@echo off\r\n\"C:\\tools\\agent.cmd\" \"--flag\"\r\n(goto) 2>nul & del \"%~f0\"\r\n"
        );
    }
}
