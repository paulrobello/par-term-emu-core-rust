//! Kitty graphics on the host (config `kitty-graphics`): placement
//! mapping through the pane layout, the host byte diff, the gates, and
//! the replay/occlusion rules.

use super::*;

/// A kitty transmit-and-display of a `w`x`h` RGBA image with extra
/// placement keys (`c=`, `r=`, `C=1`, …).
fn kitty_image(w: usize, h: usize, keys: &str) -> Vec<u8> {
    use base64::Engine as _;
    let pixels = vec![0x7fu8; w * h * 4];
    let data = base64::engine::general_purpose::STANDARD.encode(pixels);
    format!("\x1b_Ga=T,f=32,s={w},v={h},{keys};{data}\x1b\\").into_bytes()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A two-pane renderer with a 20-column side panel: pane 2's content
/// starts at frame column 20 + 40.
fn two_pane_with_sidebar() -> PaneRenderer {
    let side = 20u16;
    let mut renderer = PaneRenderer::new(80 + side, 24, Glyphs::Unicode);
    renderer.set_sidebar_width(side);
    renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    renderer
}

/// An image the right pane draws at its cell (2, 3) maps to the frame
/// cell through the side panel and the pane's rect, spanning the cells
/// its pixel size covers at the reported cell size (20x40 px at 10x20 =
/// 2x2 cells).
#[test]
fn placement_maps_through_the_layout_and_sidebar() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(2, b"\x1b[4;3H");
    renderer.feed_output(2, &kitty_image(20, 40, "C=1"));
    let placements = renderer.kitty_placements();
    assert_eq!(placements.len(), 1);
    let g = placements[0].geom;
    assert_eq!((g.x, g.y), (20 + 40 + 2, 3));
    assert_eq!((g.cols, g.rows), (2, 2));
    assert_eq!((g.src_w, g.src_h), (20, 40));
}

/// The emulator's image math follows the reported cell size: an
/// unsized image advances the mirror's cursor by the rows the DAEMON's
/// pane advanced (the 10x20 handshake default, then the probed size),
/// not core's (1, 2) half-block default.
#[test]
fn emulators_use_the_reported_cell_size_for_image_spans() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(1, &kitty_image(20, 40, ""));
    assert_eq!(renderer.pane_terminal(1).unwrap().cursor().row, 2);
    let placements = renderer.kitty_placements();
    assert_eq!((placements[0].geom.cols, placements[0].geom.rows), (2, 2));

    renderer.set_cell_px((5, 10));
    let placements = renderer.kitty_placements();
    assert_eq!(
        (placements[0].geom.cols, placements[0].geom.rows),
        (2, 2),
        "an existing image keeps the cells its grid reserved (as the daemon's copy does)"
    );
    renderer.feed_output(1, &kitty_image(20, 40, ""));
    assert_eq!(
        renderer.pane_terminal(1).unwrap().cursor().row,
        6,
        "a later image spans at the probed size"
    );
    renderer.reset_pane(2);
    renderer.feed_output(2, &kitty_image(20, 40, ""));
    assert_eq!(
        renderer.pane_terminal(2).unwrap().cursor().row,
        4,
        "a fresh emulator starts at the probed size"
    );
}

/// An image larger than its pane's visible area is cropped to it: the
/// extent clamps to the pane and the source rect shrinks in proportion.
#[test]
fn placement_crops_to_the_pane_content_view() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(1, b"\x1b[1;37H");
    renderer.feed_output(1, &kitty_image(100, 40, "C=1"));
    let g = renderer.kitty_placements()[0].geom;
    assert_eq!((g.x, g.y), (20 + 36, 0));
    assert_eq!(
        (g.cols, g.rows),
        (4, 2),
        "10 cols wide, 4 fit before the edge"
    );
    assert_eq!(g.src_w, 40, "4/10 of the 100 px source");
}

/// The host bytes: one upload (`a=t`, RGBA, quiet) and one placement
/// at the host cell (frame row + the tab strip), `C=1` so the host
/// cursor never moves. A second frame with nothing changed writes
/// nothing; a moved placement re-places without re-uploading; an image
/// gone deletes its placement and frees the host copy.
#[test]
fn host_bytes_upload_once_place_and_diff() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(2, b"\x1b[4;3H");
    renderer.feed_output(2, &kitty_image(20, 40, "C=1"));
    let mut host = crate::mux::attach::render::kitty::HostGraphics::default();
    host.supported = true;
    let to_host = |x: u16, y: u16| (x, y + 1);

    let first = text(&host.update(&renderer.kitty_placements(), to_host));
    assert!(
        first.contains("\x1b_Ga=t,t=d,f=32,s=20,v=40,i=10000,q=2,m=1;"),
        "{first:?}"
    );
    assert!(
        first.contains("\x1b[5;63H\x1b_Ga=p,i=10000,p=1,c=2,r=2,C=1,q=2,w=20,h=40;\x1b\\"),
        "{first:?}"
    );
    assert!(
        host.update(&renderer.kitty_placements(), to_host)
            .is_empty(),
        "an unchanged frame writes nothing"
    );

    renderer.scroll_viewport(2, 0);
    renderer.feed_output(2, b"\x1b[24;1H\n");
    let moved = text(&host.update(&renderer.kitty_placements(), to_host));
    assert!(!moved.contains("a=t"), "no re-upload: {moved:?}");
    assert!(
        moved.contains("\x1b[4;63H\x1b_Ga=p,i=10000,p=1,"),
        "{moved:?}"
    );

    renderer.feed_output(2, b"\x1b[2J");
    let gone = text(&host.update(&renderer.kitty_placements(), to_host));
    assert!(
        gone.contains("\x1b_Ga=d,d=i,i=10000,p=1,q=2;\x1b\\"),
        "{gone:?}"
    );
    assert!(
        gone.contains("\x1b_Ga=d,d=I,i=10000,q=2;\x1b\\"),
        "{gone:?}"
    );
}

/// A full repaint (resize, zoom, window switch) first deletes every
/// placement and image the client put on the host — a host whose screen
/// clear keeps kitty state would otherwise show the image at its old
/// cell, and an image the refit's replay dropped would never be freed —
/// then the next frame re-uploads and re-places what is still wanted.
#[test]
fn invalidate_deletes_what_the_host_holds_then_redraws() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(2, b"\x1b[4;3H");
    renderer.feed_output(2, &kitty_image(20, 40, "C=1"));
    let mut host = crate::mux::attach::render::kitty::HostGraphics::default();
    host.supported = true;
    let to_host = |x: u16, y: u16| (x, y + 1);
    assert!(!host
        .update(&renderer.kitty_placements(), to_host)
        .is_empty());

    let stale = text(&host.invalidate());
    assert!(
        stale.contains("\x1b_Ga=d,d=i,i=10000,p=1,q=2;\x1b\\"),
        "{stale:?}"
    );
    assert!(
        stale.contains("\x1b_Ga=d,d=I,i=10000,q=2;\x1b\\"),
        "{stale:?}"
    );
    assert!(!host.holds_any());
    let redraw = text(&host.update(&renderer.kitty_placements(), to_host));
    assert!(
        redraw.contains("a=t,") && redraw.contains("a=p,"),
        "{redraw:?}"
    );

    // A replay that dropped the image: invalidate still frees it.
    renderer.feed_replay(2, b"");
    let stale = text(&host.invalidate());
    assert!(stale.contains("a=d,d=I,"), "{stale:?}");
    assert!(host
        .update(&renderer.kitty_placements(), to_host)
        .is_empty());
}

/// Large images upload in 4096-byte base64 chunks; only the first
/// carries the keys, the rest `m=` alone.
#[test]
fn upload_chunks_carry_keys_only_on_the_first() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(1, &kitty_image(40, 40, "C=1"));
    let mut host = crate::mux::attach::render::kitty::HostGraphics::default();
    host.supported = true;
    let bytes = text(&host.update(&renderer.kitty_placements(), |x, y| (x, y)));
    let chunks: Vec<&str> = bytes.split("\x1b_G").filter(|c| c.contains("m=")).collect();
    // 40x40 RGBA = 6400 bytes = 8536 base64 bytes: three chunks.
    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].starts_with("a=t,") && chunks[0].contains("m=1;"));
    assert!(chunks[1].starts_with("q=2,m=1;"));
    assert!(chunks[2].starts_with("q=2,m=0;"));
    assert!(chunks[..2]
        .iter()
        .all(|c| c.split(';').nth(1).unwrap().len() == 4096 + 2));
}

/// The session gates: nothing goes to the host until the host answered
/// the kitty query; `kitty-graphics = false` takes what was drawn off
/// the host; a modal hides the placements (they would paint over it)
/// and closing it brings them back.
#[test]
fn session_gates_on_support_and_config_and_hides_under_modals() {
    let mut session = WindowSession::new(80, 24);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.feed_output(1, &kitty_image(20, 40, "C=1"));
    let mut sink = RecordingSink::default();

    session.frame(&mut sink);
    assert!(
        !text(&sink.raw).contains("\x1b_G"),
        "unsupported host: no graphics"
    );

    session.set_kitty_supported(true);
    session.frame(&mut sink);
    let drawn = text(&std::mem::take(&mut sink.raw));
    assert!(drawn.contains("a=p,i=10000,p=1,"), "{drawn:?}");
    assert!(
        !sink.placements.is_empty(),
        "the graphics write re-places the host cursor"
    );

    session
        .renderer
        .set_overlay(Some((" help ", Vec::new(), None)));
    session.frame(&mut sink);
    let hidden = text(&std::mem::take(&mut sink.raw));
    assert!(hidden.contains("a=d,d=i,i=10000,p=1"), "{hidden:?}");
    assert!(
        !hidden.contains("a=d,d=I"),
        "the upload is kept: {hidden:?}"
    );
    session.renderer.set_overlay(None);
    session.frame(&mut sink);
    let back = text(&std::mem::take(&mut sink.raw));
    assert!(back.contains("a=p,i=10000,p=1,"), "{back:?}");
    assert!(!back.contains("a=t"), "no re-upload: {back:?}");

    session.set_kitty_enabled(false);
    session.frame(&mut sink);
    let off = text(&std::mem::take(&mut sink.raw));
    assert!(off.contains("a=d,d=I,i=10000"), "{off:?}");
    session.renderer.feed_output(1, b"x");
    session.frame(&mut sink);
    assert!(sink.raw.is_empty(), "off stays quiet");
}

/// The settings dialog's general tab carries `kitty graphics` (the
/// CLAUDE.md rule: every TUI `[client]` key is editable there): Enter
/// flips it, writes the key, and the reload applies it live.
#[test]
fn settings_row_toggles_kitty_graphics_live() {
    let (_rx, mut conn, mut session) = command_menu_session("kittysettings");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[client]\n").unwrap();
    session.settings_path = Some(path.clone());
    session.enter_settings();
    session.route_stdin_bytes(&mut conn, b"\t\t");
    let rows = overlay_text(&session);
    assert!(
        rows.iter()
            .any(|r| r.contains("kitty graphics") && r.contains("on")),
        "the row shows the default in force: {rows:#?}"
    );
    assert!(session.kitty.enabled);
    session.route_stdin_bytes(&mut conn, b"jjjj\r");
    settle(&mut session, &mut conn);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("kitty-graphics = false"), "{written}");
    assert!(!session.kitty.enabled, "applied live");
}

/// A replay rebuilds the grid from the daemon's screen state, which
/// carries no images: the mirror's graphics are dropped first, so none
/// survives at a stale row over the rebuilt text.
#[test]
fn replay_drops_the_mirrors_graphics() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(1, b"\x1b[10;1H");
    renderer.feed_output(1, &kitty_image(20, 40, ""));
    assert_eq!(renderer.kitty_placements().len(), 1);
    let replay = renderer
        .pane_terminal(1)
        .unwrap()
        .export_screen_restore_sequence();
    renderer.feed_replay(1, replay.as_bytes());
    assert!(renderer.kitty_placements().is_empty());
}

/// A placement the in-app toast card overlaps is left out (an image
/// would paint over the card).
#[test]
fn toast_overlap_suppresses_the_placement() {
    let mut renderer = two_pane_with_sidebar();
    renderer.feed_output(2, b"\x1b[21;30H");
    renderer.feed_output(2, &kitty_image(60, 40, "C=1"));
    assert_eq!(renderer.kitty_placements().len(), 1);
    renderer.set_toast(Some(crate::mux::attach::render::toast::Toast {
        title: "agent done".to_string(),
        body: "pane".to_string(),
        message: None,
        status: crate::mux::attach::panels::AgentStatus::Done,
        pane: None,
    }));
    assert!(renderer.kitty_placements().is_empty());
}
