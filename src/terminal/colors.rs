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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_terminal() -> Terminal {
        Terminal::new(80, 24)
    }

    #[test]
    fn test_default_ansi_palette() {
        let palette = Terminal::default_ansi_palette();

        // Test standard colors (0-7)
        assert_eq!(palette[0], Color::Rgb(0x14, 0x19, 0x1E)); // Black
        assert_eq!(palette[1], Color::Rgb(0xB4, 0x3C, 0x2A)); // Red
        assert_eq!(palette[2], Color::Rgb(0x00, 0x81, 0x5B)); // Green
        assert_eq!(palette[3], Color::Rgb(0xCF, 0xA5, 0x18)); // Yellow
        assert_eq!(palette[4], Color::Rgb(0x30, 0x65, 0xB8)); // Blue
        assert_eq!(palette[5], Color::Rgb(0x88, 0x18, 0xA3)); // Magenta
        assert_eq!(palette[6], Color::Rgb(0x00, 0x93, 0x99)); // Cyan
        assert_eq!(palette[7], Color::Rgb(0xE5, 0xE5, 0xE5)); // White

        // Test bright colors (8-15)
        assert_eq!(palette[8], Color::Rgb(0x68, 0x73, 0x78)); // Bright Black
        assert_eq!(palette[9], Color::Rgb(0xFF, 0x61, 0x48)); // Bright Red
        assert_eq!(palette[10], Color::Rgb(0x00, 0xC9, 0x84)); // Bright Green
        assert_eq!(palette[11], Color::Rgb(0xFF, 0xC5, 0x31)); // Bright Yellow
        assert_eq!(palette[12], Color::Rgb(0x4F, 0x9C, 0xFE)); // Bright Blue
        assert_eq!(palette[13], Color::Rgb(0xC5, 0x4F, 0xFF)); // Bright Magenta
        assert_eq!(palette[14], Color::Rgb(0x00, 0xCC, 0xCC)); // Bright Cyan
        assert_eq!(palette[15], Color::Rgb(0xFF, 0xFF, 0xFF)); // Bright White
    }

    #[test]
    fn test_default_fg_get_set() {
        let mut term = create_test_terminal();
        let original = term.default_fg();

        let new_color = Color::Rgb(255, 100, 50);
        term.set_default_fg(new_color);

        assert_eq!(term.default_fg(), new_color);
        assert_ne!(term.default_fg(), original);
    }

    #[test]
    fn test_default_bg_get_set() {
        let mut term = create_test_terminal();
        let original = term.default_bg();

        let new_color = Color::Rgb(10, 20, 30);
        term.set_default_bg(new_color);

        assert_eq!(term.default_bg(), new_color);
        assert_ne!(term.default_bg(), original);
    }

    #[test]
    fn test_cursor_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.cursor_color();

        let new_color = Color::Rgb(128, 200, 255);
        term.set_cursor_color(new_color);

        assert_eq!(term.cursor_color(), new_color);
        assert_ne!(term.cursor_color(), original);
    }

    #[test]
    fn test_link_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.link_color();

        let new_color = Color::Rgb(0, 0, 255);
        term.set_link_color(new_color);

        assert_eq!(term.link_color(), new_color);
        assert_ne!(term.link_color(), original);
    }

    #[test]
    fn test_bold_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.bold_color();

        let new_color = Color::Rgb(255, 255, 0);
        term.set_bold_color(new_color);

        assert_eq!(term.bold_color(), new_color);
        assert_ne!(term.bold_color(), original);
    }

    #[test]
    fn test_cursor_guide_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.cursor_guide_color();

        let new_color = Color::Rgb(100, 100, 100);
        term.set_cursor_guide_color(new_color);

        assert_eq!(term.cursor_guide_color(), new_color);
        assert_ne!(term.cursor_guide_color(), original);
    }

    #[test]
    fn test_badge_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.badge_color();

        let new_color = Color::Rgb(200, 50, 50);
        term.set_badge_color(new_color);

        assert_eq!(term.badge_color(), new_color);
        assert_ne!(term.badge_color(), original);
    }

    #[test]
    fn test_match_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.match_color();

        let new_color = Color::Rgb(255, 255, 100);
        term.set_match_color(new_color);

        assert_eq!(term.match_color(), new_color);
        assert_ne!(term.match_color(), original);
    }

    #[test]
    fn test_selection_bg_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.selection_bg_color();

        let new_color = Color::Rgb(64, 64, 128);
        term.set_selection_bg_color(new_color);

        assert_eq!(term.selection_bg_color(), new_color);
        assert_ne!(term.selection_bg_color(), original);
    }

    #[test]
    fn test_selection_fg_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.selection_fg_color();

        let new_color = Color::Rgb(255, 255, 255);
        term.set_selection_fg_color(new_color);

        assert_eq!(term.selection_fg_color(), new_color);
        assert_ne!(term.selection_fg_color(), original);
    }

    #[test]
    fn test_use_bold_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.use_bold_color();

        term.set_use_bold_color(!original);
        assert_eq!(term.use_bold_color(), !original);

        term.set_use_bold_color(original);
        assert_eq!(term.use_bold_color(), original);
    }

    #[test]
    fn test_use_underline_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.use_underline_color();

        term.set_use_underline_color(!original);
        assert_eq!(term.use_underline_color(), !original);

        term.set_use_underline_color(original);
        assert_eq!(term.use_underline_color(), original);
    }

    #[test]
    fn test_use_cursor_guide_get_set() {
        let mut term = create_test_terminal();
        let original = term.use_cursor_guide();

        term.set_use_cursor_guide(!original);
        assert_eq!(term.use_cursor_guide(), !original);

        term.set_use_cursor_guide(original);
        assert_eq!(term.use_cursor_guide(), original);
    }

    #[test]
    fn test_use_selected_text_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.use_selected_text_color();

        term.set_use_selected_text_color(!original);
        assert_eq!(term.use_selected_text_color(), !original);

        term.set_use_selected_text_color(original);
        assert_eq!(term.use_selected_text_color(), original);
    }

    #[test]
    fn test_smart_cursor_color_get_set() {
        let mut term = create_test_terminal();
        let original = term.smart_cursor_color();

        term.set_smart_cursor_color(!original);
        assert_eq!(term.smart_cursor_color(), !original);

        term.set_smart_cursor_color(original);
        assert_eq!(term.smart_cursor_color(), original);
    }

    #[test]
    fn test_set_ansi_palette_color_valid_indices() {
        let mut term = create_test_terminal();

        // Test all valid indices (0-15)
        for i in 0..16 {
            let color = Color::Rgb(
                (i as u8).wrapping_mul(10),
                (i as u8).wrapping_mul(12),
                (i as u8).wrapping_mul(8),
            );
            let result = term.set_ansi_palette_color(i, color);
            assert!(result.is_ok());
            assert_eq!(term.ansi_palette[i], color);
        }
    }

    #[test]
    fn test_set_ansi_palette_color_invalid_index() {
        let mut term = create_test_terminal();

        // Test invalid indices
        let color = Color::Rgb(255, 0, 0);

        let result = term.set_ansi_palette_color(16, color);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid palette index: 16 (must be 0-15)"
        );

        let result = term.set_ansi_palette_color(100, color);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid palette index: 100 (must be 0-15)"
        );

        let result = term.set_ansi_palette_color(1000, color);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_ansi_palette_color_boundary() {
        let mut term = create_test_terminal();

        // Test boundary indices
        let color = Color::Rgb(128, 128, 128);

        // Index 0 should work
        assert!(term.set_ansi_palette_color(0, color).is_ok());

        // Index 15 should work
        assert!(term.set_ansi_palette_color(15, color).is_ok());

        // Index 16 should fail
        assert!(term.set_ansi_palette_color(16, color).is_err());
    }

    #[test]
    fn test_ansi_palette_preservation() {
        let mut term = create_test_terminal();

        // Modify some palette colors
        let red = Color::Rgb(255, 0, 0);
        let green = Color::Rgb(0, 255, 0);
        let blue = Color::Rgb(0, 0, 255);

        term.set_ansi_palette_color(1, red).unwrap();
        term.set_ansi_palette_color(2, green).unwrap();
        term.set_ansi_palette_color(4, blue).unwrap();

        // Verify they are preserved
        assert_eq!(term.ansi_palette[1], red);
        assert_eq!(term.ansi_palette[2], green);
        assert_eq!(term.ansi_palette[4], blue);

        // Verify others remain unchanged (default palette)
        let default_palette = Terminal::default_ansi_palette();
        assert_eq!(term.ansi_palette[0], default_palette[0]);
        assert_eq!(term.ansi_palette[3], default_palette[3]);
        assert_eq!(term.ansi_palette[5], default_palette[5]);
    }

    #[test]
    fn test_multiple_color_settings() {
        let mut term = create_test_terminal();

        // Set multiple colors at once
        let fg = Color::Rgb(255, 255, 255);
        let bg = Color::Rgb(0, 0, 0);
        let cursor = Color::Rgb(255, 0, 0);
        let link = Color::Rgb(0, 0, 255);

        term.set_default_fg(fg);
        term.set_default_bg(bg);
        term.set_cursor_color(cursor);
        term.set_link_color(link);

        // Verify all are set correctly
        assert_eq!(term.default_fg(), fg);
        assert_eq!(term.default_bg(), bg);
        assert_eq!(term.cursor_color(), cursor);
        assert_eq!(term.link_color(), link);
    }

    #[test]
    fn test_color_flags_independence() {
        let mut term = create_test_terminal();

        // Test that boolean flags are independent
        term.set_use_bold_color(true);
        term.set_use_underline_color(false);
        term.set_use_cursor_guide(true);
        term.set_use_selected_text_color(false);
        term.set_smart_cursor_color(true);

        assert!(term.use_bold_color());
        assert!(!term.use_underline_color());
        assert!(term.use_cursor_guide());
        assert!(!term.use_selected_text_color());
        assert!(term.smart_cursor_color());
    }

    #[test]
    fn test_ansi_palette_indexed_colors() {
        let mut term = create_test_terminal();

        // Test that we can set and retrieve all 16 ANSI colors
        let test_colors: Vec<Color> = (0..16_u8)
            .map(|i| {
                let r = i.saturating_mul(16);
                let g = i.saturating_mul(15);
                let b = 255_u8.saturating_sub(i.saturating_mul(16));
                Color::Rgb(r, g, b)
            })
            .collect();

        for (i, color) in test_colors.iter().enumerate() {
            term.set_ansi_palette_color(i, *color).unwrap();
        }

        for (i, expected_color) in test_colors.iter().enumerate() {
            assert_eq!(term.ansi_palette[i], *expected_color);
        }
    }
}
