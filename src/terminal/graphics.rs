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

    /// Clear all graphics
    pub fn clear_graphics(&mut self) {
        self.graphics_store.clear();
    }

    /// Get mutable access to graphics store
    pub fn graphics_store_mut(&mut self) -> &mut crate::graphics::GraphicsStore {
        &mut self.graphics_store
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
        self.graphics_store.adjust_for_scroll_up(n, top, bottom);

        debug::log(
            debug::DebugLevel::Debug,
            "GRAPHICS",
            &format!(
                "Adjusted graphics for scroll_up: n={}, top={}, bottom={}, remaining graphics={}",
                n,
                top,
                bottom,
                self.graphics_store.graphics_count()
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
    /// Format: File=name=<b64>;size=<bytes>;inline=1:<base64 data>
    pub(crate) fn handle_iterm_image(&mut self, data: &str) {
        use crate::graphics::iterm::ITermParser;

        // Split into params and image data at the colon
        let (params_str, image_data) = match data.split_once(':') {
            Some((p, d)) => (p, d),
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    "No image data found in OSC 1337",
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

                // Add to graphics store (limit enforced internally)
                self.graphics_store.add_graphic(graphic.clone());

                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "Added iTerm image at ({}, {}), size {}x{}",
                        position.0, position.1, graphic.width, graphic.height
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
