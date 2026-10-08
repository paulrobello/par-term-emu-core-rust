//! Chord, prompt, menu, picker, and pointer-router suites.

use super::*;

/// A session whose status saw `$0` but whose session roster comes
/// back EMPTY (another client killed the last session before the
/// throttled refresh noticed): prefix `)` must be a no-op — no
/// select, the view unchanged — never a panic that skips the
/// terminal restore.
#[test]
fn switch_session_with_an_empty_roster_is_a_no_op() {
    let (rx, mut conn) = recording_conn("sw-empty");
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();
    session.status.session_id = Some("$0".to_string());
    drained(&rx);
    let mut prefix_pending = false;
    assert!(!session.route_plain(
        &[crate::mux::attach::C_B, b')'],
        &mut conn,
        &mut prefix_pending
    ));
    assert_eq!(session.window, "@0", "the view stays put");
    let lines = drained(&rx);
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("select-window") || l.starts_with("switch-client -t")),
        "nothing to select: {lines:?}"
    );
}

/// Split right: `split-window -t <focused> -h`, the view re-seeds
/// the window, and focus lands on the NEW pane the reply names —
/// the select-pane rides the wire AFTER the reseed (the reseed
/// resets focus to the first leaf). The `%layout-change` the size
/// report carries makes the fresh pane part of the layout.
#[test]
fn split_chord_reseeds_and_focuses_the_new_pane() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("split-window -t %1 -h".to_string(), "%3".to_string());
    reseed_replies(&mut replies, "@0 * main", "@0", "%1\n%3");
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 80x23".to_string(),
        "%layout-change @0 0000,80x23,0,0{40x23,0,0,1,39x23,41,0,3} 0000,80x23,0,0{40x23,0,0,1,39x23,41,0,3} *"
            .to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "split-r",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'%');
    let lines = drained(&rx);
    assert_eq!(lines[0], "split-window -t %1 -h");
    assert!(
        lines.contains(&"refresh-client -t %1 -C 80x23".to_string()),
        "the reseed reports the size: {lines:?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("select-pane -t %3"),
        "focus lands on the new pane last: {lines:?}"
    );
    assert_eq!(session.renderer.focused(), Some(3));
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![1, 3], "the reseed applied the fresh layout");
}

/// Split down omits `-h`; a daemon error reply leaves the view alone
/// (no reseed, no focus move).
#[test]
fn split_down_chord_refused_does_nothing_more() {
    let mut failing = std::collections::HashSet::new();
    failing.insert("split-window -t %1".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "split-d",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'"');
    assert_eq!(drained(&rx), vec!["split-window -t %1".to_string()]);
    assert_eq!(session.renderer.focused(), Some(1));
}

/// Kill with a survivor: the `*`-marked pane of `list-panes` takes
/// focus after the reseed. Kill with NO survivor (the last pane
/// died): nothing is reseeded and the status is marked stale so the
/// pump's refresh ends the view.
#[test]
fn kill_chord_lands_on_the_survivor_or_marks_the_view_stale() {
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * main", "@0", "%1 @0 - x\n%2 @0 * y");
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 80x23".to_string(),
        "%layout-change @0 0000,80x23,0,0,2 0000,80x23,0,0,2 *".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "kill-surv",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'x');
    let lines = drained(&rx);
    assert_eq!(lines[0], "kill-pane -t %1");
    assert_eq!(lines[1], "list-panes -t @0");
    assert_eq!(
        lines.last().map(String::as_str),
        Some("select-pane -t %2"),
        "the marked survivor takes focus: {lines:?}"
    );
    assert_eq!(session.renderer.focused(), Some(2));

    // No survivor: list-panes comes back empty.
    let (rx, mut conn, mut session) = two_pane_session("kill-last", FakeScript::default());
    session.status_dirty = false;
    chord(&mut session, &mut conn, b'x');
    assert_eq!(
        drained(&rx),
        vec![
            "kill-pane -t %1".to_string(),
            "list-panes -t @0".to_string()
        ],
        "no reseed, no select"
    );
    assert!(session.status_dirty, "the refresh ends the view");
}

/// Swap prev/next pick the layout-order neighbor and wrap; a single
/// pane has no neighbor and sends nothing.
#[test]
fn swap_chords_pick_the_layout_neighbor_and_wrap() {
    let (rx, mut conn, mut session) = two_pane_session("swap", FakeScript::default());
    chord(&mut session, &mut conn, b'}');
    assert_eq!(drained(&rx), vec!["swap-pane -s %1 -t %2".to_string()]);
    chord(&mut session, &mut conn, b'{');
    assert_eq!(
        drained(&rx),
        vec!["swap-pane -s %1 -t %2".to_string()],
        "prev from the first pane wraps to the last"
    );
    session.renderer.focus(2);
    chord(&mut session, &mut conn, b'}');
    assert_eq!(
        drained(&rx),
        vec!["swap-pane -s %2 -t %1".to_string()],
        "next from the last pane wraps to the first"
    );

    let (rx, mut conn) = recording_conn("swap-one");
    let mut single = WindowSession::new(80, 25);
    single
        .renderer
        .apply_layout(parse_layout("0000,80x23,0,0,1").expect("parses"));
    drained(&rx);
    chord(&mut single, &mut conn, b'}');
    assert!(drained(&rx).is_empty(), "one pane: nothing to swap");
}

/// New window: `new-window -t $0`, then select + reseed onto the id
/// the reply names. With no known session the chord sends nothing.
#[test]
fn new_window_chord_creates_selects_and_reseeds() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("new-window -t $0".to_string(), "@4".to_string());
    reseed_replies(&mut replies, "@0 - main\n@4 * fresh", "@4", "%9");
    let (rx, mut conn, mut session) = two_pane_session(
        "neww",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'c');
    let lines = drained(&rx);
    assert_eq!(lines[0], "new-window -t $0");
    assert_eq!(lines[1], "switch-client -t @4");
    assert!(
        lines.contains(&"refresh-client -t %9 -C 80x23".to_string()),
        "{lines:?}"
    );
    assert_eq!(session.window, "@4");
    assert_eq!(session.status.active_window.as_deref(), Some("@4"));

    let (rx, mut conn) = recording_conn("neww-none");
    let mut orphan = WindowSession::new(80, 25);
    drained(&rx);
    chord(&mut orphan, &mut conn, b'c');
    assert!(drained(&rx).is_empty(), "no session: nothing to create in");
}

/// Zoom toggles the cue on an ok reply (flash + ` Z |` head on the
/// status row) and back; a refused zoom leaves the cue alone.
#[cfg(unix)]
#[test]
fn zoom_chord_toggles_the_cue_only_on_success() {
    let (rx, mut conn, mut session) = two_pane_session("zoom", FakeScript::default());
    chord(&mut session, &mut conn, b'z');
    assert_eq!(drained(&rx), vec!["resize-pane -t %1 -Z".to_string()]);
    assert!(session.zoomed);
    assert_eq!(session.flash.as_deref(), Some("zoomed"));
    let row: String = session
        .status_row
        .diff()
        .iter()
        .map(|(_, _, c)| c.symbol().to_string())
        .collect();
    assert!(
        row.starts_with(" Z |"),
        "the zoom head leads the row: {row}"
    );
    chord(&mut session, &mut conn, b'z');
    assert!(!session.zoomed);
    assert_eq!(session.flash.as_deref(), Some("unzoomed"));

    let mut failing = std::collections::HashSet::new();
    failing.insert("resize-pane -t %1 -Z".to_string());
    let (_rx, mut conn, mut session) = two_pane_session(
        "zoom-no",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'z');
    assert!(!session.zoomed, "a refused zoom keeps the cue off");
    assert_eq!(session.flash, None);
}

/// The border chord cycles the glyph set; reaching herdr turns on
/// per-pane boxes at the SESSION level, so a reseed (a split is one)
/// keeps them — the regression the chord's comment names.
#[test]
fn border_cycle_reaches_herdr_boxes_that_survive_a_reseed() {
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * main", "@0", "%1");
    let (_rx, mut conn, mut session) = two_pane_session(
        "border",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    for expected in ["double", "heavy", "ascii"] {
        chord(&mut session, &mut conn, b'B');
        assert_eq!(session.border_glyphs.name(), expected);
        assert!(!session.pane_borders, "{expected}: shared dividers");
    }
    chord(&mut session, &mut conn, b'B');
    assert_eq!(session.border_glyphs, Glyphs::Herdr);
    assert!(session.pane_borders && session.renderer.pane_borders);
    assert_eq!(session.flash.as_deref(), Some("border style: herdr"));
    session.reseed_window(&mut conn, "@0");
    assert!(
        session.renderer.pane_borders,
        "the rebuilt renderer keeps the herdr boxes"
    );
    chord(&mut session, &mut conn, b'B');
    assert_eq!(session.border_glyphs, Glyphs::Unicode, "the cycle wraps");
    assert!(!session.renderer.pane_borders);
}

/// The labels chord flips the in-border titles with a flash; the
/// status-bar chord hides/shows the row and parks a grid refit; the
/// sidebar chord opens the panel (roster queried, refit parked) and
/// closes it without re-querying.
#[test]
fn toggle_chords_flip_their_state_and_park_refits() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-workspaces".to_string(),
        "+0: alpha active\n+1: beta".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "toggles",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'l');
    assert!(session.show_label_in_border);
    assert_eq!(session.flash.as_deref(), Some("labels on"));
    chord(&mut session, &mut conn, b'l');
    assert!(!session.show_label_in_border);
    assert_eq!(session.flash.as_deref(), Some("labels off"));

    chord(&mut session, &mut conn, b'S');
    assert!(!session.status_bar_on);
    assert!(session.pending_grid_refit);
    let mut sink = RecordingSink::default();
    assert!(
        !session.flush_status_row(&mut sink),
        "a hidden bar flushes nothing"
    );
    chord(&mut session, &mut conn, b'S');
    assert!(session.status_bar_on);
    session.pending_grid_refit = false;
    assert!(drained(&rx).is_empty(), "the local toggles touch no wire");

    chord(&mut session, &mut conn, b's');
    assert!(session.sidebar_on && session.pending_grid_refit);
    assert_eq!(session.renderer.sidebar_width(), 20);
    assert_eq!(drained(&rx), vec!["list-workspaces".to_string()]);
    assert_eq!(
        session.renderer.sidebar_row_at(2, 2),
        Some("ws:+1".to_string()),
        "the queried roster fills the panel"
    );
    chord(&mut session, &mut conn, b's');
    assert!(!session.sidebar_on);
    assert_eq!(session.renderer.sidebar_width(), 0);
    assert_eq!(session.flash.as_deref(), Some("sidebar off"));
    assert!(drained(&rx).is_empty(), "closing re-queries nothing");
}

/// The rename chords open the prompt seeded from the live name: `,`
/// the shown window's name, `$` the focused pane's title.
#[test]
fn rename_chords_open_seeded_prompts() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert("list-windows -t $0".to_string(), "@0 * editor".to_string());
    replies.insert("pane-title -t %1".to_string(), "build".to_string());
    let (_rx, mut conn, mut session) = two_pane_session(
        "rename",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    chord(&mut session, &mut conn, b',');
    assert!(session.prompt_mode);
    assert_eq!(
        session.prompt_target,
        PromptTarget::Window("@0".to_string())
    );
    assert_eq!(session.prompt_text, "editor");
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(crate::mux::attach::PROMPT_WINDOW_OVERLAY_TITLE)
    );
    session.leave_prompt();
    chord(&mut session, &mut conn, b'$');
    assert_eq!(session.prompt_target, PromptTarget::Pane);
    assert_eq!(session.prompt_text, "build");
}

/// Workspace next/prev: the active roster entry's neighbor (wrapping)
/// is selected and the view lands on its first session's active
/// window; a roster without an active marker does nothing.
#[test]
fn workspace_chords_select_the_neighbor_and_land_on_it() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-workspaces".to_string(),
        "+0: alpha active\n+1: beta".to_string(),
    );
    replies.insert(
        "list-sessions -t +1".to_string(),
        "+1: beta: $1: lab".to_string(),
    );
    replies.insert(
        "list-windows -t $1".to_string(),
        "@5 - a\n@6 * b".to_string(),
    );
    replies.insert("list-sessions".to_string(), "+1: beta: $1: lab".to_string());
    replies.insert("list-panes -t @6".to_string(), "%7".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "ws-next",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'W');
    let lines = drained(&rx);
    // The bare switch-client query answers empty here (an old daemon's
    // shape), so the landing falls back to the first listed session; the
    // landing itself rides switch-client -t (card 01a11bd1).
    assert_eq!(
        lines[..7].to_vec(),
        vec![
            "list-workspaces",
            "select-workspace -t +1",
            "select-workspace -t +1",
            "list-sessions -t +1",
            "switch-client",
            "list-windows -t $1",
            "switch-client -t @6",
        ],
        "{lines:?}"
    );
    assert_eq!(session.window, "@6", "the session's active window");
    assert_eq!(session.status.session_id.as_deref(), Some("$1"));
    // C-w from the active +0 wraps backward to the last (+1).
    chord(&mut session, &mut conn, 0x17);
    assert!(drained(&rx).contains(&"select-workspace -t +1".to_string()));

    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-workspaces".to_string(),
        "+0: alpha\n+1: beta".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "ws-noact",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'W');
    assert_eq!(drained(&rx), vec!["list-workspaces".to_string()]);
    assert_eq!(session.window, "@0");
}

/// Card 01a11bd1: a workspace landing takes the session the daemon
/// resumed (the bare `switch-client` answer), not the first listed one —
/// the session every follower is told to show.
#[test]
fn workspace_landing_takes_the_resumed_session_not_the_first_listed() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-sessions -t +1".to_string(),
        "+1: beta: $1: first\n+1: beta: $2: resumed".to_string(),
    );
    replies.insert("switch-client".to_string(), "$2".to_string());
    replies.insert("list-windows -t $2".to_string(), "@9 * r".to_string());
    replies.insert("list-panes -t @9".to_string(), "%9".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "ws-resumed",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.land_on_workspace(&mut conn, "+1");
    let lines = drained(&rx);
    assert!(
        lines.contains(&"list-windows -t $2".to_string()),
        "{lines:?}"
    );
    assert!(
        !lines.contains(&"list-windows -t $1".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"switch-client -t @9".to_string()),
        "{lines:?}"
    );
    assert_eq!(session.window, "@9");
}

/// Landing on a workspace with no sessions selects it but leaves the
/// view on its window and marks the status stale.
#[test]
fn landing_on_an_empty_workspace_only_marks_the_status_stale() {
    let (rx, mut conn, mut session) = two_pane_session("ws-empty", FakeScript::default());
    session.status_dirty = false;
    session.land_on_workspace(&mut conn, "+3");
    assert_eq!(
        drained(&rx),
        vec![
            "select-workspace -t +3".to_string(),
            "list-sessions -t +3".to_string(),
            "switch-client".to_string()
        ]
    );
    assert_eq!(session.window, "@0");
    assert!(session.status_dirty);
}

/// The workspace-picker chord lists the roster, opens on the active
/// row, and Enter on another row lands there; an empty roster opens
/// nothing.
#[test]
fn workspace_picker_opens_on_the_active_row_and_lands_on_enter() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-workspaces".to_string(),
        "+0: alpha\n+1: beta active\n+2: gamma".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "wspick",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'g');
    assert!(session.picker_mode);
    assert_eq!(session.picker_selected, 1, "opens on the active workspace");
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(crate::mux::attach::WORKSPACE_PICKER_OVERLAY_TITLE)
    );
    let text = overlay_text(&session).join("\n");
    assert!(
        text.contains(">+1  beta *"),
        "the active row is marked: {text}"
    );
    assert!(text.contains(" +2  gamma"), "{text}");
    drained(&rx);
    session.picker_byte(&mut conn, b'j');
    session.picker_byte(&mut conn, b'\r');
    assert!(!session.picker_mode && session.picker_workspaces.is_none());
    let lines = drained(&rx);
    assert_eq!(lines[0], "select-workspace -t +2", "{lines:?}");

    let (rx, mut conn, mut session) = two_pane_session("wspick-none", FakeScript::default());
    chord(&mut session, &mut conn, b'g');
    assert!(!session.picker_mode, "no workspaces: no picker");
    assert_eq!(drained(&rx), vec!["list-workspaces".to_string()]);
}

/// Each rename target's commit spelling: the pane title and the
/// workspace name ride `wire_quote` (their parsers take one quoted
/// word), the window name rides UNQUOTED (rename-window takes the
/// rest of the line verbatim). Every commit closes the prompt and
/// marks the status stale.
#[test]
fn prompt_commits_send_each_targets_wire_spelling() {
    let (rx, mut conn, mut session) = two_pane_session("prompt-ren", FakeScript::default());
    for (target, typed, wire) in [
        (
            PromptTarget::Pane,
            "my build",
            "select-pane -t %1 -T 'my build'",
        ),
        (
            PromptTarget::Window("@0".to_string()),
            "my notes",
            "rename-window -t @0 my notes",
        ),
        (
            PromptTarget::Workspace("+2".to_string()),
            "it's",
            "rename-workspace -t +2 'it'\\''s'",
        ),
    ] {
        session.enter_prompt(target.clone());
        session.status_dirty = false;
        type_and_commit(&mut session, &mut conn, typed);
        assert_eq!(drained(&rx), vec![wire.to_string()], "{target:?}");
        assert!(!session.prompt_mode, "{target:?}: the commit closes");
        assert_eq!(session.renderer.overlay, None);
        assert!(session.status_dirty, "{target:?}: the status re-queries");
    }
}

/// An empty or whitespace-only input cancels: nothing rides the wire
/// (an empty name is not expressible on it).
#[test]
fn whitespace_only_prompt_cancels_without_sending() {
    let (rx, mut conn, mut session) = two_pane_session("prompt-empty", FakeScript::default());
    session.enter_prompt(PromptTarget::Window("@0".to_string()));
    type_and_commit(&mut session, &mut conn, "   ");
    assert!(!session.prompt_mode);
    assert!(drained(&rx).is_empty());
}

/// The prompt's byte editing: printable bytes append, Backspace pops,
/// ^C clears, non-printable bytes are ignored, and Escape cancels
/// without sending; the overlay mirrors the buffer as it edits.
#[test]
fn prompt_bytes_edit_the_buffer_and_escape_cancels() {
    let (rx, mut conn, mut session) = two_pane_session("prompt-edit", FakeScript::default());
    session.enter_prompt(PromptTarget::Window("@0".to_string()));
    session.prompt_text.clear();
    for &b in b"abc" {
        assert!(session.prompt_byte(&mut conn, b));
    }
    assert!(session.prompt_byte(&mut conn, 0x7f));
    assert_eq!(session.prompt_text, "ab");
    assert!(
        overlay_text(&session).join("\n").contains("ab"),
        "the overlay mirrors the buffer"
    );
    assert!(session.prompt_byte(&mut conn, 0x01), "^A is ignored");
    assert_eq!(session.prompt_text, "ab");
    assert!(session.prompt_byte(&mut conn, 0x03));
    assert_eq!(session.prompt_text, "", "^C clears the input");
    session.prompt_byte(&mut conn, b'z');
    assert!(!session.prompt_byte(&mut conn, 0x1b), "Escape closes");
    assert!(!session.prompt_mode);
    assert!(drained(&rx).is_empty(), "a cancel sends nothing");
}

/// The prompt's key path: characters (including non-ASCII) append,
/// other keys are consumed without effect, Escape cancels; the mouse
/// is swallowed while it is up.
#[test]
fn prompt_keys_append_chars_and_swallow_the_mouse() {
    let (rx, mut conn, mut session) = two_pane_session("prompt-key", FakeScript::default());
    session.enter_prompt(PromptTarget::Pane);
    session.prompt_text.clear();
    session.prompt_key(&mut conn, &TermKeyEvent::char_('é', 0));
    session.prompt_key(&mut conn, &TermKeyEvent::functional(TermKey::Left, 0));
    assert_eq!(session.prompt_text, "é");
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 10,
            row: 6,
            release: false,
        },
    );
    assert_eq!(session.renderer.focused(), Some(1));
    assert!(session.prompt_mode, "the click did not dismiss it");
    assert!(drained(&rx).is_empty(), "the click selected nothing");
    session.prompt_key(&mut conn, &TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(!session.prompt_mode);
}

/// The new-tab prompt seeds the next free index; its commit creates
/// the window (quoted name), selects it, and lands the view there.
/// With no session known it cancels without sending.
#[test]
fn new_window_prompt_creates_and_lands_on_the_window() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("new-window -t $0 -n 'logs'".to_string(), "@3".to_string());
    reseed_replies(&mut replies, "@0 - 0\n@3 * logs", "@3", "%5");
    let (rx, mut conn, mut session) = two_pane_session(
        "prompt-neww",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    drained(&rx);
    session.enter_prompt(PromptTarget::NewWindow);
    assert_eq!(
        session.prompt_text, "4",
        "one past the highest window ordinal (@3)"
    );
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(crate::mux::attach::PROMPT_NEW_WINDOW_OVERLAY_TITLE)
    );
    type_and_commit(&mut session, &mut conn, "logs");
    let lines = drained(&rx);
    assert_eq!(lines[0], "new-window -t $0 -n 'logs'");
    assert_eq!(lines[1], "switch-client -t @3");
    assert_eq!(session.window, "@3");

    let (rx, mut conn) = recording_conn("prompt-neww-no");
    let mut orphan = WindowSession::new(80, 25);
    drained(&rx);
    orphan.enter_prompt(PromptTarget::NewWindow);
    type_and_commit(&mut orphan, &mut conn, "x");
    assert!(!orphan.prompt_mode);
    assert!(drained(&rx).is_empty());
}

/// The new-workspace prompt's commit creates the workspace and lands
/// on the id the reply names; a refused create closes the prompt and
/// lands nowhere.
#[test]
fn new_workspace_prompt_creates_and_lands_or_stops_on_error() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("new-workspace -n 'ops'".to_string(), "+4".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "prompt-newws",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.enter_prompt(PromptTarget::NewWorkspace);
    type_and_commit(&mut session, &mut conn, "ops");
    let lines = drained(&rx);
    assert_eq!(
        lines[..4].to_vec(),
        vec![
            "new-workspace -n 'ops'",
            "select-workspace -t +4",
            "select-workspace -t +4",
            "list-sessions -t +4",
        ]
    );

    let mut failing = std::collections::HashSet::new();
    failing.insert("new-workspace -n 'ops'".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "prompt-newws-no",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    session.enter_prompt(PromptTarget::NewWorkspace);
    type_and_commit(&mut session, &mut conn, "ops");
    assert_eq!(drained(&rx), vec!["new-workspace -n 'ops'".to_string()]);
    assert!(!session.prompt_mode);
}

/// The tab menu's close on a window the view is NOT showing: only
/// the kill rides the wire, and the status is marked stale.
#[test]
fn closing_an_unshown_tab_only_kills_it() {
    let (rx, mut conn, mut session) = two_pane_session("close-other", FakeScript::default());
    session.status_dirty = false;
    session.open_menu(MenuTarget::Tab("@7".to_string()));
    session.menu_click(&mut conn, 2);
    assert!(session.menu.is_none());
    assert_eq!(drained(&rx), vec!["kill-window -t @7".to_string()]);
    assert_eq!(session.window, "@0");
    assert!(session.status_dirty);
}

/// Closing the SHOWN tab lands on the session's `*`-marked survivor
/// (not merely the first window); closing the last window leaves
/// nothing to land on and marks the view stale instead.
#[test]
fn closing_the_shown_tab_lands_on_the_marked_survivor() {
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@1 - one\n@2 * two", "@2", "%4");
    let (rx, mut conn, mut session) = two_pane_session(
        "close-shown",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    session.menu_click(&mut conn, 2);
    let lines = drained(&rx);
    assert_eq!(
        lines[..3].to_vec(),
        vec![
            "kill-window -t @0",
            "list-windows -t $0",
            "switch-client -t @2"
        ]
    );
    assert_eq!(session.window, "@2");

    let (rx, mut conn, mut session) = two_pane_session("close-last", FakeScript::default());
    session.status_dirty = false;
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    session.menu_click(&mut conn, 2);
    assert_eq!(
        drained(&rx),
        vec![
            "kill-window -t @0".to_string(),
            "list-windows -t $0".to_string()
        ]
    );
    assert_eq!(session.window, "@0");
    assert!(session.status_dirty);
}

/// The workspace menu's close: when the shown session lives in the
/// killed workspace, the view lands on the first surviving one; when
/// it does not, only the kill rides; when none survive, the view is
/// marked stale.
#[test]
fn closing_a_workspace_lands_on_a_survivor_only_when_it_was_shown() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-sessions -t +0".to_string(),
        "+0: main: $0: work".to_string(),
    );
    replies.insert("list-workspaces".to_string(), "+1: beta active".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "closews-shown",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.open_menu(MenuTarget::Workspace("+0".to_string()));
    session.menu_click(&mut conn, 2);
    let lines = drained(&rx);
    assert_eq!(
        lines[..5].to_vec(),
        vec![
            "list-sessions -t +0",
            "kill-workspace -t +0",
            "list-workspaces",
            "select-workspace -t +1",
            "select-workspace -t +1",
        ]
    );

    // Another workspace's close: the shown session is not inside it.
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-sessions -t +5".to_string(),
        "+5: far: $9: else".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "closews-other",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status_dirty = false;
    session.open_menu(MenuTarget::Workspace("+5".to_string()));
    session.menu_click(&mut conn, 2);
    assert_eq!(
        drained(&rx),
        vec![
            "list-sessions -t +5".to_string(),
            "kill-workspace -t +5".to_string()
        ]
    );
    assert!(session.status_dirty);

    // The last workspace: nothing survives to land on.
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-sessions -t +0".to_string(),
        "+0: main: $0: work".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "closews-last",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status_dirty = false;
    session.open_menu(MenuTarget::Workspace("+0".to_string()));
    session.menu_click(&mut conn, 2);
    assert_eq!(
        drained(&rx),
        vec![
            "list-sessions -t +0".to_string(),
            "kill-workspace -t +0".to_string(),
            "list-workspaces".to_string()
        ]
    );
    assert!(session.status_dirty);
    assert_eq!(session.window, "@0");
}

/// The tab and workspace menus' remaining rows open their prompts
/// with the right target and seed; keys/bytes other than q/Escape
/// leave the menu up.
#[test]
fn menu_rows_open_their_prompts_and_q_or_escape_close() {
    let (rx, mut conn, mut session) = two_pane_session("menu-rows", FakeScript::default());
    for (target, row, expected) in [
        (
            MenuTarget::Tab("@9".to_string()),
            1,
            PromptTarget::Window("@9".to_string()),
        ),
        (
            MenuTarget::Tab("@9".to_string()),
            3,
            PromptTarget::NewWindow,
        ),
        (
            MenuTarget::Workspace("+2".to_string()),
            1,
            PromptTarget::Workspace("+2".to_string()),
        ),
        (
            MenuTarget::Workspace("+2".to_string()),
            3,
            PromptTarget::NewWorkspace,
        ),
    ] {
        session.open_menu(target.clone());
        session.menu_click(&mut conn, row);
        assert!(session.menu.is_none());
        assert!(session.prompt_mode, "{target:?} row {row}");
        assert_eq!(session.prompt_target, expected);
        session.leave_prompt();
    }
    assert!(drained(&rx).is_empty(), "opening prompts sends nothing");

    session.open_menu(MenuTarget::Tab("@0".to_string()));
    assert_eq!(
        overlay_text(&session)[0],
        " @0 ",
        "an unknown window's menu names its id"
    );
    session.menu_click(&mut conn, 9);
    assert!(session.menu.is_some(), "an off-panel row is consumed");
    assert!(session.menu_byte(b'x'), "other bytes leave it up");
    session.menu_key(&TermKeyEvent::functional(TermKey::Down, 0));
    assert!(session.menu.is_some());
    session.menu_key(&TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(session.menu.is_none());
    session.open_menu(MenuTarget::Commands);
    let mut prefix_pending = false;
    session.route_plain(b"aq", &mut conn, &mut prefix_pending);
    assert!(session.menu.is_none(), "q in a plain run closes the menu");
    assert!(drained(&rx).is_empty(), "nothing leaked into the pane");
}

/// prefix n / p: the next/previous window of the shown session,
/// wrapping at both ends; a refused select leaves the view put; a
/// view whose window vanished from the list moves nowhere.
#[test]
fn window_switch_chords_wrap_and_respect_a_refused_select() {
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * a\n@1 - b\n@2 - c", "@2", "%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "win-sw",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'p');
    let lines = drained(&rx);
    assert_eq!(
        lines[..2].to_vec(),
        vec!["list-windows -t $0", "switch-client -t @2"],
        "p from the first window wraps to the last"
    );
    assert_eq!(session.window, "@2");
    chord(&mut session, &mut conn, b'n');
    assert_eq!(
        drained(&rx)[1],
        "switch-client -t @0",
        "n from the last wraps to the first"
    );

    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 * a\n@1 - b".to_string(),
    );
    // Both landing spellings refused (the window is gone): the
    // switch-client attempt, then the select-window fallback an older
    // daemon answers — and no reseed.
    let mut failing = std::collections::HashSet::new();
    failing.insert("switch-client -t @1".to_string());
    failing.insert("select-window -t @1".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "win-sw-no",
        FakeScript {
            replies,
            failing,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'n');
    assert_eq!(
        drained(&rx),
        vec![
            "list-windows -t $0".to_string(),
            "switch-client -t @1".to_string(),
            "select-window -t @1".to_string()
        ],
        "a refused landing reseeds nothing"
    );
    assert_eq!(session.window, "@0");

    session.window = "@8".to_string();
    chord(&mut session, &mut conn, b'n');
    assert_eq!(
        drained(&rx),
        vec!["list-windows -t $0".to_string()],
        "an unlisted shown window: nowhere to move from"
    );
}

/// prefix ( / ): the neighbor session in roster order (wrapping),
/// landing on its `*` window; a stale shown window (not in its
/// session any more) falls back to the roster head.
#[test]
fn session_switch_chords_land_on_the_neighbors_active_window() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-sessions".to_string(),
        "$0: work\n$1: lab\n$2: ops".to_string(),
    );
    replies.insert("list-windows -t $0".to_string(), "@0 * a".to_string());
    replies.insert(
        "list-windows -t $2".to_string(),
        "@7 - x\n@8 * y".to_string(),
    );
    replies.insert("list-windows -t $1".to_string(), "@4 - only".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "sess-sw",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'(');
    let lines = drained(&rx);
    assert_eq!(
        lines[..4].to_vec(),
        vec![
            "list-sessions",
            "list-windows -t $0",
            "list-windows -t $2",
            "switch-client -t @8"
        ],
        "( from the head wraps to the last session's active window"
    );
    assert_eq!(session.window, "@8");

    // A stale state: the shown window is not in $0's list, so the
    // head ($0) is the base and ) moves to $1, whose only window is
    // unmarked — the first window is the landing.
    session.window = "@99".to_string();
    session.status.session_id = Some("$0".to_string());
    chord(&mut session, &mut conn, b')');
    let lines = drained(&rx);
    assert_eq!(lines[2], "list-windows -t $1", "{lines:?}");
    assert_eq!(lines[3], "switch-client -t @4");
}

/// prefix o cycles focus through the layout order (wrapping) and
/// tells the daemon; a single pane does nothing.
#[test]
fn cycle_chord_walks_the_layout_and_wraps() {
    let (rx, mut conn, mut session) = two_pane_session("cycle", FakeScript::default());
    chord(&mut session, &mut conn, b'o');
    assert_eq!(session.renderer.focused(), Some(2));
    chord(&mut session, &mut conn, b'o');
    assert_eq!(session.renderer.focused(), Some(1), "wraps to the first");
    assert_eq!(
        drained(&rx),
        vec![
            "select-pane -t %2".to_string(),
            "select-pane -t %1".to_string()
        ]
    );
}

/// The plain-byte router's own cases: a literal prefix (prefix
/// prefix) forwards one prefix byte, an unbound chord key is
/// consumed, typed bytes forward as hex `send-keys`, `d` detaches,
/// and resize mode swallows a typed run while leaving the mode.
#[test]
fn plain_router_forwards_literals_and_consumes_unbound_keys() {
    let (rx, mut conn, mut session) = two_pane_session("plain", FakeScript::default());
    let mut pending = false;
    assert!(!session.route_plain(
        &[crate::mux::attach::C_B, crate::mux::attach::C_B, b'a'],
        &mut conn,
        &mut pending
    ));
    assert_eq!(drained(&rx), vec!["send-keys -t %1 -H 02 61".to_string()]);
    assert!(!session.route_plain(&[crate::mux::attach::C_B, b'Q'], &mut conn, &mut pending));
    assert!(drained(&rx).is_empty(), "an unbound chord key is consumed");
    // A prefix at the END of one run completes in the next run.
    assert!(!session.route_plain(&[crate::mux::attach::C_B], &mut conn, &mut pending));
    assert!(pending);
    assert!(
        session.route_plain(b"d", &mut conn, &mut pending),
        "d detaches"
    );

    session.resize_mode = true;
    assert!(!session.route_plain(b"xyz", &mut conn, &mut pending));
    assert!(!session.resize_mode, "a typed run leaves resize mode");
    assert!(drained(&rx).is_empty(), "and nothing leaks into the pane");
}

/// prefix + arrow focuses the neighbor in that direction (an edge
/// sends nothing); Shift+arrow swaps with it and flashes, and at an
/// edge flashes "no pane in that direction" without a swap. While
/// zoomed the cached daemon layout is the geometry, so an arrow can
/// aim at a pane the zoomed renderer does not even hold.
#[test]
fn prefix_arrows_navigate_swap_and_aim_through_a_zoom() {
    let (rx, mut conn, mut session) = two_pane_session("arrows", FakeScript::default());
    let right = TermKeyEvent::functional(TermKey::Right, 0);
    let left = TermKeyEvent::functional(TermKey::Left, 0);
    session.prefix_pane_arrow(&mut conn, &left);
    assert!(drained(&rx).is_empty(), "the left edge: nowhere to go");
    session.prefix_pane_arrow(&mut conn, &right);
    assert_eq!(session.renderer.focused(), Some(2));
    assert_eq!(drained(&rx), vec!["select-pane -t %2".to_string()]);
    session.prefix_pane_arrow(
        &mut conn,
        &TermKeyEvent::functional(TermKey::Right, crate::keyboard::modifiers::ALT),
    );
    assert!(drained(&rx).is_empty(), "a non-shift modifier is ignored");

    let shift = crate::keyboard::modifiers::SHIFT;
    session.prefix_pane_arrow(&mut conn, &TermKeyEvent::functional(TermKey::Right, shift));
    assert!(drained(&rx).is_empty());
    assert_eq!(session.flash.as_deref(), Some("no pane in that direction"));
    session.prefix_pane_arrow(&mut conn, &TermKeyEvent::functional(TermKey::Left, shift));
    assert_eq!(drained(&rx), vec!["swap-pane -s %2 -t %1".to_string()]);
    assert_eq!(session.flash.as_deref(), Some("swapped %2 with %1"));

    // Zoomed onto pane 1: the renderer holds only the zoomed rect,
    // the cached tree still knows pane 2 to the right.
    session.daemon_layout = session.renderer.layout().to_vec();
    session
        .renderer
        .apply_layout(parse_layout("0000,80x23,0,0,1").expect("parses"));
    session.renderer.focus(1);
    session.zoomed = true;
    session.prefix_pane_arrow(&mut conn, &right);
    assert_eq!(
        drained(&rx),
        vec!["select-pane -t %2".to_string()],
        "the arrow leaves the zoom toward the cached neighbor"
    );
}

/// Scroll mode's keys: Up/PgUp scroll back, Down returns, Home jumps
/// to the top of history, and End or q leave the mode back at live.
#[test]
fn scroll_mode_keys_move_the_viewport_and_exit() {
    let (_rx, _conn, mut session) = two_pane_session("scrollkeys", FakeScript::default());
    for i in 0..60 {
        session
            .renderer
            .feed_output(1, format!("line{i}\r\n").as_bytes());
    }
    let history = session
        .renderer
        .pane_terminal(1)
        .expect("pane")
        .active_grid()
        .scrollback_len();
    assert!(history > 0);
    assert!(session.renderer.enter_scroll_mode(1));
    session.scroll_mode = true;
    let start = session.renderer.scroll_offset_of(1);
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::Up, 0));
    assert_eq!(session.renderer.scroll_offset_of(1), start + 1);
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::Down, 0));
    assert_eq!(session.renderer.scroll_offset_of(1), start);
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::PageDown, 0));
    assert_eq!(
        session.renderer.scroll_offset_of(1),
        0,
        "a page down to live"
    );
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::PageUp, 0));
    assert_eq!(
        session.renderer.scroll_offset_of(1),
        start,
        "a page is the pane height"
    );
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::Home, 0));
    assert_eq!(
        session.renderer.scroll_offset_of(1),
        history,
        "the top of history"
    );
    session.scroll_mode_key(&TermKeyEvent::functional(TermKey::End, 0));
    assert!(!session.scroll_mode);
    assert!(!session.renderer.scroll_mode_active(1));
    assert_eq!(session.renderer.scroll_offset_of(1), 0);

    assert!(session.renderer.enter_scroll_mode(1));
    session.scroll_mode = true;
    session.scroll_mode_key(&TermKeyEvent::char_('q', 0));
    assert!(!session.scroll_mode, "q leaves too");
}

/// prefix [ enters scroll mode only when the focused pane has
/// history; in the mode, typed bytes never reach the pane and q
/// leaves it.
#[test]
fn scroll_chord_needs_history_and_owns_typed_bytes() {
    let (rx, mut conn, mut session) = two_pane_session("scrollchord", FakeScript::default());
    chord(&mut session, &mut conn, b'[');
    assert!(!session.scroll_mode, "no history: nothing to scroll");
    for i in 0..40 {
        session
            .renderer
            .feed_output(1, format!("l{i}\r\n").as_bytes());
    }
    chord(&mut session, &mut conn, b'[');
    assert!(session.scroll_mode);
    let mut pending = false;
    session.route_plain(b"abc", &mut conn, &mut pending);
    assert!(session.scroll_mode);
    assert!(drained(&rx).is_empty(), "typed bytes stay out of the pane");
    session.route_plain(b"q", &mut conn, &mut pending);
    assert!(!session.scroll_mode);
}

/// The help panel's key path: j/k and arrows scroll (clamped at the
/// top), `/` opens the filter that typed chars extend, a non-char
/// key commits the filter, and an unbound key closes the panel.
#[test]
fn help_keys_scroll_filter_and_close() {
    let (_rx, _conn, mut session) = two_pane_session("helpkeys", FakeScript::default());
    session.enter_help();
    session.help_key(&TermKeyEvent::char_('k', 0));
    assert_eq!(session.help_scroll, 0, "clamped at the top");
    session.help_key(&TermKeyEvent::char_('j', 0));
    session.help_key(&TermKeyEvent::functional(TermKey::Down, 0));
    assert_eq!(session.help_scroll, 2);
    session.help_key(&TermKeyEvent::functional(TermKey::Up, 0));
    session.help_key(&TermKeyEvent::functional(TermKey::PageDown, 0));
    assert_eq!(session.help_scroll, 11);
    session.help_key(&TermKeyEvent::functional(TermKey::PageUp, 0));
    assert_eq!(session.help_scroll, 1);
    session.help_key(&TermKeyEvent::char_('/', 0));
    assert!(session.help_filtering);
    for c in "zoom".chars() {
        session.help_key(&TermKeyEvent::char_(c, 0));
    }
    assert_eq!(session.help_filter, "zoom");
    assert!(overlay_text(&session).join("\n").contains("zoom"));
    session.help_key(&TermKeyEvent::functional(TermKey::Down, 0));
    assert!(!session.help_filtering, "a non-char key commits the filter");
    assert_eq!(session.help_filter, "zoom", "the filter is kept");
    assert!(session.help_mode);
    session.help_key(&TermKeyEvent::functional(TermKey::Tab, 0));
    assert!(!session.help_mode, "an unbound key closes");
    assert_eq!(session.renderer.overlay, None);

    session.enter_help();
    session.help_key(&TermKeyEvent::char_('/', 0));
    session.help_key(&TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(!session.help_mode, "Escape in the filter closes the panel");
    session.enter_help();
    session.help_byte(b'/');
    session.help_byte(b'a');
    session.help_byte(0x7f);
    assert_eq!(session.help_filter, "", "Backspace pops the filter");
    session.help_byte(b'\r');
    assert!(!session.help_filtering && session.help_mode);
}

/// The picker's key and byte paths: j/k/arrows move (wrapping), `/`
/// filters as you type (Backspace pops, Enter commits, Escape
/// closes), and an unbound key closes.
#[test]
fn picker_keys_move_filter_and_close() {
    let replies = std::collections::HashMap::from([
        ("list-sessions".to_string(), "$0: work\n$1: lab".to_string()),
        ("list-windows -t $0".to_string(), "@0 * main".to_string()),
        ("list-windows -t $1".to_string(), "@2 * logs".to_string()),
    ]);
    let (_rx, mut conn, mut session) = two_pane_session(
        "pickkeys",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'w');
    assert!(session.picker_mode);
    let count = session.picker_refs.len();
    assert_eq!(count, 4, "two sessions with one window each");
    assert_eq!(session.picker_selected, 0, "opens on the current session");
    session.picker_key(&mut conn, &TermKeyEvent::functional(TermKey::Up, 0));
    assert_eq!(session.picker_selected, count - 1, "Up wraps to the bottom");
    session.picker_key(&mut conn, &TermKeyEvent::functional(TermKey::Down, 0));
    session.picker_key(&mut conn, &TermKeyEvent::char_('j', 0));
    session.picker_key(&mut conn, &TermKeyEvent::char_('k', 0));
    assert_eq!(session.picker_selected, 0);
    session.picker_key(&mut conn, &TermKeyEvent::char_('/', 0));
    for c in "logs".chars() {
        session.picker_key(&mut conn, &TermKeyEvent::char_(c, 0));
    }
    assert_eq!(session.picker_filter, "logs");
    let text = overlay_text(&session).join("\n");
    assert!(text.contains("logs") && !text.contains("main"), "{text}");
    session.picker_key(&mut conn, &TermKeyEvent::functional(TermKey::Down, 0));
    assert!(!session.picker_filtering, "a non-char key commits");
    session.picker_byte(&mut conn, b'/');
    session.picker_byte(&mut conn, 0x7f);
    assert_eq!(session.picker_filter, "log");
    session.picker_byte(&mut conn, b'\r');
    assert!(!session.picker_filtering && session.picker_mode);
    session.picker_key(&mut conn, &TermKeyEvent::functional(TermKey::Tab, 0));
    assert!(!session.picker_mode, "an unbound key closes");

    chord(&mut session, &mut conn, b'w');
    session.picker_byte(&mut conn, b'/');
    assert!(!session.picker_byte(&mut conn, 0x1b), "Escape closes");
    assert!(!session.picker_mode);
    chord(&mut session, &mut conn, b'w');
    session.picker_key(&mut conn, &TermKeyEvent::char_('/', 0));
    session.picker_key(&mut conn, &TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(!session.picker_mode);
}

/// Coordinates SGR never sends (column or row 0) are dropped
/// outright, and a press in the content area focuses the pane under
/// the pointer with select-pane (no forwarding: the pane does not
/// track the mouse).
#[test]
fn mouse_press_focuses_and_zero_coordinates_drop() {
    let (rx, mut conn, mut session) = two_pane_session("m-press", FakeScript::default());
    session.route_mouse(&mut conn, sgr(0, 0, 5, false));
    session.route_mouse(&mut conn, sgr(0, 60, 0, false));
    assert!(drained(&rx).is_empty(), "zero coordinates are dropped");
    // Host col 61 (x 60), host row 6 (content row 4): pane 2.
    session.route_mouse(&mut conn, sgr(0, 61, 6, false));
    assert_eq!(session.renderer.focused(), Some(2));
    assert_eq!(drained(&rx), vec!["select-pane -t %2".to_string()]);
    // A release/motion over a pane that does not own the mouse does
    // nothing; neither does a press past the layout's last row.
    session.route_mouse(&mut conn, sgr(0, 61, 6, true));
    session.route_mouse(&mut conn, sgr(32, 10, 6, false));
    // (The 24-row daemon layout overhangs the 23-row renderer, so
    // the first row past it is host row 26.)
    session.route_mouse(&mut conn, sgr(0, 10, 26, false));
    assert!(drained(&rx).is_empty());
    assert_eq!(session.renderer.focused(), Some(2));
}

/// A pane that tracks the mouse gets its events re-encoded as
/// pane-relative SGR over `send-keys -H`: the press (after the focus
/// select), the release, and the wheel — which then does NOT scroll
/// the client's scrollback.
#[test]
fn mouse_owning_pane_receives_rebased_sgr_reports() {
    let (rx, mut conn, mut session) = two_pane_session("m-fwd", FakeScript::default());
    session.renderer.feed_output(2, b"\x1b[?1000h\x1b[?1006h");
    // Host (61, 6) 1-based → x 60, content row 4 → pane 2 origin
    // x 40: pane-relative (20, 4) → SGR `ESC[<0;21;5M`.
    session.route_mouse(&mut conn, sgr(0, 61, 6, false));
    let hex = |s: &str| {
        s.bytes()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(
        drained(&rx),
        vec![
            "select-pane -t %2".to_string(),
            format!("send-keys -t %2 -H {}", hex("\x1b[<0;21;5M"))
        ]
    );
    session.route_mouse(&mut conn, sgr(0, 61, 6, true));
    assert_eq!(
        drained(&rx),
        vec![format!("send-keys -t %2 -H {}", hex("\x1b[<0;21;5m"))]
    );
    for i in 0..30 {
        session
            .renderer
            .feed_output(2, format!("h{i}\r\n").as_bytes());
    }
    session.route_mouse(&mut conn, sgr(64, 61, 6, false));
    assert_eq!(
        drained(&rx),
        vec![format!("send-keys -t %2 -H {}", hex("\x1b[<64;21;5M"))]
    );
    assert_eq!(
        session.renderer.scroll_offset_of(2),
        0,
        "the owning pane's wheel is the app's, not the scrollback's"
    );
}

/// The side panel's pointer: a left press on a workspace row lands
/// on it, a RIGHT press opens that workspace's menu, the `new` chip
/// opens the new-workspace prompt, and a release in the panel does
/// nothing. A click on the TOP row inside the panel's width is the
/// tab strip's, not the panel's.
#[test]
fn sidebar_pointer_lands_opens_menus_and_prompts() {
    let (rx, mut conn, mut session) = two_pane_session("m-side", FakeScript::default());
    session.renderer.set_sidebar_width(20);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            rows: vec![
                ("ws:+0".to_string(), "alpha".to_string(), true),
                ("ws:+1".to_string(), "beta".to_string(), false),
            ],
        }]));
    // Host row 2 (1-based 3) is the second workspace row.
    session.route_mouse(&mut conn, sgr(2, 3, 3, false));
    assert_eq!(
        session.menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::Workspace("+1".to_string()))
    );
    assert!(drained(&rx).is_empty(), "opening the menu sends nothing");
    session.leave_menu();
    session.route_mouse(&mut conn, sgr(0, 3, 3, true));
    assert!(drained(&rx).is_empty(), "a release does nothing");
    session.route_mouse(&mut conn, sgr(0, 3, 3, false));
    assert_eq!(
        drained(&rx)[..2].to_vec(),
        vec!["select-workspace -t +1", "list-sessions -t +1"],
        "a left press lands on the workspace"
    );
    // The ` new ` chip: the 23-row renderer's footer row is host
    // row 23 (SGR 24); col 2.
    session.route_mouse(&mut conn, sgr(0, 3, 24, false));
    assert!(session.prompt_mode);
    assert_eq!(session.prompt_target, PromptTarget::NewWorkspace);
}

/// The tab strip's right press opens that tab's context menu; a
/// right press on a gap opens nothing; wheel and release on the strip
/// do nothing; the `+` button opens the new-tab prompt.
#[test]
fn tab_strip_right_press_opens_the_tab_menu() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 * main\n@1 - vim".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "m-tabmenu",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    drained(&rx);
    session.route_mouse(&mut conn, sgr(2, 9, 1, false));
    assert!(session.menu.is_none(), "the gap column opens nothing");
    session.route_mouse(&mut conn, sgr(2, 13, 1, false));
    assert_eq!(
        session.menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::Tab("@1".to_string()))
    );
    assert_eq!(overlay_text(&session)[0], " vim ", "the menu names the tab");
    session.leave_menu();
    session.route_mouse(&mut conn, sgr(64, 13, 1, false));
    session.route_mouse(&mut conn, sgr(0, 13, 1, true));
    assert_eq!(session.window, "@0", "wheel/release never switch");
    // Clicking the shown tab is a no-op.
    session.route_mouse(&mut conn, sgr(0, 3, 1, false));
    assert!(drained(&rx).is_empty());
    let plus = (0..80u16)
        .find(|x| session.tab_strip.plus_hit(*x))
        .expect("the strip paints a + button");
    session.route_mouse(&mut conn, sgr(0, plus + 1, 1, false));
    assert!(session.prompt_mode);
    assert_eq!(session.prompt_target, PromptTarget::NewWindow);
}

/// The pickers are modal for the pointer: wheels move the selection,
/// a click on a content row activates it, the footer and drags do
/// nothing, and while filtering a click on the filter line re-opens
/// the filter box.
#[test]
fn picker_pointer_wheels_move_and_a_click_activates() {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-workspaces".to_string(),
        "+0: alpha active\n+1: beta\n+2: gamma".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "m-picker",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    chord(&mut session, &mut conn, b'g');
    drained(&rx);
    session.route_mouse(&mut conn, sgr(65, 40, 10, false));
    assert_eq!(session.picker_selected, 1, "wheel down moves down");
    session.route_mouse(&mut conn, sgr(64, 40, 10, false));
    assert_eq!(session.picker_selected, 0, "wheel up moves up");
    session.route_mouse(&mut conn, sgr(32, 40, 10, false));
    session.route_mouse(&mut conn, sgr(0, 40, 10, true));
    assert!(session.picker_mode, "drags and releases do nothing");

    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("picker up");
    // The footer (the panel's last row; panel row r sits at SGR row
    // y0 + 3 + r): consumed.
    let footer = session.picker_panel_len - 1;
    session.route_mouse(
        &mut conn,
        sgr(0, (x0 + 4) as u16, (y0 + 3 + footer) as u16, false),
    );
    assert!(session.picker_mode, "the footer does nothing");
    assert!(drained(&rx).is_empty());

    // Filtering: panel row 0 is the filter line.
    session.picker_byte(&mut conn, b'/');
    session.picker_byte(&mut conn, b'a');
    session.picker_byte(&mut conn, b'\r');
    assert!(!session.picker_filtering);
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("picker up");
    session.route_mouse(&mut conn, sgr(0, (x0 + 4) as u16, (y0 + 3) as u16, false));
    assert!(session.picker_filtering, "the filter line re-opens the box");
    assert!(drained(&rx).is_empty());
    // "a" keeps all three rows: panel row 3 is the third content
    // row (gamma).
    session.route_mouse(&mut conn, sgr(0, (x0 + 4) as u16, (y0 + 6) as u16, false));
    assert!(!session.picker_mode, "a content click activates");
    assert_eq!(drained(&rx)[0], "select-workspace -t +2");
}

/// With a context menu up, wheels, motion and releases are swallowed
/// and a press outside the panel is consumed without dispatching.
#[test]
fn open_menu_swallows_non_press_pointer_events() {
    let (rx, mut conn, mut session) = two_pane_session("m-menu", FakeScript::default());
    session.open_menu(MenuTarget::Commands);
    for event in [
        sgr(64, 40, 12, false),
        sgr(32, 40, 12, false),
        sgr(0, 40, 12, true),
        sgr(0, 1, 2, false),
    ] {
        session.route_mouse(&mut conn, event);
    }
    assert!(session.menu.is_some(), "still up");
    assert!(!session.detach_requested && !session.help_mode);
    assert!(drained(&rx).is_empty());
    assert_eq!(session.renderer.focused(), Some(1), "no click-through");
}

/// The help panel owns the pointer: the wheel scrolls the panel (up
/// clamped at 0) and a press neither focuses nor closes.
#[test]
fn help_panel_pointer_scrolls_and_swallows_presses() {
    let (rx, mut conn, mut session) = two_pane_session("m-help", FakeScript::default());
    session.enter_help();
    session.route_mouse(&mut conn, sgr(65, 61, 6, false));
    assert_eq!(session.help_scroll, 3);
    session.route_mouse(&mut conn, sgr(64, 61, 6, false));
    session.route_mouse(&mut conn, sgr(64, 61, 6, false));
    assert_eq!(session.help_scroll, 0);
    session.route_mouse(&mut conn, sgr(0, 61, 6, false));
    assert!(session.help_mode);
    assert_eq!(session.renderer.focused(), Some(1));
    assert!(drained(&rx).is_empty());
}

/// While a divider drag is held the wheel waits, and a drag that
/// wanders onto the tab-strip row still ends on release.
#[test]
fn drag_owns_the_pointer_until_release_even_on_the_strip_row() {
    let (rx, mut conn, mut session) = two_pane_session("m-drag", FakeScript::default());
    session.route_mouse(&mut conn, sgr(0, 40, 6, false));
    assert!(matches!(session.drag, Some(DragState::Pending { .. })));
    session.route_mouse(&mut conn, sgr(64, 10, 6, false));
    assert!(session.drag.is_some(), "the wheel waits for the drag");
    session.route_mouse(&mut conn, sgr(0, 42, 1, true));
    assert!(session.drag.is_none(), "release on the strip row ends it");
    // A motionless drag is a click: no resize, the release point's
    // pane (content row 0 under the strip row) takes focus.
    assert_eq!(drained(&rx), vec!["select-pane -t %2".to_string()]);
}

// ---- ARC-001: the pure hit-testers behind route_mouse / route_plain.
// Each Hit and PrefixChord variant pinned against the state that
// produces it; the router suites above pin the effects.

/// The zero coordinates, the menu modal, and the sidebar strip.
#[test]
fn mouse_hit_maps_zero_coordinates_menu_and_sidebar() {
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    assert_eq!(session.mouse_hit(&sgr(0, 0, 5, false)), Hit::Consumed);
    assert_eq!(session.mouse_hit(&sgr(0, 5, 0, false)), Hit::Consumed);

    session.open_menu(MenuTarget::Commands);
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("menu up");
    // Panel row r sits at SGR row y0 + 3 + r, SGR col x0 + 2 is inside.
    let (col, row) = ((x0 + 2) as u16, (y0 + 3 + 1) as u16);
    assert_eq!(session.mouse_hit(&sgr(0, col, row, false)), Hit::MenuRow(1));
    assert_eq!(
        session.mouse_hit(&sgr(0, col, row, true)),
        Hit::Consumed,
        "a release on a menu row is consumed"
    );
    assert_eq!(
        session.mouse_hit(&sgr(0, 1, 2, false)),
        Hit::Consumed,
        "off the panel"
    );
    session.leave_menu();

    session.renderer.set_sidebar_width(20);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            rows: vec![("ws:+1".to_string(), "beta".to_string(), false)],
        }]));
    // Host row 1 (SGR row 2) is the first workspace row.
    assert_eq!(
        session.mouse_hit(&sgr(0, 3, 2, false)),
        Hit::SidebarWorkspace {
            id: "+1".to_string(),
            menu: false
        }
    );
    assert_eq!(
        session.mouse_hit(&sgr(2, 3, 2, false)),
        Hit::SidebarWorkspace {
            id: "+1".to_string(),
            menu: true
        },
        "a right press opens the workspace menu"
    );
    assert_eq!(
        session.mouse_hit(&sgr(0, 3, 2, true)),
        Hit::Consumed,
        "a release in the strip is consumed"
    );
    // The footer chips on the renderer's last row (SGR row 24 here).
    assert_eq!(
        session.mouse_hit(&sgr(0, 3, 24, false)),
        Hit::SidebarNewWorkspace
    );
    assert_eq!(session.mouse_hit(&sgr(0, 17, 24, false)), Hit::SidebarMenu);
}

/// Drags in flight, the prompt, the picker, and the help panel.
#[test]
fn mouse_hit_maps_drags_and_the_pointer_modals() {
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    let divider = session.renderer.divider_near(40, 4, 1).expect("divider");
    session.drag = Some(DragState::Pending {
        divider,
        x: 40,
        y: 4,
    });
    // Motion on the strip row saturates to content row 0.
    assert_eq!(
        session.mouse_hit(&sgr(32, 43, 1, false)),
        Hit::Drag {
            x: 42,
            y: 0,
            release: false
        }
    );
    assert_eq!(
        session.mouse_hit(&sgr(0, 43, 6, true)),
        Hit::Drag {
            x: 42,
            y: 4,
            release: true
        }
    );
    assert_eq!(
        session.mouse_hit(&sgr(64, 10, 6, false)),
        Hit::Consumed,
        "a wheel waits while the button is held"
    );
    session.drag = None;

    session.prompt_mode = true;
    assert_eq!(session.mouse_hit(&sgr(0, 10, 6, false)), Hit::Consumed);
    session.prompt_mode = false;

    session.help_mode = true;
    assert_eq!(
        session.mouse_hit(&sgr(64, 10, 6, false)),
        Hit::HelpScroll(-3)
    );
    assert_eq!(
        session.mouse_hit(&sgr(65, 10, 6, false)),
        Hit::HelpScroll(3)
    );
    assert_eq!(session.mouse_hit(&sgr(0, 10, 6, false)), Hit::Consumed);
    session.help_mode = false;

    session.picker_mode = true;
    session.picker_entries = vec![super::super::super::PickerEntry {
        session_id: "$0".to_string(),
        session_name: "work".to_string(),
        windows: vec![("@0".to_string(), "main".to_string())],
        active_window: Some("@0".to_string()),
        current: true,
    }];
    session.refresh_picker();
    assert_eq!(
        session.mouse_hit(&sgr(64, 10, 6, false)),
        Hit::PickerMove(-1)
    );
    assert_eq!(
        session.mouse_hit(&sgr(65, 10, 6, false)),
        Hit::PickerMove(1)
    );
    assert_eq!(session.mouse_hit(&sgr(32, 10, 6, false)), Hit::Consumed);
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("picker up");
    let col = (x0 + 4) as u16;
    assert_eq!(
        session.mouse_hit(&sgr(0, col, (y0 + 3 + 1) as u16, false)),
        Hit::PickerRow(1)
    );
    let footer = session.picker_panel_len - 1;
    assert_eq!(
        session.mouse_hit(&sgr(0, col, (y0 + 3 + footer) as u16, false)),
        Hit::Consumed,
        "the footer does nothing"
    );
    session.picker_filtering = true;
    session.refresh_picker();
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("picker up");
    assert_eq!(
        session.mouse_hit(&sgr(0, (x0 + 4) as u16, (y0 + 3) as u16, false)),
        Hit::PickerFilter
    );
    assert_eq!(
        session.mouse_hit(&sgr(0, (x0 + 4) as u16, (y0 + 3 + 1) as u16, false)),
        Hit::PickerRow(0),
        "content starts under the filter line"
    );
}

/// The tab strip row and the pane area.
#[test]
fn mouse_hit_maps_the_tab_strip_and_the_pane_area() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 * main\n@1 - vim".to_string(),
    );
    let (_rx, mut conn, mut session) = two_pane_session(
        "m-hit",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    assert_eq!(
        session.mouse_hit(&sgr(2, 13, 1, false)),
        Hit::TabMenu("@1".to_string())
    );
    assert_eq!(
        session.mouse_hit(&sgr(2, 9, 1, false)),
        Hit::Consumed,
        "a right press on a gap column"
    );
    assert_eq!(session.mouse_hit(&sgr(0, 13, 1, false)), Hit::TabPress(12));
    assert_eq!(session.mouse_hit(&sgr(64, 13, 1, false)), Hit::Consumed);

    // Pane 2 holds cols 40..80; content row 4 is SGR row 6.
    let pane2 = *session.renderer.pane_at(60, 4).expect("pane 2");
    assert_eq!(
        session.mouse_hit(&sgr(0, 61, 6, false)),
        Hit::PanePress(pane2)
    );
    assert_eq!(
        session.mouse_hit(&sgr(64, 61, 6, false)),
        Hit::Wheel {
            rect: pane2,
            x: 60,
            cy: 4,
            delta: 3
        }
    );
    assert_eq!(
        session.mouse_hit(&sgr(0, 61, 6, true)),
        Hit::Consumed,
        "a release over a pane that does not own the mouse"
    );
    session.renderer.feed_output(2, b"\x1b[?1000h");
    assert_eq!(
        session.mouse_hit(&sgr(0, 61, 6, true)),
        Hit::PaneForward(pane2)
    );
    let divider = session.renderer.divider_near(40, 4, 1).expect("divider");
    assert_eq!(
        session.mouse_hit(&sgr(0, 41, 6, false)),
        Hit::DragStart {
            divider,
            x: 40,
            y: 4
        }
    );
}

/// Every PrefixChord arm, and the precedence between the configurable
/// keys and the fixed table.
#[test]
fn prefix_chord_classifies_bytes_in_precedence_order() {
    let mut session = WindowSession::new(80, 25);
    assert_eq!(session.prefix_chord(0x12), PrefixChord::Reload);
    assert_eq!(
        session.prefix_chord(b'%'),
        PrefixChord::Management(ManagementKey::SplitRight)
    );
    assert_eq!(
        session.prefix_chord(b's'),
        PrefixChord::Management(ManagementKey::Sidebar)
    );
    assert_eq!(session.prefix_chord(b'R'), PrefixChord::Resize);
    assert_eq!(session.prefix_chord(b'?'), PrefixChord::Help);
    assert_eq!(session.prefix_chord(b'w'), PrefixChord::Picker);
    assert_eq!(session.prefix_chord(b'd'), PrefixChord::Detach);
    assert_eq!(session.prefix_chord(b'['), PrefixChord::Scroll);
    for key in *b"np()o" {
        assert_eq!(session.prefix_chord(key), PrefixChord::Switch);
    }
    assert_eq!(session.prefix_chord(0x02), PrefixChord::Literal);
    assert_eq!(session.prefix_chord(b'q'), PrefixChord::Unbound);

    // A management key rebound onto a fixed-table byte wins over it.
    session.management.zoom = b'n';
    assert_eq!(
        session.prefix_chord(b'n'),
        PrefixChord::Management(ManagementKey::Zoom)
    );
    // The reload key never shadows detach.
    session.reload_key = b'd';
    assert_eq!(session.prefix_chord(b'd'), PrefixChord::Detach);
    // Reload outranks a management chord on the same byte.
    session.reload_key = b'%';
    assert_eq!(session.prefix_chord(b'%'), PrefixChord::Reload);
}
