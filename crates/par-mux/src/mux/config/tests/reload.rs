//! Daemon-side reload_report tests: the live-applied settings vs the
//! restart-required ones.

use crate::mux::config::*;

/// remain-on-exit and exit-empty are the LIVE daemon settings: a stated
/// difference is applied to the applied copy and reported `applied:`;
/// the same value or an absent key stays `unchanged:`.
#[test]
fn reload_applies_remain_on_exit_live() {
    let mut applied = EffectiveConfig::default();
    let file: ConfigFile = toml::from_str("[daemon]\nremain-on-exit = true").unwrap();
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("applied: daemon.remain-on-exit"),
        "a stated difference applies live: {report}"
    );
    assert!(applied.remain_on_exit, "the applied copy moved");
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("unchanged: daemon.remain-on-exit"),
        "the same value is unchanged: {report}"
    );
    let file: ConfigFile = toml::from_str("[daemon]\nremain-on-exit = false").unwrap();
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("applied: daemon.remain-on-exit"),
        "flipping back applies too: {report}"
    );
    assert!(!applied.remain_on_exit, "the applied copy moved back");
}

/// exit-empty behaves identically: a stated difference applies live
/// (`applied:`), the same value stays `unchanged:`.
#[test]
fn reload_applies_exit_empty_live() {
    let mut applied = EffectiveConfig::default();
    let file: ConfigFile = toml::from_str("[daemon]\nexit-empty = false").unwrap();
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("applied: daemon.exit-empty"),
        "a stated difference applies live: {report}"
    );
    assert!(!applied.exit_empty, "the applied copy moved");
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("unchanged: daemon.exit-empty"),
        "the same value is unchanged: {report}"
    );
}

/// The agent-append keys are LIVE like the other boot-surviving
/// settings: a stated difference applies to the applied copy and
/// reports `applied:`; the same value, an omitted key, and the
/// absent-file report all stay `unchanged:`.
#[test]
fn reload_applies_agent_append_live() {
    let mut applied = EffectiveConfig::default();
    let file: ConfigFile =
        toml::from_str("[daemon.agent-append]\nclaude = \"--continue\"\n").unwrap();
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("applied: daemon.agent-append.claude"),
        "a stated difference applies live: {report}"
    );
    assert_eq!(applied.agent_append.claude, "--continue");
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("unchanged: daemon.agent-append.claude"),
        "the same value is unchanged: {report}"
    );
    let absent = reload_report(&mut applied, None);
    for kind in AGENT_APPEND_KINDS {
        assert!(
            absent.contains(&format!("unchanged: daemon.agent-append.{kind}")),
            "the absent file keeps every kind unchanged: {report}"
        );
    }
}

/// exit-action is the third LIVE daemon setting: a stated difference
/// applies to the applied copy (`applied:`), the same value or an
/// absent key stays `unchanged:`.
#[test]
fn reload_applies_exit_action_live() {
    let mut applied = EffectiveConfig::default();
    let file: ConfigFile = toml::from_str("[daemon]\nexit-action = \"shell\"").unwrap();
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("applied: daemon.exit-action"),
        "a stated difference applies live: {report}"
    );
    assert_eq!(applied.exit_action, ExitAction::Shell);
    let report = reload_report(&mut applied, Some(&file));
    assert!(
        report.contains("unchanged: daemon.exit-action"),
        "the same value is unchanged: {report}"
    );
}
