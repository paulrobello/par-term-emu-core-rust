//! Sixel limits, cell pixel dimensions, and graphics animation (ARC-003).

use super::Terminal;
use crate::sixel;

impl Terminal {
    /// Get current Sixel resource limits
    pub fn sixel_limits(&self) -> sixel::SixelLimits {
        self.graphics.sixel_limits
    }

    /// Set Sixel resource limits (pixels and repeat count).
    ///
    /// Limits are clamped to safe hard maxima defined in `sixel.rs` and
    /// applied to new Sixel parser instances created for subsequent DCS
    /// sequences.
    pub fn set_sixel_limits(&mut self, max_width: usize, max_height: usize, max_repeat: usize) {
        self.graphics.sixel_limits = sixel::SixelLimits::new(max_width, max_height, max_repeat);
    }

    /// Get cell dimensions in pixels (width, height)
    ///
    /// Used for sixel graphics scroll calculations.
    /// Default is (1, 2) for TUI half-block rendering.
    pub fn cell_dimensions(&self) -> (u32, u32) {
        self.graphics.cell_dimensions
    }

    /// Set cell dimensions in pixels (width, height)
    ///
    /// Pixel-based renderers should call this with actual cell dimensions
    /// so sixel graphics scroll correctly. TUI renderers using half-blocks
    /// should use the default (1, 2).
    pub fn set_cell_dimensions(&mut self, width: u32, height: u32) {
        self.graphics.cell_dimensions = (width.max(1), height.max(1));
    }

    /// Get the maximum number of graphics retained for this terminal
    pub fn max_sixel_graphics(&self) -> usize {
        self.graphics.graphics_store.limits().max_graphics_count
    }

    /// Set the maximum number of graphics retained for this terminal.
    ///
    /// The value is clamped to a safe range and applies to graphics created
    /// after the change. If the new limit is lower than the current number of
    /// graphics, the oldest graphics are dropped to respect the limit.
    pub fn set_max_sixel_graphics(&mut self, max_graphics: usize) {
        use crate::sixel::SIXEL_HARD_MAX_GRAPHICS;

        let clamped = max_graphics.clamp(1, SIXEL_HARD_MAX_GRAPHICS);
        self.graphics.graphics_store.set_max_graphics(clamped);
    }

    /// Get the number of graphics dropped due to limits
    pub fn dropped_sixel_graphics(&self) -> usize {
        self.graphics.graphics_store.dropped_count()
    }

    /// Update all Kitty graphics animations and return list of image IDs that changed frames
    ///
    /// This method should be called regularly (e.g., 60Hz) to advance animation frames.
    /// It returns a list of image IDs whose frames changed, allowing frontends to
    /// selectively refresh only graphics that were updated.
    ///
    /// Returns:
    ///     List of image IDs that changed frames
    pub fn update_animations(&mut self) -> Vec<u32> {
        self.graphics.graphics_store.update_animations()
    }

    /// Get Sixel statistics for this terminal.
    ///
    /// Returns:
    /// - limits: SixelLimits (max width/height/repeat)
    /// - max_graphics: maximum number of retained graphics
    /// - current_graphics: current number of graphics stored
    /// - dropped_graphics: number of graphics dropped due to limits
    pub fn sixel_stats(&self) -> (sixel::SixelLimits, usize, usize, usize) {
        let limits = self.graphics.sixel_limits;
        let max_graphics = self.graphics.graphics_store.limits().max_graphics_count;
        let current_graphics = self.graphics.graphics_store.graphics_count();
        let dropped_graphics = self.graphics.graphics_store.dropped_count();
        (limits, max_graphics, current_graphics, dropped_graphics)
    }
}
