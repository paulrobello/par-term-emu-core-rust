//! Client chord and display-option tests: reload_client_chords resolution,
//! the defaults Chords::with_defaults spells, and gen-config parity.

use crate::mux::config::*;

/// `[client] agents-sort` resolves into the client chords (`grouped`
/// default, `priority` respected, an unknown word keeping the
/// current), and the generated file spells the setting.
#[test]
fn agents_sort_round_trips_through_the_file_and_chords() {
    let mut file = ConfigFile::default();
    file.client.agents_sort = Some("priority".to_string());
    let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("chords");
    assert_eq!(chords.agents_sort, "priority");

    // An unknown word keeps the current value.
    file.client.agents_sort = Some("newest".to_string());
    let chords = reload_client_chords(&file, &chords).expect("chords");
    assert_eq!(chords.agents_sort, "priority");

    // The generated file names the key.
    let eff = resolve(&file, &Overrides::default());
    assert!(render(&eff).contains("agents-sort"));
}

/// The management chord overrides parse through the same prefix
/// grammar and rebind only the stated keys; a malformed spelling
/// errors with the key's name in the message.
/// The herdr-parity display options: absent keys keep the defaults
/// (all OFF — the byte-identical contract), stated keys resolve, and
/// gen-config emits all three at their defaults.
#[test]
fn session_picker_defaults_off_resolves_and_reload_keeps_it() {
    let defaults = Chords::with_defaults();
    assert!(!defaults.session_picker, "off unless the user opts in");
    let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
    assert!(
        !reload_client_chords(&absent, &defaults)
            .unwrap()
            .session_picker
    );
    let on: ConfigFile = toml::from_str("[client]\nsession-picker = true\n").unwrap();
    let chords = reload_client_chords(&on, &defaults).unwrap();
    assert!(chords.session_picker);
    assert!(
        reload_client_chords(&absent, &chords)
            .unwrap()
            .session_picker,
        "an absent key keeps the value in force"
    );
    let text = render(&EffectiveConfig::default());
    assert!(text.contains("session-picker = false"), "{text}");
}

/// The git badge key: default ON (unlike `session-picker`), a stated
/// key resolves, an absent key keeps the value in force, gen-config
/// emits it, and the daemon-side `EffectiveConfig` gate follows the
/// file.
#[test]
fn git_status_defaults_on_resolves_and_reload_keeps_it() {
    let defaults = Chords::with_defaults();
    assert!(defaults.git_status, "on by default");
    let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
    assert!(reload_client_chords(&absent, &defaults).unwrap().git_status);
    let off: ConfigFile = toml::from_str("[client]\ngit-status = false\n").unwrap();
    let chords = reload_client_chords(&off, &defaults).unwrap();
    assert!(!chords.git_status);
    assert!(
        !reload_client_chords(&absent, &chords).unwrap().git_status,
        "an absent key keeps the value in force (still the false above)"
    );
    // The daemon-side EffectiveConfig gate: default on, a false key
    // turns it off, and gen-config emits the default.
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert!(eff.git_status);
    let off_file: ConfigFile = toml::from_str("[client]\ngit-status = false\n").unwrap();
    assert!(!resolve(&off_file, &Overrides::default()).git_status);
    let text = render(&EffectiveConfig::default());
    assert!(text.contains("git-status = true"), "{text}");
}

/// `git-poll-secs`: default 10, 0 disables, otherwise clamped
/// 3-600 on resolve and reload,
/// and gen-config emits the default.
#[test]
fn git_poll_secs_defaults_clamps_and_emits() {
    let defaults = Chords::with_defaults();
    assert_eq!(defaults.git_poll_secs, 10);
    let absent: ConfigFile = toml::from_str("[client]\n").unwrap();
    assert_eq!(
        reload_client_chords(&absent, &defaults)
            .unwrap()
            .git_poll_secs,
        10
    );
    let zero: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 0\n").unwrap();
    assert_eq!(
        reload_client_chords(&zero, &defaults)
            .unwrap()
            .git_poll_secs,
        0,
        "0 disables the poll"
    );
    let huge: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 99999\n").unwrap();
    assert_eq!(
        reload_client_chords(&huge, &defaults)
            .unwrap()
            .git_poll_secs,
        600,
        "above the ceiling clamps to 600"
    );
    let one: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 1\n").unwrap();
    assert_eq!(
        reload_client_chords(&one, &defaults).unwrap().git_poll_secs,
        3,
        "1-2 clamps to the 3s floor (never below the 2s git run)"
    );
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert_eq!(eff.git_poll_secs, 10);
    let file: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 45\n").unwrap();
    assert_eq!(resolve(&file, &Overrides::default()).git_poll_secs, 45);
    let off: ConfigFile = toml::from_str("[client]\ngit-poll-secs = 0\n").unwrap();
    assert_eq!(
        resolve(&off, &Overrides::default()).git_poll_secs,
        0,
        "0 disables the poll on the resolve path too"
    );
    let text = render(&EffectiveConfig::default());
    assert!(text.contains("git-poll-secs = 10"), "{text}");
}

#[test]
fn herdr_parity_display_options_resolve_and_gen_config_emits() {
    let defaults = Chords::with_defaults();
    let empty: ConfigFile = toml::from_str("[client]\n").unwrap();
    let kept = reload_client_chords(&empty, &defaults).unwrap();
    assert_eq!(
        (
            kept.pane_gaps,
            kept.scrollbar_gutter,
            kept.change_cursor_shape
        ),
        (0, false, true),
        "absent keys keep the defaults (change-cursor-shape on)"
    );
    assert_eq!(
        (kept.tab_width_min, kept.tab_width_max),
        (0, 0),
        "absent tab-width bounds keep the unbounded defaults"
    );
    let file: ConfigFile = toml::from_str(
            "[client]\npane-gaps = 2\nscrollbar-gutter = true\nchange-cursor-shape = false\ntab-width-min = 12\ntab-width-max = 40\n",
        )
        .unwrap();
    let moved = reload_client_chords(&file, &defaults).unwrap();
    assert_eq!(
        (
            moved.pane_gaps,
            moved.scrollbar_gutter,
            moved.change_cursor_shape
        ),
        (2, true, false),
        "stated keys resolve; an explicit false beats the on default"
    );
    assert_eq!(
        (moved.tab_width_min, moved.tab_width_max),
        (12, 40),
        "stated tab-width bounds resolve"
    );
    // gen-config emits all three; the output parses back.
    let text = render(&EffectiveConfig::default());
    let file: ConfigFile = toml::from_str(&text).unwrap();
    assert_eq!(file.client.pane_gaps, Some(0), "gen-config emits pane-gaps");
    assert_eq!(
        file.client.tab_width_min,
        Some(0),
        "gen-config emits tab-width-min"
    );
    assert_eq!(
        file.client.tab_width_max,
        Some(0),
        "gen-config emits tab-width-max"
    );
    assert_eq!(
        file.client.scrollbar_gutter,
        Some(false),
        "gen-config emits scrollbar-gutter"
    );
    assert_eq!(
        file.client.change_cursor_shape,
        Some(true),
        "gen-config emits change-cursor-shape"
    );
}

/// The rename (`drag-cursor-shape` → `change-cursor-shape`): the
/// `[client]` section tolerates unknown keys, so a pre-rename file
/// still loads and the new default applies — the old key never pins
/// the setting off.
#[test]
fn change_cursor_shape_renamed_key_and_merge() {
    let defaults = Chords::with_defaults();
    let empty: ConfigFile = toml::from_str("[client]\n").unwrap();
    assert!(
        reload_client_chords(&empty, &defaults)
            .unwrap()
            .change_cursor_shape,
        "the default is on"
    );
    let off: ConfigFile = toml::from_str("[client]\nchange-cursor-shape = false\n").unwrap();
    assert!(
        !reload_client_chords(&off, &defaults)
            .unwrap()
            .change_cursor_shape,
        "an explicit false beats the on default"
    );
    let legacy: ConfigFile = toml::from_str("[client]\ndrag-cursor-shape = false\n").unwrap();
    assert!(
        reload_client_chords(&legacy, &defaults)
            .unwrap()
            .change_cursor_shape,
        "the pre-rename key is an unknown key: the default applies"
    );
}

#[test]
fn parse_chord_understands_m_keys() {
    assert_eq!(parse_chord("M-1"), Some(ChordKey::alt(b'1')));
    assert_eq!(parse_chord("m-a"), Some(ChordKey::alt(b'a')));
    assert_eq!(parse_chord("M-A"), Some(ChordKey::alt(b'A')));
    assert_eq!(parse_chord("M-%"), Some(ChordKey::alt(b'%')));
    assert_eq!(parse_chord("M-"), None);
    assert_eq!(parse_chord("M-12"), None);
    assert_eq!(parse_chord("M-C-a"), None);
}

#[test]
fn parse_chord_keeps_plain_and_c_keys() {
    assert_eq!(parse_chord("%"), Some(ChordKey::plain(b'%')));
    assert_eq!(parse_chord("C-a"), Some(ChordKey::plain(0x01)));
    assert_eq!(parse_chord("C-Space"), Some(ChordKey::plain(0x00)));
    assert_eq!(parse_chord("M"), Some(ChordKey::plain(b'M')));
    assert_eq!(parse_chord("Mx"), None);
    assert_eq!(parse_chord("ab"), None);
    assert_eq!(parse_chord(""), None);
}

#[test]
fn spell_chord_round_trips_parse_chord() {
    for spec in ["M-1", "%", "C-a", "C-Space", "x", "M-x", "M-%"] {
        let parsed = parse_chord(spec).expect(spec);
        assert_eq!(spell_chord(&parsed), spec);
    }
}

#[test]
fn config_file_layout_keys_round_trip() {
    let defaults = Chords::with_defaults();
    let file: ConfigFile =
        toml::from_str("[client]\nlayout-even-horizontal = \"M-x\"\nlayout-tiled = \"t\"\n")
            .unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(m.layout_even_horizontal, ChordKey::alt(b'x'));
    assert_eq!(m.layout_tiled, ChordKey::plain(b't'));
    assert_eq!(
        m.layout_even_vertical,
        ChordKey::alt(b'2'),
        "unstated keeps default"
    );
    let bad: ConfigFile = toml::from_str("[client]\nlayout-tiled = \"M-\"\n").unwrap();
    assert!(reload_client_chords(&bad, &defaults).is_err());
    // The generated file does not emit the layout keys (defaults stand).
    let eff = resolve(&ConfigFile::default(), &Overrides::default());
    assert!(!render(&eff).contains("layout-"));
}

#[test]
fn management_chords_parse_route_and_error() {
    let defaults = Chords {
        prefix: 0x02,
        reload: 0x12,
        ..Chords::with_defaults()
    };
    // Defaults.
    let file: ConfigFile = toml::from_str("[client]\n").unwrap();
    assert_eq!(
        reload_client_chords(&file, &defaults).unwrap().management,
        Management {
            split_right: ChordKey::plain(b'%'),
            split_down: ChordKey::plain(b'"'),
            kill_pane: ChordKey::plain(b'x'),
            break_pane: ChordKey::plain(b'!'),
            join_pane: ChordKey::plain(b'J'),
            palette: ChordKey::plain(b':'),
            new_window: ChordKey::plain(b'c'),
            resize: ChordKey::plain(b'R'),
            swap_prev: ChordKey::plain(b'{'),
            swap_next: ChordKey::plain(b'}'),
            move_window_left: ChordKey::plain(b'<'),
            move_window_right: ChordKey::plain(b'>'),
            tab_strip_left: ChordKey::plain(b'H'),
            tab_strip_right: ChordKey::plain(b'L'),
            workspace_next: ChordKey::plain(b'W'),
            workspace_prev: ChordKey::plain(0x17),
            help: ChordKey::plain(b'?'),
            picker: ChordKey::plain(b'w'),
            zoom: ChordKey::plain(b'z'),
            rename_window: ChordKey::plain(b','),
            rename_pane: ChordKey::plain(b'$'),
            border_cycle: ChordKey::plain(b'B'),
            label_toggle: ChordKey::plain(b'l'),
            workspace_picker: ChordKey::plain(b'g'),
            sidebar: ChordKey::plain(b's'),
            status_bar: ChordKey::plain(b'S'),
            respawn: ChordKey::plain(b'r'),
            notify_jump: ChordKey::plain(b'j'),
            ..Management::default()
        }
    );
    // Full override, tmux spellings and literals alike.
    let file: ConfigFile = toml::from_str(
            "[client]\nsplit-right = \"C-s\"\nsplit-down = \"v\"\nkill-pane = \"C-x\"\nnew-window = \"W\"\n",
        )
        .unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(m.split_right, ChordKey::plain(0x13));
    assert_eq!(m.split_down, ChordKey::plain(b'v'));
    assert_eq!(m.kill_pane, ChordKey::plain(0x18));
    assert_eq!(m.new_window, ChordKey::plain(b'W'));
    // A partial override keeps the untouched keys.
    let partial: ConfigFile = toml::from_str("[client]\nnew-window = \"n\"\n").unwrap();
    let m = reload_client_chords(&partial, &defaults)
        .unwrap()
        .management;
    assert_eq!(
        m.split_right,
        ChordKey::plain(b'%'),
        "unstated keeps the current/default"
    );
    assert_eq!(m.new_window, ChordKey::plain(b'n'));
    // The workspace chords parse through the same grammar; unstated
    // keeps the defaults (W / C-w).
    let file: ConfigFile =
        toml::from_str("[client]\nworkspace-next = \"C-n\"\nworkspace-prev = \"C-p\"\n").unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(
        m.workspace_next,
        ChordKey::plain(0x0e),
        "C-n overrides the W default"
    );
    assert_eq!(
        m.workspace_prev,
        ChordKey::plain(0x10),
        "C-p overrides the C-w default"
    );
    let empty: ConfigFile = toml::from_str("[client]\n").unwrap();
    let m = reload_client_chords(&empty, &defaults).unwrap().management;
    assert_eq!(m.workspace_next, ChordKey::plain(b'W'));
    assert_eq!(m.workspace_prev, ChordKey::plain(0x17), "C-w");
    // Malformed spellings error, naming the key.
    for (key, spec) in [
        ("split-right", "C-"),
        ("split-down", "C-1"),
        ("kill-pane", "xy"),
        ("new-window", "C-bb"),
    ] {
        let body = format!("[client]\n{key} = \"{spec}\"\n");
        let file: ConfigFile = toml::from_str(&body).unwrap();
        let err = reload_client_chords(&file, &defaults).unwrap_err();
        assert!(err.contains(key), "the error names the key: {err}");
    }
}

/// The tab-strip scroll chords: default H / L, rebindable with the
/// tmux spelling, a partial override keeping the sibling's default.
#[test]
fn tab_strip_chords_parse_and_rebind() {
    let defaults = Chords::with_defaults();
    // Defaults.
    let file: ConfigFile = toml::from_str("[client]\n").unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(m.tab_strip_left, ChordKey::plain(b'H'));
    assert_eq!(m.tab_strip_right, ChordKey::plain(b'L'));
    // A rebind parses with the tmux spelling.
    let file: ConfigFile =
        toml::from_str("[client]\ntab-strip-left = \"C-j\"\ntab-strip-right = \"C-l\"\n").unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(m.tab_strip_left, ChordKey::plain(0x0a), "C-j");
    assert_eq!(m.tab_strip_right, ChordKey::plain(0x0c), "C-l");
    // A partial override keeps the sibling's default.
    let file: ConfigFile = toml::from_str("[client]\ntab-strip-left = \"h\"\n").unwrap();
    let m = reload_client_chords(&file, &defaults).unwrap().management;
    assert_eq!(m.tab_strip_left, ChordKey::plain(b'h'));
    assert_eq!(
        m.tab_strip_right,
        ChordKey::plain(b'L'),
        "unstated keeps the default"
    );
    // Malformed spellings error, naming the key.
    let file: ConfigFile = toml::from_str("[client]\ntab-strip-left = \"C-]\"\n").unwrap();
    let err = reload_client_chords(&file, &defaults).unwrap_err();
    assert!(
        err.contains("tab-strip-left"),
        "the error names the key: {err}"
    );
}

/// The reload chord resolves through the shared prefix parser: the
/// last token is the key; a malformed token anywhere is an error.
#[test]
fn reload_chord_key_is_the_last_token_and_validates_all_tokens() {
    assert_eq!(reload_chord_key("C-b C-r").unwrap(), 0x12);
    assert_eq!(reload_chord_key("C-b C-a").unwrap(), 0x01);
    // A single token is a chord of just that key.
    assert_eq!(reload_chord_key("C-r").unwrap(), 0x12);
    // The literal spellings the prefix parser accepts.
    assert_eq!(reload_chord_key("C-b C-Space").unwrap(), 0x00);
    assert!(reload_chord_key("").is_err());
    assert!(reload_chord_key("C-b C-1").is_err());
    assert!(
        reload_chord_key("C- C-r").is_err(),
        "a bad head is an error too"
    );
}

/// The resize affordances: the resize chord rebinds like the other
/// management keys, and the step floors at 1 — a zero step would make
/// every resize a no-op by construction.
#[test]
fn resize_chord_and_step_parse_with_the_step_floored() {
    let defaults = Chords::with_defaults();
    // Unstated keys keep their current values.
    let file: ConfigFile = toml::from_str("[client]\n").unwrap();
    let chords = reload_client_chords(&file, &defaults).unwrap();
    assert_eq!(chords.management.resize, ChordKey::plain(b'R'));
    assert_eq!(chords.resize_step, 1);
    // Full override: the chord through the shared grammar, the step
    // through the file tier.
    let file: ConfigFile = toml::from_str("[client]\nresize = \"C-z\"\nresize-step = 3\n").unwrap();
    let chords = reload_client_chords(&file, &defaults).unwrap();
    assert_eq!(chords.management.resize, ChordKey::plain(0x1a));
    assert_eq!(chords.resize_step, 3);
    // Zero and unstated-file floors: 0 clamps to 1.
    let file: ConfigFile = toml::from_str("[client]\nresize-step = 0\n").unwrap();
    let chords = reload_client_chords(&file, &defaults).unwrap();
    assert_eq!(chords.resize_step, 1, "a zero step floors at 1");
    // A malformed resize chord errors naming the key.
    let file: ConfigFile = toml::from_str("[client]\nresize = \"C-1\"\n").unwrap();
    let err = reload_client_chords(&file, &defaults).unwrap_err();
    assert!(err.contains("resize"), "the error names the key: {err}");
}

/// ARC-131: every display default the generated config spells equals
/// `Chords::with_defaults()`, and the generated file resolves back to
/// exactly that chord set.
#[test]
fn gen_config_display_defaults_equal_with_defaults() {
    let defaults = Chords::with_defaults();
    let text = render(&EffectiveConfig::default());
    let file: ConfigFile = toml::from_str(&text).expect("parses");
    let c = &file.client;
    assert_eq!(c.sidebar_width, Some(defaults.sidebar_width));
    assert_eq!(c.sidebar_on_launch, Some(defaults.sidebar_on_launch));
    assert_eq!(
        c.border_lines.as_deref(),
        Some(defaults.border_lines.as_str())
    );
    assert_eq!(c.show_label_in_border, Some(defaults.show_label_in_border));
    assert_eq!(c.pane_gaps, Some(defaults.pane_gaps));
    assert_eq!(c.scrollbar_gutter, Some(defaults.scrollbar_gutter));
    assert_eq!(c.change_cursor_shape, Some(defaults.change_cursor_shape));
    assert_eq!(c.kitty_graphics, Some(defaults.kitty_graphics));
    assert_eq!(c.session_picker, Some(defaults.session_picker));
    assert_eq!(
        reload_client_chords(&file, &Chords::with_defaults()).expect("resolves"),
        defaults
    );
}

/// `agent-rows` resolves to the two-line default when absent, follows
/// an explicit table, drops empty lines, and keeps an unknown token
/// for the renderer's warning (never a reload error); the generated
/// config spells the default.
#[test]
fn agent_rows_resolve_default_explicit_and_warn_on_unknown_tokens() {
    let resolve_with = |text: &str| {
        let file: ConfigFile = toml::from_str(text).expect("parses");
        reload_client_chords(&file, &Chords::with_defaults()).map(|c| c.agent_rows)
    };
    assert_eq!(resolve_with("").unwrap(), default_agent_rows());
    assert_eq!(
        resolve_with("[client]\nagent-rows = [[\"agent\", \"state\"], [], [\"pane\"]]\n").unwrap(),
        vec![
            vec!["agent".to_string(), "state".to_string()],
            vec!["pane".to_string()]
        ]
    );
    assert_eq!(
        resolve_with("[client]\nagent-rows = []\n").unwrap(),
        default_agent_rows(),
        "an empty table is the default"
    );
    let file: ConfigFile =
        toml::from_str("[client]\nagent-rows = [[\"agent\", \"bogus\"]]\n").unwrap();
    let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("never an error");
    let warnings = display_option_warnings(&chords);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("\"bogus\""), "{warnings:?}");
    let generated = render(&EffectiveConfig::default());
    let file: ConfigFile = toml::from_str(&generated).expect("parses");
    assert_eq!(file.client.agent_rows, Some(default_agent_rows()));
}

/// The notification keys resolve to `terminal` + blocked/done when
/// absent, follow explicit values (an empty state list notifies
/// nothing), and warn on unknown spellings without failing.
#[test]
fn notify_keys_resolve_defaults_explicit_and_warn_on_unknowns() {
    let resolve_with = |text: &str| {
        let file: ConfigFile = toml::from_str(text).expect("parses");
        reload_client_chords(&file, &Chords::with_defaults())
            .map(|c| (c.notify_delivery, c.notify_states))
    };
    assert_eq!(
        resolve_with("").unwrap(),
        (
            "terminal".to_string(),
            vec!["blocked".to_string(), "done".to_string()]
        )
    );
    assert_eq!(
        resolve_with("[client]\nnotify-delivery = \"system\"\nnotify-states = []\n").unwrap(),
        ("system".to_string(), Vec::new())
    );
    let file: ConfigFile = toml::from_str(
        "[client]\nnotify-delivery = \"toast\"\nnotify-states = [\"finished\", \"done\"]\n",
    )
    .unwrap();
    let chords = reload_client_chords(&file, &Chords::with_defaults()).expect("never an error");
    let warnings = display_option_warnings(&chords);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("\"toast\""), "{warnings:?}");
    assert!(warnings[1].contains("\"finished\""), "{warnings:?}");
    assert!(display_option_warnings(&Chords::with_defaults()).is_empty());
    let generated = render(&EffectiveConfig::default());
    assert!(
        generated.contains("notify-delivery = \"terminal\""),
        "{generated}"
    );
}

/// The sound keys resolve to off / OS player / built-ins / no mutes
/// when absent, follow explicit values, keep the current value for
/// an absent key on reload, and the generated config spells them.
#[test]
fn sound_keys_resolve_defaults_and_explicit_values() {
    let file: ConfigFile = toml::from_str("").unwrap();
    let chords = reload_client_chords(&file, &Chords::with_defaults()).unwrap();
    assert_eq!(chords.sound, SoundSettings::default());
    assert!(!chords.sound.enabled);
    let file: ConfigFile = toml::from_str(
        "[client]\nsound = true\nsound-player = \"mpv --no-video\"\n\
             sound-done = \"sounds/done.mp3\"\nsound-blocked = \"/tmp/b.wav\"\n\
             sound-mute = [\"droid\", \"codex\"]\n",
    )
    .unwrap();
    let chords = reload_client_chords(&file, &Chords::with_defaults()).unwrap();
    assert_eq!(
        chords.sound,
        SoundSettings {
            enabled: true,
            player: "mpv --no-video".to_string(),
            done: "sounds/done.mp3".to_string(),
            blocked: "/tmp/b.wav".to_string(),
            mute: vec!["droid".to_string(), "codex".to_string()],
        }
    );
    // An absent key keeps what is in force.
    let partial: ConfigFile = toml::from_str("[client]\nsound = false\n").unwrap();
    let kept = reload_client_chords(&partial, &chords).unwrap();
    assert!(!kept.sound.enabled);
    assert_eq!(kept.sound.mute, chords.sound.mute);
    let generated = render(&EffectiveConfig::default());
    for key in [
        "sound = false",
        "sound-player = \"\"",
        "sound-done = \"\"",
        "sound-blocked = \"\"",
        "sound-mute = []",
    ] {
        assert!(generated.contains(key), "{key}: {generated}");
    }
}

/// `sidebar-on-launch` resolves to true when absent, and follows an
/// explicit value either way; the generated config spells it.
#[test]
fn sidebar_on_launch_resolves_default_true_and_explicit_values() {
    let resolve_with = |text: &str| {
        let file: ConfigFile = toml::from_str(text).expect("parses");
        reload_client_chords(&file, &Chords::with_defaults())
            .expect("resolves")
            .sidebar_on_launch
    };
    assert!(resolve_with(""), "absent = default true");
    assert!(resolve_with("[client]\n"), "absent key = default true");
    assert!(!resolve_with("[client]\nsidebar-on-launch = false\n"));
    assert!(resolve_with("[client]\nsidebar-on-launch = true\n"));
    assert!(
        render(&EffectiveConfig::default()).contains("sidebar-on-launch = true"),
        "--gen-config documents the key"
    );
}
