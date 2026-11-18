//! WindowSession suites: resize-in-place replay, reland, and kitty graphics.

use super::*;

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
    session.reseed_window(&mut conn, "@0", false);
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
    session.reseed_window(&mut conn, "@1", false);
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

/// The re-land ghost report: a re-seed repaints the view IN PLACE —
/// unlike the seed and `resize_to` it never clears the host — so the
/// composed host screen after a re-land (the old view's flush, then the
/// re-land's flush on top) must hold the new view alone, with no stale
/// fragment of the previous window anywhere the new view is blank.
/// Pinning this also pins the invariant the whole re-land contract
/// leans on: after `invalidate_frame` the next frame's diff re-emits
/// every cell (the core grid's blank cells carry a non-default fg, so
/// they diff against a blank baseline) — if that ever stops holding,
/// the previous window's fragments start surviving here first.
#[test]
fn reland_repaints_the_cells_the_new_view_leaves_blank() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (_rx, mut conn, mut session) = two_pane_session(
        "reland-ghost",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    // View A: pane %1 carries a dialog row and figures; frame it onto
    // the (simulated) host.
    session
        .renderer
        .feed_output(1, b"[ approved ] confirm?\r\ntotal 12 34 56\r\n");
    session.renderer.feed_output(2, b"side pane body\r\n");
    let mut view_a = RecordingSink::default();
    session.frame(&mut view_a);

    // Re-land on @1, whose panes are nearly blank: the %layout-change
    // rides the size report, so the reseed installs the new layout
    // synchronously.
    assert!(
        session.reseed_window(&mut conn, "@1", false),
        "the reseed landed"
    );
    session.renderer.feed_output(5, b"$");
    session.renderer.feed_output(6, b"$");
    let mut view_b = RecordingSink::default();
    session.frame(&mut view_b);

    let text = composed_host_text(&view_a, &view_b);
    assert!(
        text.contains('$'),
        "the new view reached the host: {text:?}"
    );
    assert!(
        !text.contains("approved"),
        "the previous view's dialog row survived the re-land: {text:?}"
    );
    assert!(
        !text.contains("confirm?") && !text.contains("12 34 56"),
        "stale fragments of the previous view survived: {text:?}"
    );
}

/// The same re-land contract through the LAGGING `%layout-change` path
/// (a roster click or split whose layout broadcast misses the reseed's
/// drain): the reseed returns with the old view still laid out, the
/// broadcast parks `pending.layout` on a later tick, and `apply_pending`
/// installs it with NO invalidation — the frame diffs the new view
/// against the old baseline in place. The composed host screen must
/// still hold the new view alone.
#[test]
fn reland_via_lagging_layout_leaves_no_stale_fragments() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (_rx, mut conn, mut session) = two_pane_session(
        "reland-lag",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session
        .renderer
        .feed_output(1, b"[ approved ] confirm?\r\ntotal 12 34 56\r\n");
    session.renderer.feed_output(2, b"side pane body\r\n");
    let mut view_a = RecordingSink::default();
    session.frame(&mut view_a);

    // The reseed's drain misses the broadcast (nothing notify-scripted);
    // the layout arrives afterwards and parks for the pump.
    assert!(
        session.reseed_window(&mut conn, "@1", false),
        "the reseed landed"
    );
    session.on_layout_change(layout, layout, "*");
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("pending install");
    session.renderer.feed_output(5, b"$");
    session.renderer.feed_output(6, b"$");
    let mut view_b = RecordingSink::default();
    session.frame(&mut view_b);

    let text = composed_host_text(&view_a, &view_b);
    assert!(
        text.contains('$'),
        "the new view reached the host: {text:?}"
    );
    assert!(
        !text.contains("approved"),
        "the previous view's dialog row survived the re-land: {text:?}"
    );
    assert!(
        !text.contains("confirm?") && !text.contains("12 34 56"),
        "stale fragments of the previous view survived: {text:?}"
    );
}

/// A transmit-and-display of a `w`x`h` RGBA image (the kitty.rs test
/// helper's shape, local so the session suite stays self-contained).
fn reland_kitty_image(w: usize, h: usize) -> Vec<u8> {
    use base64::Engine as _;
    let pixels = vec![0x7fu8; w * h * 4];
    let data = base64::engine::general_purpose::STANDARD.encode(pixels);
    format!("\x1b_Ga=T,f=32,s={w},v={h},C=1;{data}\x1b\\").into_bytes()
}

/// The re-land's kitty half: a re-seed replaces the view, so what the
/// previous one put on the host is deleted at the seam — the same
/// contract the seed and `resize_to` serve with their own
/// `kitty.invalidate()`. The new view's replays carry no images, so the
/// frame after the re-land deletes the old placement and image and
/// never draws the previous view's graphic again.
#[test]
fn reseed_window_deletes_the_previous_views_kitty_graphics() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut notify = std::collections::HashMap::new();
    notify.insert(
        "refresh-client -t %5 -C 80x23".to_string(),
        format!("%layout-change @1 {layout} {layout} *"),
    );
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (_rx, mut conn, mut session) = two_pane_session(
        "reland-kitty",
        FakeScript {
            replies,
            notify,
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"\x1b[4;3H");
    session.renderer.feed_output(1, &reland_kitty_image(20, 40));
    session.set_kitty_supported(true);
    let mut view_a = RecordingSink::default();
    session.frame(&mut view_a);
    assert!(
        String::from_utf8_lossy(&view_a.raw).contains("a=p,i=10000"),
        "view A drew the image: {:?}",
        String::from_utf8_lossy(&view_a.raw)
    );

    assert!(
        session.reseed_window(&mut conn, "@1", false),
        "the reseed landed"
    );
    let mut view_b = RecordingSink::default();
    session.frame(&mut view_b);
    let drawn = String::from_utf8_lossy(&view_b.raw);
    assert!(
        drawn.contains("\x1b_Ga=d,d=i,i=10000,p=1,q=2;\x1b\\"),
        "the old placement must be deleted at the re-land: {drawn:?}"
    );
    assert!(
        drawn.contains("\x1b_Ga=d,d=I,i=10000,q=2;\x1b\\"),
        "the old image must be freed at the re-land: {drawn:?}"
    );
    assert!(
        !drawn.contains("a=p,") && !drawn.contains("a=t,"),
        "the previous view's graphic must not paint into the new view: {drawn:?}"
    );
}

/// The same seam through the LAGGING `%layout-change` path: the reseed
/// returns with the previous layout still installed, whose emulators
/// still hold their graphics — the next frame's placement diff would
/// keep every old pixel — so the delete must go out at the seam
/// regardless of when the new layout lands.
#[test]
fn reseed_window_deletes_the_kitty_graphics_even_when_the_layout_lags() {
    let layout = "0000,80x23,0,0{40x23,0,0,5,40x23,40,0,6}";
    let mut replies = std::collections::HashMap::new();
    reseed_replies(&mut replies, "@0 main\n@1 * other", "@1", "%5\n%6");
    let (_rx, mut conn, mut session) = two_pane_session(
        "reland-kitty-lag",
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    );
    session.renderer.feed_output(1, b"\x1b[4;3H");
    session.renderer.feed_output(1, &reland_kitty_image(20, 40));
    session.set_kitty_supported(true);
    let mut view_a = RecordingSink::default();
    session.frame(&mut view_a);
    assert!(
        String::from_utf8_lossy(&view_a.raw).contains("a=p,i=10000"),
        "view A drew the image: {:?}",
        String::from_utf8_lossy(&view_a.raw)
    );

    // The reseed's drain misses the broadcast; the old layout stands.
    assert!(
        session.reseed_window(&mut conn, "@1", false),
        "the reseed landed"
    );
    let mut interim = RecordingSink::default();
    session.frame(&mut interim);
    assert!(
        String::from_utf8_lossy(&interim.raw).contains("\x1b_Ga=d,d=i,i=10000,p=1,q=2;\x1b\\"),
        "the seam delete must fire even with the layout event pending: {:?}",
        String::from_utf8_lossy(&interim.raw)
    );
    assert!(
        !String::from_utf8_lossy(&interim.raw).contains("a=t,"),
        "the dropped mirrors must not re-upload the deleted image in the same frame: {:?}",
        String::from_utf8_lossy(&interim.raw)
    );

    session.on_layout_change(layout, layout, "*");
    session
        .apply_pending(&mut conn, &mut RecordingSink::default())
        .expect("pending install");
    let mut view_b = RecordingSink::default();
    session.frame(&mut view_b);
    let drawn = String::from_utf8_lossy(&view_b.raw);
    assert!(
        !drawn.contains("a=t,") && !drawn.contains("a=p,"),
        "the previous view's graphic must not paint into the new view: {drawn:?}"
    );
}

/// The composed host screen of two consecutive flushes, as text: what a
/// terminal that painted `a`'s cells and then `b`'s cells on top shows.
fn composed_host_text(a: &RecordingSink, b: &RecordingSink) -> String {
    let mut screen = vec![vec![' '; 80]; 25];
    for cells in [&a.cells, &b.cells] {
        for &(x, y, ref cell) in cells.iter() {
            if let Some(sym) = cell.symbol().chars().next() {
                if x < 80 && y < 25 {
                    screen[y as usize][x as usize] = sym;
                }
            }
        }
    }
    screen
        .iter()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
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
