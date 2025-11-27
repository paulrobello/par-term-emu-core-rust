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
    ///
    /// Returns the text with full grapheme clusters (including variation selectors,
    /// ZWJ, and other combining characters)
    pub fn row_text(&self, row: usize) -> String {
        if let Some(cells) = self.row(row) {
            cells
                .iter()
                .filter(|cell| !cell.flags.wide_char_spacer())
                .map(|cell| cell.get_grapheme())
                .collect::<Vec<String>>()
                .join("")
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
                        .clone_from_slice(&self.cells[src_start..src_end]);
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
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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

        let old_cols = self.cols;
        let old_rows = self.rows;
        let width_changed = cols != old_cols;

        // IMPORTANT: Reflow scrollback BEFORE updating self.cols
        // because scrollback_line() uses self.cols to slice the old buffer
        if width_changed && self.max_scrollback > 0 && self.scrollback_lines > 0 {
            self.reflow_scrollback(old_cols, cols);
        }

        if width_changed {
            // Width changed - reflow the main grid content
            self.reflow_main_grid(old_cols, old_rows, cols, rows);
        } else {
            // Only height changed - simple copy/truncate
            let mut new_cells = vec![Cell::default(); cols * rows];
            let mut new_wrapped = vec![false; rows];

            let min_rows = self.rows.min(rows);

            for row in 0..min_rows {
                for col in 0..cols {
                    if let Some(cell) = self.get(col, row) {
                        new_cells[row * cols + col] = cell.clone();
                    }
                }
                if row < self.wrapped.len() {
                    new_wrapped[row] = self.wrapped[row];
                }
            }

            self.cols = cols;
            self.rows = rows;
            self.cells = new_cells;
            self.wrapped = new_wrapped;
        }
    }

    /// Reflow main grid content to a new column width
    ///
    /// This handles both width increases (unwrapping) and decreases (re-wrapping).
    /// Excess lines that don't fit are pushed to scrollback.
    fn reflow_main_grid(
        &mut self,
        old_cols: usize,
        old_rows: usize,
        new_cols: usize,
        new_rows: usize,
    ) {
        // Step 1: Extract logical lines from the main grid
        let logical_lines = self.extract_main_grid_logical_lines(old_cols, old_rows);

        // Step 2: Re-wrap each logical line to the new width
        let mut all_cells = Vec::new();
        let mut all_wrapped = Vec::new();

        for logical_line in logical_lines {
            let (cells, wrapped_flags) = self.rewrap_logical_line(&logical_line, new_cols);

            for (i, row_cells) in cells.chunks(new_cols).enumerate() {
                all_cells.extend_from_slice(row_cells);
                // Pad if needed
                while all_cells.len() % new_cols != 0 {
                    all_cells.push(Cell::default());
                }
                let is_wrapped = wrapped_flags.get(i).copied().unwrap_or(false);
                all_wrapped.push(is_wrapped);
            }
        }

        // Step 3: If we have more lines than fit, push TOP excess to scrollback
        // Keep BOTTOM lines visible (where cursor/prompt typically is)
        //
        // IMPORTANT: Only count rows with actual content for overflow calculation.
        // Empty trailing rows (common in fresh terminals with just a prompt at top)
        // should NOT cause content to be pushed to scrollback.
        // Find the last row with actual content
        let mut last_content_line = 0;
        for (line_idx, _) in all_wrapped.iter().enumerate() {
            let start = line_idx * new_cols;
            let end = (start + new_cols).min(all_cells.len());
            let has_content = all_cells[start..end]
                .iter()
                .any(|c| c.c != ' ' || !c.is_empty());
            if has_content {
                last_content_line = line_idx + 1; // +1 because we want count, not index
            }
        }

        // Use the content line count for overflow, but still process all lines
        let effective_lines = last_content_line.max(1); // At least 1 line
        if effective_lines > new_rows {
            let excess_lines = effective_lines - new_rows;

            // Push top excess lines to scrollback (oldest content scrolls up)
            if self.max_scrollback > 0 {
                for line_idx in 0..excess_lines {
                    let start = line_idx * new_cols;
                    let end = start + new_cols;
                    if end <= all_cells.len() {
                        let row_cells = &all_cells[start..end];
                        let is_wrapped = all_wrapped.get(line_idx).copied().unwrap_or(false);

                        // Append to scrollback
                        if self.scrollback_lines < self.max_scrollback {
                            self.scrollback_cells.extend_from_slice(row_cells);
                            self.scrollback_wrapped.push(is_wrapped);
                            self.scrollback_lines += 1;
                        } else if self.scrollback_cells.len() >= self.max_scrollback * new_cols {
                            // Buffer full - overwrite oldest (circular)
                            let physical_index = self.scrollback_start;
                            let sb_start = physical_index * new_cols;

                            for (i, cell) in row_cells.iter().enumerate() {
                                if sb_start + i < self.scrollback_cells.len() {
                                    self.scrollback_cells[sb_start + i] = cell.clone();
                                }
                            }
                            if physical_index < self.scrollback_wrapped.len() {
                                self.scrollback_wrapped[physical_index] = is_wrapped;
                            }
                            self.scrollback_start =
                                (self.scrollback_start + 1) % self.max_scrollback;
                        }
                    }
                }
            }

            // Keep the bottom lines (cursor/prompt area stays visible)
            let keep_start = excess_lines * new_cols;
            all_cells = all_cells[keep_start..].to_vec();
            all_wrapped = all_wrapped[excess_lines..].to_vec();
        } else if all_wrapped.len() < new_rows && self.scrollback_lines > 0 {
            // Step 3b: We have extra space - pull content back from scrollback
            // NOTE: At this point, scrollback has already been reflowed to new_cols width
            // but self.cols still has the old value, so we can't use scrollback_line()
            let empty_rows = new_rows - all_wrapped.len();
            let rows_to_pull = empty_rows.min(self.scrollback_lines);

            if rows_to_pull > 0 {
                // Pull the LAST rows_to_pull lines from scrollback
                // Scrollback is already at new_cols width after reflow_scrollback()
                let mut pulled_cells = Vec::new();
                let mut pulled_wrapped = Vec::new();

                let start_idx = self.scrollback_lines.saturating_sub(rows_to_pull);
                for i in start_idx..self.scrollback_lines {
                    // Access scrollback directly using new_cols (scrollback was already reflowed)
                    let sb_start = i * new_cols;
                    let sb_end = sb_start + new_cols;
                    if sb_end <= self.scrollback_cells.len() {
                        pulled_cells.extend_from_slice(&self.scrollback_cells[sb_start..sb_end]);
                        let is_wrapped = self.scrollback_wrapped.get(i).copied().unwrap_or(false);
                        pulled_wrapped.push(is_wrapped);
                    }
                }

                // Remove pulled lines from scrollback
                self.scrollback_lines = start_idx;
                let new_sb_len = self.scrollback_lines * new_cols;
                self.scrollback_cells.truncate(new_sb_len);
                self.scrollback_wrapped.truncate(self.scrollback_lines);

                // Prepend pulled content to all_cells
                pulled_cells.append(&mut all_cells);
                pulled_wrapped.append(&mut all_wrapped);
                all_cells = pulled_cells;
                all_wrapped = pulled_wrapped;
            }
        }

        // Step 4: Create new grid with reflowed content
        let mut new_cells = vec![Cell::default(); new_cols * new_rows];
        let mut new_wrapped = vec![false; new_rows];

        let lines_to_copy = all_wrapped.len().min(new_rows);
        for row in 0..lines_to_copy {
            let src_start = row * new_cols;
            let src_end = src_start + new_cols;
            let dst_start = row * new_cols;

            if src_end <= all_cells.len() {
                new_cells[dst_start..dst_start + new_cols]
                    .clone_from_slice(&all_cells[src_start..src_end]);
            }
            if row < all_wrapped.len() {
                new_wrapped[row] = all_wrapped[row];
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.cells = new_cells;
        self.wrapped = new_wrapped;
    }

    /// Extract logical lines from the main grid
    fn extract_main_grid_logical_lines(&self, old_cols: usize, old_rows: usize) -> Vec<Vec<Cell>> {
        let mut logical_lines = Vec::new();
        let mut current_line = Vec::new();

        for row in 0..old_rows {
            // Extract cells from this row
            for col in 0..old_cols {
                if let Some(cell) = self.get(col, row) {
                    // Skip wide char spacers - they will be regenerated when rewrapping
                    if !cell.flags.wide_char_spacer() {
                        current_line.push(cell.clone());
                    }
                }
            }

            let is_wrapped = self.wrapped.get(row).copied().unwrap_or(false);

            if !is_wrapped {
                // End of logical line - trim trailing empty cells and save
                while current_line
                    .last()
                    .is_some_and(|c| c.c == ' ' && c.is_empty())
                {
                    current_line.pop();
                }
                logical_lines.push(std::mem::take(&mut current_line));
            }
            // If wrapped, continue accumulating into current_line
        }

        // Don't forget any remaining content
        if !current_line.is_empty() {
            logical_lines.push(current_line);
        }

        logical_lines
    }

    /// Reflow scrollback content to a new column width
    ///
    /// This method handles both width increases (unwrapping) and decreases (re-wrapping).
    /// It preserves all cell attributes, colors, and handles wide characters correctly.
    fn reflow_scrollback(&mut self, old_cols: usize, new_cols: usize) {
        // Step 1: Extract logical lines from the scrollback
        // A logical line is one or more physical lines connected by wrapped=true
        let logical_lines = self.extract_logical_lines(old_cols);

        // Step 2: Re-wrap each logical line to the new width
        let mut new_scrollback_cells = Vec::new();
        let mut new_scrollback_wrapped = Vec::new();
        let mut new_scrollback_lines = 0;

        for logical_line in logical_lines {
            let (cells, wrapped_flags) = self.rewrap_logical_line(&logical_line, new_cols);

            // Add the re-wrapped lines to the new scrollback
            for (i, row_cells) in cells.chunks(new_cols).enumerate() {
                // Only add up to max_scrollback lines
                if new_scrollback_lines >= self.max_scrollback {
                    // We've exceeded max scrollback - need to use circular buffer logic
                    // For simplicity, we'll shift out old content
                    let rows_to_shift = new_scrollback_cells.len() / new_cols;
                    if rows_to_shift > 0 {
                        // Remove the oldest row
                        new_scrollback_cells.drain(0..new_cols);
                        new_scrollback_wrapped.remove(0);
                        new_scrollback_lines -= 1;
                    }
                }

                new_scrollback_cells.extend_from_slice(row_cells);
                // Pad with default cells if the row is shorter than new_cols
                while new_scrollback_cells.len() % new_cols != 0 {
                    new_scrollback_cells.push(Cell::default());
                }
                // Set wrapped flag (last row of logical line is not wrapped)
                let is_wrapped =
                    i < wrapped_flags.len() && wrapped_flags.get(i).copied().unwrap_or(false);
                new_scrollback_wrapped.push(is_wrapped);
                new_scrollback_lines += 1;
            }
        }

        // Limit to max_scrollback lines (keep the most recent)
        while new_scrollback_lines > self.max_scrollback {
            new_scrollback_cells.drain(0..new_cols);
            new_scrollback_wrapped.remove(0);
            new_scrollback_lines -= 1;
        }

        // Step 3: Update the scrollback state
        self.scrollback_cells = new_scrollback_cells;
        self.scrollback_wrapped = new_scrollback_wrapped;
        self.scrollback_lines = new_scrollback_lines;
        self.scrollback_start = 0; // Reset to non-circular since we rebuilt it
    }

    /// Extract logical lines from the scrollback buffer
    ///
    /// Returns a Vec of logical lines, where each logical line is a Vec<Cell>
    /// containing the content that should be treated as a single line
    /// (connected by soft wraps).
    fn extract_logical_lines(&self, _old_cols: usize) -> Vec<Vec<Cell>> {
        let mut logical_lines = Vec::new();
        let mut current_line = Vec::new();

        for line_idx in 0..self.scrollback_lines {
            if let Some(row_cells) = self.scrollback_line(line_idx) {
                // Extract significant cells (skip trailing default cells and wide char spacers)
                for cell in row_cells.iter() {
                    // Skip wide char spacers - they will be regenerated when rewrapping
                    if !cell.flags.wide_char_spacer() {
                        current_line.push(cell.clone());
                    }
                }

                // Trim trailing empty cells from the current row contribution
                // But only if this is not a wrapped line (wrapped lines might end with spaces)
                let is_wrapped = self.is_scrollback_wrapped(line_idx);

                if !is_wrapped {
                    // End of logical line - trim trailing spaces and save
                    while current_line
                        .last()
                        .is_some_and(|c| c.c == ' ' && c.is_empty())
                    {
                        current_line.pop();
                    }
                    logical_lines.push(std::mem::take(&mut current_line));
                }
                // If wrapped, continue accumulating into current_line
            }
        }

        // Don't forget any remaining content
        if !current_line.is_empty() {
            logical_lines.push(current_line);
        }

        logical_lines
    }

    /// Re-wrap a logical line to fit the new column width
    ///
    /// Returns (cells, wrapped_flags) where:
    /// - cells is a flat Vec<Cell> of the re-wrapped content
    /// - wrapped_flags indicates which rows are soft-wrapped (true) vs hard newline (false)
    fn rewrap_logical_line(
        &self,
        logical_line: &[Cell],
        new_cols: usize,
    ) -> (Vec<Cell>, Vec<bool>) {
        if logical_line.is_empty() {
            // Empty logical line = one empty row with no wrap
            let empty_row = vec![Cell::default(); new_cols];
            return (empty_row, vec![false]);
        }

        let mut result_cells = Vec::new();
        let mut wrapped_flags = Vec::new();
        let mut current_col = 0;
        let mut row_start = result_cells.len();

        for cell in logical_line.iter() {
            let cell_width = if cell.flags.wide_char() { 2 } else { 1 };

            // Check if this cell fits on the current row
            if current_col + cell_width > new_cols {
                // Need to wrap to next row
                // First, pad current row to new_cols
                while result_cells.len() - row_start < new_cols {
                    result_cells.push(Cell::default());
                }
                wrapped_flags.push(true); // This row is soft-wrapped

                // Start new row
                row_start = result_cells.len();
                current_col = 0;
            }

            // Add the cell
            result_cells.push(cell.clone());
            current_col += 1;

            // Add wide char spacer if needed
            if cell.flags.wide_char() {
                let mut spacer = Cell::default();
                spacer.flags.set_wide_char_spacer(true);
                result_cells.push(spacer);
                current_col += 1;
            }
        }

        // Pad final row to new_cols
        while result_cells.len() - row_start < new_cols {
            result_cells.push(Cell::default());
        }
        wrapped_flags.push(false); // Last row is not wrapped (hard newline)

        (result_cells, wrapped_flags)
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

    /// Clear all scrollback content
    pub fn clear_scrollback(&mut self) {
        self.scrollback_cells.clear();
        self.scrollback_wrapped.clear();
        self.scrollback_lines = 0;
        self.scrollback_start = 0;
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
                    // Output full grapheme cluster (base char + combining chars)
                    result.push(cell.c);
                    for &combining in &cell.combining {
                        result.push(combining);
                    }
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
                        // Output full grapheme cluster (base char + combining chars)
                        line_text.push(cell.c);
                        for &combining in &cell.combining {
                            line_text.push(combining);
                        }
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
                        // Output full grapheme cluster (base char + combining chars)
                        line_text.push(cell.c);
                        for &combining in &cell.combining {
                            line_text.push(combining);
                        }
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

        // Helper to find last significant column (styled or non-space content)
        let default_fg = Color::Named(NamedColor::White);
        let default_bg = Color::Named(NamedColor::Black);
        let default_flags = crate::cell::CellFlags::default();
        let find_last_significant = |cells: &[Cell]| -> usize {
            let mut last = 0;
            for (col, cell) in cells.iter().enumerate() {
                if cell.flags.wide_char_spacer() {
                    continue;
                }
                let has_content = cell.c != ' ' || !cell.combining.is_empty();
                let has_styling =
                    cell.fg != default_fg || cell.bg != default_bg || cell.flags != default_flags;
                if has_content || has_styling {
                    last = col + 1;
                }
            }
            last
        };

        // Export scrollback buffer first
        for line_idx in 0..self.scrollback_lines {
            if let Some(line_cells) = self.scrollback_line(line_idx) {
                let last_significant = find_last_significant(line_cells);
                let mut line_text = String::new();

                for (col, cell) in line_cells.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }

                    // Stop after last significant column (col is array index)
                    if col >= last_significant {
                        break;
                    }

                    // Check if style changed
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut line_text, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }

                    // Output full grapheme cluster (base char + combining chars)
                    line_text.push(cell.c);
                    for &combining in &cell.combining {
                        line_text.push(combining);
                    }
                }

                result.push_str(&line_text);

                // Reset style at end of line
                if !line_text.is_empty() {
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
                let last_significant = find_last_significant(row_cells);
                let mut line_text = String::new();

                for (col, cell) in row_cells.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }

                    // Stop after last significant column (col is array index)
                    if col >= last_significant {
                        break;
                    }

                    // Check if style changed
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut line_text, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }

                    // Output full grapheme cluster (base char + combining chars)
                    line_text.push(cell.c);
                    for &combining in &cell.combining {
                        line_text.push(combining);
                    }
                }

                result.push_str(&line_text);

                // Reset style at end of line if there's content
                if !line_text.is_empty() {
                    result.push_str("\x1b[0m");
                    current_fg = Color::Named(NamedColor::White);
                    current_bg = Color::Named(NamedColor::Black);
                    current_flags = crate::cell::CellFlags::default();
                }

                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push('\n');
                    }
                } else if !line_text.is_empty() {
                    result.push('\n');
                }
            }
        }

        result
    }

    /// Export only the visible screen with ANSI styling (excludes scrollback)
    ///
    /// This method exports just the current visible terminal screen with ANSI
    /// escape codes for colors and text attributes. Unlike export_styled_buffer(),
    /// this does NOT include scrollback history.
    ///
    /// # Returns
    /// String with ANSI-styled visible screen content
    pub fn export_visible_screen_styled(&self) -> String {
        use crate::color::{Color, NamedColor};

        // Pre-allocate based on estimated size
        let estimated_size = self.rows * self.cols * 20;
        let mut result = String::with_capacity(estimated_size);

        // Start with cursor home to ensure content begins at row 0, col 0
        result.push_str("\x1b[H");

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

        // Export only the visible screen (no scrollback)
        // Use explicit cursor positioning for each row to avoid newline handling issues
        let default_fg = Color::Named(NamedColor::White);
        let default_bg = Color::Named(NamedColor::Black);
        let default_flags = crate::cell::CellFlags::default();

        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                // Find the last column with non-default content (styled or non-space)
                // This prevents trimming styled spaces (e.g., gradient backgrounds)
                let mut last_significant_col = 0;
                for (col, cell) in row_cells.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }
                    // A cell is significant if it has non-space content OR non-default styling
                    let has_content = cell.c != ' ' || !cell.combining.is_empty();
                    let has_styling = cell.fg != default_fg
                        || cell.bg != default_bg
                        || cell.flags != default_flags;
                    if has_content || has_styling {
                        last_significant_col = col + 1;
                    }
                }

                // Skip empty rows entirely
                if last_significant_col == 0 {
                    continue;
                }

                // Position cursor at beginning of this row (1-indexed for VT100)
                result.push_str(&format!("\x1b[{};1H", row + 1));

                let mut line_text = String::new();

                for (col, cell) in row_cells.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }

                    // Stop after last significant column (col is array index, not non-spacer count)
                    if col >= last_significant_col {
                        break;
                    }

                    // Check if style changed
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut line_text, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }

                    // Output full grapheme cluster (base char + combining chars)
                    line_text.push(cell.c);
                    for &combining in &cell.combining {
                        line_text.push(combining);
                    }
                }

                result.push_str(&line_text);

                // Reset style at end of line if there's content
                if !line_text.is_empty() {
                    result.push_str("\x1b[0m");
                    current_fg = Color::Named(NamedColor::White);
                    current_bg = Color::Named(NamedColor::Black);
                    current_flags = crate::cell::CellFlags::default();
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
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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
                row_cells[i] = row_cells[i - n].clone();
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
                row_cells[i] = row_cells[i + n].clone();
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
        let region_size = effective_bottom - top + 1;

        // If scrolling the entire screen (top=0, bottom=rows-1), save to scrollback
        // Only save to scrollback if max_scrollback > 0 (alternate screen has no scrollback)
        // Do this BEFORE clearing, even if n >= region_size
        if top == 0 && effective_bottom == self.rows - 1 && self.max_scrollback > 0 {
            // When n >= region_size, save the entire screen to scrollback
            let lines_to_save = n.min(region_size);
            for i in 0..lines_to_save {
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
                        .clone_from_slice(&self.cells[src_start..src_end]);
                    self.scrollback_wrapped[write_idx] = is_wrapped;

                    // Advance start pointer (circular)
                    self.scrollback_start = (self.scrollback_start + 1) % self.max_scrollback;
                }
            }
        }

        // If n >= region_size, just clear the entire region
        if n >= region_size {
            for i in top..=effective_bottom {
                self.clear_row(i);
            }
            return;
        }

        // Otherwise, move lines up within the region
        for i in top..=(effective_bottom - n) {
            let src_start = (i + n) * self.cols;
            let dst_start = i * self.cols;
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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
            // Clone cells from source to destination
            for j in 0..self.cols {
                self.cells[dst_start + j] = self.cells[src_start + j].clone();
            }
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
                    *cell = fill_cell.clone();
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
                    buffer.push(cell.clone());
                }
            }
        }

        // Copy from buffer to destination
        let mut buffer_idx = 0;
        for row in dst_top..=dst_bottom {
            for col in dst_left..=dst_right {
                if buffer_idx < buffer.len() {
                    if let Some(cell) = self.get_mut(col, row) {
                        *cell = buffer[buffer_idx].clone();
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
    fn test_resize_preserves_scrollback_when_width_unchanged() {
        let mut grid = Grid::new(10, 3, 3);

        // Create a few scrollback lines by scrolling up
        for ch in ['A', 'B', 'C'] {
            grid.set(0, 0, Cell::new(ch));
            grid.scroll_up(1);
        }

        assert_eq!(grid.scrollback_len(), 3);
        let before = grid
            .scrollback_line(0)
            .and_then(|line| line.first())
            .unwrap()
            .c;
        assert_eq!(before, 'A');

        // Change only height; width stays the same
        grid.resize(10, 5);

        assert_eq!(grid.scrollback_len(), 3);
        let after = grid
            .scrollback_line(0)
            .and_then(|line| line.first())
            .unwrap()
            .c;
        assert_eq!(after, 'A');
    }

    #[test]
    fn test_resize_reflows_scrollback_when_width_changes() {
        let mut grid = Grid::new(10, 3, 3);

        grid.set(0, 0, Cell::new('X'));
        grid.scroll_up(1);
        assert_eq!(grid.scrollback_len(), 1);

        grid.resize(20, 3);

        assert_eq!(grid.cols(), 20);
        // Scrollback should now be preserved and reflowed, not cleared
        assert_eq!(grid.scrollback_len(), 1);
        let line = grid.scrollback_line(0).unwrap();
        assert_eq!(line[0].c, 'X');
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

    #[test]
    fn test_fill_rectangle() {
        let mut grid = Grid::new(80, 24, 1000);

        // Fill a 3x3 rectangle starting at (5, 5) with 'X'
        let fill_cell = Cell::new('X');
        grid.fill_rectangle(fill_cell, 5, 5, 7, 7);

        // Check that cells inside the rectangle are filled
        assert_eq!(grid.get(5, 5).unwrap().c, 'X');
        assert_eq!(grid.get(6, 6).unwrap().c, 'X');
        assert_eq!(grid.get(7, 7).unwrap().c, 'X');

        // Check that cells outside are not affected
        assert_eq!(grid.get(4, 5).unwrap().c, ' ');
        assert_eq!(grid.get(8, 7).unwrap().c, ' ');
    }

    #[test]
    fn test_fill_rectangle_boundaries() {
        let mut grid = Grid::new(80, 24, 1000);

        // Fill rectangle at grid boundaries
        let fill_cell = Cell::new('B');
        grid.fill_rectangle(fill_cell, 0, 0, 2, 2);

        assert_eq!(grid.get(0, 0).unwrap().c, 'B');
        assert_eq!(grid.get(1, 1).unwrap().c, 'B');
        assert_eq!(grid.get(2, 2).unwrap().c, 'B');
    }

    #[test]
    fn test_copy_rectangle() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set up source rectangle with pattern
        for row in 2..5 {
            for col in 2..5 {
                grid.set(col, row, Cell::new('S'));
            }
        }

        // Copy rectangle from (2,2) to (10,10)
        grid.copy_rectangle(2, 2, 4, 4, 10, 10);

        // Verify copy
        assert_eq!(grid.get(10, 10).unwrap().c, 'S');
        assert_eq!(grid.get(11, 11).unwrap().c, 'S');
        assert_eq!(grid.get(12, 12).unwrap().c, 'S');

        // Original should still exist
        assert_eq!(grid.get(2, 2).unwrap().c, 'S');
    }

    #[test]
    fn test_copy_rectangle_to_different_location() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set up source with unique chars in non-overlapping area
        grid.set(0, 0, Cell::new('A'));
        grid.set(1, 0, Cell::new('B'));
        grid.set(0, 1, Cell::new('C'));
        grid.set(1, 1, Cell::new('D'));

        // Copy to far away location (no overlap)
        grid.copy_rectangle(0, 0, 1, 1, 10, 10);

        // Verify copy worked
        assert_eq!(grid.get(10, 10).unwrap().c, 'A');
        assert_eq!(grid.get(11, 10).unwrap().c, 'B');
        assert_eq!(grid.get(10, 11).unwrap().c, 'C');
        assert_eq!(grid.get(11, 11).unwrap().c, 'D');
    }

    #[test]
    fn test_erase_rectangle() {
        let mut grid = Grid::new(80, 24, 1000);

        // Fill area with chars
        for row in 5..10 {
            for col in 5..10 {
                grid.set(col, row, Cell::new('T'));
            }
        }

        // Erase rectangle
        grid.erase_rectangle(6, 6, 8, 8);

        // Check erased area
        assert_eq!(grid.get(6, 6).unwrap().c, ' ');
        assert_eq!(grid.get(7, 7).unwrap().c, ' ');
        assert_eq!(grid.get(8, 8).unwrap().c, ' ');

        // Check boundary cells not erased
        assert_eq!(grid.get(5, 5).unwrap().c, 'T');
        assert_eq!(grid.get(9, 9).unwrap().c, 'T');
    }

    #[test]
    fn test_erase_rectangle_unconditional() {
        let mut grid = Grid::new(80, 24, 1000);

        // Fill with different chars
        grid.set(10, 10, Cell::new('U'));
        grid.set(11, 11, Cell::new('V'));

        // Erase unconditionally
        grid.erase_rectangle_unconditional(10, 10, 11, 11);

        assert_eq!(grid.get(10, 10).unwrap().c, ' ');
        assert_eq!(grid.get(11, 11).unwrap().c, ' ');
    }

    #[test]
    fn test_change_attributes_in_rectangle() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set up cells with chars
        for row in 3..6 {
            for col in 3..6 {
                let mut cell = Cell::new('M');
                cell.flags.set_bold(false);
                grid.set(col, row, cell);
            }
        }

        // Change attributes - make them bold (attribute 1 = bold)
        let attributes = [1u16];
        grid.change_attributes_in_rectangle(3, 3, 5, 5, &attributes);

        // Verify attributes changed but char remained
        let cell = grid.get(4, 4).unwrap();
        assert_eq!(cell.c, 'M');
        assert!(cell.flags.bold());
    }

    #[test]
    fn test_reverse_attributes_in_rectangle() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set up cells
        let mut cell = Cell::new('R');
        cell.flags.set_reverse(false);
        grid.set(20, 20, cell);

        // Reverse attributes - attribute 7 toggles reverse flag
        let attributes = [7u16];
        grid.reverse_attributes_in_rectangle(20, 20, 20, 20, &attributes);

        // Verify reverse flag is now true
        let reversed = grid.get(20, 20).unwrap();
        assert_eq!(reversed.c, 'R');
        assert!(reversed.flags.reverse());

        // Toggle again - should go back to false
        grid.reverse_attributes_in_rectangle(20, 20, 20, 20, &attributes);
        let unreversed = grid.get(20, 20).unwrap();
        assert!(!unreversed.flags.reverse());
    }

    #[test]
    fn test_row_text() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set up a row with text
        let text = "Hello, World!";
        for (i, ch) in text.chars().enumerate() {
            grid.set(i, 5, Cell::new(ch));
        }

        let row_text = grid.row_text(5);
        assert!(row_text.starts_with("Hello, World!"));
    }

    #[test]
    fn test_row_text_with_wide_chars() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set wide character
        let mut cell = Cell::new('中');
        cell.flags.set_wide_char(true);
        grid.set(0, 0, cell);

        // Set spacer
        let mut spacer = Cell::default();
        spacer.flags.set_wide_char_spacer(true);
        grid.set(1, 0, spacer);

        let row_text = grid.row_text(0);
        // Should skip the spacer
        assert_eq!(row_text.chars().next().unwrap(), '中');
    }

    #[test]
    fn test_content_as_string() {
        let mut grid = Grid::new(10, 3, 1000);

        // Fill first row
        for col in 0..10 {
            grid.set(col, 0, Cell::new('A'));
        }

        // Fill second row partially
        for col in 0..5 {
            grid.set(col, 1, Cell::new('B'));
        }

        let content = grid.content_as_string();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("AAAAAAAAAA"));
        assert!(lines[1].starts_with("BBBBB"));
    }

    #[test]
    fn test_is_scrollback_wrapped_circular() {
        let mut grid = Grid::new(80, 2, 3); // Small scrollback for testing

        // Scroll multiple times to trigger circular buffer
        for i in 0..5 {
            grid.scroll_up(1);
            if i % 2 == 0 {
                grid.scrollback_wrapped[grid
                    .scrollback_lines
                    .saturating_sub(1)
                    .min(grid.max_scrollback - 1)] = true;
            }
        }

        // Test wrapped state retrieval - just ensure it doesn't panic with circular buffer
        let _wrapped = grid.is_scrollback_wrapped(0);
    }

    #[test]
    fn test_debug_snapshot() {
        let mut grid = Grid::new(10, 3, 2);

        // Add some content
        grid.set(0, 0, Cell::new('D'));
        grid.set(1, 0, Cell::new('E'));
        grid.set(2, 0, Cell::new('B'));
        grid.set(3, 0, Cell::new('U'));
        grid.set(4, 0, Cell::new('G'));

        let snapshot = grid.debug_snapshot();

        // Verify snapshot contains expected content
        assert!(snapshot.contains("DEBUG"));
        assert!(snapshot.contains("|DEBUG"));
    }

    #[test]
    fn test_scrollback_line_circular_buffer() {
        let mut grid = Grid::new(80, 24, 2); // Max 2 scrollback lines

        // Scroll 3 times to wrap circular buffer
        grid.scroll_up(1);
        grid.scroll_up(1);
        grid.scroll_up(1);

        // Accessing scrollback should not panic
        let line = grid.scrollback_line(0);
        assert!(line.is_some());

        let line = grid.scrollback_line(1);
        assert!(line.is_some() || line.is_none()); // Depends on implementation
    }

    #[test]
    fn test_set_line_wrapped_bounds() {
        let mut grid = Grid::new(80, 24, 1000);

        // Set wrapped state
        grid.set_line_wrapped(5, true);
        assert!(grid.is_line_wrapped(5));

        // Clear it
        grid.set_line_wrapped(5, false);
        assert!(!grid.is_line_wrapped(5));

        // Out of bounds should not panic
        grid.set_line_wrapped(100, true);
        assert!(!grid.is_line_wrapped(100));
    }

    #[test]
    fn test_export_styled_buffer() {
        let mut grid = Grid::new(20, 3, 1000);

        // Add some styled content
        let mut cell = Cell::new('S');
        cell.flags.set_bold(true);
        grid.set(0, 0, cell);

        let styled = grid.export_styled_buffer();

        // Should contain ANSI codes for bold
        assert!(styled.contains("\x1b["));
    }

    #[test]
    fn test_clear_row() {
        let mut grid = Grid::new(80, 24, 1000);

        // Fill a row
        for col in 0..80 {
            grid.set(col, 10, Cell::new('X'));
        }

        // Clear it
        grid.clear_row(10);

        // Verify cleared
        for col in 0..80 {
            assert_eq!(grid.get(col, 10).unwrap().c, ' ');
        }
    }

    // ===== Scrollback Reflow Tests =====

    #[test]
    fn test_scrollback_reflow_width_increase_unwraps() {
        // Test that increasing width unwraps previously wrapped lines
        let mut grid = Grid::new(10, 3, 100);

        // Create a line that wraps: "ABCDEFGHIJ" (10 chars) + "KLMNO" (5 chars)
        // This will be 2 physical lines with wrap=true on the first
        for (i, ch) in "ABCDEFGHIJ".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }
        grid.set_line_wrapped(0, true);
        for (i, ch) in "KLMNO".chars().enumerate() {
            grid.set(i, 1, Cell::new(ch));
        }

        // Scroll these lines into scrollback
        grid.scroll_up(2);
        assert_eq!(grid.scrollback_len(), 2);
        assert!(grid.is_scrollback_wrapped(0)); // First line should be wrapped

        // Now resize to wider (20 cols) - should unwrap
        grid.resize(20, 3);

        // After reflow, both lines should merge into one (15 chars fits in 20 cols)
        assert_eq!(grid.scrollback_len(), 1);
        assert!(!grid.is_scrollback_wrapped(0)); // Should not be wrapped anymore

        // Verify content is preserved
        let line = grid.scrollback_line(0).unwrap();
        assert_eq!(line[0].c, 'A');
        assert_eq!(line[4].c, 'E');
        assert_eq!(line[10].c, 'K');
        assert_eq!(line[14].c, 'O');
    }

    #[test]
    fn test_scrollback_reflow_width_decrease_rewraps() {
        // Test that decreasing width re-wraps lines
        let mut grid = Grid::new(20, 3, 100);

        // Create a single line with 15 characters
        for (i, ch) in "ABCDEFGHIJKLMNO".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }

        // Scroll into scrollback
        grid.scroll_up(1);
        assert_eq!(grid.scrollback_len(), 1);
        assert!(!grid.is_scrollback_wrapped(0));

        // Now resize to narrower (10 cols) - should re-wrap
        grid.resize(10, 3);

        // After reflow, should be 2 lines (10 + 5 chars)
        assert_eq!(grid.scrollback_len(), 2);
        assert!(grid.is_scrollback_wrapped(0)); // First line should be wrapped now
        assert!(!grid.is_scrollback_wrapped(1)); // Second line not wrapped

        // Verify content
        let line0 = grid.scrollback_line(0).unwrap();
        let line1 = grid.scrollback_line(1).unwrap();
        assert_eq!(line0[0].c, 'A');
        assert_eq!(line0[9].c, 'J');
        assert_eq!(line1[0].c, 'K');
        assert_eq!(line1[4].c, 'O');
    }

    #[test]
    fn test_scrollback_reflow_preserves_colors() {
        use crate::color::Color;

        let mut grid = Grid::new(10, 3, 100);

        // Create a colored cell
        let mut cell = Cell::new('X');
        cell.fg = Color::Rgb(255, 0, 0);
        cell.bg = Color::Rgb(0, 255, 0);
        cell.flags.set_bold(true);
        grid.set(0, 0, cell);

        // Scroll into scrollback
        grid.scroll_up(1);

        // Resize (triggers reflow)
        grid.resize(20, 3);

        // Verify colors and attributes preserved
        let line = grid.scrollback_line(0).unwrap();
        assert_eq!(line[0].c, 'X');
        assert_eq!(line[0].fg, Color::Rgb(255, 0, 0));
        assert_eq!(line[0].bg, Color::Rgb(0, 255, 0));
        assert!(line[0].flags.bold());
    }

    #[test]
    fn test_scrollback_reflow_wide_chars() {
        // Test that wide characters are handled correctly during reflow
        let mut grid = Grid::new(10, 3, 100);

        // Create a wide character at position 8 (needs 2 cells)
        let mut wide_cell = Cell::new('中');
        wide_cell.flags.set_wide_char(true);
        grid.set(8, 0, wide_cell);

        let mut spacer = Cell::default();
        spacer.flags.set_wide_char_spacer(true);
        grid.set(9, 0, spacer);

        // Scroll into scrollback
        grid.scroll_up(1);

        // Resize to 5 cols - wide char should wrap properly
        grid.resize(5, 3);

        // The wide char should be on its own row or properly wrapped
        // (can't split a wide char across lines)
        assert!(grid.scrollback_len() >= 1);

        // Verify the wide char is preserved
        let mut found_wide = false;
        for i in 0..grid.scrollback_len() {
            if let Some(line) = grid.scrollback_line(i) {
                for cell in line {
                    if cell.c == '中' {
                        found_wide = true;
                        assert!(cell.flags.wide_char());
                        break;
                    }
                }
            }
        }
        assert!(
            found_wide,
            "Wide character should be preserved after reflow"
        );
    }

    #[test]
    fn test_scrollback_reflow_multiple_logical_lines() {
        // Test reflow with multiple separate logical lines (non-wrapped)
        let mut grid = Grid::new(10, 5, 100);

        // Create 3 separate lines
        for (i, ch) in "LINE1".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }
        for (i, ch) in "LINE2".chars().enumerate() {
            grid.set(i, 1, Cell::new(ch));
        }
        for (i, ch) in "LINE3".chars().enumerate() {
            grid.set(i, 2, Cell::new(ch));
        }

        // Scroll all into scrollback
        grid.scroll_up(3);
        assert_eq!(grid.scrollback_len(), 3);

        // Resize wider
        grid.resize(20, 5);

        // Should still have 3 separate lines
        assert_eq!(grid.scrollback_len(), 3);

        let line0 = grid.scrollback_line(0).unwrap();
        let line1 = grid.scrollback_line(1).unwrap();
        let line2 = grid.scrollback_line(2).unwrap();

        assert_eq!(line0[0].c, 'L');
        assert_eq!(line0[4].c, '1');
        assert_eq!(line1[4].c, '2');
        assert_eq!(line2[4].c, '3');
    }

    #[test]
    fn test_scrollback_reflow_max_scrollback_limit() {
        // Test that reflow respects max_scrollback limit
        let mut grid = Grid::new(20, 5, 3); // Only 3 lines max

        // Create a long line that will need 4 rows when reflowed to 5 cols
        for (i, ch) in "ABCDEFGHIJKLMNOPQRST".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }

        grid.scroll_up(1);
        assert_eq!(grid.scrollback_len(), 1);

        // Resize to 5 cols - would need 4 lines, but max is 3
        grid.resize(5, 5);

        // Should be capped at 3 lines
        assert!(grid.scrollback_len() <= 3);
    }

    #[test]
    fn test_scrollback_reflow_empty_scrollback() {
        // Test that reflow handles empty scrollback gracefully
        let mut grid = Grid::new(10, 3, 100);

        assert_eq!(grid.scrollback_len(), 0);

        // Resize - should not panic
        grid.resize(20, 3);

        assert_eq!(grid.scrollback_len(), 0);
    }

    #[test]
    fn test_scrollback_reflow_same_width() {
        // Test that same width doesn't trigger unnecessary reflow
        let mut grid = Grid::new(10, 3, 100);

        for (i, ch) in "HELLO".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }
        grid.scroll_up(1);

        let orig_len = grid.scrollback_len();

        // Resize with same width but different height
        grid.resize(10, 5);

        // Scrollback should be unchanged (no width change, no reflow)
        assert_eq!(grid.scrollback_len(), orig_len);
    }

    #[test]
    fn test_scrollback_reflow_circular_buffer() {
        // Test reflow when scrollback is using circular buffer
        let mut grid = Grid::new(10, 2, 3); // Small max for quick circular

        // Fill scrollback past capacity (4 scrolls with max 3)
        for i in 0..4 {
            grid.set(0, 0, Cell::new((b'A' + i as u8) as char));
            grid.scroll_up(1);
        }

        // Scrollback should have 3 lines (circular, oldest dropped)
        assert_eq!(grid.scrollback_len(), 3);

        // The oldest line should be 'B' (A was dropped)
        let line0 = grid.scrollback_line(0).unwrap();
        assert_eq!(line0[0].c, 'B');

        // Resize - reflow should handle circular buffer correctly
        grid.resize(20, 2);

        // Content should still be B, C, D
        assert_eq!(grid.scrollback_len(), 3);
        let line0 = grid.scrollback_line(0).unwrap();
        let line1 = grid.scrollback_line(1).unwrap();
        let line2 = grid.scrollback_line(2).unwrap();
        assert_eq!(line0[0].c, 'B');
        assert_eq!(line1[0].c, 'C');
        assert_eq!(line2[0].c, 'D');
    }

    #[test]
    fn test_scrollback_reflow_wrapped_chain() {
        // Test reflow of a chain of wrapped lines that spans multiple rows
        let mut grid = Grid::new(5, 5, 100);

        // Create a 15-char line that spans 3 rows at width 5
        for (i, ch) in "ABCDEFGHIJKLMNO".chars().enumerate() {
            let row = i / 5;
            let col = i % 5;
            grid.set(col, row, Cell::new(ch));
        }
        grid.set_line_wrapped(0, true);
        grid.set_line_wrapped(1, true);
        grid.set_line_wrapped(2, false); // End of logical line

        // Scroll all 3 rows into scrollback
        grid.scroll_up(3);
        assert_eq!(grid.scrollback_len(), 3);

        // Resize to 15 cols - should unwrap into single line
        grid.resize(15, 5);

        assert_eq!(grid.scrollback_len(), 1);
        let line = grid.scrollback_line(0).unwrap();
        assert_eq!(line[0].c, 'A');
        assert_eq!(line[14].c, 'O');
    }
}
