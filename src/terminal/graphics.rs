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
                    if n > graphic_row {
                        // Graphic scrolls completely off top - remove it
                        return false;
                    }
                    graphic.position.1 = graphic_row.saturating_sub(n);
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
