//! The vi-style copy-mode state machine (ported from par-term's
//! `copy_mode`): an independent cursor roaming scrollback plus the
//! frozen screen, character/line/block visual selections, count
//! prefixes, named marks, and a search query — all unit-testable
//! without rendering. The key tables and the pane plumbing (viewport
//! scrolling, OSC 52, painting) live in the modal + session layers.
//!
//! Absolute line indexing: line 0 is the oldest scrollback line, line
//! `scrollback_len - 1` the newest, and `scrollback_len + row` the
//! live grid's row — the same coordinate the renderer's
//! [`Selection`] paints and reads. The live rows are snapshotted at
//! entry (`live_rows`): lines past the scrollback read that frozen
//! snapshot, so output that rewrites the screen under an open mode
//! cannot move the text under the cursor.

use std::collections::HashMap;

use super::Selection;

/// Visual selection mode in copy mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualMode {
    /// No visual selection active
    None,
    /// Character-wise selection (v)
    Char,
    /// Line-wise selection (V)
    Line,
    /// Block/rectangular selection (Ctrl+V)
    Block,
}

/// Search direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

/// A named mark position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) col: usize,
    pub(crate) absolute_line: usize,
}

/// Copy mode state machine (see the module docs for the coordinates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyModeState {
    /// Cursor column position
    pub(crate) cursor_col: usize,
    /// Cursor absolute line position
    pub(crate) cursor_absolute_line: usize,
    /// Current visual selection mode
    pub(crate) visual_mode: VisualMode,
    /// Selection anchor point (absolute_line, col) - set when entering visual mode
    pub(crate) selection_anchor: Option<(usize, usize)>,
    /// Count prefix for motions (e.g., 5j moves down 5 lines)
    pub(crate) count: Option<usize>,
    /// Named marks (a-z)
    pub(crate) marks: HashMap<char, Mark>,
    /// Terminal columns
    pub(crate) cols: usize,
    /// Terminal rows
    pub(crate) rows: usize,
    /// Scrollback buffer length (kept current; rebased on pushed lines)
    pub(crate) scrollback_len: usize,
    /// The visible live rows at entry — the frozen screen lines read
    /// past the scrollback.
    pub(crate) live_rows: Vec<String>,
    /// Current search query
    pub(crate) search_query: String,
    /// Search direction
    pub(crate) search_direction: SearchDirection,
    /// Whether search input mode is active
    pub(crate) is_searching: bool,
    /// Waiting for second 'g' in 'gg'
    pub(crate) pending_g: bool,
    /// Waiting for mark name after 'm'
    pub(crate) pending_mark_set: bool,
    /// Waiting for mark name after "'"
    pub(crate) pending_mark_goto: bool,
}

impl Default for CopyModeState {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyModeState {
    /// Create a new copy mode state (not entered yet).
    pub(crate) fn new() -> Self {
        Self {
            cursor_col: 0,
            cursor_absolute_line: 0,
            visual_mode: VisualMode::None,
            selection_anchor: None,
            count: None,
            marks: HashMap::new(),
            cols: 80,
            rows: 24,
            scrollback_len: 0,
            live_rows: Vec::new(),
            search_query: String::new(),
            search_direction: SearchDirection::Forward,
            is_searching: false,
            pending_g: false,
            pending_mark_set: false,
            pending_mark_goto: false,
        }
    }

    /// Enter copy mode at the given screen cursor position.
    pub(crate) fn enter(
        &mut self,
        cursor_col: usize,
        cursor_row: usize,
        cols: usize,
        rows: usize,
        scrollback_len: usize,
    ) {
        self.cols = cols;
        self.rows = rows;
        self.scrollback_len = scrollback_len;
        // Convert screen row to absolute line
        self.cursor_absolute_line = scrollback_len + cursor_row;
        self.cursor_col = cursor_col.min(cols.saturating_sub(1));
        self.visual_mode = VisualMode::None;
        self.selection_anchor = None;
        self.count = None;
        self.search_query.clear();
        self.is_searching = false;
        self.pending_g = false;
        self.pending_mark_set = false;
        self.pending_mark_goto = false;
    }

    /// Total number of lines (scrollback + screen)
    pub(crate) fn total_lines(&self) -> usize {
        self.scrollback_len + self.rows
    }

    /// Maximum valid absolute line index
    pub(crate) fn max_line(&self) -> usize {
        self.total_lines().saturating_sub(1)
    }

    /// Push a digit to the count prefix
    pub(crate) fn push_count_digit(&mut self, digit: u8) {
        let current = self.count.unwrap_or(0);
        self.count = Some(current * 10 + digit as usize);
    }

    /// Get the effective count (defaults to 1 if no count set)
    pub(crate) fn effective_count(&mut self) -> usize {
        let c = self.count.unwrap_or(1);
        self.count = None;
        c
    }

    /// Track a resize or scrollback growth: clamps the cursor into the
    /// valid range without moving it off its content.
    pub(crate) fn update_dimensions(&mut self, cols: usize, rows: usize, scrollback_len: usize) {
        self.cols = cols;
        self.rows = rows;
        self.scrollback_len = scrollback_len;
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.cursor_absolute_line = self.cursor_absolute_line.min(self.max_line());
    }

    /// Rebase every absolute-line coordinate by `pushed` — the lines
    /// the pane pushed into scrollback while the mode was open. The
    /// frozen snapshot stays: the cursor rides the screen rows it
    /// occupied at entry and its lines keep reading frozen text.
    pub(crate) fn rebase(&mut self, pushed: usize) {
        if pushed == 0 {
            return;
        }
        self.scrollback_len += pushed;
        self.cursor_absolute_line += pushed;
        if let Some((line, _)) = &mut self.selection_anchor {
            *line += pushed;
        }
        for mark in self.marks.values_mut() {
            mark.absolute_line += pushed;
        }
        self.cursor_absolute_line = self.cursor_absolute_line.min(self.max_line());
    }

    /// The text of absolute line `line`: scrollback lines come from the
    /// live grid (immutable history), screen lines from the frozen
    /// entry snapshot. `grid_line` supplies a scrollback line's text.
    pub(crate) fn line_text(
        &self,
        grid_line: impl Fn(usize) -> Option<String>,
        line: usize,
    ) -> String {
        if line < self.scrollback_len {
            grid_line(line).unwrap_or_default()
        } else {
            self.live_rows
                .get(line - self.scrollback_len)
                .cloned()
                .unwrap_or_default()
        }
    }

    // ========================================================================
    // Marks
    // ========================================================================

    /// Set a named mark at the current cursor position
    pub(crate) fn set_mark(&mut self, name: char) {
        self.marks.insert(
            name,
            Mark {
                col: self.cursor_col,
                absolute_line: self.cursor_absolute_line,
            },
        );
    }

    /// Jump to a named mark, returning true if the mark exists
    pub(crate) fn goto_mark(&mut self, name: char) -> bool {
        if let Some(mark) = self.marks.get(&name) {
            self.cursor_col = mark.col;
            self.cursor_absolute_line = mark.absolute_line;
            true
        } else {
            false
        }
    }

    // ========================================================================
    // Search
    // ========================================================================

    /// Start search input mode
    pub(crate) fn start_search(&mut self, direction: SearchDirection) {
        self.is_searching = true;
        self.search_direction = direction;
        self.search_query.clear();
    }

    /// Cancel search input mode without executing
    pub(crate) fn cancel_search(&mut self) {
        self.is_searching = false;
        self.search_query.clear();
    }
}

// ============================================================================
// Word motions
// ============================================================================
//
// Vim's `w`/`b`/`e` and `W`/`B`/`E` motions differ only in which characters
// belong to a word: alphanumerics plus configured extras for words, anything
// that is not whitespace for WORDs. Each motion is one scanner parameterized
// by that predicate.

fn is_word_char(ch: char, word_chars: &str) -> bool {
    ch.is_alphanumeric() || word_chars.contains(ch)
}

fn is_big_word_char(ch: char) -> bool {
    !ch.is_whitespace()
}

/// Start of the next word after `col`, `count` times.
fn scan_forward(
    chars: &[char],
    mut col: usize,
    count: usize,
    in_word: impl Fn(char) -> bool,
) -> usize {
    for _ in 0..count {
        if col >= chars.len() {
            break;
        }
        // Skip the rest of the current word, then the gap after it.
        while col < chars.len() && in_word(chars[col]) {
            col += 1;
        }
        while col < chars.len() && !in_word(chars[col]) {
            col += 1;
        }
    }
    col
}

/// Start of the previous word before `col`, `count` times.
///
/// `col` must already be clamped below `chars.len()` (or be 0); it only
/// decreases, which keeps every index in bounds.
fn scan_backward(
    chars: &[char],
    mut col: usize,
    count: usize,
    in_word: impl Fn(char) -> bool,
) -> usize {
    for _ in 0..count {
        if col == 0 {
            break;
        }
        col = col.saturating_sub(1);
        // Skip the gap backward, then back up to the word's first character.
        while col > 0 && !in_word(chars[col]) {
            col -= 1;
        }
        while col > 0 && in_word(chars[col - 1]) {
            col -= 1;
        }
    }
    col
}

/// End of the current or next word after `col`, `count` times.
fn scan_end(chars: &[char], mut col: usize, count: usize, in_word: impl Fn(char) -> bool) -> usize {
    let last = chars.len().saturating_sub(1);
    for _ in 0..count {
        if col >= last {
            break;
        }
        col += 1;
        // Skip the gap, then advance to the word's last character.
        while col < chars.len() && !in_word(chars[col]) {
            col += 1;
        }
        while col < last && in_word(chars[col + 1]) {
            col += 1;
        }
    }
    col
}

impl CopyModeState {
    // ========================================================================
    // Basic motions
    // ========================================================================

    /// Move cursor left by count
    pub(crate) fn move_left(&mut self) {
        let count = self.effective_count();
        self.cursor_col = self.cursor_col.saturating_sub(count);
    }

    /// Move cursor right by count
    pub(crate) fn move_right(&mut self) {
        let count = self.effective_count();
        self.cursor_col = (self.cursor_col + count).min(self.cols.saturating_sub(1));
    }

    /// Move cursor up by count
    pub(crate) fn move_up(&mut self) {
        let count = self.effective_count();
        self.cursor_absolute_line = self.cursor_absolute_line.saturating_sub(count);
    }

    /// Move cursor down by count
    pub(crate) fn move_down(&mut self) {
        let count = self.effective_count();
        self.cursor_absolute_line = (self.cursor_absolute_line + count).min(self.max_line());
    }

    /// Move cursor to start of line
    pub(crate) fn move_to_line_start(&mut self) {
        self.cursor_col = 0;
    }

    /// Move cursor to end of line
    pub(crate) fn move_to_line_end(&mut self) {
        self.cursor_col = self.cols.saturating_sub(1);
    }

    /// Move cursor to first non-blank character on the line
    pub(crate) fn move_to_first_non_blank(&mut self, line_text: &str) {
        let first_non_blank = line_text
            .chars()
            .position(|c| !c.is_whitespace())
            .unwrap_or(0);
        self.cursor_col = first_non_blank.min(self.cols.saturating_sub(1));
    }

    // ========================================================================
    // Page motions
    // ========================================================================

    /// Move half page up
    pub(crate) fn half_page_up(&mut self) {
        let half = self.rows / 2;
        let count = self.effective_count();
        self.cursor_absolute_line = self.cursor_absolute_line.saturating_sub(half * count);
    }

    /// Move half page down
    pub(crate) fn half_page_down(&mut self) {
        let half = self.rows / 2;
        let count = self.effective_count();
        self.cursor_absolute_line = (self.cursor_absolute_line + half * count).min(self.max_line());
    }

    /// Move full page up
    pub(crate) fn page_up(&mut self) {
        let count = self.effective_count();
        self.cursor_absolute_line = self.cursor_absolute_line.saturating_sub(self.rows * count);
    }

    /// Move full page down
    pub(crate) fn page_down(&mut self) {
        let count = self.effective_count();
        self.cursor_absolute_line =
            (self.cursor_absolute_line + self.rows * count).min(self.max_line());
    }

    /// Go to top of buffer (line 0)
    pub(crate) fn goto_top(&mut self) {
        self.cursor_absolute_line = 0;
    }

    /// Go to bottom of buffer (last line)
    pub(crate) fn goto_bottom(&mut self) {
        self.cursor_absolute_line = self.max_line();
    }

    /// Go to specific absolute line (for count+G)
    pub(crate) fn goto_line(&mut self, line: usize) {
        self.cursor_absolute_line = line.min(self.max_line());
    }

    // ========================================================================
    // Word motions
    // ========================================================================

    /// Clamp a forward motion's result to the grid.
    fn clamp_to_grid(&self, col: usize) -> usize {
        col.min(self.cols.saturating_sub(1))
    }

    /// Seed column for a backward motion.
    ///
    /// `cursor_col` is clamped to the grid width, which can exceed the
    /// character count when the row holds wide characters or trailing
    /// blanks. The backward scan only decreases its column, so this seed
    /// clamp is what keeps every `chars[..]` access in bounds.
    fn backward_seed(&self, chars: &[char]) -> usize {
        self.cursor_col.min(chars.len().saturating_sub(1))
    }

    /// Move forward to start of next word
    pub(crate) fn move_word_forward(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let col = scan_forward(&chars, self.cursor_col, count, |c| is_word_char(c, ""));
        self.cursor_col = self.clamp_to_grid(col);
    }

    /// Move backward to start of previous word
    pub(crate) fn move_word_backward(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let seed = self.backward_seed(&chars);
        self.cursor_col = scan_backward(&chars, seed, count, |c| is_word_char(c, ""));
    }

    /// Move forward to end of current/next word
    pub(crate) fn move_word_end(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let col = scan_end(&chars, self.cursor_col, count, |c| is_word_char(c, ""));
        self.cursor_col = self.clamp_to_grid(col);
    }

    /// Move forward to start of next WORD (whitespace-delimited)
    pub(crate) fn move_big_word_forward(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let col = scan_forward(&chars, self.cursor_col, count, is_big_word_char);
        self.cursor_col = self.clamp_to_grid(col);
    }

    /// Move backward to start of previous WORD (whitespace-delimited)
    pub(crate) fn move_big_word_backward(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let seed = self.backward_seed(&chars);
        self.cursor_col = scan_backward(&chars, seed, count, is_big_word_char);
    }

    /// Move forward to end of current/next WORD (whitespace-delimited)
    pub(crate) fn move_big_word_end(&mut self, line_text: &str) {
        let count = self.effective_count();
        let chars: Vec<char> = line_text.chars().collect();
        let col = scan_end(&chars, self.cursor_col, count, is_big_word_char);
        self.cursor_col = self.clamp_to_grid(col);
    }

    // ========================================================================
    // Viewport helpers
    // ========================================================================

    /// Get the cursor position in screen coordinates, if visible.
    ///
    /// Returns `(col, row)` where row is relative to the viewport top.
    /// Returns `None` if the cursor is outside the visible viewport.
    /// Calculate the scroll offset needed to make the cursor visible.
    ///
    /// Returns `Some(new_offset)` if scrolling is needed, `None` if the
    /// cursor already sits in the viewport.
    pub(crate) fn required_scroll_offset(&self, current_offset: usize) -> Option<usize> {
        let viewport_top = self.scrollback_len.saturating_sub(current_offset);
        let viewport_bottom = viewport_top + self.rows;
        if self.cursor_absolute_line < viewport_top {
            // Cursor above the viewport: scroll to the buffer top.
            Some(
                self.scrollback_len
                    .saturating_sub(self.cursor_absolute_line),
            )
        } else if self.cursor_absolute_line >= viewport_bottom {
            // Cursor below the viewport: scroll so it sits on the
            // viewport's bottom row.
            let target_viewport_top = self
                .cursor_absolute_line
                .saturating_sub(self.rows.saturating_sub(1));
            Some(
                self.scrollback_len
                    .saturating_sub(target_viewport_top)
                    .min(self.scrollback_len),
            )
        } else {
            None
        }
    }

    /// Enter or leave `mode`, anchoring the selection at the cursor.
    pub(crate) fn toggle_visual(&mut self, mode: VisualMode) {
        if self.visual_mode == mode {
            self.visual_mode = VisualMode::None;
            self.selection_anchor = None;
        } else {
            self.visual_mode = mode;
            self.selection_anchor = Some((self.cursor_absolute_line, self.cursor_col));
        }
    }

    /// Leave visual mode without yanking.
    pub(crate) fn clear_visual(&mut self) {
        self.visual_mode = VisualMode::None;
        self.selection_anchor = None;
    }

    /// The visual selection as a renderer [`Selection`] in absolute
    /// lines — the coordinate painting and text extraction share.
    /// `None` when no visual mode is up.
    pub(crate) fn selection(&self, pane: u32) -> Option<Selection> {
        let (anchor_line, anchor_col) = self.selection_anchor?;
        let head_line = self.cursor_absolute_line;
        let head_col = self.cursor_col;
        let (top, bottom) = if anchor_line <= head_line {
            (anchor_line, head_line)
        } else {
            (head_line, anchor_line)
        };
        let (left, right) = if anchor_col <= head_col {
            (anchor_col, head_col)
        } else {
            (head_col, anchor_col)
        };
        match self.visual_mode {
            VisualMode::None => None,
            VisualMode::Char => Some(Selection::Chars {
                pane,
                anchor: (anchor_line, anchor_col),
                head: (head_line, head_col),
            }),
            VisualMode::Line => Some(Selection::Lines {
                pane,
                a: top,
                b: bottom,
            }),
            VisualMode::Block => Some(Selection::Block {
                pane,
                top,
                bottom,
                left,
                right,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CopyModeState, SearchDirection, VisualMode};

    /// A fresh state entered at scroll position 0 of a 80x24 screen
    /// over `scrollback` history lines.
    fn entered(scrollback: usize) -> CopyModeState {
        let mut cm = CopyModeState::new();
        cm.enter(0, 0, 80, 24, scrollback);
        cm.live_rows = (0..24).map(|r| format!("live-{r:02}")).collect();
        cm
    }

    fn at(col: usize, row: usize, scrollback: usize) -> CopyModeState {
        let mut cm = entered(scrollback);
        cm.cursor_col = col;
        cm.cursor_absolute_line = scrollback + row;
        cm
    }

    /// A CJK row: spacer cells drop, so the text holds fewer characters
    /// than the grid has columns.
    const CJK_LINE: &str = "日本語 test";

    #[test]
    fn motions_clamp_to_the_buffer() {
        let mut cm = entered(10);
        cm.goto_top();
        cm.move_up();
        assert_eq!(cm.cursor_absolute_line, 0, "up from the top clamps");
        cm.goto_bottom();
        assert_eq!(cm.cursor_absolute_line, 10 + 24 - 1, "bottom is max line");
        cm.move_down();
        assert_eq!(cm.cursor_absolute_line, 10 + 23, "down clamps at bottom");
        cm.cursor_col = 79;
        cm.move_right();
        assert_eq!(cm.cursor_col, 79, "right clamps at cols - 1");
        cm.move_left();
        cm.move_left();
        assert_eq!(cm.cursor_col, 77);
    }

    #[test]
    fn counts_scale_motions_and_clear_after_use() {
        let mut cm = entered(100);
        cm.push_count_digit(1);
        cm.push_count_digit(2);
        cm.move_down();
        assert_eq!(cm.cursor_absolute_line, 100 + 12);
        assert_eq!(cm.count, None, "the count cleared");
        cm.move_down();
        assert_eq!(cm.cursor_absolute_line, 100 + 13, "plain j after 12j");
    }

    #[test]
    fn zero_is_line_start_unless_a_count_is_up() {
        let mut cm = at(5, 3, 10);
        cm.move_to_line_start();
        assert_eq!(cm.cursor_col, 0);
        cm.cursor_col = 5;
        cm.push_count_digit(1);
        cm.push_count_digit(0);
        assert_eq!(cm.count, Some(10), "0 rides the count");
    }

    #[test]
    fn goto_line_and_gg() {
        let mut cm = entered(100);
        cm.push_count_digit(4);
        cm.push_count_digit(2);
        let count = cm.effective_count();
        cm.goto_line(count.saturating_sub(1));
        assert_eq!(cm.cursor_absolute_line, 41, "42G is 1-based");
        cm.goto_top();
        assert_eq!(cm.cursor_absolute_line, 0);
        cm.goto_bottom();
        assert_eq!(cm.cursor_absolute_line, cm.max_line());
    }

    #[test]
    fn page_motions_move_by_rows() {
        let mut cm = entered(100);
        cm.page_down();
        assert_eq!(cm.cursor_absolute_line, 123, "clamped at the buffer bottom");
        cm.half_page_up();
        assert_eq!(cm.cursor_absolute_line, 111);
    }

    #[test]
    fn word_motions_scan_both_classes() {
        const LINE: &str = "foo.bar  baz-qux end";
        let cases: [(usize, usize, usize, usize, usize); 4] = [
            // (start, w, e, W, E)
            (0, 4, 2, 9, 6),
            (4, 9, 6, 9, 6),
            (9, 13, 11, 17, 15),
            (17, 20, 19, 20, 19),
        ];
        for (start, word_fwd, word_end, big_fwd, big_end) in cases {
            let mut cm = at(start, 0, 0);
            cm.move_word_forward(LINE);
            assert_eq!(cm.cursor_col, word_fwd, "w from {start}");
            let mut cm = at(start, 0, 0);
            cm.move_word_end(LINE);
            assert_eq!(cm.cursor_col, word_end, "e from {start}");
            let mut cm = at(start, 0, 0);
            cm.move_big_word_forward(LINE);
            assert_eq!(cm.cursor_col, big_fwd, "W from {start}");
            let mut cm = at(start, 0, 0);
            cm.move_big_word_end(LINE);
            assert_eq!(cm.cursor_col, big_end, "E from {start}");
        }
        // Backward from a wide seed column stays in bounds (a row whose
        // text is shorter than the grid width).
        let mut cm = at(79, 0, 0);
        cm.move_word_backward(CJK_LINE);
        assert!(cm.cursor_col < CJK_LINE.chars().count());
        let mut cm = at(79, 0, 0);
        cm.move_big_word_backward("café au lait");
        assert_eq!(cm.cursor_col, 8, "B lands on `lait`");
    }

    #[test]
    fn visual_modes_anchor_extend_and_clear() {
        let mut cm = at(2, 5, 10);
        cm.toggle_visual(VisualMode::Char);
        assert_eq!(cm.visual_mode, VisualMode::Char);
        cm.move_down();
        cm.move_right();
        let sel = cm.selection(1).expect("char selection");
        let super::Selection::Chars { pane, anchor, head } = sel else {
            panic!("char mode paints Chars");
        };
        assert_eq!(pane, 1);
        assert_eq!(anchor, (10 + 5, 2));
        assert_eq!(head, (10 + 6, 3));
        cm.toggle_visual(VisualMode::Char);
        assert_eq!(cm.visual_mode, VisualMode::None);
        assert!(cm.selection(1).is_none(), "cleared clears the selection");

        cm.toggle_visual(VisualMode::Line);
        cm.move_up();
        let super::Selection::Lines { a, b, .. } = cm.selection(1).expect("line selection") else {
            panic!("line mode paints Lines");
        };
        assert_eq!((a, b), (10 + 5, 10 + 6), "line mode spans whole lines");
    }

    #[test]
    fn block_selection_spans_a_column_range() {
        let mut cm = at(4, 2, 10);
        cm.toggle_visual(VisualMode::Block);
        cm.move_down();
        cm.move_right();
        cm.move_right();
        let super::Selection::Block {
            pane: _,
            top,
            bottom,
            left,
            right,
        } = cm.selection(1).expect("block selection")
        else {
            panic!("block mode paints Block");
        };
        assert_eq!((top, bottom), (10 + 2, 10 + 3));
        assert_eq!((left, right), (4, 6));
    }

    #[test]
    fn marks_set_and_jump_and_rebase() {
        let mut cm = entered(10);
        cm.cursor_absolute_line = 12;
        cm.cursor_col = 7;
        cm.set_mark('a');
        cm.goto_bottom();
        assert!(cm.goto_mark('a'), "the mark exists");
        assert_eq!((cm.cursor_absolute_line, cm.cursor_col), (12, 7));
        assert!(!cm.goto_mark('z'), "unset marks refuse");

        cm.rebase(3);
        assert_eq!(cm.cursor_absolute_line, 15, "the jump target rebased");
        assert_eq!(cm.marks[&'a'].absolute_line, 15);
    }

    #[test]
    fn rebase_moves_cursor_anchor_and_scroll_length() {
        let mut cm = at(3, 5, 10);
        cm.toggle_visual(VisualMode::Char);
        cm.move_down();
        cm.rebase(4);
        assert_eq!(cm.scrollback_len, 14);
        assert_eq!(cm.cursor_absolute_line, 16 + 4, "cursor rode the push");
        assert_eq!(
            cm.selection_anchor,
            Some((15 + 4, 3)),
            "the anchor rode the push"
        );
    }

    #[test]
    fn line_text_reads_grid_below_and_snapshot_at_or_above() {
        let mut cm = entered(3);
        cm.live_rows = vec!["screen-0".into(), "screen-1".into()];
        let grid_line = |line: usize| Some(format!("hist-{line}"));
        assert_eq!(cm.line_text(grid_line, 1), "hist-1");
        assert_eq!(cm.line_text(grid_line, 3), "screen-0", "first live row");
        assert_eq!(cm.line_text(grid_line, 4), "screen-1");
        assert_eq!(cm.line_text(grid_line, 9), "", "past the screen is blank");
    }

    #[test]
    fn search_state_round_trips() {
        let mut cm = entered(5);
        cm.start_search(SearchDirection::Backward);
        assert!(cm.is_searching);
        cm.cancel_search();
        assert!(!cm.is_searching);
        assert_eq!(cm.search_query, "");
    }

    #[test]
    fn first_non_blank_skips_leading_blanks() {
        let mut cm = entered(0);
        cm.move_to_first_non_blank("    indented");
        assert_eq!(cm.cursor_col, 4);
        cm.move_to_first_non_blank("   ");
        assert_eq!(cm.cursor_col, 0, "all-blank lands at 0");
    }
}
