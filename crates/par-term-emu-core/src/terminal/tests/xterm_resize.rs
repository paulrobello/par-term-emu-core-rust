//! Resizing the way xterm.js resizes its buffers (`Terminal::set_xterm_resize`):
//! the cursor, the saved cursor, the alternate screen, the viewport and images around
//! `Grid::resize_xterm`, and any stream followed by any resizes keeping the grid's invariants.

use crate::grid::Grid;
use crate::terminal::Terminal;
use proptest::prelude::*;

fn terminal(cols: usize, rows: usize, scrollback: usize) -> Terminal {
    let mut term = Terminal::with_scrollback(cols, rows, scrollback);
    term.set_compact_scrollback(true);
    term.set_xterm_resize(true);
    term
}

fn screen(grid: &Grid) -> Vec<String> {
    (0..grid.rows())
        .map(|row| grid.row_text(row).trim_end().to_string())
        .collect()
}

#[test]
fn a_pending_wrap_ends_with_the_cursor_past_the_text() {
    let mut term = terminal(10, 3, 100);
    term.process(b"0123456789");
    term.resize(12, 3);
    assert_eq!((term.cursor().col, term.cursor().row), (10, 0));
    term.process(b"X");
    assert_eq!(screen(term.grid())[..2], ["0123456789X", ""]);

    // Narrower, the cursor's line is cut and the cursor clamped into the last column.
    let mut term = terminal(10, 3, 100);
    term.process(b"0123456789");
    term.resize(8, 3);
    assert_eq!((term.cursor().col, term.cursor().row), (7, 0));
    term.process(b"X");
    assert_eq!(screen(term.grid())[0], "0123456X");
}

#[test]
fn a_resize_to_the_same_size_changes_nothing() {
    let mut term = terminal(10, 3, 100);
    term.process(b"0123456789");
    term.resize(10, 3);
    term.process(b"X");
    assert_eq!(screen(term.grid())[..2], ["0123456789", "X"]);
}

#[test]
fn the_saved_cursor_follows_its_line() {
    let mut term = terminal(10, 5, 100);
    term.process(b"0123456789abcde\r\nx\x1b7\r\n$ ");
    assert_eq!(term.cursor().row, 3);
    term.resize(20, 5);
    assert_eq!((term.cursor().col, term.cursor().row), (2, 2));
    term.process(b"\x1b8Z");
    assert_eq!(screen(term.grid()), ["0123456789abcde", "xZ", "$", "", ""]);
}

#[test]
fn both_screens_keep_their_cursor_lines() {
    let mut term = terminal(10, 4, 100);
    term.process(b"0123456789abcdef\r\n$ ");
    term.process(b"\x1b[?1049h\x1b[4;1Hbottom");
    term.resize(20, 2);
    // The alternate screen drops its top rows to keep the cursor's line.
    assert_eq!(screen(term.alt_grid()), ["", "bottom"]);
    assert_eq!((term.cursor().col, term.cursor().row), (6, 1));
    // The primary screen reflowed underneath, its cursor with it.
    term.process(b"\x1b[?1049lls");
    assert_eq!(screen(term.grid()), ["0123456789abcdef", "$ ls"]);
}

#[test]
fn the_viewport_moves_by_xterms_rules() {
    let mut term = terminal(10, 3, 100);
    for line in 0..20 {
        term.process(format!("{line}\r\n").as_bytes());
    }
    assert_eq!(term.grid().scrollback_len(), 18);
    // A scrolled-back viewport moves up a line per row the screen gains.
    assert_eq!(term.resize_with_viewport(10, 5, 5), 3);
    assert_eq!(term.grid().scrollback_len(), 16);
    // A viewport at the bottom stays there.
    let bottom = term.grid().scrollback_len();
    assert_eq!(term.resize_with_viewport(10, 3, bottom), 18);
}

#[test]
fn images_keep_their_lines() {
    let mut term = terminal(10, 6, 100);
    term.process(b"0123456789abc\r\n");
    // A 2x2 kitty image on the third row.
    term.process(b"\x1b_Ga=T,f=32,s=2,v=2;/////////////////////w==\x1b\\");
    assert_eq!(term.all_graphics().len(), 1);
    assert_eq!(term.all_graphics()[0].position.1, 2);
    // Wider, the wrapped line above joins and the image moves up with its line.
    term.resize(20, 6);
    assert_eq!(term.all_graphics()[0].position.1, 1);
    assert_eq!(term.all_graphics()[0].scroll_offset_rows, 0);
}

/// A 2x2 kitty image at the cursor.
fn place_image(term: &mut Terminal) {
    term.process(b"\x1b_Ga=T,f=32,s=2,v=2;/////////////////////w==\x1b\\");
}

/// A primary image and an alternate-screen image, each screen's cursor left
/// on its last row, so an 8→4 row resize moves the two buffers differently:
/// the primary pushes its top four lines into scrollback while the
/// alternate's (scrollback-less) top four lines drop away.
fn terminal_with_images_on_both_buffers() -> Terminal {
    let mut term = terminal(10, 8, 100);
    term.process(b"0123456789abc\r\n");
    // The primary image sits on row 2, below the wrapped pair; the primary
    // cursor moves to the last row.
    place_image(&mut term);
    term.process(b"\x1b[8;1H");
    term.process(b"\x1b[?1049h");
    // On the alternate screen: text and an image on the second-to-last row.
    term.process(b"\x1b[7;1Halt");
    place_image(&mut term);
    term
}

#[test]
fn placements_stamp_their_owning_buffer() {
    let mut term = terminal(10, 8, 100);
    place_image(&mut term);
    assert_eq!(
        term.all_graphics()[0].placement.buffer,
        crate::graphics::PlacementBuffer::Primary
    );
    term.process(b"\x1b[?1049h");
    place_image(&mut term);
    assert_eq!(
        term.all_graphics()[1].placement.buffer,
        crate::graphics::PlacementBuffer::Alternate
    );
    term.process(b"\x1b[?1049l");
    place_image(&mut term);
    assert_eq!(
        term.all_graphics()[2].placement.buffer,
        crate::graphics::PlacementBuffer::Primary
    );
}

#[test]
fn a_resize_while_the_alternate_screen_is_shown_moves_each_image_with_its_own_buffer() {
    let mut term = terminal_with_images_on_both_buffers();
    assert_eq!(term.all_graphics()[0].position.1, 2);
    assert_eq!(term.all_graphics()[1].position.1, 6);
    term.resize(10, 4);
    // The alternate screen dropped its top four lines (its text proves it);
    // its image followed them from row 6 to row 2.
    assert_eq!(screen(term.alt_grid())[2], "alt");
    assert_eq!(term.all_graphics()[1].position.1, 2);
    assert_eq!(term.all_graphics()[1].scroll_offset_rows, 0);
    // The primary screen pushed its top four lines into scrollback; its
    // image's top edge went with them even though the alternate screen is
    // shown (rows 0-1 held the wrapped pair).
    assert_eq!(term.grid().scrollback_len(), 4);
    assert_eq!(term.all_graphics()[0].position.1, 0);
    assert_eq!(term.all_graphics()[0].scroll_offset_rows, 2);
}

#[test]
fn a_resize_while_the_primary_screen_is_shown_moves_each_image_with_its_own_buffer() {
    let mut term = terminal_with_images_on_both_buffers();
    term.process(b"\x1b[?1049l");
    term.resize(10, 4);
    // The alternate screen dropped its top four lines; its image followed.
    assert_eq!(screen(term.alt_grid())[2], "alt");
    assert_eq!(term.all_graphics()[1].position.1, 2);
    // The primary screen pushed its top four lines into scrollback; its
    // image's top edge went with them.
    assert_eq!(term.grid().scrollback_len(), 4);
    assert_eq!(term.all_graphics()[0].position.1, 0);
    assert_eq!(term.all_graphics()[0].scroll_offset_rows, 2);
}

#[test]
fn a_width_change_moves_only_the_primary_images_while_the_alternate_screen_is_shown() {
    let mut term = terminal_with_images_on_both_buffers();
    term.resize(20, 8);
    // The primary screen rewraps: the wrapped pair above its image joins and
    // the image's line follows onto the joined pair.
    assert_eq!(screen(term.grid())[0], "0123456789abc");
    assert_eq!(term.all_graphics()[0].position.1, 1);
    // The alternate screen never rewraps: its lines stay put, its image
    // with them.
    assert_eq!(screen(term.alt_grid())[6], "alt");
    assert_eq!(term.all_graphics()[1].position.1, 6);
}

#[test]
fn a_legacy_resize_never_moves_placements() {
    let mut term = Terminal::with_scrollback(10, 8, 100);
    term.set_compact_scrollback(true);
    // The default resize mode never reflows, so neither buffer's lines move
    // and neither image does either.
    term.process(b"0123456789abc\r\n");
    place_image(&mut term);
    term.process(b"\x1b[?1049h");
    term.process(b"\x1b[6;1Halt");
    place_image(&mut term);
    term.process(b"\x1b[?1049l");
    term.resize(10, 4);
    assert_eq!(term.all_graphics()[0].position.1, 2);
    assert_eq!(term.all_graphics()[0].scroll_offset_rows, 0);
    assert_eq!(term.all_graphics()[1].position.1, 5);
    assert_eq!(term.all_graphics()[1].scroll_offset_rows, 0);
}

#[test]
fn the_switch_survives_a_reset() {
    let mut term = terminal(10, 3, 100);
    term.process(b"\x1bc0123456789abc\r\n$ ");
    term.resize(20, 3);
    assert_eq!(screen(term.grid())[..2], ["0123456789abc", "$"]);
}

/// Pieces of output that wrap, scroll, move the cursor and switch screens.
fn chunk() -> impl Strategy<Value = Vec<u8>> {
    let fixed: Vec<&'static [u8]> = vec![
        b"\r\n",
        b"\n",
        b"\r",
        b"\x1b[H",
        b"\x1b[5;3H",
        b"\x1b[2J",
        b"\x1b[K",
        b"\x1b7",
        b"\x1b8",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[41m  \x1b[K\x1b[0m",
        "\u{6f22}\u{8a9e}".as_bytes(),
        "e\u{301}".as_bytes(),
    ];
    prop_oneof![
        proptest::sample::select(fixed).prop_map(|bytes| bytes.to_vec()),
        "[a-z0-9 ]{1,40}".prop_map(String::into_bytes),
    ]
}

/// What the rest of the engine relies on after a resize.
fn assert_invariants(term: &Terminal) {
    let (cols, rows) = term.size();
    for grid in [term.grid(), term.alt_grid()] {
        assert_eq!((grid.cols(), grid.rows()), (cols, rows));
        for row in 0..rows {
            assert_eq!(grid.row(row).unwrap().len(), cols);
        }
        assert!(grid.blank_past_written_part());
        assert!(grid.scrollback_len() <= grid.max_scrollback());
        for index in 0..grid.scrollback_len() {
            let line = grid.scrollback_line(index).unwrap();
            assert!(
                line.len() <= cols,
                "scrollback line {index} is wider than the screen"
            );
            if grid.is_scrollback_wrapped(index) {
                assert_eq!(line.len(), cols, "wrapped scrollback line {index}");
            }
        }
    }
    assert!(term.cursor().col < cols && term.cursor().row < rows);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn any_stream_and_resizes_keep_the_grid_whole(
        steps in proptest::collection::vec(
            (proptest::collection::vec(chunk(), 1..12), 2usize..30, 1usize..12),
            1..8,
        ),
    ) {
        let mut term = terminal(16, 6, 20);
        for (chunks, cols, rows) in steps {
            for chunk in chunks {
                term.process(&chunk);
            }
            term.resize(cols, rows);
            assert_invariants(&term);
            term.process(b"x\r\n");
        }
    }
}
