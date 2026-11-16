//! The tests xterm.js has for its buffer resize, ported (`Buffer.test.ts` "resize" and
//! "buffer marked to have no scrollback", `BufferReflow.test.ts`; MIT, Copyright (c) 2017-2019
//! The xterm.js authors), then the grid and terminal around the port. Left out: the Windows
//! ConPTY cases and `reflowCursorLine` (neither is ported).

// The resize and reflow tests in this file are ported from xterm.js
// (https://github.com/xtermjs/xterm.js), which is under the MIT License:
//
// Copyright (c) 2017-2019, The xterm.js authors (https://github.com/xtermjs/xterm.js)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use super::*;
use crate::grid::Grid;

const INIT_COLS: usize = 80;
const INIT_ROWS: usize = 24;
const INIT_SCROLLBACK: usize = 1000;

fn buffer_with(scrollback: usize, has_scrollback: bool) -> XtermBuffer {
    XtermBuffer::new(
        Vec::new(),
        INIT_COLS,
        INIT_ROWS,
        scrollback,
        has_scrollback,
        0,
    )
}

fn buffer() -> XtermBuffer {
    buffer_with(INIT_SCROLLBACK, true)
}

/// `fillViewportRows`. Blank lines are as wide as the test's buffer service says (80).
fn fill_viewport_rows(buffer: &mut XtermBuffer) {
    if buffer.lines.is_empty() {
        for _ in 0..buffer.rows {
            buffer.lines.push(Line::new(INIT_COLS, false));
        }
    }
}

/// A one-cell character, whatever its usual width (the tests write '😁' as one cell).
fn ch(c: char) -> Cell {
    let mut cell = Cell::new(c);
    cell.width = 1;
    cell.flags.set_wide_char(false);
    cell
}

fn wide(c: char) -> Cell {
    let mut cell = Cell::new(c);
    cell.width = 2;
    cell.flags.set_wide_char(true);
    cell
}

/// The right half of a wide character: `[0, '', 0, 0]`.
fn spacer() -> Cell {
    let mut cell = Cell::default();
    cell.flags.set_wide_char_spacer(true);
    cell
}

fn set_text(line: &mut Line, text: &str) {
    for (index, c) in text.chars().enumerate() {
        line.set_cell(index, ch(c));
    }
}

/// `translateToString(trimRight)`.
fn text_of(line: &Line, trim_right: bool) -> String {
    let end = if trim_right {
        line.len.min(line.trimmed_len())
    } else {
        line.len
    };
    let mut text = String::new();
    let mut index = 0;
    while index < end {
        match line.cells.get(index) {
            Some(cell) if has_content(cell) => cell.push_grapheme(&mut text),
            _ => text.push(' '),
        }
        index += line.width(index).max(1);
    }
    text
}

fn line(buffer: &XtermBuffer, index: usize) -> String {
    text_of(&buffer.lines[index], false)
}

fn trimmed(buffer: &XtermBuffer, index: usize) -> String {
    text_of(&buffer.lines[index], true)
}

/// Where the line first at `origin` is now (xterm.js's markers), `None` once it is gone.
fn marker(buffer: &XtermBuffer, origin: usize) -> Option<usize> {
    buffer
        .lines
        .iter()
        .position(|line| line.origin == Some(origin))
}

fn assert_wrapped(buffer: &XtermBuffer, wrapped: &[usize]) {
    for (index, line) in buffer.lines.iter().enumerate() {
        assert_eq!(
            line.wrapped,
            wrapped.contains(&index),
            "line {index} isWrapped must equal {}",
            wrapped.contains(&index)
        );
    }
}

// ---- Buffer.test.ts: resize ------------------------------------------------------------------

#[test]
fn column_size_reduced_trims_the_data_in_the_buffer() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(INIT_COLS / 2, INIT_ROWS);
    assert_eq!(buffer.lines.len(), INIT_ROWS);
    for line in &buffer.lines {
        assert_eq!(line.len, INIT_COLS / 2);
    }
}

#[test]
fn column_size_increased_adds_pad_columns() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(INIT_COLS + 10, INIT_ROWS);
    assert_eq!(buffer.lines.len(), INIT_ROWS);
    for line in &buffer.lines {
        assert_eq!(line.len, INIT_COLS + 10);
    }
}

#[test]
fn row_size_reduced_trims_blank_lines_from_the_end() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(INIT_COLS, INIT_ROWS - 10);
    assert_eq!(buffer.lines.len(), INIT_ROWS - 10);
}

#[test]
fn row_size_reduced_moves_the_viewport_down_when_it_is_at_the_end() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    // The cursor has 5 blank lines below it.
    buffer.y = INIT_ROWS - 5 - 1;
    buffer.resize(INIT_COLS, INIT_ROWS - 10);
    // 5 rows trimmed.
    assert_eq!(buffer.lines.len(), INIT_ROWS - 5);
    // The viewport shifted down 5 rows.
    assert_eq!(buffer.ydisp, 5);
    assert_eq!(buffer.ybase, 5);
}

#[test]
fn row_size_reduced_without_scrollback_trims_the_top_when_the_cursor_is_at_the_bottom() {
    let mut buffer = buffer_with(0, true);
    assert_eq!(buffer.max_length, INIT_ROWS);
    buffer.y = INIT_ROWS - 1;
    fill_viewport_rows(&mut buffer);
    buffer.lines[5].set_cell(0, ch('a'));
    buffer.lines[INIT_ROWS - 1].set_cell(0, ch('b'));
    buffer.resize(INIT_COLS, INIT_ROWS - 5);
    assert_eq!(buffer.lines[0].cells[0].c, 'a');
    assert_eq!(buffer.lines[INIT_ROWS - 1 - 5].cells[0].c, 'b');
}

#[test]
fn row_size_increased_on_an_empty_buffer_adds_blank_lines_to_the_end() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    assert_eq!(buffer.ydisp, 0);
    buffer.resize(INIT_COLS, INIT_ROWS + 10);
    assert_eq!(buffer.ydisp, 0);
    assert_eq!(buffer.lines.len(), INIT_ROWS + 10);
}

#[test]
fn row_size_increased_on_a_filled_buffer_shows_more_of_the_buffer_above() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    // 10 extra blank lines.
    for _ in 0..10 {
        buffer.lines.push(Line::new(INIT_COLS, false));
    }
    // The cursor at the bottom of the buffer, scrolled down 10 lines.
    buffer.y = INIT_ROWS - 1;
    buffer.ybase = 10;
    buffer.ydisp = 10;
    assert_eq!(buffer.lines.len(), INIT_ROWS + 10);
    buffer.resize(INIT_COLS, INIT_ROWS + 5);
    // 5 more lines show.
    assert_eq!(buffer.ydisp, 5);
    assert_eq!(buffer.ybase, 5);
    // The buffer is not trimmed.
    assert_eq!(buffer.lines.len(), INIT_ROWS + 10);
}

#[test]
fn row_size_increased_shows_more_below_when_the_viewport_is_at_the_top_of_the_buffer() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    for _ in 0..10 {
        buffer.lines.push(Line::new(INIT_COLS, false));
    }
    buffer.y = INIT_ROWS - 1;
    buffer.ybase = 10;
    buffer.ydisp = 0;
    assert_eq!(buffer.lines.len(), INIT_ROWS + 10);
    buffer.resize(INIT_COLS, INIT_ROWS + 5);
    // The viewport stays at the top; the screen moves up 5 lines.
    assert_eq!(buffer.ydisp, 0);
    assert_eq!(buffer.ybase, 5);
    assert_eq!(buffer.lines.len(), INIT_ROWS + 10);
}

#[test]
fn row_and_column_increased_resizes_properly() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(INIT_COLS + 5, INIT_ROWS + 5);
    assert_eq!(buffer.lines.len(), INIT_ROWS + 5);
    for line in &buffer.lines {
        assert_eq!(line.len, INIT_COLS + 5);
    }
}

#[test]
fn reflow_does_not_wrap_empty_lines() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    assert_eq!(buffer.lines.len(), INIT_ROWS);
    buffer.resize(INIT_COLS - 5, INIT_ROWS);
    assert_eq!(buffer.lines.len(), INIT_ROWS);
}

#[test]
fn reflow_shrinks_row_length() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(5, 10);
    assert_eq!(buffer.lines.len(), 10);
    for line in &buffer.lines {
        assert_eq!(line.len, 5);
    }
}

#[test]
fn reflow_wraps_and_unwraps_lines() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(5, 10);
    set_text(&mut buffer.lines[0], "abcde");
    buffer.y = 1;
    assert_eq!(buffer.lines[0].len, 5);
    assert_eq!(line(&buffer, 0), "abcde");
    buffer.resize(1, 10);
    assert_eq!(buffer.lines.len(), 10);
    for (index, expected) in ["a", "b", "c", "d", "e", " ", " ", " ", " ", " "]
        .iter()
        .enumerate()
    {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    buffer.resize(5, 10);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(line(&buffer, 0), "abcde");
    for index in 1..10 {
        assert_eq!(line(&buffer, index), "     ", "line {index}");
    }
}

#[test]
fn reflow_leaves_the_wrapped_line_holding_the_cursor_alone() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(5, 10);
    set_text(&mut buffer.lines[0], "abcde");
    buffer.resize(1, 10);
    buffer.y = 2;
    buffer.resize(5, 10);
    assert_ne!(line(&buffer, 0), "abcde");
}

#[test]
fn reflow_discards_parts_of_wrapped_lines_that_go_out_of_the_scrollback() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.scrollback = 1;
    buffer.resize(10, 5);
    set_text(&mut buffer.lines[3], "abcdefghij");
    assert_eq!(buffer.lines.len(), 5);
    buffer.y = 4;
    buffer.resize(2, 5);
    assert_eq!(buffer.y, 4);
    assert_eq!(buffer.ybase, 1);
    assert_eq!(buffer.lines.len(), 6);
    for (index, expected) in ["ab", "cd", "ef", "gh", "ij", "  "].iter().enumerate() {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    buffer.resize(1, 5);
    assert_eq!(buffer.y, 4);
    assert_eq!(buffer.ybase, 1);
    assert_eq!(buffer.lines.len(), 6);
    for (index, expected) in ["f", "g", "h", "i", "j", " "].iter().enumerate() {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    buffer.resize(10, 5);
    assert_eq!(buffer.y, 1);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 5);
    assert_eq!(line(&buffer, 0), "fghij     ");
    for index in 1..5 {
        assert_eq!(line(&buffer, index), "          ", "line {index}");
    }
}

#[test]
fn reflow_removes_the_correct_amount_of_rows_when_reflowing_larger() {
    // Successive wrapped lines that lose 3+ rows on a reflow remove the right rows.
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(10, 10);
    buffer.y = 2;
    set_text(&mut buffer.lines[0], "abcdefghij");
    set_text(&mut buffer.lines[1], "0123456789");
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(line(&buffer, 0), "abcdefghij");
    assert_eq!(line(&buffer, 1), "0123456789");
    for index in 2..10 {
        assert_eq!(line(&buffer, index), "          ");
    }
    buffer.resize(2, 10);
    assert_eq!(buffer.ybase, 1);
    assert_eq!(buffer.lines.len(), 11);
    for (index, expected) in [
        "ab", "cd", "ef", "gh", "ij", "01", "23", "45", "67", "89", "  ",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    buffer.resize(10, 10);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(line(&buffer, 0), "abcdefghij");
    assert_eq!(line(&buffer, 1), "0123456789");
    for index in 2..10 {
        assert_eq!(line(&buffer, index), "          ");
    }
}

#[test]
fn reflow_transfers_combined_char_data_over_to_reflowed_lines() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(4, 3);
    buffer.y = 2;
    set_text(&mut buffer.lines[0], "abc😁");
    assert_eq!(buffer.lines.len(), 3);
    assert_eq!(line(&buffer, 0), "abc😁");
    assert_eq!(line(&buffer, 1), "    ");
    buffer.resize(2, 3);
    assert_eq!(line(&buffer, 0), "ab");
    assert_eq!(line(&buffer, 1), "c😁");
}

fn three_marked_lines(buffer: &mut XtermBuffer) {
    set_text(&mut buffer.lines[0], "abcdefghij");
    set_text(&mut buffer.lines[1], "0123456789");
    set_text(&mut buffer.lines[2], "klmnopqrst");
    for (index, line) in buffer.lines.iter_mut().enumerate() {
        line.origin = Some(index);
    }
}

#[test]
fn reflow_adjusts_markers() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(10, 16);
    three_marked_lines(&mut buffer);
    buffer.y = 3;
    assert_eq!(line(&buffer, 0), "abcdefghij");
    assert_eq!(line(&buffer, 1), "0123456789");
    assert_eq!(line(&buffer, 2), "klmnopqrst");
    assert_eq!(
        [marker(&buffer, 0), marker(&buffer, 1), marker(&buffer, 2)],
        [Some(0), Some(1), Some(2)]
    );
    buffer.resize(2, 16);
    for (index, expected) in [
        "ab", "cd", "ef", "gh", "ij", "01", "23", "45", "67", "89", "kl", "mn", "op", "qr", "st",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    assert_eq!(marker(&buffer, 0), Some(0), "the first marker stays");
    assert_eq!(
        marker(&buffer, 1),
        Some(5),
        "the second marker moves as the first line wrapped"
    );
    assert_eq!(
        marker(&buffer, 2),
        Some(10),
        "the third marker moves as the first and second lines wrapped"
    );
    buffer.resize(10, 16);
    assert_eq!(line(&buffer, 0), "abcdefghij");
    assert_eq!(line(&buffer, 1), "0123456789");
    assert_eq!(line(&buffer, 2), "klmnopqrst");
    assert_eq!(
        [marker(&buffer, 0), marker(&buffer, 1), marker(&buffer, 2)],
        [Some(0), Some(1), Some(2)],
        "the markers are back on their lines"
    );
}

#[test]
fn reflow_disposes_markers_whose_rows_are_trimmed() {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.scrollback = 1;
    buffer.resize(10, 11);
    three_marked_lines(&mut buffer);
    buffer.y = 3;
    buffer.resize(2, 11);
    for (index, expected) in [
        "ij", "01", "23", "45", "67", "89", "kl", "mn", "op", "qr", "st",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(line(&buffer, index), *expected, "line {index}");
    }
    assert_eq!(
        marker(&buffer, 1),
        Some(1),
        "the second marker stays: shifted 4 and trimmed 4"
    );
    assert_eq!(
        marker(&buffer, 2),
        Some(6),
        "the third marker moves as the first and second lines wrapped"
    );
    assert_eq!(marker(&buffer, 0), None, "the first marker was trimmed");
    buffer.resize(10, 11);
    assert_eq!(line(&buffer, 0), "ij        ");
    assert_eq!(line(&buffer, 1), "0123456789");
    assert_eq!(line(&buffer, 2), "klmnopqrst");
    assert_eq!(marker(&buffer, 1), Some(1), "the second marker is back");
    assert_eq!(marker(&buffer, 2), Some(2), "the third marker is back");
}

/// "ab  " (wrapped) and "cd" at four columns: a row ending in blank space (a tab).
fn tab_wrapped(buffer: &mut XtermBuffer) {
    fill_viewport_rows(buffer);
    buffer.resize(4, 10);
    buffer.y = 2;
    set_text(&mut buffer.lines[0], "ab");
    set_text(&mut buffer.lines[1], "cd");
    buffer.lines[1].wrapped = true;
}

#[test]
fn reflow_larger_keeps_the_blank_end_of_a_wrapped_row() {
    let mut buffer = buffer();
    tab_wrapped(&mut buffer);
    buffer.resize(5, 10);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(trimmed(&buffer, 0), "ab  c");
    assert_eq!(line(&buffer, 1), "d    ");
    buffer.resize(6, 10);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(trimmed(&buffer, 0), "ab  cd");
    assert_eq!(line(&buffer, 1), "      ");
}

#[test]
fn reflow_smaller_keeps_the_blank_end_of_a_wrapped_row() {
    let mut buffer = buffer();
    tab_wrapped(&mut buffer);
    buffer.resize(3, 10);
    assert_eq!(buffer.y, 2);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(line(&buffer, 0), "ab ");
    assert_eq!(line(&buffer, 1), " cd");
    buffer.resize(2, 10);
    assert_eq!(buffer.y, 3);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(line(&buffer, 0), "ab");
    assert_eq!(line(&buffer, 1), "  ");
    assert_eq!(line(&buffer, 2), "cd");
}

/// 汉语汉语汉语 (wrapped) and 汉语汉语汉语 at twelve columns.
fn wide_wrapped(buffer: &mut XtermBuffer) {
    fill_viewport_rows(buffer);
    buffer.resize(12, 10);
    for line in buffer.lines.iter_mut().take(2) {
        for index in (0..12).step_by(4) {
            line.set_cell(index, wide('汉'));
            line.set_cell(index + 2, wide('语'));
        }
        for index in (1..12).step_by(2) {
            line.set_cell(index, spacer());
        }
    }
    buffer.lines[1].wrapped = true;
}

#[test]
fn reflow_larger_wraps_wide_characters_correctly() {
    let mut buffer = buffer();
    wide_wrapped(&mut buffer);
    buffer.y = 2;
    assert_eq!(trimmed(&buffer, 0), "汉语汉语汉语");
    assert_eq!(trimmed(&buffer, 1), "汉语汉语汉语");
    buffer.resize(13, 10);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    assert_eq!(trimmed(&buffer, 0), "汉语汉语汉语");
    assert_eq!(line(&buffer, 0), "汉语汉语汉语 ");
    assert_eq!(trimmed(&buffer, 1), "汉语汉语汉语");
    assert_eq!(line(&buffer, 1), "汉语汉语汉语 ");
    buffer.resize(14, 10);
    assert_eq!(trimmed(&buffer, 0), "汉语汉语汉语汉");
    assert_eq!(line(&buffer, 0), "汉语汉语汉语汉");
    assert_eq!(trimmed(&buffer, 1), "语汉语汉语");
    assert_eq!(line(&buffer, 1), "语汉语汉语    ");
}

#[test]
fn reflow_larger_after_a_wide_character_was_cut_from_the_cursor_line() {
    let mut buffer = buffer();
    wide_wrapped(&mut buffer);
    buffer.y = 1;
    // The cursor's line is not reflowed, so shrinking cuts the last wide character of each row.
    buffer.resize(11, 10);
    assert_eq!(line(&buffer, 0), "汉语汉语汉 ");
    assert_eq!(line(&buffer, 1), "汉语汉语汉 ");
    assert_eq!(buffer.lines[1].trimmed_len(), 10);
    // The cursor moves off the wrapped line, then the terminal grows by one column.
    buffer.y = 2;
    buffer.resize(12, 10);
    assert_eq!(trimmed(&buffer, 0), "汉语汉语汉汉");
    assert_eq!(trimmed(&buffer, 1), "语汉语汉");
    assert!(buffer.lines[1].wrapped);
    assert!(!buffer.lines[2].wrapped);
}

#[test]
fn reflow_smaller_wraps_wide_characters_correctly() {
    let mut buffer = buffer();
    wide_wrapped(&mut buffer);
    buffer.y = 2;
    buffer.resize(11, 10);
    assert_eq!(buffer.ybase, 0);
    assert_eq!(buffer.lines.len(), 10);
    let rows = |buffer: &XtermBuffer, count: usize| -> Vec<String> {
        (0..count).map(|index| trimmed(buffer, index)).collect()
    };
    assert_eq!(rows(&buffer, 3), ["汉语汉语汉", "语汉语汉语", "汉语"]);
    buffer.resize(10, 10);
    assert_eq!(rows(&buffer, 3), ["汉语汉语汉", "语汉语汉语", "汉语"]);
    buffer.resize(9, 10);
    assert_eq!(rows(&buffer, 3), ["汉语汉语", "汉语汉语", "汉语汉语"]);
    buffer.resize(8, 10);
    assert_eq!(rows(&buffer, 3), ["汉语汉语", "汉语汉语", "汉语汉语"]);
    buffer.resize(7, 10);
    assert_eq!(rows(&buffer, 4), ["汉语汉", "语汉语", "汉语汉", "语汉语"]);
    buffer.resize(6, 10);
    assert_eq!(rows(&buffer, 4), ["汉语汉", "语汉语", "汉语汉", "语汉语"]);
}

/// 'ab', 'cd' (wrapped), 'ef', 'gh' (wrapped), 'ij', 'kl' (wrapped), then four blank rows, at
/// two columns.
fn larger_case() -> XtermBuffer {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(2, 10);
    for (index, text) in ["ab", "cd", "ef", "gh", "ij", "kl"].iter().enumerate() {
        set_text(&mut buffer.lines[index], text);
    }
    for index in [1, 3, 5] {
        buffer.lines[index].wrapped = true;
    }
    buffer
}

fn assert_larger_case_screen(buffer: &XtermBuffer, first: usize) {
    for index in 0..first {
        assert_eq!(line(buffer, index), "    ", "line {index}");
    }
    assert_eq!(line(buffer, first), "abcd");
    assert_eq!(line(buffer, first + 1), "efgh");
    assert_eq!(line(buffer, first + 2), "ijkl");
    for index in first + 3..buffer.lines.len() {
        assert_eq!(line(buffer, index), "    ", "line {index}");
    }
    assert_wrapped(buffer, &[]);
}

/// Ten blank lines above the screen.
fn scrolled(buffer: &mut XtermBuffer) {
    for _ in 0..10 {
        buffer.lines.insert(0, Line::new(INIT_COLS, false));
    }
    buffer.ybase = 10;
}

#[test]
fn reflow_larger_with_the_viewport_not_filled_moves_the_cursor_up_and_adds_empty_lines() {
    let mut buffer = larger_case();
    buffer.y = 6;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (3, 0, 0));
    assert_eq!(buffer.lines.len(), 10);
    assert_larger_case_screen(&buffer, 0);
}

#[test]
fn reflow_larger_with_scrollback_remaining_and_ybase_0_moves_the_cursor_up() {
    let mut buffer = larger_case();
    buffer.y = 9;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (6, 0, 0));
    assert_eq!(buffer.lines.len(), 10);
    assert_larger_case_screen(&buffer, 0);
}

#[test]
fn reflow_larger_with_scrollback_keeps_ydisp_at_ybase() {
    let mut buffer = larger_case();
    buffer.y = 9;
    scrolled(&mut buffer);
    buffer.ydisp = 10;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (9, 7, 7));
    assert_eq!(buffer.lines.len(), 17);
    assert_larger_case_screen(&buffer, 10);
}

#[test]
fn reflow_larger_with_scrollback_keeps_a_scrolled_back_ydisp() {
    let mut buffer = larger_case();
    buffer.y = 9;
    scrolled(&mut buffer);
    buffer.ydisp = 5;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (9, 5, 7));
    assert_eq!(buffer.lines.len(), 17);
    assert_larger_case_screen(&buffer, 10);
}

#[test]
fn reflow_larger_with_no_scrollback_left_trims_lines_and_keeps_ydisp_at_ybase() {
    let mut buffer = larger_case();
    buffer.scrollback = 10;
    scrolled(&mut buffer);
    buffer.y = 9;
    buffer.ydisp = 10;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (9, 7, 7));
    assert_eq!(buffer.lines.len(), 17);
    assert_larger_case_screen(&buffer, 10);
}

#[test]
fn reflow_larger_with_no_scrollback_left_does_not_change_a_scrolled_back_ydisp() {
    let mut buffer = larger_case();
    buffer.scrollback = 10;
    scrolled(&mut buffer);
    buffer.y = 9;
    buffer.ydisp = 5;
    buffer.resize(4, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (9, 5, 7));
    assert_eq!(buffer.lines.len(), 17);
    assert_larger_case_screen(&buffer, 10);
}

/// 'abcd', 'efgh' (wrapped), 'ijkl', then seven blank rows, at four columns.
fn smaller_case() -> XtermBuffer {
    let mut buffer = buffer();
    fill_viewport_rows(&mut buffer);
    buffer.resize(4, 10);
    set_text(&mut buffer.lines[0], "abcd");
    set_text(&mut buffer.lines[1], "efgh");
    set_text(&mut buffer.lines[2], "ijkl");
    buffer
}

fn assert_smaller_case_screen(buffer: &XtermBuffer, first: usize, blank_above: usize) {
    for index in 0..blank_above {
        assert_eq!(line(buffer, index), "  ", "line {index}");
    }
    for (offset, expected) in ["ab", "cd", "ef", "gh", "ij", "kl"].iter().enumerate() {
        assert_eq!(
            line(buffer, first + offset),
            *expected,
            "line {}",
            first + offset
        );
    }
    for index in first + 6..buffer.lines.len() {
        assert_eq!(line(buffer, index), "  ", "line {index}");
    }
    assert_wrapped(buffer, &[first + 1, first + 3, first + 5]);
}

#[test]
fn reflow_smaller_with_the_viewport_not_filled_moves_the_cursor_down() {
    let mut buffer = smaller_case();
    buffer.y = 3;
    buffer.resize(2, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (6, 0, 0));
    assert_eq!(buffer.lines.len(), 10);
    assert_smaller_case_screen(&buffer, 0, 0);
}

#[test]
fn reflow_smaller_with_scrollback_remaining_and_ybase_0_trims_the_top() {
    let mut buffer = smaller_case();
    buffer.y = 9;
    buffer.resize(2, 10);
    assert_eq!((buffer.y, buffer.ydisp, buffer.ybase), (9, 3, 3));
    assert_eq!(buffer.lines.len(), 13);
    assert_smaller_case_screen(&buffer, 0, 0);
}

#[test]
fn reflow_smaller_with_scrollback_keeps_ydisp_at_ybase() {
    let mut buffer = smaller_case();
    buffer.y = 9;
    scrolled(&mut buffer);
    buffer.ydisp = 10;
    buffer.resize(2, 10);
    assert_eq!((buffer.ydisp, buffer.ybase), (13, 13));
    assert_eq!(buffer.lines.len(), 23);
    assert_smaller_case_screen(&buffer, 10, 10);
}

#[test]
fn reflow_smaller_with_scrollback_keeps_a_scrolled_back_ydisp() {
    let mut buffer = smaller_case();
    buffer.y = 9;
    scrolled(&mut buffer);
    buffer.ydisp = 5;
    buffer.resize(2, 10);
    assert_eq!((buffer.ydisp, buffer.ybase), (5, 13));
    assert_eq!(buffer.lines.len(), 23);
    assert_smaller_case_screen(&buffer, 10, 10);
}

#[test]
fn reflow_smaller_with_no_scrollback_left_trims_lines_and_keeps_ydisp_at_ybase() {
    let mut buffer = smaller_case();
    buffer.scrollback = 10;
    scrolled(&mut buffer);
    buffer.ydisp = 10;
    buffer.y = 13;
    buffer.resize(2, 10);
    assert_eq!((buffer.ydisp, buffer.ybase), (10, 10));
    assert_eq!(buffer.lines.len(), 20);
    assert_smaller_case_screen(&buffer, 7, 7);
}

#[test]
fn reflow_smaller_with_no_scrollback_left_does_not_change_a_scrolled_back_ydisp() {
    let mut buffer = smaller_case();
    buffer.scrollback = 10;
    scrolled(&mut buffer);
    buffer.ydisp = 5;
    buffer.y = 13;
    buffer.resize(2, 10);
    assert_eq!((buffer.ydisp, buffer.ybase), (5, 10));
    assert_eq!(buffer.lines.len(), 20);
    assert_smaller_case_screen(&buffer, 7, 7);
}

#[test]
fn a_buffer_without_scrollback_always_has_a_scrollback_of_0() {
    let mut buffer = buffer_with(1000, false);
    fill_viewport_rows(&mut buffer);
    assert_eq!(buffer.max_length, INIT_ROWS);
    buffer.resize(INIT_COLS, INIT_ROWS * 2);
    assert_eq!(buffer.max_length, INIT_ROWS * 2);
    buffer.resize(INIT_COLS, INIT_ROWS / 2);
    assert_eq!(buffer.max_length, INIT_ROWS / 2);
}

// ---- BufferReflow.test.ts --------------------------------------------------------------------

fn wide_line(cells: &[Cell]) -> Line {
    let mut line = Line::new(cells.len(), false);
    for (index, cell) in cells.iter().enumerate() {
        line.set_cell(index, cell.clone());
    }
    line
}

#[test]
fn new_line_lengths_for_a_small_line_with_wide_characters() {
    let line = wide_line(&[wide('汉'), spacer(), wide('语'), spacer()]);
    assert_eq!(text_of(&line, true), "汉语");
    let lines = [line];
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 4, 3), [2, 2]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 4, 2), [2, 2]);
}

#[test]
fn new_line_lengths_for_a_large_line_with_wide_characters() {
    let mut cells = Vec::new();
    for _ in 0..3 {
        cells.extend([wide('汉'), spacer(), wide('语'), spacer()]);
    }
    let line = wide_line(&cells);
    assert_eq!(text_of(&line, false), "汉语汉语汉语");
    let lines = [line];
    for (cols, expected) in [
        (11, vec![10, 2]),
        (10, vec![10, 2]),
        (9, vec![8, 4]),
        (8, vec![8, 4]),
        (7, vec![6, 6]),
        (6, vec![6, 6]),
        (5, vec![4, 4, 4]),
        (4, vec![4, 4, 4]),
        (3, vec![2, 2, 2, 2, 2, 2]),
        (2, vec![2, 2, 2, 2, 2, 2]),
    ] {
        assert_eq!(
            reflow_smaller_new_line_lengths(&lines, 12, cols),
            expected,
            "{cols} columns"
        );
    }
}

fn mixed_cells() -> Vec<Cell> {
    vec![ch('a'), wide('汉'), spacer(), wide('语'), spacer(), ch('b')]
}

#[test]
fn new_line_lengths_for_a_string_with_wide_and_single_characters() {
    let line = wide_line(&mixed_cells());
    assert_eq!(text_of(&line, false), "a汉语b");
    let lines = [line];
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 5), [5, 1]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 4), [3, 3]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 3), [3, 3]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 2), [1, 2, 2, 1]);
}

#[test]
fn new_line_lengths_for_a_wrapped_line_with_wide_and_single_characters() {
    let first = wide_line(&mixed_cells());
    let mut second = wide_line(&mixed_cells());
    second.wrapped = true;
    let lines = [first, second];
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 5), [5, 4, 3]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 4), [3, 4, 4, 1]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 6, 3), [3, 3, 3, 3]);
    assert_eq!(
        reflow_smaller_new_line_lengths(&lines, 6, 2),
        [1, 2, 2, 2, 2, 2, 1]
    );
}

#[test]
fn new_line_lengths_for_lines_ending_in_null_space() {
    let line = wide_line(&[wide('汉'), spacer(), wide('语'), spacer(), Cell::default()]);
    assert_eq!(text_of(&line, true), "汉语");
    assert_eq!(text_of(&line, false), "汉语 ");
    let lines = [line];
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 4, 3), [2, 2]);
    assert_eq!(reflow_smaller_new_line_lengths(&lines, 4, 2), [2, 2]);
}

/// One-column lines holding `chars`, each continuing the one above.
fn wrapped_chars(chars: &str) -> XtermBuffer {
    let lines = chars
        .chars()
        .enumerate()
        .map(|(index, c)| {
            let mut line = Line::new(1, index > 0);
            line.set_cell(0, ch(c));
            line
        })
        .collect();
    XtermBuffer::new(lines, 1, chars.len(), 0, true, 0)
}

#[test]
fn lines_to_remove_skips_the_wrapped_line_holding_the_cursor() {
    assert!(wrapped_chars("abcde")
        .reflow_larger_lines_to_remove(5, 2)
        .is_empty());
}

#[test]
fn lines_to_remove_reflows_wrapped_lines_when_the_cursor_is_outside() {
    assert!(!wrapped_chars("abcde")
        .reflow_larger_lines_to_remove(5, 10)
        .is_empty());
}

// ---- the grid ------------------------------------------------------------------------------

/// A grid fed `text` like a terminal with autowrap: rows wrap at the width, `\n` starts a new
/// line, and the screen scrolls into the scrollback. Returns the cursor (column, row).
fn feed(grid: &mut Grid, text: &str) -> (usize, usize) {
    let (mut col, mut row) = (0, 0);
    for c in text.chars() {
        if c == '\n' {
            col = 0;
            if row + 1 == grid.rows() {
                grid.scroll_up(1);
            } else {
                row += 1;
            }
            continue;
        }
        if col == grid.cols() {
            grid.set_line_wrapped(row, true);
            col = 0;
            if row + 1 == grid.rows() {
                grid.scroll_up(1);
            } else {
                row += 1;
            }
        }
        grid.set(col, row, Cell::new(c));
        col += 1;
    }
    (col, row)
}

fn buffer_lines(grid: &Grid) -> Vec<String> {
    let mut lines = Vec::new();
    for index in 0..grid.scrollback_len() {
        let text: String = grid
            .scrollback_line(index)
            .unwrap()
            .iter()
            .map(|cell| cell.c)
            .collect();
        lines.push(text.trim_end().to_string());
    }
    for row in 0..grid.rows() {
        lines.push(grid.row_text(row).trim_end().to_string());
    }
    lines
}

fn grid_with(cols: usize, rows: usize, scrollback: usize) -> Grid {
    let mut grid = Grid::new(cols, rows, scrollback);
    grid.set_compact_scrollback(true);
    grid
}

fn positions(col: usize, row: usize, viewport_top: usize) -> BufferPositions {
    BufferPositions {
        cursor_col: col,
        cursor_row: row,
        viewport_top,
        ..BufferPositions::default()
    }
}

#[test]
fn a_line_running_from_the_scrollback_onto_the_screen_reflows_as_one() {
    let mut grid = grid_with(10, 3, 100);
    let (col, row) = feed(&mut grid, "first\n0123456789abcdefghijklmnopqrstuvwxyz\n$ ");
    // first | 0123456789 abcdefghij (wrapped) || klmnopqrst uvwxyz (wrapped) $
    assert_eq!(grid.scrollback_len(), 3);
    assert_eq!((col, row), (2, 2));
    let mut at = positions(col, row, grid.scrollback_len());
    grid.resize_xterm(20, 3, true, &mut at);
    assert_eq!(
        buffer_lines(&grid),
        ["first", "0123456789abcdefghij", "klmnopqrstuvwxyz", "$"]
    );
    // The joined line came back from the scrollback; the prompt kept its line.
    assert_eq!(grid.scrollback_len(), 1);
    assert_eq!((at.cursor_col, at.cursor_row), (2, 2));
    assert_eq!(at.viewport_top, 1, "a viewport at the bottom stays there");
    assert!(grid.is_line_wrapped(0));
    assert!(!grid.is_line_wrapped(1));

    grid.resize_xterm(7, 3, true, &mut at);
    assert_eq!(
        buffer_lines(&grid),
        ["first", "0123456", "789abcd", "efghijk", "lmnopqr", "stuvwxy", "z", "$"]
    );
    assert_eq!((at.cursor_col, at.cursor_row), (2, 2));
    assert_eq!(at.viewport_top, grid.scrollback_len());
}

#[test]
fn the_cursor_follows_its_line_when_lines_above_it_rewrap() {
    let mut grid = grid_with(8, 6, 100);
    let (col, row) = feed(&mut grid, "abcdefghijkl\nxy\n$ ls");
    assert_eq!(buffer_lines(&grid)[..4], ["abcdefgh", "ijkl", "xy", "$ ls"]);
    assert_eq!((col, row), (4, 3));
    let mut at = positions(col, row, 0);
    grid.resize_xterm(4, 6, true, &mut at);
    assert_eq!(
        buffer_lines(&grid),
        ["abcd", "efgh", "ijkl", "xy", "$ ls", ""]
    );
    assert_eq!((at.cursor_col, at.cursor_row), (3, 4));
    assert_eq!(grid.scrollback_len(), 0);
}

#[test]
fn a_taller_screen_pulls_lines_back_from_the_scrollback() {
    let mut grid = grid_with(10, 3, 100);
    let (col, row) = feed(&mut grid, "one\ntwo\nthree\nfour\n$ ");
    assert_eq!(grid.scrollback_len(), 2);
    let mut at = positions(col, row, 2);
    grid.resize_xterm(10, 5, true, &mut at);
    assert_eq!(grid.scrollback_len(), 0);
    assert_eq!(buffer_lines(&grid), ["one", "two", "three", "four", "$"]);
    assert_eq!((at.cursor_col, at.cursor_row), (2, 4));
    // Shorter again: the lines go back into the scrollback, the cursor keeps its line.
    grid.resize_xterm(10, 2, true, &mut at);
    assert_eq!(grid.scrollback_len(), 3);
    assert_eq!(at.cursor_row, 1);
    assert_eq!(grid.row_text(1).trim_end(), "$");
}

#[test]
fn the_reflow_tells_where_every_line_went() {
    let mut grid = grid_with(4, 3, 100);
    let (col, row) = feed(&mut grid, "abcdefgh\nij\n$");
    // abcd efgh (wrapped) ij $ : two lines in the scrollback.
    assert_eq!(grid.scrollback_len(), 1);
    let mut at = positions(col, row, 1);
    grid.resize_xterm(8, 3, true, &mut at);
    let reflow = grid.take_buffer_reflow().unwrap();
    // "efgh" joined "abcd": its row went away, and it follows into the joined line.
    assert_eq!(reflow.lines, [Some(0), None, Some(1), Some(2)]);
    assert_eq!(reflow.follow(1), Some(0));
    assert_eq!(reflow.trimmed, 0);
    assert!(grid.take_buffer_reflow().is_none());
}

#[test]
fn a_compact_scrollback_stays_compact_and_wrapped_rows_full_width() {
    let mut grid = grid_with(10, 2, 100);
    let (col, row) = feed(&mut grid, "short\n0123456789abc\nx\n$");
    let mut at = positions(col, row, grid.scrollback_len());
    grid.resize_xterm(6, 2, true, &mut at);
    let rows: Vec<usize> = (0..grid.scrollback_len())
        .map(|index| grid.scrollback_line(index).unwrap().len())
        .collect();
    let wrapped: Vec<bool> = (0..grid.scrollback_len())
        .map(|index| grid.is_scrollback_wrapped(index))
        .collect();
    // short | 012345 6789ab (wrapped) c || x $
    assert_eq!(rows, [5, 6, 6, 1]);
    assert_eq!(wrapped, [false, true, true, false]);
    assert_eq!(grid.row(0).unwrap().len(), 6);
}

#[test]
fn zones_follow_their_lines_through_a_reflow() {
    use crate::zone::{Zone, ZoneType};
    let mut grid = grid_with(4, 3, 100);
    let (col, row) = feed(&mut grid, "abcdefgh\nij\nkl\n$");
    // abcd efgh(wrapped) ij kl $: "ij" is absolute line 2.
    grid.push_zone(Zone::new(0, ZoneType::Output, 2, None));
    grid.close_current_zone(3);
    let mut at = positions(col, row, grid.scrollback_len());
    grid.resize_xterm(8, 3, true, &mut at);
    let zone = &grid.zones()[0];
    assert_eq!((zone.abs_row_start, zone.abs_row_end), (1, 2));
}
