//! macOS responsible-process attribution check for the running daemon
//! (the `doctor` control verb).
//!
//! macOS Local Network Privacy and TCC consent bind to a process's
//! *responsible app*: the signed bundle at the root of its ancestry. Pane
//! children inherit the daemon's attribution, so a daemon that is its own
//! responsible process — every shape that starts outside Par-mux.app or
//! the launchd job — silently loses LAN egress (non-Apple pane binaries
//! fail with `EHOSTUNREACH` while Apple platform binaries keep working)
//! and TCC consent like 1Password's "Always Allow" cannot persist (herdr
//! shipped this exact bug class as herdr#808). The probe is
//! `responsibility_get_pid_responsible_for_pid` from libSystem: no root,
//! no private framework, one call.

/// The responsible pid for `pid`, when the platform exposes the probe.
#[cfg(target_os = "macos")]
pub fn probe_responsible_pid(pid: i32) -> Option<i32> {
    extern "C" {
        fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    }
    // Non-positive results mean the kernel could not resolve a
    // responsible process; report that as "unknown", not as a pid.
    let responsible = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
    (responsible > 0).then_some(responsible)
}

/// Non-macOS builds carry no probe: the reply says so instead of guessing.
#[cfg(not(target_os = "macos"))]
pub fn probe_responsible_pid(_pid: i32) -> Option<i32> {
    None
}

/// The code-signing team identifier of `exe`, when the binary carries a
/// Developer-ID signature: a *stable identity* that TCC and Local Network
/// grants can bind to across restarts and reinstalls (the 0.85.0 signing
/// model). `codesign -dv` prints its display block on stderr.
#[cfg(target_os = "macos")]
pub fn probe_team_identifier(exe: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/codesign")
        .arg("-dv")
        .arg(exe)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    for line in text.lines() {
        if let Some(team) = line.strip_prefix("TeamIdentifier=") {
            let team = team.trim();
            return (!team.is_empty() && team != "notatall" && team != "unknown")
                .then(|| team.to_string());
        }
    }
    None
}

/// Non-macOS builds carry no signature probe.
#[cfg(not(target_os = "macos"))]
pub fn probe_team_identifier(_exe: &std::path::Path) -> Option<String> {
    None
}

/// The `doctor` reply lines for a daemon pid, its probe result, and the
/// daemon binary's code-signing team.
///
/// `responsible == daemon pid` is the self-responsible shape. With an
/// ad-hoc or unsigned binary it is `tainted`: nothing above the daemon
/// holds a grantable app identity, and the taint is silent — Apple
/// platform binaries keep reaching the LAN, which reads as a network
/// problem rather than an attribution one. A Developer-ID-signed binary
/// carries a *stable identity* the grants bind to, so the same pid shape
/// is `stable-identity` (LAN verified working in this shape, minimac
/// 2026-11-22): still not the app-bundle shape, so the reply names the
/// preference without calling it broken. The remediation names the shapes
/// that produce the taint and the way out.
pub fn reply_lines(
    macos: bool,
    daemon_pid: i32,
    responsible: Option<i32>,
    team: Option<&str>,
) -> Vec<String> {
    let mut lines = vec![format!("daemon-pid: {daemon_pid}")];
    if !macos {
        lines.push("verdict: not-applicable".to_string());
        lines.push(
            "attribution probe is macOS-only; Linux and Windows panes carry no responsible-app model"
                .to_string(),
        );
        return lines;
    }
    match responsible {
        None => {
            lines.push("verdict: unknown".to_string());
            lines.push(
                "responsibility probe returned nothing; treat LAN loss as suspected".to_string(),
            );
        }
        Some(p) if p == daemon_pid => match team {
            Some(team) => {
                lines.push(format!("responsible-pid: {p}"));
                lines.push("verdict: stable-identity".to_string());
                lines.push(format!(
                    "the daemon is its own responsible process, but its binary is Developer-ID-signed (team {team}): Local Network and TCC grants bind to this stable identifier and survive restarts and reinstalls"
                ));
                lines.push(
                    "preference: the app bundle or launchd job still widens TCC scope; an unsigned or ad-hoc binary in this pid shape would be tainted".to_string(),
                );
            }
            None => {
                lines.push(format!("responsible-pid: {p}"));
                lines.push("verdict: tainted".to_string());
                lines.push(
                    "the daemon is its own responsible process: pane children lose LAN egress and TCC consent (herdr#808 class)".to_string(),
                );
                lines.push(
                    "fix: start through /Applications/Par-mux.app or the com.paulrobello.par-mux launchd job; unset PAR_MUX_NO_APP_WRAPPER; restart with `par-mux --restart`".to_string(),
                );
            }
        },
        Some(p) => {
            lines.push(format!("responsible-pid: {p}"));
            lines.push("verdict: healthy".to_string());
            lines.push(
                "pane children inherit the responsible app's Local Network and TCC grants"
                    .to_string(),
            );
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::reply_lines;

    /// Positive control for the tainted classification: the probe value
    /// equal to the daemon pid is the shape every remediation line exists
    /// for — flipping the equality arm must fail this test.
    #[test]
    fn tainted_when_self_responsible() {
        let lines = reply_lines(true, 40082, Some(40082), None);
        assert!(lines.iter().any(|l| l.contains("verdict: tainted")));
        assert!(lines.iter().any(|l| l.contains("responsible-pid: 40082")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("PAR_MUX_NO_APP_WRAPPER") && l.contains("--restart")),
            "the tainted verdict names the taint shapes and the fix"
        );
    }

    /// The false positive this card exists for (minimac, 2026-11-22): a
    /// self-responsible daemon whose binary is Developer-ID-signed is NOT
    /// tainted — the grants bind to the stable team identity, so pane
    /// children keep the LAN (verified in-pane, gateway curl 302). The
    /// verdict must name the team and keep the bundle preference, without
    /// the tainted arm's fix line.
    #[test]
    fn stable_identity_when_self_responsible_and_team_signed() {
        let lines = reply_lines(true, 40082, Some(40082), Some("QMLVG482FY"));
        assert!(lines.iter().any(|l| l.contains("verdict: stable-identity")));
        assert!(!lines.iter().any(|l| l.contains("verdict: tainted")));
        assert!(lines.iter().any(|l| l.contains("team QMLVG482FY")));
        assert!(lines.iter().any(|l| l.contains("stable identifier")));
        assert!(
            lines.iter().any(|l| l.contains("preference:")),
            "the reply still names the bundle preference"
        );
        assert!(
            !lines.iter().any(|l| l.contains("fix:")),
            "no remediation is required in this shape"
        );
    }

    #[test]
    fn healthy_when_a_bundle_ancestor_is_responsible() {
        let lines = reply_lines(true, 40082, Some(44120), None);
        assert!(lines.iter().any(|l| l.contains("verdict: healthy")));
        assert!(lines.iter().any(|l| l.contains("responsible-pid: 44120")));
        assert!(!lines.iter().any(|l| l.contains("fix:")));
    }

    #[test]
    fn unknown_when_the_probe_returns_nothing() {
        let lines = reply_lines(true, 40082, None, None);
        assert!(lines.iter().any(|l| l.contains("verdict: unknown")));
        assert!(!lines.iter().any(|l| l.contains("responsible-pid")));
    }

    #[test]
    fn not_applicable_off_macos() {
        let lines = reply_lines(false, 40082, None, None);
        assert!(lines.iter().any(|l| l.contains("verdict: not-applicable")));
        assert!(
            lines.iter().any(|l| l.contains("macOS-only")),
            "the platform line says the probe is macOS-only"
        );
    }

    /// Every report leads with the pid the probe was about, so a
    /// multi-daemon host can tell which daemon answered.
    #[test]
    fn every_report_names_the_daemon_pid() {
        for (macos, responsible) in [(true, Some(1)), (true, None), (false, None)] {
            let lines = reply_lines(macos, 1234, responsible, None);
            assert!(
                lines[0].contains("daemon-pid: 1234"),
                "first line names the daemon: {lines:?}"
            );
        }
    }

    /// Live probe smoke test (macOS CI runners): the call succeeds and
    /// returns a positive pid. The verdict is deliberately not asserted —
    /// a cargo-test process is self-responsible or not depending on how
    /// the runner launched it.
    #[cfg(target_os = "macos")]
    #[test]
    fn probe_returns_a_positive_pid_on_macos() {
        assert!(super::probe_responsible_pid(std::process::id() as i32).is_some());
    }
}
