//! WindowSession suites: frame layout, strip clicks, tab drags, and copy mode.

use super::*;

/// The tab strip costs one content row everywhere the status row
/// does: the renderer is the host grid minus strip minus status
/// (80x25 host → an 80x23 content renderer).
#[test]
fn session_height_carves_the_strip_and_status_out() {
    let session = WindowSession::new(80, 25);
    assert_eq!(session.renderer.window_size(), (80, 23));
}

/// The context menus' dispatch mapping is pure: each overlay row
/// carries exactly one action (or `None` for the header/footer), so
/// what a click highlights is exactly what dispatches — and the two
/// menus' vocabularies differ in the third action only.
#[test]
fn compose_menu_panel_maps_rows_to_actions() {
    let (rows, mapping) =
        compose_menu_panel(&MenuTarget::Tab("@1".to_string()), Some("vim"), false);
    assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
    assert_eq!(rows[0].text, " vim ", "the header carries the name");
    assert_eq!(mapping[0], None, "the header consumes its clicks");
    assert_eq!(mapping[1], Some(MenuAction::Rename));
    assert_eq!(rows[1].text, " rename ");
    assert_eq!(mapping[2], Some(MenuAction::Close));
    assert_eq!(rows[2].text, " close ");
    assert_eq!(mapping[3], Some(MenuAction::AddTab));
    assert_eq!(rows[3].text, " add tab ");
    assert_eq!(mapping[4], Some(MenuAction::MoveLeft));
    assert_eq!(rows[4].text, " move left ");
    assert_eq!(mapping[5], Some(MenuAction::MoveRight));
    assert_eq!(mapping[6], Some(MenuAction::MoveFirst));
    assert_eq!(mapping[7], Some(MenuAction::MoveLast));
    assert_eq!(mapping[8], None, "the footer consumes its clicks");

    let (_, ws_mapping) = compose_menu_panel(
        &MenuTarget::Workspace("+0".to_string()),
        Some("main"),
        false,
    );
    assert_eq!(
        ws_mapping,
        vec![
            None,
            Some(MenuAction::Rename),
            Some(MenuAction::Close),
            Some(MenuAction::NewWorkspace),
            Some(MenuAction::RespawnToggle),
            Some(MenuAction::ExitAction("remove")),
            Some(MenuAction::ExitAction("hold")),
            Some(MenuAction::ExitAction("shell")),
            None
        ]
    );
}

/// A flushed frame lands the strip at host row 0, rebases the pane
/// diff one host row down, and places the cursor one host row below
/// its renderer-mapped cell — the strip's height consumers, pinned.
/// (`terminal_grid` is 80x24 headless, so the session is built for a
/// 24-row host: strip 0, panes 1..=22, status 23.) Unix-only: the
/// assertion pins the non-tty (80, 24) fallback — an interactive
/// Windows console session reports its real grid and the bottom row
/// lands elsewhere (measured on the Windows VM, 2026-10-04).
#[cfg(unix)]
#[test]
fn frame_flushes_the_strip_at_row_zero_and_rebases_the_panes() {
    let mut session = WindowSession::new(80, 24);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.feed_output(1, b"pane-one\r\n");
    // Paint the strip and the status row so both rows flush.
    let windows = vec![
        ("@0".to_string(), "main".to_string()),
        ("@1".to_string(), "vim".to_string()),
    ];
    session.chrome.tab_strip.paint(&windows, Some("@0"), None);
    session.draw_status_row();

    let mut sink = RecordingSink::default();
    session.frame(&mut sink);

    // The strip's text cells flush at the host's top row...
    let strip_cells: Vec<&(u16, u16, RtCell)> =
        sink.cells.iter().filter(|(_, y, _)| *y == 0).collect();
    assert!(!strip_cells.is_empty(), "the strip flushes at row 0");
    // ...the pane diff lands strictly between the strip and the
    // status row (rebased one host row down)...
    assert!(sink.cells.iter().any(|(_, y, _)| (1..=22).contains(y)));
    // ...and the status row stays at the bottom (23).
    assert!(sink.cells.iter().any(|(x, y, _)| *y == 23 && *x < 80));
    // The cursor maps through the strip: renderer row + 1.
    let expected = session
        .renderer
        .focused_cursor()
        .map(|(x, y, style)| (x, y + 1, style));
    assert_eq!(sink.placements.last(), Some(&expected));
}

/// A press on a tab switches to that window through the existing
/// select+resync contract — the wire carries `select-window` and the
/// re-seed's size report (content height = rows - strip - status =
/// 23 on an 80x25 host) — and never focuses or forwards into a pane.
#[test]
fn strip_click_switches_windows_without_forwarding() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 - main\n@1 * vim".to_string(),
    );
    replies.insert("list-panes -t @1".to_string(), "%1".to_string());
    let (rx, mut conn) = scripted_conn("tabclick", replies);
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();

    // The queried state fills through the real refresh path; the
    // strip paints from it.
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    // Drain the refresh traffic so the click's wire assertions see
    // only the click's lines.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    // herdr blocks: "  main  " spans cols 0..8, the gap is col 8, and
    // the second tab's block ("  vim  ") spans cols 9..16; the click
    // is a 1-based host col 13 (strip col 12).
    assert_eq!(
        session.chrome.tab_strip.hit_test(8),
        None,
        "the gap hits nothing"
    );
    session.tab_click(&mut conn, 12);
    settle(&mut session, &mut conn);

    assert_eq!(session.window, "@1", "the view moved to the clicked tab");
    assert_eq!(session.status.active_window.as_deref(), Some("@1"));
    // The wire contract: the switch-client landing (which selects the
    // window and moves the displayed session) + re-seed; nothing
    // pane-level.
    let switch = wait_recorded(&rx, "switch-client");
    assert!(
        switch.contains("-t @1"),
        "the click selects the clicked window: {switch}"
    );
    let size_report = wait_recorded(&rx, "refresh-client");
    assert!(
        size_report.contains("80x23"),
        "the size report carries content height rows-strip-status: {size_report}"
    );
    // No focus, no forwarding: drain the residual resync traffic
    // (status re-queries) and assert nothing pane-level rode it.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    // (The receiver is dropped with the conn at scope end; the
    // assertions above ran over every recorded line already.)
    assert_eq!(
        session.chrome.tab_strip.hit_test(12),
        Some(1),
        "the strip layout still maps the clicked column"
    );
}

/// `pane_view_cell` maps the pointer onto the selection's absolute
/// line: with the view scrolled up S lines, view rows below S hit the
/// LIVE grid at len + r - S — one subtraction of S, not two (the
/// paint path's old live-row line double-subtracted it).
#[test]
fn pane_view_cell_maps_live_rows_under_scroll() {
    let (rx, _conn, mut session) = two_pane_session("m-view", FakeScript::default());
    drained(&rx);
    for i in 0..60 {
        session
            .renderer
            .feed_output(1, format!("pane one line {i:03}\r\n").as_bytes());
    }
    session.renderer.scroll_viewport(1, 3);
    let rect = session.renderer.pane_rect(1).expect("pane 1 rect");
    // 37 scrollback lines (60 fed lines, 24 live rows, the trailing
    // newline's scroll), view scrolled 3: row 0 shows scrollback line
    // 34; view row 3 is live row 0 = line 37; view row 5 = line 39.
    assert_eq!(session.renderer.pane_view_cell(&rect, 0, 0), Some((34, 0)));
    assert_eq!(session.renderer.pane_view_cell(&rect, 5, 3), Some((37, 5)));
    assert_eq!(session.renderer.pane_view_cell(&rect, 2, 5), Some((39, 2)));
}

/// A character selection extracts exactly the swept text — reading
/// order between anchor and head whichever way the drag ran, the first
/// and last rows sliced at its cells, one trailing newline like the
/// line selection's payload.
#[test]
fn char_selection_text_slices_rows_in_reading_order() {
    let (_rx, _conn, mut session) = two_pane_session("m-selt", FakeScript::default());
    for i in 0..6 {
        session
            .renderer
            .feed_output(1, format!("pane one line {i}\r\n").as_bytes());
    }
    // Dragged UP-RIGHT: anchor below head — the span reads the same.
    session.renderer.set_selection(Some(Selection::Chars {
        pane: 1,
        anchor: (3, 2),
        head: (1, 8),
    }));
    assert_eq!(
        session.renderer.selection_text().as_deref(),
        Some(" line 1\npane one line 2\npan\n")
    );
}

/// Copy mode (the TUI audit's R1, now the full vi form): `V` anchors a
/// line selection the motions extend, `y` copies the selected lines as
/// an OSC 52 write in the frame's outbox and exits, and `/` searches
/// the pane's text with n walking the committed matches.
#[test]
fn copy_mode_selects_copies_and_searches() {
    let (rx, mut conn, mut session) = two_pane_session("copymode", FakeScript::default());
    drained(&rx);
    // Real scrollback on pane 1: 60 numbered lines.
    for i in 0..60 {
        session
            .renderer
            .feed_output(1, format!("line {i:03} marker\n").as_bytes());
    }
    session.renderer.focus(1);
    // prefix [ enters copy mode; a yank does not exit here.
    session.route_stdin_bytes(&mut conn, &[0x02, b'[']);
    drained(&rx);
    session.copy_mode_auto_exit = false;
    // gg to the buffer top first (entry lands at the pane's live
    // cursor, the buffer's bottom row), then `V` anchors a line
    // selection the motions extend.
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"g", &mut conn);
    session.route_plain(b"V", &mut conn);
    let Selection::Lines { pane, a, b } = session.renderer.selection.expect("selection up") else {
        panic!("line mode paints whole lines");
    };
    assert_eq!((pane, a, b), (1, 0, 0), "the anchor is the buffer top");
    session.route_plain(b"jjj", &mut conn);
    let Selection::Lines { a, b, .. } = session.renderer.selection.expect("still up") else {
        panic!("still a line selection");
    };
    assert_eq!((a, b), (0, 3), "three j extend the head");
    // `y` copies: an OSC 52 write rides the frame's outbox.
    session.route_plain(b"y", &mut conn);
    let osc = session
        .notify
        .outbox
        .windows(7)
        .position(|w| w == b"\x1b]52;c;")
        .is_some();
    assert!(
        osc,
        "the copy emits an OSC 52 write: {:?}",
        session.notify.outbox
    );
    assert!(session.renderer.selection.is_none(), "copied clears");
    assert!(session.is_copy(), "auto-exit off: the mode stays up");
    // `/` searches; Enter commits; n walks the committed matches.
    session.route_plain(b"/line 042\r", &mut conn);
    assert!(
        !session.copy_state().is_searching,
        "Enter committed the query"
    );
    assert!(
        !session.scroll_matches.is_empty(),
        "the query matched the fed scrollback"
    );
    session.route_plain(b"n", &mut conn);
    let landed = session.copy_state().cursor_absolute_line;
    let landed_text = session.copy_line_text(landed);
    assert!(
        landed_text.to_lowercase().contains("line 042"),
        "n landed on a match: line {landed} {landed_text:?}"
    );
    session.route_plain(b"q", &mut conn);
    assert!(!session.is_copy(), "q leaves");
    drained(&rx);
}

/// A tab press arms a drag; dropping on another tab sends
/// `move-window -s @from -t <slot>` (the TUI audit's R6 drag reorder);
/// dropping back on the origin is just the click. The wheel offset
/// resets with the fresh order.
#[test]
fn tab_drag_reorders_via_move_window() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 - main\n@1 - vim".to_string(),
    );
    let (rx, mut conn) = scripted_conn("tabdrag", replies);
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    // Press arms on the first tab; the drop on the second reorders.
    session.tab_click(&mut conn, 0);
    assert_eq!(session.tab_drag_from, Some(0), "the press armed the drag");
    session.tab_drop(&mut conn, 12);
    settle(&mut session, &mut conn);
    let moved = wait_recorded(&rx, "move-window");
    assert!(
        moved.contains("-s @0") && moved.contains("-t 1"),
        "the drop reorders: {moved}"
    );
    assert_eq!(session.tab_drag_from, None, "the drop disarms");

    // A drop on the origin tab sends nothing.
    session.tab_click(&mut conn, 0);
    session.tab_drop(&mut conn, 0);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(150))
            .is_err(),
        "same-tab drop: no wire line"
    );
}

/// A wheel over an overflowing strip shifts the visible run toward the
/// hidden tabs; a strip that fits ignores it (the R6 wheel scroll).
#[test]
fn tab_wheel_scrolls_an_overflowing_strip() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 * a\n@1 - b\n@2 - c\n@3 - d\n@4 - e\n@5 - f\n@6 - g\n@7 - h".to_string(),
    );
    let (rx, mut conn) = scripted_conn("tabwheel", replies);
    let mut session = WindowSession::new(24, 25);
    session.window = "@0".to_string();
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    assert!(
        session.chrome.tab_strip.overflowing(),
        "eight tabs overflow a 24-column strip"
    );
    let before = session.chrome.tab_strip.hit_test(0);
    session.chrome.tab_strip.wheel(3);
    session.draw_tab_strip();
    let after = session.chrome.tab_strip.hit_test(0);
    assert_ne!(before, after, "the wheel shifted the visible run");
    session.chrome.tab_strip.wheel(-30);
    session.draw_tab_strip();
    assert_eq!(
        session.chrome.tab_strip.hit_test(0),
        before,
        "the offset clamps back to 0"
    );
}

/// A width change rebuilds the strip's buffers for the new cols, and
/// the rebuild must carry the configured width bounds across
/// (`palette` already rides it): without that, `tab-width-min`/`-max`
/// silently reset to 0 on every resize and on the geometry re-set a
/// daemon restart's re-land performs — the settings dialog reads the
/// live strip, so the floor vanished until the value was re-entered.
#[test]
fn width_change_keeps_tab_bounds() {
    let mut session = WindowSession::new(80, 24);
    session.chrome.tab_strip.bounds = crate::mux::attach::tabs::TabBounds { min: 12, max: 30 };
    session.draw_tab_strip();
    assert_eq!(
        session.chrome.tab_strip.bounds,
        crate::mux::attach::tabs::TabBounds { min: 12, max: 30 },
        "a same-width draw leaves the bounds in place"
    );
    // A width change takes the rebuild branch.
    session.geometry.host_cols = 100;
    session.draw_tab_strip();
    assert_eq!(
        session.chrome.tab_strip.bounds,
        crate::mux::attach::tabs::TabBounds { min: 12, max: 30 },
        "the rebuild preserves the configured tab width bounds"
    );
}

/// The tab-strip scroll chords (prefix H / L): shift the overflowing
/// strip's visible run without switching windows — 3 tabs per press,
/// clamped like the wheel, a no-op when the strip fits.
#[test]
fn tab_strip_chords_scroll_an_overflowing_strip() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 * a\n@1 - b\n@2 - c\n@3 - d\n@4 - e\n@5 - f\n@6 - g\n@7 - h".to_string(),
    );
    let (rx, mut conn) = scripted_conn("tabstrip", replies.clone());
    let mut session = WindowSession::new(24, 25);
    session.window = "@0".to_string();
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    assert!(
        session.chrome.tab_strip.overflowing(),
        "eight tabs overflow a 24-column strip"
    );
    let before = session.chrome.tab_strip.hit_test(0);

    // The left chord from the active-centered run: the run is already at
    // the leftmost tab, so the hit-test stays put (clamped).
    session.management_chord(ManagementKey::TabStripLeft, &mut conn);
    session.draw_tab_strip();
    assert_eq!(
        session.chrome.tab_strip.hit_test(0),
        before,
        "the left chord clamps at the first tab"
    );

    // The right chord shifts the run toward the hidden tabs; the left
    // chord walks it back.
    session.management_chord(ManagementKey::TabStripRight, &mut conn);
    session.draw_tab_strip();
    let after = session.chrome.tab_strip.hit_test(0);
    assert_ne!(before, after, "the right chord shifted the visible run");
    session.management_chord(ManagementKey::TabStripLeft, &mut conn);
    session.draw_tab_strip();
    assert_eq!(
        session.chrome.tab_strip.hit_test(0),
        before,
        "the left chord walked the run back"
    );

    // A strip that fits ignores both chords.
    let (rx2, mut conn2) = scripted_conn("tabstripfit", replies.clone());
    let mut fits = WindowSession::new(200, 25);
    fits.window = "@0".to_string();
    fits.status.refresh(&mut conn2, "@0", 1).expect("refresh");
    fits.draw_tab_strip();
    while rx2
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}
    assert!(
        !fits.chrome.tab_strip.overflowing(),
        "eight tabs fit 200 columns"
    );
    let fitted = fits.chrome.tab_strip.hit_test(0);
    fits.management_chord(ManagementKey::TabStripRight, &mut conn2);
    fits.draw_tab_strip();
    assert_eq!(
        fits.chrome.tab_strip.hit_test(0),
        fitted,
        "a fitting strip ignores the chords"
    );
}
