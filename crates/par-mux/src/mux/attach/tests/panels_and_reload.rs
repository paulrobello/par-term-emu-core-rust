//! Help/picker panels, listbox scrolling, and client reload chords.

use super::*;

/// The help panel compose: the filter line is ALWAYS present (the
/// placeholder when inactive — the round-3 no-feedback fix), the
/// filter narrows rows live (headers hide when nothing beneath them
/// matches), the window scrolls, and the footer line names the
/// controls. The border ring/title/badge are the renderer's
/// paint_overlay job, not the compose's.
#[test]
fn compose_help_panel_filters_scrolls_and_chromes() {
    let rows = super::help_rows(0x02, 0x12, Default::default(), 1);
    // Full panel: content + footer — no idle placeholder (the footer
    // already advertises `search /`), and entering filter mode swaps
    // in the cursor-input line immediately — `/` must give visible
    // feedback before any typing.
    let panel = super::compose_help_panel(&rows, "", false, 100, 0);
    let text: Vec<&str> = panel.iter().map(|r| r.text.as_str()).collect();
    assert!(
        !text.iter().any(|t| t.contains("press / to filter")),
        "no idle placeholder row: {text:?}"
    );
    assert_eq!(
        text[0], " global ",
        "the content leads the idle panel: {}",
        text[0]
    );
    let active = super::compose_help_panel(&rows, "", true, 100, 0);
    assert_eq!(
        active[0].text, " /▌",
        "the active filter line shows the input cursor: {}",
        active[0].text
    );
    assert!(
        text.last()
            .copied()
            .unwrap_or("")
            .contains("close esc/enter"),
        "the footer names the controls"
    );
    // Filter: "swap" keeps only the swap rows (and their header).
    let filtered = super::compose_help_panel(&rows, "swap", false, 100, 0);
    let ftext: String = filtered
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ftext.contains("swap"), "the matching rows survive: {ftext}");
    assert!(
        ftext.contains("panes"),
        "the matching rows' header survives"
    );
    assert!(!ftext.contains("detach"), "non-matching rows drop: {ftext}");
    assert!(
        !ftext.contains(" global "),
        "a category with no matching rows hides: {ftext}"
    );
    // Filter box drawn while active.
    assert!(
        ftext.contains("/swap▌"),
        "the filter box shows the live filter: {ftext}"
    );
    // Scroll: a tiny window shows a slice, and scrolling past the end
    // clamps.
    let small = super::compose_help_panel(&rows, "", false, 3, 0);
    let big = super::compose_help_panel(&rows, "", false, 3, 10_000);
    assert_ne!(small[1..4], big[1..4], "scrolling moves the window");
    assert_eq!(
        super::compose_help_panel(&rows, "", false, 3, 10_000).len(),
        small.len(),
        "the panel shape is stable under scroll"
    );
}

/// The picker's rows: every session's windows as one flat list — no
/// session rows, a session not being a landing target — with the shown
/// window `>`-marked, its session's active window `*`-suffixed, and
/// the parallel refs mapping each row to its window.
#[test]
fn picker_rows_list_windows_flat_and_mark_current() {
    let entries = vec![
        super::PickerEntry {
            session_id: "$0".to_string(),
            session_name: "work".to_string(),
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@1".to_string()),
            current: true,
        },
        super::PickerEntry {
            session_id: "$1".to_string(),
            session_name: "build".to_string(),
            windows: vec![("@2".to_string(), "logs".to_string())],
            active_window: Some("@2".to_string()),
            current: false,
        },
    ];
    let (rows, refs) = super::picker_rows(&entries, Some("@0"));
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text.len(), 3, "three windows, no session rows: {text:?}");
    assert_eq!(text[0], " >  @0: main", "the shown window is >-marked");
    assert_eq!(text[1], "    @1: vim *", "the active window carries *");
    assert_eq!(text[2], "    @2: logs *");
    assert!(
        rows.iter().all(|r| !r.accent),
        "no row is an accent (header) row"
    );
    assert_eq!(
        refs,
        vec![
            super::PickerRef::Window(0, 0),
            super::PickerRef::Window(0, 1),
            super::PickerRef::Window(1, 0),
        ]
    );
}

/// A window NAME two sessions share is prefixed with its session's
/// name (`session/name`) so the flat list stays unambiguous; a name
/// one session owns stays bare.
#[test]
fn picker_rows_prefix_a_session_shared_window_name() {
    let entries = vec![
        super::PickerEntry {
            session_id: "$0".to_string(),
            session_name: "work".to_string(),
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@0".to_string()),
            current: true,
        },
        super::PickerEntry {
            session_id: "$1".to_string(),
            session_name: "build".to_string(),
            windows: vec![
                ("@2".to_string(), "main".to_string()),
                ("@3".to_string(), "logs".to_string()),
            ],
            active_window: Some("@2".to_string()),
            current: false,
        },
    ];
    let (rows, _refs) = super::picker_rows(&entries, None);
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        text,
        [
            "    @0: work/main *",
            "    @1: vim",
            "    @2: build/main *",
            "    @3: logs",
        ],
        "shared names carry their session, owned names stay bare"
    );
}

/// The picker compose: the filter line, the filtered content, the `▸`
/// cursor on the selected row, the footer, and the refs/start return
/// that maps a click at composed row `i` to content row `start + i -
/// 1`.
#[test]
fn compose_picker_panel_filters_marks_and_maps_clicks() {
    let entries = vec![
        super::PickerEntry {
            session_id: "$0".to_string(),
            session_name: "work".to_string(),
            windows: vec![
                ("@0".to_string(), "main".to_string()),
                ("@1".to_string(), "vim".to_string()),
            ],
            active_window: Some("@1".to_string()),
            current: true,
        },
        super::PickerEntry {
            session_id: "$1".to_string(),
            session_name: "build".to_string(),
            windows: vec![("@2".to_string(), "logs".to_string())],
            active_window: Some("@2".to_string()),
            current: false,
        },
    ];
    let (rows, refs) = super::picker_rows(&entries, Some("@0"));
    // Full panel: 3 content rows + footer — no idle placeholder (the
    // footer already advertises `filter /`).
    let (panel, refs2, start) =
        super::compose_picker_panel(&rows, &refs, "", false, 0, 100, 0, super::PICKER_FOOTER);
    assert_eq!(panel.len(), 4, "content + footer: {panel:?}");
    assert_eq!(start, 0);
    assert_eq!(refs2, refs);
    assert_eq!(
        panel.last().map(|r| r.text.as_str()),
        Some(super::PICKER_FOOTER)
    );
    // Filter "vim" keeps only that window row.
    let (filtered, frefs, fstart) =
        super::compose_picker_panel(&rows, &refs, "vim", false, 0, 100, 0, super::PICKER_FOOTER);
    let ftext = filtered
        .iter()
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ftext.contains("vim"), "the matching row survives: {ftext}");
    assert_eq!(frefs, vec![super::PickerRef::Window(0, 1)]);
    assert_eq!(fstart, 0);
    // The selection cursor: selected=1 marks the second content row.
    let (marked, _, _) =
        super::compose_picker_panel(&rows, &refs, "", false, 1, 100, 0, super::PICKER_FOOTER);
    assert!(
        marked[1].text.starts_with('▸'),
        "the selected row carries the cursor glyph: {}",
        marked[1].text
    );
    assert!(
        !marked[0].text.starts_with('▸'),
        "only the selected row carries it"
    );
    // Click mapping: composed row 1 (content row 1) maps to content
    // index start + 1.
    let (small, _, small_start) =
        super::compose_picker_panel(&rows, &refs, "", false, 0, 2, 0, super::PICKER_FOOTER);
    assert_eq!(small.len(), 3, "2 visible + footer");
    assert_eq!(small_start, 0);
    // Panning: a selected row below the window moves `start`.
    let (_panned, _, panned_start) =
        super::compose_picker_panel(&rows, &refs, "", false, 2, 2, 0, super::PICKER_FOOTER);
    assert_eq!(
        panned_start, 1,
        "the window pans to keep the cursor visible"
    );
}

/// The palette rows' chord column: every chorded action spells its
/// EFFECTIVE binding (the help panel's spelling — a remapped chord
/// shows its remapped key), and the chordless actions spell none.
#[test]
fn palette_action_chord_spells_the_live_binding() {
    let mut m = crate::mux::config::Management::default();
    // Defaults: new window c, kill pane x, keybinds ?, reload C-r,
    // detach d — prefix C-b.
    assert_eq!(
        palette_action_chord(4, 0x02, 0x12, &m).as_deref(),
        Some("C-b c")
    );
    assert_eq!(
        palette_action_chord(9, 0x02, 0x12, &m).as_deref(),
        Some("C-b x")
    );
    assert_eq!(
        palette_action_chord(1, 0x02, 0x12, &m).as_deref(),
        Some("C-b ?")
    );
    assert_eq!(
        palette_action_chord(2, 0x02, 0x12, &m).as_deref(),
        Some("C-b C-r")
    );
    assert_eq!(
        palette_action_chord(3, 0x02, 0x12, &m).as_deref(),
        Some("C-b d")
    );
    // A remapped chord shows the remapped key.
    m.kill_pane = crate::mux::config::ChordKey::plain(b'K');
    assert_eq!(
        palette_action_chord(9, 0x02, 0x12, &m).as_deref(),
        Some("C-b K")
    );
    // The chordless action (settings), and an index past the table.
    assert_eq!(palette_action_chord(0, 0x02, 0x12, &m), None);
    assert_eq!(palette_action_chord(99, 0x02, 0x12, &m), None);
}

/// The listbox panning rule: `start` moves only when `selected`
/// leaves the visible window.
#[test]
fn listbox_scroll_pans_only_at_the_edges() {
    assert_eq!(super::listbox_scroll(0, 0, 3), 0);
    assert_eq!(super::listbox_scroll(0, 2, 3), 0, "inside the window");
    assert_eq!(super::listbox_scroll(0, 3, 3), 1, "one past the end");
    assert_eq!(super::listbox_scroll(1, 0, 3), 0, "above the window");
    assert_eq!(super::listbox_scroll(0, 0, 0), 0, "an empty window is safe");
}

/// A management chord spelling that does not parse is a reload error
/// (the flash), not a silent keep-the-old.
#[test]
fn management_chord_override_errors_on_malformed_spelling() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nsplit-right = \"C-\"\n").expect("parse");
    assert!(
        crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management::default(),
                resize_step: 1,
                ..crate::mux::config::Chords::with_defaults()
            },
        )
        .is_err(),
        "a malformed chord is an error"
    );
}

/// The reload chord: a written config with a new prefix rebinds the
/// running session's chords LIVE — the prefix typed after the reload
/// detaches under the NEW prefix, the old one forwards — and the
/// `reload-config` control command rides the same chord to the
/// daemon.
#[test]
fn reload_chord_rebinds_prefix_and_sends_reload_config() {
    let (_daemon, path) = FakeDaemon::bind("reload");
    let mut session = Session {
        conn: conn::AttachConn::connect(&path).expect("connect"),
        socket_path: path.clone(),
        pane: "%0".to_string(),
        emulator: render::PaneEmulator::new(80, 24),
        window: String::new(),
        session_id: None,
        session_name: String::new(),
        workspaces: Vec::new(),
        active_workspace: None,
        session_workspace: None,
        pane_title: String::new(),
        agents: 0,
        exited: None,
        drawn_size: None,
        settling: false,
        prefix: 0x02,
        prefix_pending: false,
        prefix_utf8: None,
        reload_key: 0x12,
        management: crate::mux::config::Management::default(),
        resize_step: 1,
        resize_mode: false,
        flash: None,
        paste: PasteTracker::default(),
    };
    // A config naming a new prefix (C-a) and a moved reload chord
    // (C-a C-s): the rebind is pure over the parsed file, so the test
    // feeds it directly (the env-pin path is covered end to end by
    // the PTY-level attach tests, where the child's env is injectable
    // — QA-196 forbids env mutation in this suite).
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-a\"\nreload = \"C-a C-s\"\n").unwrap();
    let new_chords = crate::mux::config::reload_client_chords(
        &file,
        &crate::mux::config::Chords {
            prefix: session.prefix,
            reload: session.reload_key,
            management: session.management,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("chords parse");
    session.prefix = new_chords.prefix;
    session.reload_key = new_chords.reload;
    session.management = new_chords.management;
    // The reloaded session routes: prefix d (0x01 'd') detaches under
    // the NEW prefix; the old prefix byte no longer intercepts.
    assert!(
        session.route_bytes(&[0x01, b'd']),
        "the NEW prefix detaches"
    );
    assert!(
        !session.route_bytes(&[0x02, b'x']),
        "the OLD prefix no longer intercepts"
    );
}

/// The pure chord rebind errors on a malformed chord (the error is
/// what the status flash shows) instead of silently keeping the old
/// chords.
#[test]
fn reload_client_chords_errors_on_malformed_chords() {
    let file: crate::mux::config::ConfigFile =
        toml::from_str("[client]\nprefix = \"C-\"\n").unwrap();
    assert!(
        crate::mux::config::reload_client_chords(
            &file,
            &crate::mux::config::Chords {
                prefix: 0x02,
                reload: 0x12,
                management: crate::mux::config::Management::default(),
                ..crate::mux::config::Chords::with_defaults()
            }
        )
        .is_err(),
        "a malformed prefix is an error"
    );
    // An absent tier keeps the current chords.
    let partial: crate::mux::config::ConfigFile =
        toml::from_str("[daemon]\nsocket = \"work\"\n").unwrap();
    let kept = crate::mux::config::reload_client_chords(
        &partial,
        &crate::mux::config::Chords {
            prefix: 0x02,
            reload: 0x12,
            management: crate::mux::config::Management {
                split_right: crate::mux::config::ChordKey::plain(b'&'),
                split_down: crate::mux::config::ChordKey::plain(b'*'),
                kill_pane: crate::mux::config::ChordKey::plain(b'X'),
                break_pane: crate::mux::config::ChordKey::plain(b'!'),
                join_pane: crate::mux::config::ChordKey::plain(b'J'),
                palette: crate::mux::config::ChordKey::plain(b':'),
                new_window: crate::mux::config::ChordKey::plain(b'C'),
                resize: crate::mux::config::ChordKey::plain(b'R'),
                swap_prev: crate::mux::config::ChordKey::plain(b'{'),
                swap_next: crate::mux::config::ChordKey::plain(b'}'),
                move_window_left: crate::mux::config::ChordKey::plain(b'<'),
                move_window_right: crate::mux::config::ChordKey::plain(b'>'),
                tab_strip_left: crate::mux::config::ChordKey::plain(b'H'),
                tab_strip_right: crate::mux::config::ChordKey::plain(b'L'),
                workspace_next: crate::mux::config::ChordKey::plain(b'N'),
                workspace_prev: crate::mux::config::ChordKey::plain(b'P'),
                help: crate::mux::config::ChordKey::plain(b'?'),
                picker: crate::mux::config::ChordKey::plain(b'w'),
                zoom: crate::mux::config::ChordKey::plain(b'z'),
                rename_window: crate::mux::config::ChordKey::plain(b','),
                rename_pane: crate::mux::config::ChordKey::plain(b'$'),
                border_cycle: crate::mux::config::ChordKey::plain(b'B'),
                label_toggle: crate::mux::config::ChordKey::plain(b'l'),
                workspace_picker: crate::mux::config::ChordKey::plain(b'g'),
                sidebar: crate::mux::config::ChordKey::plain(b's'),
                status_bar: crate::mux::config::ChordKey::plain(b'S'),
                respawn: crate::mux::config::ChordKey::plain(b'r'),
                notify_jump: crate::mux::config::ChordKey::plain(b'j'),
                ..crate::mux::config::Management::default()
            },
            resize_step: 1,
            ..crate::mux::config::Chords::with_defaults()
        },
    )
    .expect("partial file");
    assert_eq!(
        kept,
        crate::mux::config::Chords {
            prefix: 0x02,
            reload: 0x12,
            management: crate::mux::config::Management {
                split_right: crate::mux::config::ChordKey::plain(b'&'),
                split_down: crate::mux::config::ChordKey::plain(b'*'),
                kill_pane: crate::mux::config::ChordKey::plain(b'X'),
                break_pane: crate::mux::config::ChordKey::plain(b'!'),
                join_pane: crate::mux::config::ChordKey::plain(b'J'),
                palette: crate::mux::config::ChordKey::plain(b':'),
                new_window: crate::mux::config::ChordKey::plain(b'C'),
                resize: crate::mux::config::ChordKey::plain(b'R'),
                swap_prev: crate::mux::config::ChordKey::plain(b'{'),
                swap_next: crate::mux::config::ChordKey::plain(b'}'),
                move_window_left: crate::mux::config::ChordKey::plain(b'<'),
                move_window_right: crate::mux::config::ChordKey::plain(b'>'),
                tab_strip_left: crate::mux::config::ChordKey::plain(b'H'),
                tab_strip_right: crate::mux::config::ChordKey::plain(b'L'),
                workspace_next: crate::mux::config::ChordKey::plain(b'N'),
                workspace_prev: crate::mux::config::ChordKey::plain(b'P'),
                help: crate::mux::config::ChordKey::plain(b'?'),
                picker: crate::mux::config::ChordKey::plain(b'w'),
                zoom: crate::mux::config::ChordKey::plain(b'z'),
                rename_window: crate::mux::config::ChordKey::plain(b','),
                rename_pane: crate::mux::config::ChordKey::plain(b'$'),
                border_cycle: crate::mux::config::ChordKey::plain(b'B'),
                label_toggle: crate::mux::config::ChordKey::plain(b'l'),
                workspace_picker: crate::mux::config::ChordKey::plain(b'g'),
                sidebar: crate::mux::config::ChordKey::plain(b's'),
                status_bar: crate::mux::config::ChordKey::plain(b'S'),
                respawn: crate::mux::config::ChordKey::plain(b'r'),
                notify_jump: crate::mux::config::ChordKey::plain(b'j'),
                ..crate::mux::config::Management::default()
            },
            resize_step: 1,
            ..crate::mux::config::Chords::with_defaults()
        }
    );
}
