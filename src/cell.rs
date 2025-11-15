use crate::color::Color;
use bitflags::bitflags;

/// Underline style for text decoration (SGR 4:x)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    /// No underline
    #[default]
    None,
    /// Straight/single underline (default, SGR 4 or 4:1)
    Straight,
    /// Double underline (SGR 4:2)
    Double,
    /// Curly underline (SGR 4:3) - used for spell check, errors
    Curly,
    /// Dotted underline (SGR 4:4)
    Dotted,
    /// Dashed underline (SGR 4:5)
    Dashed,
}

bitflags! {
    /// Bitflags for cell text attributes
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CellBitflags: u16 {
        const BOLD = 1 << 0;
        const DIM = 1 << 1;
        const ITALIC = 1 << 2;
        const UNDERLINE = 1 << 3;
        const BLINK = 1 << 4;
        const REVERSE = 1 << 5;
        const HIDDEN = 1 << 6;
        const STRIKETHROUGH = 1 << 7;
        const OVERLINE = 1 << 8;
        const GUARDED = 1 << 9;
        const WIDE_CHAR = 1 << 10;
        const WIDE_CHAR_SPACER = 1 << 11;
    }
}

/// Flags for cell attributes (optimized with bitflags)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellFlags {
    /// Bitflags for boolean attributes
    bits: CellBitflags,
    /// Underline style (SGR 4:x)
    pub underline_style: UnderlineStyle,
    /// Hyperlink ID (reference to URL in Terminal's hyperlinks HashMap)
    pub hyperlink_id: Option<u32>,
}

impl Default for CellFlags {
    fn default() -> Self {
        Self {
            bits: CellBitflags::empty(),
            underline_style: UnderlineStyle::None,
            hyperlink_id: None,
        }
    }
}

impl CellFlags {
    // Getter methods for each flag
    #[inline]
    pub fn bold(&self) -> bool {
        self.bits.contains(CellBitflags::BOLD)
    }

    #[inline]
    pub fn dim(&self) -> bool {
        self.bits.contains(CellBitflags::DIM)
    }

    #[inline]
    pub fn italic(&self) -> bool {
        self.bits.contains(CellBitflags::ITALIC)
    }

    #[inline]
    pub fn underline(&self) -> bool {
        self.bits.contains(CellBitflags::UNDERLINE)
    }

    #[inline]
    pub fn blink(&self) -> bool {
        self.bits.contains(CellBitflags::BLINK)
    }

    #[inline]
    pub fn reverse(&self) -> bool {
        self.bits.contains(CellBitflags::REVERSE)
    }

    #[inline]
    pub fn hidden(&self) -> bool {
        self.bits.contains(CellBitflags::HIDDEN)
    }

    #[inline]
    pub fn strikethrough(&self) -> bool {
        self.bits.contains(CellBitflags::STRIKETHROUGH)
    }

    #[inline]
    pub fn overline(&self) -> bool {
        self.bits.contains(CellBitflags::OVERLINE)
    }

    #[inline]
    pub fn guarded(&self) -> bool {
        self.bits.contains(CellBitflags::GUARDED)
    }

    #[inline]
    pub fn wide_char(&self) -> bool {
        self.bits.contains(CellBitflags::WIDE_CHAR)
    }

    #[inline]
    pub fn wide_char_spacer(&self) -> bool {
        self.bits.contains(CellBitflags::WIDE_CHAR_SPACER)
    }

    // Setter methods for each flag
    #[inline]
    pub fn set_bold(&mut self, value: bool) {
        self.bits.set(CellBitflags::BOLD, value);
    }

    #[inline]
    pub fn set_dim(&mut self, value: bool) {
        self.bits.set(CellBitflags::DIM, value);
    }

    #[inline]
    pub fn set_italic(&mut self, value: bool) {
        self.bits.set(CellBitflags::ITALIC, value);
    }

    #[inline]
    pub fn set_underline(&mut self, value: bool) {
        self.bits.set(CellBitflags::UNDERLINE, value);
    }

    #[inline]
    pub fn set_blink(&mut self, value: bool) {
        self.bits.set(CellBitflags::BLINK, value);
    }

    #[inline]
    pub fn set_reverse(&mut self, value: bool) {
        self.bits.set(CellBitflags::REVERSE, value);
    }

    #[inline]
    pub fn set_hidden(&mut self, value: bool) {
        self.bits.set(CellBitflags::HIDDEN, value);
    }

    #[inline]
    pub fn set_strikethrough(&mut self, value: bool) {
        self.bits.set(CellBitflags::STRIKETHROUGH, value);
    }

    #[inline]
    pub fn set_overline(&mut self, value: bool) {
        self.bits.set(CellBitflags::OVERLINE, value);
    }

    #[inline]
    pub fn set_guarded(&mut self, value: bool) {
        self.bits.set(CellBitflags::GUARDED, value);
    }

    #[inline]
    pub fn set_wide_char(&mut self, value: bool) {
        self.bits.set(CellBitflags::WIDE_CHAR, value);
    }

    #[inline]
    pub fn set_wide_char_spacer(&mut self, value: bool) {
        self.bits.set(CellBitflags::WIDE_CHAR_SPACER, value);
    }
}

/// A single cell in the terminal grid
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character stored in this cell
    pub c: char,
    /// Foreground color
    pub fg: Color,
    /// Background color
    pub bg: Color,
    /// Underline color (SGR 58/59) - None means use foreground color
    pub underline_color: Option<Color>,
    /// Text attributes/flags
    pub flags: CellFlags,
    /// Cached display width of the character (1 or 2, typically)
    pub(crate) width: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color::Named(crate::color::NamedColor::White),
            bg: Color::Named(crate::color::NamedColor::Black),
            underline_color: None,
            flags: CellFlags::default(),
            width: 1, // Space has width 1
        }
    }
}

impl Cell {
    /// Create a new cell with a character
    pub fn new(c: char) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) as u8;
        Self {
            c,
            width,
            ..Default::default()
        }
    }

    /// Create a new cell with character and colors
    pub fn with_colors(c: char, fg: Color, bg: Color) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) as u8;
        Self {
            c,
            fg,
            bg,
            underline_color: None,
            flags: CellFlags::default(),
            width,
        }
    }

    /// Check if this cell is empty (contains a space with default attributes)
    pub fn is_empty(&self) -> bool {
        self.c == ' ' && self.flags == CellFlags::default()
    }

    /// Reset the cell to default state
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Get the display width of the character (cached value)
    pub fn width(&self) -> usize {
        self.width as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_cell() {
        let cell = Cell::default();
        assert_eq!(cell.c, ' ');
        assert!(cell.is_empty());
    }

    #[test]
    fn test_cell_with_char() {
        let cell = Cell::new('A');
        assert_eq!(cell.c, 'A');
        assert!(!cell.is_empty());
    }

    #[test]
    fn test_cell_width() {
        let cell = Cell::new('A');
        assert_eq!(cell.width(), 1);

        let wide_cell = Cell::new('中');
        assert_eq!(wide_cell.width(), 2);
    }

    #[test]
    fn test_cell_flags() {
        let mut flags = CellFlags::default();
        assert!(!flags.bold());

        flags.set_bold(true);
        assert!(flags.bold());
    }
}
