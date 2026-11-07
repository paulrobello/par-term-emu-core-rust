//! Tier-resolution tests: flags > env > file > built-in defaults, the
//! exit-action fold, and the agent-append table.

use crate::mux::config::*;

#[test]
fn automatic_updates_are_opt_in_with_daily_default() {
    let defaults: ConfigFile = toml::from_str("").unwrap();
    let effective = resolve(&defaults, &Overrides::default());
    assert!(!effective.auto_update);
    assert_eq!(effective.update_interval, "daily");
    for interval in ["hourly", "daily", "weekly"] {
        let config: ConfigFile = toml::from_str(&format!(
            "[daemon]\nauto-update = true\nupdate-interval = '{interval}'"
        ))
        .unwrap();
        let effective = resolve(&config, &Overrides::default());
        assert!(effective.auto_update);
        assert_eq!(effective.update_interval, interval);
        let rendered = render(&effective);
        let roundtrip: ConfigFile = toml::from_str(&rendered).unwrap();
        assert_eq!(
            resolve(&roundtrip, &Overrides::default()).update_interval,
            interval
        );
    }
    assert!(toml::from_str::<ConfigFile>("[daemon]\nupdate-interval = 'bad'").is_err());
}

/// The precedence chain: file over built-in defaults, env over file,
/// flags over env — per setting, with the other tiers silent.
#[test]
fn resolution_is_flags_over_env_over_file_over_defaults() {
    // Defaults only.
    assert_eq!(
        resolve(&ConfigFile::default(), &Overrides::default()),
        EffectiveConfig::default()
    );

    // File speaks.
    let file = ConfigFile {
        client: ClientSection {
            prefix: Some("C-a".into()),
            ..Default::default()
        },
        daemon: DaemonSection {
            pane_endpoints: Some(true),
            ..Default::default()
        },
        machines: Default::default(),
    };
    let eff = resolve(&file, &Overrides::default());
    assert_eq!(eff.prefix, "C-a");
    assert!(eff.pane_endpoints);
    assert_eq!(eff.mode, "render", "unset settings keep the default");

    // Env beats file (socket only — the one setting with an env tier).
    let eff = resolve(
        &file,
        &Overrides {
            env_socket: Some("/tmp/from-env.sock".into()),
            ..Default::default()
        },
    );
    assert_eq!(eff.socket, "/tmp/from-env.sock");
    assert_eq!(eff.prefix, "C-a", "env does not speak for other settings");

    // Flags beat env and file.
    let eff = resolve(
        &file,
        &Overrides {
            prefix: Some("C-s".into()),
            socket: Some("/tmp/from-flag.sock".into()),
            pane_endpoints: Some(false),
            env_socket: Some("/tmp/from-env.sock".into()),
            ..Default::default()
        },
    );
    assert_eq!(eff.prefix, "C-s");
    assert_eq!(eff.socket, "/tmp/from-flag.sock");
    assert!(!eff.pane_endpoints, "the flag tier wins over the file");
}

/// The attach mode tiers: unspecified = render; a file `passthrough`
/// beats that default; the `--mode` flag beats the file.
#[test]
fn attach_mode_defaults_to_render_and_honors_file_then_flag() {
    assert_eq!(
        resolve(&ConfigFile::default(), &Overrides::default()).mode,
        "render"
    );
    let file = ConfigFile {
        client: ClientSection {
            mode: Some("passthrough".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(resolve(&file, &Overrides::default()).mode, "passthrough");
    let flagged = Overrides {
        mode: Some("render".into()),
        ..Default::default()
    };
    assert_eq!(resolve(&file, &flagged).mode, "render");
}

/// An empty env value counts as unset (a pane script expanding an
/// unset variable produces the empty string), so the file tier speaks.
#[test]
fn empty_env_socket_falls_through_to_the_file() {
    let file = ConfigFile {
        daemon: DaemonSection {
            socket: Some("/tmp/from-file.sock".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let eff = resolve(
        &file,
        &Overrides {
            env_socket: Some(String::new()),
            ..Default::default()
        },
    );
    assert_eq!(eff.socket, "/tmp/from-file.sock");
}

/// Every supported kind has a table key: the sentinel round-trip
/// fails for any kind whose key the struct lacks (the parser drops
/// unknown TOML keys silently). Unknown kinds and the blank default
/// read empty.
#[test]
fn agent_append_covers_every_supported_kind() {
    let mut text = String::from("[daemon.agent-append]\n");
    for kind in AGENT_APPEND_KINDS {
        text.push_str(&format!("{kind} = \"--{kind}-flag\"\n"));
    }
    let eff = resolve(
        &toml::from_str::<ConfigFile>(&text).unwrap(),
        &Overrides::default(),
    );
    for kind in AGENT_APPEND_KINDS {
        let want = format!("--{kind}-flag");
        assert_eq!(eff.agent_append.entry(kind), want, "{kind} keeps its entry");
    }
    assert_eq!(
        eff.agent_append.entry("unknown-agent"),
        "",
        "an unknown kind reads blank"
    );
    assert_eq!(
        EffectiveConfig::default().agent_append.entry("claude"),
        "",
        "the default table is blank"
    );
}

/// exit-empty's default is ON — exit-when-empty ships enabled, as
/// before the knob existed.
#[test]
fn exit_empty_defaults_to_on() {
    assert!(EffectiveConfig::default().exit_empty);
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert!(eff.exit_empty);
}

/// The product default: remain-on-exit is OFF (auto-remove) — both the
/// built-in default and a file silent on the key.
#[test]
fn remain_on_exit_defaults_to_auto_remove() {
    assert!(!EffectiveConfig::default().remain_on_exit);
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert!(!eff.remain_on_exit);
}

/// The exit-action default: `remove` for a file silent on the key,
/// `hold` while the legacy `remain-on-exit = true` stands, an
/// explicit key winning over the boolean, and an unknown spelling
/// keeping `remove` (the split-start-dir rule).
#[test]
fn exit_action_resolves_from_key_remain_on_exit_and_defaults() {
    assert_eq!(EffectiveConfig::default().exit_action, ExitAction::Remove);
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert_eq!(eff.exit_action, ExitAction::Remove);
    let file: ConfigFile = toml::from_str("[daemon]\nremain-on-exit = true").unwrap();
    let eff = resolve(&file, &Overrides::default());
    assert_eq!(
        eff.exit_action,
        ExitAction::Hold,
        "the legacy boolean folds to hold"
    );
    for spelling in ["hold", "shell", "remove"] {
        let file: ConfigFile = toml::from_str(&format!(
            "[daemon]\nremain-on-exit = true\nexit-action = \"{spelling}\""
        ))
        .unwrap();
        let eff = resolve(&file, &Overrides::default());
        assert_eq!(
            eff.exit_action,
            ExitAction::parse(spelling).unwrap(),
            "an explicit exit-action wins over remain-on-exit"
        );
    }
    let file: ConfigFile = toml::from_str("[daemon]\nexit-action = \"nonsense\"").unwrap();
    let eff = resolve(&file, &Overrides::default());
    assert_eq!(
        eff.exit_action,
        ExitAction::Remove,
        "an unknown spelling keeps remove"
    );
}
