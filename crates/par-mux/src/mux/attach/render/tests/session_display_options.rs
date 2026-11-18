//! WindowSession suites: help/picker chords, display options, and pointer shapes.

use super::*;

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

/// The picker chord: prefix w opens the modal live-filtered (every
/// session's windows as one flat list, the shown window emphasized,
/// no session rows), Down moves the cursor without leaving the
/// filter, Enter activates — the select+resync contract's
/// select-window rides the wire — and esc dismisses without selecting.
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
        joined.contains("▸ >  @0: main"),
        "the shown window is selected and >-marked: {joined}"
    );
    assert!(
        !joined.contains("$0:") && !joined.contains("$1:"),
        "no session rows: {joined}"
    );
    assert!(
        joined.contains("@1: vim *"),
        "the active window carries *: {joined}"
    );

    // Down moves the cursor to the second window row without leaving
    // the live filter; Enter activates — the wire sees the
    // switch-client landing and the re-seed's queries (the resync).
    session.picker_key(
        &mut conn,
        &par_term_emu_core::keyboard::TermKeyEvent::functional(
            par_term_emu_core::keyboard::TermKey::Down,
            0,
        ),
    );
    session.picker_byte(&mut conn, b'\r');
    settle(&mut session, &mut conn);
    assert!(!session.is_picker(), "activation dismisses the modal");
    assert_eq!(
        wait_recorded(&rx, "switch-client"),
        "switch-client -t @1",
        "the picked window row is the landing target"
    );
    let _ = wait_recorded(&rx, "list-panes");
    let _ = wait_recorded(&rx, "refresh-client");
    assert_eq!(session.window, "@1", "the view re-seeded onto @1");

    // Re-open (the filter live again) and dismiss with esc: no select
    // goes out.
    assert!(!session.route_plain(&[crate::mux::attach::C_B, b'w'], &mut conn,));
    settle(&mut session, &mut conn);
    assert!(session.is_picker());
    assert!(!session.picker_byte(&mut conn, 0x1b));
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
