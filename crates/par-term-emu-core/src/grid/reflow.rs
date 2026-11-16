//! The buffer resize of xterm.js, ported from xterm.js 6.1 (`Buffer.resize`,
//! `Buffer._reflowLarger`, `Buffer._reflowSmaller` in `src/common/buffer/Buffer.ts` and the
//! helpers in `src/common/buffer/BufferReflow.ts`; MIT, Copyright (c) 2017-2019 The xterm.js
//! authors).
//!
//! xterm.js keeps the scrollback and the screen in one list of lines and resizes that list as a
//! whole: rows added at the bottom pull lines back from the scrollback while the cursor sits on
//! the last line, rows removed take the blank lines below the cursor first and then push the top
//! lines into the scrollback, and a width change rewraps every logical line except the one the
//! cursor is on (xterm.js's `reflowCursorLine` option, off by default, is not ported), moving
//! the cursor, the saved cursor and the viewport with the lines. [`XtermBuffer`] is that list;
//! [`super::Grid::resize_xterm`] builds it from a grid, resizes it and stores it back.
//!
//! The port keeps xterm.js's statements and their order, including its edge cases (a cursor that
//! moves up when lines below it unwrap, lines dropped from the bottom when the screen is not
//! full), so the result matches xterm.js line for line. Where xterm.js would index past an array
//! (and throw) the port stops instead.

// The resize and reflow in this file are ported from xterm.js
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

use crate::cell::Cell;
use crate::color::{Color, NamedColor};
use crate::grid::Grid;

/// The positions an xterm.js resize moves along with the lines
/// ([`Grid::resize_xterm`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferPositions {
    /// The cursor's column: the screen width while a wrap is pending (xterm.js's cursor stands
    /// past the last column then).
    pub cursor_col: usize,
    /// The cursor's screen row.
    pub cursor_row: usize,
    /// The saved cursor's column (DECSC).
    pub saved_col: usize,
    /// The saved cursor's screen row.
    pub saved_row: usize,
    /// The viewport's top line, a buffer line (scrollback first): the screen's first line when
    /// the viewport is at the bottom.
    pub viewport_top: usize,
}

/// Where an xterm.js resize moved the buffer's lines
/// ([`Grid::take_buffer_reflow`]), for callers that keep line numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferReflow {
    /// For every line before the resize (scrollback first, then the screen), its buffer line
    /// after the resize, or `None` when it went away: a row emptied by joining a wrapped line
    /// (xterm.js disposes the markers on it), a line dropped below the cursor, or one trimmed
    /// off the top.
    pub lines: Vec<Option<usize>>,
    /// Lines the resize trimmed off the top of the buffer; they count as trimmed lines, so the
    /// absolute number of buffer line `n` is the old first absolute line plus `trimmed + n`.
    pub trimmed: usize,
}

impl BufferReflow {
    /// The buffer line that holds the start of old line `old` after the resize: its own, or for
    /// a line that went away the nearest line above it that is still there.
    pub fn follow(&self, old: usize) -> Option<usize> {
        let end = old.min(self.lines.len().checked_sub(1)?);
        self.lines[..=end].iter().rev().find_map(|line| *line)
    }
}

impl Grid {
    /// Resize the way xterm.js resizes a buffer (`Buffer.resize`): the scrollback
    /// and the screen are one list of lines, so a taller screen pulls lines back from the
    /// scrollback, a shorter one drops the blank lines below the cursor before it pushes lines
    /// into the scrollback, and on a width change (`normal`, the buffer with scrollback) every
    /// logical line but the cursor's is rewrapped across the scrollback and the screen alike.
    /// `positions` (the cursor, the saved cursor and the viewport) move with their lines. Where
    /// every line went is kept for [`Grid::take_buffer_reflow`]. A no-op when the size is
    /// unchanged or either dimension is zero.
    pub fn resize_xterm(
        &mut self,
        cols: usize,
        rows: usize,
        normal: bool,
        positions: &mut BufferPositions,
    ) {
        if (self.cols == cols && self.rows == rows) || cols == 0 || rows == 0 {
            return;
        }
        let scrollback = self.scrollback_rows.len();
        let old_len = scrollback + self.rows;
        let floor = self
            .total_lines_scrolled
            .saturating_sub(self.scrollback_lines);

        // One list, xterm.js's way round: a line's flag says it continues the line above.
        let mut lines = Vec::with_capacity(old_len + rows);
        let mut continues = false;
        let stored = std::mem::take(&mut self.scrollback_rows);
        let stored_wrapped = std::mem::take(&mut self.scrollback_wrapped);
        for (index, (row, wraps)) in stored.into_iter().zip(stored_wrapped).enumerate() {
            lines.push(Line::from_cells(
                row.into_vec(),
                self.cols,
                continues,
                Some(index),
            ));
            continues = wraps;
        }
        for (row_index, row) in std::mem::take(&mut self.screen).into_iter().enumerate() {
            lines.push(Line::from_cells(
                row.into_vec(),
                self.cols,
                continues,
                Some(scrollback + row_index),
            ));
            continues = self.wrapped.get(row_index).copied().unwrap_or(false);
        }

        let mut buffer = XtermBuffer::new(
            lines,
            self.cols,
            self.rows,
            self.max_scrollback,
            normal,
            scrollback,
        );
        buffer.x = positions.cursor_col;
        buffer.y = positions.cursor_row;
        buffer.saved_x = positions.saved_col;
        buffer.saved_y = scrollback + positions.saved_row;
        buffer.ydisp = positions.viewport_top.min(scrollback);
        buffer.resize(cols, rows);

        // The engine keeps at most `max_scrollback` lines above the screen and exactly `rows`
        // screen rows.
        let mut ybase = buffer.ybase.min(buffer.lines.len());
        let over = ybase.saturating_sub(self.max_scrollback);
        if over > 0 {
            buffer.trim_start(over);
            ybase -= over;
            buffer.ydisp = buffer.ydisp.saturating_sub(over);
            buffer.saved_y = buffer.saved_y.saturating_sub(over);
        }
        buffer.lines.truncate(ybase + rows);

        let mut moved = vec![None; old_len];
        for (index, line) in buffer.lines.iter().enumerate() {
            if let Some(origin) = line.origin.filter(|origin| *origin < old_len) {
                moved[origin] = Some(index);
            }
        }
        let wraps_next: Vec<bool> = (0..buffer.lines.len())
            .map(|index| buffer.lines.get(index + 1).is_some_and(|line| line.wrapped))
            .collect();

        let mut lines = std::mem::take(&mut buffer.lines).into_iter().enumerate();
        for (index, line) in lines.by_ref().take(ybase) {
            let mut cells = line.cells;
            // A row that continues on the next one keeps the full width; a line's last row is
            // stored without its blank tail when the scrollback is compact.
            if wraps_next[index] || !self.compact_scrollback {
                cells.resize(cols, Cell::default());
            } else {
                cells.truncate(super::used_len(&cells));
            }
            self.scrollback_rows.push_back(cells.into_boxed_slice());
            self.scrollback_wrapped.push_back(wraps_next[index]);
        }
        let mut screen = Vec::with_capacity(rows);
        let mut row_used = Vec::with_capacity(rows);
        let mut wrapped = Vec::with_capacity(rows);
        for (index, line) in lines {
            let mut cells = line.cells;
            cells.resize(cols, Cell::default());
            row_used.push(super::used_len(&cells));
            screen.push(cells.into_boxed_slice());
            wrapped.push(wraps_next[index]);
        }
        while screen.len() < rows {
            screen.push(super::blank_row(cols));
            row_used.push(0);
            wrapped.push(false);
        }
        self.screen = screen;
        self.row_used = row_used;
        self.wrapped = wrapped;
        self.cols = cols;
        self.rows = rows;
        self.scrollback_lines = self.scrollback_rows.len();
        let trimmed = buffer.trimmed;
        self.total_lines_scrolled = floor + trimmed + self.scrollback_lines;

        let reflow = BufferReflow {
            lines: moved,
            trimmed,
        };
        // Zones follow their lines.
        let new_len = ybase + rows;
        let first = floor + trimmed;
        let follow = |abs: usize| -> usize {
            match abs.checked_sub(floor) {
                Some(index) if index < old_len => {
                    reflow.follow(index).map_or(first, |line| first + line)
                }
                Some(index) => first + new_len + (index - old_len),
                None => abs,
            }
        };
        for zone in &mut self.zones {
            zone.abs_row_start = follow(zone.abs_row_start);
            zone.abs_row_end = follow(zone.abs_row_end).max(zone.abs_row_start);
        }
        self.evict_zones(first);

        positions.cursor_col = buffer.x;
        positions.cursor_row = buffer.y.min(rows - 1);
        positions.saved_col = buffer.saved_x;
        positions.saved_row = buffer.saved_y.saturating_sub(ybase).min(rows - 1);
        positions.viewport_top = buffer.ydisp.min(ybase);

        self.reset_damage_for_resize();
        self.last_buffer_reflow = Some(reflow);
    }

    /// Where the last [`Grid::resize_xterm`] moved the buffer's lines, once.
    pub fn take_buffer_reflow(&mut self) -> Option<BufferReflow> {
        self.last_buffer_reflow.take()
    }

    /// Where the last [`Grid::resize_xterm`] moved the buffer's lines, until it
    /// is taken.
    pub fn buffer_reflow(&self) -> Option<&BufferReflow> {
        self.last_buffer_reflow.as_ref()
    }
}

/// Whether xterm.js would count `cell` as content: a character a program wrote. Blank and erased
/// cells (a space with the built-in default foreground and no marks, whatever their background)
/// and the right half of a wide character have none, like xterm.js's null cells.
pub(crate) fn has_content(cell: &Cell) -> bool {
    !cell.flags.wide_char_spacer()
        && cell.c != '\0'
        && !(cell.c == ' '
            && cell.combining.is_none()
            && cell.fg.unpack() == Color::Named(NamedColor::White))
}

/// One buffer line (xterm.js `BufferLine`). Cells past `cells.len()` (and below `len`) are null
/// cells, so a short scrollback row is used as it is stored.
#[derive(Debug, Clone, Default)]
pub(crate) struct Line {
    pub(crate) cells: Vec<Cell>,
    /// The line's length in cells (`BufferLine.length`).
    pub(crate) len: usize,
    /// `isWrapped`: this line continues the line above it.
    pub(crate) wrapped: bool,
    /// The line's index before the resize; `None` for a line the resize added.
    pub(crate) origin: Option<usize>,
}

impl Line {
    pub(crate) fn new(len: usize, wrapped: bool) -> Self {
        Self {
            cells: Vec::new(),
            len,
            wrapped,
            origin: None,
        }
    }

    /// A line of `len` cells holding `cells` (cut to `len`).
    pub(crate) fn from_cells(
        mut cells: Vec<Cell>,
        len: usize,
        wrapped: bool,
        origin: Option<usize>,
    ) -> Self {
        cells.truncate(len);
        Self {
            cells,
            len,
            wrapped,
            origin,
        }
    }

    fn cell(&self, index: usize) -> Cell {
        self.cells.get(index).cloned().unwrap_or_default()
    }

    /// `getWidth`: 2 for the left half of a wide character, 0 for its right half, else 1.
    pub(crate) fn width(&self, index: usize) -> usize {
        match self.cells.get(index) {
            Some(cell) if cell.flags.wide_char_spacer() => 0,
            Some(cell) if cell.flags.wide_char() => 2,
            _ => 1,
        }
    }

    /// `hasContent`.
    pub(crate) fn has_content(&self, index: usize) -> bool {
        index < self.len && self.cells.get(index).is_some_and(has_content)
    }

    /// `getTrimmedLength`: the cells up to and including the last one with content.
    pub(crate) fn trimmed_len(&self) -> usize {
        let end = self.cells.len().min(self.len);
        (0..end)
            .rev()
            .find(|&index| has_content(&self.cells[index]))
            .map_or(0, |index| index + self.width(index))
    }

    /// `setCell`.
    pub(crate) fn set_cell(&mut self, index: usize, cell: Cell) {
        if index >= self.len {
            return;
        }
        if index >= self.cells.len() {
            if cell == Cell::default() {
                return;
            }
            self.cells.resize(index + 1, Cell::default());
        }
        self.cells[index] = cell;
    }

    /// `replaceCells(start, end, nullCell)`: null cells over `[start, end)`, and over the other
    /// half of a wide character cut at either end.
    fn clear_cells(&mut self, start: usize, end: usize) {
        if start > 0 && self.width(start - 1) == 2 {
            self.set_cell(start - 1, Cell::default());
        }
        if end < self.len && end > 0 && self.width(end - 1) == 2 {
            self.set_cell(end, Cell::default());
        }
        let stop = end.min(self.len);
        if start >= stop {
            return;
        }
        if stop >= self.cells.len() {
            self.cells.truncate(start);
        } else {
            for cell in &mut self.cells[start..stop] {
                *cell = Cell::default();
            }
        }
    }

    /// `resize(cols, nullCell)`: a longer line gets null cells, a shorter one is cut (a wide
    /// character cut in half becomes a null cell).
    pub(crate) fn resize(&mut self, len: usize) {
        if len >= self.len {
            self.len = len;
            return;
        }
        self.cells.truncate(len);
        self.len = len;
        if len > 0 && self.width(len - 1) == 2 {
            self.set_cell(len - 1, Cell::default());
        }
    }
}

/// `copyCellsFrom` between two lines of `lines` (or within one), as a move of the cell range:
/// xterm.js picks the copy direction so overlapping ranges behave the same.
fn copy_cells(
    lines: &mut [Line],
    src: usize,
    src_col: usize,
    dest: usize,
    dest_col: usize,
    count: usize,
) {
    if count == 0 {
        return;
    }
    if src == dest {
        let cells: Vec<Cell> = (src_col..src_col + count)
            .map(|index| lines[src].cell(index))
            .collect();
        for (offset, cell) in cells.into_iter().enumerate() {
            lines[dest].set_cell(dest_col + offset, cell);
        }
        return;
    }
    let (from, to) = if src < dest {
        let (head, tail) = lines.split_at_mut(dest);
        (&head[src], &mut tail[0])
    } else {
        let (head, tail) = lines.split_at_mut(src);
        (&tail[0], &mut head[dest])
    };
    for offset in 0..count {
        to.set_cell(dest_col + offset, from.cell(src_col + offset));
    }
}

/// `getWrappedLineTrimmedLength`: the cells a row of a wrapped line holds. A row other than the
/// last counts as full, except when it ends in a null cell because the next row starts with a
/// wide character that did not fit.
pub(crate) fn wrapped_line_trimmed_len(lines: &[Line], index: usize, cols: usize) -> usize {
    if index + 1 == lines.len() {
        return lines[index].trimmed_len();
    }
    let ends_in_null = !lines[index].has_content(cols - 1) && lines[index].width(cols - 1) == 1;
    let next_starts_wide = lines[index + 1].width(0) == 2;
    if ends_in_null && next_starts_wide {
        cols - 1
    } else {
        cols
    }
}

/// `reflowSmallerGetNewLineLengths`: the length of each row a wrapped line takes at `new_cols`
/// (`new_cols - 1` where a wide character moves on to the next row).
pub(crate) fn reflow_smaller_new_line_lengths(
    lines: &[Line],
    old_cols: usize,
    new_cols: usize,
) -> Vec<usize> {
    let mut lengths = Vec::new();
    let cells_needed: usize = (0..lines.len())
        .map(|index| wrapped_line_trimmed_len(lines, index, old_cols))
        .sum();
    let mut src_col = 0;
    let mut src_line = 0;
    let mut cells_available = 0;
    while cells_available < cells_needed {
        if cells_needed - cells_available < new_cols {
            lengths.push(cells_needed - cells_available);
            break;
        }
        src_col += new_cols;
        let old_trimmed = wrapped_line_trimmed_len(lines, src_line, old_cols);
        if src_col > old_trimmed {
            src_col -= old_trimmed;
            src_line += 1;
        }
        let ends_with_wide = lines
            .get(src_line)
            .is_some_and(|line| src_col > 0 && line.width(src_col - 1) == 2);
        if ends_with_wide {
            src_col -= 1;
        }
        let length = if ends_with_wide {
            new_cols - 1
        } else {
            new_cols
        };
        // xterm.js locks up here at one column with a wide character.
        if length == 0 {
            break;
        }
        lengths.push(length);
        cells_available += length;
    }
    lengths
}

/// xterm.js's `Buffer` as far as a resize touches it.
#[derive(Debug, Clone)]
pub(crate) struct XtermBuffer {
    /// Scrollback, then the screen.
    pub(crate) lines: Vec<Line>,
    /// `lines.maxLength`: the rows plus the scrollback.
    pub(crate) max_length: usize,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    /// The scrollback option.
    pub(crate) scrollback: usize,
    /// The normal buffer (it has scrollback and reflows); the alternate one has neither.
    pub(crate) has_scrollback: bool,
    /// The first screen line.
    pub(crate) ybase: usize,
    /// The viewport's top line.
    pub(crate) ydisp: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) saved_x: usize,
    /// The saved cursor's line (absolute in the buffer, as xterm.js saves `ybase + y`).
    pub(crate) saved_y: usize,
    /// Lines the resize dropped off the top of the buffer.
    pub(crate) trimmed: usize,
}

impl XtermBuffer {
    /// A buffer of `cols` × `rows` holding `lines`, its screen starting at `ybase`.
    pub(crate) fn new(
        lines: Vec<Line>,
        cols: usize,
        rows: usize,
        scrollback: usize,
        has_scrollback: bool,
        ybase: usize,
    ) -> Self {
        let mut buffer = Self {
            lines,
            max_length: 0,
            cols,
            rows,
            scrollback,
            has_scrollback,
            ybase,
            ydisp: ybase,
            x: 0,
            y: 0,
            saved_x: 0,
            saved_y: 0,
            trimmed: 0,
        };
        buffer.max_length = buffer.correct_buffer_length(rows);
        buffer
    }

    /// `_getCorrectBufferLength`.
    fn correct_buffer_length(&self, rows: usize) -> usize {
        if self.has_scrollback {
            rows + self.scrollback
        } else {
            rows
        }
    }

    /// Drop `count` lines off the top (`CircularList.trimStart`).
    fn trim_start(&mut self, count: usize) {
        let count = count.min(self.lines.len());
        self.lines.drain(..count);
        self.trimmed += count;
    }

    /// `Buffer.resize`.
    pub(crate) fn resize(&mut self, new_cols: usize, new_rows: usize) {
        let new_max_length = self.correct_buffer_length(new_rows);
        if new_max_length > self.max_length {
            self.max_length = new_max_length;
        }

        if !self.lines.is_empty() {
            // Deal with columns increasing (reducing needs to happen after reflow).
            if self.cols < new_cols {
                for line in &mut self.lines {
                    line.resize(new_cols);
                }
            }

            // Resize rows in both directions as needed.
            let mut add_to_y = 0;
            if self.rows < new_rows {
                for _ in self.rows..new_rows {
                    if self.lines.len() < new_rows + self.ybase {
                        if self.ybase > 0 && self.lines.len() <= self.ybase + self.y + add_to_y + 1
                        {
                            // There is room above the buffer and there are no empty elements
                            // below the line: scroll up.
                            self.ybase -= 1;
                            add_to_y += 1;
                            if self.ydisp > 0 {
                                // The viewport is at the top of the buffer: it grows downwards.
                                self.ydisp -= 1;
                            }
                        } else {
                            // Add a blank line when there is no buffer left at the top to scroll
                            // to, or when there are blank lines after the cursor.
                            self.lines.push(Line::new(new_cols, false));
                        }
                    }
                }
            } else {
                for _ in new_rows..self.rows {
                    if self.lines.len() > new_rows + self.ybase {
                        if self.lines.len() > self.ybase + self.y + 1 {
                            // A line below the cursor: remove it.
                            self.lines.pop();
                        } else {
                            // The line is the cursor's: scroll down.
                            self.ybase += 1;
                            self.ydisp += 1;
                        }
                    }
                }
            }

            // Reduce the maximum length after the adjustments, which would otherwise cut lines
            // off the bottom of the buffer.
            if new_max_length < self.max_length {
                let amount = self.lines.len().saturating_sub(new_max_length);
                if amount > 0 {
                    self.trim_start(amount);
                    self.ybase = self.ybase.saturating_sub(amount);
                    self.ydisp = self.ydisp.saturating_sub(amount);
                    self.saved_y = self.saved_y.saturating_sub(amount);
                }
                self.max_length = new_max_length;
            }

            // Make sure that the cursor stays on screen.
            self.x = self.x.min(new_cols - 1);
            self.y = self.y.min(new_rows - 1);
            self.y += add_to_y;
            self.saved_x = self.saved_x.min(new_cols - 1);
        }

        if self.has_scrollback {
            self.reflow(new_cols, new_rows);
        }
        // Trim the end of the lines off when the columns shrank. (xterm.js leaves the alternate
        // buffer's lines long; the engine's screen rows are exactly as wide as the screen.)
        if self.cols > new_cols {
            for line in &mut self.lines {
                line.resize(new_cols);
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;

        // The cursor's line must be in the buffer.
        if !self.lines.is_empty() {
            let max_y = self.lines.len().saturating_sub(self.ybase + 1);
            self.y = self.y.min(max_y);
        }
    }

    /// `_reflow`.
    fn reflow(&mut self, new_cols: usize, new_rows: usize) {
        if self.cols == new_cols {
            return;
        }
        if new_cols > self.cols {
            self.reflow_larger(new_cols, new_rows);
        } else {
            self.reflow_smaller(new_cols, new_rows);
        }
    }

    /// `_reflowLarger`.
    fn reflow_larger(&mut self, new_cols: usize, new_rows: usize) {
        let to_remove = self.reflow_larger_lines_to_remove(new_cols, self.ybase + self.y);
        if to_remove.is_empty() {
            return;
        }
        // `reflowLargerCreateNewLayout` and `reflowLargerApplyNewLayout` in one pass.
        let mut removed = 0;
        let mut ranges = to_remove.iter().peekable();
        let mut kept = Vec::with_capacity(self.lines.len());
        let mut skip_until = 0;
        for (index, line) in std::mem::take(&mut self.lines).into_iter().enumerate() {
            if index < skip_until {
                continue;
            }
            if let Some(&&(start, count)) = ranges.peek() {
                if start == index {
                    ranges.next();
                    removed += count;
                    skip_until = index + count;
                    continue;
                }
            }
            kept.push(line);
        }
        self.lines = kept;
        self.reflow_larger_adjust_viewport(new_cols, new_rows, removed);
    }

    /// `reflowLargerGetLinesToRemove` (with `reflowCursorLine` off): joins every wrapped line
    /// but the cursor's at the new width, and returns the `(index, count)` runs of rows that are
    /// left empty.
    fn reflow_larger_lines_to_remove(
        &mut self,
        new_cols: usize,
        cursor_line: usize,
    ) -> Vec<(usize, usize)> {
        let old_cols = self.cols;
        let mut to_remove = Vec::new();
        let mut y = 0;
        while y + 1 < self.lines.len() {
            // Check whether this row is wrapped, and for how many rows.
            let mut next = y + 1;
            if !self.lines[next].wrapped {
                y += 1;
                continue;
            }
            while next < self.lines.len() && self.lines[next].wrapped {
                next += 1;
            }
            let count = next - y;

            // The program fixes up the wrapped line the cursor is on.
            if cursor_line >= y && cursor_line < next {
                y += count;
                continue;
            }

            let mut group: Vec<Line> = self.lines[y..next].iter_mut().map(std::mem::take).collect();

            // Copy the cells to their new places.
            let mut dest = 0;
            let mut dest_col = wrapped_line_trimmed_len(&group, 0, old_cols);
            let mut src = 1;
            let mut src_col = 0;
            while src < group.len() && dest < group.len() {
                let src_trimmed = wrapped_line_trimmed_len(&group, src, old_cols);
                let src_remaining = src_trimmed.saturating_sub(src_col);
                let dest_remaining = new_cols - dest_col;
                let cells = src_remaining.min(dest_remaining);

                copy_cells(&mut group, src, src_col, dest, dest_col, cells);

                dest_col += cells;
                if dest_col == new_cols {
                    dest += 1;
                    dest_col = 0;
                }
                src_col += cells;
                if src_col >= src_trimmed {
                    src += 1;
                    src_col = 0;
                }

                // Make sure the last cell is not wide; if it is, it moves to the next row.
                if dest_col == 0
                    && dest != 0
                    && dest < group.len()
                    && group[dest - 1].width(new_cols - 1) == 2
                {
                    copy_cells(&mut group, dest - 1, new_cols - 1, dest, dest_col, 1);
                    dest_col += 1;
                    // Null out the end of the last row.
                    group[dest - 1].set_cell(new_cols - 1, Cell::default());
                }
            }

            // Clear out the remaining cells or fragments could remain.
            if let Some(line) = group.get_mut(dest) {
                line.clear_cells(dest_col, new_cols);
            }

            // Work backwards and remove the rows at the end that hold only null cells.
            let mut count_to_remove = 0;
            for index in (1..group.len()).rev() {
                if index > dest || group[index].trimmed_len() == 0 {
                    count_to_remove += 1;
                } else {
                    break;
                }
            }

            for (offset, line) in group.into_iter().enumerate() {
                self.lines[y + offset] = line;
            }
            if count_to_remove > 0 {
                to_remove.push((y + count - count_to_remove, count_to_remove));
            }
            y += count;
        }
        to_remove
    }

    /// `_reflowLargerAdjustViewport`.
    fn reflow_larger_adjust_viewport(&mut self, new_cols: usize, new_rows: usize, removed: usize) {
        for _ in 0..removed {
            if self.ybase == 0 {
                if self.y > 0 {
                    self.y -= 1;
                }
                if self.lines.len() < new_rows {
                    // Add an extra row at the bottom of the viewport.
                    self.lines.push(Line::new(new_cols, false));
                }
            } else {
                if self.ydisp == self.ybase {
                    self.ydisp -= 1;
                }
                self.ybase -= 1;
            }
        }
        self.saved_y = self.saved_y.saturating_sub(removed);
    }

    /// `_reflowSmaller` (with `reflowCursorLine` off).
    fn reflow_smaller(&mut self, new_cols: usize, new_rows: usize) {
        let old_cols = self.cols;
        // The rows to insert, gathered so the list is rebuilt once: (the last row of the wrapped
        // line they follow, the rows), bottom first.
        let mut to_insert: Vec<(usize, Vec<Line>)> = Vec::new();
        let mut count_to_insert = 0;
        // Go backwards, as many lines may be trimmed and this avoids considering them.
        let mut y = self.lines.len() as isize - 1;
        while y >= 0 {
            let last = y as usize;
            y -= 1;
            let Some(line) = self.lines.get(last) else {
                continue;
            };
            if !line.wrapped && line.trimmed_len() <= new_cols {
                continue;
            }

            // Gather the wrapped rows; `start` is the first.
            let mut start = last;
            while self.lines[start].wrapped && start > 0 {
                start -= 1;
            }
            y = start as isize - 1;
            let count = last + 1 - start;

            // The program fixes up the wrapped line the cursor is on.
            let cursor_line = self.ybase + self.y;
            if cursor_line >= start && cursor_line < start + count {
                continue;
            }

            let last_line_length = self.lines[last].trimmed_len();
            let lengths =
                reflow_smaller_new_line_lengths(&self.lines[start..=last], old_cols, new_cols);
            if lengths.is_empty() {
                continue;
            }
            let lines_to_add = lengths.len().saturating_sub(count);
            let trimmed_lines = if self.ybase == 0 && self.y + 1 != self.lines.len() {
                // The top of the buffer is not filled yet.
                (self.y + lines_to_add).saturating_sub(self.max_length)
            } else {
                (self.lines.len() + lines_to_add).saturating_sub(self.max_length)
            };

            let mut group: Vec<Line> = self.lines[start..=last]
                .iter_mut()
                .map(std::mem::take)
                .collect();
            group.extend((0..lines_to_add).map(|_| Line::new(new_cols, true)));

            // Copy the cells to their new places, backwards so it can be done in place.
            let mut dest = lengths.len() as isize - 1;
            let mut dest_col = lengths[dest as usize];
            if dest_col == 0 {
                dest -= 1;
                dest_col = if dest >= 0 { lengths[dest as usize] } else { 0 };
            }
            let mut src = count as isize - 1;
            let mut src_col = last_line_length;
            while src >= 0 {
                // xterm.js checks the row exists (it has been known not to).
                if dest < 0 || dest as usize >= group.len() {
                    break;
                }
                let cells = src_col.min(dest_col);
                copy_cells(
                    &mut group,
                    src as usize,
                    src_col - cells,
                    dest as usize,
                    dest_col - cells,
                    cells,
                );
                dest_col -= cells;
                if dest_col == 0 {
                    dest -= 1;
                    dest_col = if dest >= 0 {
                        lengths.get(dest as usize).copied().unwrap_or(0)
                    } else {
                        0
                    };
                }
                src_col -= cells;
                if src_col == 0 {
                    src -= 1;
                    let index = src.max(0) as usize;
                    src_col = wrapped_line_trimmed_len(&group, index, old_cols);
                }
            }

            // Null out the ends of the rows a wide character wrapped away from.
            for (index, line) in group.iter_mut().enumerate() {
                if let Some(&length) = lengths.get(index) {
                    if length < new_cols {
                        line.set_cell(length, Cell::default());
                    }
                }
            }

            let new_lines = group.split_off(count);
            for (offset, line) in group.into_iter().enumerate() {
                self.lines[start + offset] = line;
            }
            if !new_lines.is_empty() {
                count_to_insert += new_lines.len();
                to_insert.push((last, new_lines));
            }

            // Adjust the viewport as needed.
            let mut adjustments = lines_to_add as isize - trimmed_lines as isize;
            while adjustments > 0 {
                adjustments -= 1;
                if self.ybase == 0 {
                    if self.y + 1 < new_rows {
                        self.y += 1;
                        self.lines.pop();
                    } else {
                        self.ybase += 1;
                        self.ydisp += 1;
                    }
                } else {
                    // Ensure ybase does not exceed its maximum value.
                    let limit = self.max_length.min(self.lines.len() + count_to_insert) as isize
                        - new_rows as isize;
                    if (self.ybase as isize) < limit {
                        if self.ybase == self.ydisp {
                            self.ydisp += 1;
                        }
                        self.ybase += 1;
                    }
                }
            }
            self.saved_y = (self.saved_y + lines_to_add).min(self.ybase + new_rows - 1);
        }

        // Rearrange the lines once, rather than splicing per wrapped line. A wrapped line whose
        // last row was dropped from the bottom above has its new rows go last, as in xterm.js.
        if !to_insert.is_empty() {
            let original = std::mem::take(&mut self.lines);
            let last = original.len().saturating_sub(1);
            let mut merged = Vec::with_capacity(original.len() + count_to_insert);
            let mut inserts = to_insert.into_iter().rev().peekable();
            for (index, line) in original.into_iter().enumerate() {
                merged.push(line);
                while inserts
                    .peek()
                    .is_some_and(|(after, _)| (*after).min(last) == index)
                {
                    merged.extend(inserts.next().map(|(_, lines)| lines).unwrap_or_default());
                }
            }
            for (_, lines) in inserts {
                merged.extend(lines);
            }
            let excess = merged.len().saturating_sub(self.max_length);
            merged.drain(..excess);
            self.trimmed += excess;
            self.lines = merged;
        }
    }
}

#[cfg(test)]
mod tests;
