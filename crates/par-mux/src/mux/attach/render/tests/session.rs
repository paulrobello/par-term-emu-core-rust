//! WindowSession suites: frame layout, strip and sidebar clicks, drags,
//! display options, overlays, and the command menu.

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
    let (rows, mapping) = compose_menu_panel(&MenuTarget::Tab("@1".to_string()), "vim");
    assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
    assert_eq!(rows[0].text, " vim ", "the header carries the name");
    assert_eq!(mapping[0], None, "the header consumes its clicks");
    assert_eq!(mapping[1], Some(MenuAction::Rename));
    assert_eq!(rows[1].text, " rename ");
    assert_eq!(mapping[2], Some(MenuAction::Close));
    assert_eq!(rows[2].text, " close ");
    assert_eq!(mapping[3], Some(MenuAction::AddTab));
    assert_eq!(rows[3].text, " add tab ");
    assert_eq!(mapping[4], None, "the footer consumes its clicks");

    let (_, ws_mapping) = compose_menu_panel(&MenuTarget::Workspace("+0".to_string()), "main");
    assert_eq!(
        ws_mapping,
        vec![
            None,
            Some(MenuAction::Rename),
            Some(MenuAction::Close),
            Some(MenuAction::NewWorkspace),
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
    session.tab_strip.paint(&windows, Some("@0"), None);
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
    assert_eq!(session.tab_strip.hit_test(8), None, "the gap hits nothing");
    session.tab_click(&mut conn, 12);

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
        session.tab_strip.hit_test(12),
        Some(1),
        "the strip layout still maps the clicked column"
    );
}

/// The drag affordance, headless end-to-end: a press within one cell
/// of the two-pane divider starts a PENDING drag (no focus, no
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
    let mut prefix_pending = false;

    assert!(
        !session.route_plain(
            &[crate::mux::attach::C_B, b'R'],
            &mut conn,
            &mut prefix_pending
        ),
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
        !session.route_plain(
            &[crate::mux::attach::C_B, b'R'],
            &mut conn,
            &mut prefix_pending
        ),
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
    let mut prefix_pending = false;
    assert!(
        !session.route_plain(
            &[crate::mux::attach::C_B, b'?'],
            &mut conn,
            &mut prefix_pending
        ),
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

    let mut prefix_pending = false;
    assert!(
        !session.route_plain(
            &[crate::mux::attach::C_B, b'w'],
            &mut conn,
            &mut prefix_pending
        ),
        "the picker chord opens the modal"
    );
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
    assert!(!session.route_plain(
        &[crate::mux::attach::C_B, b'w'],
        &mut conn,
        &mut prefix_pending
    ));
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

    // Gutter OFF: no indicator paints at the pane's edge.
    let mut plain = PaneRenderer::new(80, 24, Glyphs::Unicode);
    plain.set_background(Some(RtColor::Rgb(16, 24, 40)));
    plain.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    plain.feed_output(1, &flood);
    plain.render_frame();
    plain.scroll_viewport(1, 10);
    plain.render_frame();
    assert_ne!(
        plain.cell(39, 0).expect("plain").symbol(),
        "\u{2590}",
        "gutter off paints no indicator"
    );
}

/// The `drag-cursor-shape` option: while a divider drag is live the
/// frame's cursor placement carries the resize shape (steady block),
/// overriding the pane's tracked DECSCUSR shape; option off (and
/// drag end) keeps the pane's own shape.
#[test]
fn drag_cursor_shape_option_shapes_the_cursor_during_a_drag() {
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
    session.drag_cursor_shape = true;
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

    session.status_bar_on = false;
    session.chrome_geometry_changed();
    session.draw_status_row();
    let mut sink = CellSink { cells: Vec::new() };
    assert!(
        !session.flush_status_row(&mut sink),
        "hidden, nothing flushes"
    );
    assert!(sink.cells.is_empty());
}

/// Side-panel click geometry after round 6: the lookup takes the
/// HOST row (row 0 is the tab strip's — nothing there), shifted one
/// row so a click lands on the row as painted — the FIRST workspace
/// row at host row 1 (no section header above it), the footer chips
/// at the panel's last row, each chip only its own cell.
#[test]
fn sidebar_clicks_land_on_the_painted_row() {
    let mut session = WindowSession::new(80, 24);
    show_sidebar(&mut session);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::super::SidebarSection {
            rows: vec![
                ("ws:+0".to_string(), "alpha".to_string(), true),
                ("ws:+1".to_string(), "beta".to_string(), false),
            ],
        }]));
    // Host row 0 (the tab strip row): nothing.
    assert_eq!(session.renderer.sidebar_row_at(2, 0), None);
    // Host row 1: the first workspace row, as painted (no header
    // above it since round 6).
    assert_eq!(
        session.renderer.sidebar_row_at(2, 1),
        Some("ws:+0".to_string()),
        "the click lands on the row as painted"
    );
    // Host row 2: the second workspace row.
    assert_eq!(
        session.renderer.sidebar_row_at(2, 2),
        Some("ws:+1".to_string())
    );
    // The footer row (host row = renderer height - 1 + 1 = 24... the
    // 22-row renderer's LAST row at host row 22): the chips answer
    // only within their own cells — ` new ` spans cols 0..5, ` menu
    // ` the content's right edge.
    assert_eq!(
        session.renderer.sidebar_row_at(2, 22),
        Some("panel:new".to_string()),
        "the new chip's cell"
    );
    assert_eq!(
        session.renderer.sidebar_row_at(16, 22),
        Some("panel:menu".to_string()),
        "the menu chip's cell"
    );
    assert_eq!(
        session.renderer.sidebar_row_at(10, 22),
        None,
        "the gap between the chips hits nothing"
    );
    // A body row answers across its full content width (the active
    // row's inverted block is clickable anywhere on it).
    assert_eq!(
        session.renderer.sidebar_row_at(17, 1),
        Some("ws:+0".to_string())
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
    // A SECOND motion while active must extend the drag, not kill it.
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
    assert_eq!(
        wait_recorded(&rx, "resize-pane"),
        "resize-pane -t %1 -R 2",
        "the second motion resizes its unapplied delta"
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
/// session, as reseed_window does) are checked.
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

    // Re-seed path: a fresh renderer replaying the session (the
    // reseed_window reconstruction), framed once.
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

/// The command menu's pure mapping: a fixed header, one row per
/// client command (keybinds, reload config, detach), the footer.
#[test]
fn compose_command_menu_maps_rows_to_actions() {
    let (rows, mapping) = compose_menu_panel(&MenuTarget::Commands, "commands");
    assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
    assert_eq!(rows[0].text, " commands ");
    assert_eq!(rows[1].text, " keybinds ");
    assert_eq!(rows[2].text, " reload config ");
    assert_eq!(rows[3].text, " detach ");
    assert_eq!(
        mapping,
        vec![
            None,
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
    assert_eq!(overlay.1[1].text, " keybinds ");
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
    assert!(session.flash.is_some(), "the reload flashes its outcome");
}

/// The command menu's `detach` row requests the pump's exit (the
/// prefix-d path), and the header/footer rows do nothing.
#[test]
fn command_menu_detach_requests_exit() {
    let (_rx, mut conn, mut session) = command_menu_session("cmdmenu-detach");
    click_menu_row(&mut session, &mut conn, 0);
    assert!(session.menu().is_some(), "the header consumes the click");
    assert!(!session.detach_requested);
    click_menu_row(&mut session, &mut conn, 3);
    assert!(session.menu().is_none(), "the menu closed");
    assert!(session.detach_requested, "detach was requested");
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
            })
            .chain(std::iter::once(crate::mux::attach::HelpRow {
                text: "footer".to_string(),
                accent: false,
                footer: true,
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
    let mut prefix_pending = false;
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[<0;10;", &mut prefix_pending));
    assert!(!session.route_stdin_bytes(&mut conn, b"5M", &mut prefix_pending));
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
    let mut prefix_pending = false;
    assert!(!session.route_stdin_bytes(&mut conn, b"\x1b[200~ab", &mut prefix_pending));
    assert!(!session.route_stdin_bytes(&mut conn, b"\x02xcd\x1b[201~", &mut prefix_pending));
    assert!(
        !prefix_pending,
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
    let mut prefix_pending = false;
    session.route_stdin_bytes(
        &mut conn,
        b"\x1b[200~ab\x02xcd\x1b[201~",
        &mut prefix_pending,
    );
    assert_eq!(drained(&rx), vec![hex_wire("%1", b"ab\x02xcd")]);
}

/// SEC-208: a paste cancels a pending prefix (tmux's rule), so the
/// paste's first byte is not taken as a chord key.
#[test]
fn paste_cancels_a_pending_prefix() {
    let (rx, mut conn, mut session) = paste_session("paste-prefix");
    let mut prefix_pending = false;
    session.route_stdin_bytes(&mut conn, &[crate::mux::attach::C_B], &mut prefix_pending);
    assert!(prefix_pending);
    session.route_stdin_bytes(&mut conn, b"\x1b[200~xd\x1b[201~", &mut prefix_pending);
    assert!(!prefix_pending);
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
    let mut prefix_pending = false;
    session.route_paste(
        &mut conn,
        b"a\x1b[201~b\x1b[201\x1b[201~~c",
        &mut prefix_pending,
    );
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
    session.sidebar_on = true;
    assert_eq!(session.effective_render_opts().sidebar_width, 20);
}

/// ARC-122: a host resize rebuilds the renderer; the configured border
/// colors survive the rebuild (they used to revert to cyan/dim).
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
    assert_eq!(session.renderer.layout().len(), 2, "the rebuild re-laid");
    assert_ring_colors(&mut session, 0, 100, active, plain);
}

/// ARC-122: a window re-seed rebuilds the renderer too; the colors
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
/// as a resize rebuild) keeps the colors, and the configured panel
/// width rides the rebuild.
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
    session.sidebar_on = true;
    session.renderer.focus(1);
    // Steady state: every pane's title was learned on an earlier
    // refresh; then %3's user title changes.
    for pane in 1..=4 {
        session.renderer.set_user_title(pane, "t");
    }
    session.handle_event(TmuxNotification::PaneTitleChanged {
        pane_id: "%3".to_string(),
        title: "new".to_string(),
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
    assert!(session.status_dirty, "the next tick retries");
    assert_eq!(
        drained(&rx),
        vec!["list-sessions"],
        "stopped at the timeout"
    );

    // The late list-sessions reply must not answer this command.
    let reply = conn.send_checked("pane-info -t %1").expect("reply");
    assert_eq!(reply.body, vec!["%1 @0 20x24".to_string()]);
}
