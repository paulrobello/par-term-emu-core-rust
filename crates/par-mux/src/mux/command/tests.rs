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
            command: None,
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
            restore_start_command: false,
            name: Some("work".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
}

#[test]
fn parses_new_session_env_assignments_in_order_with_quoting() {
    let cmd = parse_command("new-session -s work -e A=1 -e 'B=two words' -e C=x=y -e D=").unwrap();
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("work".into()),
            env: vec![
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "x=y".into()),
                ("D".into(), String::new()),
            ],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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
            restore_start_command: false,
            name: None,
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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
                restore_start_command: false,
                name: Some("Par Mux Test".into()),
                env: vec![],
                workspace: None,
                window_name: None,
                start_dir: None,
                cols: None,
                rows: None,
                command: None,
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
            restore_start_command: false,
            name: Some("Paul's box".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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
            restore_start_command: false,
            name: Some("work".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-session   -s   work  ").expect("parses"),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("work".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-session -s").expect("parses"),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: None,
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
    // An unquoted value keeps the flat scan's verbatim bytes: a bare
    // backslash is part of the name, not an escape.
    assert_eq!(
        parse_command(r"new-session -s a\b").expect("parses"),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some(r"a\b".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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
            restore_start_command: false,
            name: Some("-n".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t $0 -n '-s'").expect("parses"),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("-s".into()),
            start_dir: None,
            command: None,
        }
    );
    // The name flag need not come first.
    assert_eq!(
        parse_command("new-window -n 'two words' -t $0").expect("parses"),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("two words".into()),
            start_dir: None,
            command: None,
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
            command: None,
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
    // A navigation key does follow the kitty flags — with the spec's
    // functional-key table form (`CSI 15 ~`, not the PUA `CSI 57380u`
    // spelling; 57376+ are F13+ per spec, which apps like claude ignore).
    assert_eq!(
        encoded_on("send-keys -t %3 F5", &term),
        b"\x1b[15~".to_vec()
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
        MuxCommand::ListPanes {
            target: None,
            format: None,
        }
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

/// QA-219's original sin was the SILENT drop; R4 gives the trailing
/// command its tmux meaning instead, so these all parse into `command`.
#[test]
fn new_window_accepts_a_trailing_command() {
    for (line, command) in [
        ("new-window sleep 5", "sleep 5"),
        ("new-window -n w sleep 5", "sleep 5"),
        ("new-window -t $0 -- top", "top"),
    ] {
        match parse_command(line).expect(line) {
            MuxCommand::NewWindow { command: got, .. } => {
                assert_eq!(got.as_deref(), Some(command), "{line}");
            }
            other => panic!("{line}: not a new-window: {other:?}"),
        }
    }
}

#[test]
fn new_session_accepts_a_trailing_command() {
    match parse_command("new-session -s a top").expect("positional") {
        MuxCommand::NewSession { command, .. } => {
            assert_eq!(command.as_deref(), Some("top"));
        }
        other => panic!("not a new-session: {other:?}"),
    }
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
/// row; the documented wire grammar is `[a-z0-9-]+` (numerals since
/// ENH-043's `client-snapshot v1`), and anything else
/// would hand clients a token they are told to treat as well-formed.
#[test]
fn every_command_feature_token_matches_the_wire_grammar() {
    for (name, _, features) in COMMANDS {
        for token in *features {
            assert!(
                !token.is_empty()
                    && token
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
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
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t $0").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: None,
            start_dir: None,
            command: None,
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
            command: None,
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
        MuxCommand::ListWindows {
            session: None,
            all: false,
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-sessions").unwrap(),
        MuxCommand::ListSessions {
            workspace: None,
            format: None,
        }
    );
}

#[test]
fn parses_targeted_list_windows_and_list_panes() {
    assert_eq!(
        parse_command("list-windows -t $2").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Id(SessionId(2))),
            all: false,
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-windows -t work").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Name("work".to_string())),
            all: false,
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-panes -t @7").unwrap(),
        MuxCommand::ListPanes {
            target: Some(AnyTarget::Window(WindowId(7))),
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-panes -t editor").unwrap(),
        MuxCommand::ListPanes {
            target: Some(AnyTarget::Name("editor".to_string())),
            format: None,
        }
    );
    // A `-F` is the format (R6), not a positional pane name.
    assert_eq!(
        parse_command("list-windows -t work -F '#{window_name}'").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Name("work".to_string())),
            all: false,
            format: Some("#{window_name}".to_string()),
        }
    );
    // A stray positional is rejected.
    assert!(parse_command("list-windows stray").is_err());
    assert!(parse_command("list-panes stray").is_err());
}

/// `list-windows -a` (ARC-125): the all-sessions form; a `-F` is the
/// format (R6), and `-a` with `-t` is rejected.
#[test]
fn parses_list_windows_all() {
    assert_eq!(
        parse_command("list-windows -a").unwrap(),
        MuxCommand::ListWindows {
            session: None,
            all: true,
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-windows -a -F '#{window_name}'").unwrap(),
        MuxCommand::ListWindows {
            session: None,
            all: true,
            format: Some("#{window_name}".to_string()),
        }
    );
    assert!(parse_command("list-windows -a -t $0").is_err());
}

#[test]
fn parses_list_agents() {
    assert_eq!(
        parse_command("list-agents").unwrap(),
        MuxCommand::ListAgents
    );
}

#[test]
fn parses_events_with_and_without_since() {
    assert_eq!(
        parse_command("events").unwrap(),
        MuxCommand::Events { since: None }
    );
    assert_eq!(
        parse_command("events --since 42").unwrap(),
        MuxCommand::Events { since: Some(42) }
    );
    assert!(
        parse_command("events --since nope").is_err(),
        "a non-numeric watermark is a parse error"
    );
    assert!(
        parse_command("events stray").is_err(),
        "positionals are rejected"
    );
}

#[test]
fn parses_client_snapshot() {
    let cmd = parse_command("client-snapshot -t @3").unwrap();
    assert_eq!(
        cmd,
        MuxCommand::ClientSnapshot {
            window: Target::Id(WindowId(3))
        }
    );
    assert!(!cmd.mutates(), "client-snapshot is a query");
    let missing = parse_command("client-snapshot").unwrap_err();
    assert!(missing.contains("requires -t"), "{missing}");
    assert!(parse_command("client-snapshot -t @3 stray").is_err());
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
            command: None,
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
            command: None,
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
            command: None,
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
            command: None,
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
            command: None,
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
            command: None,
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
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t @2").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Window(WindowId(2))),
            name: None,
            start_dir: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t %1 -n logs").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Pane(PaneId(1))),
            name: Some("logs".into()),
            start_dir: None,
            command: None,
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
            start_dir: Some("/tmp".into()),
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-window -t $0 -n logs -c '/tmp/my dir'").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: Some("logs".into()),
            start_dir: Some("/tmp/my dir".into()),
            command: None,
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
            pane: Target::Id(PaneId(3)),
            mirror: false,
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
            chrome: None,
            graphics: false,
        }
    );
    assert_eq!(
        parse_command("refresh-client -t %0 -C 120x40").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: Some((120, 40)),
            cell_pixels: None,
            chrome: None,
            graphics: false,
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
            chrome: None,
            graphics: false,
        }
    );
    assert_eq!(
        parse_command("refresh-client -t %0 -p 9x17").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: None,
            cell_pixels: Some((9, 17)),
            chrome: None,
            graphics: false,
        }
    );
    // `-G`: the seed's graphics section — the streaming mirror's flag.
    assert_eq!(
        parse_command("refresh-client -t %0 -G").unwrap(),
        MuxCommand::RefreshClient {
            pane: Some(Target::Id(PaneId(0))),
            size: None,
            cell_pixels: None,
            chrome: None,
            graphics: true,
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
            graphics: false,
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
            chrome: None,
            graphics: false,
        }
    );
    // Either measurement can appear alone in the target-less form.
    assert_eq!(
        parse_command("refresh-client -C 100x30").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: Some((100, 30)),
            cell_pixels: None,
            chrome: None,
            graphics: false,
        }
    );
    assert_eq!(
        parse_command("refresh-client -p 10x20").unwrap(),
        MuxCommand::RefreshClient {
            pane: None,
            size: None,
            cell_pixels: Some((10, 20)),
            chrome: None,
            graphics: false,
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
            chrome: None,
            graphics: false,
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
            chrome: None,
            graphics: false,
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
            restore_start_command: false,
            name: None,
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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
            graphics: false,
        },
        MuxCommand::NewWindow {
            target: None,
            name: None,
            start_dir: None,
            command: None,
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
            command: None,
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
        MuxCommand::ListPanes {
            target: None,
            format: None,
        },
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
            graphics: false,
        },
        MuxCommand::ListWindows {
            session: None,
            all: false,
            format: None,
        },
        MuxCommand::ListSessions {
            workspace: None,
            format: None,
        },
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
        Line::Control(MuxCommand::ListPanes {
            target: None,
            format: None,
        })
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
            workspace: Target::Name("dev".to_string()),
            group: false,
        }
    );
    assert_eq!(
        parse_command("kill-workspace -t +2 -g").unwrap(),
        MuxCommand::KillWorkspace {
            workspace: Target::Id(WorkspaceId(2)),
            group: true,
        }
    );
    // select-workspace requires -t; rename-workspace requires a name.
    assert!(parse_command("select-workspace").is_err());
    assert!(parse_command("rename-workspace -t +1").is_err());
    assert!(parse_command("rename-workspace -t +1 a b").is_err());
}

#[test]
fn parses_set_workspace_respawn_values_and_query() {
    for (word, change) in [
        ("on", RespawnChange::On),
        ("off", RespawnChange::Off),
        ("toggle", RespawnChange::Toggle),
    ] {
        let command = parse_command(&format!("set-workspace-respawn -t +1 {word}")).unwrap();
        assert_eq!(
            command,
            MuxCommand::SetWorkspaceRespawn {
                workspace: Target::Id(WorkspaceId(1)),
                change: Some(change),
            }
        );
        assert!(command.mutates(), "{word} saves");
    }
    let query = parse_command("set-workspace-respawn -t main").unwrap();
    assert_eq!(
        query,
        MuxCommand::SetWorkspaceRespawn {
            workspace: Target::Name("main".to_string()),
            change: None,
        }
    );
    assert!(!query.mutates(), "the query form reads only");
    assert!(
        parse_command("set-workspace-respawn on").is_err(),
        "-t required"
    );
    assert!(parse_command("set-workspace-respawn -t +1 maybe").is_err());
    assert!(parse_command("set-workspace-respawn -t +1 on off").is_err());
}

#[test]
fn parses_set_exit_action_values_and_query() {
    for (word, expected) in [
        ("remove", crate::mux::config::ExitAction::Remove),
        ("hold", crate::mux::config::ExitAction::Hold),
        ("shell", crate::mux::config::ExitAction::Shell),
    ] {
        let command = parse_command(&format!("set-exit-action -t +1 {word}")).unwrap();
        assert_eq!(
            command,
            MuxCommand::SetExitAction {
                workspace: Target::Id(WorkspaceId(1)),
                action: Some(expected),
            }
        );
        assert!(command.mutates(), "{word} saves");
    }
    let query = parse_command("set-exit-action -t main").unwrap();
    assert_eq!(
        query,
        MuxCommand::SetExitAction {
            workspace: Target::Name("main".to_string()),
            action: None,
        }
    );
    assert!(!query.mutates(), "the query form reads only");
    assert!(parse_command("set-exit-action").is_err(), "-t required");
    assert!(parse_command("set-exit-action -t +1 maybe").is_err());
    assert!(parse_command("set-exit-action -t +1 hold shell").is_err());
}

#[test]
fn parses_list_sessions_with_and_without_a_workspace_filter() {
    assert_eq!(
        parse_command("list-sessions").unwrap(),
        MuxCommand::ListSessions {
            workspace: None,
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-sessions -t +0").unwrap(),
        MuxCommand::ListSessions {
            workspace: Some(Target::Id(WorkspaceId(0))),
            format: None,
        }
    );
    assert_eq!(
        parse_command("list-sessions -t work").unwrap(),
        MuxCommand::ListSessions {
            workspace: Some(Target::Name("work".to_string())),
            format: None,
        }
    );
}

#[test]
fn parses_new_session_with_a_workspace_target() {
    assert_eq!(
        parse_command("new-session -t dev -s build").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("build".to_string()),
            env: vec![],
            workspace: Some(Target::Name("dev".to_string())),
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("new-session").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: None,
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: None,
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

#[test]
fn parses_new_session_window_name_start_dir_and_size() {
    let cmd = parse_command("new-session -s work -n main -c /tmp -x 120 -y 40 -t default")
        .expect("parses");
    assert_eq!(
        cmd,
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("work".into()),
            env: vec![],
            workspace: Some(Target::Name("default".into())),
            window_name: Some("main".into()),
            start_dir: Some("/tmp".into()),
            cols: Some(120),
            rows: Some(40),
            command: None,
        }
    );
}

#[test]
fn parses_new_session_trailing_start_command() {
    let cmd = parse_command("new-session -s logs -- tail -f /var/log/system.log").expect("parses");
    match cmd {
        MuxCommand::NewSession { command, .. } => {
            assert_eq!(command.as_deref(), Some("tail -f /var/log/system.log"));
        }
        other => panic!("not a new-session: {other:?}"),
    }
    // Without --: the first non-flag word starts the command, tmux form.
    let cmd = parse_command("new-session -s logs tail -f /var/log/x").expect("parses");
    match cmd {
        MuxCommand::NewSession { command, .. } => {
            assert_eq!(command.as_deref(), Some("tail -f /var/log/x"));
        }
        other => panic!("not a new-session: {other:?}"),
    }
}

#[test]
fn new_session_rejects_unsupported_flags_loudly() {
    assert!(
        parse_command("new-session -s x -F '#{session_id}'").is_err(),
        "-F is not silently dropped"
    );
    assert!(
        parse_command("new-session -s x -f").is_err(),
        "-f is not silently dropped"
    );
}

#[test]
fn parses_new_window_and_split_window_start_commands() {
    match parse_command("new-window -n logs -- tail -f x").expect("parses") {
        MuxCommand::NewWindow { command, .. } => {
            assert_eq!(command.as_deref(), Some("tail -f x"));
        }
        other => panic!("not a new-window: {other:?}"),
    }
    match parse_command("split-window -t %0 -h -- watch ps").expect("parses") {
        MuxCommand::SplitWindow { command, .. } => {
            assert_eq!(command.as_deref(), Some("watch ps"));
        }
        other => panic!("not a split-window: {other:?}"),
    }
    // The legacy rejection is gone: a start command is now meaningful.
    match parse_command("split-window -t %0 sleep 5").expect("parses") {
        MuxCommand::SplitWindow { command, .. } => {
            assert_eq!(command.as_deref(), Some("sleep 5"));
        }
        other => panic!("not a split-window: {other:?}"),
    }
}

#[test]
fn send_keys_enter_flag_appends_the_enter_byte() {
    assert_eq!(
        encoded("send-keys -t %3 -l 'make check' --enter"),
        b"make check\r".to_vec()
    );
    assert_eq!(
        encoded("send-keys -t %3 --enter -l 'make check'"),
        b"make check\r".to_vec()
    );
    // A mid-text --enter stays literal.
    assert_eq!(
        encoded("send-keys -t %3 -l 'run --enter now'"),
        b"run --enter now".to_vec()
    );
    // Without the flag, no terminator (the standing contract).
    assert_eq!(
        encoded("send-keys -t %3 -l 'make check'"),
        b"make check".to_vec()
    );
    // The flag alone is not a payload.
    assert!(parse_command("send-keys -t %3 --enter").is_err());
}

#[test]
fn list_panes_accepts_every_target_kind() {
    match parse_command("list-panes -t $0").expect("parses") {
        MuxCommand::ListPanes {
            target: Some(AnyTarget::Session(_)),
            format: None,
        } => {}
        other => panic!("session target: {other:?}"),
    }
    match parse_command("list-panes -t @1").expect("parses") {
        MuxCommand::ListPanes {
            target: Some(AnyTarget::Window(_)),
            ..
        } => {}
        other => panic!("window target: {other:?}"),
    }
    match parse_command("list-panes -t %2").expect("parses") {
        MuxCommand::ListPanes {
            target: Some(AnyTarget::Pane(_)),
            ..
        } => {}
        other => panic!("pane target: {other:?}"),
    }
}

#[test]
fn list_commands_parse_the_format_flag() {
    for (command, var) in [
        (
            "list-sessions -F '#{session_id} #{session_name}'",
            "session_id",
        ),
        ("list-windows -F '#{window_id}'", "window_id"),
        ("list-panes -F '#{pane_id}'", "pane_id"),
    ] {
        match parse_command(command).expect("parses") {
            MuxCommand::ListSessions {
                format: Some(f), ..
            }
            | MuxCommand::ListWindows {
                format: Some(f), ..
            }
            | MuxCommand::ListPanes {
                format: Some(f), ..
            } => {
                assert!(f.contains(var), "{command}: {f}");
            }
            other => panic!("{command} did not carry a format: {other:?}"),
        }
    }
}

#[test]
fn parses_has_session() {
    match parse_command("has-session -t work").expect("parses") {
        MuxCommand::HasSession { session } => {
            assert_eq!(session, Target::Name("work".into()));
        }
        other => panic!("not a has-session: {other:?}"),
    }
    assert!(parse_command("has-session").is_err(), "-t is required");
}

#[test]
fn parses_show_options_and_set_option() {
    assert_eq!(
        parse_command("show-options").expect("parses"),
        MuxCommand::ShowOptions
    );
    match parse_command("set-option exit-empty off").expect("parses") {
        MuxCommand::SetOption { key, value } => {
            assert_eq!((key.as_str(), value.as_str()), ("exit-empty", "off"));
        }
        other => panic!("not a set-option: {other:?}"),
    }
    assert!(
        parse_command("set-option exit-empty").is_err(),
        "two words required"
    );
    assert!(
        parse_command("set-option exit-empty off extra").is_err(),
        "exactly two words"
    );
}

#[test]
fn new_verbs_appear_in_list_commands() {
    let body = list_commands_body();
    for name in ["has-session", "show-options", "set-option"] {
        assert!(
            body.lines().any(|line| line.starts_with(name)),
            "{name} must be discoverable via list-commands"
        );
    }
    assert!(
        body.lines().any(|line| line == "wait-output match regex"),
        "wait-output advertises its matchers: {body}"
    );
}

#[test]
fn parses_wait_output_substring_and_regex() {
    assert_eq!(
        parse_command("wait-output -t %3 -m 'build done' -T 500").unwrap(),
        MuxCommand::WaitOutput {
            pane: Target::Id(PaneId(3)),
            pattern: OutputPattern::Substring("build done".to_string()),
            timeout_ms: Some(500),
            start_line: None,
            end_line: None,
        }
    );
    assert_eq!(
        parse_command(r"wait-output -t %3 -r 'READY-\d+' -S -200 -E -1").unwrap(),
        MuxCommand::WaitOutput {
            pane: Target::Id(PaneId(3)),
            pattern: OutputPattern::Regex(r"READY-\d+".to_string()),
            timeout_ms: None,
            start_line: Some(-200),
            end_line: Some(-1),
        }
    );
    // A quoted pattern carrying flag-like words stays one value.
    assert_eq!(
        parse_command("wait-output -m 'x -r y' -t %1").unwrap(),
        MuxCommand::WaitOutput {
            pane: Target::Id(PaneId(1)),
            pattern: OutputPattern::Substring("x -r y".to_string()),
            timeout_ms: None,
            start_line: None,
            end_line: None,
        }
    );
    assert!(!parse_command("wait-output -t %1 -m x").unwrap().mutates());
}

#[test]
fn wait_output_rejects_bad_shapes() {
    for (line, needle) in [
        ("wait-output -m x", "requires -t"),
        ("wait-output -t %1", "requires -m TEXT or -r REGEX"),
        ("wait-output -t %1 -m a -r b", "exclusive"),
        ("wait-output -t %1 -m ''", "non-empty"),
        ("wait-output -t %1 -r '('", "invalid -r"),
        ("wait-output -t %1 -m a -T soon", "invalid -T"),
        ("wait-output -t %1 -m a -T -5", "invalid -T"),
        ("wait-output -t %1 -m a -S top", "invalid -S"),
        ("wait-output -t %1 -m a extra", "unexpected argument"),
        ("wait-output -t %1 -m a -x 1", "unknown flag -x"),
        ("wait-output -t %1 -m a -m b", "duplicate -m"),
    ] {
        let err = parse_command(line).expect_err(line);
        assert!(err.contains(needle), "{line}: {err}");
    }
}

#[test]
fn wait_output_reply_timeout_covers_the_wait() {
    use std::time::Duration;
    assert_eq!(
        reply_timeout("wait-output -t %1 -m x -T 30000"),
        Some(Duration::from_millis(35_000))
    );
    assert!(reply_timeout("wait-output -t %1 -m x").unwrap() > Duration::from_secs(86_400));
    assert!(reply_timeout(&format!("wait-output -t %1 -m x -T {}", u64::MAX)).is_some());
    assert_eq!(reply_timeout("capture-pane -t %1"), None);
    assert_eq!(reply_timeout("wait-output -t %1"), None);
}

/// H-API: the embedded JSON schema is the COMMANDS table, row for row —
/// names, feature tokens, and the daemon-level `features` line. A table
/// edit without `make mux-schema` fails here as well as in the gate.
#[test]
fn command_schema_matches_the_commands_table() {
    let schema: serde_json::Value =
        serde_json::from_str(COMMAND_SCHEMA_JSON).expect("schema is valid JSON");
    assert_eq!(schema["version"], 1);
    assert_eq!(schema["format"], "par-mux-command-schema");
    let commands = schema["commands"].as_array().expect("commands array");
    let mut table: Vec<(&str, Vec<&str>)> = COMMANDS
        .iter()
        .map(|(name, _, features)| (*name, features.to_vec()))
        .collect();
    table.sort();
    let schema_rows: Vec<(&str, Vec<&str>)> = commands
        .iter()
        .map(|c| {
            (
                c["name"].as_str().expect("name"),
                c["features"]
                    .as_array()
                    .expect("features")
                    .iter()
                    .map(|f| f.as_str().expect("token"))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        schema_rows, table,
        "stale crates/par-mux/schema/par-mux-commands.json — run: make mux-schema"
    );
    let body = list_commands_body();
    let daemon_line = body.lines().last().expect("trailing features line");
    let daemon: Vec<&str> = schema["daemon_features"]
        .as_array()
        .expect("daemon_features")
        .iter()
        .map(|f| f.as_str().expect("token"))
        .collect();
    assert_eq!(daemon_line, format!("features {}", daemon.join(" ")));
}

#[test]
fn list_commands_parses_the_schema_flag_and_refuses_others() {
    assert_eq!(
        parse_command("list-commands").expect("bare"),
        MuxCommand::ListCommands { schema: false }
    );
    assert_eq!(
        parse_command("list-commands -J").expect("-J"),
        MuxCommand::ListCommands { schema: true }
    );
    assert!(parse_command("list-commands -F '#{name}'").is_err());
    assert!(parse_command("list-commands extra").is_err());
    assert!(list_commands_body()
        .lines()
        .any(|line| line == "list-commands schema"));
}
#[test]
fn worktree_commands_parse_quoted_paths_and_flags() {
    assert_eq!(
        parse_command("worktree-create -c '/src/my repo' -b feat/x --base main -p /wt/x -s 'my x'")
            .unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/src/my repo".to_string(),
            branch: "feat/x".to_string(),
            base: Some("main".to_string()),
            path: Some("/wt/x".to_string()),
            name: Some("my x".to_string()),
        }
    );
    assert!(
        parse_command("worktree-create -c /src").is_err(),
        "-b is required"
    );
    assert!(
        parse_command("worktree-create -b x").is_err(),
        "-c is required"
    );
    assert!(parse_command("worktree-create -c /src -b x extra").is_err());
    let err = parse_command("worktree-create -c /src -n x").unwrap_err();
    assert!(
        err.contains("-b <branch>"),
        "the pre-D7 -n spelling points at -b: {err}"
    );
    assert_eq!(
        parse_command("worktree-open -c /wt/x").unwrap(),
        MuxCommand::WorktreeOpen {
            checkout: "/wt/x".to_string(),
            name: None,
        }
    );
    assert_eq!(
        parse_command("worktree-remove -t +3 -f").unwrap(),
        MuxCommand::WorktreeRemove {
            workspace: Target::Id(WorkspaceId(3)),
            force: true,
        }
    );
    assert!(parse_command("worktree-remove").is_err());
    assert_eq!(
        parse_command("list-worktrees").unwrap(),
        MuxCommand::ListWorktrees
    );
    assert!(reply_timeout("worktree-create -c /src -b x").is_some());
    assert!(reply_timeout("worktree-remove -t +1").is_some());
}
#[test]
fn parses_agent_start() {
    assert_eq!(
        parse_command("agent-start -t %2 claude --model 'opus 5'").unwrap(),
        MuxCommand::AgentStart {
            pane: Target::Id(PaneId(2)),
            kind: "claude".to_string(),
            program: None,
            args: vec!["--model".to_string(), "opus 5".to_string()],
            timeout_ms: None,
        }
    );
    assert_eq!(
        parse_command("agent-start -T 9000 -x /opt/bin/cc -t %0 claude").unwrap(),
        MuxCommand::AgentStart {
            pane: Target::Id(PaneId(0)),
            kind: "claude".to_string(),
            program: Some("/opt/bin/cc".to_string()),
            args: Vec::new(),
            timeout_ms: Some(9000),
        }
    );
    assert!(parse_command("agent-start -t %0 codex").unwrap().mutates());
    for (line, needle) in [
        ("agent-start claude", "requires -t"),
        ("agent-start -t %0", "requires an agent kind"),
        ("agent-start -t %0 Claude", "invalid agent kind"),
        ("agent-start -t %0 'two words'", "invalid agent kind"),
        ("agent-start -t %0 -T 0 claude", "-T must be between"),
        ("agent-start -t %0 -T 300001 claude", "-T must be between"),
        ("agent-start -t %0 -T soon claude", "invalid -T"),
        ("agent-start -t %0 -x '' claude", "non-empty program"),
        ("agent-start -t %0 -q claude", "unknown flag -q"),
    ] {
        let err = parse_command(line).expect_err(line);
        assert!(err.contains(needle), "{line}: {err}");
    }
}

#[test]
fn parses_agent_prompt() {
    assert_eq!(
        parse_command("agent-prompt -t %1 -w -u idle,done -T 60000 -- fix the -t flag").unwrap(),
        MuxCommand::AgentPrompt {
            pane: Target::Id(PaneId(1)),
            text: "fix the -t flag".to_string(),
            wait: true,
            until: vec!["idle".to_string(), "done".to_string()],
            timeout_ms: Some(60000),
        }
    );
    // The raw rest of the line is the text; one quoted word is unquoted.
    assert_eq!(
        parse_command("agent-prompt -t %1 say \"hi\" twice").unwrap(),
        MuxCommand::AgentPrompt {
            pane: Target::Id(PaneId(1)),
            text: "say \"hi\" twice".to_string(),
            wait: false,
            until: Vec::new(),
            timeout_ms: None,
        }
    );
    assert_eq!(
        parse_command("agent-prompt -t %1 'one quoted prompt'").unwrap(),
        MuxCommand::AgentPrompt {
            pane: Target::Id(PaneId(1)),
            text: "one quoted prompt".to_string(),
            wait: false,
            until: Vec::new(),
            timeout_ms: None,
        }
    );
    assert!(!parse_command("agent-prompt -t %1 hi").unwrap().mutates());
    for (line, needle) in [
        ("agent-prompt -t %1", "requires prompt text"),
        ("agent-prompt hi", "requires -t"),
        ("agent-prompt -t %1 -T 5 hi", "-T requires -w"),
        ("agent-prompt -t %1 -u idle hi", "-u requires -w"),
        ("agent-prompt -t %1 -w -u sleeping hi", "invalid -u state"),
    ] {
        let err = parse_command(line).expect_err(line);
        assert!(err.contains(needle), "{line}: {err}");
    }
}

#[test]
fn parses_agent_explain() {
    assert_eq!(
        parse_command("agent-explain -t %4").unwrap(),
        MuxCommand::AgentExplain {
            pane: Target::Id(PaneId(4))
        }
    );
    assert!(!parse_command("agent-explain -t %4").unwrap().mutates());
    assert!(parse_command("agent-explain").is_err());
    assert!(parse_command("agent-explain -t %4 extra").is_err());
}

#[test]
fn agent_verbs_reply_timeout_covers_their_waits() {
    use std::time::Duration;
    assert_eq!(
        reply_timeout("agent-start -t %0 claude"),
        Some(Duration::from_millis(
            AGENT_START_DEFAULT_TIMEOUT_MS + 5_000
        ))
    );
    assert_eq!(
        reply_timeout("agent-start -t %0 -T 60000 claude"),
        Some(Duration::from_millis(65_000))
    );
    assert_eq!(
        reply_timeout("agent-prompt -t %0 -w -T 120000 -- go"),
        Some(Duration::from_millis(125_000))
    );
    assert!(reply_timeout("agent-prompt -t %0 -w -- go").unwrap() > Duration::from_secs(86_400));
    assert_eq!(reply_timeout("agent-prompt -t %0 go"), None);
    assert_eq!(reply_timeout("agent-explain -t %0"), None);
}

#[test]
fn agent_verbs_appear_in_list_commands() {
    let body = list_commands_body();
    for line in ["agent-start", "agent-prompt wait until", "agent-explain"] {
        assert!(body.lines().any(|l| l == line), "{line}: {body}");
    }
}

/// `show-notification [-b body] [--] <title>`: the title is the rest of
/// the line (one quoted word unquoted), `--` admits a leading dash,
/// control characters fold, both fields truncate to their caps, and an
/// empty title or an unknown flag is refused.
#[test]
fn parses_show_notification() {
    let parsed = |line: &str| parse_command(line).expect(line);
    assert_eq!(
        parsed("show-notification build failed"),
        MuxCommand::ShowNotification {
            title: "build failed".into(),
            body: String::new(),
        }
    );
    assert_eq!(
        parsed("show-notification -b 'see the log' 'tests: 3 failed'"),
        MuxCommand::ShowNotification {
            title: "tests: 3 failed".into(),
            body: "see the log".into(),
        }
    );
    assert_eq!(
        parsed("show-notification -- -b is the title"),
        MuxCommand::ShowNotification {
            title: "-b is the title".into(),
            body: String::new(),
        }
    );
    assert_eq!(
        parsed("show-notification -b \"a\tb\" t"),
        MuxCommand::ShowNotification {
            title: "t".into(),
            body: "a b".into(),
        }
    );
    let long = "x".repeat(NOTIFICATION_TITLE_MAX + 20);
    let MuxCommand::ShowNotification { title, body } = parsed(&format!(
        "show-notification -b {} {long}",
        "y".repeat(NOTIFICATION_BODY_MAX + 5)
    )) else {
        panic!("variant");
    };
    assert_eq!(title.chars().count(), NOTIFICATION_TITLE_MAX);
    assert_eq!(body.chars().count(), NOTIFICATION_BODY_MAX);
    for bad in [
        "show-notification",
        "show-notification -b body",
        "show-notification ''",
        "show-notification -x title",
    ] {
        assert!(parse_command(bad).is_err(), "{bad} must be refused");
    }
    assert!(!parsed("show-notification t").mutates());
    assert!(list_commands_body()
        .lines()
        .any(|l| l == "show-notification"));
}

#[test]
fn parses_the_pane_stream_verbs() {
    assert_eq!(
        parse_command("observe-pane -t %4").unwrap(),
        MuxCommand::PaneStream {
            pane: Target::Id(PaneId(4)),
            control: false,
        }
    );
    assert_eq!(
        parse_command("control-pane -t 'my pane'").unwrap(),
        MuxCommand::PaneStream {
            pane: Target::Name("my pane".to_string()),
            control: true,
        }
    );
    assert!(!parse_command("control-pane -t %4").unwrap().mutates());
    for line in ["observe-pane", "control-pane", "observe-pane -t %4 extra"] {
        assert!(parse_command(line).is_err(), "{line} is refused");
    }
    let body = list_commands_body();
    for line in ["observe-pane", "control-pane"] {
        assert!(body.lines().any(|l| l == line), "{line}: {body}");
    }
}

/// A start command's own flags are not the create command's: once `--`
/// (or the command's first word) is seen, flag reading stops, so
/// `sh -c 'echo hi'`'s `-c` is not the start directory.
#[test]
fn split_window_stops_reading_flags_at_the_start_command() {
    for line in [
        "split-window -t %0 -- sh -c 'echo hi'",
        "split-window -t %0 sh -c 'echo hi'",
    ] {
        assert_eq!(
            parse_command(line).unwrap(),
            MuxCommand::SplitWindow {
                target: AnyTarget::Pane(PaneId(0)),
                direction: SplitDirection::Horizontal,
                percent: 50,
                before: false,
                start_dir: None,
                command: Some("sh -c 'echo hi'".into()),
            },
            "{line}"
        );
    }
    // Flags before `--` still count.
    assert_eq!(
        parse_command("split-window -t %0 -h -b -p 30 -c '/a b' -- sh -c x").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Vertical,
            percent: 30,
            before: true,
            start_dir: Some("/a b".into()),
            command: Some("sh -c x".into()),
        }
    );
    // Geometry and target words inside the command are the command's.
    assert_eq!(
        parse_command("split-window -t %0 -- ls -h -b -p 30 -t %9").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
            command: Some("ls -h -b -p 30 -t %9".into()),
        }
    );
    // `-p abc` inside the command used to fail the parse as a bad
    // percentage.
    assert!(parse_command("split-window -t %0 -- grep -p abc").is_ok());
}

#[test]
fn new_session_and_new_window_stop_reading_flags_at_the_start_command() {
    assert_eq!(
        parse_command("new-session -s a -- ssh -t host -e X=1 -c /x -n w -x 10").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("a".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: Some("ssh -t host -e X=1 -c /x -n w -x 10".into()),
        }
    );
    assert_eq!(
        parse_command("new-session -s a sh -c 'echo hi'").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("a".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: None,
            rows: None,
            command: Some("sh -c 'echo hi'".into()),
        }
    );
    assert_eq!(
        parse_command("new-window -t $0 -- sh -c x -n y -t $9").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(0))),
            name: None,
            start_dir: None,
            command: Some("sh -c x -n y -t $9".into()),
        }
    );
    // A quoted `--` is still the separator: words compare after quote
    // removal, as getopt sees them (respawn-pane's rule too).
    match parse_command("split-window -t %0 '--' sh -c x").unwrap() {
        MuxCommand::SplitWindow {
            start_dir, command, ..
        } => {
            assert_eq!(start_dir, None);
            assert_eq!(command.as_deref(), Some("sh -c x"));
        }
        other => panic!("not a split-window: {other:?}"),
    }
}

#[test]
fn respawn_pane_start_command_keeps_its_own_flags() {
    match parse_command("respawn-pane -t %0 -- sh -c 'echo hi'").unwrap() {
        MuxCommand::RespawnPane {
            start_dir, command, ..
        } => {
            assert_eq!(start_dir, None);
            assert_eq!(command.as_deref(), Some("sh -c 'echo hi'"));
        }
        other => panic!("not a respawn-pane: {other:?}"),
    }
}

/// Only a leading `-H` selects the hex form; a positional payload may
/// contain the word.
#[test]
fn set_buffer_hex_mode_needs_a_leading_flag() {
    assert_eq!(
        parse_command("set-buffer 'grep -H x'").unwrap(),
        MuxCommand::SetBuffer {
            content: "grep -H x".into()
        }
    );
    assert_eq!(
        parse_command("set-buffer -H 61 62").unwrap(),
        MuxCommand::SetBuffer {
            content: "ab".into()
        }
    );
}

/// Flags lead `set-environment` (getopt semantics): a VALUE that is
/// literally `-u` or `-t` is the value, not the unset flag or a target.
#[test]
fn set_environment_value_shaped_like_a_flag_is_the_value() {
    for (line, value) in [
        ("set-environment -t $0 NAME -u", "-u"),
        ("set-environment -t $0 NAME -t", "-t"),
        ("set-environment -t $0 NAME '-u'", "-u"),
    ] {
        assert_eq!(
            parse_command(line).unwrap(),
            MuxCommand::SetEnvironment {
                session: Target::Id(SessionId(0)),
                name: "NAME".into(),
                value: Some(value.into()),
            },
            "{line}"
        );
    }
    assert_eq!(
        parse_command("set-environment -t $0 -u -- NAME").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(0)),
            name: "NAME".into(),
            value: None,
        }
    );
    // An unsupported flag is refused, not taken as a NAME.
    assert!(parse_command("set-environment -g -t $0 NAME V").is_err());
}

/// `worktree-create`'s `-n` refusal reads shell words: a `-n` inside a
/// quoted value, or as a value, is not the removed branch flag.
#[test]
fn worktree_create_n_refusal_ignores_n_inside_values() {
    assert_eq!(
        parse_command("worktree-create -c /src -b x -s 'a -n b'").unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/src".to_string(),
            branch: "x".to_string(),
            base: None,
            path: None,
            name: Some("a -n b".to_string()),
        }
    );
    assert_eq!(
        parse_command("worktree-create -c /src -b x -s -n").unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/src".to_string(),
            branch: "x".to_string(),
            base: None,
            path: None,
            name: Some("-n".to_string()),
        }
    );
    let err = parse_command("worktree-create -c /src -s 'a b' -n x").unwrap_err();
    assert!(err.contains("-b <branch>"), "{err}");
}

/// The start point is `--base` (long-only): git's `worktree add -B`
/// means create-or-reset, so a short `-B` is refused with a pointer
/// (D7, 2026-10-20) — and the refusal reads shell words, so a `-B`
/// inside a quoted value or as a value is not the flag.
#[test]
fn worktree_create_b_refusal_points_at_base() {
    let err = parse_command("worktree-create -c /src -b x -B main").unwrap_err();
    assert!(err.contains("--base <base>"), "{err}");
    assert_eq!(
        parse_command("worktree-create -c /src -b x -s 'a -B b' --base dev").unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/src".to_string(),
            branch: "x".to_string(),
            base: Some("dev".to_string()),
            path: None,
            name: Some("a -B b".to_string()),
        }
    );
    assert_eq!(
        parse_command("worktree-create -c /src -b x --base -B").unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/src".to_string(),
            branch: "x".to_string(),
            base: Some("-B".to_string()),
            path: None,
            name: None,
        }
    );
}

/// `rename-window` drops one leading `--` (flags end there) and skips a
/// quoted target whole.
#[test]
fn rename_window_strips_a_leading_double_dash() {
    for (line, name) in [
        ("rename-window -t @0 -- x", "x"),
        ("rename-window -t @0 -- -- x", "-- x"),
        ("rename-window -t @0 x -- y", "x -- y"),
        ("rename-window -t @0 --  my   notes", "my notes"),
    ] {
        assert_eq!(
            parse_command(line).unwrap(),
            MuxCommand::RenameWindow {
                window: Target::Id(WindowId(0)),
                name: name.into(),
            },
            "{line}"
        );
    }
    assert!(parse_command("rename-window -t @0 --").is_err());
    assert_eq!(
        parse_command("rename-window -t 'my win' new").unwrap(),
        MuxCommand::RenameWindow {
            window: Target::Name("my win".into()),
            name: "new".into(),
        }
    );
}

/// tmux's `split-window -l <size>` (and `-e`/`-F`) are refused: their value
/// must never become the start command.
#[test]
fn split_window_refuses_size_env_and_format_flags() {
    for line in [
        "split-window -t %0 -l 30",
        "split-window -t %0 -h -l 30%",
        "split-window -t %0 -e A=1",
        "split-window -t %0 -F '#{pane_id}'",
    ] {
        let err = parse_command(line).unwrap_err();
        assert!(err.contains("not supported"), "{line}: {err}");
    }
    // Inside the start command they are the command's.
    assert_eq!(
        parse_command("split-window -t %0 -- ls -l 30").unwrap(),
        MuxCommand::SplitWindow {
            target: AnyTarget::Pane(PaneId(0)),
            direction: SplitDirection::Horizontal,
            percent: 50,
            before: false,
            start_dir: None,
            command: Some("ls -l 30".into()),
        }
    );
}

/// A flag word inside another flag's quoted value is part of that value:
/// `Args::flag`/`quoted_flag` used to find the `-t` in `-n 'a -t b'` and
/// send the command to target `b'`.
#[test]
fn flag_scan_skips_flags_inside_quoted_values() {
    assert_eq!(
        parse_command("new-window -n 'a -t b' -t s1").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Name("s1".into())),
            name: Some("a -t b".into()),
            start_dir: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("worktree-create -s 'a -c b' -c /r -b x").unwrap(),
        MuxCommand::WorktreeCreate {
            repo_dir: "/r".into(),
            branch: "x".into(),
            base: None,
            path: None,
            name: Some("a -c b".into()),
        }
    );
    // A quoted value that IS a flag word, ahead of the real flag whose own
    // value is quoted: the re-split used to take the first `-t` word.
    assert_eq!(
        parse_command("new-window -n '-t' -t 'my s'").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Name("my s".into())),
            name: Some("-t".into()),
            start_dir: None,
            command: None,
        }
    );
}

/// Every target helper reads through `quoted_flag`, so each kind skips
/// a flag hidden in an earlier quoted value.
#[test]
fn target_helpers_skip_flags_inside_quoted_values() {
    assert_eq!(
        parse_command("swap-pane -s 'x -t %9' -t %1").unwrap(),
        MuxCommand::SwapPanes {
            target: Target::Id(PaneId(1)),
            source: Target::Name("x -t %9".into()),
        }
    );
    assert_eq!(
        parse_command("break-pane -n 'w -s %9' -s %2").unwrap(),
        MuxCommand::BreakPane {
            source: Target::Id(PaneId(2)),
            name: Some("w -s %9".into()),
        }
    );
    assert_eq!(
        parse_command("list-windows -F '#{window_name} -t $9' -t $1").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Id(SessionId(1))),
            all: false,
            format: Some("#{window_name} -t $9".into()),
        }
    );
    assert_eq!(
        parse_command("list-sessions -F '#{session_name} -t +9' -t +1").unwrap(),
        MuxCommand::ListSessions {
            workspace: Some(Target::Id(WorkspaceId(1))),
            format: Some("#{session_name} -t +9".into()),
        }
    );
    assert_eq!(
        parse_command("new-session -s 'w -t +9' -t +1 -n 'x -c /bad' -c /good").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("w -t +9".into()),
            env: vec![],
            workspace: Some(Target::Id(WorkspaceId(1))),
            window_name: Some("x -c /bad".into()),
            start_dir: Some("/good".into()),
            cols: None,
            rows: None,
            command: None,
        }
    );
}

/// The numeric `Args::flag` readers skip quoted values too: the size
/// scan used to read `5'` out of the quoted name and fail the command.
#[test]
fn numeric_flags_skip_flags_inside_quoted_values() {
    assert_eq!(
        parse_command("new-session -s 'w -x 5' -x 80").unwrap(),
        MuxCommand::NewSession {
            restore_start_command: false,
            name: Some("w -x 5".into()),
            env: vec![],
            workspace: None,
            window_name: None,
            start_dir: None,
            cols: Some(80),
            rows: None,
            command: None,
        }
    );
    assert_eq!(
        parse_command("move-window -s 'w -t 9' -t 1").unwrap(),
        MuxCommand::MoveWindow {
            source: Target::Name("w -t 9".into()),
            index: 1,
        }
    );
    assert_eq!(
        parse_command("capture-pane -t 'p -S -5' -S -2").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Name("p -S -5".into()),
            start_line: Some(-2),
            end_line: None,
            escape: false,
        }
    );
}

/// `select-pane` reads `-T` through the quote-aware scan, so a `-T` that
/// only appears inside the quoted target is not a title: the line selects
/// the pane and leaves its title alone.
#[test]
fn select_pane_title_is_not_read_from_a_quoted_target() {
    assert_eq!(
        parse_command("select-pane -t 'a -T b'").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Name("a -T b".into()),
            title: None,
        }
    );
    assert_eq!(
        parse_command("select-pane -t 'a -T b' -T new").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Name("a -T b".into()),
            title: Some("new".into()),
        }
    );
}

/// A valueless flag inside another flag's quoted value, or quoted as that
/// value, does not fire: the destructive ones (`-f` force, `-g` group
/// kill) used to whenever the flag sat mid-value (`'x -f y'`), because
/// `has_flag` matched whitespace tokens.
#[test]
fn boolean_flags_skip_flags_inside_quoted_values() {
    let remove = |line: &str| match parse_command(line).unwrap() {
        MuxCommand::WorktreeRemove { workspace, force } => (workspace, force),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        remove("worktree-remove -t 'x -f y'"),
        (Target::Name("x -f y".into()), false)
    );
    assert_eq!(
        remove("worktree-remove -t 'x -f'"),
        (Target::Name("x -f".into()), false)
    );
    assert_eq!(
        remove("worktree-remove -t \"x -f y\""),
        (Target::Name("x -f y".into()), false)
    );
    assert_eq!(
        remove("worktree-remove -t '-f'"),
        (Target::Name("-f".into()), false)
    );
    assert_eq!(
        remove("worktree-remove -t 'x' -f"),
        (Target::Name("x".into()), true)
    );

    let kill = |line: &str| match parse_command(line).unwrap() {
        MuxCommand::KillWorkspace { workspace, group } => (workspace, group),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        kill("kill-workspace -t 'a -g b'"),
        (Target::Name("a -g b".into()), false)
    );
    assert_eq!(
        kill("kill-workspace -t 'a -g'"),
        (Target::Name("a -g".into()), false)
    );
    assert_eq!(
        kill("kill-workspace -t \"-g\""),
        (Target::Name("-g".into()), false)
    );
    assert_eq!(
        kill("kill-workspace -t 'a' -g"),
        (Target::Name("a".into()), true)
    );
}

/// The geometry and mode flags read through the same scan: a `-h`/`-b`
/// inside `split-window`'s quoted start dir, a `-h` inside `join-pane`'s
/// quoted source, a `-Z` inside `resize-pane`'s quoted target, a `-a`
/// inside `list-windows`' quoted format, and a `-e` inside
/// `capture-pane`'s quoted target are all part of their values.
#[test]
fn mode_flags_skip_flags_inside_quoted_values() {
    match parse_command("split-window -t %0 -c '/p -h -b x'").unwrap() {
        MuxCommand::SplitWindow {
            direction,
            before,
            start_dir,
            ..
        } => {
            assert_eq!(direction, SplitDirection::Horizontal);
            assert!(!before);
            assert_eq!(start_dir.as_deref(), Some("/p -h -b x"));
        }
        other => panic!("{other:?}"),
    }
    match parse_command("split-window -t %0 -c '/p' -h -b").unwrap() {
        MuxCommand::SplitWindow {
            direction, before, ..
        } => {
            assert_eq!(direction, SplitDirection::Vertical);
            assert!(before);
        }
        other => panic!("{other:?}"),
    }
    match parse_command("join-pane -s 'a -h b' -t %1").unwrap() {
        MuxCommand::JoinPane { direction, .. } => {
            assert_eq!(direction, SplitDirection::Horizontal)
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse_command("resize-pane -t 'a -Z b' -L").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Name("a -Z b".into()),
            adjustment: ResizeAdjustment::Relative {
                direction: ResizeDirection::Left,
                cells: 5,
            },
        }
    );
    assert_eq!(
        parse_command("resize-pane -t 'a' -Z").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Name("a".into()),
            adjustment: ResizeAdjustment::Zoom,
        }
    );
    // A direction flag only inside the target is no direction at all.
    assert!(parse_command("resize-pane -t 'a -L b'").is_err());
    assert_eq!(
        parse_command("list-windows -F '#{window_name} -a #{window_id}' -t $1").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Id(SessionId(1))),
            all: false,
            format: Some("#{window_name} -a #{window_id}".into()),
        }
    );
    assert_eq!(
        parse_command("capture-pane -t 'x -e y'").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Name("x -e y".into()),
            start_line: None,
            end_line: None,
            escape: false,
        }
    );
}

/// The `-t` verbs with free words after the target read their flags only
/// until the first other word: a key, name, or value that is literally
/// `-t` (bare after `--`, or quoted) used to be eaten as a second `-t`.
#[test]
fn value_shaped_t_is_a_word_not_the_target_flag() {
    assert_eq!(
        parse_command("set-option -- remain-on-exit -t").unwrap(),
        MuxCommand::SetOption {
            key: "remain-on-exit".into(),
            value: "-t".into(),
        }
    );
    assert_eq!(
        parse_command("set-option exit-empty '-t'").unwrap(),
        MuxCommand::SetOption {
            key: "exit-empty".into(),
            value: "-t".into(),
        }
    );
    assert_eq!(
        parse_command("set-option -t $1 '-t' on").unwrap(),
        MuxCommand::SetOption {
            key: "-t".into(),
            value: "on".into(),
        }
    );
    assert_eq!(
        parse_command("rename-session -t $1 -- -t").unwrap(),
        MuxCommand::RenameSession {
            session: Target::Id(SessionId(1)),
            name: "-t".into(),
        }
    );
    assert_eq!(
        parse_command("rename-session -t $1 '-t'").unwrap(),
        MuxCommand::RenameSession {
            session: Target::Id(SessionId(1)),
            name: "-t".into(),
        }
    );
    assert_eq!(
        parse_command("rename-workspace -t +1 '-t'").unwrap(),
        MuxCommand::RenameWorkspace {
            workspace: Target::Id(WorkspaceId(1)),
            name: "-t".into(),
        }
    );
    // The value word used to vanish, turning a set into the read-only
    // query; now it is refused as the bad value it is.
    let err = parse_command("set-workspace-respawn -t +1 '-t'").unwrap_err();
    assert!(err.contains("expected on, off, or toggle"), "{err}");
    let err = parse_command("set-workspace-respawn -t +1 -- -t").unwrap_err();
    assert!(err.contains("expected on, off, or toggle"), "{err}");
    // A second unquoted `-t` in flag position is a loud duplicate.
    assert!(parse_command("rename-session -t $1 -t $2 x").is_err());
    // The flag-free and missing/empty-target shapes keep their errors.
    assert!(parse_command("rename-session x").is_err(), "-t required");
    let err = parse_command("rename-workspace -t '' x").unwrap_err();
    assert!(err.contains("non-empty"), "{err}");
}

/// `new-window -e` is refused with a pointer, never dropped: a pane's
/// environment is its session's, which respawn and restore re-seed from.
#[test]
fn new_window_refuses_e_with_a_pointer() {
    let err = parse_command("new-window -t $1 -e A=1").unwrap_err();
    assert!(err.contains("the -e flag is not supported here"), "{err}");
    assert!(err.contains("set-environment"), "{err}");
    // Inside the start command it belongs to the command.
    assert_eq!(
        parse_command("new-window -t $1 -- env -e A=1").unwrap(),
        MuxCommand::NewWindow {
            target: Some(AnyTarget::Session(SessionId(1))),
            name: None,
            start_dir: None,
            command: Some("env -e A=1".into()),
        }
    );
}

/// `join-pane` refuses what it does not read, the way `split-window`
/// refuses `-l`: tmux's `-l 30` used to be dropped silently.
#[test]
fn join_pane_refuses_leftover_arguments() {
    for line in [
        "join-pane -s %1 -t %2 -l 30",
        "join-pane -s %1 -t %2 -h -l 30%",
        "join-pane -s %1 -t %2 -d",
    ] {
        let err = parse_command(line).unwrap_err();
        assert!(err.contains("flag is not supported here"), "{line}: {err}");
    }
    let err = parse_command("join-pane -s %1 -t %2 extra").unwrap_err();
    assert!(err.contains("unexpected argument"), "{err}");
    assert_eq!(
        parse_command("join-pane -s %1 -t %2 -h -p 30").unwrap(),
        MuxCommand::JoinPane {
            source: Target::Id(PaneId(1)),
            target: Target::Id(PaneId(2)),
            direction: SplitDirection::Vertical,
            percent: 30,
        }
    );
    // A pane named `-h` is a value, not the arrangement flag.
    assert_eq!(
        parse_command("join-pane -s -h -t %2").unwrap(),
        MuxCommand::JoinPane {
            source: Target::Name("-h".into()),
            target: Target::Id(PaneId(2)),
            direction: SplitDirection::Horizontal,
            percent: 50,
        }
    );
}

/// `new-session -e` reads only real flag words: a quoted `'-e'` value is
/// a name, not an environment flag swallowing the next word.
#[test]
fn quoted_e_value_is_not_an_env_flag() {
    match parse_command("new-session -s '-e' -e A=1 -e 'B=two words'").unwrap() {
        MuxCommand::NewSession { name, env, .. } => {
            assert_eq!(name.as_deref(), Some("-e"));
            assert_eq!(
                env,
                vec![("A".into(), "1".into()), ("B".into(), "two words".into())]
            );
        }
        other => panic!("{other:?}"),
    }
    match parse_command("new-session -n '-e' -s x").unwrap() {
        MuxCommand::NewSession { name, env, .. } => {
            assert_eq!(name.as_deref(), Some("x"));
            assert!(env.is_empty(), "{env:?}");
        }
        other => panic!("{other:?}"),
    }
}

/// An unquoted flag word spelled as another flag's value is that value:
/// `worktree-remove -t -f` used to set force AND target `-f`.
#[test]
fn flag_spelled_value_is_read_once() {
    assert_eq!(
        parse_command("worktree-remove -t -f").unwrap(),
        MuxCommand::WorktreeRemove {
            workspace: Target::Name("-f".into()),
            force: false,
        }
    );
    assert_eq!(
        parse_command("worktree-remove -t -f -f").unwrap(),
        MuxCommand::WorktreeRemove {
            workspace: Target::Name("-f".into()),
            force: true,
        }
    );
    assert_eq!(
        parse_command("kill-workspace -t -g").unwrap(),
        MuxCommand::KillWorkspace {
            workspace: Target::Name("-g".into()),
            group: false,
        }
    );
    match parse_command("split-window -t -h").unwrap() {
        MuxCommand::SplitWindow {
            target, direction, ..
        } => {
            assert_eq!(target, AnyTarget::Name("-h".into()));
            assert_eq!(direction, SplitDirection::Horizontal);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse_command("capture-pane -t -e").unwrap(),
        MuxCommand::CapturePane {
            pane: Target::Name("-e".into()),
            start_line: None,
            end_line: None,
            escape: false,
        }
    );
    assert_eq!(
        parse_command("resize-pane -t -Z -L").unwrap(),
        MuxCommand::ResizePane {
            pane: Target::Name("-Z".into()),
            adjustment: ResizeAdjustment::Relative {
                direction: ResizeDirection::Left,
                cells: 5,
            },
        }
    );
    assert_eq!(
        parse_command("select-pane -t -T").unwrap(),
        MuxCommand::SelectPane {
            pane: Target::Name("-T".into()),
            title: None,
        }
    );
    assert_eq!(
        parse_command("list-windows -t -a").unwrap(),
        MuxCommand::ListWindows {
            session: Some(Target::Name("-a".into())),
            all: false,
            format: None,
        }
    );
}

/// `split_leading_flags` compares raw words: a quoted `'-u'` NAME is the
/// name, not `set-environment`'s unset flag.
#[test]
fn leading_flags_read_a_quoted_flag_word_as_the_tail() {
    assert_eq!(
        parse_command("set-environment -t $1 '-u' v").unwrap(),
        MuxCommand::SetEnvironment {
            session: Target::Id(SessionId(1)),
            name: "-u".into(),
            value: Some("v".into()),
        }
    );
}

/// A quoted pane target parses exactly like the bare form on `send-keys`,
/// the way every other `-t` verb (`pane-info` here) already reads it —
/// quoting a `%N` id used to leave the quotes in the token and refuse the
/// target (kanban 01a1699e, probe 1).
#[test]
fn send_keys_parses_a_quoted_target_like_pane_info() {
    let bare = parse_command("send-keys -t %0 -l 'make check'").unwrap();
    let quoted = parse_command("send-keys -t '%0' -l 'make check'").unwrap();
    assert_eq!(bare, quoted, "quoting a pane target must not change it");
    let double = parse_command("send-keys -t \"%0\" --enter -l x").unwrap();
    assert_eq!(
        double,
        parse_command("send-keys -t %0 --enter -l x").unwrap(),
        "double quotes group the same"
    );
    // A quoted spaced name rides whole, as on every other verb.
    match parse_command("send-keys -t 'my pane' -l hello").unwrap() {
        MuxCommand::SendKeys { pane, .. } => {
            assert_eq!(pane, Target::Name("my pane".into()));
        }
        other => panic!("not a send-keys: {other:?}"),
    }
    // The reference verb: pane-info accepts the quoted id.
    match parse_command("pane-info -t '%0'").unwrap() {
        MuxCommand::PaneInfo { pane, .. } => assert_eq!(pane, Target::Id(PaneId(0))),
        other => panic!("not a pane-info: {other:?}"),
    }
}

/// A start tail that is exactly one quoted word groups like tmux's argv:
/// the quotes come off before `sh -c`, so `split-window -t %0 'sleep 60'`
/// runs `sleep 60` instead of one unrunnable `'sleep 60'` word (kanban
/// 01a1699e, probe 2). Anything else keeps the raw tail — quoting inside
/// it is the pane shell's business.
#[test]
fn quoted_start_tail_groups_like_tmux() {
    for line in [
        "split-window -t %0 'sleep 60'",
        "split-window -t %0 -- 'sleep 60'",
        "split-window -t %0 \"sleep 60\"",
        "split-window -t %0 -h 'sleep 60'",
    ] {
        match parse_command(line).unwrap_or_else(|e| panic!("{line}: {e}")) {
            MuxCommand::SplitWindow { command, .. } => {
                assert_eq!(command.as_deref(), Some("sleep 60"), "{line}");
            }
            other => panic!("{line}: not a split-window: {other:?}"),
        }
    }
    // The tail is not one quoted word: the raw text rides, quotes inside
    // it included.
    match parse_command("split-window -t %0 echo 'hi there'").unwrap() {
        MuxCommand::SplitWindow { command, .. } => {
            assert_eq!(command.as_deref(), Some("echo 'hi there'"));
        }
        other => panic!("not a split-window: {other:?}"),
    }
    // The sibling surfaces that share the tail contract.
    for (line, expected) in [
        ("new-session -s logs -- 'tail -f x'", "tail -f x"),
        ("new-window -n logs 'tail -f x'", "tail -f x"),
        ("respawn-pane -t %0 'echo hi'", "echo hi"),
    ] {
        match parse_command(line).unwrap_or_else(|e| panic!("{line}: {e}")) {
            MuxCommand::NewSession { command, .. }
            | MuxCommand::NewWindow { command, .. }
            | MuxCommand::RespawnPane { command, .. } => {
                assert_eq!(command.as_deref(), Some(expected), "{line}");
            }
            other => panic!("{line}: unexpected command: {other:?}"),
        }
    }
}

#[test]
fn send_keys_literal_carries_control_bytes_verbatim() {
    assert_eq!(
        encoded("send-keys -t %0 -l '\x1b]1337;x\x07'"),
        b"\x1b]1337;x\x07"
    );
}

#[test]
fn send_keys_literal_keeps_backslashes_inside_double_quotes() {
    assert_eq!(
        encoded(r#"send-keys -t %0 -l "printf '\033]x'""#),
        br"printf '\033]x'"
    );
}

#[test]
fn send_keys_literal_keeps_single_quoted_backslashes() {
    assert_eq!(
        encoded(r"send-keys -t %0 -l 'printf \033]x'"),
        br"printf \033]x"
    );
}

#[test]
fn report_state_parses_the_full_form() {
    let MuxCommand::ReportState {
        pane,
        agent,
        state,
        message,
    } = parse_command("report-state -t %3 -a claude -s working -m 'awaiting 2 background agents'")
        .expect("parses")
    else {
        panic!("not report-state");
    };
    assert_eq!(pane, Target::parse("%3").unwrap());
    assert_eq!(agent.as_deref(), Some("claude"));
    assert_eq!(state.as_deref(), Some("working"));
    assert_eq!(message.as_deref(), Some("awaiting 2 background agents"));
}

#[test]
fn report_state_without_s_is_the_clear_form() {
    let MuxCommand::ReportState { state, message, .. } =
        parse_command("report-state -t %3").expect("parses")
    else {
        panic!("not report-state");
    };
    assert_eq!(state, None);
    assert_eq!(message, None);
}

#[test]
fn report_state_rejects_unknown_states() {
    // `unknown` is the absence of a claim, never a stored state — the clear
    // form is `-s` omitted, not `-s unknown`. (`-s` with no following word
    // reads as omitted under the shared flag grammar, the clear form.)
    for raw in ["unknown", "WORKING", "banana"] {
        assert!(
            parse_command(&format!("report-state -t %3 -s {raw}")).is_err(),
            "{raw} must not parse"
        );
    }
}

#[test]
fn report_state_rejects_message_without_state() {
    assert!(parse_command("report-state -t %3 -m lonely").is_err());
}

#[test]
fn report_state_rejects_stray_words() {
    assert!(parse_command("report-state -t %3 stray").is_err());
}

#[test]
fn parses_set_workspace_restore_policy_values_and_query() {
    for (word, expected) in [
        (
            "start-command",
            crate::mux::config::RestorePolicy::StartCommand,
        ),
        (
            "agent-resume",
            crate::mux::config::RestorePolicy::AgentResume,
        ),
        ("none", crate::mux::config::RestorePolicy::NoRestore),
    ] {
        let command = parse_command(&format!("set-workspace-restore-policy -t +1 {word}"))
            .unwrap_or_else(|err| panic!("{word} parses: {err}"));
        assert_eq!(
            command,
            MuxCommand::SetWorkspaceRestorePolicy {
                workspace: Target::Id(WorkspaceId(1)),
                policy: Some(expected),
            }
        );
        assert!(command.mutates(), "{word} saves");
    }
    let query = parse_command("set-workspace-restore-policy -t main").unwrap();
    assert_eq!(
        query,
        MuxCommand::SetWorkspaceRestorePolicy {
            workspace: Target::Name("main".to_string()),
            policy: None,
        }
    );
    assert!(!query.mutates(), "the query form reads only");
    assert!(
        parse_command("set-workspace-restore-policy none").is_err(),
        "-t required"
    );
    assert!(parse_command("set-workspace-restore-policy -t +1 maybe").is_err());
    assert!(parse_command("set-workspace-restore-policy -t +1 none off").is_err());
}

#[test]
fn parses_new_session_restore_start_command() {
    let command = parse_command(
        "new-session --restore start-command claude-wrapper --model sonnet --plugin-dir /opt/mod",
    )
    .unwrap();
    let MuxCommand::NewSession {
        command: start,
        restore_start_command,
        ..
    } = command
    else {
        panic!("a new-session line parses to NewSession");
    };
    assert!(
        restore_start_command,
        "--restore start-command sets the pane's keep flag"
    );
    assert_eq!(
        start.as_deref(),
        Some("claude-wrapper --model sonnet --plugin-dir /opt/mod"),
        "the start command rides whole after the flag"
    );

    // The plain create is unchanged.
    let command = parse_command("new-session -s dev").unwrap();
    let MuxCommand::NewSession {
        restore_start_command,
        ..
    } = command
    else {
        panic!("a new-session line parses to NewSession");
    };
    assert!(!restore_start_command);
}

#[test]
fn new_session_restore_flag_rejects_bad_values_and_a_missing_start_command() {
    assert!(
        parse_command("new-session --restore agent-resume claude").is_err(),
        "the create flag has one value"
    );
    assert!(
        parse_command("new-session --restore start-command").is_err(),
        "a default shell has nothing to keep"
    );
}
