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

    #[test]
    fn test_all_cell_flags() {
        let mut flags = CellFlags::default();

        // Test bold
        assert!(!flags.bold());
        flags.set_bold(true);
        assert!(flags.bold());
        flags.set_bold(false);
        assert!(!flags.bold());

        // Test dim
        assert!(!flags.dim());
        flags.set_dim(true);
        assert!(flags.dim());
        flags.set_dim(false);
        assert!(!flags.dim());

        // Test italic
        assert!(!flags.italic());
        flags.set_italic(true);
        assert!(flags.italic());
        flags.set_italic(false);
        assert!(!flags.italic());

        // Test underline
        assert!(!flags.underline());
        flags.set_underline(true);
        assert!(flags.underline());
        flags.set_underline(false);
        assert!(!flags.underline());

        // Test blink
        assert!(!flags.blink());
        flags.set_blink(true);
        assert!(flags.blink());
        flags.set_blink(false);
        assert!(!flags.blink());

        // Test reverse
        assert!(!flags.reverse());
        flags.set_reverse(true);
        assert!(flags.reverse());
        flags.set_reverse(false);
        assert!(!flags.reverse());

        // Test hidden
        assert!(!flags.hidden());
        flags.set_hidden(true);
        assert!(flags.hidden());
        flags.set_hidden(false);
        assert!(!flags.hidden());

        // Test strikethrough
        assert!(!flags.strikethrough());
        flags.set_strikethrough(true);
        assert!(flags.strikethrough());
        flags.set_strikethrough(false);
        assert!(!flags.strikethrough());

        // Test overline
        assert!(!flags.overline());
        flags.set_overline(true);
        assert!(flags.overline());
        flags.set_overline(false);
        assert!(!flags.overline());

        // Test guarded
        assert!(!flags.guarded());
        flags.set_guarded(true);
        assert!(flags.guarded());
        flags.set_guarded(false);
        assert!(!flags.guarded());

        // Test wide_char
        assert!(!flags.wide_char());
        flags.set_wide_char(true);
        assert!(flags.wide_char());
        flags.set_wide_char(false);
        assert!(!flags.wide_char());

        // Test wide_char_spacer
        assert!(!flags.wide_char_spacer());
        flags.set_wide_char_spacer(true);
        assert!(flags.wide_char_spacer());
        flags.set_wide_char_spacer(false);
        assert!(!flags.wide_char_spacer());
    }

    #[test]
    fn test_cell_flags_combinations() {
        let mut flags = CellFlags::default();

        // Set multiple flags
        flags.set_bold(true);
        flags.set_italic(true);
        flags.set_underline(true);

        assert!(flags.bold());
        assert!(flags.italic());
        assert!(flags.underline());
        assert!(!flags.blink());

        // Disable one flag
        flags.set_bold(false);
        assert!(!flags.bold());
        assert!(flags.italic());
        assert!(flags.underline());
    }

    #[test]
    fn test_underline_styles() {
        let mut flags = CellFlags::default();
        assert_eq!(flags.underline_style, UnderlineStyle::None);

        flags.underline_style = UnderlineStyle::Straight;
        assert_eq!(flags.underline_style, UnderlineStyle::Straight);

        flags.underline_style = UnderlineStyle::Double;
        assert_eq!(flags.underline_style, UnderlineStyle::Double);

        flags.underline_style = UnderlineStyle::Curly;
        assert_eq!(flags.underline_style, UnderlineStyle::Curly);

        flags.underline_style = UnderlineStyle::Dotted;
        assert_eq!(flags.underline_style, UnderlineStyle::Dotted);

        flags.underline_style = UnderlineStyle::Dashed;
        assert_eq!(flags.underline_style, UnderlineStyle::Dashed);
    }

    #[test]
    fn test_cell_with_colors() {
        let fg = Color::Rgb(255, 128, 64);
        let bg = Color::Rgb(32, 64, 128);
        let cell = Cell::with_colors('X', fg, bg);

        assert_eq!(cell.c, 'X');
        assert_eq!(cell.fg, fg);
        assert_eq!(cell.bg, bg);
        assert_eq!(cell.width(), 1);
    }

    #[test]
    fn test_cell_reset() {
        let mut cell = Cell::new('A');
        cell.fg = Color::Rgb(255, 0, 0);
        cell.bg = Color::Rgb(0, 255, 0);
        cell.flags.set_bold(true);
        cell.flags.set_italic(true);

        assert!(!cell.is_empty());
        assert!(cell.flags.bold());

        cell.reset();

        assert_eq!(cell.c, ' ');
        assert!(cell.is_empty());
        assert!(!cell.flags.bold());
        assert!(!cell.flags.italic());
    }

    #[test]
    fn test_cell_is_empty() {
        let cell = Cell::default();
        assert!(cell.is_empty());

        let mut cell = Cell::new('A');
        assert!(!cell.is_empty());

        cell.c = ' ';
        assert!(cell.is_empty());

        cell.flags.set_bold(true);
        assert!(!cell.is_empty());
    }

    #[test]
    fn test_cell_with_emoji() {
        let cell = Cell::new('😀');
        assert_eq!(cell.c, '😀');
        // Emoji should have width 2
        assert_eq!(cell.width(), 2);
    }

    #[test]
    fn test_cell_with_zero_width_char() {
        // Combining characters have width 0
        let cell = Cell::new('\u{0301}'); // Combining acute accent
        assert_eq!(cell.c, '\u{0301}');
        // Zero-width chars actually have width 0, not defaulting to 1
        assert_eq!(cell.width(), 0);
    }

    #[test]
    fn test_cell_hyperlink_id() {
        let mut flags = CellFlags::default();
        assert_eq!(flags.hyperlink_id, None);

        flags.hyperlink_id = Some(42);
        assert_eq!(flags.hyperlink_id, Some(42));

        flags.hyperlink_id = None;
        assert_eq!(flags.hyperlink_id, None);
    }

    #[test]
    fn test_cell_underline_color() {
        let mut cell = Cell::default();
        assert_eq!(cell.underline_color, None);

        cell.underline_color = Some(Color::Rgb(255, 0, 0));
        assert_eq!(cell.underline_color, Some(Color::Rgb(255, 0, 0)));

        cell.underline_color = None;
        assert_eq!(cell.underline_color, None);
    }

    #[test]
    fn test_cell_flags_equality() {
        let mut flags1 = CellFlags::default();
        let mut flags2 = CellFlags::default();

        assert_eq!(flags1, flags2);

        flags1.set_bold(true);
        assert_ne!(flags1, flags2);

        flags2.set_bold(true);
        assert_eq!(flags1, flags2);
    }

    #[test]
    fn test_underline_style_equality() {
        assert_eq!(UnderlineStyle::None, UnderlineStyle::None);
        assert_eq!(UnderlineStyle::Straight, UnderlineStyle::Straight);
        assert_ne!(UnderlineStyle::None, UnderlineStyle::Straight);
        assert_ne!(UnderlineStyle::Curly, UnderlineStyle::Dotted);
    }

    #[test]
    fn test_cell_clone() {
        let mut cell1 = Cell::new('A');
        cell1.fg = Color::Rgb(255, 0, 0);
        cell1.flags.set_bold(true);

        let cell2 = cell1;

        assert_eq!(cell1.c, cell2.c);
        assert_eq!(cell1.fg, cell2.fg);
        assert_eq!(cell1.flags, cell2.flags);
    }
}
