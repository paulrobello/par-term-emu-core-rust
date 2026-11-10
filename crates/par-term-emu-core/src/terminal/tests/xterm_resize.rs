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
