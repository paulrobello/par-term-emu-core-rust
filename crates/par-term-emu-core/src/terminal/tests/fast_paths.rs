//! The fast paths for output floods against the general ones.
//!
//! Printable ASCII is written a row segment at a time
//! (`Terminal::write_ascii_run`), screen rows scroll by pointer, and compact
//! scrollback stores lines without their trailing blank cells. For any byte
//! stream, in any chunking, the terminal must end up exactly where the
//! per-character path leaves it, and compact scrollback must read exactly
//! like full-width lines padded with blank cells.

use crate::cell::Cell;
use crate::grid::Grid;
use crate::terminal::write::PER_CHAR_PRINT;
use crate::terminal::Terminal;
use proptest::prelude::*;

const COLS: usize = 16;
const ROWS: usize = 6;
const SCROLLBACK: usize = 12;

/// One piece of output: text, controls, and the sequences that change how
/// text lands (margins, modes, charsets, attributes, scrolling, editing).
fn chunk() -> impl Strategy<Value = Vec<u8>> {
    let fixed: Vec<&'static [u8]> = vec![
        b"\r",
        b"\n",
        b"\r\n",
        b"\t",
        b"\x08",
        "\u{6f22}".as_bytes(),
        "\u{1f680}".as_bytes(),
        "e\u{301}".as_bytes(),
        "\u{1f1fa}\u{1f1f8}".as_bytes(),
        b"\x1b[1;31m",
        b"\x1b[0m",
        b"\x1b[4:3m",
        b"\x1b[38;2;1;2;3m",
        b"\x1b[7m",
        b"\x1b[r",
        b"\x1bM",
        b"\x1bD",
        b"\x1bE",
        b"\x1b[?7l",
        b"\x1b[?7h",
        b"\x1b[4h",
        b"\x1b[4l",
        b"\x1b[?69h",
        b"\x1b[3;12s",
        b"\x1b[?69l",
        b"\x1b(0",
        b"\x1b(B",
        b"\x0e",
        b"\x0f",
        b"\x1b]8;;http://example.com\x1b\\",
        b"\x1b]8;;\x1b\\",
        b"\x1b[1\"q",
        b"\x1b[0\"q",
        b"\x1b[2J",
        b"\x1b[K",
        b"\x1b[1K",
        b"\x1b[3X",
        b"\x1b[2@",
        b"\x1b[2P",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[6n",
        b"\x1b[3g",
        b"\x1bH",
    ];
    prop_oneof![
        4 => "[ -~]{1,40}".prop_map(String::into_bytes),
        3 => proptest::sample::select(fixed).prop_map(<[u8]>::to_vec),
        1 => (1usize..=ROWS + 2, 1usize..=COLS + 2)
            .prop_map(|(r, c)| format!("\x1b[{r};{c}H").into_bytes()),
        1 => (1usize..=ROWS, 1usize..=ROWS)
            .prop_map(|(t, b)| format!("\x1b[{t};{b}r").into_bytes()),
        1 => (0usize..4, proptest::sample::select(vec!['L', 'M', 'S', 'T']))
            .prop_map(|(n, f)| format!("\x1b[{n}{f}").into_bytes()),
    ]
}

fn stream() -> impl Strategy<Value = (Vec<u8>, Vec<usize>)> {
    (
        proptest::collection::vec(chunk(), 1..60).prop_map(|chunks| chunks.concat()),
        proptest::collection::vec(1usize..64, 0..12),
    )
}

fn terminal(compact: bool) -> Terminal {
    let mut term = Terminal::with_scrollback(COLS, ROWS, SCROLLBACK);
    term.set_compact_scrollback(compact);
    term
}

/// Feed `data` in pieces of the given sizes (the rest in one piece).
fn feed(term: &mut Terminal, data: &[u8], cuts: &[usize]) {
    let mut rest = data;
    for &cut in cuts {
        let (head, tail) = rest.split_at(cut.min(rest.len()));
        term.process(head);
        rest = tail;
    }
    term.process(rest);
}

/// The per-character path over the same pieces. Both terminals get the
/// same chunking so that only the print path differs: vte 0.15.0 itself is
/// not chunking-invariant (a UTF-8 sequence split across two `advance`
/// calls can drop the character after it, alacritty/vte#159).
fn per_char(data: &[u8], cuts: &[usize], compact: bool) -> Terminal {
    PER_CHAR_PRINT.with(|flag| flag.set(true));
    let mut term = terminal(compact);
    feed(&mut term, data, cuts);
    PER_CHAR_PRINT.with(|flag| flag.set(false));
    term
}

fn padded(line: &[Cell], cols: usize) -> Vec<Cell> {
    let mut line = line.to_vec();
    if line.len() < cols {
        line.resize(cols, Cell::default());
    }
    line
}

/// Screen rows, their wrap flags, scrollback lines, their wrap flags, and
/// the lines scrolled so far.
type GridView = (Vec<Vec<Cell>>, Vec<bool>, Vec<Vec<Cell>>, Vec<bool>, usize);

/// Everything a reader can see of a grid, scrollback lines padded to the
/// screen width.
fn grid_view(grid: &Grid) -> GridView {
    let cols = grid.cols();
    (
        (0..grid.rows())
            .map(|r| grid.row(r).unwrap().to_vec())
            .collect(),
        (0..grid.rows()).map(|r| grid.is_line_wrapped(r)).collect(),
        (0..grid.scrollback_len())
            .map(|i| padded(grid.scrollback_line(i).unwrap(), cols))
            .collect(),
        (0..grid.scrollback_len())
            .map(|i| grid.is_scrollback_wrapped(i))
            .collect(),
        grid.total_lines_scrolled(),
    )
}

fn assert_same(a: &mut Terminal, b: &mut Terminal) {
    assert_eq!(grid_view(a.grid()), grid_view(b.grid()), "primary grid");
    assert_eq!(
        grid_view(a.alt_grid()),
        grid_view(b.alt_grid()),
        "alternate grid"
    );
    assert_eq!(a.is_alt_screen_active(), b.is_alt_screen_active());
    assert_eq!(
        (a.cursor().col, a.cursor().row),
        (b.cursor().col, b.cursor().row),
        "cursor"
    );
    assert_eq!(a.pending_wrap, b.pending_wrap, "pending wrap");
    assert_eq!(a.drain_responses(), b.drain_responses(), "replies");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Row-segment writes, in any chunking, match one character at a time
    /// over the same pieces.
    #[test]
    fn ascii_runs_match_the_per_character_path((data, cuts) in stream()) {
        let mut expected = per_char(&data, &cuts, false);
        let mut fast = terminal(false);
        feed(&mut fast, &data, &cuts);
        assert_same(&mut fast, &mut expected);
    }

    /// Cells past a row's written part are blank whatever wrote, erased,
    /// scrolled or resized the screen.
    #[test]
    fn cells_past_the_written_part_are_blank(
        (data, cuts) in stream(),
        (cols, rows) in (2usize..24, 1usize..9),
    ) {
        let mut term = terminal(true);
        feed(&mut term, &data, &cuts);
        prop_assert!(term.grid().blank_past_written_part());
        prop_assert!(term.alt_grid().blank_past_written_part());
        term.resize(cols, rows);
        feed(&mut term, &data, &[]);
        prop_assert!(term.grid().blank_past_written_part());
        prop_assert!(term.alt_grid().blank_past_written_part());
    }

    /// Compact scrollback reads like full-width lines padded with blanks,
    /// and the screen is untouched by it.
    #[test]
    fn compact_scrollback_reads_like_full_width_lines((data, cuts) in stream()) {
        let mut full = terminal(false);
        feed(&mut full, &data, &cuts);
        let mut compact = terminal(true);
        feed(&mut compact, &data, &cuts);
        assert_same(&mut compact, &mut full);
        for i in 0..compact.grid().scrollback_len() {
            let line = compact.grid().scrollback_line(i).unwrap();
            if compact.grid().is_scrollback_wrapped(i) {
                prop_assert_eq!(line.len(), COLS, "wrapped lines keep their width");
            } else {
                prop_assert!(line.last().is_none_or(|cell| *cell != Cell::default()));
            }
        }
    }
}

#[test]
fn scrolled_lines_keep_their_wrap_flag_inside_a_region() {
    // A wrapped line moved by a region scroll still continues on the line
    // below it: the flag moves with the row.
    let mut term = terminal(false);
    term.process(b"\x1b[2;5r\x1b[3;1H");
    term.process(&[b'x'; COLS + 3]);
    assert!(term.grid().is_line_wrapped(2));
    assert!(!term.grid().is_line_wrapped(3));
    // A line feed at the bottom of the region scrolls it up by one.
    term.process(b"\x1b[5;1H\n");
    assert_eq!(term.grid().row_text(1), "x".repeat(COLS));
    assert!(term.grid().is_line_wrapped(1));
    assert_eq!(term.grid().row_text(2).trim_end(), "xxx");
    assert!(!term.grid().is_line_wrapped(2));
}

#[test]
fn compact_scrollback_stores_only_the_written_cells() {
    let mut term = terminal(true);
    term.process(b"abc\r\n\r\nxy  \x1b[41m \x1b[0m\r\n");
    for _ in 0..ROWS {
        term.process(b"\r\n");
    }
    let grid = term.grid();
    assert_eq!(grid.scrollback_line(0).unwrap().len(), 3);
    assert_eq!(grid.scrollback_line(1).unwrap().len(), 0);
    // Written spaces with the default pen equal blank cells; the one with a
    // background color is content.
    assert_eq!(grid.scrollback_line(2).unwrap().len(), 5);
    assert!(term.compact_scrollback());
    // RIS keeps the setting.
    term.process(b"\x1bc");
    assert!(term.compact_scrollback());
}

#[test]
fn full_screen_damage_is_reported_for_every_row() {
    let mut term = terminal(false);
    term.process(b"one\r\ntwo");
    let since = term.damage_generation();
    term.process(b"\x1b[6;1H\n");
    let mut ranges = Vec::new();
    term.active_grid()
        .for_each_damage_range_since(since, |start, end| ranges.push((start, end)));
    assert_eq!(ranges, vec![(0, ROWS as u32 - 1)]);
    assert_eq!(
        term.active_grid().damage_indices(since).collect::<Vec<_>>(),
        (0..ROWS).collect::<Vec<_>>()
    );
    // A later single-row change reports that row only.
    let since = term.damage_generation();
    term.process(b"z");
    assert_eq!(
        term.active_grid().damage_indices(since).collect::<Vec<_>>(),
        vec![ROWS - 1]
    );
}
