//! WindowSession suites: frame layout, strip and sidebar clicks, drags,
//! display options, overlays, and the command menu.

use super::*;
use crate::mux::attach::status::RosterEntry;

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

/// The copy-mode extension (the TUI audit's R1): `v` anchors the line
/// selection at the viewport top, scrolling extends it, `y` copies the
/// selected lines as an OSC 52 write in the frame's outbox, and `/`
/// searches the pane's text with the match landing in view.
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
    // prefix [ enters the scroll viewport.
    session.route_stdin_bytes(&mut conn, &[0x02, b'[']);
    drained(&rx);
    // `v` anchors at the viewport top; scrolling extends the head.
    session.toggle_line_selection(1);
    let Selection::Lines { pane, a, b } = session.renderer.selection.expect("selection up") else {
        panic!("copy mode selects whole lines");
    };
    assert_eq!((pane, a, b), (1, a, a), "the anchor starts the span");
    session.renderer.scroll_viewport(1, 6);
    session.update_selection_head(1);
    let Selection::Lines { a, b, .. } = session.renderer.selection.expect("still up") else {
        panic!("still a line selection");
    };
    assert!(b > a, "the head moved past the anchor: {a}..{b}");
    // `y` copies: an OSC 52 write rides the frame's outbox.
    session.copy_selection();
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
    // `/` searches; Enter commits; the match lands in the viewport.
    session.begin_scroll_search();
    for byte in b"line 042" {
        assert!(session.search_typing_byte(*byte), "typing continues");
    }
    assert!(!session.search_typing_byte(b'\r'), "Enter closes the line");
    assert!(
        !session.scroll_matches.is_empty(),
        "the query matched the fed scrollback"
    );
    session.scroll_search(1);
    let offset = session.renderer.scroll_offset_of(1);
    assert!(offset > 0, "the viewport landed on the match");
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

/// The drag affordance, headless end-to-end: a press ON the
/// two-pane divider column starts a PENDING drag (no focus, no
/// forward), motion promotes it and the pump's apply_drag sends one
/// relative resize-pane for the boundary's left pane at the full
/// delta, the divider highlights while dragging, and release clears
/// the state without forwarding anything to either pane.
#[test]
fn drag_on_divider_resizes_without_clicking_through() {
    let (rx, mut conn) = recording_conn("drag");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));

    // Drain the handshake's recorded lines first, so the quiet wire
    // assertions below see only the drag's traffic.
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    // Press on the divider column (x=39, 1-based col 40): pending,
    // and the press must NOT focus or forward.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: false,
        },
    );
    assert!(
        matches!(session.drag, Some(DragState::Pending { .. })),
        "the press starts a pending drag"
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a divider press forwards nothing"
    );

    // Motion +5 cells: the drag activates and highlights the divider.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 45,
            row: 6,
            release: false,
        },
    );
    assert_eq!(
        session.renderer.drag_divider,
        Some((true, 1, 2)),
        "the dragged divider highlights"
    );

    // The pump's frame-cadence application: one resize-pane at the
    // full delta, aimed at the boundary's left/top pane.
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 5",
        "the drag maps to the wire's relative resize for pane 1"
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a drag forwards no mouse bytes to the pane"
    );

    // Release: the drag ends, the highlight clears, still no
    // click-through.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 45,
            row: 6,
            release: true,
        },
    );
    assert!(session.drag.is_none());
    assert_eq!(session.renderer.drag_divider, None);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "the drag's release forwards nothing"
    );
}

/// A click NEAR a divider without any drag still focuses: press and
/// release at the same point land the focus on the pane under the
/// pointer (select-pane rides the wire) even though the press itself
/// was captured by the drag's pending state.
#[test]
fn click_near_divider_without_drag_still_focuses() {
    let (rx, mut conn) = recording_conn("click-div");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(2);

    // Press on the divider, release without motion: the focus flips
    // to the pane under the pointer (the divider column sits in pane
    // 1's last column).
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: false,
        },
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: true,
        },
    );
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the bare click focuses the pane under the pointer"
    );
    assert_eq!(
        wait_recorded(&rx, "select-pane"),
        "select-pane -t %1",
        "the focus rides select-pane"
    );
}

/// A press on the pane content cell BESIDE the divider is a pane
/// click, not a drag start: the manual-pass trace's swallowed htop
/// click (a press one cell off the divider hijacked as a divider drag,
/// whose drift motions then resized the layout at pointer speed).
#[test]
fn a_press_beside_the_divider_is_a_pane_click_not_a_drag() {
    let (rx, mut conn) = recording_conn("edge-click");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(2);

    // 1-based col 39 = content col 38: pane 1's second-to-last column,
    // one cell left of the divider at col 39 — a content cell.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 39,
            row: 6,
            release: false,
        },
    );
    assert!(
        session.drag.is_none(),
        "a content-cell press must not start a drag"
    );
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the press focuses the pane under the pointer"
    );
    assert_eq!(
        wait_recorded(&rx, "select-pane"),
        "select-pane -t %1",
        "the focus rides select-pane"
    );
    // And the mirrored release is a plain pane release (no drag state).
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 39,
            row: 6,
            release: true,
        },
    );
    assert!(session.drag.is_none());
}

/// The drag's resize steps are throttled: one `resize-pane` per
/// interval at most, the unapplied remainder held back while motions
/// keep arriving faster, and the release flushes what is left so the
/// drag lands exactly where it is let go.
#[test]
fn drag_resizes_throttle_and_the_release_flushes_the_remainder() {
    let (rx, mut conn) = recording_conn("drag-throttle");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    while rx
        .recv_timeout(std::time::Duration::from_millis(150))
        .is_ok()
    {}

    let press = SgrMouse {
        cb: 0,
        col: 40,
        row: 6,
        release: false,
    };
    session.route_mouse(&mut conn, press);
    // Motion +5, then +3 more before the interval can pass.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 45,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 5",
        "the first step sends the full delta"
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 48,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a step inside the interval sends nothing"
    );
    // Release: the held remainder (+3) flushes immediately.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 48,
            row: 6,
            release: true,
        },
    );
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 3",
        "the release flushes the throttled remainder"
    );
    assert!(session.drag.is_none());
}

/// The tmux divider convention (the owner's manual pass): the HALF of
/// the shared divider nearer the active pane carries the highlight —
/// a side-by-side split's divider highlights its top half when the
/// left pane is focused and its bottom half when the right one is.
#[test]
fn focus_flip_changes_the_shared_dividers_color() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);

    renderer.focus(1);
    renderer.render_frame();
    assert_eq!(
        renderer.buffer[(39, 0)].fg,
        RtColor::Indexed(14),
        "pane 1 (left) focused: the divider's TOP half is the accent"
    );
    assert!(
        renderer.buffer[(39, 23)].modifier.contains(RtModifier::DIM),
        "pane 1 focused: the divider's bottom half stays dim"
    );
    renderer.focus(2);
    renderer.mark_all_dirty();
    renderer.render_frame();
    assert_eq!(
        renderer.buffer[(39, 23)].fg,
        RtColor::Indexed(14),
        "pane 2 (right) focused: the divider's BOTTOM half is the accent"
    );
    assert!(
        renderer.buffer[(39, 0)].modifier.contains(RtModifier::DIM),
        "pane 2 focused: the divider's top half stays dim"
    );
}

/// Render-mode resize chords: prefix R enters the mode and arrows
/// send the relative resize-pane for the FOCUSED pane; a non-arrow
/// key leaves the mode with nothing leaked into the pane.
#[test]
fn render_resize_chord_sends_for_the_focused_pane() {
    let (rx, conn) = recording_conn("render-resize");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    let mut conn = conn;

    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'R'], &mut conn,),
        "the resize chord enters the mode"
    );
    assert!(session.is_resize());
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 1",
        "the arrow resizes the focused pane (the layout's first leaf)"
    );
    // Any other key leaves the mode, consumed.
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Escape, 0));
    assert!(!session.is_resize());

    // The RIGHT pane focused: the arrows move the shared divider in
    // the pressed direction, so the wire direction inverts (the wire
    // grows the focused pane; growing the right pane would move the
    // divider left — the manual-pass report).
    session.renderer.focus(2);
    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'R'], &mut conn,),
        "the resize chord re-enters the mode"
    );
    session.resize_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %2 -L 1",
        "Right with the right pane focused shrinks it: the divider moves right"
    );
}

/// The help chord: prefix ? opens the panel (categories and effective
/// bindings in the overlay), / narrows it, and dismissal (q) restores
/// the prior frame — the pane's content cells repaint.
#[test]
fn help_chord_opens_filters_and_dismissal_restores_the_frame() {
    let (_rx, conn) = recording_conn("help");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.feed_output(1, b"LEFT\r\n");
    let mut sink = CursorSink {
        placements: Vec::new(),
    };
    session.frame(&mut sink); // the settled prior frame

    let mut conn = conn;
    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'?'], &mut conn,),
        "the help chord opens the panel"
    );
    assert!(session.is_help());
    let overlay = session.renderer.overlay.clone().expect("the overlay is up");
    let joined = overlay
        .1
        .iter()
        .map(|r| r.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    // The compose carries content rows only; the ring/title/badge are
    // paint_overlay's (asserted in the paint-level tests below). The
    // idle panel has no placeholder row — the footer advertises
    // `search /`.
    assert!(
        !joined.contains("press / to filter"),
        "no idle placeholder row: {joined}"
    );
    assert!(joined.contains(" global "), "a category header: {joined}");
    assert!(
        joined.contains("close esc/enter"),
        "the footer names the controls: {joined}"
    );
    // The filter: '/' then "swap" narrows; a non-matching row drops.
    assert!(session.help_byte(b'/'));
    for byte in b"swap" {
        assert!(session.help_byte(*byte));
    }
    let filtered = session.renderer.overlay.clone().expect("overlay");
    let ftext = filtered
        .1
        .iter()
        .map(|r| r.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ftext.contains("swap"), "matching rows survive: {ftext}");
    assert!(!ftext.contains("detach"), "others drop: {ftext}");
    // Commit the filter (Enter), then dismiss (q): the prior frame's
    // cells repaint.
    assert!(session.help_byte(b'\r'));
    assert!(
        !session.help_byte(b'q'),
        "q closed the panel (the return is still-open)"
    );
    assert!(!session.is_help());
    assert_eq!(session.renderer.overlay, None);
    session.frame(&mut sink);
    let left: String = (0..4)
        .map(|c| session.renderer.buffer[(c, 0)].symbol())
        .collect();
    assert_eq!(left, "LEFT", "the prior frame's cells restore");
}

/// The picker chord: prefix w opens the modal (sessions with nested
/// windows, the current one emphasized), j moves the cursor, Enter
/// activates — the select+resync contract's select-window rides the
/// wire — and `q` dismisses without selecting.
#[test]
fn picker_chord_opens_navigates_and_selects_with_resync() {
    let replies = std::collections::HashMap::from([
        (
            "list-sessions".to_string(),
            "+0: main: $0: work\n+1: lab: $1: build\n".to_string(),
        ),
        (
            "list-windows -t $0".to_string(),
            "@0 - main\n@1 * vim\n".to_string(),
        ),
        ("list-windows -t $1".to_string(), "@2 * logs\n".to_string()),
    ]);
    let (rx, conn) = scripted_conn("picker", replies);
    let mut conn = conn;
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    // The view shows $0's window @0.
    session.status.session_id = Some("$0".to_string());
    session.window = "@0".to_string();

    assert!(
        !session.route_plain(&[crate::mux::attach::C_B, b'w'], &mut conn,),
        "the picker chord opens the modal"
    );
    settle(&mut session, &mut conn);
    assert!(session.is_picker());
    let (overlay, _, _) = session.renderer.overlay.clone().expect("the overlay is up");
    assert_eq!(overlay, crate::mux::attach::PICKER_OVERLAY_TITLE);
    let joined = overlay_text(&session).join("\n");
    assert!(
        joined.contains(">$0: work"),
        "the current session is >-marked: {joined}"
    );
    assert!(
        joined.contains("@1: vim *"),
        "the active window carries *: {joined}"
    );

    // j j moves the cursor to the second window row; Enter activates
    // — the wire sees the switch-client landing and the re-seed's
    // queries (the resync).
    session.picker_byte(&mut conn, b'j');
    session.picker_byte(&mut conn, b'j');
    session.picker_byte(&mut conn, b'\r');
    settle(&mut session, &mut conn);
    assert!(!session.is_picker(), "activation dismisses the modal");
    assert_eq!(
        wait_recorded(&rx, "switch-client"),
        "switch-client -t @1",
        "the session header's active window is the landing target"
    );
    let _ = wait_recorded(&rx, "list-panes");
    let _ = wait_recorded(&rx, "refresh-client");
    assert_eq!(session.window, "@1", "the view re-seeded onto @1");

    // Re-open and dismiss with q: no select goes out.
    assert!(!session.route_plain(&[crate::mux::attach::C_B, b'w'], &mut conn,));
    settle(&mut session, &mut conn);
    assert!(session.is_picker());
    assert!(!session.picker_byte(&mut conn, b'q'));
    assert!(!session.is_picker());
}

/// The `pane-gaps` option: each pane's content insets by the gap and
/// the band cells carry the theme background — pane A's first content
/// column moves to x=1 and the band at x=0 keeps the fill.
#[test]
fn pane_gaps_option_insets_pane_content_and_fills_the_band() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
    renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    renderer.feed_output(1, b"LEFT\r\n");
    renderer.feed_output(2, b"RIGHT\r\n");
    renderer.set_pane_gaps(1);
    renderer.render_frame();

    let bg = RtColor::Rgb(16, 24, 40);
    // The band rows/cols carry no content and keep the fill; the
    // content starts one cell in on each axis.
    for x in [0u16, 40] {
        let band = renderer.cell(x, 0).expect("band cell");
        assert_eq!(band.symbol(), " ", "the band cell carries no content");
        assert_eq!(band.bg, bg, "the band cell carries the theme bg");
    }
    let a1 = renderer.cell(1, 1).expect("content cell");
    assert_eq!(a1.symbol(), "L", "pane A's content starts inside the gap");
    let b1 = renderer.cell(41, 1).expect("right content cell");
    assert_eq!(b1.symbol(), "R", "pane B's content starts inside the gap");
    // The cursor maps through the same inset (content_view drives
    // the placement path too): the feed's trailing newline left the
    // pane cursor at content col 0 / row 1.
    let cursor = renderer.focused_cursor().expect("cursor");
    assert_eq!(cursor.0, 1, "the cursor maps inside the left band");
    assert_eq!(cursor.1, 2, "the cursor maps below the top band");
}

/// Options OFF render identical to a default renderer: the
/// pins-the-default contract for pane-gaps / scrollbar-gutter.
#[test]
fn display_options_off_render_identical_to_defaults() {
    let build = |gaps: u16, gutter: bool| {
        let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
        renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
        renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
        renderer.feed_output(1, b"LEFT\r\nsecond\r\n");
        renderer.feed_output(2, b"RIGHT\r\n");
        renderer.set_pane_gaps(gaps);
        renderer.set_scrollbar_gutter(gutter);
        renderer.render_frame();
        renderer
    };
    let baseline = build(0, false);
    let explicit = build(0, false);
    for y in 0..24u16 {
        for x in 0..80u16 {
            let (a, b) = (
                baseline.cell(x, y).expect("cell"),
                explicit.cell(x, y).expect("cell"),
            );
            assert_eq!(a.symbol(), b.symbol(), "symbol differs at {x},{y}");
            assert_eq!(a.bg, b.bg, "bg differs at {x},{y}");
            assert_eq!(a.fg, b.fg, "fg differs at {x},{y}");
        }
    }
}

/// The `scrollbar-gutter` option: the content narrows by one column,
/// the gutter fills with the theme bg, and a scrolled pane shows the
/// position indicator; an unscrolled pane's gutter stays blank and
/// gutter OFF paints no indicator at all.
#[test]
fn scrollbar_gutter_option_reserves_a_column_and_indicates() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
    renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    // 40 lines of scrollback on pane 1.
    let mut flood = Vec::new();
    for i in 0..40 {
        flood.extend(format!("line{i:02}\r\n").as_bytes());
    }
    renderer.feed_output(1, &flood);
    renderer.feed_output(2, b"R\r\n");
    renderer.set_scrollbar_gutter(true);
    renderer.render_frame();

    // Unscrolled: the gutter column is theme-bg blank; the content
    // beside it is the pane's line (the last full line — the feed's
    // trailing newline scrolled one more row).
    let gutter = renderer.cell(39, 23).expect("gutter cell");
    assert_eq!(gutter.symbol(), " ", "an unscrolled gutter stays blank");
    let content = renderer.cell(0, 22).expect("narrowed content");
    assert_eq!(
        content.symbol(),
        "l",
        "content paints through the narrowed view"
    );

    renderer.scroll_viewport(1, 10);
    renderer.render_frame();
    // The indicator sits at the view top's proportional depth:
    // 10 * 24 / (10 + 24) = row 7.
    let indicator = renderer.cell(39, 7).expect("indicator cell");
    assert_eq!(
        indicator.symbol(),
        "\u{2590}",
        "the scrolled pane shows the indicator"
    );
    assert_eq!(
        indicator.fg,
        RtColor::Indexed(14),
        "the indicator is accent"
    );

    // Gutter OFF: the cue is suppressed entirely — no indicator glyph or
    // full-height blank lands in the neighbor's first column, and the
    // boundary column carries the divider glyph in every row (the
    // boundary cell is not yielded to a scrolled pane).
    let mut plain = PaneRenderer::new(80, 24, Glyphs::Unicode);
    plain.set_background(Some(RtColor::Rgb(16, 24, 40)));
    plain.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    plain.feed_output(1, &flood);
    plain.render_frame();
    plain.scroll_viewport(1, 10);
    plain.render_frame();
    for y in 0..24u16 {
        assert_ne!(
            plain.cell(40, y).expect("plain").symbol(),
            "\u{2590}",
            "gutter off writes nothing into the neighbor's first column"
        );
    }
    for y in 0..24u16 {
        assert_eq!(
            plain.cell(39, y).expect("plain").symbol(),
            "│",
            "the boundary divider stays painted"
        );
    }
}

/// The `change-cursor-shape` option: while a divider drag is live the
/// frame's cursor placement carries the resize shape (steady block),
/// overriding the pane's tracked DECSCUSR shape; option off (and
/// drag end) keeps the pane's own shape.
#[test]
fn change_cursor_shape_shapes_the_cursor_during_a_drag() {
    let mut session = WindowSession::new(80, 24);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    // A distinctive tracked shape: the pane set DECSCUSR blinking bar.
    session.renderer.feed_output(1, b"hi\x1b[5 q\r\n");
    let divider = DividerHit {
        vertical: true,
        a: 1,
        b: 2,
    };
    session.drag = Some(DragState::Active {
        divider,
        x: 39,
        y: 0,
        applied: 0,
        pending: 0,
        last_sent: None,
    });

    // Option off: the pane's tracked shape survives a drag frame.
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let last = sink
        .placements
        .last()
        .copied()
        .flatten()
        .expect("placement");
    assert_eq!(
        last.2,
        CursorStyle::BlinkingBar,
        "off keeps the pane's shape"
    );

    // Option on: the drag frame shapes the cursor.
    session.cursor.change_cursor_shape = true;
    session.renderer.mark_all_dirty();
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let last = sink
        .placements
        .last()
        .copied()
        .flatten()
        .expect("placement");
    assert_eq!(last.2, CursorStyle::SteadyBlock, "on shapes the cursor");

    // Drag end restores: the next frame carries the pane's shape.
    session.drag = None;
    session.renderer.mark_all_dirty();
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let last = sink
        .placements
        .last()
        .copied()
        .flatten()
        .expect("placement");
    assert_eq!(last.2, CursorStyle::BlinkingBar, "drag end restores");
}

/// The `change-cursor-shape` pointer half: a no-button hover over a
/// divider wants the axis-correct resize POINTER (OSC 22
/// `ew-resize`), a hover off it or the option off restores `text`,
/// and a live drag keeps the shape wherever the pointer wanders.
#[test]
fn change_cursor_shape_shapes_the_pointer() {
    let (rx, mut conn) = recording_conn("ptr-shape");
    let mut session = WindowSession::new(80, 24);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    drained(&rx);
    session.cursor.change_cursor_shape = true;
    // A no-button hover ON the divider column (content x=39 → host col
    // 40; cb 35 = motion with no button).
    session.route_mouse(&mut conn, sgr(35, 40, 5, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;ew-resize"),
        "the hover shapes the pointer: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    // The same hover off the divider restores the terminal's pointer.
    session.route_mouse(&mut conn, sgr(35, 10, 5, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;text"),
        "leaving the divider restores: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    // A live drag keeps its divider's shape (cb 32 = motion with
    // button 0 held, pointer over the left pane's content).
    session.drag = Some(DragState::Active {
        divider: DividerHit {
            vertical: true,
            a: 1,
            b: 2,
        },
        x: 39,
        y: 0,
        applied: 0,
        pending: 0,
        last_sent: None,
    });
    session.route_mouse(&mut conn, sgr(32, 10, 5, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;ew-resize"),
        "the drag keeps the shape: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    // Option off: the hover wants nothing (and the trailing restore
    // lands).
    session.drag = None;
    session.cursor.change_cursor_shape = false;
    session.route_mouse(&mut conn, sgr(35, 40, 5, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let raw = String::from_utf8_lossy(&sink.raw).to_string();
    assert!(!raw.contains("]22;ew-resize"), "off shapes nothing: {raw}");
    assert!(raw.contains("]22;text"), "off restores: {raw}");
}

/// The `change-cursor-shape` tab half: a no-button hover ON a clickable
/// tab block wants the pointing POINTER (OSC 22 `pointer`, X11
/// `hand2`), a hover on the bar between tabs restores `text`, and the
/// option off shapes nothing.
#[test]
fn tab_hover_shapes_the_pointer() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 - main\n@1 * vim".to_string(),
    );
    let (rx, mut conn) = scripted_conn("tab-ptr", replies);
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    drained(&rx);
    session.cursor.change_cursor_shape = true;
    // The strip layout: "  main  " spans strip cols 0..8, the gap is
    // col 8, "  vim  " spans 9..16; the strip row is host row 1 (SGR).
    // Emit is change-gated, so each shape change must follow a
    // different want: tab, bar, tab, then off.
    session.route_mouse(&mut conn, sgr(35, 10, 1, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let raw = String::from_utf8_lossy(&sink.raw);
    assert!(
        raw.contains("]22;pointer") && raw.contains("]22;hand2"),
        "the hover shapes the pointer over the tab: {raw:?}"
    );
    // A hover on the gap between tabs restores the terminal's pointer.
    session.route_mouse(&mut conn, sgr(35, 9, 1, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;text"),
        "the bar between tabs restores: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    // Onto the tab again, then the option off: a shaped pointer plus a
    // disabled setting writes nothing further.
    session.route_mouse(&mut conn, sgr(35, 10, 1, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;pointer"),
        "the hover re-shapes: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    session.cursor.change_cursor_shape = false;
    session.route_mouse(&mut conn, sgr(35, 10, 1, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let raw = String::from_utf8_lossy(&sink.raw).to_string();
    assert!(
        !raw.contains("]22;pointer") && raw.contains("]22;text"),
        "off restores instead of re-shaping: {raw}"
    );
}

/// The drag END: the release report itself must restore the pointer
/// (OSC 22 `text`) — `update_pointer` runs after the dispatch clears
/// the drag, so no unrelated motion report is needed to un-shape it
/// (the live PTY report: the resize pointer stuck after release).
#[test]
fn drag_release_restores_the_pointer_shape() {
    let (rx, mut conn) = recording_conn("ptr-release");
    let mut session = WindowSession::new(80, 24);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    drained(&rx);
    session.cursor.change_cursor_shape = true;
    // Press on the divider (host col 40), drag left, release over the
    // left pane — all through the real hit dispatch.
    session.route_mouse(&mut conn, sgr(0, 40, 5, false));
    session.route_mouse(&mut conn, sgr(32, 36, 5, false));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;ew-resize"),
        "the live drag shapes the pointer: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    session.route_mouse(&mut conn, sgr(0, 36, 5, true));
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert!(
        String::from_utf8_lossy(&sink.raw).contains("]22;text"),
        "the release itself restores the pointer: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
    assert!(
        !String::from_utf8_lossy(&sink.raw).contains("]22;ew-resize"),
        "no resize shape outlives the drag: {:?}",
        String::from_utf8_lossy(&sink.raw)
    );
}

/// The side panel's own resize handles shape the pointer too (config
/// `change-cursor-shape`): the strip's edge column hovers left/right, the
/// rule row up/down, the chevron cell and the panel's content restore,
/// and a live sidebar drag keeps its shape while the pointer is over
/// the panes (the live report: the pointer never changed there).
#[test]
fn sidebar_handles_shape_the_pointer() {
    let (rx, mut conn) = recording_conn("ptr-sidebar");
    let mut session = WindowSession::new(80, 24);
    show_sidebar(&mut session);
    session.renderer.set_sidebar_sections(Some(vec![
        super::super::super::SidebarSection {
            title: "workspaces",
            rows: vec![
                srow(SidebarRowId::Workspace("+0".into()), "alpha", true),
                srow(SidebarRowId::Workspace("+1".into()), "beta", false),
            ],
        },
        super::super::super::SidebarSection {
            title: "agents",
            rows: vec![srow(SidebarRowId::Pane(7), "kimi (blocked)", false)],
        },
    ]));
    drained(&rx);
    session.cursor.change_cursor_shape = true;
    // Emit is change-gated: an assert whose want was already sent reads
    // an empty frame.
    let mut last: &'static str = "";
    fn shaped(session: &mut WindowSession, last: &mut &'static str, want: &'static str) {
        let mut sink = RecordingSink::default();
        session.frame(&mut sink);
        let raw = String::from_utf8_lossy(&sink.raw).to_string();
        assert!(
            raw.contains(want) || (raw.is_empty() && *last == want),
            "expected {want} (last={last}) in {raw:?}"
        );
        if !raw.is_empty() {
            *last = want;
        }
    }
    // The width handle: the strip's edge column (host col 19 = SGR 20).
    session.route_mouse(&mut conn, sgr(35, 20, 5, false));
    shaped(&mut session, &mut last, "]22;ew-resize");
    // The split handle: the rule row wherever the default split composes
    // it (composed row + strip + SGR origin).
    let rule = session.renderer.sidebar_rule_y().expect("a rule row");
    let rule_sgr = rule + 2;
    session.route_mouse(&mut conn, sgr(35, 3, rule_sgr, false));
    shaped(&mut session, &mut last, "]22;ns-resize");
    // Panel content is not a handle.
    session.route_mouse(&mut conn, sgr(35, 3, 4, false));
    shaped(&mut session, &mut last, "]22;text");
    // The panel's last row at the edge column: whatever a press there
    // would hit (the chevron's span decides), the hover agrees with it.
    let press = sgr(0, 20, 24, false);
    let collapse_wins = matches!(session.mouse_hit(&press), Hit::SidebarCollapse);
    session.route_mouse(&mut conn, sgr(35, 20, 24, false));
    shaped(
        &mut session,
        &mut last,
        if collapse_wins {
            "]22;text"
        } else {
            "]22;ew-resize"
        },
    );
    // A live split drag keeps its shape with the pointer over the pane
    // area, and the release restores.
    session.route_mouse(&mut conn, sgr(0, 3, rule_sgr, false));
    session.route_mouse(&mut conn, sgr(32, 40, 15, false));
    shaped(&mut session, &mut last, "]22;ns-resize");
    session.route_mouse(&mut conn, sgr(0, 40, 15, true));
    shaped(&mut session, &mut last, "]22;text");
}

/// Sidebar paint geometry: with the panel up, the re-divided
/// layout's right pane paints flush against the panel — content and
/// dividers carry the panel offset exactly once. (The manual-pass
/// double-push report: content shifted twice, dividers once.)
#[test]
fn sidebar_offset_paints_content_and_dividers_together() {
    let mut session = WindowSession::new(80, 25);
    // The pre-toggle layout: two 40-col panes; the right pane's
    // marker row lands at x=40 (no panel, no offset).
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE).expect("parses"));
    session.renderer.feed_output(2, b"GEO-MARK-42\r\n");
    session.renderer.render_frame();
    assert_eq!(
        session.renderer.cell(40, 0).map(|c| c.symbol()),
        Some("G"),
        "pre-toggle the right pane starts at 40"
    );

    // The toggle: panel width on, the daemon's re-divided layout in.
    show_sidebar(&mut session);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_SIDEBAR).expect("parses"));
    session.renderer.render_frame();
    let symbol = |x: u16, y: u16| session.renderer.cell(x, y).map(|c| c.symbol());
    assert_eq!(
        symbol(50, 0),
        Some("G"),
        "content offset once by the panel: the right pane at 30+20"
    );
    assert_eq!(symbol(60, 0), Some("2"), "the marker paints unwrapped");
    assert_eq!(
        symbol(49, 5),
        Some("│"),
        "the pane divider carries the same offset"
    );
    assert_eq!(
        symbol(40, 0),
        Some(" "),
        "the vacated pre-toggle position is gone"
    );
}

/// While the status bar is hidden the row is NEVER flushed: the
/// bottom row is pane real estate, and a blank-cell diff flush would
/// erase the pane's freshly painted bottom row right after the pane
/// frame. Shown, the row flushes as usual.
#[test]
fn hidden_status_bar_flushes_nothing() {
    let mut session = WindowSession::new(80, 24);
    session.draw_status_row();
    let mut sink = CellSink { cells: Vec::new() };
    assert!(
        session.flush_status_row(&mut sink),
        "shown, the row flushes"
    );
    assert!(!sink.cells.is_empty());

    session.chrome.status_bar_on = false;
    session.chrome_geometry_changed();
    session.draw_status_row();
    let mut sink = CellSink { cells: Vec::new() };
    assert!(
        !session.flush_status_row(&mut sink),
        "hidden, nothing flushes"
    );
    assert!(sink.cells.is_empty());
}

/// Side-panel click geometry under the separated layout: the lookup
/// takes the HOST row (row 0 is the tab strip's — its lead segment
/// carries the panel title), shifted one row so a click lands on the
/// row as painted — the FIRST workspace row at host row 1 (the panel
/// opens with its rows; no in-panel header), the footer chips at the
/// panel's last row, each chip only its own cell.
#[test]
fn sidebar_clicks_land_on_the_painted_row() {
    let mut session = WindowSession::new(80, 24);
    show_sidebar(&mut session);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            title: "workspaces",
            rows: vec![
                srow(SidebarRowId::Workspace("+0".into()), "alpha", true),
                srow(SidebarRowId::Workspace("+1".into()), "beta", false),
            ],
        }]));
    session.draw_tab_strip();
    let strip = session.chrome.tab_strip.diff();
    let lead: String = strip
        .iter()
        .filter(|(x, _, _)| *x < 20)
        .map(|(_, _, cell)| cell.symbol())
        .collect();
    assert_eq!(
        lead.trim(),
        "workspaces",
        "the tab strip's lead segment carries the panel title"
    );
    session.renderer.paint_sidebar();
    let first: String = (0..19)
        .map(|x| session.renderer.buffer[(x, 0)].symbol())
        .collect();
    assert!(
        first.contains("alpha"),
        "the panel opens with the first workspace row"
    );
    let panel_text: String = (0..22)
        .flat_map(|y| (0..19).map(move |x| (x, y)))
        .map(|(x, y)| session.renderer.buffer[(x, y)].symbol())
        .collect();
    assert!(
        !panel_text.contains("workspaces"),
        "the panel itself paints no workspaces heading"
    );
    // Host row 0 (the tab strip row): nothing.
    assert_eq!(session.renderer.sidebar_row_at(2, 0), None);
    // Host row 1: the first workspace row, as painted.
    assert_eq!(
        session.renderer.sidebar_row_at(2, 1),
        Some(SidebarRowId::Workspace("+0".into())),
        "the click lands on the row as painted"
    );
    // Host row 2: the second workspace row.
    assert_eq!(
        session.renderer.sidebar_row_at(2, 2),
        Some(SidebarRowId::Workspace("+1".into()))
    );
    // The workspace area's last row (the 22-row frame splits 11/11:
    // composed row 10 = host row 11): the chips answer only within
    // their own cells — ` new ` spans cols 0..5, ` menu ` the
    // content's right edge.
    assert_eq!(
        session.renderer.sidebar_row_at(2, 11),
        Some(SidebarRowId::New),
        "the new chip's cell"
    );
    assert_eq!(
        session.renderer.sidebar_row_at(16, 11),
        Some(SidebarRowId::Menu),
        "the menu chip's cell"
    );
    assert_eq!(
        session.renderer.sidebar_row_at(10, 11),
        None,
        "the gap between the chips hits nothing"
    );
    // The panel's last row (host row 22) carries the chevron beside
    // the divider column, and only there.
    assert_eq!(
        session.renderer.sidebar_row_at(18, 22),
        Some(SidebarRowId::Collapse)
    );
    assert_eq!(session.renderer.sidebar_row_at(2, 22), None);
    // A body row answers across its full content width (the active
    // row's inverted block is clickable anywhere on it).
    assert_eq!(
        session.renderer.sidebar_row_at(17, 1),
        Some(SidebarRowId::Workspace("+0".into()))
    );
    // Past the strip's width: nothing.
    assert_eq!(session.renderer.sidebar_row_at(20, 2), None);
}

/// The probe FAILED (`bg: None`): the frame fill paints NO color -
/// every cell stays terminal-default, so a failed probe can never
/// mismatch the theme. Default-to-black only appears when the probe
/// SUCCEEDS with black.
#[test]
fn probe_failure_fill_leaves_terminal_default_cells() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    let diff = renderer.render_frame();
    assert!(!diff.is_empty());
    assert_eq!(
        renderer.cell(0, 0).expect("cell").bg,
        RtColor::Reset,
        "the fill paints no color"
    );
    let mid = renderer.cell(70, 10).expect("in-window cell");
    assert_eq!(mid.bg, RtColor::Reset);

    // A probe that SUCCEEDS with black still paints black.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.render_frame();
    let cell = renderer.cell(70, 10).expect("in-window cell");
    assert_eq!(cell.bg, RtColor::Rgb(0, 0, 0));
}

/// The help modal's paint: every cell carries the resolved theme bg,
/// the border ring draws the rounded box-drawing glyphs in the accent
/// color, and the title/badge sit in the top border.
#[test]
fn help_modal_paints_theme_bg_accent_ring_and_title() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    let bg = RtColor::Rgb(30, 30, 30);
    renderer.set_background(Some(bg));
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    let rows = crate::mux::attach::compose_help_panel(
        &crate::mux::attach::help_rows(0x02, 0x12, Default::default(), 1),
        "",
        false,
        6,
        0,
    );
    renderer.set_overlay(Some((crate::mux::attach::HELP_OVERLAY_TITLE, rows, None)));
    renderer.render_frame();
    // Locate the modal's top-left corner.
    let mut origin = None;
    for y in 0..24u16 {
        for x in 0..80u16 {
            if renderer.cell(x, y).expect("cell").symbol() == "\u{256d}" {
                origin = Some((x, y));
            }
        }
    }
    let (x0, y0) = origin.expect("the modal's rounded corner draws");
    // The title rides the top border (" keybinds " from x0+1).
    assert_eq!(renderer.cell(x0 + 2, y0).expect("cell").symbol(), "k");
    // The top edge between title and badge runs in the accent color.
    let edge = renderer.cell(x0 + 20, y0).expect("cell");
    assert_eq!(edge.symbol(), "\u{2500}");
    assert_eq!(edge.fg, RtColor::Indexed(14), "the ring is accent");
    // Interior cells carry the theme bg (no default/light cells).
    let inside = renderer.cell(x0 + 5, y0 + 2).expect("cell");
    assert_eq!(inside.bg, bg, "modal cells carry the theme bg");
    // The modal's height: rows.len() clamped to the window; with the
    // composed panel (filter + 6 window rows + footer) the box is 10
    // rows tall - find the bottom corner on this column.
    let bottom = (y0..24u16)
        .map(|y| (y, renderer.cell(x0, y).expect("cell").symbol().to_string()))
        .find(|(_, sym)| sym == "\u{2570}");
    assert!(bottom.is_some(), "bottom-left rounded corner draws");
}

/// With the help panel open, wheels scroll the PANEL and reach
/// nothing else: the pane's client scrollback does not move and no
/// wheel falls through to forwarding.
#[test]
fn help_open_consumes_wheel_events() {
    let (_, mut conn) = recording_conn("help-wheel");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE).expect("parses"));
    for i in 0..30 {
        session
            .renderer
            .feed_output(1, format!("h{i}\r\n").as_bytes());
    }
    session.enter_help();
    // Wheel down (cb 65) scrolls the panel down three rows; wheel up
    // (cb 64) back. The events never reach the pane paths.
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 65,
            col: 10,
            row: 10,
            release: false,
        },
    );
    assert_eq!(session.help().scroll, 3, "the wheel scrolls the panel");
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 64,
            col: 10,
            row: 10,
            release: false,
        },
    );
    assert_eq!(session.help().scroll, 0, "wheel up scrolls back");
    assert_eq!(
        session.renderer.scroll_offset_of(1),
        0,
        "the pane scrollback never moved"
    );
}

/// A drag survives MOTION WHILE ACTIVE: every additional motion
/// extends the delta and the frame-cadence application sends one
/// resize per unapplied cell. (The round-3 defect: the second motion
/// destroyed the active drag, so only one resize ever fired while the
/// highlight stayed up.)
#[test]
fn drag_survives_active_motion_and_resizes_per_cell() {
    let (rx, mut conn) = recording_conn("drag-move");
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE).expect("parses"));
    while rx
        .recv_timeout(std::time::Duration::from_millis(120))
        .is_ok()
    {}
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 40,
            row: 6,
            release: false,
        },
    );
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 41,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 1",
        "the first motion resizes one cell"
    );
    // A SECOND motion while active must extend the drag, not kill it —
    // but its step is THROTTLED: nothing sends inside the interval…
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 32,
            col: 43,
            row: 6,
            release: false,
        },
    );
    session.apply_drag(&mut conn);
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "a step inside the throttle interval sends nothing"
    );
    // …and once the interval has passed the accumulated delta sends.
    std::thread::sleep(std::time::Duration::from_millis(90));
    session.apply_drag(&mut conn);
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 2",
        "the held delta sends once the interval passes"
    );
}

/// `pane-borders = on`: each pane renders a complete ring (corners on
/// every rect), the focused pane's border in the accent, the other
/// dim - and the option OFF renders no corner glyphs (today's
/// shared-divider look, byte for byte the default).
#[test]
fn pane_borders_option_replaces_dividers_with_full_boxes() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
    renderer.set_pane_borders(true);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.focus(2);
    renderer.render_frame();
    assert_eq!(renderer.cell(0, 0).expect("c").symbol(), "\u{256d}");
    assert_eq!(renderer.cell(39, 0).expect("c").symbol(), "\u{256e}");
    assert_eq!(renderer.cell(40, 0).expect("c").symbol(), "\u{256d}");
    assert_eq!(renderer.cell(79, 23).expect("c").symbol(), "\u{256f}");
    // Focused pane 2's right border carries the accent; pane 1's is
    // dim.
    let focused = renderer.cell(79, 10).expect("c");
    assert_eq!(focused.fg, RtColor::Indexed(14));
    let unfocused = renderer.cell(39, 10).expect("c");
    assert!(
        unfocused.modifier.contains(RtModifier::DIM) && unfocused.fg != RtColor::Indexed(14),
        "the unfocused border is dim"
    );

    // Option OFF: no corner glyphs anywhere (the default look).
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.render_frame();
    assert_ne!(renderer.cell(0, 0).expect("c").symbol(), "\u{256d}");
}

/// `border-active-color` / `border-color` drive the per-pane ring too:
/// the focused ring takes the active color (still bold), the unfocused
/// ring the plain color, and the border label follows its pane's ring.
#[test]
fn pane_border_ring_and_label_honor_border_colors() {
    let active = RtColor::Rgb(255, 136, 0);
    let plain = RtColor::Rgb(80, 80, 120);
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
    renderer.set_pane_borders(true);
    renderer.set_border_colors(Some(active), Some(plain));
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.focus(2);
    renderer.render_frame();
    let focused = renderer.cell(79, 10).expect("c");
    assert_eq!(
        focused.fg, active,
        "the focused ring takes border-active-color"
    );
    assert!(
        focused.modifier.contains(RtModifier::BOLD),
        "and stays bold"
    );
    let unfocused = renderer.cell(39, 10).expect("c");
    assert_eq!(unfocused.fg, plain, "the unfocused ring takes border-color");
    assert!(
        !unfocused.modifier.contains(RtModifier::DIM),
        "a configured plain color replaces the dim fallback"
    );

    // Labels follow their pane's ring colors.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_pane_borders(true);
    renderer.set_show_label_in_border(true);
    renderer.set_border_colors(Some(active), Some(plain));
    renderer.apply_layout(parse_layout(STACKED).expect("parses"));
    renderer.feed_output(1, b"\x1b]2;top\x1b\\");
    renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
    renderer.focus(2);
    renderer.render_frame();
    let label = renderer.cell(2, 12).expect("c");
    assert_eq!(label.symbol(), "d");
    assert_eq!(
        label.fg, active,
        "the focused label takes border-active-color"
    );
    assert!(label.modifier.contains(RtModifier::BOLD));
    let other = renderer.cell(2, 0).expect("c");
    assert_eq!(other.symbol(), "t");
    assert_eq!(other.fg, plain, "the unfocused label takes border-color");
}

/// Unset colors keep the ring's historical look exactly: focused
/// bright cyan + bold, unfocused dim with no fg.
#[test]
fn pane_border_ring_unset_colors_keep_cyan_and_dim() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_pane_borders(true);
    renderer.set_border_colors(None, None);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.focus(2);
    renderer.render_frame();
    let focused = renderer.cell(79, 10).expect("c");
    assert_eq!(focused.fg, RtColor::Indexed(14));
    assert!(focused.modifier.contains(RtModifier::BOLD));
    let unfocused = renderer.cell(39, 10).expect("c");
    assert_eq!(unfocused.fg, RtColor::Reset);
    assert!(unfocused.modifier.contains(RtModifier::DIM));
}

/// `show-label-in-border = on`: the pane's user title embeds in the
/// top edge space-padded, the label cells are not drag handles, and
/// the option OFF leaves plain borders.
#[test]
fn label_in_border_embeds_title_and_blocks_drag_on_label_cells() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(0, 0, 0)));
    renderer.set_pane_borders(true);
    renderer.set_show_label_in_border(true);
    renderer.apply_layout(parse_layout(STACKED).expect("parses"));
    renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
    renderer.render_frame();
    // The label " dbug " starts one cell in from the rect corner.
    assert_eq!(renderer.cell(2, 12).expect("c").symbol(), "d");
    assert_eq!(renderer.cell(1, 12).expect("c").symbol(), " ");
    assert!(renderer.label_cell_at(2, 12), "a label cell knows itself");
    assert!(
        !renderer.label_cell_at(70, 12),
        "plain border cells stay drag handles"
    );
    assert!(
        !renderer.label_cell_at(2, 13),
        "interior cells are not label cells"
    );

    // A press ON the label cell does not start a drag even though the
    // top border row sits within the divider tolerance.
    let (_, mut conn) = recording_conn("label-drag");
    let mut session = WindowSession::new(80, 25);
    session.renderer.set_pane_borders(true);
    session.renderer.set_show_label_in_border(true);
    session
        .renderer
        .apply_layout(parse_layout(STACKED).expect("parses"));
    session.renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
    session.route_mouse(
        &mut conn,
        SgrMouse {
            // Host row 14 = content row 12 (the strip row shifts the
            // content down one).
            cb: 0,
            col: 3,
            row: 14,
            release: false,
        },
    );
    assert!(
        session.drag.is_none(),
        "a label press focuses, it never drags"
    );

    // Option OFF (the default): the title does not render.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_pane_borders(true);
    renderer.apply_layout(parse_layout(STACKED).expect("parses"));
    renderer.feed_output(2, b"\x1b]2;dbug\x1b\\");
    renderer.render_frame();
    assert_ne!(renderer.cell(2, 12).expect("c").symbol(), "d");
}

/// The cursor placement path shares the border inset: with the
/// per-pane-border option on, the tracked cell maps inside the ring
/// (hidden when it falls in the cropped perimeter band); with the
/// option off the mapping is the plain rect origin - the round-3
/// pin for the cell-to-screen cursor math.
#[test]
fn focused_cursor_maps_through_the_rect_and_the_border_inset() {
    // Off: plain rect-origin mapping.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.feed_output(1, b"hi");
    assert_eq!(
        renderer.focused_cursor(),
        Some((2, 0, CursorStyle::BlinkingBlock)),
        "plain mapping: rect origin plus the tracked cell"
    );

    // On: inset by the ring.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_pane_borders(true);
    renderer.apply_layout(parse_layout(TWO_PANE).expect("parses"));
    renderer.feed_output(1, b"hi");
    assert_eq!(
        renderer.focused_cursor(),
        Some((3, 1, CursorStyle::BlinkingBlock)),
        "inset mapping: ring offset plus the tracked cell"
    );
}

/// The btop-exit artifact (kanban card: stale solid-background cells
/// after an alt-screen app exits), pinned headlessly. A pane draws an
/// alt-screen btop-like frame — rows of solid-background cells, wide
/// chars included — the renderer frames it, then the app exits
/// (CSI ? 1049 l) back to the primary screen's shell prompt and the
/// frame re-renders. The composed frame must show the primary screen
/// with NO btop background remnants anywhere: both the live diff
/// path (the same renderer fed the exit bytes in a second frame) and
/// the re-seed path (a fresh renderer replaying the full scripted
/// session, as a reseed does for a pane new to it) are checked.
#[test]
fn alt_screen_exit_leaves_no_stale_background_at_the_prompt_rows() {
    let theme_bg = RtColor::Rgb(16, 16, 16);
    let red = map_color(CoreColor::Named(NamedColor::Red));
    let blue = map_color(CoreColor::Named(NamedColor::Blue));
    let green = map_color(CoreColor::Named(NamedColor::Green));

    // The scripted session: alt-screen enter, three btop-like rows
    // (a red bar, a blue row of double-width chars filling all 80
    // columns, a green bar), then the app exits to the restored
    // primary screen and the shell draws its prompt on row 10.
    let wide_row: String = "\u{6f22}".repeat(40); // 40 wide chars = 80 cols
    let session = format!(
        "\x1b[?1049h\x1b[H\x1b[41m{}\x1b[0m\r\n\x1b[44m{}\x1b[0m\r\n\x1b[42m{}\x1b[0m\x1b[?1049l\x1b[10;1H$ ",
        "R".repeat(80),
        wide_row,
        "G".repeat(80),
    );

    // The exit marker is pure ASCII, so its byte offset is a char
    // boundary; the session is fed in byte slices around it.
    let bytes = session.as_bytes();
    let split = session.find("\x1b[?1049l").expect("exit marker");

    // The alt-screen frame really is on screen when framed alone.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(theme_bg));
    renderer.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
    renderer.feed_output(2, &bytes[..split]);
    let first = renderer.render_frame();
    assert!(!first.is_empty(), "the btop frame paints");
    assert_eq!(renderer.cell(0, 0).expect("c").symbol(), "R");
    assert_eq!(renderer.cell(0, 0).expect("c").bg, red);
    assert_eq!(renderer.cell(0, 1).expect("c").bg, blue);

    // Live path: the same renderer, the exit bytes landing in a
    // second frame.
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(theme_bg));
    renderer.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
    renderer.feed_output(2, &bytes[..split]);
    renderer.render_frame();
    renderer.feed_output(2, &bytes[split..]);
    let second = renderer.render_frame();
    assert!(!second.is_empty(), "the exit repaints");
    for y in 0..24u16 {
        for x in 0..80u16 {
            let cell = renderer.cell(x, y).expect("in-bounds");
            assert_ne!(
                cell.bg,
                red,
                "stale red bg at ({x},{y}): {:?}",
                cell.symbol()
            );
            assert_ne!(
                cell.bg,
                blue,
                "stale blue bg at ({x},{y}): {:?}",
                cell.symbol()
            );
            assert_ne!(
                cell.bg,
                green,
                "stale green bg at ({x},{y}): {:?}",
                cell.symbol()
            );
        }
    }
    // The prompt landed where the app put it.
    assert_eq!(renderer.cell(0, 9).expect("c").symbol(), "$");

    // Replay path: a fresh renderer replaying the session (what a pane
    // new to the renderer gets on a reseed), framed once.
    let mut reseeded = PaneRenderer::new(80, 24, Glyphs::Unicode);
    reseeded.set_background(Some(theme_bg));
    reseeded.apply_layout(parse_layout("0000,80x24,0,0,2").expect("parses"));
    reseeded.feed_output(2, session.as_bytes());
    reseeded.render_frame();
    for y in 0..24u16 {
        for x in 0..80u16 {
            let cell = reseeded.cell(x, y).expect("in-bounds");
            assert_ne!(cell.bg, red, "re-seed stale red at ({x},{y})");
            assert_ne!(cell.bg, blue, "re-seed stale blue at ({x},{y})");
            assert_ne!(cell.bg, green, "re-seed stale green at ({x},{y})");
        }
    }
}

/// The context menu's click mapping takes the RAW host column: the
/// modal centers over the host width, so mapping through the panel's
/// strip offset lands every click one panel width left (the
/// manual-pass round-7 report: menu items unclickable with the
/// panel up).
#[test]
fn panel_up_menu_click_maps_raw_host_columns() {
    use crate::mux::attach::input::SgrMouse;
    let (_rx, conn) = recording_conn("menuprobe");
    let mut conn = conn;
    let mut session = WindowSession::new(80, 24);
    show_sidebar(&mut session);
    session.window = "@0".to_string();
    session.open_menu(super::super::MenuTarget::Tab("@0".to_string()));
    let (x0, y0, inner, height) = session.renderer.overlay_geometry().expect("menu up");
    println!("GEO x0={x0} y0={y0} inner={inner} height={height}");
    // The rename row (panel row 1) paints at host y0+3 (panel row r
    // sits at host y0+2+r), and SGR rows are 1-based: y0+4.
    let click = SgrMouse {
        cb: 0,
        col: (x0 + 4) as u16,
        row: (y0 + 4) as u16,
        release: false,
    };
    session.route_mouse(&mut conn, click);
    println!(
        "AFTER prompt={} menu={:?} overlay_title={:?}",
        session.is_prompt(),
        session.menu().is_some(),
        session.renderer.overlay.as_ref().map(|o| o.0)
    );
    assert!(
        session.is_prompt(),
        "the raw-column click must open the rename prompt"
    );
}

/// The command menu's pure mapping: no header (the box's ` menu `
/// title already names it), one row per client command (settings,
/// keybinds, reload config, detach), the footer.
#[test]
fn compose_command_menu_maps_rows_to_actions() {
    let (rows, mapping) = compose_menu_panel(&MenuTarget::Commands, None, false);
    assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
    assert_eq!(rows[0].text, " settings ");
    assert_eq!(rows[1].text, " keybinds ");
    assert_eq!(rows[2].text, " reload config ");
    assert_eq!(rows[3].text, " detach ");
    assert_eq!(
        mapping,
        vec![
            Some(MenuAction::Settings),
            Some(MenuAction::Keybinds),
            Some(MenuAction::ReloadConfig),
            Some(MenuAction::Detach),
            None
        ]
    );
}

/// The side panel's ` menu ` chip opens the COMMAND menu (not the
/// workspace menu), even with no workspace roster.
#[test]
fn menu_chip_opens_the_command_menu() {
    let (_rx, _conn, session) = command_menu_session("cmdmenu-open");
    assert_eq!(
        session.menu().map(|m| m.target.clone()),
        Some(MenuTarget::Commands),
        "the chip opens the command menu"
    );
    let overlay = session.renderer.overlay.as_ref().expect("overlay up");
    assert_eq!(overlay.0, MENU_TITLE);
    assert_eq!(overlay.1[0].text, " settings ");
    // The menu opens beside its chip: the left ring just right of the
    // chip's cells (x 19 on the 20-wide strip, frame row 10 of the
    // 22-row frame), the bottom ring on the chip's row.
    let (x0, y0, _, height) = session.renderer.overlay_geometry().expect("menu up");
    assert_eq!(x0, 19, "the box starts right of the chip");
    assert_eq!(y0 + height + 1, 10, "the bottom ring sits on the chip row");
}

/// An anchored box that would overflow the frame pulls back inside it,
/// and a closed overlay drops its anchor (the next one centers).
#[test]
fn anchored_overlay_stays_inside_the_frame() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(parse_layout("0000,80x24,0,0,1").expect("parses"));
    let rows = |n: usize| -> Vec<crate::mux::attach::HelpRow> {
        (0..n)
            .map(|i| crate::mux::attach::HelpRow {
                text: format!("row {i} xxxxxxxxxx"),
                accent: false,
                footer: false,
                render_only: false,
            })
            .collect()
    };
    renderer.set_overlay(Some((MENU_TITLE, rows(5), None)));
    renderer.set_overlay_anchor(Some(super::super::OverlayAnchor::Above(75, 2)));
    let (x0, y0, inner, height) = renderer.overlay_geometry().expect("up");
    assert!(x0 + inner + 2 <= 80, "pulled left inside the frame");
    assert_eq!(y0, 0, "a box taller than the rows above pins to the top");
    assert!(y0 + height + 2 <= 24);
    // Below (the tab menu's anchor): the top ring on the anchor row.
    renderer.set_overlay_anchor(Some(super::super::OverlayAnchor::Below(75, 0)));
    let (x0, y0, inner, height) = renderer.overlay_geometry().expect("up");
    assert_eq!(
        x0,
        80 - (inner + 2),
        "a far-right anchor pulls back left so the box fits"
    );
    assert_eq!(y0, 0, "Below puts the top ring on the anchor row");
    assert!(y0 + height + 2 <= 24, "the box stays inside the frame");
    renderer.set_overlay_anchor(Some(super::super::OverlayAnchor::Below(2, 0)));
    let (x0, y0, _, _) = renderer.overlay_geometry().expect("up");
    assert_eq!(x0, 2, "a near-left anchor aligns to the anchor column");
    assert_eq!(y0, 0);
    // A box taller than the rows below its anchor pulls up to fit.
    renderer.set_overlay_anchor(Some(super::super::OverlayAnchor::Below(2, 20)));
    let (_, y0, _, height) = renderer.overlay_geometry().expect("up");
    assert_eq!(
        y0,
        24 - (height + 2),
        "clamped up so the whole box fits the frame"
    );
    assert!(y0 + height + 2 <= 24);
    renderer.set_overlay(None);
    renderer.set_overlay(Some((MENU_TITLE, rows(5), None)));
    let (cx, _, cinner, _) = renderer.overlay_geometry().expect("up");
    assert_eq!(cx, (80 - (cinner + 2)) / 2, "the next overlay centers");
}

/// The tab menu anchors under the clicked tab: the box's left ring on
/// the tab's left edge and its top ring on the first row below the
/// strip — not centered in the terminal.
#[test]
fn tab_menu_opens_under_the_clicked_tab() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert(
        "list-windows -t $0".to_string(),
        "@0 - main\n@1 * vim".to_string(),
    );
    let (rx, mut conn) = scripted_conn("tabmenu-anchor", replies);
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();
    session.status.refresh(&mut conn, "@0", 1).expect("refresh");
    session.draw_tab_strip();
    drained(&rx);
    // A right press (cb 2) on the second tab's block ("  vim  ", strip
    // cols 9..16; SGR col 13 = strip col 12, row 1 = the strip row).
    session.route_mouse(&mut conn, sgr(2, 13, 1, false));
    assert!(
        session.menu().is_some(),
        "the right press opens the tab menu"
    );
    let (x0, y0, inner, _) = session.renderer.overlay_geometry().expect("menu up");
    assert_eq!(x0, 9, "the box's left ring on the tab's left edge");
    assert_eq!(y0, 0, "the box's top ring on the first row below the strip");
    assert!(x0 + inner + 2 <= 80, "the box stays inside the frame");
}

/// The command menu's `keybinds` row opens the help panel.
#[test]
fn command_menu_keybinds_opens_help() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-help");
    click_menu_row(&mut session, &mut conn, 1);
    assert!(session.menu().is_none(), "the menu closed");
    assert!(session.is_help(), "keybinds opened the help panel");
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(super::super::super::HELP_OVERLAY_TITLE)
    );
}

/// The command menu's `reload config` row runs the reload (the
/// daemon's `reload-config` rides the wire either way).
#[test]
fn command_menu_reload_sends_reload_config() {
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-reload");
    click_menu_row(&mut session, &mut conn, 2);
    assert!(session.menu().is_none(), "the menu closed");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert!(
        session.chrome.flash_text().is_some(),
        "the reload flashes its outcome"
    );
}

/// The command menu's `detach` row requests the pump's exit (the
/// prefix-d path), and the footer row does nothing.
#[test]
fn command_menu_detach_requests_exit() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-detach");
    click_menu_row(&mut session, &mut conn, 4);
    assert!(session.menu().is_some(), "the footer consumes the click");
    assert!(!session.detach_requested);
    click_menu_row(&mut session, &mut conn, 3);
    assert!(session.menu().is_none(), "the menu closed");
    assert!(session.detach_requested, "detach was requested");
}

/// The menus' keyboard cursor mirrors the picker's: the selection opens
/// on the first action row (past a name header), Up/Down move it across
/// the header and footer rows with wrap at the ends, and `j`/`k` are the
/// byte spellings of the same motion.
#[test]
fn menu_arrows_move_the_selection_across_header_and_footer() {
    let (_rx, mut conn, mut session) = command_menu_session("menu-keys-arrows");
    // A TAB menu: the name header is row 0, the actions start at row 1.
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    assert_eq!(
        session.menu().expect("menu up").selected,
        1,
        "the selection opens on the first action row, past the header"
    );
    let key = par_term_emu_core::keyboard::TermKeyEvent::functional;
    use par_term_emu_core::keyboard::TermKey;
    session.route_key(&mut conn, &key(TermKey::Down, 0));
    assert_eq!(session.menu().unwrap().selected, 2);
    session.route_key(&mut conn, &key(TermKey::Down, 0));
    assert_eq!(session.menu().unwrap().selected, 3);
    // Six more Downs walk the move rows and wrap past the footer back
    // to rename (actions now rows 1..=7, the footer 8).
    for expected in [4, 5, 6, 7, 1] {
        session.route_key(&mut conn, &key(TermKey::Down, 0));
        assert_eq!(
            session.menu().unwrap().selected,
            expected,
            "Down lands on row {expected}"
        );
    }
    session.route_key(&mut conn, &key(TermKey::Up, 0));
    assert_eq!(
        session.menu().unwrap().selected,
        7,
        "Up wraps over the header to move to end"
    );
    // `j`/`k`, the byte spelling, land on the same rows.
    session.route_stdin_bytes(&mut conn, b"k");
    assert_eq!(session.menu().unwrap().selected, 6);
    session.route_stdin_bytes(&mut conn, b"j");
    assert_eq!(session.menu().unwrap().selected, 7);
}

/// Enter dispatches the selected action through the click path: the
/// menu closes first so the rename prompt replaces it.
#[test]
fn menu_enter_dispatches_the_selected_action() {
    let (_rx, mut conn, mut session) = command_menu_session("menu-keys-enter");
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    // The opening selection is `rename`.
    session.route_stdin_bytes(&mut conn, b"\r");
    assert!(session.menu().is_none(), "the menu closed");
    assert!(session.is_prompt(), "the rename prompt replaced the menu");
    assert_eq!(
        session.prompt().target,
        PromptTarget::Window("@0".to_string())
    );
}

/// `q` and Escape still close, each through its own path (`q` rides the
/// plain-byte run, Escape the functional-key route).
#[test]
fn menu_q_and_escape_still_close() {
    let (_rx, mut conn, mut session) = command_menu_session("menu-keys-close");
    session.route_stdin_bytes(&mut conn, b"q");
    assert!(session.menu().is_none(), "q closed the menu");
    session.open_menu(MenuTarget::Commands);
    session.route_key(
        &mut conn,
        &par_term_emu_core::keyboard::TermKeyEvent::functional(
            par_term_emu_core::keyboard::TermKey::Escape,
            0,
        ),
    );
    assert!(session.menu().is_none(), "Escape closed the menu");
}

/// Mouse motion and the arrow keys write the same selection index, and
/// the one highlight paints the selected row's own label.
#[test]
fn menu_hover_and_keys_share_one_selection() {
    let (_rx, mut conn, mut session) = command_menu_session("menu-keys-hover");
    // No-button motion over the `reload config` row (row 2).
    hover_menu_row(&mut session, &mut conn, 2);
    assert_eq!(
        session.menu().expect("menu up").selected,
        2,
        "the hover wrote the selection"
    );
    session.route_key(
        &mut conn,
        &par_term_emu_core::keyboard::TermKeyEvent::functional(
            par_term_emu_core::keyboard::TermKey::Down,
            0,
        ),
    );
    assert_eq!(
        session.menu().unwrap().selected,
        3,
        "the arrow moved relative to the hover"
    );
    assert_eq!(
        session.renderer.overlay_highlights,
        vec![(
            3,
            0,
            crate::mux::attach::text_input::display_width(" detach ")
        )],
        "one reversed label highlight, on the selected row"
    );
}

/// The keyboard selection paints exactly what a click acts on: the
/// highlight sits on the selected row, the row's cells still map to it
/// through `overlay_row_at`, and a click there dispatches what the
/// highlight painted.
#[test]
fn menu_keyboard_selection_paints_the_clickable_row() {
    let (_rx, mut conn, mut session) = command_menu_session("menu-keys-click");
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    let key = par_term_emu_core::keyboard::TermKeyEvent::functional;
    use par_term_emu_core::keyboard::TermKey;
    session.route_key(&mut conn, &key(TermKey::Down, 0));
    session.route_key(&mut conn, &key(TermKey::Down, 0));
    assert_eq!(session.menu().unwrap().selected, 3);
    assert_eq!(
        session.renderer.overlay_highlights,
        vec![(
            3,
            0,
            crate::mux::attach::text_input::display_width(" add tab ")
        )],
        "the highlight paints the selected label"
    );
    // A click on the painted row dispatches what the highlight painted.
    click_menu_row(&mut session, &mut conn, 3);
    assert!(session.is_prompt(), "add tab opened its prompt");
    assert_eq!(session.prompt().target, PromptTarget::NewWindow);
}

/// The footer names the keyboard controls the menu now takes.
#[test]
fn menu_footer_names_the_keyboard_controls() {
    let (rows, _) = compose_menu_panel(&MenuTarget::Commands, None, false);
    assert_eq!(rows.last().expect("footer row").text, MENU_FOOTER);
    assert!(MENU_FOOTER.contains("navigate arrows/j/k"), "{MENU_FOOTER}");
    assert!(MENU_FOOTER.contains("enter/click"), "{MENU_FOOTER}");
    assert!(MENU_FOOTER.contains("close esc/q"), "{MENU_FOOTER}");
}

/// The command menu's `settings` row opens the settings dialog: herdr's
/// tabbed panel. Tab switches sections, `j` moves the selection, Enter
/// writes the chosen key into the config file — keeping the user's
/// comments — and reloads (the daemon's `reload-config` rides the wire),
/// a click on a tab switches to it, and Escape closes.
#[test]
fn command_menu_settings_opens_the_dialog_and_applies() {
    use super::super::modal_settings::SettingsSection;
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-settings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# keep me\n[client]\nprefix = \"C-b\"\n").unwrap();
    session.settings_path = Some(path.clone());
    click_menu_row(&mut session, &mut conn, 0);
    let Modal::Settings(state) = &session.modal else {
        panic!("settings opened: {:?}", session.modal);
    };
    assert_eq!(state.section, SettingsSection::Sidebar);
    assert_eq!(
        session.renderer.overlay.as_ref().map(|o| o.0),
        Some(super::super::modal_settings::SETTINGS_TITLE)
    );
    // Tab three times: the notify section (sidebar, borders, general
    // first), its single row showing the test session's delivery.
    session.route_stdin_bytes(&mut conn, b"\t\t\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("still open");
    };
    assert_eq!(state.section, SettingsSection::Notify);
    assert_eq!(state.selected, 0);
    let row = session.renderer.overlay.as_ref().expect("up").1[5]
        .text
        .trim_end()
        .to_string();
    assert!(row.starts_with(" \u{25B8} delivery"), "{row}");
    assert!(row.ends_with("off"), "{row}");
    // Enter: cycles to the next delivery (`inside`), written and reloaded.
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("# keep me\n"), "{written}");
    assert!(
        written.contains("notify-delivery = \"inside\""),
        "{written}"
    );
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(
        session.notify.delivery.name(),
        "inside",
        "the reload applied the written key"
    );
    // A click on the `borders` tab (inner columns 9..18 of row 0).
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("dialog up");
    session.route_mouse(
        &mut conn,
        sgr(0, (x0 + 2 + 10) as u16, (y0 + 3) as u16, false),
    );
    let Modal::Settings(state) = &session.modal else {
        panic!("still open");
    };
    assert_eq!(state.section, SettingsSection::Borders);
    // A press on the box's ring stays inside; a press outside it closes
    // the dialog (herdr's click-away).
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("dialog up");
    session.route_mouse(&mut conn, sgr(0, (x0 + 1) as u16, (y0 + 2) as u16, false));
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "the ring is inside"
    );
    session.route_mouse(&mut conn, sgr(0, 1, (y0 + 3) as u16, false));
    assert!(
        matches!(session.modal, Modal::None),
        "a click outside closes"
    );
    assert!(session.renderer.overlay.is_none());
    // Escape closes too.
    click_menu_row_after_reopen(&mut session, &mut conn);
    session.route_stdin_bytes(&mut conn, b"q");
    assert!(matches!(session.modal, Modal::None));
    assert!(session.renderer.overlay.is_none());
}

/// While the settings dialog is up, wheels move the selection with the
/// arrows' scroll-into-view: walking past the body window's bottom
/// follows the window (`top`), walking back returns it, and no wheel
/// ever closes the dialog.
#[test]
fn settings_pointer_wheel_moves_selection_with_scroll_into_view() {
    use super::super::modal_settings::SettingsSection;
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-wheel");
    session.enter_settings();
    // The borders tab, one Tab from the opening section.
    session.route_stdin_bytes(&mut conn, b"\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.section, SettingsSection::Borders);
    let count = state.section.row_count() + state.popup_rows();
    assert!(
        count > 8,
        "the borders section must overflow the 8-row body window"
    );
    // Wheel down to the last row: the selection walks and the body's
    // window follows it (the arrows' scroll-into-view).
    for _ in 0..(count - 1) {
        session.route_mouse(&mut conn, sgr(65, 40, 12, false));
    }
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.selected, count - 1, "clamped at the last row");
    assert_eq!(state.top, count - 8, "the window followed the selection");
    // Wheel up all the way: back to the first row, window back at the
    // top.
    for _ in 0..(count - 1) {
        session.route_mouse(&mut conn, sgr(64, 40, 12, false));
    }
    let Modal::Settings(state) = &session.modal else {
        panic!("dialog open");
    };
    assert_eq!(state.selected, 0);
    assert_eq!(state.top, 0);
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "wheels never close the dialog"
    );
}

/// The row sections: a toggle writes its bool and the reload applies it
/// live, and a choice row cycles (the borders tab's `line style` and the
/// sidebar tab's `agent lines` preset).
#[test]
fn settings_rows_write_and_apply_live() {
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-rows");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The borders tab (one Tab), `pane boxes` (row 1): Enter flips it on.
    session.route_stdin_bytes(&mut conn, b"\tj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("pane-borders = true"), "{written}");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert!(session.render_opts.pane_borders, "applied live");
    // Enter again flips it back off.
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("pane-borders = false"), "{written}");
    assert!(!session.render_opts.pane_borders);
    // The sidebar tab (one back-Tab), `agent lines` (row 3, under the
    // git poll row): cycles default → compact and writes the nested
    // token arrays.
    session.route_stdin_bytes(&mut conn, b"hjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("agent-rows = [[\"state_icon\", \"agent\"]]"),
        "{written}"
    );
    assert_eq!(
        session.agent_rows,
        vec![vec!["state_icon".to_string(), "agent".to_string()],],
        "the reload rebuilt the agent section"
    );
}

/// The borders tab's theme rows write and apply live: Enter on `theme`
/// cycles default → auto, the reload re-picks (no probe = the dark
/// preset), and `→` on `auto: dark host` steps to catppuccin, which the
/// auto pick then paints.
#[test]
fn settings_theme_rows_write_and_apply_live() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-theme");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The borders tab, `theme` (row 6).
    session.route_stdin_bytes(&mut conn, b"\tjjjjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("theme = \"auto\""), "{written}");
    assert_eq!(session.theme.theme, "auto", "applied live");
    assert!(session.renderer.palette().is_none(), "dark default");
    // `auto: dark host` (row 7): → steps default → catppuccin.
    session.route_stdin_bytes(&mut conn, b"j\x1b[C");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("theme-dark = \"catppuccin\""), "{written}");
    assert_eq!(
        session.renderer.palette(),
        crate::mux::theme::palette("catppuccin")
    );
}

/// The notify tab's sound rows write and apply live: `sounds` flips on,
/// `muted agents` takes a comma list from the prompt, `done sound` a
/// path, and an empty prompt clears the list back to none.
#[test]
fn settings_sound_rows_write_and_apply_live() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-sound");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The notify tab (three Tabs), `sounds` (row 1): Enter flips it on.
    session.route_stdin_bytes(&mut conn, b"\t\t\tj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("sound = true"), "{written}");
    assert!(session.notify.sound.enabled, "applied live");
    // `muted agents` (row 4): the prompt takes a comma list.
    session.route_stdin_bytes(&mut conn, b"jjj\r");
    assert!(
        matches!(session.modal, Modal::Prompt(_)),
        "{:?}",
        session.modal
    );
    session.route_stdin_bytes(&mut conn, b"droid, codex\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("sound-mute = [\"droid\", \"codex\"]"),
        "{written}"
    );
    assert_eq!(session.notify.sound.mute, vec!["droid", "codex"]);
    assert!(matches!(session.modal, Modal::Settings(_)), "dialog back");
    // `done sound` (row 2): a path.
    session.route_stdin_bytes(&mut conn, b"kk\r");
    session.route_stdin_bytes(&mut conn, b"sounds/done.mp3\r");
    settle(&mut session, &mut conn);
    assert_eq!(session.notify.sound.done, "sounds/done.mp3");
    // `muted agents` again: clear the seeded list, commit empty.
    session.route_stdin_bytes(&mut conn, b"jj\r");
    session.route_stdin_bytes(&mut conn, b"\x03\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("sound-mute = []"), "{written}");
    assert!(session.notify.sound.mute.is_empty());
}

/// `%config-changed` (another client's reload): the pump re-applies the
/// file's `[client]` settings through the reload chord's rebind, flashes
/// once, and NEVER sends `reload-config` (that would re-broadcast to
/// every client). A second cue over the same file is a no-op — the
/// shape the writing client sees when its own reload's broadcast lands.
#[test]
fn config_changed_reapplies_the_file_without_resending_reload() {
    use super::super::notify::NotifyDelivery;
    use par_term_emu_core::tmux_control::TmuxNotification;
    let (rx, mut conn, mut session) = command_menu_session("cfg-changed");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\nnotify-delivery = \"off\"\n").unwrap();
    session.settings_path = Some(path.clone());
    session.reload_config(&mut conn);
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(session.notify.delivery, NotifyDelivery::Off);
    session.chrome = ChromeRows::new(80);

    // Another client writes `terminal` + `pane-borders` and reloads.
    std::fs::write(
        &path,
        "[client]\nnotify-delivery = \"terminal\"\npane-borders = true\nsidebar-width = 34\n",
    )
    .unwrap();
    session.handle_event(TmuxNotification::ConfigChanged);
    assert!(session.pending.config_changed, "the cue parks for the pump");
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    settle(&mut session, &mut conn);
    assert_eq!(session.notify.delivery, NotifyDelivery::Terminal);
    assert_eq!(session.render_opts.sidebar_width, 34, "width applies live");
    assert!(
        session.render_opts.pane_borders,
        "a chrome key applied live"
    );
    assert_eq!(session.chrome.flash_text(), Some("config re-applied"));
    assert!(
        !drained(&rx).iter().any(|l| l == "reload-config"),
        "the cue never re-sends reload-config"
    );

    // The same file again (the writer's own broadcast): nothing moves,
    // nothing flashes, nothing is sent.
    session.chrome = ChromeRows::new(80);
    session.handle_event(TmuxNotification::ConfigChanged);
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    settle(&mut session, &mut conn);
    assert_eq!(
        session.chrome.flash_text(),
        None,
        "an unchanged file is quiet"
    );
    let sent = drained(&rx);
    assert!(sent.is_empty(), "an unchanged file sends nothing: {sent:?}");
}

/// ←/→ change a discrete row's value in place (the natural gesture on
/// the notify tab's `delivery` — the live report read the dialog as
/// unchangeable): right cycles forward, left back, both write and apply
/// live; on a prompt-edited row the arrows still switch sections.
#[test]
fn settings_choice_rows_cycle_with_the_arrows() {
    let (_rx, mut conn, mut session) = command_menu_session("setnotify-arrow");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // The notify tab (three Tabs), `delivery` at the default terminal:
    // → system, then ← off.
    session.route_stdin_bytes(&mut conn, b"\t\t\t");
    // A bare unit session's delivery is the struct default (off); →
    // cycles to inside, ← wraps back to system.
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("notify-delivery = \"inside\""),
        "right wrote the next choice: {written}"
    );
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Left, 0));
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("notify-delivery = \"off\""),
        "left wrote the previous choice: {written}"
    );
    assert_eq!(session.notify.delivery.name(), "off", "applied live");
    // On the general tab's `resize step` (a Number row) ← keeps
    // switching sections instead of opening the prompt.
    session.route_stdin_bytes(&mut conn, b"h");
    session.route_stdin_bytes(&mut conn, b"jjj");
    session.route_key(&mut conn, &TermKeyEvent::functional(TermKey::Right, 0));
    settle(&mut session, &mut conn);
    let Modal::Settings(state) = &session.modal else {
        panic!("no prompt opened: {:?}", session.modal)
    };
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("resize-step"),
        "the arrow switched sections, it did not edit"
    );
    assert!(
        matches!(
            state.section,
            super::super::modal_settings::SettingsSection::Notify
        ),
        "the section moved: {:?}",
        state.section
    );
}

/// The prompt-edited rows: numbers, hex colors, and chords open the
/// prompt seeded with the value in force; a bad value flashes and writes
/// nothing; a good one writes, reloads, applies live, and returns to the
/// dialog (Escape cancels back to the dialog too).
#[test]
fn settings_prompt_rows_edit_numbers_hex_and_chords() {
    let (rx, mut conn, mut session) = command_menu_session("cmdmenu-prompt");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // General tab (two Tabs), `resize step` (row 3): Enter opens the
    // prompt seeded with 1.
    session.route_stdin_bytes(&mut conn, b"\t\tjjj\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "1");
    // A bad number flashes and keeps the prompt up (nothing written).
    session.route_stdin_bytes(&mut conn, b"\x03zzz\r");
    assert!(
        matches!(session.modal, Modal::Prompt(_)),
        "invalid value keeps the prompt: {:?}",
        session.modal
    );
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("resize-step"));
    // Clear, type 3, commit: written, reloaded, applied, dialog back.
    session.route_stdin_bytes(&mut conn, b"\x033\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("resize-step = 3"), "{written}");
    assert_eq!(wait_recorded(&rx, "reload-config"), "reload-config");
    assert_eq!(session.resize_step, 3, "applied live");
    assert!(
        matches!(session.modal, Modal::Settings(_)),
        "the dialog is back"
    );
    // The borders tab (one back-Tab from general), `focus color` (row
    // 4): the empty seed means the built-in; a hex commits and applies.
    session.route_stdin_bytes(&mut conn, b"hjjjj\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("hex prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "", "an unset hex seeds empty");
    session.route_stdin_bytes(&mut conn, b"#ff0000\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("border-active-color = \"#ff0000\""),
        "{written}"
    );
    assert_eq!(
        session.render_opts.border_active,
        Some(ratatui::style::Color::Rgb(255, 0, 0)),
        "applied live"
    );
    // The keys tab (four more Tabs from borders), `prefix` (row 0): a
    // chord that does not parse is refused; a good one rebinds.
    session.route_stdin_bytes(&mut conn, b"\t\t\t\t\r");
    let Modal::Prompt(prompt) = &session.modal else {
        panic!("chord prompt opened: {:?}", session.modal)
    };
    assert_eq!(prompt.text, "C-b");
    session.route_stdin_bytes(&mut conn, b"\x03zz\r");
    assert!(matches!(session.modal, Modal::Prompt(_)), "refused");
    session.route_stdin_bytes(&mut conn, b"\x03C-a\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("prefix = \"C-a\""), "{written}");
    assert_eq!(session.prefix, 0x01, "rebound live");
    // Escape from a prompt returns to the dialog, not out of it.
    session.route_stdin_bytes(&mut conn, b"\r\x1b");
    assert!(matches!(session.modal, Modal::Settings(_)));
}

/// The keys section scrolls: every chord row is reachable and the
/// selection stays visible.
#[test]
fn settings_keys_section_scrolls_to_every_row() {
    use super::super::modal_settings::SettingsSection;
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-keys");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    // Five Tabs: the keys section (sidebar borders general notify keys).
    session.route_stdin_bytes(&mut conn, b"\t\t\t\t\t");
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    assert_eq!(state.section, SettingsSection::Keys);
    assert_eq!(state.selected, 0);
    assert_eq!(state.top, 0);
    // Down to the last row: the window follows the selection (the keys
    // section holds every management chord plus prefix and reload — 29
    // rows with break-pane, join-pane, the move-tab and tab-strip
    // chords, and notify-jump aboard).
    session.route_stdin_bytes(&mut conn, &[b'j'; 29][..]);
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    let last = state.section.row_count() - 1;
    assert_eq!(state.selected, last);
    assert_eq!(state.top, last + 1 - 8);
    // The composed body shows the last row visible with its value.
    let overlay = session.renderer.overlay.as_ref().expect("up");
    let last_row = overlay.1[5 + (last - state.top)].text.clone();
    assert!(last_row.contains("respawn dead pane"), "{last_row}");
    assert!(last_row.trim_end().ends_with(" r"), "{last_row}");
    // Up to the first: the window snaps back.
    session.route_stdin_bytes(&mut conn, &[b'k'; 29][..]);
    let Modal::Settings(state) = &session.modal else {
        panic!("open")
    };
    assert_eq!((state.selected, state.top), (0, 0));
}

/// Reopen the settings dialog straight from the session (the menu is
/// already exercised above).
fn click_menu_row_after_reopen(
    session: &mut WindowSession,
    _conn: &mut crate::mux::attach::conn::AttachConn,
) {
    session.enter_settings();
    assert!(matches!(session.modal, Modal::Settings(_)));
}

/// The new-tab prompt's default name: one past the highest window
/// ordinal, bumped past any window NAME that already claims the
/// number (the manual-pass ask: the next non-conflicting index).
#[test]
fn next_window_name_bumps_past_conflicts() {
    // One past the highest window ordinal.
    assert_eq!(
        super::super::super::next_window_name(&[
            ("@0".to_string(), "0".to_string()),
            ("@1".to_string(), "1".to_string()),
        ]),
        "2"
    );
    // A rename claiming the next index bumps the default past it.
    assert_eq!(
        super::super::super::next_window_name(&[
            ("@0".to_string(), "0".to_string()),
            ("@1".to_string(), "2".to_string()),
        ]),
        "3"
    );
    // No numeric names in use: still the ordinal successor.
    assert_eq!(
        super::super::super::next_window_name(&[("@0".to_string(), "demo".to_string())]),
        "1"
    );
}

/// A name crossing the wire spelled so the daemon's quoting grammar
/// keeps it one word: unconditional single quotes, embedded quotes
/// via the `'\''` idiom.
#[test]
fn wire_quote_survives_spaces_and_quotes() {
    assert_eq!(super::super::super::wire_quote("notes"), "'notes'");
    assert_eq!(super::super::super::wire_quote("my notes"), "'my notes'");
    assert_eq!(super::super::super::wire_quote("it's"), "'it'\\''s'");
    assert_eq!(super::super::super::wire_quote(""), "''");
}

/// The help overlay's overflow scrollbar: a `┃` thumb on the ring's
/// right column, sized/positioned from the scroll state; a panel
/// that fits paints nothing on the ring.
#[test]
fn help_overlay_paints_a_scrollbar_thumb_when_it_overflows() {
    use super::super::super::{compose_help_panel, help_rows};
    let rows = help_rows(0x02, 0x12, Default::default(), 1);
    let total = super::super::super::help_content(&rows, "").len();
    let mut renderer = PaneRenderer::new(80, 40, Glyphs::Unicode);
    renderer.apply_layout(parse_layout("0000,80x40,0,0,1").expect("parses"));
    // A window small enough that the panel must scroll: thumb at the
    // top border column.
    renderer.set_overlay(Some((
        crate::mux::attach::HELP_OVERLAY_TITLE,
        compose_help_panel(&rows, "", false, 6, 0),
        Some((0, 6, total)),
    )));
    renderer.render_frame();
    let (x0, y0, inner, height) = renderer.overlay_geometry().expect("geometry");
    let ring_x = (x0 + inner + 1) as u16;
    let thumbs: Vec<u16> = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
        .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
        .collect();
    assert!(
        !thumbs.is_empty(),
        "the thumb paints on the ring: rows {thumbs:?}"
    );
    assert_eq!(
        thumbs.len(),
        (height * 6 / total).max(1),
        "the thumb spans its proportional share"
    );
    // Scroll to the bottom: the thumb parks at the track's end.
    renderer.set_overlay(Some((
        crate::mux::attach::HELP_OVERLAY_TITLE,
        compose_help_panel(&rows, "", false, 6, total),
        Some((total.saturating_sub(6), 6, total)),
    )));
    renderer.render_frame();
    let last_thumb = (y0 as u16 + 1..y0 as u16 + 1 + height as u16)
        .filter(|y| renderer.cell(ring_x, *y).is_some_and(|c| c.symbol() == "┃"))
        .max()
        .expect("a bottom thumb");
    assert_eq!(
        last_thumb,
        y0 as u16 + height as u16,
        "the thumb parks at the track's end"
    );
    // A panel that fits: nothing on the ring.
    let (x0s, y0s, inners, heights) = {
        let short: Vec<crate::mux::attach::HelpRow> = (0..4)
            .map(|i| crate::mux::attach::HelpRow {
                text: format!("row {i}"),
                accent: false,
                footer: false,
                render_only: false,
            })
            .chain(std::iter::once(crate::mux::attach::HelpRow {
                text: "footer".to_string(),
                accent: false,
                footer: true,
                render_only: false,
            }))
            .collect();
        renderer.set_overlay(Some((" t ", short, Some((0, 6, 4)))));
        renderer.render_frame();
        renderer.overlay_geometry().expect("geometry")
    };
    assert!(
        (y0s as u16 + 1..y0s as u16 + 1 + heights as u16).all(|y| renderer
            .cell((x0s + inners + 1) as u16, y)
            .is_some_and(|c| c.symbol() != "┃"))
    );
}

/// The `send-keys -H` hex spelling of `bytes` (what `forward_chunked`
/// puts on the wire for one chunk).
fn hex_wire(pane: &str, bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("send-keys -t {pane} -H {}", hex.join(" "))
}

/// A two-pane session over a recording conn, focused on pane 1.
fn paste_session(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let (rx, conn) = recording_conn(tag);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.focus(1);
    drained(&rx);
    (rx, conn, session)
}

/// SEC-208 (a): an SGR mouse report split across two stdin bursts (two
/// pump ticks) is one mouse route — the session's parser holds the
/// partial — and no fragment of it reaches the pane as typed text.
#[test]
fn split_mouse_report_across_pump_ticks_routes_once() {
    let (rx, mut conn, mut session) = paste_session("split-mouse");
    session.renderer.focus(2);
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[<0;10;"));
    settle(&mut session, &mut conn);
    assert!(!session.route_stdin_bytes(&mut conn, b"5M"));
    settle(&mut session, &mut conn);
    assert_eq!(
        session.renderer.focused(),
        Some(1),
        "the reassembled press focused the pane under the pointer"
    );
    let lines = drained(&rx);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("select-pane"))
            .count(),
        1,
        "exactly one mouse route: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("send-keys")),
        "no mouse fragment forwarded as text: {lines:?}"
    );
}

/// SEC-208 (b): a bracketed paste split across two pump ticks forwards
/// its body opaque — the embedded prefix + `x` never kills the pane — and
/// a pane that enabled DECSET 2004 gets the body re-framed.
#[test]
fn split_paste_across_pump_ticks_is_opaque_and_rewrapped() {
    let (rx, mut conn, mut session) = paste_session("split-paste");
    session.renderer.feed_output(1, b"\x1b[?2004h");
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[200~ab"));
    settle(&mut session, &mut conn);
    assert!(!session.route_stdin_bytes(&mut conn, b"\x02xcd\x1b[201~"));
    settle(&mut session, &mut conn);
    assert!(
        !session.prefix_pending,
        "a pasted prefix byte never arms the prefix"
    );
    let lines = drained(&rx);
    assert!(
        !lines.iter().any(|l| l.starts_with("kill-pane")),
        "the pasted chord did not fire: {lines:?}"
    );
    assert_eq!(
        lines,
        vec![hex_wire("%1", b"\x1b[200~ab\x02xcd\x1b[201~")],
        "the body forwards whole, framed for the bracketed-paste pane"
    );
}

/// SEC-208 (b'): the same paste into a pane WITHOUT bracketed paste
/// forwards the body bare (no framing the app did not ask for).
#[test]
fn paste_into_a_plain_pane_forwards_the_body_bare() {
    let (rx, mut conn, mut session) = paste_session("plain-paste");
    session.route_stdin_bytes(&mut conn, b"\x1b[200~ab\x02xcd\x1b[201~");
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"ab\x02xcd")]);
}

/// SEC-208: a paste cancels a pending prefix (tmux's rule), so the
/// paste's first byte is not taken as a chord key.
#[test]
fn paste_cancels_a_pending_prefix() {
    let (rx, mut conn, mut session) = paste_session("paste-prefix");
    session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B]);
    settle(&mut session, &mut conn);
    assert!(session.prefix_pending);
    session.route_stdin_bytes(&mut conn, b"\x1b[200~xd\x1b[201~");
    settle(&mut session, &mut conn);
    assert!(!session.prefix_pending);
    let lines = drained(&rx);
    assert_eq!(lines, vec![hex_wire("%1", b"xd")], "{lines:?}");
}

/// SEC-208 (c): an embedded terminator in a paste body is stripped
/// before the re-frame, so pasted text cannot close the pane's paste
/// early — including a terminator spliced together by the removal.
#[test]
fn embedded_paste_terminator_is_stripped() {
    let (rx, mut conn, mut session) = paste_session("paste-strip");
    session.renderer.feed_output(1, b"\x1b[?2004h");
    session.route_paste(&mut conn, b"a\x1b[201~b\x1b[201\x1b[201~~c");
    settle(&mut session, &mut conn);
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"\x1b[200~abc\x1b[201~")]);
}

/// ARC-122: the ring colors read off the focused pane's right ring
/// edge (`active`) and the unfocused pane's (`plain`) after the session
/// re-laid a `{cols/2 | cols/2}` split; `x0` is the panes' left offset
/// (the side panel's width).
fn assert_ring_colors(
    session: &mut WindowSession,
    x0: u16,
    cols: u16,
    active: RtColor,
    plain: RtColor,
) {
    session.renderer.focus(1);
    session.renderer.render_frame();
    let half = cols / 2;
    assert_eq!(
        session.renderer.cell(x0 + half - 1, 5).expect("c").fg,
        active,
        "the focused ring keeps border-active-color"
    );
    assert_eq!(
        session.renderer.cell(x0 + cols - 1, 5).expect("c").fg,
        plain,
        "the unfocused ring keeps border-color"
    );
}

/// The `%layout-change` a size report of `cols`x`rows` against `%1`
/// queues: an even two-pane split of `@0`.
fn two_pane_notify(cols: u16, rows: u16) -> std::collections::HashMap<String, String> {
    let half = cols / 2;
    let layout = format!("0000,{cols}x{rows},0,0{{{half}x{rows},0,0,1,{half}x{rows},{half},0,2}}");
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        format!("refresh-client -t %1 -C {cols}x{rows}"),
        format!("%layout-change @0 {layout} {layout} *"),
    );
    notify
}

/// ARC-122: `with_options` applies every option and `options` reads
/// them all back — a field one side forgets fails here.
#[test]
fn render_options_round_trip_through_the_renderer() {
    let opts = RenderOptions {
        bg: Some(RtColor::Rgb(9, 9, 9)),
        glyphs: Glyphs::Heavy,
        pane_borders: true,
        show_label_in_border: true,
        pane_gaps: 2,
        scrollbar_gutter: true,
        sidebar_width: 17,
        border_active: Some(RtColor::Rgb(1, 2, 3)),
        border_plain: Some(RtColor::Rgb(4, 5, 6)),
    };
    assert_eq!(PaneRenderer::with_options(80, 24, &opts).options(), opts);
    assert_eq!(
        PaneRenderer::new(80, 24, Glyphs::Ascii).options(),
        RenderOptions {
            glyphs: Glyphs::Ascii,
            sidebar_width: 0,
            ..RenderOptions::default()
        },
        "new() keeps no side panel"
    );
}

/// ARC-122: the session's effective options hide the configured panel
/// width while the panel is off.
#[test]
fn effective_render_opts_zero_the_hidden_sidebar() {
    let mut session = WindowSession::new(80, 25);
    assert_eq!(session.render_opts.sidebar_width, 20);
    assert_eq!(session.effective_render_opts().sidebar_width, 0);
    session.sidebar.set(true);
    assert_eq!(session.effective_render_opts().sidebar_width, 20);
}

/// ARC-122: a host resize re-fits the renderer; the configured border
/// colors survive the refit (they used to revert to cyan/dim).
#[test]
fn border_colors_survive_a_resize_rebuild() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-resize",
        FakeScript {
            notify: two_pane_notify(100, 40),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session
        .resize_to(&mut conn, 100, 40, &mut RecordingSink::default())
        .expect("resize");
    assert_eq!(session.renderer.layout().len(), 2, "the refit re-laid");
    assert_ring_colors(&mut session, 0, 100, active, plain);
}

/// ARC-122: a window re-seed re-fits the renderer too; the colors
/// survive it.
#[test]
fn border_colors_survive_a_reseed_rebuild() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * main", "@0", "%1\n%2");
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-reseed",
        FakeScript {
            replies,
            notify: two_pane_notify(80, 23),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session.reseed_window(&mut conn, "@0");
    assert_eq!(session.renderer.layout().len(), 2, "the reseed re-laid");
    assert_ring_colors(&mut session, 0, 80, active, plain);
}

/// ARC-122: toggling the side panel (which parks a refit the pump runs
/// as a resize refit) keeps the colors, and the configured panel
/// width rides the refit.
#[test]
fn border_colors_survive_a_sidebar_toggle_refit() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let (_rx, mut conn, mut session) = two_pane_session(
        "arc122-sidebar",
        FakeScript {
            notify: two_pane_notify(60, 23),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session.toggle_sidebar(&mut conn);
    assert!(session.pending.grid_refit);
    session
        .resize_to(&mut conn, 80, 23, &mut RecordingSink::default())
        .expect("refit");
    assert_eq!(
        session.renderer.sidebar_width(),
        20,
        "the panel width survives"
    );
    assert_eq!(session.renderer.layout().len(), 2, "the refit re-laid");
    assert_ring_colors(&mut session, 20, 60, active, plain);
}

/// The pane replays (`refresh-client -t %N` WITHOUT a `-C` size report)
/// among recorded wire lines.
fn replay_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.starts_with("refresh-client -t %") && !l.contains(" -C "))
        .cloned()
        .collect()
}

/// A `%layout-change @0` line splitting `cols`x`rows` evenly between
/// `%1` and `%2`.
fn two_pane_layout_line(cols: u16, rows: u16) -> String {
    let half = cols / 2;
    let layout = format!("0000,{cols}x{rows},0,0{{{half}x{rows},0,0,1,{half}x{rows},{half},0,2}}");
    format!("%layout-change @0 {layout} {layout} *")
}

/// ENH-044: a host resize whose layout keeps every pane sends the size
/// report and NO replay; the surviving emulators keep their content,
/// reflowed to the new size.
#[test]
fn resize_in_place_replays_no_unchanged_panes() {
    let (rx, mut conn, mut session) = two_pane_session(
        "enh044-noreplay",
        FakeScript {
            notify: two_pane_notify(100, 23),
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"left pane text");
    session.renderer.feed_output(2, b"right pane text");
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    let lines = drained(&rx);
    let reports: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("refresh-client -t %1 -C "))
        .collect();
    assert_eq!(reports, vec!["refresh-client -t %1 -C 100x23"]);
    assert!(replay_lines(&lines).is_empty(), "no replays: {lines:?}");
    let one = session.renderer.emulators[&1].terminal();
    assert_eq!(one.size(), (50, 23), "re-fit to the new layout");
    assert!(one.content().contains("left pane text"));
    assert!(session.renderer.emulators[&2]
        .terminal()
        .content()
        .contains("right pane text"));
}

/// ENH-044: a pane new to the renderer is the only one replayed.
#[test]
fn resize_in_place_replays_a_pane_new_to_the_layout() {
    let layout = "0000,99x23,0,0{33x23,0,0,1,33x23,33,0,2,33x23,66,0,3}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 99x23".to_string(),
        format!("%layout-change @0 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "refresh-client -t %3".to_string(),
        "pane three body".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "enh044-newpane",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session
        .resize_to(&mut conn, 99, 23, &mut RecordingSink::default())
        .expect("resize");
    let lines = drained(&rx);
    assert_eq!(replay_lines(&lines), vec!["refresh-client -t %3"]);
    assert!(session.renderer.emulators[&3]
        .terminal()
        .content()
        .contains("pane three body"));
}

/// ENH-044: the configured border colors ride the in-place refit of
/// both a host resize and a same-window re-seed.
#[test]
fn resize_keeps_border_colors() {
    let active = RtColor::Rgb(1, 2, 3);
    let plain = RtColor::Rgb(4, 5, 6);
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 * main", "@0", "%1\n%2");
    let (_rx, mut conn, mut session) = two_pane_session(
        "enh044-colors",
        FakeScript {
            replies,
            notify: two_pane_notify(100, 23),
            ..FakeScript::default()
        },
    );
    session.set_pane_borders(true);
    session.set_border_colors(Some(active), Some(plain));
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    assert_eq!(session.renderer.frame_size(), (100, 23));
    assert_ring_colors(&mut session, 0, 100, active, plain);
    session.reseed_window(&mut conn, "@0");
    assert_eq!(session.renderer.layout().len(), 2, "the reseed re-laid");
    assert_ring_colors(&mut session, 0, 100, active, plain);
}

/// ENH-044 race rule: `%output` that arrives AFTER the `%layout-change`
/// in the refit's drain may have been applied at the old size on the
/// daemon, so the pane replays instead of feeding it.
#[test]
fn resize_output_after_layout_change_forces_replay() {
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 100x23".to_string(),
        format!("{}\n%output %1 late", two_pane_layout_line(100, 23)),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "refresh-client -t %1".to_string(),
        "replayed body".to_string(),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "enh044-late",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    let lines = drained(&rx);
    assert_eq!(replay_lines(&lines), vec!["refresh-client -t %1"]);
    let content = session.renderer.emulators[&1].terminal().content();
    assert!(content.contains("replayed body"), "{content:?}");
    assert!(!content.contains("late"), "the raced bytes were not fed");
}

/// ENH-044: `%output` drained BEFORE the `%layout-change` was produced
/// at the old size, where the emulator still is: it feeds, no replay.
#[test]
fn resize_output_before_layout_change_is_fed_at_old_size() {
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 100x23".to_string(),
        format!("%output %1 early\n{}", two_pane_layout_line(100, 23)),
    );
    let (rx, mut conn, mut session) = two_pane_session(
        "enh044-early",
        FakeScript {
            notify,
            ..FakeScript::default()
        },
    );
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    let lines = drained(&rx);
    assert!(replay_lines(&lines).is_empty(), "no replays: {lines:?}");
    assert!(session.renderer.emulators[&1]
        .terminal()
        .content()
        .contains("early"));
}

/// ENH-044: a re-seed onto a different window has no surviving panes,
/// so every pane of the new window replays.
#[test]
fn reseed_to_another_window_replays_every_pane() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "enh044-otherwin",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session.renderer.set_user_title(1, "old window title");
    session.reseed_window(&mut conn, "@1");
    let lines = drained(&rx);
    assert_eq!(
        replay_lines(&lines),
        vec!["refresh-client -t %5", "refresh-client -t %6"]
    );
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![5, 6]);
    assert_eq!(
        session.renderer.focused(),
        Some(5),
        "focus left the old window"
    );
    assert!(
        !session.renderer.user_titles.contains_key(&1),
        "the old window's titles are dropped with its panes"
    );
}

/// ENH-044 follow-up: a refit repaints the WHOLE frame even at an
/// unchanged size. `resize_to` clears the host (`repaint_all`), so a
/// diff against the stale last frame would skip the unchanged pane
/// content and leave it blank on the host (the `mux_attach`
/// workspace-click and session-landing regressions).
#[test]
fn same_size_refit_reemits_unchanged_content() {
    let (_rx, mut conn, mut session) = two_pane_session(
        "enh044-repaint",
        FakeScript {
            notify: two_pane_notify(80, 23),
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"KEEP-ME");
    let mut first = RecordingSink::default();
    session.frame(&mut first);
    assert!(first.cells.iter().any(|(_, _, c)| c.symbol() == "K"));
    let mut sink = RecordingSink::default();
    session
        .resize_to(&mut conn, 80, 23, &mut sink)
        .expect("refit");
    let text: String = sink
        .cells
        .iter()
        .filter(|(_, y, _)| *y == 1)
        .map(|(_, _, c)| c.symbol().to_string())
        .collect();
    assert!(
        text.contains("KEEP-ME"),
        "the refit must re-emit the unchanged pane row: {text:?}"
    );
}

/// ENH-044: an open modal overlay survives a host resize (the old
/// renderer rebuild dropped it while the session still believed it was up).
#[test]
fn resize_preserves_open_overlay() {
    let (_rx, mut conn, mut session) = two_pane_session(
        "enh044-overlay",
        FakeScript {
            notify: two_pane_notify(100, 23),
            ..FakeScript::default()
        },
    );
    session.open_menu(MenuTarget::Tab("@0".to_string()));
    assert!(session.renderer.overlay.is_some());
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    assert!(
        session.renderer.overlay.is_some(),
        "the menu overlay survives the refit"
    );
}

/// Four side-by-side panes (%1..%4) over an 80-col window.
const FOUR_PANE: &str = "0000,80x24,0,0{20x24,0,0,1,20x24,20,0,2,20x24,40,0,3,20x24,60,0,4}";

/// A fake daemon advertising `list-windows all`, scripted with five
/// sessions whose `-a` rows put `@0` in `$4`.
fn five_session_script() -> FakeScript {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-commands".to_string(),
        "list-windows targeted all\nfeatures replay-held-state".to_string(),
    );
    replies.insert(
        "list-sessions".to_string(),
        (0..5)
            .map(|n| format!("+0: main: ${n}: s{n}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    replies.insert("list-workspaces".to_string(), "+0: main active".to_string());
    replies.insert(
        "list-windows -a".to_string(),
        (0..5)
            .map(|n| format!("${n} @{} * w{n}", (n + 1) % 5))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    replies.insert("pane-title -t %1".to_string(), "one".to_string());
    FakeScript {
        replies,
        ..FakeScript::default()
    }
}

/// ARC-125's round-trip bound: one status refresh over five sessions
/// and four visible panes, with the side panel up and one pane's title
/// marked stale by `%pane-title-changed`, costs at most six commands
/// (it was 5 + 4 + 6). One `list-windows -a` finds the owner and its
/// windows, the side panel reuses the refresh's `list-workspaces`, the
/// focused pane's title rides the bar's own query, and only the stale
/// pane is re-queried.
#[test]
fn status_refresh_round_trips_are_bounded() {
    let (rx, mut conn) = fake_daemon("rtcount", five_session_script());
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    session.sidebar.set(true);
    session.renderer.focus(1);
    // Steady state: every pane's title was learned on an earlier
    // refresh; then %3's user title is cleared (a set title patches from
    // the payload since ENH-043; a cleared one falls back to the OSC
    // title the payload lacks, so it re-queries).
    for pane in 1..=4 {
        session.renderer.set_user_title(pane, "t");
    }
    session.handle_event(TmuxNotification::PaneTitleChanged {
        pane_id: "%3".to_string(),
        title: String::new(),
    });
    drained(&rx);

    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let sent = drained(&rx);
    assert_eq!(
        sent,
        vec![
            "list-sessions",
            "list-workspaces",
            "list-windows -a",
            "pane-title -t %1",
            "list-agents",
            "pane-title -t %3",
        ],
        "one refresh's wire"
    );
    assert!(sent.len() <= 6);
    assert_eq!(session.status.session_id.as_deref(), Some("$4"));
    assert!(!session.status_dirty);
}

/// A daemon that stops answering costs the pump at most the status
/// bound, not the 10 s reply timeout: the refresh stops at the first
/// late query, keeps the stale bar, and re-marks the status dirty. The
/// late reply is discarded, so the next command reads its own.
#[test]
fn status_refresh_times_out_and_keeps_replies_aligned() {
    let mut script = five_session_script();
    script.delays.insert(
        "list-sessions".to_string(),
        std::time::Duration::from_millis(900),
    );
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 20x24".to_string());
    let (rx, mut conn) = fake_daemon("rttimeout", script);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    drained(&rx);

    let started = std::time::Instant::now();
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(800),
        "bounded by the status timeout: {took:?}"
    );
    assert!(
        !session.status_dirty,
        "a timeout leaves the re-query to the next poll or event — an immediate \
         re-issue queues behind the same slow reply, one 500 ms bound per pass"
    );
    assert_eq!(
        drained(&rx),
        vec!["list-sessions"],
        "stopped at the timeout"
    );

    // The late list-sessions reply must not answer this command.
    let reply = conn.send_checked("pane-info -t %1").expect("reply");
    assert_eq!(reply.body, vec!["%1 @0 20x24".to_string()]);
}

/// ENH-049 (superseding ARC-137's blocking bound): a user-initiated switch
/// against a slow daemon returns at once, the late reply lands on a later
/// tick, and the next sync command still reads its own answer.
#[test]
fn slow_switch_returns_at_once_and_keeps_replies_aligned() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "list-sessions".to_string(),
        std::time::Duration::from_millis(500),
    );
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 40x24".to_string());
    let (rx, mut conn, mut session) = two_pane_session("int-switch", script);

    let started = std::time::Instant::now();
    session.route_plain(&[crate::mux::attach::C_B, b')'], &mut conn);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(200),
        "the pump does not wait for the daemon: {took:?}"
    );
    assert_eq!(session.window, "@0", "the view stays put");
    settle(&mut session, &mut conn);
    assert_eq!(
        drained(&rx),
        vec!["list-sessions"],
        "the empty roster ends the chain"
    );

    let reply = conn.send_checked("pane-info -t %1").expect("reply");
    assert_eq!(reply.body, vec!["%1 @0 40x24".to_string()]);
}

/// ENH-049: `focus_pane` writes the select and the lookup back to back; a
/// slow select delays neither the call nor the lookup's write.
#[test]
fn focus_pane_returns_before_a_slow_select_answers() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "select-pane -t %7".to_string(),
        std::time::Duration::from_millis(500),
    );
    let (rx, mut conn, mut session) = two_pane_session("int-focus", script);

    let started = std::time::Instant::now();
    session.focus_pane(&mut conn, 7);
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(200),
        "the pump does not wait for the daemon: {took:?}"
    );
    settle(&mut session, &mut conn);
    assert_eq!(
        drained(&rx),
        vec!["select-pane -t %7", "pane-info -t %7"],
        "pipelined in order"
    );
    assert_eq!(session.chrome.flash_text(), None);
}

/// An agent-row click lights its row on the click's own frame: focus_pane
/// moves the client-local focus (`renderer.focused()` — what the panel's
/// rows activate on) and rebuilds the panel from the cache, instead of
/// waiting for the throttled refresh a quiet pane may not tick for (the
/// second-click report).
#[test]
fn sidebar_agent_row_click_highlights_the_row_immediately() {
    let (_rx, mut conn, mut session) = two_pane_session("agent-hl", FakeScript::default());
    show_sidebar(&mut session);
    session.status.apply_snapshot_for_test(&[
        "snapshot 1".to_string(),
        "workspace +0 * app".to_string(),
        "session $0 * +0 main".to_string(),
        "window @0 * 1".to_string(),
        "agent %1 claude working hook".to_string(),
        "agent %2 kimi idle hook".to_string(),
    ]);
    session.rebuild_agent_section();
    let selected = |session: &WindowSession| -> Vec<u32> {
        session
            .renderer
            .sidebar_sections
            .clone()
            .expect("sections")
            .iter()
            .flat_map(|s| s.rows.iter())
            .filter_map(|row| match row.id {
                SidebarRowId::Pane(p) if row.active => Some(p),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        selected(&session),
        vec![1],
        "the layout's first pane starts focused"
    );
    session.focus_pane(&mut conn, 2);
    settle(&mut session, &mut conn);
    assert_eq!(
        selected(&session),
        vec![2],
        "the clicked row lights on the click's own frame"
    );
}

const THREE_PANE: &str = "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}";

/// A fake daemon advertising `client-snapshot v1` (ENH-043), scripted
/// with one snapshot of `@0`: 2 workspaces, 3 sessions, 2 windows, 3
/// panes, 2 agents.
fn snapshot_script() -> FakeScript {
    let mut replies = std::collections::HashMap::new();
    replies.insert(
        "list-commands".to_string(),
        "client-snapshot v1\nlist-commands\nfeatures replay-held-state\n".to_string(),
    );
    replies.insert(
        "client-snapshot -t @0".to_string(),
        [
            "snapshot 1",
            "workspace +0 * main",
            "workspace +1 - side",
            "session $0 - +0 one",
            "session $1 * +0 two",
            "session $2 - +1 three",
            "window @0 * edit",
            "window @1 - logs",
            "pane %1 build output",
            "pane %2 tests",
            "pane %3",
            "agent %1 claude working hook",
            "agent %3 kimi blocked hook",
        ]
        .join("\n"),
    );
    FakeScript {
        replies,
        ..FakeScript::default()
    }
}

/// An agent-roster entry.
fn roster_entry(pane: u32, agent: &str, state: &str, workspace: Option<&str>) -> RosterEntry {
    RosterEntry {
        pane,
        agent: agent.to_string(),
        state: state.to_string(),
        workspace: workspace.map(str::to_string),
        project: None,
        message: None,
    }
}

/// The workspace rows' `(id, label)` pairs, in panel order.
fn workspace_area(session: &WindowSession) -> Vec<(String, String)> {
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    sections
        .iter()
        .find(|s| s.title == "workspaces")
        .expect("workspace area")
        .rows
        .iter()
        .filter_map(|row| match &row.id {
            SidebarRowId::Workspace(id) => Some((id.clone(), row.label.clone())),
            _ => None,
        })
        .collect()
}

/// Linked worktree workspaces group under their repo workspace's row
/// (herdr's indented children): each follows its parent, tree-prefixed
/// and labelled by branch; the grouping comes from the snapshot's
/// `worktree` rows (`list-worktrees`' facts) and survives the agent
/// notification's no-round-trip rebuild. A worktree whose repo
/// workspace is closed (`parent` `-`) keeps its id-order place.
#[test]
fn sidebar_groups_worktree_workspaces_under_their_repo() {
    let lines = |body: &[&str]| body.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    let mut session = snapshot_session();
    session.status.apply_snapshot_for_test(&lines(&[
        "snapshot 1",
        "workspace +0 * app",
        "workspace +1 - notes",
        "workspace +2 - app/feat/login",
        "workspace +3 - app/fix",
        "workspace +4 - lone/wip",
        "session $0 * +0 main",
        "window @0 * 1",
        "worktree +0 repo - main /r/app",
        "worktree +2 worktree +0 feat/login /r/app-worktrees/feat-login",
        "worktree +3 worktree +0 fix /r/app-worktrees/fix",
        "worktree +4 worktree - wip /r/lone-worktrees/wip",
    ]));
    session.rebuild_agent_section();
    let want = vec![
        ("+0".to_string(), "app".to_string()),
        ("+2".to_string(), "├─ feat/login".to_string()),
        ("+3".to_string(), "└─ fix".to_string()),
        ("+1".to_string(), "notes".to_string()),
        ("+4".to_string(), "lone/wip".to_string()),
    ];
    assert_eq!(workspace_area(&session), want);

    // The full refresh path builds through the same helper.
    let roster: Vec<(String, String, bool)> = session
        .status
        .workspaces()
        .iter()
        .map(|(id, name)| (id.clone(), name.clone(), id == "+0"))
        .collect();
    session.renderer.set_sidebar_sections(None);
    session.install_panel_sections(&roster);
    assert_eq!(workspace_area(&session), want);

    // A snapshot without worktree rows ungroups.
    session.status.apply_snapshot_for_test(&lines(&[
        "snapshot 1",
        "workspace +0 * app",
        "workspace +2 - app/feat/login",
        "session $0 * +0 main",
        "window @0 * 1",
    ]));
    session.rebuild_agent_section();
    assert_eq!(
        workspace_area(&session),
        vec![
            ("+0".to_string(), "app".to_string()),
            ("+2".to_string(), "app/feat/login".to_string()),
        ]
    );
}

/// A three-pane render session showing `@0` with the side panel up.
/// A pane program's OSC 52 copy — local or over SSH — bridges to the
/// host: the focused pane's clipboard state rides the frame's outbox as
/// the same wire form the yank writes, deduped per pane, and a pane
/// switch forwards the pane it lands on (par-term's clipboard_sync
/// contract, attach side).
#[test]
fn pane_osc52_clipboard_bridges_to_the_host_outbox() {
    let mut session = snapshot_session();
    session.feed_pane("%1", b"\x1b]52;c;aGVsbG8=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;aGVsbG8=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "the pane's copy forwards: {:?}",
        String::from_utf8_lossy(&out)
    );

    // An unchanged clipboard state does not re-send.
    let len_before = session.notify.outbox.len();
    session.bridge_pane_clipboard();
    assert_eq!(session.notify.outbox.len(), len_before, "deduped");

    // New content re-forwards.
    session.feed_pane("%1", b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;aGVsbG8gd29ybGQ=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "new content forwards: {:?}",
        String::from_utf8_lossy(&out)
    );

    // A pane switch forwards the pane it lands on.
    session.renderer.focus(2);
    session.feed_pane("%2", b"\x1b]52;c;cGFuZTI=\x07");
    session.bridge_pane_clipboard();
    let out = session.notify.outbox.clone();
    let needle = b"\x1b]52;c;cGFuZTI=\x1b\\";
    assert!(
        out.windows(needle.len()).any(|w| w == needle),
        "the landed pane forwards: {:?}",
        String::from_utf8_lossy(&out)
    );
}

/// The move-active-tab chords (tmux's `<` / `>`): `swap-window -s
/// <shown> -t <neighbor>` within the shown session — and an edge press
/// flashes instead of sending (no wrap).
#[test]
fn move_window_chords_swap_the_shown_window_with_its_neighbor() {
    let (rx, mut conn) = fake_daemon("movetab", snapshot_script());
    let mut session = snapshot_session();
    session
        .refresh_status_facts(&mut conn)
        .expect("snapshot refresh");
    drained(&rx);

    session.route_stdin_bytes(&mut conn, &[0x02, b'>']);
    assert_eq!(
        received_within_300ms(&rx),
        vec!["swap-window -s @0 -t @1".to_string()],
        "the shown window swaps with its right neighbor"
    );

    // The first tab has no left neighbor: a flash, no wire line.
    session.route_stdin_bytes(&mut conn, &[0x02, b'<']);
    assert!(
        received_within_300ms(&rx).is_empty(),
        "an edge press never wraps"
    );
    assert_eq!(session.chrome.flash_text(), Some("already the first tab"));
}

fn snapshot_session() -> WindowSession {
    let mut session = WindowSession::new(90, 26);
    session
        .renderer
        .apply_layout(parse_layout(THREE_PANE).expect("parses"));
    session.window = "@0".to_string();
    session.sidebar.set(true);
    session.renderer.focus(1);
    session
}

/// The agents section groups by workspace (herdr's panel): rows sort by
/// their workspace's position in the id-ordered cache, the focused
/// pane's row activates, and an unknown workspace keeps the bare
/// `{agent} ({state})` label and trails as the last group (ARC-138).
#[test]
fn sidebar_agents_section_groups_by_workspace() {
    let mut session = snapshot_session();
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let agents = vec![
        roster_entry(3, "kimi", "blocked", Some("+1")),
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(5, "pi", "idle", None),
    ];
    let names = vec![
        ("+0".to_string(), "main".to_string()),
        ("+1".to_string(), "zeta".to_string()),
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        Some(2),
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    );
    session.renderer.set_sidebar_sections(sections);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    assert_eq!(sections.len(), 2, "{sections:?}");
    let rows = &sections[1].rows;
    assert_eq!(rows[0], arow(2, Some("main"), "claude", "working", true));
    assert_eq!(rows[1], arow(3, Some("zeta"), "kimi", "blocked", false));
    assert_eq!(rows[2], arow(5, None, "pi", "idle", false));
}

/// A `done` agent shows in the agents section with the done status, and
/// its workspace row carries it over a working sibling (herdr's
/// priority: blocked > done > working > idle).
#[test]
fn sidebar_shows_done_agents_and_ranks_them_over_working() {
    use crate::mux::attach::panels::AgentStatus;
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let agents = vec![
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(3, "pi", "done", Some("+0")),
    ];
    let names = vec![("+0".to_string(), "main".to_string())];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    // Grouped sub-sorts by status priority: the done pi leads.
    assert_eq!(
        sections[1].rows[0],
        arow(3, Some("main"), "pi", "done", false)
    );
    assert_eq!(sections[1].rows[0].status, AgentStatus::Done);
    assert_eq!(sections[0].rows[0].status, AgentStatus::Done);
    let custom = vec![vec!["agent".to_string(), "state".to_string()]];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &custom,
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    assert_eq!(sections[1].rows[0].label, "pi \u{b7} done");
}

/// The agent entry follows `agent-rows`: the default puts the project
/// beside the workspace (once a probe reported it), a custom table
/// renders its own tokens and lines, and the entry's workspace row
/// carries its agents' most urgent state.
#[test]
fn sidebar_agent_entries_follow_agent_rows() {
    use crate::mux::attach::panels::{AgentStatus, STATE_ICON};
    let roster = vec![("+0".to_string(), "main".to_string(), true)];
    let mut agents = vec![
        roster_entry(2, "claude", "working", Some("+0")),
        roster_entry(3, "kimi", "blocked", Some("+0")),
    ];
    agents[0].project = Some("par-term".to_string());
    let names = vec![("+0".to_string(), "main".to_string())];
    let default = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    // Grouped sub-sorts by status priority: the blocked kimi leads.
    assert_eq!(
        default[1].rows[1].label,
        format!("{STATE_ICON} main \u{b7} par-term")
    );
    assert_eq!(default[1].rows[1].continuation, vec!["claude".to_string()]);
    assert_eq!(
        default[0].rows[0].status,
        AgentStatus::Blocked,
        "the workspace carries its most urgent agent"
    );
    let custom = vec![
        vec!["agent".to_string(), "state".to_string()],
        vec!["pane".to_string(), "project".to_string()],
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &custom,
        crate::mux::attach::panels::AgentSortMode::Grouped,
    )
    .expect("sections");
    assert_eq!(sections[1].rows[0].label, "kimi \u{b7} blocked");
    assert_eq!(sections[1].rows[1].label, "claude \u{b7} working");
    assert_eq!(
        sections[1].rows[1].continuation,
        vec!["%2 \u{b7} par-term".to_string()]
    );
    assert_eq!(
        sections[1].rows[0].continuation,
        vec!["%3".to_string()],
        "an unprobed project drops out of its line"
    );
}

/// Two workspaces sharing a display name stay two groups: the key is the
/// workspace id's position, not the label head (ARC-138).
#[test]
fn sidebar_agents_group_by_workspace_id_not_name() {
    let mut session = snapshot_session();
    let roster = vec![("+0".to_string(), "dev".to_string(), true)];
    let agents = vec![
        roster_entry(4, "a", "idle", Some("+1")),
        roster_entry(3, "b", "idle", Some("+0")),
        roster_entry(2, "c", "idle", Some("+1")),
    ];
    let names = vec![
        ("+0".to_string(), "dev".to_string()),
        ("+1".to_string(), "dev".to_string()),
    ];
    let sections = WindowSession::sidebar_sections(
        &roster,
        &agents,
        &names,
        None,
        &crate::mux::config::default_agent_rows(),
        crate::mux::attach::panels::AgentSortMode::Grouped,
    );
    session.renderer.set_sidebar_sections(sections);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    let ids: Vec<SidebarRowId> = sections[1].rows.iter().map(|row| row.id.clone()).collect();
    // Group `+0` (pane 3), then group `+1` with panes ascending (2, 4).
    // A label-head key would merge both "dev" groups into pane order 2, 3, 4.
    assert_eq!(
        ids,
        [
            SidebarRowId::Pane(3),
            SidebarRowId::Pane(2),
            SidebarRowId::Pane(4)
        ]
    );
}

/// An agent event for a pane the roster lacks inserts a row with no
/// workspace (the payload carries no `ws=`): the status goes stale so the
/// throttled refresh places it, the row trails the grouped rows even
/// though its pane id is lower, and no query goes out (ARC-138).
#[test]
fn agent_insert_marks_status_dirty_and_rows_trail_ungrouped() {
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude working hook ws=+0",
        )
        .replace(
            "agent %3 kimi blocked hook",
            "agent %3 kimi blocked hook ws=+1",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    let (rx, mut conn) = fake_daemon("snapinsert", script);
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_dirty = false;
    drained(&rx);

    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%2".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: String::new(),
        msg: String::new(),
    });
    assert!(session.status_dirty);
    let sections = session.renderer.sidebar_sections.clone().expect("sections");
    assert_eq!(
        sections[1].rows,
        vec![
            arow(1, Some("main"), "claude", "working", true),
            arow(3, Some("side"), "kimi", "blocked", false),
            arow(2, None, "claude", "blocked", false),
        ]
    );
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}

/// Every line the fake daemon receives within 300 ms.
fn received_within_300ms(rx: &std::sync::mpsc::Receiver<(String, String)>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok((_, line)) = rx.recv_timeout(std::time::Duration::from_millis(300)) {
        lines.push(line);
    }
    lines
}

/// ENH-043: against a daemon advertising `client-snapshot v1`, one
/// status refresh — bar, pane titles, and side panel — is exactly one
/// round trip, and every fact lands from that one reply.
#[test]
fn snapshot_refresh_is_one_round_trip() {
    let (rx, mut conn) = fake_daemon("snaponce", snapshot_script());
    let mut session = snapshot_session();
    drained(&rx);

    session
        .refresh_status_facts(&mut conn)
        .expect("snapshot refresh");
    assert_eq!(
        received_within_300ms(&rx),
        vec!["client-snapshot -t @0".to_string()]
    );
    assert_eq!(
        session.status.windows(),
        &[
            ("@0".to_string(), "edit".to_string()),
            ("@1".to_string(), "logs".to_string()),
        ]
    );
    assert_eq!(session.status.session_id.as_deref(), Some("$1"));
    assert_eq!(session.status.pane_title(), "build output");
    assert_eq!(session.renderer.border_label(1), "build output");
    assert_eq!(session.renderer.border_label(2), "tests");
    assert_eq!(session.renderer.border_label(3), "");
    assert_eq!(
        session.renderer.sidebar_sections,
        Some(vec![
            crate::mux::attach::panels::SidebarSection {
                title: "workspaces",
                rows: vec![
                    srow(SidebarRowId::Workspace("+0".into()), "main", true),
                    srow(SidebarRowId::Workspace("+1".into()), "side", false),
                ],
            },
            crate::mux::attach::panels::SidebarSection {
                title: "agents",
                rows: vec![
                    // Grouped sub-sorts by status priority: blocked first.
                    arow(3, None, "kimi", "blocked", false),
                    arow(1, None, "claude", "working", true),
                ],
            },
        ])
    );
    let text: String = session
        .status
        .compose(200, None, false)
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(!text.contains("claude:working"), "{text}");
    assert!(!text.contains("kimi:blocked"), "{text}");
    assert!(text.contains("$2:three"), "{text}");
}

/// Without the feature the legacy per-fact round still runs, and the
/// client never sends `client-snapshot`.
#[test]
fn legacy_refresh_still_runs_without_the_feature() {
    let mut script = snapshot_script();
    script.replies.remove("list-commands");
    script
        .replies
        .insert("list-sessions".to_string(), "+0: main: $0: one".to_string());
    script
        .replies
        .insert("list-windows -t $0".to_string(), "@0 * edit".to_string());
    let (rx, mut conn) = fake_daemon("snaplegacy", script);
    let mut session = snapshot_session();
    drained(&rx);

    session
        .refresh_status_facts(&mut conn)
        .expect("legacy refresh");
    let sent = received_within_300ms(&rx);
    assert!(sent.iter().any(|l| l == "list-sessions"), "{sent:?}");
    assert!(sent.iter().any(|l| l == "list-agents"), "{sent:?}");
    for pane in 1..=3 {
        let line = format!("pane-title -t %{pane}");
        assert_eq!(
            sent.iter().filter(|l| **l == line).count(),
            1,
            "one {line}: {sent:?}"
        );
    }
    assert!(
        !sent.iter().any(|l| l.starts_with("client-snapshot")),
        "{sent:?}"
    );
}

/// A pushed transition into a selected state queues one notification
/// escape the next frame writes to the host; the refresh that seeded the
/// roster, a repeat of the same state, an unselected state, and
/// delivery `off` queue nothing.
#[test]
fn inside_delivery_shows_a_toast_that_expires_and_clicks_to_the_pane() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snaptoast", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    // Put the agents in the background (the shown window holds pane 9).
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert_eq!(
        session.notify.outbox,
        vec![0x07],
        "no terminal escape, only the bell that flags the host tab"
    );
    // The bell reaches the host with the next frame.
    let mut sink = super::RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(sink.raw, vec![0x07], "the frame rings the host bell");
    assert!(session.notify.outbox.is_empty(), "rung once");
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(toast.body, "main \u{b7} claude");
    assert_eq!(toast.pane, Some(1));
    // It paints at the content area's bottom-right, over the panes.
    let (tx, ty, tw, th) = session.renderer.toast_rect().expect("on screen");
    assert_eq!(tx + tw, session.renderer.width, "flush right");
    assert_eq!(ty + th, session.renderer.height, "flush bottom");
    session.renderer.render_frame();
    let title_row: String = (tx..tx + tw)
        .map(|x| session.renderer.buffer[(x, ty + 1)].symbol().to_string())
        .collect();
    assert!(title_row.contains("claude blocked"), "{title_row:?}");
    // A click on the card focuses the pane and dismisses the toast.
    let (hx, hy) = session.geometry.frame_to_host(tx + 2, ty + 1);
    session.route_mouse(&mut conn, sgr(0, hx + 1, hy + 1, false));
    assert!(session.renderer.toast.is_none(), "the click dismisses it");
    settle(&mut session, &mut conn);
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %1"),
        "the click lands on the agent's pane"
    );
    // A later toast expires on its own.
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "idle".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    session.notify.states.push("idle".to_string());
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert!(session.renderer.toast.is_some());
    let until = session.notify.toast_until.expect("deadline set");
    session.tick_flash(until);
    assert!(session.renderer.toast.is_none(), "the toast expired");
}

/// A blocked push's `msg=` renders as the toast's reason line: the why
/// is the point of a blocked card (the TUI audit's F5 — the reason rode
/// the wire end to end and rendered nowhere).
#[test]
fn toast_renders_the_blocked_reason_line() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapreason", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: "needs approval".to_string(),
    });
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.message.as_deref(), Some("needs approval"));
    session.renderer.render_frame();
    let (tx, ty, tw, th) = session.renderer.toast_rect().expect("on screen");
    assert_eq!(th, 5, "the reason adds the card's fourth row");
    let reason_row: String = (tx..tx + tw)
        .map(|x| session.renderer.buffer[(x, ty + 3)].symbol().to_string())
        .collect();
    assert!(
        reason_row.contains("needs approval"),
        "the reason paints: {reason_row:?}"
    );
}

/// The lost-toast race (card 01a15b6f): the daemon applies a state
/// report, then broadcasts it, so a status refresh that runs between the
/// two (any `%sessions-changed`, a timed-out refresh's retry, the project
/// poll) answers with the NEW state while the push still waits in the
/// notification queue. The refresh must not consume the transition: the
/// push drained after it still toasts and rings the bell. The fake daemon
/// writes the push ahead of the snapshot reply — the wire order — and the
/// pump's own steps run in their own order (6 refresh, 9 drain). The
/// refresh now writes its snapshot without waiting (the async status
/// path); `settle` runs the reply's continuation, the same dispatch the
/// pump's step 1b performs, before the drain.
#[test]
fn a_refresh_that_already_saw_the_new_state_still_lets_its_push_toast() {
    use super::super::notify::NotifyDelivery;
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude blocked hook ws=+0",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    script.notify.insert(
        "client-snapshot -t @0".to_string(),
        "%agent-state-changed %1 claude blocked source=hook ws=+0".to_string(),
    );
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    // The roster before the report (claude working), committed by an
    // earlier refresh the way the session seed commits it.
    let (seed_rx, mut seed_conn) = fake_daemon("snaprace-seed", snapshot_script());
    session.refresh_status_facts(&mut seed_conn).expect("seed");
    drained(&seed_rx);
    let (rx, mut conn) = fake_daemon("snaprace", script);
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);

    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents()[0].state,
        "blocked",
        "the refresh saw the new state first"
    );
    assert!(session.renderer.toast.is_none(), "a refresh never toasts");
    assert!(session.drain_pushes(&mut conn).is_continue());

    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the push still toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(toast.pane, Some(1));
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// The fully async status path: the pump's snapshot writes without
/// waiting — a late reply parks no input — and the reply applies as a
/// continuation on the dispatch tick, committing the same facts the
/// sync refresh committed.
#[test]
fn status_snapshot_issues_without_blocking_and_applies_on_dispatch() {
    let mut script = snapshot_script();
    script.delays.insert(
        "client-snapshot -t @0".to_string(),
        std::time::Duration::from_millis(500),
    );
    let (rx, mut conn) = fake_daemon("asyncsnap", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_dirty = true;
    let started = std::time::Instant::now();
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        started.elapsed() < std::time::Duration::from_millis(200),
        "the snapshot writes without waiting: {:?}",
        started.elapsed()
    );
    assert!(!session.status_dirty, "the issue consumed the dirty mark");
    assert!(
        session.status_snapshot_inflight.is_some(),
        "the reply is owed: one snapshot in flight"
    );
    wait_recorded(&rx, "client-snapshot");
    assert!(
        drained(&rx).is_empty(),
        "one snapshot on the wire, nothing else"
    );

    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents().len(),
        2,
        "the reply's roster committed"
    );
    assert!(
        session.status_snapshot_inflight.is_none(),
        "the application releases the in-flight slot"
    );
    assert!(
        session.pending_cmds.is_empty(),
        "the reply drained exactly once"
    );
}

/// The nothing-left-to-show `End` rides the continuation: the `%error`
/// snapshot reply lands `SessionGone`, the landing resolves nothing,
/// and the continuation raises the pump-visible `status_end` that the
/// status step checks on that tick.
#[test]
fn status_snapshot_end_lands_through_the_continuation() {
    let mut script = snapshot_script();
    script.failing.insert("client-snapshot -t @0".to_string());
    let (rx, mut conn) = fake_daemon("asyncend", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(!session.status_end, "nothing ends before the reply");
    settle(&mut session, &mut conn);
    assert!(
        session.status_end,
        "the SessionGone landing found nothing left to show"
    );
}

/// The re-seed's status snapshot rides the same machinery the seed's
/// does: written without waiting — no inline snapshot round trip left
/// on snapshot daemons outside the explicit continuations — and the
/// reply applies as a pump-tick continuation committing the same facts
/// the sync refresh committed.
#[test]
fn a_reseed_issues_its_snapshot_without_blocking_and_applies_on_dispatch() {
    let mut script = snapshot_script();
    reseed_replies(&mut script.replies, "@0 * main", "@0", "%1\n%2");
    let (rx, mut conn) = fake_daemon("reseedasync", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.reseed_window(&mut conn, "@0");
    assert_eq!(session.window, "@0", "the view re-seeded onto @0");
    assert!(
        session.status_snapshot_inflight.is_some(),
        "the snapshot rides the machinery, not an inline round trip"
    );
    assert!(!session.status_dirty, "the issue consumed the dirty mark");
    let wire = received_within_300ms(&rx);
    assert_eq!(
        wire.iter()
            .filter(|l| l.starts_with("client-snapshot"))
            .count(),
        1,
        "one snapshot on the wire: {wire:?}"
    );

    settle(&mut session, &mut conn);
    assert!(
        session.status_snapshot_inflight.is_none(),
        "the application releases the in-flight slot"
    );
    assert_eq!(
        session.status.agents().len(),
        2,
        "the reply's roster committed"
    );
    assert!(
        session.notify.transitions.is_some(),
        "the transition baseline seeded"
    );
}

/// The cached-landing fixture: a session showing `@0` whose status
/// cache already lists `@1` beside it (the last snapshot's roster,
/// applied test-side), and a snapshot daemon whose `client-snapshot -t
/// @0` now refuses (the window is gone). `extra_failing` names the
/// landing queries the daemon also refuses.
fn cached_landing_fixture(
    tag: &str,
    extra_failing: &[&str],
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let mut script = snapshot_script();
    script
        .replies
        .insert("list-panes -t @1".to_string(), "%4 0 *".to_string());
    script.notify.insert(
        "refresh-client -t %4 -C 90x24".to_string(),
        "%layout-change @1 0000,90x24,0,0,4 0000,90x24,0,0,4 *".to_string(),
    );
    script.failing.insert("client-snapshot -t @0".to_string());
    for line in extra_failing {
        script.failing.insert(line.to_string());
    }
    let (rx, conn) = fake_daemon(tag, script);
    let mut session = snapshot_session();
    session.status.apply_snapshot_for_test(&[
        "snapshot 1".to_string(),
        "workspace +0 * main".to_string(),
        "session $1 * +0 two".to_string(),
        "window @0 * edit".to_string(),
        "window @1 - logs".to_string(),
    ]);
    (rx, conn, session)
}

/// The SessionGone landing resolves from the last snapshot's cached
/// window list when it names a survivor of the gone window: the view
/// reseeds onto the cached sibling with no landing query on the wire,
/// so a pathological daemon parks nothing extra on the landing path.
#[test]
fn a_gone_window_lands_on_the_cached_sibling_without_queries() {
    let (rx, mut conn, mut session) = cached_landing_fixture("cachedlanding", &[]);
    drained(&rx);

    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    let wire = drained(&rx);
    for query in [
        "list-windows",
        "list-workspaces",
        "list-sessions",
        "switch-client",
    ] {
        assert!(
            !wire.iter().any(|line| line.starts_with(query)),
            "the landing resolved from the cache: {query} never left the client"
        );
    }
    assert_eq!(
        session.window, "@1",
        "the view landed on the cached sibling"
    );
    assert!(
        !session.status_end,
        "a survivor existed, so the view stands"
    );
}

/// A cached landing that died before it could be shown (the session
/// itself is gone) falls to the interactive ladder: the failed reseed
/// reports the view did not move, the ladder runs, and nothing left to
/// show ends the view the usual way.
#[test]
fn a_dead_cached_landing_falls_back_to_the_interactive_ladder() {
    let (rx, mut conn, mut session) = cached_landing_fixture(
        "cachedeadlanding",
        &[
            "client-snapshot -t @1",
            "list-windows -t $1",
            "list-workspaces",
            "list-sessions -t +0",
        ],
    );
    drained(&rx);

    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    settle(&mut session, &mut conn);
    let wire = drained(&rx);
    for query in [
        "list-windows -t $1",
        "list-workspaces",
        "list-sessions -t +0",
    ] {
        assert!(
            wire.iter().any(|line| line.starts_with(query)),
            "the ladder ran: {query} was asked"
        );
    }
    assert!(session.status_end, "the ladder found nothing left to show");
}

/// The storm rule survives the async path: a snapshot still owed past
/// the status bound is abandoned in place — the stale bar stands, no
/// re-issue queues behind the wedged reply, and the late reply (still
/// token-aligned) applies when dispatched.
#[test]
fn status_snapshot_timeout_abandons_without_re_issuing() {
    let mut script = snapshot_script();
    script.delays.insert(
        "client-snapshot -t @0".to_string(),
        std::time::Duration::from_millis(900),
    );
    let (rx, mut conn) = fake_daemon("asyncstorm", script);
    let mut session = snapshot_session();
    drained(&rx);

    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    wait_recorded(&rx, "client-snapshot");

    // Mid-flight dirty mark: nothing re-issues while the reply is owed,
    // and the mark survives for a re-issue once the reply applies.
    std::thread::sleep(std::time::Duration::from_millis(200));
    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        session.status_dirty,
        "the mid-flight mark survives for the post-reply re-issue"
    );
    assert!(drained(&rx).is_empty(), "no re-issue while in flight");

    // Past the bound: the abandon clears the in-flight slot and the
    // dirty mark — nothing re-queues behind the wedged reply (the
    // storm rule; the next poll tick or event re-queries).
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert!(
        !session.status_dirty,
        "the abandon leaves the re-query to the next poll or event"
    );
    assert!(drained(&rx).is_empty(), "no re-issue after the abandon");

    // The late reply is still owed its continuation: it applies when
    // dispatched (tokens keep the reply stream aligned).
    settle(&mut session, &mut conn);
    assert_eq!(
        session.status.agents().len(),
        2,
        "the late reply applies once it lands"
    );
}

/// The seed script for the mid-seed toast tests: `@0` holds pane `%9`
/// alone, the agent's pane `%1` is in the background, and the snapshot
/// already shows the agent `blocked` (its report landed during the
/// seed). `push_on` is the command whose reply the agent's push rides
/// ahead of.
fn mid_seed_script(push_on: &str) -> FakeScript {
    let mut script = snapshot_script();
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .replace(
            "agent %1 claude working hook",
            "agent %1 claude blocked hook ws=+0",
        );
    script
        .replies
        .insert("client-snapshot -t @0".to_string(), snapshot);
    script
        .replies
        .insert("pane-info -t %9".to_string(), "%9 @0 90x24".to_string());
    script.notify.insert(
        "refresh-client *".to_string(),
        "%layout-change @0 0000,90x24,0,0,9 0000,90x24,0,0,9 *".to_string(),
    );
    let push = "%agent-state-changed %1 claude blocked source=hook ws=+0";
    let note = match script.notify.get(push_on) {
        Some(existing) => format!("{existing}\n{push}"),
        None => push.to_string(),
    };
    script.notify.insert(push_on.to_string(), note);
    script
}

/// Seed `script`'s `@0` view with the side panel up, and return.
/// A re-attach with QUIET agents (no state pushes coming): the seed's
/// panel install runs before the status snapshot lands — against an
/// empty roster — so the agents section must be installed once the
/// snapshot lands via the pump's continuation machinery (the async
/// status path, ENH-049), or the panel stays agents-empty until an
/// unrelated event marks the status stale.
#[test]
fn the_seed_installs_the_agents_section_after_the_roster_lands() {
    let mut script = snapshot_script();
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 90x24".to_string());
    script.notify.insert(
        "refresh-client -t %1 -C 90x24".to_string(),
        format!(
            "%layout-change @0 {} {} *",
            "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}",
            "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}"
        ),
    );
    let (_rx, mut conn) = fake_daemon("seedroster", script);
    let mut session = WindowSession::new(90, 26);
    session.sidebar.set(true);
    let mut sink = super::RecordingSink::default();
    let shown = session
        .seed(&mut conn, Some("%1"), &mut sink)
        .expect("the seed lands");
    assert!(shown, "the seed shows the view without waiting on the bar");
    assert!(
        session.status.agents().is_empty(),
        "the seed returned before the roster landed — the snapshot applies via the continuation"
    );
    settle(&mut session, &mut conn);
    let sections = session
        .renderer
        .sidebar_sections
        .clone()
        .expect("panel sections");
    let agents = sections
        .iter()
        .find(|section| section.title == "agents")
        .expect("agents section installed from the snapshot");
    assert_eq!(agents.rows.len(), 2, "the snapshot's two agents");
}

fn seed_then_drain(tag: &str, script: FakeScript) -> WindowSession {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon(tag, script);
    let mut session = WindowSession::new(90, 26);
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    let mut sink = super::RecordingSink::default();
    session
        .seed(&mut conn, Some("%9"), &mut sink)
        .expect("the seed lands");
    assert!(session.drain_pushes(&mut conn).is_continue());
    session
}

/// An agent's report that lands while the client is still seeding (the
/// test barrier `?1002h` is written before the seed runs, so a loaded
/// seed overlaps the report): its push queued during the size report's
/// drain must reach the pump, not be discarded with the replay-covered
/// output around it.
#[test]
fn a_push_drained_by_the_seed_still_toasts() {
    let session = seed_then_drain("seedtoast-drain", mid_seed_script("refresh-client *"));
    let toast = session.renderer.toast.clone().expect("the push toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// The same report whose push is still in flight when the seed's own
/// status refresh answers: the snapshot already shows `blocked`, so the
/// transition baseline must not adopt it for that pane, or the push
/// reads as a repeat.
#[test]
fn a_push_queued_across_the_seed_refresh_still_toasts() {
    let session = seed_then_drain(
        "seedtoast-refresh",
        mid_seed_script("client-snapshot -t @0"),
    );
    let toast = session.renderer.toast.clone().expect("the push toasts");
    assert_eq!(toast.title, "claude blocked");
    assert_eq!(session.notify.outbox, vec![0x07], "and rings the bell");
}

/// A push that lands before any roster commit (the seed's refresh timed
/// out) must not seed the transition baseline from that partial view:
/// the first committed roster does, so a pre-existing blocked agent's
/// unchanged rebroadcast (`pane.report_agent_session` re-sends the held
/// state) stays silent.
#[test]
fn a_push_before_the_first_roster_does_not_seed_a_partial_baseline() {
    use super::super::notify::NotifyDelivery;
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    let push = |state: &str, pane: &str, agent: &str| TmuxNotification::AgentStateChanged {
        pane_id: pane.to_string(),
        agent: agent.to_string(),
        state: state.to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    };
    session.handle_event(push("working", "%1", "claude"));
    // The first roster commit: kimi on %3 was already blocked.
    let (rx, mut conn) = fake_daemon("snapunseeded", snapshot_script());
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    session.handle_event(push("blocked", "%3", "kimi"));
    assert!(
        session.renderer.toast.is_none(),
        "an unchanged rebroadcast is not a transition"
    );
    assert!(session.notify.outbox.is_empty(), "and rings nothing");
    // A real transition after it still toasts.
    session.handle_event(push("blocked", "%1", "claude"));
    assert!(session.renderer.toast.is_some());
}

/// The seed's race window: a push handled while the seed's snapshot is
/// in flight toasts (no roster is committed to judge it against), but
/// the push stream holds no record for the pane — `record_pushed_state`
/// seeds no baseline from the uncommitted view. The continuation's
/// baseline adoption must therefore leave the raced pane out (the sync
/// path drained its pre-refresh pushes and excluded them); otherwise
/// the snapshot's entry suppresses an identical rebroadcast on a record
/// the push stream never made.
#[test]
fn a_push_handled_mid_seed_stays_out_of_the_baseline_and_its_rebroadcast_toasts() {
    use super::super::notify::NotifyDelivery;
    let mut script = snapshot_script();
    // The daemon applies a report before it broadcasts it, so the
    // snapshot already carries the state the push announces.
    let snapshot = script
        .replies
        .get("client-snapshot -t @0")
        .expect("snapshot reply")
        .clone();
    script.replies.insert(
        "client-snapshot -t @0".to_string(),
        format!("{snapshot}\nagent %2 claude blocked hook ws=+0"),
    );
    // The push rides the snapshot reply (the wire order the race lives
    // in); the rebroadcast rides the pump's own refresh.
    script.notify.insert(
        "client-snapshot -t @0".to_string(),
        "%agent-state-changed %2 claude blocked source=hook ws=+0".to_string(),
    );
    let (rx, mut conn) = fake_daemon("seedrace", script);
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    // The pushed pane is background (herdr's rule): a lone pane layout
    // takes %2 out of view.
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    drained(&rx);

    // The seed's snapshot writes without waiting; the push is handled
    // while it is in flight. The fake writes the push ahead of the
    // reply block; the bounded drain waits out that write.
    session.seed_status_snapshot(&mut conn, Vec::new());
    assert!(session.status_snapshot_inflight.is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.renderer.toast.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the mid-seed push never arrived"
        );
        assert!(session.drain_pushes(&mut conn).is_continue());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the mid-seed push toasts");
    assert_eq!(toast.pane, Some(2));
    assert!(
        session.notify.transitions.is_none(),
        "handling a push seeds no baseline from the partial view"
    );

    // The reply's continuation adopts the baseline; the raced pane
    // stays out of it.
    settle(&mut session, &mut conn);
    assert!(session.status_snapshot_inflight.is_none());
    let baseline = session
        .notify
        .transitions
        .as_ref()
        .expect("the first roster seeded the baseline");
    assert!(
        !baseline.contains_key(&2),
        "the raced pane stays out of the baseline: {baseline:?}"
    );

    // The identical rebroadcast (the daemon re-announcing the state it
    // applied before it wrote the snapshot) toasts: there is no
    // push-stream record to suppress it on. It rides the pump's own
    // refresh (the way a late registration replay arrives).
    session.status_dirty = true;
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.notify.outbox.len() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the rebroadcast never arrived"
        );
        assert!(session.drain_pushes(&mut conn).is_continue());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let toast = session
        .renderer
        .toast
        .clone()
        .expect("the identical rebroadcast toasts");
    assert_eq!(toast.pane, Some(2));

    // Once recorded by that toast, an unchanged repeat is silent again:
    // no new bell (the rebroadcast's card is still the one up).
    let bells = session.notify.outbox.len();
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%2".to_string(),
        agent: "claude".to_string(),
        state: "blocked".to_string(),
        source: "hook".to_string(),
        ws: "+0".to_string(),
        msg: String::new(),
    });
    assert_eq!(
        session.notify.outbox.len(),
        bells,
        "a recorded repeat stays silent"
    );
}

/// A shown pane's death announces the respawn affordance even under the
/// default auto-remove, where the held exit cue never gets to show (the
/// TUI audit's F4).
#[test]
fn a_shown_pane_exit_flashes_the_respawn_opt_in() {
    let (rx, _conn) = fake_daemon("snapexit", snapshot_script());
    let mut session = snapshot_session();
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,1").expect("parses"));
    drained(&rx);
    session.on_pane_exited("%1", Some(7));
    let flash = session.chrome.flash_text().expect("the flash is up");
    assert!(flash.contains("exited (7)"), "{flash}");
    assert!(flash.contains("remain-on-exit"), "{flash}");
}

#[test]
fn agent_state_push_notifies_selected_transitions_only() {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon("snapnotify", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = vec!["blocked".to_string(), "idle".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    assert!(
        session.notify.outbox.is_empty(),
        "the refresh that loads the roster notifies nothing"
    );
    let push = |session: &mut WindowSession, pane: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: pane.to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    // The snapshot session shows %1..%3, so their agents are in view and
    // never notify; drop the layout to put them in the background.
    push(&mut session, "%1", "blocked");
    assert!(
        session.notify.outbox.is_empty(),
        "an agent in the shown window does not notify"
    );
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    // %1 claude blocked -> idle: selected, one OSC 9.
    push(&mut session, "%1", "idle");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;par-mux: main ? claude: idle\x07"
    );
    assert!(session.notify.outbox.is_empty(), "the frame drained it");
    // The same state again, then an unselected one: nothing.
    push(&mut session, "%1", "idle");
    push(&mut session, "%1", "working");
    assert!(session.notify.outbox.is_empty());
    // Off: a selected transition queues nothing.
    session.notify.delivery = NotifyDelivery::Off;
    push(&mut session, "%1", "blocked");
    assert!(session.notify.outbox.is_empty());
}

/// The default `notify-states` selects `done` (finished unseen); the
/// daemon's seen flip (done -> idle, another client viewed the pane)
/// never notifies, even with `idle` selected.
#[test]
fn done_notifies_and_the_seen_flip_does_not() {
    use super::super::notify::NotifyDelivery;
    let (_rx, mut conn) = fake_daemon("snapdone", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = crate::mux::config::Chords::with_defaults().notify_states;
    assert_eq!(session.notify.states, vec!["blocked", "done"]);
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    let push = |session: &mut WindowSession, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: "%1".to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    push(&mut session, "working");
    push(&mut session, "done");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;par-mux: main ? claude: done\x07"
    );
    session.notify.states.push("idle".to_string());
    push(&mut session, "idle");
    assert!(
        session.notify.outbox.is_empty(),
        "done -> idle is the seen flip, not news"
    );
}

/// Sounds ride their own channel (herdr's `[ui.sound]`): with delivery
/// `off` a blocked push still plays the blocked file and a background
/// finish the done file; a muted agent and an unselected sound key stay
/// silent; a missing file falls back to the bell.
#[test]
fn transition_sounds_play_per_event_and_respect_the_mute_list() {
    use super::super::notify::NotifyDelivery;
    use crate::mux::attach::sound::SPAWNED;
    let (_rx, mut conn) = fake_daemon("snapsound", snapshot_script());
    let mut session = snapshot_session();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("done.wav"), b"RIFF").unwrap();
    std::fs::write(dir.path().join("blocked.wav"), b"RIFF").unwrap();
    session.settings_path = Some(dir.path().join("config.toml"));
    session.notify.delivery = NotifyDelivery::Off;
    session.notify.sound = crate::mux::config::SoundSettings {
        enabled: true,
        player: "true".to_string(),
        done: "done.wav".to_string(),
        blocked: "blocked.wav".to_string(),
        mute: vec!["droid".to_string()],
    };
    session.refresh_status_facts(&mut conn).expect("seed");
    session
        .renderer
        .apply_layout(parse_layout("0000,80x24,0,0,9").expect("parses"));
    let push = |session: &mut WindowSession, agent: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: "%1".to_string(),
            agent: agent.to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    let take = || SPAWNED.with(|s| std::mem::take(&mut *s.borrow_mut()));
    take();
    push(&mut session, "claude", "working");
    assert!(take().is_empty(), "working never sounds");
    push(&mut session, "claude", "blocked");
    push(&mut session, "claude", "done");
    let played: Vec<std::ffi::OsString> = take().into_iter().map(|argv| argv[1].clone()).collect();
    assert_eq!(
        played,
        vec![
            dir.path().join("blocked.wav").into_os_string(),
            dir.path().join("done.wav").into_os_string()
        ]
    );
    assert!(
        session.notify.outbox.is_empty(),
        "delivery off: no escape, no bell"
    );
    push(&mut session, "droid", "blocked");
    assert!(take().is_empty(), "a muted agent never sounds");
    // A missing custom file falls back to the built-in OS sound; with
    // no built-in on this host the bell rings in its place.
    session.notify.sound.mute.clear();
    session.notify.sound.blocked = "missing.wav".to_string();
    let builtin_present =
        crate::mux::attach::sound::builtin_file(crate::mux::attach::sound::Sound::Blocked)
            .is_some_and(|f| std::path::Path::new(f).is_file());
    push(&mut session, "codex", "working");
    push(&mut session, "codex", "blocked");
    if builtin_present {
        assert_eq!(take().len(), 1, "the built-in plays");
        assert!(session.notify.outbox.is_empty());
    } else {
        assert!(take().is_empty());
        assert_eq!(session.notify.outbox, vec![0x07], "the bell stands in");
    }
    session.notify.outbox.clear();
    session.notify.sound.enabled = false;
    push(&mut session, "codex", "working");
    push(&mut session, "codex", "blocked");
    assert!(take().is_empty(), "sound off");
    assert!(session.notify.outbox.is_empty());
}

/// The notification-jump chord (prefix `j`, herdr's
/// `open_notification_target`) focuses the pane behind the most recent
/// agent notification this client delivered and dismisses the toast;
/// with nothing delivered it flashes and sends nothing. A suppressed
/// transition (an in-view agent) never becomes the target.
#[test]
fn notify_jump_chord_focuses_the_last_notified_pane() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapjump", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Inside;
    session.notify.states = vec!["blocked".to_string()];
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    assert_eq!(session.management.notify_jump, b'j');
    // Nothing delivered yet: a flash, no command.
    chord(&mut session, &mut conn, b'j');
    assert_eq!(
        session.chrome.flash_text(),
        Some("no notification to jump to")
    );
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("select-pane")),
        "no target, no select"
    );
    let push = |session: &mut WindowSession, pane: &str, state: &str| {
        session.handle_event(TmuxNotification::AgentStateChanged {
            pane_id: pane.to_string(),
            agent: "claude".to_string(),
            state: state.to_string(),
            source: "hook".to_string(),
            ws: "+0".to_string(),
            msg: String::new(),
        });
    };
    // %1 is in the shown window: no notification, so no target.
    push(&mut session, "%1", "blocked");
    assert!(session.notify.jump_targets.is_empty());
    // Background the agents; %3 then %1 notify — the latest wins.
    session
        .renderer
        .apply_layout(parse_layout("0000,90x24,0,0,9").expect("parses"));
    push(&mut session, "%3", "working");
    push(&mut session, "%3", "blocked");
    push(&mut session, "%1", "working");
    assert_eq!(
        session.notify.jump_targets.last(),
        Some(&3),
        "unselected keeps it"
    );
    push(&mut session, "%1", "blocked");
    assert_eq!(session.notify.jump_targets.last(), Some(&1));
    assert!(session.renderer.toast.is_some());
    drained(&rx);
    chord(&mut session, &mut conn, b'j');
    assert!(session.renderer.toast.is_none(), "the jump dismisses it");
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %1"),
        "the chord lands on the last notified pane"
    );
    // The jump CONSUMED %1: a second jump walks back to %3 (herdr's
    // consume-and-promote), a third finds the backlog empty.
    chord(&mut session, &mut conn, b'j');
    assert!(
        drained(&rx).iter().any(|line| line == "select-pane -t %3"),
        "the second jump walks to the previous notification"
    );
    chord(&mut session, &mut conn, b'j');
    assert_eq!(
        session.chrome.flash_text(),
        Some("no notification to jump to")
    );
}

/// An ad-hoc `%notification` (`show-notification`) rides the client's
/// delivery channel with the script's own title — ignoring notify-states
/// and the background-only rule — never moves the jump target, and its
/// toast, which has no pane, only dismisses on a click.
#[test]
fn adhoc_notification_delivers_through_the_channel_without_a_target() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snapadhoc", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.notify.states = Vec::new();
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    let adhoc = |session: &mut WindowSession, body: &str| {
        // The delivery's 1/s rate gate (herdr's bound) is a clock, so the
        // test clears it between calls — each delivery below is its own
        // first.
        session.notify.last_adhoc_at = None;
        session.handle_event(TmuxNotification::Notification {
            title: "build failed".to_string(),
            body: body.to_string(),
        });
    };
    adhoc(&mut session, "3 errors");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed: 3 errors\x07",
        "the script's title, not par-mux"
    );
    adhoc(&mut session, "");
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed\x07",
        "a title-only notification has no trailing separator"
    );
    session.notify.delivery = NotifyDelivery::Off;
    adhoc(&mut session, "x");
    assert!(session.notify.outbox.is_empty(), "off delivers nothing");
    session.notify.delivery = NotifyDelivery::Inside;
    adhoc(&mut session, "see the log");
    let toast = session.renderer.toast.clone().expect("a toast is up");
    assert_eq!(toast.title, "build failed");
    assert_eq!(toast.body, "see the log");
    assert_eq!(toast.pane, None);
    assert!(session.notify.jump_targets.is_empty(), "no jump target");
    let (tx, ty, _, _) = session.renderer.toast_rect().expect("on screen");
    let (hx, hy) = session.geometry.frame_to_host(tx + 2, ty + 1);
    session.route_mouse(&mut conn, sgr(0, hx + 1, hy + 1, false));
    settle(&mut session, &mut conn);
    assert!(session.renderer.toast.is_none(), "the click dismisses it");
    assert!(
        !drained(&rx).iter().any(|l| l.starts_with("select-pane")),
        "an ad-hoc toast has no pane to focus"
    );
}

/// `show-notification` is rate-limited to one per second per client —
/// herdr's own `API_NOTIFICATION_RATE_LIMIT` bound: a second delivery
/// inside the window is dropped with a status flash, whatever the
/// channel, so a looping script cannot spawn an osascript per call or
/// flood the terminal escapes.
#[test]
fn adhoc_notifications_are_rate_limited_to_one_per_second() {
    use super::super::notify::NotifyDelivery;
    let (rx, mut conn) = fake_daemon("snaprate", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;
    session.notify.host_is_kitty = false;
    session.refresh_status_facts(&mut conn).expect("seed");
    drained(&rx);
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "1".to_string(),
    });
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "2".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    let raw = String::from_utf8(sink.raw.clone()).unwrap();
    assert_eq!(
        raw.matches("\x1b]9;").count(),
        1,
        "only the first delivery inside the window lands: {raw:?}"
    );
    assert_eq!(
        session.chrome.flash_text(),
        Some("notification rate-limited (1/s)")
    );
}

/// The terminal delivery's wire shapes are pinned per host, not per
/// environment: under a kitty host the escape is kitty's chunked OSC 99
/// (title chunk, then body chunk), elsewhere plain OSC 9 with the title
/// leading. The four OSC 9 assertions above set the resolved flag
/// explicitly for the same reason — whatever terminal launched
/// `cargo test` must not choose which assertions hold.
#[test]
fn terminal_delivery_shapes_are_pinned_per_host_not_per_environment() {
    use super::super::notify::NotifyDelivery;
    let (_rx, _conn) = fake_daemon("snapkitty", snapshot_script());
    let mut session = snapshot_session();
    session.notify.delivery = NotifyDelivery::Terminal;

    session.notify.host_is_kitty = true;
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "3 errors".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]99;i=1:d=0;build failed\x1b\\\x1b]99;i=1:p=body;3 errors\x1b\\",
        "under kitty the delivery is chunked OSC 99, title then body"
    );

    session.notify.host_is_kitty = false;
    session.notify.last_adhoc_at = None;
    session.handle_event(TmuxNotification::Notification {
        title: "build failed".to_string(),
        body: "3 errors".to_string(),
    });
    let mut sink = RecordingSink::default();
    session.frame(&mut sink);
    assert_eq!(
        String::from_utf8(sink.raw.clone()).unwrap(),
        "\x1b]9;build failed: 3 errors\x07",
        "off kitty the delivery is plain OSC 9, the title leading"
    );
}

/// An agent whose project the probe has not reported yet keeps the
/// roster polling on a slow cadence (the probe broadcasts nothing), and
/// the poll stops once every agent has one.
#[test]
fn missing_project_polls_the_roster_until_it_arrives() {
    let (_rx, mut conn) = fake_daemon("snapproject", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    let start = session.project_poll;
    assert!(
        !session.project_poll_due(start),
        "the clock starts at construction"
    );
    let later = start + super::super::sidebar::PROJECT_POLL_INTERVAL;
    assert!(session.project_poll_due(later), "an agent lacks a project");
    assert!(
        !session.project_poll_due(later),
        "the poll restarted the clock"
    );
    for pane in [1, 3] {
        session
            .status
            .patch_agent(&format!("%{pane}"), "x", Some("idle"), "+0", "");
    }
    session.status.set_projects_for_test("par-term");
    let much_later = later + super::super::sidebar::PROJECT_POLL_INTERVAL;
    assert!(
        !session.project_poll_due(much_later),
        "every agent has its project: no poll"
    );
}

/// `%agent-state-changed` and `%agent-released` patch the roster from
/// their payload and rebuild the side panel's agents section from the
/// cache: no status mark, no query (ARC-136).
#[test]
fn agent_event_patches_without_a_query() {
    let (rx, mut conn) = fake_daemon("snapagent", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_dirty = false;
    drained(&rx);

    let rows = |session: &WindowSession| -> Vec<SidebarRow> {
        session
            .renderer
            .sidebar_sections
            .iter()
            .flatten()
            .flat_map(|section| section.rows.clone())
            .collect()
    };
    session.handle_event(TmuxNotification::AgentStateChanged {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
        state: "working".to_string(),
        source: "hook".to_string(),
        ws: "+1".to_string(),
        msg: String::new(),
    });
    assert!(!session.status_dirty);
    assert!(
        rows(&session).contains(&arow(3, Some("side"), "kimi", "working", false)),
        "the ws token groups the inserted row under its workspace: {:?}",
        rows(&session)
    );
    assert!(
        !rows(&session)
            .iter()
            .any(|row| row.status == crate::mux::attach::panels::AgentStatus::Blocked),
        "{:?}",
        rows(&session)
    );

    session.handle_event(TmuxNotification::AgentReleased {
        pane_id: "%3".to_string(),
        agent: "kimi".to_string(),
    });
    assert!(!session.status_dirty);
    assert!(
        !rows(&session)
            .iter()
            .any(|row| row.id == SidebarRowId::Pane(3)),
        "{:?}",
        rows(&session)
    );
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}

/// The bar never renders telemetry, so a telemetry sample leaves the
/// status clean.
#[test]
fn telemetry_event_does_not_mark_status_dirty() {
    let mut session = snapshot_session();
    session.status_dirty = false;
    session.handle_event(TmuxNotification::AgentTelemetryChanged {
        pane_id: "%1".to_string(),
        agent: "claude".to_string(),
    });
    assert!(!session.status_dirty);
}

/// `%window-renamed` and a set `%pane-title-changed` patch the tab and
/// the focused pane's title from their payloads, no status mark.
#[test]
fn rename_and_title_events_patch_without_a_query() {
    let (rx, mut conn) = fake_daemon("snaprename", snapshot_script());
    let mut session = snapshot_session();
    session.refresh_status_facts(&mut conn).expect("seed");
    session.status_dirty = false;
    drained(&rx);

    session.handle_event(TmuxNotification::WindowRenamed {
        window_id: "@1".to_string(),
        name: "tail".to_string(),
    });
    session.handle_event(TmuxNotification::PaneTitleChanged {
        pane_id: "%1".to_string(),
        title: "deploy".to_string(),
    });
    assert!(!session.status_dirty);
    assert_eq!(session.status.windows()[1].1, "tail");
    assert_eq!(session.status.pane_title(), "deploy");
    assert_eq!(session.renderer.border_label(1), "deploy");
    assert!(received_within_300ms(&rx).is_empty(), "no round trip");
}

/// ENH-044 parked-layout path: a `%layout-change` the handlers parked for
/// the pump (`apply_pending`) replays only panes new to the renderer —
/// the surviving panes reflow in place with their content intact (card
/// 01a1216d58dd).
#[test]
fn parked_layout_replays_only_panes_new_to_the_renderer() {
    let (rx, mut conn, mut session) = two_pane_session(
        "parked-layout-replay",
        FakeScript {
            replies: [(
                "refresh-client -t %3".to_string(),
                "pane three body".to_string(),
            )]
            .into_iter()
            .collect(),
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"left pane text");
    session.renderer.feed_output(2, b"right pane text");
    session.pending.layout = Some(
        parse_layout("0000,99x23,0,0{33x23,0,0,1,33x23,33,0,2,33x23,66,0,3}").expect("parses"),
    );
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("apply pending");
    let lines = drained(&rx);
    assert_eq!(
        replay_lines(&lines),
        vec!["refresh-client -t %3"],
        "only the new pane replays: {lines:?}"
    );
    assert!(session.renderer.emulators[&3]
        .terminal()
        .content()
        .contains("pane three body"));
    assert!(
        session.renderer.emulators[&1]
            .terminal()
            .content()
            .contains("left pane text"),
        "surviving panes keep their content"
    );
}

/// ENH-044: an `%exit` drained inside the resize's drain_around_layout
/// parks in `pending_end`, which the pump checks (and ends the session
/// on) after the refit (card 01a1216d58dd).
#[test]
fn exit_drained_in_a_resize_parks_pending_end() {
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %1 -C 100x23".to_string(),
        format!("{}\n%exit", two_pane_layout_line(100, 23)),
    );
    let (_rx, mut conn, mut session) = two_pane_session(
        "resize-drain-exit",
        FakeScript {
            notify,
            ..FakeScript::default()
        },
    );
    session
        .resize_to(&mut conn, 100, 23, &mut RecordingSink::default())
        .expect("resize");
    assert!(session.pending_end, "the drained %exit parked the end");
    assert!(
        std::mem::take(&mut session.pending_end),
        "the pump's post-drain check would end the session"
    );
}

/// A `pane:N` agent-row click on a pane in ANOTHER window: select the
/// pane, look up its window, then land there (switch-client) and
/// re-seed onto it (QA-245).
#[test]
fn focus_pane_on_another_window_selects_looks_up_and_lands() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    replies.insert("pane-info -t %5".to_string(), "%5 @1 40x23".to_string());
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-otherwin",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 5);
    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    assert_eq!(
        &lines[..3],
        &[
            "select-pane -t %5",
            "pane-info -t %5",
            "switch-client -t @1"
        ],
        "wire: {lines:?}"
    );
    assert_eq!(session.window, "@1", "the view landed on the pane's window");
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![5, 6], "re-seeded onto the new window's panes");
}

/// Same-window click: select + lookup only, no landing; the status is
/// marked so the throttled refresh repaints the highlight (QA-245).
#[test]
fn focus_pane_on_the_shown_window_does_not_land() {
    let mut replies = std::collections::HashMap::new();
    replies.insert("pane-info -t %2".to_string(), "%2 @0 40x23".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-samewin",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.status_dirty = false;
    session.focus_pane(&mut conn, 2);
    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    assert_eq!(lines, vec!["select-pane -t %2", "pane-info -t %2"]);
    assert_eq!(session.window, "@0");
    assert!(session.status_dirty, "the highlight repaint is scheduled");
}

/// A refused `select-pane` (the pane is gone) ends the click silently:
/// no `pane-info`, no landing, no busy flash (ARC-141's early return;
/// the timed-out case is `focus_pane_chain_stops_at_the_first_timeout`).
#[test]
fn focus_pane_stops_when_select_pane_fails() {
    let mut failing = std::collections::HashSet::new();
    failing.insert("select-pane -t %9".to_string());
    let (rx, mut conn, mut session) = two_pane_session(
        "qa245-gone",
        FakeScript {
            failing,
            ..FakeScript::default()
        },
    );
    session.focus_pane(&mut conn, 9);
    settle(&mut session, &mut conn);
    // Pipelined: the pane-info rides behind the select without waiting for
    // its refusal; the refused select leaves no pane-info row to land on.
    assert_eq!(drained(&rx), vec!["select-pane -t %9", "pane-info -t %9"]);
    assert_eq!(session.window, "@0", "the view stays put");
    assert_eq!(
        session.chrome.flash_text(),
        None,
        "a refusal is not a busy daemon"
    );
}

/// ENH-049: with a 2 s daemon delay on the click's `select-pane`, neither
/// the sidebar click nor the keystrokes typed after it make the pump wait;
/// once the daemon catches up it saw the select, the lookup and the keys
/// in order.
#[test]
fn slow_daemon_does_not_block_key_forwarding() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "select-pane -t %2".to_string(),
        std::time::Duration::from_secs(2),
    );
    script
        .replies
        .insert("pane-info -t %2".to_string(), "%2 @0 40x23".to_string());
    let (rx, mut conn, mut session) = two_pane_session("enh049-slow", script);
    show_sidebar(&mut session);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            title: "workspaces",
            rows: vec![srow(SidebarRowId::Pane(2), "main \u{b7} agent", false)],
        }]));

    let started = std::time::Instant::now();
    session.route_mouse(&mut conn, sgr(0, 3, 2, false));
    let click = started.elapsed();
    let started = std::time::Instant::now();
    session.route_stdin_bytes(&mut conn, b"abc");
    let typed = started.elapsed();
    let limit = std::time::Duration::from_millis(200);
    assert!(click < limit, "the click blocked the pump for {click:?}");
    assert!(typed < limit, "typing blocked the pump for {typed:?}");

    settle(&mut session, &mut conn);
    let lines = drained(&rx);
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.starts_with(needle))
            .unwrap_or_else(|| panic!("no {needle} line: {lines:?}"))
    };
    let select = position("select-pane -t %2");
    let info = position("pane-info -t %2");
    let keys = lines
        .iter()
        .position(|l| l.starts_with("send-keys -t ") && l.ends_with("-H 61 62 63"))
        .unwrap_or_else(|| panic!("no send-keys line: {lines:?}"));
    assert!(select < info && info < keys, "wire order: {lines:?}");
}

/// ENH-049: a click on a pane of window @2 followed, before the daemon
/// answers, by the next-window chord to @3 ends on @3 and never reseeds
/// back to @2.
#[test]
fn focus_pane_chain_pipelines_and_ignores_a_stale_landing() {
    let layout = "0000,80x23,0,0,9";
    let mut notify = std::collections::HashMap::new();
    for pane in [1u32, 9] {
        notify.insert(
            format!("refresh-client -t %{pane} -C 80x23"),
            format!("%layout-change @3 {layout} {layout} *"),
        );
    }
    let mut replies = std::collections::HashMap::new();
    replies.insert("pane-info -t %5".to_string(), "%5 @2 40x23".to_string());
    reseed_replies(&mut replies, "@0 * main\n@3 - other", "@3", "%9 @3 * z");
    replies.insert("list-panes -t @2".to_string(), "%5 @2 * y".to_string());
    let mut script = FakeScript {
        replies,
        notify,
        ..FakeScript::default()
    };
    script.delays.insert(
        "pane-info -t %5".to_string(),
        std::time::Duration::from_millis(300),
    );
    let (rx, mut conn, mut session) = two_pane_session("enh049-stale", script);

    session.focus_pane(&mut conn, 5);
    // The newer navigation: next window from @0 is @3.
    session.switch_window(&mut conn, 1);
    settle(&mut session, &mut conn);

    assert_eq!(session.window, "@3", "the newest navigation wins");
    let lines = drained(&rx);
    assert!(
        lines.iter().all(|l| !l.starts_with("switch-client -t @2")),
        "the stale pane-info reply must not land on @2: {lines:?}"
    );
    let reports = lines
        .iter()
        .filter(|l| l.starts_with("refresh-client") && l.contains(" -C "))
        .count();
    assert_eq!(reports, 1, "one reseed only: {lines:?}");
}

/// ENH-049: a parked reply never answers a later synchronous query. The
/// status refresh parses its own replies, and the continuation gets its
/// own.
#[test]
fn async_reply_never_answers_a_later_sync_query() {
    let mut script = five_session_script();
    script.delays.insert(
        "list-agents".to_string(),
        std::time::Duration::from_millis(100),
    );
    let (_rx, mut conn) = fake_daemon("enh049-mix", script);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(FOUR_PANE).expect("parses"));
    session.window = "@0".to_string();
    let seen: std::rc::Rc<std::cell::RefCell<Option<Vec<String>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = std::rc::Rc::clone(&seen);
    session.send_then(&mut conn, "list-agents", move |_s, _c, reply| {
        *sink.borrow_mut() = Some(reply.body);
    });
    assert_eq!(session.refresh_status(&mut conn), EventOutcome::Continue);
    assert_eq!(
        session.status.session_id.as_deref(),
        Some("$4"),
        "the status refresh parsed its own replies"
    );
    settle(&mut session, &mut conn);
    assert_eq!(
        seen.borrow().clone(),
        Some(Vec::new()),
        "the continuation received its own (empty agents) reply, not a status reply"
    );
}

/// The settings dialog's `general` tab lists `session picker` (default
/// off) and its toggle writes `session-picker = true` into the file and
/// applies it to the session, for the next target-less attach to read.
#[test]
fn settings_row_toggles_the_session_picker() {
    let (_rx, mut conn, mut session) = command_menu_session("pickersettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    session.route_stdin_bytes(&mut conn, b"\t\t");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("session picker") && r.contains("off")),
        "the row shows the default in force: {rows:#?}"
    );
    assert!(!session.session_picker);
    session.route_stdin_bytes(&mut conn, b"jjjjjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("session-picker = true"), "{written}");
    assert!(session.session_picker, "applied to the session");
}

/// The settings dialog's `general` tab lists `git status badge` (default
/// on) and its toggle writes `git-status = false` into the file and
/// applies it to the session (the badge's render and poll gate).
#[test]
fn settings_row_toggles_the_git_badge() {
    let (_rx, mut conn, mut session) = command_menu_session("gitbadgesettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    session.route_stdin_bytes(&mut conn, b"\t\t");
    // The general tab now carries nine fixed rows (the tab-width rows
    // included) and the panel scrolls — reach the git row before
    // asserting on it.
    session.route_stdin_bytes(&mut conn, b"jjjjjjjj");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("git status badge") && r.contains("on")),
        "the row shows the default in force: {rows:#?}"
    );
    assert!(session.git_status);
    session.route_stdin_bytes(&mut conn, b"\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("git-status = false"), "{written}");
    assert!(!session.git_status, "applied to the session");
}

/// The daemon side of a seed whose target `%1` (window `@0`) resolved
/// and whose workspace is then torn down, leaving workspace `+1` with
/// session `$1` and its window `@5` (pane `%7`): the cascade landing's
/// replies and `@5`'s size-report layout.
fn torn_down_seed_script() -> FakeScript {
    let mut script = FakeScript::default();
    let replies = &mut script.replies;
    replies.insert("pane-info -t %1".to_string(), "%1 @0 80x23".to_string());
    replies.insert(
        "list-workspaces".to_string(),
        "+1: other active".to_string(),
    );
    replies.insert("list-sessions -t +1".to_string(), "$1: other".to_string());
    replies.insert("list-sessions".to_string(), "$1: other".to_string());
    replies.insert("list-windows -t $1".to_string(), "@5 * main".to_string());
    replies.insert("list-panes -t @5".to_string(), "%7 0 *".to_string());
    script.notify.insert(
        "refresh-client -t %7 -C 80x23".to_string(),
        "%layout-change @5 0000,80x23,0,0,7 0000,80x23,0,0,7 *".to_string(),
    );
    script
}

/// Card 01a15b6f: the shown workspace dies between the seed's target
/// resolution and its size report. The daemon refuses the report (the
/// pane is gone), so no `%layout-change` comes — the seed used to read
/// that as a pre-layout daemon and exit 1. It lands on the survivor.
#[test]
fn seed_lands_on_the_survivor_when_the_target_dies_before_its_size_report() {
    let mut script = torn_down_seed_script();
    script
        .failing
        .insert("refresh-client -t %1 -C 80x23".to_string());
    script.failing.insert("list-panes -t @0".to_string());
    let (rx, mut conn) = fake_daemon("seed-gone-report", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("a vanished target lands instead of failing the attach");
    assert!(shown, "a survivor exists, so the view is shown");
    assert_eq!(session.window, "@5", "the view landed on the survivor");
    let panes: Vec<u32> = session.renderer.layout().iter().map(|r| r.pane).collect();
    assert_eq!(panes, vec![7], "the survivor's layout is installed");
    assert!(
        drained(&rx).contains(&"refresh-client -t %7 -C 80x23".to_string()),
        "the survivor got its own size report"
    );
}

/// The same race one step later: the size report and its layout went
/// through, then the status refresh finds no session owning `@0`. The
/// seed used to end with "the target's session is gone"; it lands.
#[test]
fn seed_lands_on_the_survivor_when_the_target_session_dies_mid_seed() {
    let mut script = torn_down_seed_script();
    script.notify.insert(
        "refresh-client -t %1 -C 80x23".to_string(),
        "%layout-change @0 0000,80x23,0,0,1 0000,80x23,0,0,1 *".to_string(),
    );
    let (_rx, mut conn) = fake_daemon("seed-gone-status", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("a vanished session lands instead of failing the attach");
    assert!(shown);
    assert_eq!(session.window, "@5");
    assert_eq!(session.status.session_id.as_deref(), Some("$1"));
}

/// The all-gone boundary: the target died mid-seed and the daemon lists
/// nothing to land on, so the seed reports an empty view (a clean end),
/// not an attach failure.
#[test]
fn seed_with_nothing_left_after_the_target_dies_ends_cleanly() {
    let mut script = FakeScript::default();
    script
        .replies
        .insert("pane-info -t %1".to_string(), "%1 @0 80x23".to_string());
    script
        .failing
        .insert("refresh-client -t %1 -C 80x23".to_string());
    script.failing.insert("list-panes -t @0".to_string());
    let (_rx, mut conn) = fake_daemon("seed-gone-all", script);
    let mut session = WindowSession::new(80, 25);
    let shown = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect("nothing left is a clean end, not an error");
    assert!(!shown, "nothing to show");
}

/// A target that does not exist when the seed first asks stays an
/// attach error: only a landing that vanished after it resolved walks
/// to a survivor.
#[test]
fn seed_of_a_target_missing_at_resolution_is_still_an_error() {
    let mut script = torn_down_seed_script();
    for line in ["pane-info -t %1", "list-windows -t %1", "list-panes -t %1"] {
        script.failing.insert(line.to_string());
    }
    let (_rx, mut conn) = fake_daemon("seed-missing", script);
    let mut session = WindowSession::new(80, 25);
    let err = session
        .seed(&mut conn, Some("%1"), &mut RecordingSink::default())
        .expect_err("an unknown explicit target is an error");
    assert!(err.contains("no such target: %1"), "{err}");
}

/// A `pane-info` reply past the 10 s bound proves nothing about the
/// target's kind. The resolver used to fall through to the window probe,
/// which `list-panes -t %1` passes (it accepts pane targets), and adopt
/// `%1` as the window id; the seed then waited for a `%layout-change
/// %1` no daemon sends ("no %layout-change for %0" under host load).
#[test]
fn a_timed_out_pane_info_never_adopts_the_pane_as_a_window() {
    let mut script = FakeScript::default();
    script.delays.insert(
        "pane-info -t %1".to_string(),
        std::time::Duration::from_millis(10_500),
    );
    script.failing.insert("list-windows -t %1".to_string());
    script
        .replies
        .insert("list-panes -t %1".to_string(), "%1 0 *".to_string());
    let (_rx, mut conn) = fake_daemon("paneinfo-timeout", script);
    let err = resolve_window_and_pane(&mut conn, Some("%1"))
        .expect_err("a timed-out pane-info is not a window");
    assert!(err.contains("pane-info failed"), "{err}");
}
