//! Connect-time daemon version/feature exchange shared by every control
//! client (the attach client and the streamer's `MuxSessionFactory`).
//!
//! The control protocol has no hard version gate: a launchd-managed daemon
//! older than the client is a supported runtime state, so grammar skew is
//! normal and a client must degrade, never refuse. The exchange reuses the
//! existing verbs — `version` (the daemon's [`build_stamp`]) and
//! `list-commands` (capability tokens) — so an old daemon answers with its
//! ordinary unknown-command `%error`, which this module reads as "legacy
//! daemon, version unknown" rather than a failure.

use crate::mux::build_stamp;

/// How a daemon's stamp relates to the linked client's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skew {
    /// Same stamp, or an unprovable `+unknown` pair: nothing to report.
    InSync,
    /// The daemon runs a different build than this client.
    Differs {
        /// The daemon's stamp, as served.
        daemon: String,
    },
    /// The daemon refused `version` (or never answered one): legacy,
    /// version unknown. The client continues in compatibility mode.
    Legacy,
}

/// The version-stamp warn/quiet rule (docs/MUX.md `version` bullet): quiet
/// when the stamps are equal, and quiet when the version prefixes match and
/// either side is `+unknown` — a same-version mismatch is then unprovable,
/// so clients stay quiet rather than cry wolf. Any other difference warns.
pub fn stamp_mismatch_is_quiet(mine: &str, served: &str) -> bool {
    if mine == served {
        return true;
    }
    let my_version = mine.split('+').next().unwrap_or_default();
    let served_version = served.split('+').next().unwrap_or_default();
    my_version == served_version && (mine.ends_with("+unknown") || served.ends_with("+unknown"))
}

/// Classify the daemon's served stamp (`None` = the `version` verb was
/// refused) against `mine`, this client's own [`build_stamp`].
pub fn assess(mine: &str, served: Option<&str>) -> Skew {
    match served {
        None => Skew::Legacy,
        Some(served) if stamp_mismatch_is_quiet(mine, served) => Skew::InSync,
        Some(served) => Skew::Differs {
            daemon: served.to_owned(),
        },
    }
}

/// [`assess`] against this client's linked [`build_stamp`].
pub fn assess_linked(served: Option<&str>) -> Skew {
    assess(build_stamp(), served)
}

/// The one diagnostic line for a skewed daemon, or `None` when in sync.
/// Both clients print/log exactly this text so the skew reads the same on
/// either side.
pub fn skew_note(mine: &str, skew: &Skew) -> Option<String> {
    match skew {
        Skew::InSync => None,
        Skew::Differs { daemon } => Some(format!(
            "par-mux: daemon stamp {daemon} differs from client {mine}"
        )),
        Skew::Legacy => Some(format!(
            "par-mux: daemon did not answer `version` (legacy daemon, version unknown); \
             continuing in compatibility mode (client {mine})"
        )),
    }
}

/// The daemon stamp as a short log-line field: the stamp, or `unknown`
/// for a legacy daemon that never served one.
pub fn stamp_field(served: Option<&str>) -> &str {
    served.unwrap_or("unknown")
}

/// A `version` reply block -> the served stamp. `None` for an `%error`
/// reply (the legacy-daemon signal). An empty successful body is a daemon
/// that answered but served nothing, kept as an empty stamp: it differs
/// from any real one, which is the honest outcome.
pub fn served_stamp(ok: bool, body: &[String]) -> Option<String> {
    ok.then(|| body.first().cloned().unwrap_or_default())
}

/// `list-commands` reply -> the daemon-level feature tokens: each
/// `features <token> …` line's tokens. Unknown line shapes are ignored per
/// the contract ("a client must ignore unknown tokens and lines"). A
/// pre-feature daemon never reaches this — its reply is an `%error` — and
/// an empty feature list is a legitimate outcome.
pub fn parse_feature_tokens(body: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in body {
        if let Some(rest) = line.strip_prefix("features ") {
            out.extend(rest.split_whitespace().map(str::to_owned));
        }
    }
    out
}

/// `list-commands` reply -> the per-command feature tokens: each
/// `name [feature …]` row's `(name, feature)` pairs (the daemon-level
/// `features` line is [`parse_feature_tokens`]'s, not a command row).
pub fn parse_command_features(body: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body {
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else { continue };
        if name == "features" {
            continue;
        }
        out.extend(words.map(|token| (name.to_owned(), token.to_owned())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_daemon_is_in_sync_and_says_nothing() {
        let skew = assess("0.82.0+abc", Some("0.82.0+abc"));
        assert_eq!(skew, Skew::InSync);
        assert_eq!(skew_note("0.82.0+abc", &skew), None);
    }

    #[test]
    fn an_older_daemon_is_a_skew_with_one_note_naming_both_stamps() {
        let skew = assess("0.82.0+abc", Some("0.80.0+def"));
        assert_eq!(
            skew,
            Skew::Differs {
                daemon: "0.80.0+def".to_owned()
            }
        );
        let note = skew_note("0.82.0+abc", &skew).expect("a skew reports");
        assert!(note.contains("0.80.0+def") && note.contains("0.82.0+abc"));
    }

    /// The legacy fallback: a daemon that refuses `version` is reported as
    /// legacy/compatibility mode, never as a connect failure.
    #[test]
    fn a_daemon_that_refuses_version_is_legacy_not_an_error() {
        assert_eq!(
            served_stamp(false, &["unknown command: version".to_owned()]),
            None
        );
        let skew = assess("0.82.0+abc", None);
        assert_eq!(skew, Skew::Legacy);
        let note = skew_note("0.82.0+abc", &skew).expect("legacy reports");
        assert!(note.contains("compatibility mode"), "{note}");
        assert_eq!(stamp_field(None), "unknown");
    }

    #[test]
    fn a_served_version_reply_yields_its_first_line() {
        assert_eq!(
            served_stamp(true, &["0.80.0+abc".to_owned()]).as_deref(),
            Some("0.80.0+abc")
        );
        assert_eq!(stamp_field(Some("0.80.0+abc")), "0.80.0+abc");
    }
}
