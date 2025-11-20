//! Sixel graphics management
//!
//! Handles Sixel graphics storage, retrieval, and position adjustments during scrolling.

use crate::debug;
use crate::sixel;
use crate::terminal::Terminal;

impl Terminal {
    /// Get graphics at a specific row
    pub fn graphics_at_row(&self, row: usize) -> Vec<&sixel::SixelGraphic> {
        self.graphics
            .iter()
            .filter(|g| {
                let start_row = g.position.1;
                // Each terminal row displays 2 pixel rows (using Unicode half-blocks)
                let end_row = start_row + g.height.div_ceil(2);
                row >= start_row && row < end_row
            })
            .collect()
    }

    /// Get all graphics
    pub fn graphics(&self) -> &[sixel::SixelGraphic] {
        &self.graphics
    }

    /// Get total graphics count
    pub fn graphics_count(&self) -> usize {
        self.graphics.len()
    }

    /// Clear all graphics
    pub fn clear_graphics(&mut self) {
        self.graphics.clear();
    }

    /// Adjust graphics positions when scrolling up within a region
    ///
    /// When text scrolls up, graphics should scroll up with it.
    /// Graphics that scroll completely off the top are removed.
    ///
    /// # Arguments
    /// * `n` - Number of lines scrolled
    /// * `top` - Top of scroll region (0-indexed)
    /// * `bottom` - Bottom of scroll region (0-indexed)
    pub(super) fn adjust_graphics_for_scroll_up(&mut self, n: usize, top: usize, bottom: usize) {
        // Filter and adjust graphics
        self.graphics.retain_mut(|graphic| {
            let graphic_row = graphic.position.1;
            // Calculate the graphic's extent (how many terminal rows it occupies)
            // Each terminal row displays 2 pixel rows using Unicode half-blocks
            let graphic_height_in_rows = graphic.height.div_ceil(2);
            let graphic_bottom = graphic_row + graphic_height_in_rows;

            // Check if graphic is within or overlaps the scroll region
            if graphic_bottom > top && graphic_row <= bottom {
                // Graphic is affected by scrolling
                if graphic_row >= top {
                    // Graphic starts within scroll region - adjust its position
                    // Adjust position (saturating_sub will clamp to 0 if top goes negative)
                    let new_position = graphic_row.saturating_sub(n);
                    // Track how many rows scrolled off if position was clamped to 0
                    let additional_scroll = if n > graphic_row {
                        // Graphic's top scrolled off - track the offset
                        n - graphic_row
                    } else {
                        0
                    };
                    graphic.scroll_offset_rows =
                        graphic.scroll_offset_rows.saturating_add(additional_scroll);
                    graphic.position.1 = new_position;

                    // Check if the graphic's BOTTOM has scrolled completely off
                    // Total scrolled = scroll_offset_rows, graphic height = graphic_height_in_rows
                    if graphic.scroll_offset_rows >= graphic_height_in_rows {
                        // Entire graphic has scrolled off - remove it
                        return false;
                    }
                } else {
                    // Graphic starts above scroll region but extends into it
                    // Keep it at the same position (only content within region scrolls)
                }
            }
            // Keep graphics outside scroll region or that haven't scrolled off
            true
        });

        debug::log(
            debug::DebugLevel::Debug,
            "GRAPHICS",
            &format!(
                "Adjusted graphics for scroll_up: n={}, top={}, bottom={}, remaining graphics={}",
                n,
                top,
                bottom,
                self.graphics.len()
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
        // Adjust graphics within the scroll region
        for graphic in &mut self.graphics {
            let graphic_row = graphic.position.1;
            let graphic_height_in_rows = graphic.height.div_ceil(2);
            let graphic_bottom = graphic_row + graphic_height_in_rows;

            // Check if graphic is within or overlaps the scroll region
            if graphic_bottom > top && graphic_row <= bottom {
                // Graphic is affected by scrolling
                if graphic_row >= top && graphic_row <= bottom {
                    // Graphic starts within scroll region - move it down
                    // Don't scroll beyond the bottom of the region
                    let new_row = graphic_row + n;
                    if new_row <= bottom {
                        graphic.position.1 = new_row;
                    }
                }
            }
        }

        debug::log(
            debug::DebugLevel::Debug,
            "GRAPHICS",
            &format!(
                "Adjusted graphics for scroll_down: n={}, top={}, bottom={}",
                n, top, bottom
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sixel::SixelGraphic;
    use std::collections::HashMap;

    fn create_test_terminal() -> Terminal {
        Terminal::new(80, 24)
    }

    fn create_test_graphic(col: usize, row: usize, width: usize, height: usize) -> SixelGraphic {
        SixelGraphic {
            position: (col, row),
            width,
            height,
            palette: HashMap::new(),
            pixels: vec![],
            cell_dimensions: None,
            scroll_offset_rows: 0,
        }
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
        term.graphics.push(graphic);

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
        term.graphics.push(create_test_graphic(0, 5, 10, 4));
        // Graphic 2: row 10, height 6 pixels (rows 10-12)
        term.graphics.push(create_test_graphic(0, 10, 10, 6));
        // Graphic 3: row 5, height 2 pixels (rows 5-5)
        term.graphics.push(create_test_graphic(20, 5, 10, 2));

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
        term.graphics.push(create_test_graphic(0, 10, 10, 5));

        assert_eq!(term.graphics_at_row(10).len(), 1);
        assert_eq!(term.graphics_at_row(11).len(), 1);
        assert_eq!(term.graphics_at_row(12).len(), 1);
        assert_eq!(term.graphics_at_row(13).len(), 0);
    }

    #[test]
    fn test_graphics_count() {
        let mut term = create_test_terminal();
        assert_eq!(term.graphics_count(), 0);

        term.graphics.push(create_test_graphic(0, 0, 10, 10));
        assert_eq!(term.graphics_count(), 1);

        term.graphics.push(create_test_graphic(0, 5, 10, 10));
        assert_eq!(term.graphics_count(), 2);
    }

    #[test]
    fn test_clear_graphics() {
        let mut term = create_test_terminal();
        term.graphics.push(create_test_graphic(0, 0, 10, 10));
        term.graphics.push(create_test_graphic(0, 5, 10, 10));
        assert_eq!(term.graphics_count(), 2);

        term.clear_graphics();
        assert_eq!(term.graphics_count(), 0);
        assert_eq!(term.graphics().len(), 0);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_basic() {
        let mut term = create_test_terminal();
        // Graphic at row 10
        term.graphics.push(create_test_graphic(0, 10, 10, 4));

        // Scroll up 3 lines in region 0-23
        term.adjust_graphics_for_scroll_up(3, 0, 23);

        assert_eq!(term.graphics.len(), 1);
        assert_eq!(term.graphics[0].position.1, 7); // Moved from 10 to 7
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_remove() {
        let mut term = create_test_terminal();
        // Graphic at row 2 will scroll off when scrolling up 5 lines
        term.graphics.push(create_test_graphic(0, 2, 10, 4));

        term.adjust_graphics_for_scroll_up(5, 0, 23);

        assert_eq!(term.graphics.len(), 0); // Graphic removed
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_partial_region() {
        let mut term = create_test_terminal();
        // Graphic at row 5 (inside scroll region 3-15)
        term.graphics.push(create_test_graphic(0, 5, 10, 4));
        // Graphic at row 20 (outside scroll region)
        term.graphics.push(create_test_graphic(0, 20, 10, 4));

        term.adjust_graphics_for_scroll_up(2, 3, 15);

        assert_eq!(term.graphics.len(), 2);
        assert_eq!(term.graphics[0].position.1, 3); // Moved from 5 to 3
        assert_eq!(term.graphics[1].position.1, 20); // Unchanged
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_overlapping() {
        let mut term = create_test_terminal();
        // Graphic starts above scroll region but extends into it
        // Row 2, height 6 pixels (3 terminal rows: 2, 3, 4)
        // Scroll region is 3-15
        term.graphics.push(create_test_graphic(0, 2, 10, 6));

        term.adjust_graphics_for_scroll_up(2, 3, 15);

        // Graphic starts above region, so it stays at same position
        assert_eq!(term.graphics.len(), 1);
        assert_eq!(term.graphics[0].position.1, 2);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_basic() {
        let mut term = create_test_terminal();
        // Graphic at row 10
        term.graphics.push(create_test_graphic(0, 10, 10, 4));

        // Scroll down 3 lines in region 0-23
        term.adjust_graphics_for_scroll_down(3, 0, 23);

        assert_eq!(term.graphics.len(), 1);
        assert_eq!(term.graphics[0].position.1, 13); // Moved from 10 to 13
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_at_bottom() {
        let mut term = create_test_terminal();
        // Graphic at row 22 in region 0-23
        term.graphics.push(create_test_graphic(0, 22, 10, 4));

        // Scroll down 5 lines - graphic shouldn't move beyond bottom
        term.adjust_graphics_for_scroll_down(5, 0, 23);

        assert_eq!(term.graphics.len(), 1);
        // Graphic stays at 22 because new_row (27) > bottom (23)
        assert_eq!(term.graphics[0].position.1, 22);
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_partial_region() {
        let mut term = create_test_terminal();
        // Graphic at row 5 (inside scroll region 3-15)
        term.graphics.push(create_test_graphic(0, 5, 10, 4));
        // Graphic at row 20 (outside scroll region)
        term.graphics.push(create_test_graphic(0, 20, 10, 4));

        term.adjust_graphics_for_scroll_down(2, 3, 15);

        assert_eq!(term.graphics.len(), 2);
        assert_eq!(term.graphics[0].position.1, 7); // Moved from 5 to 7
        assert_eq!(term.graphics[1].position.1, 20); // Unchanged
    }

    #[test]
    fn test_adjust_graphics_for_scroll_down_beyond_bottom() {
        let mut term = create_test_terminal();
        // Graphic at row 14 in scroll region 0-15
        term.graphics.push(create_test_graphic(0, 14, 10, 4));

        // Scroll down 3 lines - would go to row 17 which is beyond bottom (15)
        term.adjust_graphics_for_scroll_down(3, 0, 15);

        assert_eq!(term.graphics.len(), 1);
        assert_eq!(term.graphics[0].position.1, 14); // Doesn't move
    }

    #[test]
    fn test_graphics_height_calculation() {
        let mut term = create_test_terminal();
        // Height 1 pixel = 1 terminal row
        term.graphics.push(create_test_graphic(0, 5, 10, 1));
        assert_eq!(term.graphics_at_row(5).len(), 1);
        assert_eq!(term.graphics_at_row(6).len(), 0);

        term.clear_graphics();

        // Height 2 pixels = 1 terminal row
        term.graphics.push(create_test_graphic(0, 5, 10, 2));
        assert_eq!(term.graphics_at_row(5).len(), 1);
        assert_eq!(term.graphics_at_row(6).len(), 0);

        term.clear_graphics();

        // Height 3 pixels = 2 terminal rows (div_ceil)
        term.graphics.push(create_test_graphic(0, 5, 10, 3));
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
        term.graphics.push(create_test_graphic(0, 0, 600, 450));

        // Scroll up by 186 rows (simulating cursor advancing from 0 to 225, then scrolling back to fit)
        // After scroll: top would be at -186 (clamped to 0), bottom at 38 (visible!)
        term.adjust_graphics_for_scroll_up(186, 0, 39);

        // Graphic should still exist (bottom is visible)
        assert_eq!(
            term.graphics.len(),
            1,
            "Graphic should remain when bottom is visible"
        );

        // Position should be clamped to 0
        assert_eq!(
            term.graphics[0].position.1, 0,
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
            term.graphics[0].height, 450,
            "Graphic height should be unchanged"
        );

        // Verify scroll offset tracks how many rows scrolled off the top
        assert_eq!(
            term.graphics[0].scroll_offset_rows, 186,
            "Should track 186 rows scrolled off"
        );
    }

    #[test]
    fn test_adjust_graphics_for_scroll_up_tall_graphic_completely_off() {
        // Test that graphics are removed when bottom scrolls completely off
        let mut term = Terminal::new(80, 40);

        // Create a graphic at row 0, height 40 pixels = 20 terminal rows
        term.graphics.push(create_test_graphic(0, 0, 100, 40));

        // Scroll up by 25 rows (more than the graphic's height of 20 rows)
        // Bottom is at row 19, so 25 >= 20 means completely off screen
        term.adjust_graphics_for_scroll_up(25, 0, 39);

        // Graphic should be removed
        assert_eq!(
            term.graphics.len(),
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
        term.graphics.push(create_test_graphic(0, 0, 100, 40));

        // Scroll up by exactly 20 rows (n >= graphic_bottom means remove)
        term.adjust_graphics_for_scroll_up(20, 0, 39);

        // Graphic should be removed (boundary condition)
        assert_eq!(
            term.graphics.len(),
            0,
            "Graphic should be removed when n >= bottom"
        );
    }
}
