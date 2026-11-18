//! WindowSession suites: alt-screen exit, context menus, and the command menu.

use super::*;

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
/// layouts, keybinds, reload config, detach), the footer.
#[test]
fn compose_command_menu_maps_rows_to_actions() {
    let (rows, mapping) = compose_menu_panel(&MenuTarget::Commands, None, false);
    assert_eq!(rows.len(), mapping.len(), "rows and mapping stay parallel");
    assert_eq!(rows[0].text, " settings ");
    assert_eq!(rows[1].text, " Layouts… ");
    assert_eq!(rows[2].text, " keybinds ");
    assert_eq!(rows[3].text, " reload config ");
    assert_eq!(rows[4].text, " detach ");
    assert_eq!(
        mapping,
        vec![
            Some(MenuAction::Settings),
            Some(MenuAction::Layouts),
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
    click_menu_row(&mut session, &mut conn, 2);
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
    click_menu_row(&mut session, &mut conn, 3);
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
    click_menu_row(&mut session, &mut conn, 5);
    assert!(session.menu().is_some(), "the footer consumes the click");
    assert!(!session.detach_requested);
    click_menu_row(&mut session, &mut conn, 4);
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
    // No-button motion over the `reload config` row (row 3).
    hover_menu_row(&mut session, &mut conn, 3);
    assert_eq!(
        session.menu().expect("menu up").selected,
        3,
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
        4,
        "the arrow moved relative to the hover"
    );
    assert_eq!(
        session.renderer.overlay_highlights,
        vec![(
            4,
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
