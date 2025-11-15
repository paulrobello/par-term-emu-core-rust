use crate::cell::Cell;

/// A 2D grid of terminal cells
#[derive(Debug, Clone)]
pub struct Grid {
    /// Number of columns
    cols: usize,
    /// Number of rows
    rows: usize,
    /// The actual grid data (row-major order)
    cells: Vec<Cell>,
    /// Scrollback buffer (flat Vec, row-major order like main grid)
    /// Uses circular buffer indexing when full
    scrollback_cells: Vec<Cell>,
    /// Index of oldest line in circular scrollback buffer
    scrollback_start: usize,
    /// Number of lines currently in scrollback
    scrollback_lines: usize,
    /// Maximum scrollback lines
    max_scrollback: usize,
    /// Track which lines are wrapped (true = line continues to next row)
    /// Index corresponds to row number. If wrapped[i] == true, row i wraps to row i+1
    wrapped: Vec<bool>,
    /// Track wrapped state for scrollback lines (circular buffer)
    scrollback_wrapped: Vec<bool>,
}

impl Grid {
    /// Create a new grid with the specified dimensions
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let cells = vec![Cell::default(); cols * rows];
        Self {
            cols,
            rows,
            cells,
            scrollback_cells: Vec::new(),
            scrollback_start: 0,
            scrollback_lines: 0,
            max_scrollback,
            wrapped: vec![false; rows],
            scrollback_wrapped: Vec::new(),
        }
    }

    /// Get the number of columns
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get the number of rows
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Get a reference to a cell at (col, row)
    pub fn get(&self, col: usize, row: usize) -> Option<&Cell> {
        if col < self.cols && row < self.rows {
            Some(&self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Get a mutable reference to a cell at (col, row)
    pub fn get_mut(&mut self, col: usize, row: usize) -> Option<&mut Cell> {
        if col < self.cols && row < self.rows {
            Some(&mut self.cells[row * self.cols + col])
        } else {
            None
        }
    }

    /// Set a cell at (col, row)
    pub fn set(&mut self, col: usize, row: usize, cell: Cell) {
        if let Some(c) = self.get_mut(col, row) {
            *c = cell;
        }
    }

    /// Get a row as a slice
    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        if row < self.rows {
            let start = row * self.cols;
            let end = start + self.cols;
            Some(&self.cells[start..end])
        } else {
            None
        }
    }

    /// Get a mutable row
    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        if row < self.rows {
            let start = row * self.cols;
            let end = start + self.cols;
            Some(&mut self.cells[start..end])
        } else {
            None
        }
    }

    /// Get the text content of a row (for text shaping)
    pub fn row_text(&self, row: usize) -> String {
        if let Some(cells) = self.row(row) {
            cells
                .iter()
                .filter(|cell| !cell.flags.wide_char_spacer())
                .map(|cell| cell.c)
                .collect()
        } else {
            String::new()
        }
    }

    /// Clear the entire grid
    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
    }

    /// Clear a specific row
    pub fn clear_row(&mut self, row: usize) {
        if let Some(row_cells) = self.row_mut(row) {
            row_cells.fill(Cell::default());
        }
    }

    /// Clear from cursor to end of line
    pub fn clear_line_right(&mut self, col: usize, row: usize) {
        if row < self.rows {
            for c in col..self.cols {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Clear from beginning of line to cursor
    pub fn clear_line_left(&mut self, col: usize, row: usize) {
        if row < self.rows {
            for c in 0..=col.min(self.cols - 1) {
                if let Some(cell) = self.get_mut(c, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Clear from cursor to end of screen
    pub fn clear_screen_below(&mut self, col: usize, row: usize) {
        self.clear_line_right(col, row);
        for r in (row + 1)..self.rows {
            self.clear_row(r);
        }
    }

    /// Clear from beginning of screen to cursor
    pub fn clear_screen_above(&mut self, col: usize, row: usize) {
        for r in 0..row {
            self.clear_row(r);
        }
        self.clear_line_left(col, row);
    }

    /// Scroll up by n lines (moves content up, adds blank lines at bottom)
    pub fn scroll_up(&mut self, n: usize) {
        let n = n.min(self.rows);

        // Save scrolled lines to scrollback (only if scrollback is enabled)
        if self.max_scrollback > 0 {
            for i in 0..n {
                // Calculate source indices directly to avoid temporary allocation
                let src_start = i * self.cols;
                let src_end = src_start + self.cols;
                let is_wrapped = self.wrapped.get(i).copied().unwrap_or(false);

                if self.scrollback_lines < self.max_scrollback {
                    // Scrollback not full yet - append normally
                    self.scrollback_cells
                        .extend_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped.push(is_wrapped);
                    self.scrollback_lines += 1;
                } else {
                    // Scrollback is full - use circular buffer (overwrite oldest line)
                    let write_idx = self.scrollback_start;
                    let dst_start = write_idx * self.cols;
                    let dst_end = dst_start + self.cols;

                    // Overwrite the oldest line in the circular buffer
                    self.scrollback_cells[dst_start..dst_end]
                        .copy_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped[write_idx] = is_wrapped;

                    // Advance start pointer (circular)
                    self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
                }
            }
        }

        // Move lines up
        for i in n..self.rows {
            let src_start = i * self.cols;
            let dst_start = (i - n) * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
            // Move wrapped state
            if i < self.wrapped.len() && (i - n) < self.wrapped.len() {
                self.wrapped[i - n] = self.wrapped[i];
            }
        }

        // Clear bottom lines
        for i in (self.rows - n)..self.rows {
            self.clear_row(i);
            if i < self.wrapped.len() {
                self.wrapped[i] = false;
            }
        }
    }

    /// Scroll down by n lines (moves content down, adds blank lines at top)
    pub fn scroll_down(&mut self, n: usize) {
        let n = n.min(self.rows);

        // Move lines down
        for i in (n..self.rows).rev() {
            let src_start = (i - n) * self.cols;
            let dst_start = i * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
            // Move wrapped state
            if (i - n) < self.wrapped.len() && i < self.wrapped.len() {
                self.wrapped[i] = self.wrapped[i - n];
            }
        }

        // Clear top lines
        for i in 0..n {
            self.clear_row(i);
            if i < self.wrapped.len() {
                self.wrapped[i] = false;
            }
        }
    }

    /// Resize the grid
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }

        let mut new_cells = vec![Cell::default(); cols * rows];
        let mut new_wrapped = vec![false; rows];

        // Copy existing content
        let min_rows = self.rows.min(rows);
        let min_cols = self.cols.min(cols);

        for row in 0..min_rows {
            for col in 0..min_cols {
                if let Some(cell) = self.get(col, row) {
                    new_cells[row * cols + col] = *cell;
                }
            }
            // Copy wrapped state
            if row < self.wrapped.len() {
                new_wrapped[row] = self.wrapped[row];
            }
        }

        self.cols = cols;
        self.rows = rows;
        self.cells = new_cells;
        self.wrapped = new_wrapped;
    }

    /// Get scrollback buffer (returns a temporary Vec<Vec<Cell>> for API compatibility)
    ///
    /// **Note:** This creates temporary allocations. Prefer using `scrollback_line()` for
    /// efficient line-by-line access, or iterate with `(0..self.scrollback_len()).filter_map(|i| self.scrollback_line(i))`.
    ///
    /// This method is kept for API compatibility but is not used internally.
    pub fn scrollback(&self) -> Vec<Vec<Cell>> {
        let mut result = Vec::with_capacity(self.scrollback_lines);
        for line_idx in 0..self.scrollback_lines {
            if let Some(line) = self.scrollback_line(line_idx) {
                result.push(line.to_vec());
            }
        }
        result
    }

    /// Get a line from scrollback
    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        if index < self.scrollback_lines {
            // Calculate physical index in circular buffer
            let physical_index = if self.scrollback_lines < self.max_scrollback {
                // Buffer not full - use direct indexing
                index
            } else {
                // Buffer is full - use circular indexing
                (self.scrollback_start + index) % self.max_scrollback
            };
            let start = physical_index * self.cols;
            let end = start + self.cols;
            Some(&self.scrollback_cells[start..end])
        } else {
            None
        }
    }

    /// Get the number of scrollback lines
    pub fn scrollback_len(&self) -> usize {
        self.scrollback_lines
    }

    /// Get the maximum scrollback capacity
    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    /// Check if a line is wrapped (continues to next line)
    pub fn is_line_wrapped(&self, row: usize) -> bool {
        self.wrapped.get(row).copied().unwrap_or(false)
    }

    /// Set the wrapped state for a line
    pub fn set_line_wrapped(&mut self, row: usize, wrapped: bool) {
        if row < self.wrapped.len() {
            self.wrapped[row] = wrapped;
        }
    }

    /// Check if a scrollback line is wrapped
    pub fn is_scrollback_wrapped(&self, index: usize) -> bool {
        if index < self.scrollback_lines {
            // Calculate physical index in circular buffer
            let physical_index = if self.scrollback_lines < self.max_scrollback {
                index
            } else {
                (self.scrollback_start + index) % self.max_scrollback
            };
            self.scrollback_wrapped
                .get(physical_index)
                .copied()
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Convert grid to string representation
    pub fn content_as_string(&self) -> String {
        // Pre-allocate based on grid size
        let estimated_size = self.rows * (self.cols + 1);
        let mut result = String::with_capacity(estimated_size);
        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                for cell in row_cells {
                    result.push(cell.c);
                }
                result.push('\n');
            }
        }
        result
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    ///
    /// This exports all buffer contents with:
    /// - No styling, colors, or graphics
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Empty lines preserved
    pub fn export_text_buffer(&self) -> String {
        // Pre-allocate based on estimated size (cols + newline per line)
        let estimated_size = (self.scrollback_lines + self.rows) * (self.cols + 1);
        let mut result = String::with_capacity(estimated_size);

        // Export scrollback buffer first
        for line_idx in 0..self.scrollback_lines {
            if let Some(line_cells) = self.scrollback_line(line_idx) {
                // Extract characters, filtering out wide char spacers
                let mut line_text = String::new();
                for cell in line_cells {
                    // Skip wide char spacers (they're just placeholders for the second cell of wide chars)
                    if !cell.flags.wide_char_spacer() {
                        line_text.push(cell.c);
                    }
                }

                // Trim trailing spaces but preserve leading spaces
                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                // Only add newline if this line is NOT wrapped to the next
                if !self.is_scrollback_wrapped(line_idx) {
                    result.push('\n');
                }
            }
        }

        // Export current screen
        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                // Extract characters, filtering out wide char spacers
                let mut line_text = String::new();
                for cell in row_cells {
                    // Skip wide char spacers
                    if !cell.flags.wide_char_spacer() {
                        line_text.push(cell.c);
                    }
                }

                // Trim trailing spaces but preserve leading spaces
                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                // Only add newline if this is not the last row OR if the line is not wrapped
                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push('\n');
                    }
                } else {
                    // For the last row, add newline only if there's content
                    if !trimmed.is_empty() {
                        result.push('\n');
                    }
                }
            }
        }

        result
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    ///
    /// This exports all buffer contents with:
    /// - Full ANSI escape sequences for colors and text attributes
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Efficient escape sequence generation (only emits changes)
    pub fn export_styled_buffer(&self) -> String {
        use crate::color::{Color, NamedColor};

        // Pre-allocate based on estimated size (chars + ANSI sequences)
        // Estimate ~20 bytes per char for styled output (text + escape codes)
        let estimated_size = (self.scrollback_lines + self.rows) * self.cols * 20;
        let mut result = String::with_capacity(estimated_size);
        let mut current_fg = Color::Named(NamedColor::White);
        let mut current_bg = Color::Named(NamedColor::Black);
        let mut current_flags = crate::cell::CellFlags::default();

        // Helper to emit SGR sequence for color changes
        let emit_style =
            |result: &mut String, fg: &Color, bg: &Color, flags: &crate::cell::CellFlags| {
                result.push_str("\x1b[0"); // Reset

                // Set foreground color
                match fg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 30,
                            NamedColor::Red => 31,
                            NamedColor::Green => 32,
                            NamedColor::Yellow => 33,
                            NamedColor::Blue => 34,
                            NamedColor::Magenta => 35,
                            NamedColor::Cyan => 36,
                            NamedColor::White => 37,
                            NamedColor::BrightBlack => 90,
                            NamedColor::BrightRed => 91,
                            NamedColor::BrightGreen => 92,
                            NamedColor::BrightYellow => 93,
                            NamedColor::BrightBlue => 94,
                            NamedColor::BrightMagenta => 95,
                            NamedColor::BrightCyan => 96,
                            NamedColor::BrightWhite => 97,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => {
                        result.push_str(&format!(";38;5;{}", i));
                    }
                    Color::Rgb(r, g, b) => {
                        result.push_str(&format!(";38;2;{};{};{}", r, g, b));
                    }
                }

                // Set background color
                match bg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 40,
                            NamedColor::Red => 41,
                            NamedColor::Green => 42,
                            NamedColor::Yellow => 43,
                            NamedColor::Blue => 44,
                            NamedColor::Magenta => 45,
                            NamedColor::Cyan => 46,
                            NamedColor::White => 47,
                            NamedColor::BrightBlack => 100,
                            NamedColor::BrightRed => 101,
                            NamedColor::BrightGreen => 102,
                            NamedColor::BrightYellow => 103,
                            NamedColor::BrightBlue => 104,
                            NamedColor::BrightMagenta => 105,
                            NamedColor::BrightCyan => 106,
                            NamedColor::BrightWhite => 107,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => {
                        result.push_str(&format!(";48;5;{}", i));
                    }
                    Color::Rgb(r, g, b) => {
                        result.push_str(&format!(";48;2;{};{};{}", r, g, b));
                    }
                }

                // Set text attributes
                if flags.bold() {
                    result.push_str(";1");
                }
                if flags.dim() {
                    result.push_str(";2");
                }
                if flags.italic() {
                    result.push_str(";3");
                }
                if flags.underline() {
                    result.push_str(";4");
                }
                if flags.blink() {
                    result.push_str(";5");
                }
                if flags.reverse() {
                    result.push_str(";7");
                }
                if flags.hidden() {
                    result.push_str(";8");
                }
                if flags.strikethrough() {
                    result.push_str(";9");
                }

                result.push('m');
            };

        // Export scrollback buffer first
        for line_idx in 0..self.scrollback_lines {
            if let Some(line_cells) = self.scrollback_line(line_idx) {
                let mut line_text = String::new();

                for cell in line_cells {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }

                    // Check if style changed
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut line_text, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }

                    line_text.push(cell.c);
                }

                // Trim trailing spaces
                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                // Reset style at end of line
                if !trimmed.is_empty() {
                    result.push_str("\x1b[0m");
                    current_fg = Color::Named(NamedColor::White);
                    current_bg = Color::Named(NamedColor::Black);
                    current_flags = crate::cell::CellFlags::default();
                }

                if !self.is_scrollback_wrapped(line_idx) {
                    result.push('\n');
                }
            }
        }

        // Export current screen
        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                let mut line_text = String::new();

                for cell in row_cells {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }

                    // Check if style changed
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut line_text, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }

                    line_text.push(cell.c);
                }

                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                // Reset style at end of line if there's content
                if !trimmed.is_empty() {
                    result.push_str("\x1b[0m");
                    current_fg = Color::Named(NamedColor::White);
                    current_bg = Color::Named(NamedColor::Black);
                    current_flags = crate::cell::CellFlags::default();
                }

                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push('\n');
                    }
                } else if !trimmed.is_empty() {
                    result.push('\n');
                }
            }
        }

        result
    }

    /// Insert n blank lines at row, shifting lines below down (VT220 IL)
    /// Lines that are pushed off the bottom are lost
    pub fn insert_lines(&mut self, row: usize, n: usize, scroll_top: usize, scroll_bottom: usize) {
        if row >= self.rows || row < scroll_top || row > scroll_bottom {
            return;
        }

        let n = n.min(scroll_bottom - row + 1);
        let effective_bottom = scroll_bottom.min(self.rows - 1);

        // Prevent underflow when n > effective_bottom
        if n > effective_bottom {
            // Just clear all lines from row to effective_bottom
            for i in row..=effective_bottom {
                self.clear_row(i);
            }
            return;
        }

        // Move lines down from row to scroll_bottom - n
        for i in (row..=(effective_bottom - n)).rev() {
            let src_start = i * self.cols;
            let dst_start = (i + n) * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
        }

        // Clear the newly inserted lines
        for i in row..(row + n).min(self.rows) {
            self.clear_row(i);
        }
    }

    /// Delete n lines at row, shifting lines below up (VT220 DL)
    /// Blank lines are added at the bottom
    pub fn delete_lines(&mut self, row: usize, n: usize, scroll_top: usize, scroll_bottom: usize) {
        if row >= self.rows || row < scroll_top || row > scroll_bottom {
            return;
        }

        let n = n.min(scroll_bottom - row + 1);
        let effective_bottom = scroll_bottom.min(self.rows - 1);

        // Move lines up from row + n to scroll_bottom
        for i in row..=(effective_bottom.saturating_sub(n)) {
            let src_start = (i + n) * self.cols;
            let dst_start = i * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
        }

        // Clear the lines at the bottom - use saturating_sub to prevent underflow
        let clear_start = effective_bottom.saturating_sub(n - 1);
        for i in clear_start..=effective_bottom {
            if i < self.rows {
                self.clear_row(i);
            }
        }
    }

    /// Insert n blank characters at position, shifting characters right (VT220 ICH)
    /// Characters that are pushed off the right edge are lost
    pub fn insert_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }

        let n = n.min(self.cols - col);
        let cols = self.cols;

        // Move characters right from col to cols - n - 1
        if let Some(row_cells) = self.row_mut(row) {
            for i in ((col + n)..cols).rev() {
                row_cells[i] = row_cells[i - n];
            }

            // Clear the inserted characters
            for cell in row_cells.iter_mut().skip(col).take(n) {
                cell.reset();
            }
        }
    }

    /// Delete n characters at position, shifting characters left (VT220 DCH)
    /// Blank characters are added at the right edge
    pub fn delete_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }

        let n = n.min(self.cols - col);
        let cols = self.cols;

        if let Some(row_cells) = self.row_mut(row) {
            // Move characters left from col + n to cols - 1
            for i in col..(cols - n) {
                row_cells[i] = row_cells[i + n];
            }

            // Clear the characters at the end
            for cell in row_cells.iter_mut().skip(cols - n).take(n) {
                cell.reset();
            }
        }
    }

    /// Erase n characters at position (VT220 ECH)
    /// Replaces characters with spaces, does not shift
    pub fn erase_chars(&mut self, col: usize, row: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }

        let n = n.min(self.cols - col);

        if let Some(row_cells) = self.row_mut(row) {
            for cell in row_cells.iter_mut().skip(col).take(n) {
                cell.reset();
            }
        }
    }

    /// Scroll up within a region (for DECSTBM)
    pub fn scroll_region_up(&mut self, n: usize, top: usize, bottom: usize) {
        if top >= self.rows || bottom >= self.rows || top > bottom {
            return;
        }

        let n = n.min(bottom - top + 1);
        let effective_bottom = bottom.min(self.rows - 1);

        // If n is larger than or equal to region size, just clear the region
        if n > effective_bottom - top {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            return;
        }

        // If scrolling the entire screen (top=0, bottom=rows-1), save to scrollback
        // Only save to scrollback if max_scrollback > 0 (alternate screen has no scrollback)
        if top == 0 && effective_bottom == self.rows - 1 && self.max_scrollback > 0 {
            for i in 0..n {
                // Calculate source indices directly to avoid temporary allocation
                let src_start = i * self.cols;
                let src_end = src_start + self.cols;
                let is_wrapped = self.wrapped.get(i).copied().unwrap_or(false);

                if self.scrollback_lines < self.max_scrollback {
                    // Scrollback not full yet - append normally
                    self.scrollback_cells
                        .extend_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped.push(is_wrapped);
                    self.scrollback_lines += 1;
                } else {
                    // Scrollback is full - use circular buffer (overwrite oldest line)
                    let write_idx = self.scrollback_start;
                    let dst_start = write_idx * self.cols;
                    let dst_end = dst_start + self.cols;

                    // Overwrite the oldest line in the circular buffer
                    self.scrollback_cells[dst_start..dst_end]
                        .copy_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped[write_idx] = is_wrapped;

                    // Advance start pointer (circular)
                    self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
                }
            }
        }

        // Move lines up within the region
        for i in top..=(effective_bottom - n) {
            let src_start = (i + n) * self.cols;
            let dst_start = i * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
        }

        // Clear bottom lines in the region
        for i in (effective_bottom - n + 1)..=effective_bottom {
            if i < self.rows {
                self.clear_row(i);
            }
        }
    }

    /// Scroll down within a region (for DECSTBM)
    pub fn scroll_region_down(&mut self, n: usize, top: usize, bottom: usize) {
        if top >= self.rows || bottom >= self.rows || top > bottom {
            return;
        }

        let n = n.min(bottom - top + 1);
        let effective_bottom = bottom.min(self.rows - 1);

        // If n is larger than or equal to region size, just clear the region
        if n > effective_bottom - top {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            return;
        }

        // Move lines down within the region
        for i in ((top + n)..=effective_bottom).rev() {
            let src_start = (i - n) * self.cols;
            let dst_start = i * self.cols;
            let src_end = src_start + self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
        }

        // Clear top lines in the region
        for i in top..(top + n).min(self.rows) {
            self.clear_row(i);
        }
    }

    /// Fill a rectangular area with a character (DECFRA - VT420)
    ///
    /// Fills the rectangle defined by (left, top) to (right, bottom) with the given cell.
    /// Coordinates are 0-indexed and inclusive.
    pub fn fill_rectangle(
        &mut self,
        fill_cell: Cell,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        // Validate and clamp coordinates
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);

        if top > bottom || left > right {
            return;
        }

        // Fill the rectangle
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    *cell = fill_cell;
                }
            }
        }
    }

    /// Copy a rectangular area to another location (DECCRA - VT420)
    ///
    /// Copies the rectangle from (src_left, src_top) to (src_right, src_bottom)
    /// to destination starting at (dst_left, dst_top).
    /// Coordinates are 0-indexed and inclusive.
    pub fn copy_rectangle(
        &mut self,
        src_top: usize,
        src_left: usize,
        src_bottom: usize,
        src_right: usize,
        dst_top: usize,
        dst_left: usize,
    ) {
        // Validate source coordinates
        if src_top >= self.rows || src_left >= self.cols {
            return;
        }
        let src_bottom = src_bottom.min(self.rows - 1);
        let src_right = src_right.min(self.cols - 1);

        if src_top > src_bottom || src_left > src_right {
            return;
        }

        // Calculate dimensions
        let height = src_bottom - src_top + 1;
        let width = src_right - src_left + 1;

        // Validate destination fits
        if dst_top >= self.rows || dst_left >= self.cols {
            return;
        }
        let dst_bottom = (dst_top + height - 1).min(self.rows - 1);
        let dst_right = (dst_left + width - 1).min(self.cols - 1);

        // Copy to temporary buffer first to handle overlapping rectangles
        // Pre-allocate with exact capacity to avoid reallocations
        let capacity = height * width;
        let mut buffer = Vec::with_capacity(capacity);
        for row in src_top..=src_bottom {
            for col in src_left..=src_right {
                if let Some(cell) = self.get(col, row) {
                    buffer.push(*cell);
                }
            }
        }

        // Copy from buffer to destination
        let mut buffer_idx = 0;
        for row in dst_top..=dst_bottom {
            for col in dst_left..=dst_right {
                if buffer_idx < buffer.len() {
                    if let Some(cell) = self.get_mut(col, row) {
                        *cell = buffer[buffer_idx];
                    }
                    buffer_idx += 1;
                }
            }
        }
    }

    /// Erase a rectangular area selectively (DECSERA - VT420)
    ///
    /// Erases (clears to space) the rectangle defined by (left, top) to (right, bottom).
    /// Coordinates are 0-indexed and inclusive.
    /// This is "selective erase" which preserves protected/guarded characters (DECSCA).
    pub fn erase_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        // Validate and clamp coordinates
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);

        if top > bottom || left > right {
            return;
        }

        // Selectively erase the rectangle (skip guarded/protected cells)
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    // DECSERA: Only erase cells that are NOT protected/guarded
                    if !cell.flags.guarded() {
                        cell.reset();
                    }
                }
            }
        }
    }

    /// Erase a rectangular area unconditionally (DECERA - VT420)
    ///
    /// Erases (clears to space) the rectangle defined by (left, top) to (right, bottom).
    /// Coordinates are 0-indexed and inclusive.
    /// Unlike DECSERA, this does NOT respect character protection (guarded flag).
    pub fn erase_rectangle_unconditional(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        // Validate and clamp coordinates
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);

        if top > bottom || left > right {
            return;
        }

        // Unconditionally erase the rectangle (ignores guarded flag)
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    cell.reset();
                }
            }
        }
    }

    /// Change attributes in rectangular area (DECCARA - VT420)
    ///
    /// Sets the specified SGR attributes for all cells in the rectangle.
    /// Coordinates are 0-indexed and inclusive.
    /// Valid attributes: 0 (reset), 1 (bold), 4 (underline), 5 (blink), 7 (reverse), 8 (hidden)
    pub fn change_attributes_in_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        attributes: &[u16],
    ) {
        // Validate and clamp coordinates
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);

        if top > bottom || left > right {
            return;
        }

        // Apply attributes to all cells in rectangle
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    for &attr in attributes {
                        match attr {
                            0 => {
                                // Reset all attributes (but keep character and colors)
                                cell.flags.set_bold(false);
                                cell.flags.set_dim(false);
                                cell.flags.set_italic(false);
                                cell.flags.set_underline(false);
                                cell.flags.set_blink(false);
                                cell.flags.set_reverse(false);
                                cell.flags.set_hidden(false);
                                cell.flags.set_strikethrough(false);
                            }
                            1 => cell.flags.set_bold(true),
                            2 => cell.flags.set_dim(true),
                            3 => cell.flags.set_italic(true),
                            4 => cell.flags.set_underline(true),
                            5 => cell.flags.set_blink(true),
                            7 => cell.flags.set_reverse(true),
                            8 => cell.flags.set_hidden(true),
                            9 => cell.flags.set_strikethrough(true),
                            22 => {
                                cell.flags.set_bold(false);
                                cell.flags.set_dim(false);
                            }
                            23 => cell.flags.set_italic(false),
                            24 => cell.flags.set_underline(false),
                            25 => cell.flags.set_blink(false),
                            27 => cell.flags.set_reverse(false),
                            28 => cell.flags.set_hidden(false),
                            29 => cell.flags.set_strikethrough(false),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Reverse attributes in rectangular area (DECRARA - VT420)
    ///
    /// Reverses (toggles) the specified SGR attributes for all cells in the rectangle.
    /// Coordinates are 0-indexed and inclusive.
    /// Valid attributes: 1 (bold), 4 (underline), 5 (blink), 7 (reverse)
    pub fn reverse_attributes_in_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        attributes: &[u16],
    ) {
        // Validate and clamp coordinates
        if top >= self.rows || left >= self.cols {
            return;
        }
        let bottom = bottom.min(self.rows - 1);
        let right = right.min(self.cols - 1);

        if top > bottom || left > right {
            return;
        }

        // Reverse attributes in all cells in rectangle
        for row in top..=bottom {
            for col in left..=right {
                if let Some(cell) = self.get_mut(col, row) {
                    for &attr in attributes {
                        match attr {
                            0 => {
                                // Reverse all standard attributes
                                cell.flags.set_bold(!cell.flags.bold());
                                cell.flags.set_underline(!cell.flags.underline());
                                cell.flags.set_blink(!cell.flags.blink());
                                cell.flags.set_reverse(!cell.flags.reverse());
                            }
                            1 => cell.flags.set_bold(!cell.flags.bold()),
                            4 => cell.flags.set_underline(!cell.flags.underline()),
                            5 => cell.flags.set_blink(!cell.flags.blink()),
                            7 => cell.flags.set_reverse(!cell.flags.reverse()),
                            8 => cell.flags.set_hidden(!cell.flags.hidden()), // xterm extension
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Generate a debug snapshot of the grid for logging
    pub fn debug_snapshot(&self) -> String {
        use std::fmt::Write;
        // Pre-allocate based on estimated debug output size
        let estimated_size = (self.rows + self.scrollback_lines.min(6)) * (self.cols + 20);
        let mut output = String::with_capacity(estimated_size);

        // Header with dimensions
        writeln!(
            output,
            "Grid: {}x{} (scrollback: {}/{})",
            self.cols, self.rows, self.scrollback_lines, self.max_scrollback
        )
        .unwrap();
        writeln!(output, "{}", "─".repeat(self.cols.min(80))).unwrap();

        // Content
        for row in 0..self.rows {
            let line: String = (0..self.cols)
                .map(|col| {
                    if let Some(cell) = self.get(col, row) {
                        if cell.c == '\0' || cell.c == ' ' {
                            ' '
                        } else {
                            cell.c
                        }
                    } else {
                        '?'
                    }
                })
                .collect();
            writeln!(output, "{:3}: |{}|", row, line).unwrap();
        }

        // Scrollback summary
        if self.scrollback_lines > 0 {
            writeln!(output, "{}", "─".repeat(self.cols.min(80))).unwrap();
            writeln!(output, "Scrollback: {} lines", self.scrollback_lines).unwrap();
            // Show first and last few lines of scrollback
            for i in 0..3.min(self.scrollback_lines) {
                if let Some(line) = self.scrollback_line(i) {
                    let line_str: String = line
                        .iter()
                        .map(|cell| {
                            if cell.c == '\0' || cell.c == ' ' {
                                ' '
                            } else {
                                cell.c
                            }
                        })
                        .collect();
                    writeln!(output, "S{:3}: |{}|", i, line_str).unwrap();
                }
            }
            if self.scrollback_lines > 6 {
                writeln!(output, "  ... ({} more lines)", self.scrollback_lines - 6).unwrap();
            }
            let start = self.scrollback_lines.saturating_sub(3);
            for i in start..self.scrollback_lines {
                if let Some(line) = self.scrollback_line(i) {
                    let line_str: String = line
                        .iter()
                        .map(|cell| {
                            if cell.c == '\0' || cell.c == ' ' {
                                ' '
                            } else {
                                cell.c
                            }
                        })
                        .collect();
                    writeln!(output, "S{:3}: |{}|", i, line_str).unwrap();
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_creation() {
        let grid = Grid::new(80, 24, 1000);
        assert_eq!(grid.cols(), 80);
        assert_eq!(grid.rows(), 24);
    }

    #[test]
    fn test_grid_set_get() {
        let mut grid = Grid::new(80, 24, 1000);
        let cell = Cell::new('A');
        grid.set(5, 10, cell);

        let retrieved = grid.get(5, 10).unwrap();
        assert_eq!(retrieved.c, 'A');
    }

    #[test]
    fn test_grid_clear() {
        let mut grid = Grid::new(80, 24, 1000);
        grid.set(5, 10, Cell::new('A'));
        grid.clear();

        let cell = grid.get(5, 10).unwrap();
        assert_eq!(cell.c, ' ');
    }

    #[test]
    fn test_grid_scroll() {
        let mut grid = Grid::new(80, 24, 1000);
        grid.set(0, 0, Cell::new('A'));
        grid.set(0, 1, Cell::new('B'));

        grid.scroll_up(1);

        assert_eq!(grid.get(0, 0).unwrap().c, 'B');
        assert_eq!(grid.scrollback_len(), 1);
    }

    #[test]
    fn test_grid_resize() {
        let mut grid = Grid::new(80, 24, 1000);
        grid.set(5, 5, Cell::new('X'));

        grid.resize(100, 30);
        assert_eq!(grid.cols(), 100);
        assert_eq!(grid.rows(), 30);
        assert_eq!(grid.get(5, 5).unwrap().c, 'X');
    }

    #[test]
    fn test_scroll_region_up() {
        let mut grid = Grid::new(80, 10, 1000);
        for i in 0..10 {
            grid.set(0, i, Cell::new((b'0' + i as u8) as char));
        }

        grid.scroll_region_up(2, 2, 7); // Scroll lines 2-7 up by 2

        // Line 2 should now contain what was at line 4
        assert_eq!(grid.get(0, 2).unwrap().c, '4');
        // Lines 6-7 should be blank
        assert_eq!(grid.get(0, 6).unwrap().c, ' ');
        assert_eq!(grid.get(0, 7).unwrap().c, ' ');
    }

    #[test]
    fn test_scroll_region_down() {
        let mut grid = Grid::new(80, 10, 1000);
        for i in 0..10 {
            grid.set(0, i, Cell::new((b'0' + i as u8) as char));
        }

        grid.scroll_region_down(2, 2, 7); // Scroll lines 2-7 down by 2

        // Line 4 should now contain what was at line 2
        assert_eq!(grid.get(0, 4).unwrap().c, '2');
        // Lines 2-3 should be blank
        assert_eq!(grid.get(0, 2).unwrap().c, ' ');
        assert_eq!(grid.get(0, 3).unwrap().c, ' ');
    }

    #[test]
    fn test_insert_lines_edge_case() {
        let mut grid = Grid::new(80, 10, 1000);
        for i in 0..10 {
            grid.set(0, i, Cell::new((b'A' + i as u8) as char));
        }

        // Insert at bottom of scroll region
        grid.insert_lines(7, 2, 0, 9);

        assert_eq!(grid.get(0, 7).unwrap().c, ' '); // Should be blank
        assert_eq!(grid.get(0, 8).unwrap().c, ' '); // Should be blank
    }

    #[test]
    fn test_delete_lines_edge_case() {
        let mut grid = Grid::new(80, 10, 1000);
        for i in 0..10 {
            grid.set(0, i, Cell::new((b'A' + i as u8) as char));
        }

        // Delete from near bottom (delete 2 lines starting at row 7)
        // Row 7 has 'H', row 8 has 'I', row 9 has 'J'
        // After deleting rows 7 and 8, row 9 moves to row 7
        grid.delete_lines(7, 2, 0, 9);

        assert_eq!(grid.get(0, 7).unwrap().c, 'J'); // Line 9 moves to 7
        assert_eq!(grid.get(0, 8).unwrap().c, ' '); // Should be blank
        assert_eq!(grid.get(0, 9).unwrap().c, ' '); // Should be blank
    }

    #[test]
    fn test_insert_chars_at_end_of_line() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 0, Cell::new((b'0' + i as u8) as char));
        }

        grid.insert_chars(8, 0, 5); // Insert 5 at position 8 (only 2 spots left)

        assert_eq!(grid.get(8, 0).unwrap().c, ' '); // Should be blank
        assert_eq!(grid.get(9, 0).unwrap().c, ' '); // Should be blank
    }

    #[test]
    fn test_delete_chars_boundary() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 0, Cell::new((b'A' + i as u8) as char));
        }

        grid.delete_chars(7, 0, 10); // Delete 10 chars from position 7 (only 3 exist)

        assert_eq!(grid.get(7, 0).unwrap().c, ' ');
        assert_eq!(grid.get(8, 0).unwrap().c, ' ');
        assert_eq!(grid.get(9, 0).unwrap().c, ' ');
    }

    #[test]
    fn test_erase_chars_boundary() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 0, Cell::new((b'X' + i as u8) as char));
        }

        grid.erase_chars(5, 0, 20); // Erase 20 chars from position 5 (only 5 exist)

        assert_eq!(grid.get(4, 0).unwrap().c, '\\'); // Should be preserved (X + 4)
        for i in 5..10 {
            assert_eq!(grid.get(i, 0).unwrap().c, ' '); // Should be erased
        }
    }

    #[test]
    fn test_clear_line_operations() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 2, Cell::new('X'));
        }

        // Clear from position 5 to end
        grid.clear_line_right(5, 2);

        assert_eq!(grid.get(4, 2).unwrap().c, 'X'); // Preserved
        assert_eq!(grid.get(5, 2).unwrap().c, ' '); // Cleared
        assert_eq!(grid.get(9, 2).unwrap().c, ' '); // Cleared
    }

    #[test]
    fn test_clear_line_left() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 2, Cell::new('X'));
        }

        // Clear from start to position 5 (inclusive)
        grid.clear_line_left(5, 2);

        for i in 0..=5 {
            assert_eq!(grid.get(i, 2).unwrap().c, ' '); // Cleared
        }
        assert_eq!(grid.get(6, 2).unwrap().c, 'X'); // Preserved
    }

    #[test]
    fn test_clear_screen_operations() {
        let mut grid = Grid::new(10, 10, 1000);
        for row in 0..10 {
            for col in 0..10 {
                grid.set(col, row, Cell::new('X'));
            }
        }

        // Clear from (5,5) to end of screen
        grid.clear_screen_below(5, 5);

        assert_eq!(grid.get(4, 5).unwrap().c, 'X'); // Before cursor on same line - preserved
        assert_eq!(grid.get(5, 5).unwrap().c, ' '); // At cursor - cleared
        assert_eq!(grid.get(0, 6).unwrap().c, ' '); // Next line - cleared
        assert_eq!(grid.get(0, 4).unwrap().c, 'X'); // Previous line - preserved
    }

    #[test]
    fn test_clear_screen_above() {
        let mut grid = Grid::new(10, 10, 1000);
        for row in 0..10 {
            for col in 0..10 {
                grid.set(col, row, Cell::new('X'));
            }
        }

        // Clear from start of screen to (5,5)
        grid.clear_screen_above(5, 5);

        assert_eq!(grid.get(0, 4).unwrap().c, ' '); // Previous line - cleared
        assert_eq!(grid.get(5, 5).unwrap().c, ' '); // At cursor - cleared
        assert_eq!(grid.get(6, 5).unwrap().c, 'X'); // After cursor on same line - preserved
        assert_eq!(grid.get(0, 6).unwrap().c, 'X'); // Next line - preserved
    }

    #[test]
    fn test_scrollback_limit() {
        let mut grid = Grid::new(80, 5, 3); // Max 3 lines of scrollback

        // Scroll up 5 times
        for i in 0..5 {
            grid.set(0, 0, Cell::new((b'A' + i as u8) as char));
            grid.scroll_up(1);
        }

        // Should only have 3 lines in scrollback (max)
        assert_eq!(grid.scrollback_len(), 3);

        // Should have the most recent 3
        let line0 = grid.scrollback_line(0).unwrap();
        assert_eq!(line0[0].c, 'C'); // First scrolled should be 'C' (oldest kept)
    }

    #[test]
    fn test_scroll_down_no_scrollback() {
        let mut grid = Grid::new(80, 5, 100);
        for i in 0..5 {
            grid.set(0, i, Cell::new((b'A' + i as u8) as char));
        }

        grid.scroll_down(2);

        // First 2 lines should be blank
        assert_eq!(grid.get(0, 0).unwrap().c, ' ');
        assert_eq!(grid.get(0, 1).unwrap().c, ' ');
        // Line 2 should have what was at line 0
        assert_eq!(grid.get(0, 2).unwrap().c, 'A');
    }

    #[test]
    fn test_get_out_of_bounds() {
        let grid = Grid::new(80, 24, 1000);

        assert!(grid.get(100, 0).is_none());
        assert!(grid.get(0, 100).is_none());
        assert!(grid.get(100, 100).is_none());
    }

    #[test]
    fn test_row_access() {
        let mut grid = Grid::new(10, 5, 1000);
        for i in 0..10 {
            grid.set(i, 2, Cell::new((b'A' + i as u8) as char));
        }

        let row = grid.row(2).unwrap();
        assert_eq!(row.len(), 10);
        assert_eq!(row[0].c, 'A');
        assert_eq!(row[5].c, 'F');
    }

    #[test]
    fn test_resize_smaller() {
        let mut grid = Grid::new(80, 24, 1000);
        grid.set(50, 20, Cell::new('X'));

        grid.resize(40, 10); // Shrink grid

        assert_eq!(grid.cols(), 40);
        assert_eq!(grid.rows(), 10);
        // Data at (50, 20) should be lost
        assert!(grid.get(50, 20).is_none());
    }

    #[test]
    fn test_export_text_buffer_basic() {
        let mut grid = Grid::new(10, 3, 1000);

        // Set some content
        grid.set(0, 0, Cell::new('H'));
        grid.set(1, 0, Cell::new('e'));
        grid.set(2, 0, Cell::new('l'));
        grid.set(3, 0, Cell::new('l'));
        grid.set(4, 0, Cell::new('o'));

        grid.set(0, 1, Cell::new('W'));
        grid.set(1, 1, Cell::new('o'));
        grid.set(2, 1, Cell::new('r'));
        grid.set(3, 1, Cell::new('l'));
        grid.set(4, 1, Cell::new('d'));

        let text = grid.export_text_buffer();
        let lines: Vec<&str> = text.lines().collect();

        // Last empty line is not included since we don't add newline for empty last row
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Hello");
        assert_eq!(lines[1], "World");
    }

    #[test]
    fn test_export_text_buffer_with_scrollback() {
        let mut grid = Grid::new(10, 2, 1000);

        // Add first line
        grid.set(0, 0, Cell::new('L'));
        grid.set(1, 0, Cell::new('1'));

        // Scroll up (moves L1 to scrollback)
        grid.scroll_up(1);

        // Add second line
        grid.set(0, 0, Cell::new('L'));
        grid.set(1, 0, Cell::new('2'));

        let text = grid.export_text_buffer();
        let lines: Vec<&str> = text.lines().collect();

        // Should have scrollback line followed by current screen
        assert_eq!(lines[0], "L1");
        assert_eq!(lines[1], "L2");
    }

    #[test]
    fn test_export_text_buffer_trims_trailing_spaces() {
        let mut grid = Grid::new(10, 2, 1000);

        // Set content with trailing spaces
        grid.set(0, 0, Cell::new('H'));
        grid.set(1, 0, Cell::new('i'));
        // Columns 2-9 remain as spaces

        let text = grid.export_text_buffer();
        let lines: Vec<&str> = text.lines().collect();

        // Should trim trailing spaces
        assert_eq!(lines[0], "Hi");
    }

    #[test]
    fn test_export_text_buffer_handles_wrapped_lines() {
        let mut grid = Grid::new(10, 3, 1000);

        // Set first line and mark as wrapped
        grid.set(0, 0, Cell::new('A'));
        grid.set(1, 0, Cell::new('B'));
        grid.set_line_wrapped(0, true);

        // Set second line (continuation)
        grid.set(0, 1, Cell::new('C'));
        grid.set(1, 1, Cell::new('D'));

        let text = grid.export_text_buffer();

        // Should not have newline between wrapped lines
        assert!(text.starts_with("ABCD"));
    }

    #[test]
    fn test_export_text_buffer_wide_chars() {
        let mut grid = Grid::new(10, 2, 1000);

        // Set a wide character (width 2)
        let mut cell = Cell::new('中');
        cell.flags.set_wide_char(true);
        grid.set(0, 0, cell);

        // Set a wide char spacer
        let mut spacer = Cell::default();
        spacer.flags.set_wide_char_spacer(true);
        grid.set(1, 0, spacer);

        // Set another wide character
        let mut cell2 = Cell::new('文');
        cell2.flags.set_wide_char(true);
        grid.set(2, 0, cell2);

        let mut spacer2 = Cell::default();
        spacer2.flags.set_wide_char_spacer(true);
        grid.set(3, 0, spacer2);

        let text = grid.export_text_buffer();
        let lines: Vec<&str> = text.lines().collect();

        // Should skip wide char spacers, only include the actual wide characters
        assert_eq!(lines[0], "中文");
    }
}
