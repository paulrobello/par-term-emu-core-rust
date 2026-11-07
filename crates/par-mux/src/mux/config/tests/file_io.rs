//! Config-file loading and gen-config rendering: load_file classification,
//! write_file refusal, render round-trips, and socket-value mapping.

use crate::mux::config::*;
use std::path::PathBuf;

#[test]
fn full_file_parses_every_key() {
    let file: ConfigFile = toml::from_str(
        r#"
[client]
prefix = "C-a"
mode = "render"
reload = "C-a C-r"
[daemon]
socket = "/tmp/par-mux-test.sock"
state-dir = "/tmp/par-mux-state"
pane-endpoints = true
expose-control-socket = true
remain-on-exit = true
"#,
    )
    .expect("parse");
    assert_eq!(file.client.prefix.as_deref(), Some("C-a"));
    assert_eq!(file.client.mode.as_deref(), Some("render"));
    assert_eq!(file.client.reload.as_deref(), Some("C-a C-r"));
    assert_eq!(
        file.daemon.socket.as_deref(),
        Some("/tmp/par-mux-test.sock")
    );
    assert_eq!(file.daemon.pane_endpoints, Some(true));
    assert_eq!(file.daemon.expose_control_socket, Some(true));
    assert_eq!(file.daemon.remain_on_exit, Some(true));
}

/// Unknown keys and missing sections are tolerated — the file is
/// forward-compatible and partially spellable.
#[test]
fn partial_and_unknown_keys_parse_to_defaults() {
    let file: ConfigFile =
        toml::from_str("[client]\nprefix = 'C-a'\nsome-future-key = 3").expect("parse");
    assert_eq!(file.client.prefix.as_deref(), Some("C-a"));
    assert_eq!(file.daemon, DaemonSection::default());
    let empty: ConfigFile = toml::from_str("").expect("parse");
    assert_eq!(empty, ConfigFile::default());
}

/// gen-config's output parses back to the same effective settings.
#[test]
fn render_round_trips_through_the_parser() {
    let eff = EffectiveConfig {
        agent_resume_prompt: "continue where you left off".into(),
        auto_update: true,
        update_interval: "weekly".into(),
        prefix: "C-a".into(),
        mode: "render".into(),
        reload: "C-a C-r".into(),
        socket: "/tmp/rt.sock".into(),
        state_dir: "/tmp/rt-state".into(),
        pane_endpoints: true,
        expose_control_socket: true,
        remain_on_exit: true,
        exit_action: ExitAction::Hold,
        exit_empty: true,
        border_active_color: "#00ff00".into(),
        border_color: "#202020".into(),
        git_status: true,
        git_poll_secs: 30,
        split_start_dir: "previous-pane-dir".into(),
        agent_append: AgentAppend {
            claude: "--continue".into(),
            ..AgentAppend::default()
        },
        config_path: None,
    };
    let file: ConfigFile = toml::from_str(&render(&eff)).expect("round-trip parse");
    assert_eq!(resolve(&file, &Overrides::default()), eff);
}

/// The default rendering carries every key a user can start from.
#[test]
fn default_render_spells_every_setting() {
    let text = render(&EffectiveConfig::default());
    for key in [
        "prefix",
        "mode",
        "reload",
        "socket",
        "state-dir",
        "pane-endpoints",
        "expose-control-socket",
        "remain-on-exit",
        "exit-action",
        "exit-empty",
        "agent-append",
    ] {
        assert!(text.contains(key), "default render names {key}: {text}");
    }
}

#[cfg(unix)]
#[test]
fn socket_value_maps_names_and_paths() {
    assert_eq!(
        socket_value_to_path("work"),
        crate::mux::default_socket_path("work")
    );
    assert_eq!(
        socket_value_to_path("default"),
        crate::mux::default_socket_path("default")
    );
    assert_eq!(
        socket_value_to_path(""),
        crate::mux::default_socket_path("default")
    );
    assert_eq!(
        socket_value_to_path("/tmp/explicit.sock"),
        PathBuf::from("/tmp/explicit.sock")
    );
}

/// load_file: absent = None, present parses, garbage errors with the
/// path in the message.
#[test]
fn load_file_classifies_absent_parsed_and_broken() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("absent.toml");
    assert_eq!(load_file(&absent), Ok(None));

    let good = dir.path().join("good.toml");
    std::fs::write(&good, "[client]\nprefix = 'C-a'").unwrap();
    let parsed = load_file(&good).unwrap().expect("some file");
    assert_eq!(parsed.client.prefix.as_deref(), Some("C-a"));

    let bad = dir.path().join("bad.toml");
    std::fs::write(&bad, "[client").unwrap();
    let err = load_file(&bad).unwrap_err();
    assert!(err.contains("bad.toml"), "the error names the file: {err}");
}

/// write_file refuses to overwrite without force, and force writes.
#[test]
fn write_file_refuses_overwrite_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sub").join("config.toml");
    write_file(&path, &EffectiveConfig::default(), false).expect("first write");
    let first = std::fs::read_to_string(&path).unwrap();

    let changed = EffectiveConfig {
        prefix: "C-z".into(),
        ..EffectiveConfig::default()
    };
    let err = write_file(&path, &changed, false).unwrap_err();
    assert!(
        err.contains("--force"),
        "refusal names the escape hatch: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        first,
        "the existing file survived the refusal"
    );

    write_file(&path, &changed, true).expect("forced rewrite");
    assert!(std::fs::read_to_string(&path).unwrap().contains("C-z"));
}

/// Hostile files never panic the loader: wrong-typed values, unknown
/// keys, a 1MB string value, invalid UTF-8 (rejected, not lossily
/// decoded), duplicate keys, and a directory-shaped path all come back
/// as classified errors or clean parses per the documented contract.
#[test]
fn load_file_survives_hostile_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = |name: &str| dir.path().join(name);

    // Wrong-typed values: a string field given an integer / bool.
    std::fs::write(path("wrong_type.toml"), "[client]\nprefix = 5\n").expect("write");
    assert!(
        load_file(&path("wrong_type.toml")).is_err(),
        "int for a string field is a parse error"
    );
    std::fs::write(
        path("wrong_type2.toml"),
        "[daemon]\npane-endpoints = 'yes'\n",
    )
    .expect("write");
    assert!(
        load_file(&path("wrong_type2.toml")).is_err(),
        "string for a bool field is a parse error"
    );

    // Unknown keys are forward-compat ignored and resolve to defaults.
    std::fs::write(
        path("unknown.toml"),
        "[client]\nbogus = 'x'\n[daemon]\nnope = 1\n",
    )
    .expect("write");
    let file = load_file(&path("unknown.toml"))
        .expect("no io error")
        .expect("parses");
    assert_eq!(
        resolve(&file, &Overrides::default()),
        EffectiveConfig::default()
    );

    // A 1MB string value parses (no size cliff in the reader).
    let big = format!("[client]\nbogus = '{}'\n", "a".repeat(1 << 20));
    std::fs::write(path("big.toml"), big).expect("write");
    assert!(load_file(&path("big.toml")).is_ok());

    // Invalid UTF-8: rejected (read_to_string), never a panic.
    std::fs::write(path("utf8.toml"), b"[client]\nprefix = '\xff\xfe'\n").expect("write");
    assert!(load_file(&path("utf8.toml")).is_err(), "rejected");

    // Duplicate keys are a TOML error.
    std::fs::write(
        path("dup.toml"),
        "[client]\nprefix = 'C-b'\nprefix = 'C-a'\n",
    )
    .expect("write");
    assert!(load_file(&path("dup.toml")).is_err(), "duplicate key");

    // A directory where the file should be: an io error naming the
    // path, not a panic and not Ok(None).
    let err = load_file(dir.path()).expect_err("directory read fails");
    assert!(
        err.contains(dir.path().to_string_lossy().as_ref()),
        "the error names the path: {err}"
    );
}

/// gen-config output round-trips: write_file → load_file → resolve
/// reproduces the effective config it was generated from, for both
/// the defaults and a mutated prefix.
#[test]
fn gen_config_round_trips_through_load_and_resolve() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    write_file(&path, &EffectiveConfig::default(), false).expect("write");
    let loaded = load_file(&path)
        .expect("no io error")
        .expect("written file parses");
    assert_eq!(
        resolve(&loaded, &Overrides::default()),
        EffectiveConfig::default()
    );

    let mutated = EffectiveConfig {
        prefix: "C-a".into(),
        mode: "render".into(),
        ..EffectiveConfig::default()
    };
    let path2 = dir.path().join("mutated.toml");
    write_file(&path2, &mutated, false).expect("write");
    let loaded = load_file(&path2)
        .expect("no io error")
        .expect("written file parses");
    assert_eq!(resolve(&loaded, &Overrides::default()), mutated);
}
