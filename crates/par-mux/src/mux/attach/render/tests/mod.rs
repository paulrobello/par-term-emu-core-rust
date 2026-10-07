//! Render-mode unit tests: the renderer, emulator, and frame-flush suites,
//! plus the scripted-daemon helpers the session and chord suites share.

use super::input_route::*;
use super::modal::*;
use super::session::*;
use super::*;
use crate::keyboard::TermKey;
use crate::mux::attach::layout::{parse_layout, parse_layout_triple};
use std::io::BufRead as _;

mod chords;
mod session;

/// The daemon's actual render output for a 50/50 vertical split of an
/// 80x24 window (LayoutTree::render's collapsed N-ary form), matching
/// src/mux/layout.rs's own test expectations.
const TWO_PANE_LAYOUT: &str = "0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}";

/// Criterion 2: two panes fed distinct content render at the rects
/// the daemon's layout names — pane 1's bytes land in cols 0..40,
/// pane 2's in cols 40..80, same rows.
#[test]
fn two_pane_split_renders_at_daemon_rects() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);

    // Pane 1: "LEFT", pane 2: "RIGHT" — through their real emulators,
    // the same bytes a replay/%output carries.
    renderer.feed_output(1, b"LEFT\r\n");
    renderer.feed_output(2, b"RIGHT\r\n");
    assert!(renderer.needs_frame());
    let diff = renderer.render_frame();
    assert!(!diff.is_empty());
    assert!(!renderer.needs_frame(), "frame cleared the dirty flag");

    let buffer_text = |x: u16, y: u16, n: u16| -> String {
        (x..x + n)
            .map(|col| renderer.buffer[(col, y)].symbol())
            .collect()
    };
    assert_eq!(buffer_text(0, 0, 4), "LEFT");
    assert_eq!(buffer_text(40, 0, 5), "RIGHT");

    // The dividers: one vertical boundary at col 39 across the row
    // overlap, drawn as UTF-8 box drawing.
    assert_eq!(renderer.buffer[(39, 0)].symbol(), "│");
    assert_eq!(renderer.buffer[(39, 23)].symbol(), "│");
    // No horizontal dividers in this layout.
    assert_eq!(renderer.buffer[(10, 11)].symbol(), " ");
}

/// Cell mapping: styled bytes (`capture-pane -e` equivalent — the
/// emulator saw the same SGR the -e capture encodes) map to the
/// ratatui cell with fg/bg/attrs intact, truecolor passing through.
#[test]
fn styled_cells_map_fg_bg_and_attrs() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    // Bold truecolor text on SGR-44 blue — exactly what capture-pane
    // -e would carry back as escape bytes.
    renderer.feed_output(1, b"\x1b[1;38;2;10;20;30;44mAB\x1b[0m normal \r\n");
    renderer.render_frame();

    let a = &renderer.buffer[(0, 0)];
    assert_eq!(a.symbol(), "A");
    assert_eq!(a.fg, RtColor::Rgb(10, 20, 30), "truecolor fg passthrough");
    assert_eq!(a.bg, RtColor::Indexed(4), "SGR 44 -> indexed blue bg");
    assert!(a.modifier.contains(RtModifier::BOLD));

    let n = &renderer.buffer[(3, 0)];
    assert_eq!(n.symbol(), "n");
    assert_eq!(n.fg, RtColor::Indexed(7), "post-reset default fg");
}

/// Criterion 3: an output flood between two frames coalesces into one
/// frame — the dirty flag is set-once, the first render consumes it,
/// and the second render diffs empty.
#[test]
fn output_flood_coalesces_at_frame_cadence() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.render_frame(); // settle the layout frame

    // 1000 floods of one line each — a realistic `cat bigfile` burst.
    for i in 0..1000 {
        renderer.feed_output(1, format!("line {i}\r\n").as_bytes());
    }
    renderer.feed_output(2, b"tail\r\n");
    assert!(renderer.needs_frame());

    let diff = renderer.render_frame();
    assert!(
        !diff.is_empty(),
        "the flood's final state reaches the frame"
    );
    assert!(!renderer.needs_frame(), "one frame consumed the flood");
    assert_eq!(
        renderer.render_frame(),
        Vec::new(),
        "the second frame is a no-op — no unbounded redraws"
    );
    // The flood scrolled 1000 lines through 24 rows: the last line
    // written ("line 999" at row 23) moved up one row when its \r\n
    // scrolled the grid, leaving row 22 as the final text row.
    let last = (0..6)
        .map(|c| renderer.buffer[(c, 22)].symbol())
        .collect::<String>();
    assert_eq!(last, "line 9");
}

/// Zoomed windows: the Z-flag triple's visible layout is the zoomed
/// pane alone, full-window; unzooming restores the split and keeps
/// surviving emulators' grids.
#[test]
fn zoomed_triple_renders_one_full_window_pane() {
    let layout =
        crate::mux::attach::layout::parse_layout_triple(TWO_PANE_LAYOUT, "0000,80x24,0,0,2", "Z")
            .expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.feed_output(2, b"ZOOMED\r\n");
    renderer.render_frame();

    let zoomed = (0..6)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(zoomed, "ZOOMED");
    assert_eq!(renderer.layout().len(), 1, "one visible pane while zoomed");

    // Unzoom: the split layout returns and pane 2's grid survives
    // (its emulator was kept, not dropped).
    let split = parse_layout_triple(TWO_PANE_LAYOUT, TWO_PANE_LAYOUT, "").expect("parses");
    renderer.apply_layout(split);
    renderer.feed_output(1, b"LEFT\r\n");
    renderer.render_frame();
    let left = (0..6)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(left, "LEFT  ", "pane 1 on the left half");
    let right = (40..46)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(right, "ZOOMED", "pane 2 kept its grid through the zoom");
}

/// The ` Z ` cue follows the daemon's per-window zoom truth: another
/// client's `resize-pane -Z` reaches this one only as a `Z`-flagged
/// `%layout-change`, so the event (not the local chord alone) sets and
/// clears the cue — even when the geometry is unchanged (a one-pane
/// window zooms to the same rect) — and another window's zoom never
/// touches it.
#[test]
fn layout_change_flags_drive_the_zoom_cue_for_the_shown_window_only() {
    let mut session = WindowSession::new(80, 25);
    session.window = "@0".to_string();
    let change = |window: &str, flags: &str| TmuxNotification::LayoutChange {
        window_id: window.to_string(),
        window_layout: TWO_PANE_LAYOUT.to_string(),
        window_visible_layout: if flags == "Z" {
            "0000,80x24,0,0,2".to_string()
        } else {
            TWO_PANE_LAYOUT.to_string()
        },
        window_raw_flags: flags.to_string(),
    };

    session.handle_event(change("@0", "Z"));
    assert!(
        session.zoomed,
        "a Z-flagged layout for the shown window sets the cue"
    );

    session.handle_event(change("@1", ""));
    assert!(
        session.zoomed,
        "another window's layout leaves the cue alone"
    );

    session.handle_event(change("@0", ""));
    assert!(!session.zoomed, "an unflagged layout clears the cue");

    session.handle_event(change("@1", "Z"));
    assert!(!session.zoomed, "another window's zoom never sets the cue");

    // Unchanged geometry (the layout already applied) still flips it.
    let single = "0000,80x24,0,0,1";
    let one_pane = |flags: &str| TmuxNotification::LayoutChange {
        window_id: "@0".to_string(),
        window_layout: single.to_string(),
        window_visible_layout: single.to_string(),
        window_raw_flags: flags.to_string(),
    };
    session
        .renderer
        .apply_layout(parse_layout(single).expect("parses"));
    session.handle_event(one_pane("Z"));
    assert!(session.zoomed, "a same-geometry zoom still sets the cue");
    session.handle_event(one_pane(""));
    assert!(
        !session.zoomed,
        "a same-geometry unzoom still clears the cue"
    );
}

/// Pane membership changes drop only the gone panes' emulators.
#[test]
fn layout_change_drops_departed_panes() {
    let two = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(two);
    renderer.feed_output(1, b"keep\r\n");
    renderer.render_frame();

    // Pane 1 closes: the layout collapses to pane 2 alone.
    let one = parse_layout("0000,80x24,0,0,2").expect("parses");
    renderer.apply_layout(one);
    renderer.render_frame();
    let solo = (0..4)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(solo, "    ", "pane 2's grid is empty here");
    assert!(renderer.focused().is_some());

    // Back to two panes: pane 1's emulator was dropped with its
    // membership, so its old content does not return.
    renderer.apply_layout(parse_layout(TWO_PANE_LAYOUT).unwrap());
    renderer.render_frame();
    let back = (0..4)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(back, "    ");
}

/// Ascii glyphs: the ACS fallback spells dividers with | - +.
#[test]
fn ascii_glyphs_fallback() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Ascii);
    renderer.apply_layout(layout);
    renderer.render_frame();
    assert_eq!(renderer.buffer[(39, 0)].symbol(), "|");
}

/// Focus: the focused pane's boundary dividers are bold, the others
/// dim; focusing an absent pane changes nothing. The 3-pane N-ary
/// layout has two boundaries — at cols 29 and 59 — so the unfocused
/// one is observable.
/// The stacked-split regression: a horizontal boundary paints the
/// horizontal glyph along the overlap (the halves wave grouped
/// horizontal cells by the along-axis and the bounds check then
/// skipped them — the manual-pass report).
#[test]
fn horizontal_dividers_paint_on_stacked_splits() {
    const STACKED: &str = "0000,80x24,0,0[80x12,0,0,1,80x12,0,12,2]";
    let layout = parse_layout(STACKED).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.render_frame();
    let y = 11u16; // the top pane's last row — the boundary cell
    for x in (0..80u16).step_by(7) {
        assert_eq!(
            renderer.buffer[(x, y)].symbol(),
            "─",
            "the stacked split's horizontal divider paints at column {x}"
        );
    }
    // And nothing transposed: a mid-pane column keeps its content.
    assert_ne!(
        renderer.buffer[(11, 3)].symbol(),
        "─",
        "no transposed vertical streak at the boundary row's coordinate"
    );
}

/// The prefix+arrow navigation: the nearest pane strictly in the
/// direction wins; an edge is `None`.
#[test]
fn pane_in_direction_picks_the_nearest_pane_in_the_direction() {
    const MIXED: &str = "0000,90x24,0,0{30x24,0,0,1,60x24,30,0[60x12,30,0,2,60x12,30,12,3]}";
    let rects = parse_layout(MIXED).expect("parses");
    assert_eq!(pane_in_direction(&rects, 1, PaneDir::Right), Some(2));
    assert_eq!(pane_in_direction(&rects, 3, PaneDir::Up), Some(2));
    assert_eq!(pane_in_direction(&rects, 2, PaneDir::Down), Some(3));
    assert_eq!(pane_in_direction(&rects, 2, PaneDir::Left), Some(1));
    assert_eq!(pane_in_direction(&rects, 1, PaneDir::Left), None);
}

/// The tmux half rule across a three-pane side-by-side row: the
/// shared divider's half nearer the focus highlights, the other
/// stays dim.
#[test]
fn focus_highlights_adjacent_dividers() {
    const THREE_PANE: &str = "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}";
    let layout = parse_layout(THREE_PANE).expect("parses");
    let mut renderer = PaneRenderer::new(90, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.focus(1);
    renderer.render_frame();
    assert!(
        renderer.buffer[(29, 0)].modifier.contains(RtModifier::BOLD),
        "focused pane 1's right divider is bold"
    );
    assert!(
        renderer.buffer[(59, 0)].modifier.contains(RtModifier::DIM),
        "the pane2|pane3 divider (no focused neighbor) is dim"
    );

    renderer.focus(2);
    renderer.mark_all_dirty();
    renderer.render_frame();
    // The middle pane is the RIGHT side of the 1|2 boundary (bottom
    // half highlights) and the LEFT side of the 2|3 boundary (top
    // half highlights).
    assert!(
        renderer.buffer[(29, 23)]
            .modifier
            .contains(RtModifier::BOLD),
        "moving focus to pane 2: the 1|2 divider's bottom half highlights"
    );
    assert!(
        renderer.buffer[(29, 0)].modifier.contains(RtModifier::DIM),
        "moving focus to pane 2: the 1|2 divider's top half stays dim"
    );
    assert!(
        renderer.buffer[(59, 0)].modifier.contains(RtModifier::BOLD),
        "moving focus to pane 2: the 2|3 divider's top half highlights"
    );

    // An absent pane is ignored.
    renderer.focus(99);
    assert_eq!(renderer.focused(), Some(2));
}

/// Wide characters: a CJK cell occupies its rect cell and marks the
/// right spacer skip, and the neighbor's content still lands.
#[test]
fn wide_chars_mark_their_spacer() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.feed_output(1, "世X\r\n".as_bytes());
    renderer.render_frame();
    assert_eq!(renderer.buffer[(0, 0)].symbol(), "世");
    assert_eq!(
        renderer.buffer[(1, 0)].diff_option,
        CellDiffOption::Skip,
        "wide-char spacer is skip"
    );
    assert_eq!(renderer.buffer[(2, 0)].symbol(), "X");
}

/// An empty layout list is a caller contract violation and asserts.
#[test]
#[should_panic(expected = "at least one pane")]
fn empty_layout_asserts() {
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(Vec::new());
}

/// Combining marks ride the symbol: the core terminal normalizes
/// e + U+0301 to the precomposed é, which lands as one cell.
#[test]
fn combining_marks_join_the_symbol() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.feed_output(1, "e\u{0301}x\r\n".as_bytes());
    renderer.render_frame();
    assert_eq!(renderer.buffer[(0, 0)].symbol(), "\u{e9}");
    assert_eq!(renderer.buffer[(1, 0)].symbol(), "x");
}

/// Criterion 3 (client side): a wheel-up over a pane that does NOT own
/// the mouse scrolls the pane's client view into its scrollback — the
/// rect's top rows show the newest history lines, and the live screen
/// shifts down. A pane that DOES own the mouse leaves the scroll at 0
/// (its wheel is forwarded instead).
#[test]
fn wheel_scrolls_client_scrollback_when_pane_has_no_mouse_mode() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.render_frame(); // settle

    // Pane 1 floods 30 lines through its 24-row pane: the overflow
    // lands in scrollback, the rest stays on screen.
    for i in 0..30 {
        renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
    }
    renderer.render_frame();

    // The view's authority BEFORE scrolling: a snapshot of the grid.
    let grid = renderer.pane_terminal(1).unwrap().active_grid().clone();
    let len = grid.scrollback_len();
    let offset = 3usize;
    let line_text = |cells: &[crate::cell::Cell]| -> String {
        cells
            .iter()
            .take(8)
            .map(|c| c.c().to_string())
            .collect::<String>()
    };
    let expected_top = line_text(grid.scrollback_line(len - offset).expect("history"));

    // Wheel up 3 over pane 1 (any point of its rect): offset 3.
    assert!(renderer.wheel_scroll(10, 5, 3));
    renderer.render_frame();
    // View row 0 now shows the newest unscrolled-back history line.
    let top: String = (0..8)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    assert_eq!(top, expected_top, "the top row scrolled into history");

    // Wheel down returns to the live view: row 0 shows live row 0.
    assert!(renderer.wheel_scroll(10, 5, -3));
    renderer.render_frame();
    let painted: String = (0..8)
        .map(|c| renderer.buffer[(c, 0)].symbol())
        .collect::<String>();
    let live = line_text(grid.row(0).expect("live row"));
    assert_eq!(painted, live, "back at the live view");
}

/// A pane that owns mouse tracking never scrolls client-side: the
/// wheel is the pane's.
#[test]
fn wheel_does_not_scroll_a_pane_that_owns_the_mouse() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.render_frame();
    for i in 0..30 {
        renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
    }
    // Pane 1 enables normal mouse tracking (DECSET 1000) through the
    // same %output path the app would use.
    renderer.feed_output(1, b"\x1b[?1000h");
    renderer.render_frame();

    assert!(!renderer.wheel_scroll(10, 5, 3), "owning pane consumes");
}

/// pane_at: the rect containing a window-relative point. The daemon's
/// gap-free tiling means a divider OVERLAYS a content column of one
/// pane's rect, so a point on the divider resolves to the pane whose
/// column it is (pane 1 owns cols 0..40 here, divider included);
/// outside-the-window points find nothing.
#[test]
fn pane_at_maps_points_to_their_rects() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    assert_eq!(renderer.pane_at(0, 0).map(|r| r.pane), Some(1));
    assert_eq!(renderer.pane_at(39, 12).map(|r| r.pane), Some(1));
    assert_eq!(renderer.pane_at(40, 12).map(|r| r.pane), Some(2));
    assert_eq!(renderer.pane_at(79, 23).map(|r| r.pane), Some(2));
    assert_eq!(renderer.pane_at(80, 0), None, "outside the window");
    assert_eq!(renderer.pane_at(0, 24), None, "below the window");
}

/// Emulator input-mode tracking: DECCKM and mouse mode arrive through
/// the pane's own output bytes (replay or %output — same stream), and
/// feed resets the client scroll to live.
#[test]
fn emulator_tracks_input_modes_and_feed_resets_scroll() {
    let mut emulator = PaneEmulator::new(7, 80, 24);
    assert!(!emulator.application_cursor());
    assert!(!emulator.owns_mouse());
    emulator.feed(b"\x1b[?1h\x1b[?1000h");
    assert!(emulator.application_cursor());
    assert!(emulator.owns_mouse());
    assert_eq!(
        emulator.mouse_encoding(),
        crate::mouse::MouseEncoding::Default
    );
    emulator.feed(b"\x1b[?1006h");
    assert_eq!(emulator.mouse_encoding(), crate::mouse::MouseEncoding::Sgr);
    // Scroll needs actual history to move into (the offset clamps to
    // the scrollback extent), so overflow the 24-row pane first;
    // feed output afterwards — the output snaps the view back to
    // live.
    for i in 0..30 {
        emulator.feed(format!("line-{i}\r\n").as_bytes());
    }
    assert_eq!(emulator.scroll_by(2), 2);
    emulator.feed(b"x");
    assert_eq!(emulator.scroll_offset(), 0);
}

/// The mode-aware key re-encode through the renderer's pane terminal:
/// the pane's DECCKM state decides the arrow spelling (criterion 1's
/// unit-level pin; the parser-level one is in `attach::input`).
#[test]
fn key_reencode_reads_the_panes_tracked_decckm() {
    use crate::keyboard::{encode_key, TermKey, TermKeyEvent};
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.feed_output(1, b"\x1b[?1h"); // DECCKM on
    let term = renderer.pane_terminal(1).expect("pane 1");
    assert!(term.application_cursor());
    assert_eq!(
        encode_key(&TermKeyEvent::functional(TermKey::Up, 0), term),
        b"\x1bOA"
    );
}

/// Criterion 2: entering scroll mode holds the view one viewport up
/// from live and holds it against pane output; arrows move the
/// viewport; exiting snaps to live. q/Enter are the session's
/// business (routed in `route_plain`); the renderer only models the
/// offset.
#[test]
fn scroll_mode_holds_view_against_output_and_snaps_on_exit() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    // 30 numbered lines through the 24-row pane: 6+ in scrollback.
    for i in 0..30 {
        renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
    }
    renderer.render_frame();

    // A pane with no scrollback refuses scroll mode.
    assert!(!renderer.enter_scroll_mode(2), "pane 2 has no history");
    assert!(renderer.enter_scroll_mode(1), "pane 1 has history");
    let history = renderer
        .pane_terminal(1)
        .map(|t| t.active_grid().scrollback_len())
        .unwrap_or(0);
    let viewport = renderer.scroll_offset_of(1);
    assert_eq!(
        viewport,
        24usize.min(history),
        "one viewport up, clamped to the history extent"
    );
    assert!(renderer.scroll_mode_active(1));

    // Pane output while held does NOT snap to live — the view holds.
    renderer.feed_output(1, b"NEW-LINE\r\n");
    assert_eq!(
        renderer.scroll_offset_of(1),
        viewport,
        "the hold survives pane output"
    );
    // The offset clamps when history shrinks relative to the view.
    renderer.scroll_viewport(1, 1);
    assert_eq!(renderer.scroll_offset_of(1), viewport + 1);

    // Arrows-equivalent: viewport down past live clamps to the max
    // (live is reached at offset 0 only via exit).
    renderer.scroll_viewport(1, -(viewport as isize + 10));
    assert_eq!(renderer.scroll_offset_of(1), 0, "clamped at live");

    // Exit: the hold clears and the view is live; fresh output is
    // the pane's business again.
    renderer.exit_scroll_mode(1);
    assert!(!renderer.scroll_mode_active(1));
    renderer.feed_output(1, b"x");
    assert_eq!(renderer.scroll_offset_of(1), 0);
}

/// Scroll mode on an unknown pane is a no-op.
#[test]
fn scroll_mode_ignores_unknown_panes() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    assert!(!renderer.enter_scroll_mode(99));
    assert!(!renderer.scroll_mode_active(99));
    renderer.exit_scroll_mode(99);
    renderer.scroll_viewport(99, 5);
    assert_eq!(renderer.scroll_offset_of(99), 0);
}

/// While the scroll viewport is up, painted rows come from history:
/// the rect's top row shows a scrollback line, not live row 0.
#[test]
fn scroll_viewport_paints_history_rows() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    for i in 0..30 {
        renderer.feed_output(1, format!("hist-{i:02}\r\n").as_bytes());
    }
    renderer.render_frame();
    let grid = renderer.pane_terminal(1).unwrap().active_grid().clone();

    assert!(renderer.enter_scroll_mode(1));
    renderer.render_frame();
    let offset = renderer.scroll_offset_of(1);
    let expected = grid
        .scrollback_line(grid.scrollback_len() - offset)
        .expect("history line");
    let expected: String = expected.iter().take(8).map(|c| c.c().to_string()).collect();
    let top: String = (0..8).map(|c| renderer.buffer[(c, 0)].symbol()).collect();
    assert_eq!(top, expected, "the viewport paints from history");
}

/// Regression (render-mode target-less resolution): a `list-sessions`
/// line is `$N: name`, so a whitespace split keeps the colon and the
/// daemon's id parser rejects `$0:` — the client exited with "the
/// session has no windows". The resolution must strip the colon, pick
/// the NEWEST session (highest id — the documented default), and its
/// active window's active pane.
#[test]
fn target_less_resolution_takes_newest_session_without_colon() {
    use crate::mux::attach::conn;
    // The Listener trait supplies `.accept()` on unix only; the Windows
    // named-pipe listener accepts inherently.
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;
    use std::io::{BufRead as _, BufReader, Write as _};

    // Scripted daemon: two sessions (id 0 and 3, out of order to prove
    // the newest pick is by id, not line order), each with windows, the
    // newest session's window @9 marked active.
    let mut path = std::env::temp_dir();
    path.push(format!(
        "par-mux-attach-render-resolve-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use interprocess::TryClone as _;
        let Ok(stream) = listener.accept() else {
            return;
        };
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            number += 1;
            let reply = match trimmed {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => String::new(),
                "list-sessions" => "$0: old\n$3: new\n".to_string(),
                "list-windows -t $0" => "@1 - old-win\n".to_string(),
                "list-windows -t $3" => "@7 - mid\n@9 * new-win\n".to_string(),
                "list-panes -t @9" => "%5 0 -\n%8 1 *".to_string(),
                _ => String::new(),
            };
            tx.send(trimmed.to_owned()).ok();
            writer
                .write_all(crate::mux::emit_block(number, &reply, true).as_bytes())
                .ok();
            writer.flush().ok();
        }
    });

    let mut conn = conn::AttachConn::connect(&path).expect("connect");
    drop(rx);
    let (window, pane) = resolve_window_and_pane(&mut conn, None).expect("resolve");
    assert_eq!(window, "@9", "the newest session's active window");
    assert_eq!(pane, "%8", "the active pane of that window");
}

/// Fix 1 (cursor mapping): the focused pane's tracked cursor cell +
/// rect origin maps to the window-relative cell the host cursor is
/// placed at. Hidden while the view is scrolled off live, when the
/// pane hid its cursor (DECTCEM), and for an absent focus.
#[test]
fn focused_cursor_maps_cell_plus_origin_and_hides_when_scrolled() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.focus(2);

    // Pane 2's emulator sits at rect (40, 0); move its tracked cursor
    // to (5, 3) through the same %output path the pane writes.
    renderer.feed_output(2, b"\r\n\r\n\r\n     ");
    let tracked = renderer.pane_terminal(2).expect("pane 2").cursor();
    assert_eq!((tracked.col, tracked.row), (5, 3), "precondition");
    let origin_x = renderer
        .layout()
        .iter()
        .find(|r| r.pane == 2)
        .map(|r| r.x)
        .unwrap_or(0);
    assert_eq!(
        renderer.focused_cursor(),
        Some((origin_x + 5, 3, tracked.style)),
        "tracked cell + rect origin"
    );

    // Scroll pane 2's client view off live: the live cell is not on
    // screen, so the cursor hides. Pane 2 needs scrollback first (the
    // offset clamps to the history extent); 30 lines through its
    // 24-row pane leaves 6+ in history and parks the tracked cursor
    // at the bottom-left of its grid.
    for i in 0..30 {
        renderer.feed_output(2, format!("hist-{i:02}\r\n").as_bytes());
    }
    assert!(
        renderer.wheel_scroll(45, 0, 3),
        "pane 2 scrolls client-side"
    );
    assert_eq!(renderer.scroll_offset_of(2), 3, "view off live");
    assert_eq!(renderer.focused_cursor(), None, "scrolled view hides");

    // Back to live: the cursor reappears at the tracked cell —
    // bottom-left of pane 2's grid after the history flood.
    assert!(renderer.wheel_scroll(45, 0, -3));
    let tracked = renderer.pane_terminal(2).expect("pane 2").cursor();
    assert_eq!(
        (tracked.col, tracked.row),
        (0, 23),
        "precondition: flood parked it"
    );
    assert_eq!(
        renderer.focused_cursor(),
        Some((40, 23, tracked.style)),
        "live again: tracked cell + origin"
    );

    // DECTCEM hide/show through the pane's own bytes.
    renderer.feed_output(2, b"\x1b[?25l");
    assert_eq!(renderer.focused_cursor(), None, "pane hid its cursor");
    renderer.feed_output(2, b"\x1b[?25h");
    assert!(renderer.focused_cursor().is_some(), "pane re-showed");

    // A fresh renderer still maps — its emulators' cursors are
    // visible at the rect origin, which is exactly right for a pane
    // whose shell sits at home.
    let mut fresh = PaneRenderer::new(80, 24, Glyphs::Unicode);
    fresh.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    assert_eq!(
        fresh.focused_cursor(),
        Some((0, 0, CursorStyle::default())),
        "fresh emulator: tracked cell + origin"
    );
}

/// The DECSCUSR shape rides `focused_cursor` for the sink to re-emit:
/// the pane's `CSI 4 SP q` (steady underline) is tracked and mapped.
#[test]
fn focused_cursor_carries_the_tracked_decscusr_shape() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.focus(1);
    renderer.feed_output(1, b"\x1b[4 q");
    let (x, _y, style) = renderer.focused_cursor().expect("cursor");
    assert_eq!(x, 0, "pane 1's origin col");
    assert_eq!(style, CursorStyle::SteadyUnderline, "DECSCUSR 4 tracked");
}

/// Fix 2 (focus accent): the focused pane's boundary dividers carry
/// the bright-cyan accent fg (indexed 14) plus bold, clearly
/// distinguishable from the unfocused dividers (dim, default fg) —
/// the shipped bold/dim-only highlight read as identical dividers.
#[test]
fn focused_divider_carries_accent_vs_dim_unfocused() {
    const THREE_PANE: &str = "0000,90x24,0,0{30x24,0,0,1,30x24,30,0,2,30x24,60,0,3}";
    let layout = parse_layout(THREE_PANE).expect("parses");
    let mut renderer = PaneRenderer::new(90, 24, Glyphs::Unicode);
    renderer.apply_layout(layout);
    renderer.focus(1);
    renderer.render_frame();

    let focused_div = &renderer.buffer[(29, 0)];
    assert_eq!(
        focused_div.fg,
        RtColor::Indexed(14),
        "accent fg on the focused divider"
    );
    assert!(focused_div.modifier.contains(RtModifier::BOLD));

    let unfocused_div = &renderer.buffer[(59, 0)];
    assert_eq!(
        unfocused_div.fg,
        RtColor::Reset,
        "no accent on the unfocused divider"
    );
    assert!(unfocused_div.modifier.contains(RtModifier::DIM));
}

/// Fix 3 (background fill): after a frame, NO cell carries the
/// ratatui default (`Reset`) background — every cell the pane grids
/// do not cover (and the skipped cells inside them: short history
/// lines, wide-char spacers) is filled with the renderer's
/// configured background, which `set_background` can point at the
/// host's resolved value.
#[test]
fn frame_fills_every_cell_with_the_configured_background() {
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    let mut renderer = PaneRenderer::new(80, 24, Glyphs::Unicode);
    renderer.set_background(Some(RtColor::Rgb(16, 24, 40)));
    renderer.apply_layout(layout);
    // A short history line leaves the rest of its row unpainted, and
    // a wide char marks a spacer that painting skips.
    renderer.feed_output(1, "世\r\n".as_bytes());
    renderer.render_frame();

    let default_bg_cells = (0..80u16)
        .flat_map(|x| (0..24u16).map(move |y| (x, y)))
        .filter(|(x, y)| renderer.cell(*x, *y).map(|c| c.bg == RtColor::Reset) == Some(true))
        .count();
    assert_eq!(default_bg_cells, 0, "no Reset-style cell survives a frame");

    // The skipped cells specifically: the wide-char spacer (painting
    // skipped it, so the fill's bg stands) and an unpainted tail cell.
    // Painted BLANK cells also carry the probed bg (the round-3
    // contract: the resolved background fills every cell, so a pane
    // area never shows palette black where the theme bg belongs);
    // only cells the core painted with a real color keep it.
    assert_eq!(
        renderer.cell(1, 0).expect("spacer").bg,
        RtColor::Rgb(16, 24, 40)
    );
    assert_eq!(
        renderer.cell(0, 0).expect("painted").bg,
        RtColor::Rgb(16, 24, 40),
        "blank painted cells carry the probed bg"
    );

    // The grey-band case: rows BELOW the tiled layout (a renderer
    // taller than the layout rects) are painted by nothing — they
    // must still carry the fill, not the ratatui default.
    let mut banded = PaneRenderer::new(80, 30, Glyphs::Unicode);
    banded.set_background(Some(RtColor::Rgb(16, 24, 40)));
    banded.apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    banded.render_frame();
    for y in 24..30u16 {
        assert_eq!(
            banded.cell(10, y).expect("band").bg,
            RtColor::Rgb(16, 24, 40),
            "row {y} below the layout carries the fill"
        );
    }

    // Changing the background dirties and repaints with the new fill.
    renderer.set_background(Some(RtColor::Rgb(1, 2, 3)));
    let diff = renderer.render_frame();
    assert!(!diff.is_empty(), "a bg change repaints");
    assert_eq!(
        renderer.cell(1, 0).expect("spacer").bg,
        RtColor::Rgb(1, 2, 3)
    );
}

/// The status row's cells carry the frame background too — the
/// `REVERSED` style the painter uses means fg/bg swap, so the row's
/// recorded bg must be the host bg for the reversed band to read as
/// the host's foreground on the host's background.
#[test]
fn status_row_cells_carry_no_reset_bg_after_paint() {
    // The status painter sets only a REVERSED modifier, leaving bg at
    // the buffer default; the sink's write_styled skips a Reset bg, so
    // the host's own bg shows through the reversed cells' unswapped
    // half. This is the documented v1 behavior; the assertion pins it
    // so a future bg-aware status painter updates both sides.
    let segments = vec![status::Segment {
        text: "x".to_string(),
        bold: false,
        dim: false,
        shaded: false,
    }];
    let mut row = status::StatusRow::new(4);
    row.paint(&segments, None);
    let diff = row.diff();
    assert!(diff.iter().all(|(_, _, cell)| cell.bg == RtColor::Reset));
}

/// The cursor-emitting sink: the recorded place_cursor calls.
struct CursorSink {
    placements: Vec<Option<(u16, u16, CursorStyle)>>,
}

impl FlushSink for CursorSink {
    fn flush(&mut self, _diff: &[(u16, u16, RtCell)]) {}
    fn repaint_all(&mut self) {}
    fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
        self.placements.push(cursor);
    }
}

/// A modal overlay (help, picker) covers the focused pane: the frame
/// hides the host cursor instead of placing the pane's cell through
/// the panel; dismissal restores placement (the manual-pass report —
/// the block cursor drew through the help panel).
#[test]
fn modal_overlay_hides_the_pane_cursor() {
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.renderer.feed_output(1, b"hello\r\n");

    let mut sink = CursorSink {
        placements: Vec::new(),
    };
    session
        .renderer
        .set_overlay(Some(("keybinds", vec![], None)));
    session.frame(&mut sink);
    assert_eq!(
        sink.placements.last(),
        Some(&None),
        "an open modal hides the host cursor: {:?}",
        sink.placements
    );

    session.renderer.set_overlay(None);
    session.frame(&mut sink);
    let expected = session
        .renderer
        .focused_cursor()
        .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
    assert_eq!(
        sink.placements.last(),
        Some(&expected),
        "dismissal restores the pane cursor placement"
    );
}

/// End-to-end through the session's real frame path: a flushed frame
/// always re-places the cursor (the diff's per-cell CUPs moved the
/// host cursor), and a quiet pump emits nothing new. The session is
/// constructed headless — `WindowSession::new` touches no daemon —
/// and its renderer seeded directly.
#[test]
fn frame_replaces_cursor_after_flush_and_quiets_when_idle() {
    let mut session = WindowSession::new(80, 25);
    let layout = parse_layout(TWO_PANE_LAYOUT).expect("parses");
    session.renderer.apply_layout(layout);
    session.renderer.feed_output(1, b"hello\r\n");

    let mut sink = CursorSink {
        placements: Vec::new(),
    };
    // First frame: the flush must place the cursor (repaint_all's
    // hide is simulated by the initial cursor_placed = Some(None)
    // state — a placement must appear regardless, because flushed).
    session.frame(&mut sink);
    let expected = session
        .renderer
        .focused_cursor()
        .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
    assert_eq!(sink.placements.len(), 1, "flushed frame places once");
    assert_eq!(sink.placements[0], expected);

    // A quiet pump: no flush, unchanged cursor state — no emission.
    session.frame(&mut sink);
    assert_eq!(sink.placements.len(), 1, "quiet pump emits nothing");

    // Pane output: a flush — the cursor re-places even though the
    // mapped cell did not move (the diff's CUPs moved the host one).
    // The "x" feed moved the tracked cursor to (1, 1); recompute.
    session.renderer.feed_output(1, b"x");
    let moved = session
        .renderer
        .focused_cursor()
        .map(|(x, y, style)| (x, y + 1, style)); // +1: the strip row
    session.frame(&mut sink);
    assert_eq!(sink.placements.len(), 2, "flush re-places");
    assert_eq!(sink.placements[1], moved);

    // Scroll the focused pane off live: state change to hidden even
    // without a flush. Pane 1 needs scrollback to scroll into first.
    for i in 0..30 {
        session
            .renderer
            .feed_output(1, format!("h{i}\r\n").as_bytes());
    }
    assert!(session.renderer.wheel_scroll(10, 5, 3));
    session.frame(&mut sink);
    assert_eq!(sink.placements.len(), 3, "hide emits");
    assert_eq!(sink.placements[2], None);
}

/// `recording_conn` with per-command canned replies: the fake daemon
/// answers the exact command line with the body, everything else
/// with an ok empty reply.
fn scripted_conn(
    tag: &str,
    replies: std::collections::HashMap<String, String>,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
) {
    fake_daemon(
        tag,
        FakeScript {
            replies,
            ..FakeScript::default()
        },
    )
}

/// The fake daemon's script: canned reply bodies per exact command
/// line, the lines answered with an error block (`ok == false`), and
/// notification lines pushed just BEFORE a line's reply block (the
/// daemon's `%layout-change` riding a size report).
#[derive(Default)]
struct FakeScript {
    replies: std::collections::HashMap<String, String>,
    failing: std::collections::HashSet<String>,
    notify: std::collections::HashMap<String, String>,
}

fn fake_daemon(
    tag: &str,
    script: FakeScript,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
) {
    let FakeScript {
        replies,
        failing,
        notify,
    } = script;
    let mut path = std::env::temp_dir();
    path.push(format!("par-mux-render-{tag}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = crate::mux::bind_local_listener(&path).expect("bind");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        #[cfg(unix)]
        use interprocess::local_socket::traits::Listener as _;
        let Ok(stream) = listener.accept() else {
            return;
        };
        use interprocess::TryClone as _;
        let mut writer = stream.try_clone().expect("clone");
        let mut reader = std::io::BufReader::new(stream);
        let mut number = 0u32;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or("").to_owned();
            let reply = match name.as_str() {
                "version" => "9.9.9+deadbeef".to_string(),
                "list-commands" => "list-commands\nfeatures replay-held-state\n".to_string(),
                _ => replies.get(trimmed).cloned().unwrap_or_default(),
            };
            number += 1;
            tx.send((name, trimmed.to_owned())).ok();
            if let Some(note) = notify.get(trimmed) {
                writer.write_all(format!("{note}\n").as_bytes()).ok();
            }
            let ok = !failing.contains(trimmed);
            writer
                .write_all(crate::mux::emit_block(number, &reply, ok).as_bytes())
                .ok();
            writer.flush().ok();
        }
    });
    let conn = crate::mux::attach::conn::AttachConn::connect(&path).expect("connect");
    (rx, conn)
}

/// A live `AttachConn` over a recording listener: every command rides
/// the wire and gets an ok empty reply, and the recorded (name, line)
/// pairs land on the returned receiver. The resize/swap affordance
/// tests read the exact wire spellings off it.
fn recording_conn(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
) {
    scripted_conn(tag, std::collections::HashMap::new())
}

/// The next recorded line named `name`, bounding the wait.
fn wait_recorded(rx: &std::sync::mpsc::Receiver<(String, String)>, name: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("no {name} line arrived within the bound");
        }
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok((n, line)) if n == name => return line,
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("no {name} line: the fake daemon is gone");
            }
        }
    }
}

/// A sink recording every flushed cell and cursor placement — the
/// frame-path read for the strip and height assertions.
#[derive(Default)]
struct RecordingSink {
    cells: Vec<(u16, u16, RtCell)>,
    placements: Vec<Option<(u16, u16, CursorStyle)>>,
}

impl FlushSink for RecordingSink {
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
        self.cells.extend_from_slice(diff);
    }
    fn repaint_all(&mut self) {}
    fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
        self.placements.push(cursor);
    }
}

/// The overlay's rows joined (the test-side read of the modal's
/// content).
fn overlay_text(session: &WindowSession) -> Vec<String> {
    session
        .renderer
        .overlay
        .as_ref()
        .expect("the overlay is up")
        .1
        .iter()
        .map(|r| r.text.clone())
        .collect()
}

/// Two side-by-side panes (a vertical divider at x=39).
const TWO_PANE: &str = "0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}";

/// The same window re-divided for a 20-col side panel: the 60-col
/// extent split in two 30-col panes.
const TWO_PANE_SIDEBAR: &str = "0000,60x24,0,0{30x24,0,0,1,30x24,30,0,2}";

/// Two stacked panes (a horizontal divider at y=11).
const STACKED: &str = "0000,80x24,0,0{80x12,0,0,1,80x12,0,12,2}";

/// A sink that records flushed cells (the flush-gate test's probe).
struct CellSink {
    cells: Vec<(u16, u16, RtCell)>,
}

impl FlushSink for CellSink {
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
        self.cells.extend(diff.iter().cloned());
    }
    fn repaint_all(&mut self) {}
    fn place_cursor(&mut self, _cursor: Option<(u16, u16, CursorStyle)>) {}
}

/// A panel-up session (strip 20 on an 80x24 host) with the command
/// menu opened through the ` menu ` chip's own cell: the footer row
/// is the 22-row renderer's last row, host row 22 (SGR row 23), the
/// chip spanning cols 13..19 (SGR col 17 is inside it). No workspace
/// roster is loaded — the chip must not depend on one.
fn command_menu_session(
    tag: &str,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    use crate::mux::attach::input::SgrMouse;
    let (rx, mut conn) = recording_conn(tag);
    let mut session = WindowSession::new(80, 24);
    session.renderer.set_sidebar_width(20);
    session
        .renderer
        .set_sidebar_sections(Some(vec![super::super::SidebarSection { rows: vec![] }]));
    session.route_mouse(
        &mut conn,
        SgrMouse {
            cb: 0,
            col: 17,
            row: 23,
            release: false,
        },
    );
    (rx, conn, session)
}

/// Click panel row `r` of the open menu at its raw host cell (panel
/// row r paints at host y0+2+r; SGR is 1-based).
fn click_menu_row(
    session: &mut WindowSession,
    conn: &mut crate::mux::attach::conn::AttachConn,
    r: usize,
) {
    use crate::mux::attach::input::SgrMouse;
    let (x0, y0, _, _) = session.renderer.overlay_geometry().expect("menu up");
    session.route_mouse(
        conn,
        SgrMouse {
            cb: 0,
            col: (x0 + 4) as u16,
            row: (y0 + 3 + r) as u16,
            release: false,
        },
    );
}

/// Every line the fake daemon recorded so far, in wire order. A
/// `send_checked` returns only after its reply, and the fake records
/// a line BEFORE replying, so after an action returns its lines are
/// all queued: the exact-sequence read the wire assertions use.
fn drained(rx: &std::sync::mpsc::Receiver<(String, String)>) -> Vec<String> {
    rx.try_iter().map(|(_, line)| line).collect()
}

/// The canned replies a full re-seed onto `window` (session `$0`,
/// panes `panes`) needs to COMPLETE: the size-report pane lookup, the
/// status refresh's owner scan (`list-sessions` names `$0`, whose
/// windows list `window`), and the window list the strip paints.
/// Without them the reseed still sets `window` but bails at the
/// owner scan, so tests that care about completion use this.
fn reseed_replies(
    replies: &mut std::collections::HashMap<String, String>,
    windows: &str,
    window: &str,
    panes: &str,
) {
    replies.insert("list-sessions".to_string(), "$0: work".to_string());
    replies.insert("list-windows -t $0".to_string(), windows.to_string());
    replies.insert(format!("list-panes -t {window}"), panes.to_string());
}

/// A two-pane session showing `@0` of `$0`, the fake daemon scripted.
fn two_pane_session(
    tag: &str,
    script: FakeScript,
) -> (
    std::sync::mpsc::Receiver<(String, String)>,
    crate::mux::attach::conn::AttachConn,
    WindowSession,
) {
    let (rx, conn) = fake_daemon(tag, script);
    let mut session = WindowSession::new(80, 25);
    session
        .renderer
        .apply_layout(parse_layout(TWO_PANE_LAYOUT).expect("parses"));
    session.window = "@0".to_string();
    session.status.session_id = Some("$0".to_string());
    drained(&rx);
    (rx, conn, session)
}

/// One prefix chord through the plain-byte router (prefix + `key`).
fn chord(
    session: &mut WindowSession,
    conn: &mut crate::mux::attach::conn::AttachConn,
    key: u8,
) -> bool {
    let mut prefix_pending = false;
    session.route_plain(&[crate::mux::attach::C_B, key], conn, &mut prefix_pending)
}

/// Type `text` into the open prompt as plain bytes, then Enter.
fn type_and_commit(
    session: &mut WindowSession,
    conn: &mut crate::mux::attach::conn::AttachConn,
    text: &str,
) {
    session.prompt_text.clear();
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(b'\r');
    let mut prefix_pending = false;
    session.route_plain(&bytes, conn, &mut prefix_pending);
}

/// A 1-based SGR press/release/wheel at host `(col, row)`.
fn sgr(cb: u8, col: u16, row: u16, release: bool) -> SgrMouse {
    SgrMouse {
        cb,
        col,
        row,
        release,
    }
}
