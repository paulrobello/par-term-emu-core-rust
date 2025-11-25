//! Graphics management
//!
//! Handles graphics storage, retrieval, and position adjustments during scrolling.
//! Supports Sixel, iTerm2, and Kitty graphics protocols via unified GraphicsStore.

use crate::debug;
use crate::graphics::TerminalGraphic;
use crate::terminal::Terminal;

impl Terminal {
    /// Get graphics at a specific row
    pub fn graphics_at_row(&self, row: usize) -> Vec<&TerminalGraphic> {
        self.graphics_store.graphics_at_row(row)
    }

    /// Get all graphics
    pub fn all_graphics(&self) -> &[TerminalGraphic] {
        self.graphics_store.all_graphics()
    }

    /// Get total graphics count
    pub fn graphics_count(&self) -> usize {
        self.graphics_store.graphics_count()
    }

    /// Get graphics in scrollback for a range of rows
    pub fn scrollback_graphics(&self, start_row: usize, end_row: usize) -> Vec<&TerminalGraphic> {
        self.graphics_store
            .graphics_in_scrollback(start_row, end_row)
    }

    /// Get all scrollback graphics
    pub fn all_scrollback_graphics(&self) -> &[TerminalGraphic] {
        self.graphics_store.all_scrollback_graphics()
    }

    /// Get scrollback graphics count
    pub fn scrollback_graphics_count(&self) -> usize {
        self.graphics_store.scrollback_count()
    }

    /// Clear all graphics
    pub fn clear_graphics(&mut self) {
        self.graphics_store.clear();
    }

    /// Get immutable access to graphics store
    pub fn graphics_store(&self) -> &crate::graphics::GraphicsStore {
        &self.graphics_store
    }

    /// Get mutable access to graphics store
    pub fn graphics_store_mut(&mut self) -> &mut crate::graphics::GraphicsStore {
        &mut self.graphics_store
    }

    /// Insert Unicode placeholder characters for a virtual placement
    ///
    /// This inserts U+10EEEE placeholder characters into the terminal grid
    /// with appropriate foreground/underline colors and diacritics to mark
    /// where a Kitty virtual placement should be rendered.
    ///
    /// # Arguments
    /// * `image_id` - Kitty image ID
    /// * `placement_id` - Kitty placement ID (0 if not specified)
    /// * `position` - (col, row) position of the top-left corner
    /// * `cols` - Width in terminal columns
    /// * `rows` - Height in terminal rows
    pub(crate) fn insert_placeholder_chars(
        &mut self,
        image_id: u32,
        placement_id: u32,
        position: (usize, usize),
        cols: usize,
        rows: usize,
    ) {
        use crate::cell::Cell;
        use crate::color::Color;
        use crate::graphics::{create_placeholder_with_diacritics, PLACEHOLDER_CHAR};

        let (start_col, start_row) = position;
        let (grid_cols, grid_rows) = self.size();

        debug::log(
            debug::DebugLevel::Debug,
            "KITTY_PLACEHOLDER",
            &format!(
                "Inserting placeholders: image_id={}, placement_id={}, pos=({},{}), size={}x{}",
                image_id, placement_id, start_col, start_row, cols, rows
            ),
        );

        // Extract MSB from image_id if > 24 bits
        let msb = if image_id > 0x00FFFFFF {
            Some(((image_id >> 24) & 0xFF) as u8)
        } else {
            None
        };

        // Encode image_id in foreground color (lower 24 bits)
        let image_id_color = Color::Rgb(
            ((image_id >> 16) & 0xFF) as u8,
            ((image_id >> 8) & 0xFF) as u8,
            (image_id & 0xFF) as u8,
        );

        // Encode placement_id in underline color
        let placement_id_color = Color::Rgb(
            ((placement_id >> 16) & 0xFF) as u8,
            ((placement_id >> 8) & 0xFF) as u8,
            (placement_id & 0xFF) as u8,
        );

        // Insert placeholders for each cell in the virtual placement area
        for row_offset in 0..rows {
            let row = start_row + row_offset;
            if row >= grid_rows {
                break; // Don't write past grid bounds
            }

            for col_offset in 0..cols {
                let col = start_col + col_offset;
                if col >= grid_cols {
                    break; // Don't write past grid bounds
                }

                // Create placeholder character with diacritics for position
                let placeholder_str = create_placeholder_with_diacritics(
                    row_offset.min(63) as u8,
                    col_offset.min(63) as u8,
                    msb,
                );

                // For now, just insert the base placeholder character
                // TODO: Handle diacritics properly in Cell structure
                let cell = Cell {
                    c: PLACEHOLDER_CHAR,
                    combining: Vec::new(),
                    fg: image_id_color,
                    bg: self.bg, // Use current background
                    underline_color: Some(placement_id_color),
                    flags: self.flags,
                    width: 1, // Placeholders are narrow
                };

                self.active_grid_mut().set(col, row, cell);

                debug::log(
                    debug::DebugLevel::Trace,
                    "KITTY_PLACEHOLDER",
                    &format!(
                        "Inserted placeholder at ({},{}) with placeholder_str.len()={}",
                        col,
                        row,
                        placeholder_str.len()
                    ),
                );
            }
        }

        debug::log(
            debug::DebugLevel::Debug,
            "KITTY_PLACEHOLDER",
            &format!(
                "Inserted {} placeholder cells",
                rows.min(grid_rows - start_row) * cols.min(grid_cols - start_col)
            ),
        );
    }

    /// Adjust graphics positions when scrolling up within a region
    ///
    /// When text scrolls up, graphics should scroll up with it.
    /// Graphics that scroll completely off the top are moved to scrollback.
    ///
    /// # Arguments
    /// * `n` - Number of lines scrolled
    /// * `top` - Top of scroll region (0-indexed)
    /// * `bottom` - Bottom of scroll region (0-indexed)
    pub(super) fn adjust_graphics_for_scroll_up(&mut self, n: usize, top: usize, bottom: usize) {
        // Get the current scrollback length from the grid (AFTER it has already scrolled)
        // We need to pass the OLD scrollback length (before scroll) to graphics store
        // Since the grid has already grown by `n` lines, subtract `n` to get the old length
        let scrollback_len = self.active_grid().scrollback_len();
        let old_scrollback_len = scrollback_len.saturating_sub(n);

        // Adjust graphics - pass old_scrollback_len so graphics are placed at the correct position
        // Graphics entering scrollback should be placed where the text they align with went
        self.graphics_store.adjust_for_scroll_up_with_scrollback(
            n,
            top,
            bottom,
            old_scrollback_len,
        );

        debug::log(
            debug::DebugLevel::Debug,
            "GRAPHICS",
            &format!(
                "Adjusted graphics for scroll_up: n={}, top={}, bottom={}, remaining graphics={}, scrollback={}, old_scrollback_len={} (current={})",
                n,
                top,
                bottom,
                self.graphics_store.graphics_count(),
                self.graphics_store.scrollback_count(),
                old_scrollback_len,
                scrollback_len
            ),
        );
    }

    /// Adjust graphics positions when scrolling down within a region
    ///
    /// When text scrolls down, graphics should scroll down with it.
    ///
    /// # Arguments
    /// * `n` - Number of lines scrolled
    /// * `top` - Top of scroll region (0-indexed)
    /// * `bottom` - Bottom of scroll region (0-indexed)
    pub(super) fn adjust_graphics_for_scroll_down(&mut self, n: usize, top: usize, bottom: usize) {
        self.graphics_store.adjust_for_scroll_down(n, top, bottom);

        debug::log(
            debug::DebugLevel::Debug,
            "GRAPHICS",
            &format!(
                "Adjusted graphics for scroll_down: n={}, top={}, bottom={}",
                n, top, bottom
            ),
        );
    }

    /// Handle iTerm2 inline image (OSC 1337)
    ///
    /// Supports:
    /// - Single-sequence: `File=name=<b64>;size=<bytes>;inline=1:<base64 data>`
    /// - Multi-part: `MultipartFile=...` followed by `FilePart=<chunk>` sequences
    pub(crate) fn handle_iterm_image(&mut self, data: &str) {
        // Handle MultipartFile (start of chunked transfer)
        if let Some(params) = data.strip_prefix("MultipartFile=") {
            self.handle_multipart_file_start(params);
            return;
        }

        // Handle FilePart (chunk of data in multipart transfer)
        if let Some(chunk) = data.strip_prefix("FilePart=") {
            self.handle_file_part(chunk);
            return;
        }

        // Handle single-sequence File= transfer
        self.handle_single_file_transfer(data);
    }

    /// Handle MultipartFile command (start of chunked transfer)
    fn handle_multipart_file_start(&mut self, params_str: &str) {
        use std::collections::HashMap;

        // Parse parameters: inline=1;size=280459;name=...
        let mut params = HashMap::new();
        for part in params_str.split(';') {
            if let Some((key, value)) = part.split_once('=') {
                params.insert(key.to_string(), value.to_string());
            }
        }

        // Validate inline=1 is present
        if params.get("inline") != Some(&"1".to_string()) {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                "MultipartFile requires inline=1",
            );
            return;
        }

        // Get expected size if provided
        let total_size = params.get("size").and_then(|s| s.parse::<usize>().ok());

        // Check size limit (use same limit as graphics store)
        if let Some(size) = total_size {
            let limits = self.graphics_store.limits();
            if size > limits.max_total_memory {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "MultipartFile rejected: size {} exceeds limit {}",
                        size, limits.max_total_memory
                    ),
                );
                return;
            }
        }

        // Initialize multipart state
        self.iterm_multipart_buffer = Some(crate::terminal::ITermMultipartState {
            params,
            chunks: Vec::new(),
            total_size,
            accumulated_size: 0,
        });
    }

    /// Handle FilePart command (chunk of data in multipart transfer)
    fn handle_file_part(&mut self, base64_chunk: &str) {
        // Check if we have an active multipart transfer
        let state = match self.iterm_multipart_buffer.as_mut() {
            Some(s) => s,
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    "FilePart received without MultipartFile",
                );
                return;
            }
        };

        // Decode the chunk to check its size
        let decoded_size = match base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            base64_chunk.as_bytes(),
        ) {
            Ok(decoded) => decoded.len(),
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!("FilePart base64 decode failed: {}", e),
                );
                self.iterm_multipart_buffer = None;
                return;
            }
        };

        // Check if adding this chunk would exceed size limit
        let new_accumulated = state.accumulated_size + decoded_size;
        if let Some(expected_size) = state.total_size {
            if new_accumulated > expected_size {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "FilePart rejected: accumulated {} + chunk {} > expected {}",
                        state.accumulated_size, decoded_size, expected_size
                    ),
                );
                self.iterm_multipart_buffer = None;
                return;
            }
        }

        // Add chunk and update size
        state.chunks.push(base64_chunk.to_string());
        state.accumulated_size = new_accumulated;

        // Check if transfer is complete
        let is_complete = if let Some(expected_size) = state.total_size {
            state.accumulated_size >= expected_size
        } else {
            // Without size parameter, we can't determine completion automatically
            // iTerm2 spec says size should be provided, so this is an error state
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                "MultipartFile missing size parameter - cannot determine completion",
            );
            self.iterm_multipart_buffer = None;
            return;
        };

        if is_complete {
            self.finalize_multipart_transfer();
        }
    }

    /// Finalize multipart transfer and process the complete image
    fn finalize_multipart_transfer(&mut self) {
        // Take the buffer state
        let state = match self.iterm_multipart_buffer.take() {
            Some(s) => s,
            None => return,
        };

        // Join all chunks into single base64 string
        let complete_data = state.chunks.join("");

        // Reconstruct File= format string with params
        let mut params_parts = Vec::new();
        for (key, value) in &state.params {
            params_parts.push(format!("{}={}", key, value));
        }
        let params_str = params_parts.join(";");
        let file_data = format!("File={}:{}", params_str, complete_data);

        // Process as single-file transfer
        self.handle_single_file_transfer(&file_data);
    }

    /// Handle single-sequence File= transfer
    fn handle_single_file_transfer(&mut self, data: &str) {
        use crate::graphics::iterm::ITermParser;

        // Split into params and image data at the colon
        let (params_str, image_data) = match data.split_once(':') {
            Some((p, d)) => (p, d),
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    "No colon separator in File= format",
                );
                return;
            }
        };

        // Must start with "File="
        if !params_str.starts_with("File=") {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                &format!("Unsupported OSC 1337 command: {}", params_str),
            );
            return;
        }

        let params_str = &params_str[5..]; // Remove "File=" prefix

        let mut parser = ITermParser::new();

        // Parse parameters
        if let Err(e) = parser.parse_params(params_str) {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                &format!("Failed to parse iTerm params: {}", e),
            );
            return;
        }

        // Set the base64 image data
        parser.set_data(image_data.as_bytes());

        // Get cursor position for graphic placement
        let position = (self.cursor.col, self.cursor.row);

        // Decode and create graphic
        match parser.decode_image(position) {
            Ok(mut graphic) => {
                // Set cell dimensions
                let (cell_w, cell_h) = self.cell_dimensions;
                graphic.set_cell_dimensions(cell_w, cell_h);

                // Calculate graphic height in terminal rows (ceiling division)
                let graphic_height_in_rows = graphic.height.div_ceil(cell_h as usize);

                // Move cursor to line below graphic (similar to Sixel behavior)
                let new_cursor_col = 0;
                let new_cursor_row = self.cursor.row.saturating_add(graphic_height_in_rows);

                // Check if we need to scroll
                let (_, rows) = self.size();
                if new_cursor_row >= rows {
                    // Graphic pushed cursor past bottom, need to scroll
                    let scroll_amount = new_cursor_row - rows + 1;
                    let scroll_top = self.scroll_region_top;
                    let scroll_bottom = self.scroll_region_bottom;

                    // Scroll the grid and existing graphics
                    self.active_grid_mut().scroll_region_up(
                        scroll_amount,
                        scroll_top,
                        scroll_bottom,
                    );
                    self.adjust_graphics_for_scroll_up(scroll_amount, scroll_top, scroll_bottom);

                    // Adjust new graphic's position for the scroll
                    let original_row = graphic.position.1;
                    let new_row = original_row.saturating_sub(scroll_amount);
                    graphic.position.1 = new_row;

                    // Track rows that scrolled off top
                    if scroll_amount > original_row {
                        graphic.scroll_offset_rows = scroll_amount - original_row;
                    }

                    self.cursor.row = rows - 1;
                    self.cursor.col = new_cursor_col;
                } else {
                    self.cursor.row = new_cursor_row;
                    self.cursor.col = new_cursor_col;
                }

                // Add to graphics store (limit enforced internally)
                self.graphics_store.add_graphic(graphic.clone());

                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "Added iTerm image at ({}, {}), size {}x{}, cursor moved to ({}, {})",
                        position.0,
                        position.1,
                        graphic.width,
                        graphic.height,
                        self.cursor.col,
                        self.cursor.row
                    ),
                );
            }
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!("Failed to decode iTerm image: {}", e),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{next_graphic_id, GraphicProtocol, TerminalGraphic};

    fn create_test_terminal() -> Terminal {
        Terminal::new(80, 24)
    }

    fn create_test_graphic(col: usize, row: usize, width: usize, height: usize) -> TerminalGraphic {
        TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::Sixel,
            (col, row),
            width,
            height,
            vec![], // Empty pixels for tests
        )
    }

    #[test]
    fn test_graphics_at_row_empty() {
        let term = create_test_terminal();
        let graphics = term.graphics_at_row(0);
        assert_eq!(graphics.len(), 0);
    }

    #[test]
    fn test_graphics_at_row_single_graphic() {
        let mut term = create_test_terminal();
        // Graphic at row 5 with height 4 pixels (occupies 2 terminal rows: 5 and 6)
        let graphic = create_test_graphic(0, 5, 10, 4);
        term.graphics_store.add_graphic(graphic);

        let graphics_row_5 = term.graphics_at_row(5);
        assert_eq!(graphics_row_5.len(), 1);

        let graphics_row_6 = term.graphics_at_row(6);
        assert_eq!(graphics_row_6.len(), 1);

        let graphics_row_7 = term.graphics_at_row(7);
        assert_eq!(graphics_row_7.len(), 0);
    }

    #[test]
    fn test_graphics_at_row_multiple_graphics() {
        let mut term = create_test_terminal();
        // Graphic 1: row 5, height 4 pixels (rows 5-6)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 4));
        // Graphic 2: row 10, height 6 pixels (rows 10-12)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 10, 10, 6));
        // Graphic 3: row 5, height 2 pixels (rows 5-5)
        term.graphics_store
            .add_graphic(create_test_graphic(20, 5, 10, 2));

        let graphics_row_5 = term.graphics_at_row(5);
        assert_eq!(graphics_row_5.len(), 2); // Graphics 1 and 3

        let graphics_row_10 = term.graphics_at_row(10);
        assert_eq!(graphics_row_10.len(), 1); // Only graphic 2

        let graphics_row_8 = term.graphics_at_row(8);
        assert_eq!(graphics_row_8.len(), 0); // No graphics
    }

    #[test]
    fn test_graphics_at_row_odd_height() {
        let mut term = create_test_terminal();
        // Graphic with height 5 pixels (occupies 3 terminal rows due to div_ceil)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 10, 10, 5));

        assert_eq!(term.graphics_at_row(10).len(), 1);
        assert_eq!(term.graphics_at_row(11).len(), 1);
        assert_eq!(term.graphics_at_row(12).len(), 1);
        assert_eq!(term.graphics_at_row(13).len(), 0);
    }

    #[test]
    fn test_graphics_count() {
        let mut term = create_test_terminal();
        assert_eq!(term.graphics_count(), 0);

        term.graphics_store
            .add_graphic(create_test_graphic(0, 0, 10, 10));
        assert_eq!(term.graphics_count(), 1);

        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 10));
        assert_eq!(term.graphics_count(), 2);
    }

    #[test]
    fn test_clear_graphics() {
        let mut term = create_test_terminal();
        term.graphics_store
            .add_graphic(create_test_graphic(0, 0, 10, 10));
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 10));
        assert_eq!(term.graphics_count(), 2);

        term.clear_graphics();
        assert_eq!(term.graphics_count(), 0);
        assert_eq!(term.all_graphics().len(), 0);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_basic() {
        let mut term = create_test_terminal();
        // Graphic at row 10
        term.graphics_store
            .add_graphic(create_test_graphic(0, 10, 10, 4));

        // Scroll up 3 lines in region 0-23
        term.adjust_graphics_for_scroll_up(3, 0, 23);

        assert_eq!(term.graphics_store.graphics_count(), 1);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 7); // Moved from 10 to 7
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_remove() {
        let mut term = create_test_terminal();
        // Graphic at row 2 will scroll off when scrolling up 5 lines
        term.graphics_store
            .add_graphic(create_test_graphic(0, 2, 10, 4));

        term.adjust_graphics_for_scroll_up(5, 0, 23);

        assert_eq!(term.graphics_store.graphics_count(), 0); // Graphic removed
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_partial_region() {
        let mut term = create_test_terminal();
        // Graphic at row 5 (inside scroll region 3-15)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 4));
        // Graphic at row 20 (outside scroll region)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 20, 10, 4));

        term.adjust_graphics_for_scroll_up(2, 3, 15);

        assert_eq!(term.graphics_store.graphics_count(), 2);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 3); // Moved from 5 to 3
        assert_eq!(term.graphics_store.all_graphics()[1].position.1, 20); // Unchanged
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_overlapping() {
        let mut term = create_test_terminal();
        // Graphic starts above scroll region but extends into it
        // Row 2, height 6 pixels (3 terminal rows: 2, 3, 4)
        // Scroll region is 3-15
        term.graphics_store
            .add_graphic(create_test_graphic(0, 2, 10, 6));

        term.adjust_graphics_for_scroll_up(2, 3, 15);

        // Graphic starts above region, so it stays at same position
        assert_eq!(term.graphics_store.graphics_count(), 1);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 2);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_basic() {
        let mut term = create_test_terminal();
        // Graphic at row 10
        term.graphics_store
            .add_graphic(create_test_graphic(0, 10, 10, 4));

        // Scroll down 3 lines in region 0-23
        term.adjust_graphics_for_scroll_down(3, 0, 23);

        assert_eq!(term.graphics_store.graphics_count(), 1);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 13); // Moved from 10 to 13
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_at_bottom() {
        let mut term = create_test_terminal();
        // Graphic at row 22 in region 0-23
        term.graphics_store
            .add_graphic(create_test_graphic(0, 22, 10, 4));

        // Scroll down 5 lines - graphic shouldn't move beyond bottom
        term.adjust_graphics_for_scroll_down(5, 0, 23);

        assert_eq!(term.graphics_store.graphics_count(), 1);
        // Graphic stays at 22 because new_row (27) > bottom (23)
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 22);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_partial_region() {
        let mut term = create_test_terminal();
        // Graphic at row 5 (inside scroll region 3-15)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 4));
        // Graphic at row 20 (outside scroll region)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 20, 10, 4));

        term.adjust_graphics_for_scroll_down(2, 3, 15);

        assert_eq!(term.graphics_store.graphics_count(), 2);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 7); // Moved from 5 to 7
        assert_eq!(term.graphics_store.all_graphics()[1].position.1, 20); // Unchanged
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_beyond_bottom() {
        let mut term = create_test_terminal();
        // Graphic at row 14 in scroll region 0-15
        term.graphics_store
            .add_graphic(create_test_graphic(0, 14, 10, 4));

        // Scroll down 3 lines - would go to row 17 which is beyond bottom (15)
        term.adjust_graphics_for_scroll_down(3, 0, 15);

        assert_eq!(term.graphics_store.graphics_count(), 1);
        assert_eq!(term.graphics_store.all_graphics()[0].position.1, 14); // Doesn't move
    }

    #[test]
    fn test_graphics_height_calculation() {
        let mut term = create_test_terminal();
        // Height 1 pixel = 1 terminal row
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 1));
        assert_eq!(term.graphics_at_row(5).len(), 1);
        assert_eq!(term.graphics_at_row(6).len(), 0);

        term.clear_graphics();

        // Height 2 pixels = 1 terminal row
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 2));
        assert_eq!(term.graphics_at_row(5).len(), 1);
        assert_eq!(term.graphics_at_row(6).len(), 0);

        term.clear_graphics();

        // Height 3 pixels = 2 terminal rows (div_ceil)
        term.graphics_store
            .add_graphic(create_test_graphic(0, 5, 10, 3));
        assert_eq!(term.graphics_at_row(5).len(), 1);
        assert_eq!(term.graphics_at_row(6).len(), 1);
        assert_eq!(term.graphics_at_row(7).len(), 0);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_tall_graphic_bottom_visible() {
        // Bug fix test: Tall graphics should remain if their bottom is still visible
        // This reproduces the snake.sixel issue: 450px (225 rows) graphic in 40-row terminal
        let mut term = Terminal::new(80, 40);

        // Create a tall graphic at row 0, height 450 pixels = 225 terminal rows
        // Bottom is at row 224
        term.graphics_store
            .add_graphic(create_test_graphic(0, 0, 600, 450));

        // Scroll up by 186 rows (simulating cursor advancing from 0 to 225, then scrolling back to fit)
        // After scroll: top would be at -186 (clamped to 0), bottom at 38 (visible!)
        term.adjust_graphics_for_scroll_up(186, 0, 39);

        // Graphic should still exist (bottom is visible)
        assert_eq!(
            term.graphics_store.graphics_count(),
            1,
            "Graphic should remain when bottom is visible"
        );

        // Position should be clamped to 0
        assert_eq!(
            term.graphics_store.all_graphics()[0].position.1,
            0,
            "Position should be clamped to 0"
        );

        // After clamping to position 0, graphic still has height 225 rows
        // So it spans rows 0-224, meaning ALL visible terminal rows (0-39) show the graphic
        assert!(
            !term.graphics_at_row(0).is_empty(),
            "Graphic should be visible at row 0"
        );
        assert!(
            !term.graphics_at_row(39).is_empty(),
            "Graphic should be visible at row 39"
        );

        // The graphic spans to row 224, so any row >= 225 would not show it
        // But our terminal only has 40 rows, so we can't test row 225
        // Instead verify the graphic height is still 225 rows
        assert_eq!(
            term.graphics_store.all_graphics()[0].height,
            450,
            "Graphic height should be unchanged"
        );

        // Verify scroll offset tracks how many rows scrolled off the top
        assert_eq!(
            term.graphics_store.all_graphics()[0].scroll_offset_rows,
            186,
            "Should track 186 rows scrolled off"
        );
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_tall_graphic_completely_off() {
        // Test that graphics are removed when bottom scrolls completely off
        let mut term = Terminal::new(80, 40);

        // Create a graphic at row 0, height 40 pixels = 20 terminal rows
        term.graphics_store
            .add_graphic(create_test_graphic(0, 0, 100, 40));

        // Scroll up by 25 rows (more than the graphic's height of 20 rows)
        // Bottom is at row 19, so 25 >= 20 means completely off screen
        term.adjust_graphics_for_scroll_up(25, 0, 39);

        // Graphic should be removed
        assert_eq!(
            term.graphics_store.graphics_count(),
            0,
            "Graphic should be removed when bottom scrolls off"
        );
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_tall_graphic_edge_case() {
        // Test edge case where scroll amount equals graphic bottom
        let mut term = Terminal::new(80, 40);

        // Create a graphic at row 0, height 40 pixels = 20 terminal rows
        // Bottom is at row 19
        term.graphics_store
            .add_graphic(create_test_graphic(0, 0, 100, 40));

        // Scroll up by exactly 20 rows (n >= graphic_bottom means remove)
        term.adjust_graphics_for_scroll_up(20, 0, 39);

        // Graphic should be removed (boundary condition)
        assert_eq!(
            term.graphics_store.graphics_count(),
            0,
            "Graphic should be removed when n >= bottom"
        );
    }
}
