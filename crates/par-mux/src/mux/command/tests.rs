use super::*;

/// A payload of one verbatim byte run.
fn bytes(b: &[u8]) -> SendKeysPayload {
    SendKeysPayload(vec![SendKeysPart::Bytes(b.to_vec())])
}

/// The send-keys payload of `line`, encoded against `term`.
fn encoded_on(line: &str, term: &Terminal) -> Vec<u8> {
    let MuxCommand::SendKeys { keys, .. } = parse_command(line).expect("parses") else {
        panic!("not send-keys: {line}");
    };
    keys.encode(|| term)
}

/// The send-keys payload of `line`, encoded against a fresh terminal.
fn encoded(line: &str) -> Vec<u8> {
    encoded_on(line, &Terminal::new(80, 24))
}

/// Every fuzz corpus seed runs through the parser without panicking —
/// the same driver the `mux_parse_command` fuzz target uses, so the
/// corpus is regression-covered even where cargo-fuzz is not run. The
/// CI fuzz job picks the seeds up automatically.
#[test]
fn fuzz_corpus_seeds_parse_without_panicking() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus/mux_parse_command");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("corpus directory exists") {
        let path = entry.expect("corpus entry").path();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let _ = parse_command(&text);
        let _ = parse_line(&text);
        count += 1;
    }
    assert!(
        count >= 30,
        "corpus seeds went missing: only {count} under {}",
        dir.display()
    );
}

#[test]
fn pane_targets_parse_as_names_and_ids() {
    assert_eq!(
        parse_command("pane-title -t build").unwrap(),
        MuxCommand::PaneTitle {
            pane: Target::Name("build".to_string()),
        }
    );
    // A quoted name keeps its spaces, same grammar as -s/-n names.
    assert_eq!(
        parse_command("pane-title -t 'my build pane'").unwrap(),
        MuxCommand::PaneTitle {
            pane: Target::Name("my build pane".to_string()),
        }
    );
    // Sigil-prefixed values stay ids; malformed ones stay errors.
    assert_eq!(
        parse_command("pane-title -t %3").unwrap(),
        MuxCommand::PaneTitle {
            pane: Target::Id(PaneId(3)),
        }
    );
    assert!(parse_command("pane-title -t %abc").is_err());
}

#[test]
fn window_and_session_targets_parse_as_names() {
    assert_eq!(
        parse_command("select-window -t logs").unwrap(),
        MuxCommand::SelectWindow {
            window: Target::Name("logs".to_string()),
        }
    );
    assert_eq!(
        parse_command("set-environment -t work K V").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Name("work".to_string()),
            name: "K".into(),
            value: Some("V".into()),
        }
    );
    // new-window's optional session target accepts a name too.
    assert_eq!(
        parse_command("new-window -t alpha -n logs").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Name("alpha".to_string())),
            name: Some("logs".into()),
            start_dir: None,
        }
    );
}

#[test]
fn send_keys_target_may_be_a_single_word_name() {
    assert_eq!(
        parse_command("send-keys -t build Enter").unwrap(),
        MuxCommand::SendKeys {
            pane: Target::Name("build".to_string()),
            keys: bytes(b"\r"),
        }
    );
}

#[test]
fn parses_version_and_marks_it_read_only() {
    let cmd = parse_command("version").expect("parses");
    assert_eq!(cmd, MuxCommand::Version);
    assert!(!cmd.mutates(), "version must never trigger a state save");
}

#[test]
fn parses_new_session_with_a_name() {
    let cmd = parse_command("new-session -s work").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            name: Some("work".into()),
            env: vec![],
            workspace: None,
        }
    );
}

#[test]
fn parses_new_session_env_assignments_in_order_with_quoting() {
    let cmd = parse_command("new-session -s work -e A=1 -e 'B=two words' -e C=x=y -e D=").unwrap();
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            name: Some("work".into()),
            env: vec![
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "x=y".into()),
                ("D".into(), String::new()),
            ],
            workspace: None,
        }
    );
}

#[test]
fn new_session_env_rejects_malformed_assignments() {
    assert!(parse_command("new-session -e NOEQUALS").is_err());
    assert!(parse_command("new-session -e =value").is_err());
}

#[test]
fn parses_set_environment_set_and_unset() {
    assert_eq!(
        parse_command("set-environment -t $2 SSH_AUTH_SOCK /tmp/agent.sock").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(2)),
            name: "SSH_AUTH_SOCK".into(),
            value: Some("/tmp/agent.sock".into()),
        }
    );
    assert_eq!(
        parse_command("set-environment -t $0 GREETING 'hello there'").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(0)),
            name: "GREETING".into(),
            value: Some("hello there".into()),
        }
    );
    assert_eq!(
        parse_command("set-environment -t $0 EMPTY ''").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(0)),
            name: "EMPTY".into(),
            value: Some(String::new()),
        }
    );
    assert_eq!(
        parse_command("set-environment -u -t $1 DISPLAY").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(1)),
            name: "DISPLAY".into(),
            value: None,
        }
    );
}

#[test]
fn set_environment_rejects_bad_shapes() {
    for line in [
        "set-environment NAME value",
        "set-environment -t $0",
        "set-environment -t $0 NAME",
        "set-environment -t $0 NAME a b",
        "set-environment -t $0 -u",
        "set-environment -t $0 -u NAME extra",
        "set-environment -t $0 A=B value",
    ] {
        assert!(parse_command(line).is_err(), "{line} must be rejected");
    }
    // `-t %0` (pane sigil on a session target) is no longer a parse
    // error: non-`$` values are session NAMES now, resolved (and
    // reported unknown) daemon-side.
}

#[test]
fn parses_new_session_without_a_name() {
    let cmd = parse_command("new-session").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            name: None,
            env: vec![],
            workspace: None,
        }
    );
}

/// A quoted session name survives as one name. Before the fix the flag
/// scan read the pre-split tokens, so `-s 'Par Mux Test'` created a
/// session literally called `'Par` — the bug par-term filed.
#[test]
fn new_session_keeps_a_quoted_name_whole() {
    for line in [
        "new-session -s 'Par Mux Test'",
        "new-session -s \"Par Mux Test\"",
    ] {
        assert_eq!(
            parse_command(line).expect("parses"),
            MuxCommand::NewSession {
                name: Some("Par Mux Test".into()),
                env: vec![],
                workspace: None,
            },
            "line: {line}"
        );
    }
}

/// The `'\''` close-escape-reopen idiom yields a real single quote —
/// the same escaping `send-keys` payloads already round-trip.
#[test]
fn new_session_resolves_the_embedded_quote_idiom() {
    let cmd = parse_command(r"new-session -s 'Paul'\''s box'").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            name: Some("Paul's box".into()),
            env: vec![],
            workspace: None,
        }
    );
}

/// Quoting changes nothing for a name that never needed it: an
/// unquoted name, a trailing flag after it, and a bare `-s` with no
/// value all behave exactly as they did under the flat split.
#[test]
fn new_session_flat_names_are_unchanged() {
    assert_eq!(
        parse_command("new-session -s work").expect("parses"),
        MuxCommand::NewSession {
            name: Some("work".into()),
            env: vec![],
            workspace: None,
        }
    );
    assert_eq!(
        parse_command("new-session   -s   work  ").expect("parses"),
        MuxCommand::NewSession {
            name: Some("work".into()),
            env: vec![],
            workspace: None,
        }
    );
    assert_eq!(
        parse_command("new-session -s").expect("parses"),
        MuxCommand::NewSession {
            name: None,
            env: vec![],
            workspace: None,
        }
    );
    // An unquoted value keeps the flat scan's verbatim bytes: a bare
    // backslash is part of the name, not an escape.
    assert_eq!(
        parse_command(r"new-session -s a\b").expect("parses"),
        MuxCommand::NewSession {
            name: Some(r"a\b".into()),
            env: vec![],
            workspace: None,
        }
    );
}

/// The flag scan finds the name flag itself, not a same-looking value
/// of an earlier flag, and works with the name flag in any slot.
#[test]
fn quoted_names_may_look_like_flags() {
    assert_eq!(
        parse_command("new-session -s '-n'").expect("parses"),
        MuxCommand::NewSession {
            name: Some("-n".into()),
            env: vec![],
            workspace: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t $0 -n '-s'").expect("parses"),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("-s".into()),
            start_dir: None,
        }
    );
    // The name flag need not come first.
    assert_eq!(
        parse_command("new-window -n 'two words' -t $0").expect("parses"),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("two words".into()),
            start_dir: None,
        }
    );
}

/// An empty name is only expressible through quoting, so the quoted
/// path is where it has to be rejected — `""` is not a tmux name, and
/// silently falling back to the default would hide a client bug.
#[test]
fn new_session_rejects_an_explicitly_empty_name() {
    let err = parse_command("new-session -s ''").expect_err("empty name is an error");
    assert!(err.contains("-s"), "error names the flag: {err}");
}

/// `new-window -n` is the same flag shape as `new-session -s` and got
/// the same fix; its target stays a typed `$N` id.
#[test]
fn new_window_keeps_a_quoted_name_whole() {
    assert_eq!(
        parse_command("new-window -t $0 -n 'build and test'").expect("parses"),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("build and test".into()),
            start_dir: None,
        }
    );
    assert!(parse_command("new-window -t $0 -n ''").is_err());
}

#[test]
fn parses_send_keys_with_a_target() {
    let cmd = parse_command("send-keys -t %3 hello").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::SendKeys {
            pane: Target::Id(PaneId(3)),
            keys: bytes(b"hello")
        }
    );
}

#[test]
fn send_keys_tolerates_leading_whitespace_before_the_name() {
    // Fuzz-found (ENH-018): split_whitespace finds the name after any
    // leading whitespace, but the raw-line strip assumed byte 0.
    let cmd = parse_command("\t send-keys -t %3 Enter").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::SendKeys {
            pane: Target::Id(PaneId(3)),
            keys: bytes(b"\r")
        }
    );
}

#[test]
fn send_keys_interprets_key_names() {
    let cmd = parse_command("send-keys -t %3 C-c").expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::SendKeys {
            pane: Target::Id(PaneId(3)),
            keys: bytes(&[0x03])
        }
    );
    assert_eq!(
        encoded("send-keys -t %3 C-Space Escape BSpace Space Enter Tab"),
        vec![0x00, 0x1b, 0x7f, b' ', 0x0d, 0x09]
    );
}

#[test]
fn send_keys_maps_arrows_to_csi_sequences() {
    assert_eq!(
        encoded("send-keys -t %3 Up Down Left Right"),
        b"\x1b[A\x1b[B\x1b[D\x1b[C".to_vec()
    );
}

#[test]
fn send_keys_arrows_honor_application_cursor_mode() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?1h");
    assert_eq!(
        encoded_on("send-keys -t %3 Up Down Right Left Home End", &term),
        b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF".to_vec()
    );
}

#[test]
fn send_keys_resolves_navigation_and_function_keys() {
    for (token, expected) in [
        ("Home", &b"\x1b[H"[..]),
        ("End", b"\x1b[F"),
        ("PPage", b"\x1b[5~"),
        ("PageUp", b"\x1b[5~"),
        ("PgUp", b"\x1b[5~"),
        ("NPage", b"\x1b[6~"),
        ("PageDown", b"\x1b[6~"),
        ("PgDn", b"\x1b[6~"),
        ("IC", b"\x1b[2~"),
        ("Insert", b"\x1b[2~"),
        ("DC", b"\x1b[3~"),
        ("Delete", b"\x1b[3~"),
        ("F1", b"\x1bOP"),
        ("F4", b"\x1bOS"),
        ("F5", b"\x1b[15~"),
        ("F12", b"\x1b[24~"),
        ("BTab", b"\x1b[Z"),
    ] {
        assert_eq!(
            encoded(&format!("send-keys -t %3 {token}")),
            expected.to_vec(),
            "{token}"
        );
    }
}

#[test]
fn send_keys_byte_class_names_ignore_terminal_modes() {
    // par-term sends already-encoded keystrokes as these names;
    // re-encoding them against the pane's modes would double-encode.
    let line = "send-keys -t %3 C-c Escape BSpace Space Enter Tab C-Space";
    let raw = vec![0x03, 0x1b, 0x7f, b' ', 0x0d, 0x09, 0x00];
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>4;2m");
    assert_eq!(encoded_on(line, &term), raw);
    term.process(b"\x1b[>1u");
    assert_eq!(encoded_on(line, &term), raw);
    // A navigation key does follow the kitty flags.
    assert_eq!(
        encoded_on("send-keys -t %3 F5", &term),
        b"\x1b[57380u".to_vec()
    );
}

#[test]
fn send_keys_plain_bytes_never_read_the_terminal() {
    let MuxCommand::SendKeys { keys, .. } =
        parse_command("send-keys -t %3 'hi ' 0xe2 C-c Enter").expect("parses")
    else {
        panic!("send-keys");
    };
    let out = keys.encode(|| -> &Terminal { panic!("a byte payload must not lock") });
    assert_eq!(out, b"hi \xe2\x03\r".to_vec());
}

#[test]
fn send_keys_resolves_quoted_runs_and_the_quote_idiom() {
    // Quoted runs are literal, spaces inside them survive, and the
    // '\'' idiom yields a real single quote — the escape_keys_for_tmux
    // round trip.
    assert_eq!(
        encoded("send-keys -t %3 'hello world'"),
        b"hello world".to_vec()
    );
    assert_eq!(encoded("send-keys -t %3 'it'\\''s'"), b"it's".to_vec());
}

#[test]
fn send_keys_space_between_bare_words_is_explicit_not_implicit() {
    // tmux semantics: tokens join with nothing between them; a space is
    // the Space key. escape_keys_for_tmux encodes exactly this.
    assert_eq!(
        encoded("send-keys -t %3 'hello' Space 'world'"),
        b"hello world".to_vec()
    );
    assert_eq!(
        encoded("send-keys -t %3 hello world"),
        b"helloworld".to_vec()
    );
}

#[test]
fn send_keys_literal_flag_disables_interpretation() {
    assert_eq!(encoded("send-keys -t %3 -l C-c"), b"C-c".to_vec());
    assert_eq!(encoded("send-keys -t %3 -l Home"), b"Home".to_vec());
}

#[test]
fn send_keys_literal_tokens_join_with_nothing_like_tmux() {
    // Measured on tmux 3.7c: `send-keys -t 0 -l echo LEFT` types
    // `echoLEFT` — tokens join with NOTHING in -l mode, same as default
    // mode. A space must live inside a quoted run. Pinned end to end in
    // tests/mux_send_keys.rs against a live daemon too.
    assert_eq!(
        encoded("send-keys -t %3 -l echo LEFT"),
        b"echoLEFT".to_vec()
    );
    assert_eq!(
        encoded("send-keys -t %3 -l 'echo LEFT'"),
        b"echo LEFT".to_vec()
    );
}

#[test]
fn send_keys_hex_flag_takes_byte_pairs() {
    // The form format_send_hex_keys emits for CSI-u sequences.
    assert_eq!(
        encoded("send-keys -t %3 -H 1b 5b 41"),
        vec![0x1b, 0x5b, 0x41]
    );

    assert!(parse_command("send-keys -t %3 -H zz").is_err());
}

#[test]
fn send_keys_bare_hex_token_is_one_byte() {
    assert_eq!(
        encoded("send-keys -t %3 0x1b 'prompt> '"),
        b"\x1bprompt> ".to_vec()
    );
}

#[test]
fn send_keys_round_trips_an_escape_keys_for_tmux_stream() {
    // Representative output of par-term's escape_keys_for_tmux for the
    // bytes b"hi \xe2\x82\xacC-c": printable run quoted, high bytes as
    // 0xNN tokens, the control key by name.
    let cmd = parse_command("send-keys -t %3 'hi ' 0xe2 0x82 0xac C-c").unwrap();
    let MuxCommand::SendKeys { keys, .. } = cmd else {
        panic!("send-keys");
    };
    assert_eq!(keys, bytes(b"hi \xe2\x82\xac\x03"));
}

#[test]
fn send_keys_requires_a_payload() {
    assert!(parse_command("send-keys -t %3").is_err());
    assert!(parse_command("send-keys -t %3 -l").is_err());
}

#[test]
fn parses_list_panes_and_kill_pane() {
    assert_eq!(
        parse_command("list-panes").unwrap(),
        MuxCommand::ListPanes { window: None }
    );
    assert_eq!(
        parse_command("kill-pane -t %7").unwrap(),
        MuxCommand::KillPane {
            pane: Target::Id(PaneId(7))
        }
    );
}

#[test]
fn rejects_an_unknown_command() {
    assert!(parse_command("frobnicate").is_err());
}

/// QA-219: a trailing command was silently dropped; it is now an error
/// pointing at respawn-pane.
#[test]
fn new_window_rejects_a_trailing_command() {
    for line in [
        "new-window sleep 5",
        "new-window -n w sleep 5",
        "new-window -t $0 -- top",
    ] {
        let err = parse_command(line).expect_err(line);
        assert!(err.contains("unexpected argument"), "{line}: {err}");
        assert!(err.contains("respawn-pane"), "{line}: {err}");
    }
}

#[test]
fn new_session_rejects_a_trailing_command() {
    let err = parse_command("new-session -s a top").expect_err("positional");
    assert!(err.contains("unexpected argument \"top\""), "{err}");
}

/// QA-219: tmux's value flags consume their value, so a tmux-shaped
/// sender is not misread; quoted values stay one word.
#[test]
fn new_session_accepts_tmux_value_flags() {
    for line in [
        "new-session -s a -x 80 -y 24 -d",
        "new-session -d -s 'a'\\''; kill-server; '\\'''",
        "new-session -s work -e A=1 -e 'B=two words'",
        "new-window -t $0 -n 'build and test' -c '/tmp/my dir'",
        "new-window -a -d -t alpha -n logs",
    ] {
        assert!(
            parse_command(line).is_ok(),
            "{line}: {:?}",
            parse_command(line)
        );
    }
}

/// ARC-094: every `COMMANDS` row is reachable. A bare name may be a
/// usage error, but never "unknown command" (a name `parse_command`
/// cannot look up, e.g. one with whitespace), and no name appears twice
/// (the later row's parser would be dead). It does not prove a row
/// names the right parser; `mutates()` and `dispatch_command` stay
/// exhaustive matches for that.
#[test]
fn every_command_table_name_parses_or_errors_on_its_own_grammar() {
    let mut seen = std::collections::HashSet::new();
    for (name, _, _) in COMMANDS {
        assert!(seen.insert(*name), "duplicate command table row {name:?}");
        if let Err(err) = parse_command(name) {
            assert!(
                !err.starts_with("unknown command"),
                "table row {name:?} does not reach its parser: {err}"
            );
        }
    }
}

/// ENH-037: the `list-commands` reply carries every `COMMANDS` name
/// exactly once, plus the daemon-level `features` line.
#[test]
fn list_commands_reply_lists_every_command_exactly_once() {
    let body = list_commands_body();
    let lines: Vec<&str> = body.lines().collect();
    for (name, _, _) in COMMANDS {
        let hits = lines
            .iter()
            .filter(|line| line.split_whitespace().next() == Some(*name))
            .count();
        assert_eq!(
            hits, 1,
            "command {name:?} appears {hits} times in the reply"
        );
    }
    assert!(lines.contains(&"features replay-held-state"));
}

/// ENH-037/ARC-094b: feature tokens live on the command's own table
/// row; the documented wire grammar is `[a-z-]+`, and anything else
/// would hand clients a token they are told to treat as well-formed.
#[test]
fn every_command_feature_token_matches_the_wire_grammar() {
    for (name, _, features) in COMMANDS {
        for token in *features {
            assert!(
                !token.is_empty() && token.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "command {name:?} advertises malformed feature token {token:?}"
            );
        }
    }
}

#[test]
fn rejects_a_malformed_target() {
    // A non-sigil value is a NAME now, parsed fine and resolved
    // daemon-side; only sigil-prefixed garbage is malformed.
    assert!(parse_command("kill-pane -t %notapane").is_err());
    assert!(
        parse_command("kill-pane").is_err(),
        "kill-pane needs a target"
    );
}

#[test]
fn parses_new_window_with_and_without_a_name() {
    assert_eq!(
        parse_command("new-window -t $0 -n build").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("build".into()),
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t $0").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: None,
            start_dir: None,
        }
    );
    // Bare new-window — the form tmux clients issue — targets the
    // most-recently-created session, resolved server-side.
    assert_eq!(
        parse_command("new-window").unwrap(),
        MuxCommand::NewWindow {
            target: None,
            name: None,
            start_dir: None,
        }
    );
}

#[test]
fn parses_select_and_kill_window() {
    assert_eq!(
        parse_command("select-window -t @2").unwrap(),
        MuxCommand::SelectWindow {
            window: Target::Id(WindowId(2))
        }
    );
    assert_eq!(
        parse_command("kill-window -t @2").unwrap(),
        MuxCommand::KillWindow {
            window: Target::Id(WindowId(2))
        }
    );
}

#[test]
fn parses_rename_window_and_rejects_a_missing_name() {
    assert_eq!(
        parse_command("rename-window -t @1 scratch").unwrap(),
        MuxCommand::RenameWindow {
            window: Target::Id(WindowId(1)),
            name: "scratch".into()
        }
    );
    assert!(
        parse_command("rename-window -t @1").is_err(),
        "rename-window needs a new name"
    );
}

#[test]
fn parses_rename_session_with_shell_quoting() {
    assert_eq!(
        parse_command("rename-session -t $1 'My Session'").unwrap(),
        MuxCommand::RenameSession {
            session: Target::Id(SessionId(1)),
            name: "My Session".into()
        }
    );
    assert_eq!(
        parse_command(r#"rename-session -t $1 "double""#).unwrap(),
        MuxCommand::RenameSession {
            session: Target::Id(SessionId(1)),
            name: "double".into()
        }
    );
    // A name target resolves by session name, like every other
    // session-targeted command.
    assert_eq!(
        parse_command("rename-session -t alpha work").unwrap(),
        MuxCommand::RenameSession {
            session: Target::Name("alpha".into()),
            name: "work".into()
        }
    );
}

#[test]
fn rename_session_rejects_missing_target_name_and_extra_words() {
    assert!(parse_command("rename-session").is_err(), "-t is required");
    assert!(
        parse_command("rename-session -t $1").is_err(),
        "a new name is required"
    );
    assert!(
        parse_command("rename-session -t $1 one two").is_err(),
        "one name word only — quote a name with spaces"
    );
}

#[test]
fn parses_kill_session_requiring_a_target() {
    assert_eq!(
        parse_command("kill-session -t $1").unwrap(),
        MuxCommand::KillSession {
            session: Target::Id(SessionId(1))
        }
    );
    assert_eq!(
        parse_command("kill-session -t alpha").unwrap(),
        MuxCommand::KillSession {
            session: Target::Name("alpha".into())
        }
    );
    assert!(
        parse_command("kill-session").is_err(),
        "-t is required: no newest-session default (same rule as set-environment)"
    );
}

#[test]
fn parses_list_windows_and_list_sessions() {
    assert_eq!(
        parse_command("list-windows").unwrap(),
        MuxCommand::ListWindows { session: None }
    );
    assert_eq!(
        parse_command("list-sessions").unwrap(),
        MuxCommand::ListSessions { workspace: None }
    );
}

#[test]
fn parses_targeted_list_windows_and_list_panes() {
    assert_eq!(
        parse_command("list-windows -t $2").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Id(SessionId(2)))
        }
    );
    assert_eq!(
        parse_command("list-windows -t work").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Name("work".to_string()))
        }
    );
    assert_eq!(
        parse_command("list-panes -t @7").unwrap(),
        MuxCommand::ListPanes {
            window: Some(Target::Id(WindowId(7)))
        }
    );
    assert_eq!(
        parse_command("list-panes -t editor").unwrap(),
        MuxCommand::ListPanes {
            window: Some(Target::Name("editor".to_string()))
        }
    );
    // A tmux-shaped -F rides along and is ignored, not misread as a
    // positional pane name (QA-219's rule).
    assert_eq!(
        parse_command("list-windows -t work -F '#{window_name}'").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Name("work".to_string()))
        }
    );
    // A stray positional is rejected.
    assert!(parse_command("list-windows stray").is_err());
    assert!(parse_command("list-panes stray").is_err());
}

#[test]
fn parses_list_agents() {
    assert_eq!(
        parse_command("list-agents").unwrap(),
        MuxCommand::ListAgents
    );
}

#[test]
fn rejects_malformed_window_and_session_targets() {
    // Non-sigil values are names now (resolved daemon-side); only
    // sigil-prefixed garbage stays a parse error.
    assert!(parse_command("new-window -t $x").is_err());
    assert!(parse_command("select-window -t @!").is_err());
    // Bare new-window is valid (targets the newest session); the
    // malformed-TARGET cases above are what this test guards.
}

#[test]
fn parses_capture_pane_with_and_without_history() {
    assert_eq!(
        parse_command("capture-pane -t %3 -p").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Id(PaneId(3)),
            start_line: None,
            end_line: None,
            escape: false
        }
    );
    assert_eq!(
        parse_command("capture-pane -t %3 -p -S 50 -E -1").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Id(PaneId(3)),
            start_line: Some(50),
            end_line: Some(-1),
            escape: false
        }
    );
    assert_eq!(
        parse_command("capture-pane -t %3 -p -S -20 -E -11").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Id(PaneId(3)),
            start_line: Some(-20),
            end_line: Some(-11),
            escape: false
        }
    );
    assert_eq!(
        parse_command("capture-pane -t %3 -p -e -S -20 -E -11").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Id(PaneId(3)),
            start_line: Some(-20),
            end_line: Some(-11),
            escape: true
        }
    );
}

#[test]
fn parses_split_window_with_flags_and_defaults() {
    // Default: new pane below the target (-v), 50 percent.
    assert_eq!(
        parse_command("split-window -t %0").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("split-window -t %0 -v").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        }
    );
    // -h: side by side; -p: the NEW pane's share.
    assert_eq!(
        parse_command("split-window -t %0 -h -p 25").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Vertical,
            percent: 25,
            before: false,
            start_dir: None,
        }
    );
    // -b: the new pane goes BEFORE the target (left/above).
    assert_eq!(
        parse_command("split-window -t %0 -h -b -p 30").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Vertical,
            percent: 30,
            before: true,
            start_dir: None,
        }
    );
}

#[test]
fn split_window_rejects_out_of_range_percent() {
    assert!(parse_command("split-window -t %0 -p 0").is_err());
    assert!(parse_command("split-window -t %0 -p 100").is_err());
}

/// tmux parity: `split-window`/`new-window` accept all three sigils —
/// the sigil picks the kind, and a bare value keeps the command's own
/// name form (pane titles for split-window, session names for
/// new-window).
#[test]
fn split_and_new_window_targets_classify_every_sigil() {
    assert_eq!(
        parse_command("split-window -t @1").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Window(WindowId(1)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("split-window -t $0").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Session(SessionId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("split-window -t build").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Name("build".to_string()),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t @2").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Window(WindowId(2))),
            name: None,
            start_dir: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t %1 -n logs").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Pane(PaneId(1))),
            name: Some("logs".into()),
            start_dir: None,
        }
    );
    // A malformed sigil value keeps the invalid-target error, the
    // rule the single-kind helpers set.
    assert!(parse_command("split-window -t @abc").is_err());
    assert!(parse_command("new-window -t %abc").is_err());
}

/// `-c` names the new pane's start directory on both commands, quoting
/// included — a path with spaces survives the whitespace split.
#[test]
fn split_window_and_new_window_parse_a_start_directory() {
    assert_eq!(
        parse_command("split-window -t %0 -c /tmp").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: Some("/tmp".into())
        }
    );
    assert_eq!(
        parse_command("new-window -t $0 -n logs -c '/tmp/my dir'").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("logs".into()),
            start_dir: Some("/tmp/my dir".into())
        }
    );
    // An explicitly empty -c is a client bug, same rule as an empty
    // name: only quoting can express it, and silently defaulting away
    // would hide it.
    assert!(parse_command("split-window -t %0 -c ''").is_err());
}

#[test]
fn parses_select_and_swap_pane() {
    assert_eq!(
        parse_command("select-pane -t %2").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Id(PaneId(2)),
            title: None,
        }
    );
    assert_eq!(
        parse_command("swap-pane -t %2 -s %5").unwrap(),
        MuxCommand::SwapPanes {
            target: Target::Id(PaneId(2)),
            source: Target::Id(PaneId(5))
        }
    );
}

#[test]
fn parses_select_pane_title_flag() {
    // Quoted title with spaces stays one value (the same grammar
    // new-session -s uses for names).
    assert_eq!(
        parse_command("select-pane -t %0 -T 'My build pane'").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Id(PaneId(0)),
            title: Some("My build pane".to_string()),
        }
    );
    // An explicitly empty -T is the CLEAR operation, not an error.
    assert_eq!(
        parse_command("select-pane -t %0 -T ''").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Id(PaneId(0)),
            title: Some(String::new()),
        }
    );
    // Unquoted single word, and no -T at all.
    assert_eq!(
        parse_command("select-pane -t %0 -T logs").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Id(PaneId(0)),
            title: Some("logs".to_string()),
        }
    );
    // -T with nothing following it is malformed, not "clear".
    assert!(parse_command("select-pane -t %0 -T").is_err());
}

#[test]
fn parses_pane_title_query() {
    assert_eq!(
        parse_command("pane-title -t %3").unwrap(),
        MuxCommand::PaneTitle {
            pane: Target::Id(PaneId(3))
        }
    );
    assert!(parse_command("pane-title").is_err());
}

#[test]
fn parses_pane_info_query() {
    assert_eq!(
        parse_command("pane-info -t %3").unwrap(),
        MuxCommand::PaneInfo {
            pane: Target::Id(PaneId(3))
        }
    );
    assert!(parse_command("pane-info").is_err());
}

#[test]
fn parses_pane_exited_replay() {
    assert_eq!(
        parse_command("pane-exited-replay").unwrap(),
        MuxCommand::PaneExitedReplay
    );
}

#[test]
fn parses_clear_history_query() {
    assert_eq!(
        parse_command("clear-history -t %3").unwrap(),
        MuxCommand::ClearHistory {
            pane: Target::Id(PaneId(3))
        }
    );
    assert!(parse_command("clear-history").is_err());
}

#[test]
fn parses_resize_pane_with_default_and_explicit_cells() {
    assert_eq!(
        parse_command("resize-pane -t %0 -R").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Relative {
                direction: ResizeDirection::Right,
                cells: 5
            }
        }
    );
    assert_eq!(
        parse_command("resize-pane -t %0 -U 12").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Relative {
                direction: ResizeDirection::Up,
                cells: 12
            }
        }
    );
    assert!(parse_command("resize-pane -t %0").is_err());
}

#[test]
fn parses_resize_pane_absolute_extents() {
    // The renderer-driven form par-term's gateway sends on drag-resize.
    assert_eq!(
        parse_command("resize-pane -t %0 -x 120 -y 40").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Absolute {
                cols: Some(120),
                rows: Some(40)
            }
        }
    );
    // Either axis alone is valid.
    assert_eq!(
        parse_command("resize-pane -t %0 -x 25").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Absolute {
                cols: Some(25),
                rows: None
            }
        }
    );
    assert_eq!(
        parse_command("resize-pane -t %0 -y 30").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Absolute {
                cols: None,
                rows: Some(30)
            }
        }
    );
}

#[test]
fn resize_pane_absolute_rejects_zero_and_mixed_forms() {
    assert!(parse_command("resize-pane -t %0 -x 0").is_err());
    assert!(parse_command("resize-pane -t %0 -y 0").is_err());
    assert!(parse_command("resize-pane -t %0 -x abc").is_err());
    assert!(
        parse_command("resize-pane -t %0 -x 120 -R").is_err(),
        "absolute and relative forms cannot combine"
    );
}

#[test]
fn parses_resize_pane_zoom_toggle() {
    assert_eq!(
        parse_command("resize-pane -t %0 -Z").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Zoom
        }
    );
    assert!(
        parse_command("resize-pane -t %0 -Z -x 80").is_err(),
        "the zoom form cannot combine with the absolute form"
    );
    assert!(
        parse_command("resize-pane -t %0 -Z -R").is_err(),
        "the zoom form cannot combine with the relative form"
    );
}

#[test]
fn parses_break_join_and_window_reorder() {
    assert_eq!(
        parse_command("break-pane -s %1").unwrap(),
        MuxCommand::BreakPane {
            source: Target::Id(PaneId(1)),
            name: None
        }
    );
    assert_eq!(
        parse_command("break-pane -s %1 -n 'my window'").unwrap(),
        MuxCommand::BreakPane {
            source: Target::Id(PaneId(1)),
            name: Some("my window".to_string())
        }
    );
    // join-pane defaults to below (Horizontal) at 50, split-window's
    // arrangement rule; -h/-p override.
    assert_eq!(
        parse_command("join-pane -s %1 -t %0").unwrap(),
        MuxCommand::JoinPane {
            source: Target::Id(PaneId(1)),
            target: Target::Id(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50
        }
    );
    assert_eq!(
        parse_command("join-pane -s %1 -t %0 -h -p 30").unwrap(),
        MuxCommand::JoinPane {
            source: Target::Id(PaneId(1)),
            target: Target::Id(PaneId(0)),
            direction: SplitDirection::Vertical,
            percent: 30
        }
    );
    assert!(
        parse_command("join-pane -s %1 -t %0 -p 0").is_err(),
        "the share percentage stays in 1-99"
    );
    // QA-187: the shared geometry parser keeps split-window's error
    // text on join-pane too.
    assert_eq!(
        parse_command("join-pane -s %1 -t %0 -p 100").unwrap_err(),
        "percentage must be 1-99: 100"
    );
    assert_eq!(
        parse_command("join-pane -s %1 -t %0 -p x").unwrap_err(),
        "invalid percentage: x"
    );
    assert_eq!(
        parse_command("move-window -s @2 -t 0").unwrap(),
        MuxCommand::MoveWindow {
            source: Target::Id(WindowId(2)),
            index: 0
        }
    );
    assert!(
        parse_command("move-window -s @2 -t first").is_err(),
        "move-window's -t is a position, not a target"
    );
    assert_eq!(
        parse_command("swap-window -s @0 -t @2").unwrap(),
        MuxCommand::SwapWindows {
            source: Target::Id(WindowId(0)),
            target: Target::Id(WindowId(2))
        }
    );
    // respawn-pane: -k and the trailing command both ride along.
    assert_eq!(
        parse_command("respawn-pane -t %0").unwrap(),
        MuxCommand::RespawnPane {
            pane: Target::Id(PaneId(0)),
            kill: false,
            start_dir: None,
            command: None
        }
    );
    assert_eq!(
        parse_command("respawn-pane -t %0 -k -c /tmp sleep 60").unwrap(),
        MuxCommand::RespawnPane {
            pane: Target::Id(PaneId(0)),
            kill: true,
            start_dir: Some("/tmp".to_string()),
            command: Some("sleep 60".to_string())
        }
    );
    assert_eq!(
        parse_command("respawn-pane -t %0 -k top").unwrap(),
        MuxCommand::RespawnPane {
            pane: Target::Id(PaneId(0)),
            kill: true,
            start_dir: None,
            command: Some("top".to_string())
        }
    );
}

/// SEC-126: `respawn-pane` reads flags only up to the first non-flag
/// word (or `--`), and passes the rest of the line through verbatim —
/// a `-k`/`-c` inside the command belongs to the command.
#[test]
fn respawn_pane_parses_only_leading_flags() {
    type Expect = Result<
        (
            Target<PaneId>,
            bool,
            Option<&'static str>,
            Option<&'static str>,
        ),
        (),
    >;
    let id = |n: u32| Target::Id(PaneId(n));
    let cases: Vec<(&str, Expect)> = vec![
        (r"respawn-pane -t %0", Ok((id(0), false, None, None))),
        (
            r"respawn-pane -t %0 -k -c /tmp sleep 60",
            Ok((id(0), true, Some("/tmp"), Some("sleep 60"))),
        ),
        (
            r"respawn-pane -t %0 -k top",
            Ok((id(0), true, None, Some("top"))),
        ),
        (
            r"respawn-pane -t %0 sh -c 'echo X; sort -k 1 /dev/null; sleep 600'",
            Ok((
                id(0),
                false,
                None,
                Some(r"sh -c 'echo X; sort -k 1 /dev/null; sleep 600'"),
            )),
        ),
        (
            r"respawn-pane -t %0 sh -c 'echo hi'",
            Ok((id(0), false, None, Some(r"sh -c 'echo hi'"))),
        ),
        (
            r"respawn-pane -c /tmp -t %0 top",
            Ok((id(0), false, Some("/tmp"), Some("top"))),
        ),
        (
            r"respawn-pane -t %0 -c '/a b' sleep 5",
            Ok((id(0), false, Some("/a b"), Some("sleep 5"))),
        ),
        (
            r#"respawn-pane -t %0 -c "/a b" sleep 5"#,
            Ok((id(0), false, Some("/a b"), Some("sleep 5"))),
        ),
        (
            r"respawn-pane -t %0 printf '%s\n'   'a    b'",
            Ok((id(0), false, None, Some(r"printf '%s\n'   'a    b'"))),
        ),
        (
            r"respawn-pane -t %0 -- -weird --flag",
            Ok((id(0), false, None, Some("-weird --flag"))),
        ),
        (
            r"respawn-pane -t %0 -k -- top -k",
            Ok((id(0), true, None, Some("top -k"))),
        ),
        (r"respawn-pane -k -t %0", Ok((id(0), true, None, None))),
        (r"respawn-pane -t %0 --", Ok((id(0), false, None, None))),
        (
            "respawn-pane   -t   %0    -k    top  ",
            Ok((id(0), true, None, Some("top"))),
        ),
        (
            r"respawn-pane -t 'my pane' top",
            Ok((
                Target::Name("my pane".to_string()),
                false,
                None,
                Some("top"),
            )),
        ),
        (
            r"respawn-pane -t %0 echo -k",
            Ok((id(0), false, None, Some("echo -k"))),
        ),
        (
            r"respawn-pane -t %0 echo -c /etc",
            Ok((id(0), false, None, Some("echo -c /etc"))),
        ),
        (r"respawn-pane -t %0 -x foo", Err(())),
        (r"respawn-pane -t %0 -t %1", Err(())),
        (r"respawn-pane -t", Err(())),
        (r"respawn-pane -c /tmp top", Err(())),
        (r"respawn-pane -t %0 -c ''", Err(())),
        (r"respawn-pane -t %0 -kt %1", Err(())),
        (r"respawn-pane -t '' top", Err(())),
        // An unquoted value keeps its backslashes (a Windows path, a
        // name), as `Args::quoted_flag` does for every other command.
        (
            r"respawn-pane -t %0 -c C:\Users\me top",
            Ok((id(0), false, Some(r"C:\Users\me"), Some("top"))),
        ),
        (
            r"respawn-pane -t a\b top",
            Ok((Target::Name(r"a\b".to_string()), false, None, Some("top"))),
        ),
        // Not in the plan's table: a leading-whitespace line and a
        // non-ASCII command must slice on char boundaries.
        (
            "  respawn-pane -t %0 echo héllo  wörld",
            Ok((id(0), false, None, Some("echo héllo  wörld"))),
        ),
    ];
    for (line, expect) in cases {
        let got = parse_command(line).map(|cmd| match cmd {
            MuxCommand::RespawnPane {
                pane,
                kill,
                start_dir,
                command,
            } => (pane, kill, start_dir, command),
            other => panic!("{line}: parsed as {other:?}"),
        });
        match expect {
            Ok((pane, kill, dir, command)) => {
                let want = (
                    pane,
                    kill,
                    dir.map(str::to_string),
                    command.map(str::to_string),
                );
                assert_eq!(got, Ok(want), "{line}");
            }
            Err(()) => assert!(got.is_err(), "{line}: expected an error, got {got:?}"),
        }
    }
}

#[test]
fn parses_refresh_client_with_and_without_a_size_report() {
    assert_eq!(
        parse_command("refresh-client -t %0").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: None,
            cell_pixels: None,
            chrome: None
        }
    );
    assert_eq!(
        parse_command("refresh-client -t %0 -C 120x40").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((120, 40)),
            cell_pixels: None,
            chrome: None
        }
    );
    // The cell-pixel report rides along or stands alone: a client
    // re-reporting font metrics without a grid change sends only -p.
    assert_eq!(
        parse_command("refresh-client -t %0 -C 120x40 -p 10x20").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((120, 40)),
            cell_pixels: Some((10, 20)),
            chrome: None
        }
    );
    assert_eq!(
        parse_command("refresh-client -t %0 -p 9x17").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: None,
            cell_pixels: Some((9, 17)),
            chrome: None
        }
    );
}

/// Card 01a11c5b: `-I` declares the client's per-pane chrome; absent, the
/// report declares none (an older client), and a malformed value rejects.
/// The `chrome` feature token advertises the flag on refresh-client's row.
#[test]
fn parses_refresh_client_chrome_declaration() {
    assert_eq!(
        parse_command("refresh-client -t %0 -C 80x22 -I border=1,gap=0,gutter=0").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((80, 22)),
            cell_pixels: None,
            chrome: Some(crate::mux::layout::PaneChrome {
                border: true,
                gap: 0,
                gutter: false,
            }),
        }
    );
    assert!(parse_command("refresh-client -t %0 -C 80x22 -I ring=1").is_err());
    assert!(parse_command("refresh-client -t %0 -C 80x22 -I border=yes").is_err());
    let body = list_commands_body();
    assert!(
        body.lines()
            .any(|l| l == "refresh-client cell-pixels chrome"),
        "{body}"
    );
}

/// The attach handshake's shape: `refresh-client -C WxH -p WxH` with no
/// `-t`, sent before the client has resolved a pane target. It parses
/// to the `None` pane form — a pure size report, never a replay.
#[test]
fn parses_target_less_refresh_client_as_a_pure_size_report() {
    assert_eq!(
        parse_command("refresh-client -C 120x40 -p 10x20").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: Some((120, 40)),
            cell_pixels: Some((10, 20)),
            chrome: None
        }
    );
    // Either measurement can appear alone in the target-less form.
    assert_eq!(
        parse_command("refresh-client -C 100x30").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: Some((100, 30)),
            cell_pixels: None,
            chrome: None
        }
    );
    assert_eq!(
        parse_command("refresh-client -p 10x20").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: None,
            cell_pixels: Some((10, 20)),
            chrome: None
        }
    );
    // Caps and malformed values reject exactly as the -t form does.
    assert!(parse_command("refresh-client -C 1001x40").is_err());
    assert!(parse_command("refresh-client -p 10x513").is_err());
    assert!(parse_command("refresh-client -C ax40").is_err());
    // A bare `refresh-client` with neither measurement nor target is
    // a no-op size report: it parses (dispatch answers an empty ok).
    assert_eq!(
        parse_command("refresh-client").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: None,
            cell_pixels: None,
            chrome: None
        }
    );
    // And an invalid pane target still errors in the -t form.
    assert!(parse_command("refresh-client -t %abc").is_err());
}

#[test]
fn refresh_client_size_report_rejects_malformed_values() {
    assert!(parse_command("refresh-client -t %0 -C 120").is_err());
    assert!(parse_command("refresh-client -t %0 -C 0x40").is_err());
    assert!(parse_command("refresh-client -t %0 -C 120x0").is_err());
    assert!(parse_command("refresh-client -t %0 -C ax40").is_err());
    assert!(parse_command("refresh-client -t %0 -p 0x20").is_err());
    assert!(parse_command("refresh-client -t %0 -p 10").is_err());
}

/// QA-182: grid and cell-pixel reports are capped, so a client cannot
/// make the daemon allocate an unbounded grid.
#[test]
fn refresh_client_rejects_sizes_over_the_caps() {
    for line in [
        "refresh-client -t %0 -C 1001x40",
        "refresh-client -t %0 -C 120x501",
        "refresh-client -t %0 -C 65535x65535",
        "refresh-client -t %0 -p 513x20",
        "refresh-client -t %0 -p 10x513",
    ] {
        let err = parse_command(line).expect_err(line);
        assert!(err.contains("exceeds"), "{line}: {err}");
    }
    assert_eq!(
        parse_command("refresh-client -t %0 -C 1000x500 -p 512x512").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((MAX_CLIENT_COLS, MAX_CLIENT_ROWS)),
            cell_pixels: Some((MAX_CELL_PIXELS, MAX_CELL_PIXELS)),
            chrome: None
        }
    );
}

#[test]
fn parses_set_buffer_show_buffer_and_paste_buffer() {
    assert_eq!(
        parse_command("set-buffer hello world").unwrap(),
        MuxCommand::SetBuffer {
            content: "hello world".into()
        }
    );
    assert_eq!(
        parse_command("show-buffer").unwrap(),
        MuxCommand::ShowBuffer
    );
    assert_eq!(
        parse_command("paste-buffer -t %2").unwrap(),
        MuxCommand::PasteBuffer {
            pane: Target::Id(PaneId(2))
        }
    );
}

#[test]
fn parses_set_client_colors() {
    assert_eq!(
        parse_command("set-client-colors -f c0c0c0 -b 1e1e2e").unwrap(),
        MuxCommand::SetClientColors {
            fg: Some((0xc0, 0xc0, 0xc0)),
            bg: Some((0x1e, 0x1e, 0x2e)),
        }
    );
    // Each flag is independent; uppercase hex is accepted.
    assert_eq!(
        parse_command("set-client-colors -b AABBCC").unwrap(),
        MuxCommand::SetClientColors {
            fg: None,
            bg: Some((0xaa, 0xbb, 0xcc)),
        }
    );
    assert!(parse_command("set-client-colors").is_err());
    assert!(parse_command("set-client-colors -f 12345").is_err());
    assert!(parse_command("set-client-colors -f zz1234").is_err());
    assert!(parse_command("set-client-colors -b #1e1e2e").is_err());
}

#[test]
fn rejects_a_set_buffer_with_no_content() {
    assert!(parse_command("set-buffer").is_err());
}

#[test]
fn mutates_marks_exactly_the_structural_commands() {
    // The persistence rule on the type — the pre-decomposition
    // `mutated = true` set, verbatim. Structural commands save the state
    // file on success; content and read-only commands never do.
    let structural = [
        MuxCommand::NewSession {
            name: None,
            env: vec![],
            workspace: None,
        },
        MuxCommand::KillPane {
            pane: Target::Id(PaneId(0)),
        },
        MuxCommand::ClearHistory {
            pane: Target::Id(PaneId(0)),
        },
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((80, 24)),
            cell_pixels: None,
            chrome: None,
        },
        MuxCommand::NewWindow {
            target: None,
            name: None,
            start_dir: None,
        },
        MuxCommand::SelectWindow {
            window: Target::Id(WindowId(0)),
        },
        MuxCommand::KillWindow {
            window: Target::Id(WindowId(0)),
        },
        MuxCommand::RenameWindow {
            window: Target::Id(WindowId(0)),
            name: String::new(),
        },
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
        },
        MuxCommand::SelectPane {
            pane: Target::Id(PaneId(0)),
            title: None,
        },
        MuxCommand::ResizePane {
            pane: Target::Id(PaneId(0)),
            adjustment: ResizeAdjustment::Relative {
                direction: ResizeDirection::Right,
                cells: 5,
            },
        },
        MuxCommand::SwapPanes {
            target: Target::Id(PaneId(0)),
            source: Target::Id(PaneId(1)),
        },
        MuxCommand::SetBuffer {
            content: String::new(),
        },
    ];
    for command in &structural {
        assert!(
            command.mutates(),
            "{command:?} is structural and must mutate"
        );
    }
    let non_mutating = [
        MuxCommand::ListPanes { window: None },
        MuxCommand::ListAgents,
        MuxCommand::SendKeys {
            pane: Target::Id(PaneId(0)),
            keys: SendKeysPayload::default(),
        },
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: None,
            cell_pixels: None,
            chrome: None,
        },
        MuxCommand::ListWindows { session: None },
        MuxCommand::ListSessions { workspace: None },
        MuxCommand::ListWorkspaces,
        MuxCommand::CapturePane {
            pane: Target::Id(PaneId(0)),
            start_line: None,
            end_line: None,
            escape: false,
        },
        MuxCommand::ShowBuffer,
        MuxCommand::PasteBuffer {
            pane: Target::Id(PaneId(0)),
        },
    ];
    for command in &non_mutating {
        assert!(
            !command.mutates(),
            "{command:?} is content or read-only and must not mutate"
        );
    }
}

#[test]
fn parse_line_routes_hook_reports_and_control_commands() {
    assert_eq!(
        parse_line(r#" {"id":1,"method":"pane.report_agent"}"#).unwrap(),
        Line::Hook(r#" {"id":1,"method":"pane.report_agent"}"#.to_string())
    );
    assert_eq!(
        parse_line("list-panes").unwrap(),
        Line::Control(MuxCommand::ListPanes { window: None })
    );
    let Err(err) = parse_line("frobnicate") else {
        panic!("an unknown command is a parse error");
    };
    assert_eq!(err, "unknown command: frobnicate");
}

// --- Workspaces ---

#[test]
fn parses_workspace_commands() {
    assert_eq!(
        parse_command("new-workspace -n dev").unwrap(),
        MuxCommand::NewWorkspace {
            name: Some("dev".to_string())
        }
    );
    assert_eq!(
        parse_command("new-workspace").unwrap(),
        MuxCommand::NewWorkspace { name: None }
    );
    assert_eq!(
        parse_command("list-workspaces").unwrap(),
        MuxCommand::ListWorkspaces
    );
    assert_eq!(
        parse_command("select-workspace -t +2").unwrap(),
        MuxCommand::SelectWorkspace {
            workspace: Target::Id(WorkspaceId(2))
        }
    );
    assert_eq!(
        parse_command("select-workspace -t dev").unwrap(),
        MuxCommand::SelectWorkspace {
            workspace: Target::Name("dev".to_string())
        }
    );
    assert_eq!(
        parse_command("rename-workspace -t +1 prod").unwrap(),
        MuxCommand::RenameWorkspace {
            workspace: Target::Id(WorkspaceId(1)),
            name: "prod".to_string()
        }
    );
    assert_eq!(
        parse_command("kill-workspace -t dev").unwrap(),
        MuxCommand::KillWorkspace {
            workspace: Target::Name("dev".to_string())
        }
    );
    // select-workspace requires -t; rename-workspace requires a name.
    assert!(parse_command("select-workspace").is_err());
    assert!(parse_command("rename-workspace -t +1").is_err());
    assert!(parse_command("rename-workspace -t +1 a b").is_err());
}

#[test]
fn parses_list_sessions_with_and_without_a_workspace_filter() {
    assert_eq!(
        parse_command("list-sessions").unwrap(),
        MuxCommand::ListSessions { workspace: None }
    );
    assert_eq!(
        parse_command("list-sessions -t +0").unwrap(),
        MuxCommand::ListSessions {
            workspace: Some(Target::Id(WorkspaceId(0)))
        }
    );
    assert_eq!(
        parse_command("list-sessions -t work").unwrap(),
        MuxCommand::ListSessions {
            workspace: Some(Target::Name("work".to_string()))
        }
    );
}

#[test]
fn parses_new_session_with_a_workspace_target() {
    assert_eq!(
        parse_command("new-session -t dev -s build").unwrap(),
        MuxCommand::NewSession {
            name: Some("build".to_string()),
            env: vec![],
            workspace: Some(Target::Name("dev".to_string()))
        }
    );
    assert_eq!(
        parse_command("new-session").unwrap(),
        MuxCommand::NewSession {
            name: None,
            env: vec![],
            workspace: None
        }
    );
}

#[test]
fn every_workspace_command_row_appears_in_list_commands() {
    let body = list_commands_body();
    for name in [
        "new-workspace",
        "list-workspaces",
        "select-workspace",
        "rename-workspace",
        "kill-workspace",
    ] {
        assert!(
            body.lines().any(|line| line.starts_with(name)),
            "{name} must be discoverable via list-commands"
        );
    }
    // The workspace-aware commands carry the feature token.
    assert!(body
        .lines()
        .any(|line| line.starts_with("new-session workspace")));
    assert!(body
        .lines()
        .any(|line| line.starts_with("list-sessions workspace")));
}
