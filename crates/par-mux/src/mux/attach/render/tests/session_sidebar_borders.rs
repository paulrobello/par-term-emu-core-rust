//! WindowSession suites: sidebar clicks, help modal, and pane borders.

use super::*;

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
        .map(|x| session.renderer.frames.buffer[(x, 0)].symbol())
        .collect();
    assert!(
        first.contains("alpha"),
        "the panel opens with the first workspace row"
    );
    let panel_text: String = (0..22)
        .flat_map(|y| (0..19).map(move |x| (x, y)))
        .map(|(x, y)| session.renderer.frames.buffer[(x, y)].symbol())
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
