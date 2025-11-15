//! Terminal color configuration and management
//!
//! Handles terminal color settings including:
//! - ANSI palette (16 colors)
//! - Default foreground/background colors
//! - Cursor color
//! - Selection colors
//! - Link colors
//! - Bold text colors
//! - Color mode flags

use crate::color::Color;
use crate::terminal::Terminal;

impl Terminal {
    /// Get default ANSI color palette
    pub(super) fn default_ansi_palette() -> [Color; 16] {
        [
            // Standard colors (0-7)
            Color::Rgb(0x14, 0x19, 0x1E), // 0: Black
            Color::Rgb(0xB4, 0x3C, 0x2A), // 1: Red
            Color::Rgb(0x00, 0x81, 0x5B), // 2: Green
            Color::Rgb(0xCF, 0xA5, 0x18), // 3: Yellow
            Color::Rgb(0x30, 0x65, 0xB8), // 4: Blue
            Color::Rgb(0x88, 0x18, 0xA3), // 5: Magenta
            Color::Rgb(0x00, 0x93, 0x99), // 6: Cyan
            Color::Rgb(0xE5, 0xE5, 0xE5), // 7: White
            // Bright colors (8-15)
            Color::Rgb(0x68, 0x73, 0x78), // 8: Bright Black
            Color::Rgb(0xFF, 0x61, 0x48), // 9: Bright Red
            Color::Rgb(0x00, 0xC9, 0x84), // 10: Bright Green
            Color::Rgb(0xFF, 0xC5, 0x31), // 11: Bright Yellow
            Color::Rgb(0x4F, 0x9C, 0xFE), // 12: Bright Blue
            Color::Rgb(0xC5, 0x4F, 0xFF), // 13: Bright Magenta
            Color::Rgb(0x00, 0xCC, 0xCC), // 14: Bright Cyan
            Color::Rgb(0xFF, 0xFF, 0xFF), // 15: Bright White
        ]
    }

    /// Get default foreground color (OSC 10)
    pub fn default_fg(&self) -> Color {
        self.default_fg
    }

    /// Set default foreground color (OSC 10)
    pub fn set_default_fg(&mut self, color: Color) {
        self.default_fg = color;
    }

    /// Get default background color (OSC 11)
    pub fn default_bg(&self) -> Color {
        self.default_bg
    }

    /// Set default background color (OSC 11)
    pub fn set_default_bg(&mut self, color: Color) {
        self.default_bg = color;
    }

    /// Get cursor color (OSC 12)
    pub fn cursor_color(&self) -> Color {
        self.cursor_color
    }

    /// Set cursor color (OSC 12)
    pub fn set_cursor_color(&mut self, color: Color) {
        self.cursor_color = color;
    }

    /// Get link/hyperlink color
    pub fn link_color(&self) -> Color {
        self.link_color
    }

    /// Set link/hyperlink color
    pub fn set_link_color(&mut self, color: Color) {
        self.link_color = color;
    }

    /// Get bold text custom color
    pub fn bold_color(&self) -> Color {
        self.bold_color
    }

    /// Set bold text custom color
    pub fn set_bold_color(&mut self, color: Color) {
        self.bold_color = color;
    }

    /// Get cursor guide color
    pub fn cursor_guide_color(&self) -> Color {
        self.cursor_guide_color
    }

    /// Set cursor guide color
    pub fn set_cursor_guide_color(&mut self, color: Color) {
        self.cursor_guide_color = color;
    }

    /// Get badge color
    pub fn badge_color(&self) -> Color {
        self.badge_color
    }

    /// Set badge color
    pub fn set_badge_color(&mut self, color: Color) {
        self.badge_color = color;
    }

    /// Get match/search highlight color
    pub fn match_color(&self) -> Color {
        self.match_color
    }

    /// Set match/search highlight color
    pub fn set_match_color(&mut self, color: Color) {
        self.match_color = color;
    }

    /// Get selection background color
    pub fn selection_bg_color(&self) -> Color {
        self.selection_bg_color
    }

    /// Set selection background color
    pub fn set_selection_bg_color(&mut self, color: Color) {
        self.selection_bg_color = color;
    }

    /// Get selection foreground/text color
    pub fn selection_fg_color(&self) -> Color {
        self.selection_fg_color
    }

    /// Set selection foreground/text color
    pub fn set_selection_fg_color(&mut self, color: Color) {
        self.selection_fg_color = color;
    }

    /// Get whether to use custom bold color
    pub fn use_bold_color(&self) -> bool {
        self.use_bold_color
    }

    /// Set whether to use custom bold color
    pub fn set_use_bold_color(&mut self, use_bold: bool) {
        self.use_bold_color = use_bold;
    }

    /// Get whether to use custom underline color
    pub fn use_underline_color(&self) -> bool {
        self.use_underline_color
    }

    /// Set whether to use custom underline color
    pub fn set_use_underline_color(&mut self, use_underline: bool) {
        self.use_underline_color = use_underline;
    }

    /// Get whether to show cursor guide
    pub fn use_cursor_guide(&self) -> bool {
        self.use_cursor_guide
    }

    /// Set whether to show cursor guide
    pub fn set_use_cursor_guide(&mut self, use_guide: bool) {
        self.use_cursor_guide = use_guide;
    }

    /// Get whether to use custom selected text color
    pub fn use_selected_text_color(&self) -> bool {
        self.use_selected_text_color
    }

    /// Set whether to use custom selected text color
    pub fn set_use_selected_text_color(&mut self, use_selected: bool) {
        self.use_selected_text_color = use_selected;
    }

    /// Get whether smart cursor color is enabled
    pub fn smart_cursor_color(&self) -> bool {
        self.smart_cursor_color
    }

    /// Set whether smart cursor color is enabled
    pub fn set_smart_cursor_color(&mut self, smart_cursor: bool) {
        self.smart_cursor_color = smart_cursor;
    }

    /// Set ANSI palette color (0-15)
    ///
    /// # Arguments
    /// * `index` - Palette index (0-15)
    /// * `color` - RGB color
    ///
    /// # Returns
    /// Ok(()) if index is valid, Err if index >= 16
    pub fn set_ansi_palette_color(&mut self, index: usize, color: Color) -> Result<(), String> {
        if index >= 16 {
            return Err(format!("Invalid palette index: {} (must be 0-15)", index));
        }
        self.ansi_palette[index] = color;
        Ok(())
    }
}
