use crate::color::Color;
use crate::unicode_width_config::{char_width, str_width, WidthConfig};
use bitflags::bitflags;
use std::num::NonZeroU32;
use std::sync::Arc;

/// Underline style for text decoration (SGR 4:x)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct CellBitflags: u16 {
        /// Bold (SGR 1).
        const BOLD = 1 << 0;
        /// Dim/faint (SGR 2).
        const DIM = 1 << 1;
        /// Italic (SGR 3).
        const ITALIC = 1 << 2;
        /// Underline (SGR 4).
        const UNDERLINE = 1 << 3;
        /// Blink (SGR 5).
        const BLINK = 1 << 4;
        /// Reverse video (SGR 7).
        const REVERSE = 1 << 5;
        /// Hidden/concealed (SGR 8).
        const HIDDEN = 1 << 6;
        /// Strikethrough (SGR 9).
        const STRIKETHROUGH = 1 << 7;
        /// Overline (SGR 53).
        const OVERLINE = 1 << 8;
        /// Protected from selective erase (DECSCA).
        const GUARDED = 1 << 9;
        /// Leading cell of a double-width character.
        const WIDE_CHAR = 1 << 10;
        /// Trailing spacer cell of a double-width character.
        const WIDE_CHAR_SPACER = 1 << 11;
    }
}

/// Flags for cell attributes (optimized with bitflags)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CellFlags {
    /// Bitflags for boolean attributes
    bits: CellBitflags,
    /// Underline style (SGR 4:x)
    pub underline_style: UnderlineStyle,
    /// Hyperlink ID (reference to URL in Terminal's hyperlinks HashMap).
    /// Niche-optimized via `NonZeroU32` so `None` (the common case — no link)
    /// costs zero extra bytes in `CellFlags` (ARC-010). IDs are always >= 1.
    pub hyperlink_id: Option<NonZeroU32>,
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
    /// Whether the bold attribute is set.
    #[inline]
    pub fn bold(&self) -> bool {
        self.bits.contains(CellBitflags::BOLD)
    }

    /// Whether the dim attribute is set.
    #[inline]
    pub fn dim(&self) -> bool {
        self.bits.contains(CellBitflags::DIM)
    }

    /// Whether the italic attribute is set.
    #[inline]
    pub fn italic(&self) -> bool {
        self.bits.contains(CellBitflags::ITALIC)
    }

    /// Whether the underline attribute is set.
    #[inline]
    pub fn underline(&self) -> bool {
        self.bits.contains(CellBitflags::UNDERLINE)
    }

    /// Whether the blink attribute is set.
    #[inline]
    pub fn blink(&self) -> bool {
        self.bits.contains(CellBitflags::BLINK)
    }

    /// Whether the reverse attribute is set.
    #[inline]
    pub fn reverse(&self) -> bool {
        self.bits.contains(CellBitflags::REVERSE)
    }

    /// Whether the hidden attribute is set.
    #[inline]
    pub fn hidden(&self) -> bool {
        self.bits.contains(CellBitflags::HIDDEN)
    }

    /// Whether the strikethrough attribute is set.
    #[inline]
    pub fn strikethrough(&self) -> bool {
        self.bits.contains(CellBitflags::STRIKETHROUGH)
    }

    /// Whether the overline attribute is set.
    #[inline]
    pub fn overline(&self) -> bool {
        self.bits.contains(CellBitflags::OVERLINE)
    }

    /// Whether the protected (DECSCA guarded) attribute is set.
    #[inline]
    pub fn guarded(&self) -> bool {
        self.bits.contains(CellBitflags::GUARDED)
    }

    /// Whether the wide-character attribute is set.
    #[inline]
    pub fn wide_char(&self) -> bool {
        self.bits.contains(CellBitflags::WIDE_CHAR)
    }

    /// Whether the wide-character spacer attribute is set.
    #[inline]
    pub fn wide_char_spacer(&self) -> bool {
        self.bits.contains(CellBitflags::WIDE_CHAR_SPACER)
    }

    // Setter methods for each flag
    /// Set or clear the bold attribute.
    #[inline]
    pub fn set_bold(&mut self, value: bool) {
        self.bits.set(CellBitflags::BOLD, value);
    }

    /// Set or clear the dim attribute.
    #[inline]
    pub fn set_dim(&mut self, value: bool) {
        self.bits.set(CellBitflags::DIM, value);
    }

    /// Set or clear the italic attribute.
    #[inline]
    pub fn set_italic(&mut self, value: bool) {
        self.bits.set(CellBitflags::ITALIC, value);
    }

    /// Set or clear the underline attribute.
    #[inline]
    pub fn set_underline(&mut self, value: bool) {
        self.bits.set(CellBitflags::UNDERLINE, value);
    }

    /// Set or clear the blink attribute.
    #[inline]
    pub fn set_blink(&mut self, value: bool) {
        self.bits.set(CellBitflags::BLINK, value);
    }

    /// Set or clear the reverse attribute.
    #[inline]
    pub fn set_reverse(&mut self, value: bool) {
        self.bits.set(CellBitflags::REVERSE, value);
    }

    /// Set or clear the hidden attribute.
    #[inline]
    pub fn set_hidden(&mut self, value: bool) {
        self.bits.set(CellBitflags::HIDDEN, value);
    }

    /// Set or clear the strikethrough attribute.
    #[inline]
    pub fn set_strikethrough(&mut self, value: bool) {
        self.bits.set(CellBitflags::STRIKETHROUGH, value);
    }

    /// Set or clear the overline attribute.
    #[inline]
    pub fn set_overline(&mut self, value: bool) {
        self.bits.set(CellBitflags::OVERLINE, value);
    }

    /// Set or clear the protected (DECSCA guarded) attribute.
    #[inline]
    pub fn set_guarded(&mut self, value: bool) {
        self.bits.set(CellBitflags::GUARDED, value);
    }

    /// Set or clear the wide-character attribute.
    #[inline]
    pub fn set_wide_char(&mut self, value: bool) {
        self.bits.set(CellBitflags::WIDE_CHAR, value);
    }

    /// Set or clear the wide-character spacer attribute.
    #[inline]
    pub fn set_wide_char_spacer(&mut self, value: bool) {
        self.bits.set(CellBitflags::WIDE_CHAR_SPACER, value);
    }

    /// Get the underlying bitflags value as a u16
    ///
    /// Returns the raw bits representation of the cell's boolean attributes,
    /// suitable for FFI or serialization.
    #[inline]
    pub fn to_bitflags(&self) -> u16 {
        self.bits.bits()
    }
}

/// A single cell in the terminal grid
///
/// Cell is 40 bytes (down from 56): colors are packed into `u32` words
/// ([`PackedColor`]) and combining marks spill to the heap via a
/// null-optimized `Option<Arc<[char]>>` instead of an inline
/// `SmallVec<[char; 4]>`. Only cells that actually carry combining marks
/// allocate.
///
/// Note: Cell is Clone but not Copy because `combining` holds a heap
/// `Box`. Use `.clone()` explicitly when you need to copy a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(into = "CellSerde", from = "CellSerde")
)]
pub struct Cell {
    /// The character stored in this cell
    #[doc(hidden)]
    pub c: char,
    /// Combining characters (variation selectors, ZWJ, modifiers, etc.),
    /// or `None` for the overwhelmingly common no-marks case. SmallVec was
    /// replaced because its 24-byte inline buffer cost 4x the allocation
    /// path for capacity >99.9% of cells never use.
    pub(crate) combining: Option<Arc<Vec<char>>>,
    /// Foreground color
    pub(crate) fg: PackedColor,
    /// Background color
    pub(crate) bg: PackedColor,
    /// Underline color (SGR 58/59); `None` when the top bit is clear (use
    /// foreground color), else the low 31 bits are a [`PackedColor`].
    pub(crate) underline_color: PackedOptionColor,
    /// Text attributes/flags
    #[doc(hidden)]
    pub flags: CellFlags,
    /// Cached display width of the character (1 or 2, typically)
    #[doc(hidden)]
    pub width: u8,
}

/// Wire-format mirror of [`Cell`] for serde. Field names and value shapes
/// match the pre-packing derive output exactly (verified against v0.58.0),
/// so mux persistence files and replay snapshots stay byte-compatible in
/// both directions.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(not(feature = "serde"), allow(dead_code))]
struct CellSerde {
    c: char,
    combining: Vec<char>,
    fg: Color,
    bg: Color,
    underline_color: Option<Color>,
    flags: CellFlags,
    width: u8,
}

#[cfg(feature = "serde")]
impl From<Cell> for CellSerde {
    fn from(cell: Cell) -> Self {
        Self {
            c: cell.c,
            combining: cell.combining.map(|a| (*a).clone()).unwrap_or_default(),
            fg: cell.fg.unpack(),
            bg: cell.bg.unpack(),
            underline_color: cell.underline_color.unpack(),
            flags: cell.flags,
            width: cell.width,
        }
    }
}

#[cfg(feature = "serde")]
impl From<CellSerde> for Cell {
    fn from(s: CellSerde) -> Self {
        Self {
            c: s.c,
            combining: if s.combining.is_empty() {
                None
            } else {
                Some(Arc::new(s.combining))
            },
            fg: PackedColor::pack(s.fg),
            bg: PackedColor::pack(s.bg),
            underline_color: PackedOptionColor::pack(s.underline_color),
            flags: s.flags,
            width: s.width,
        }
    }
}

/// A color packed into 4 bytes: 2-bit tag + payload.
///
/// Layout: bits 24-25 tag (`00` Indexed, `01` Named, `10` Rgb), payload in
/// bits 0-23 (Rgb uses all 24; Indexed the low 8; Named the low 4). Bits
/// 26-31 stay clear so [`PackedOptionColor`] can wrap a `PackedColor`
/// verbatim in 31 bits with one bit to spare for its presence flag.
/// Unpack is two shifts and a match — branch-light for the hot grid paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedColor(u32);

impl PackedColor {
    const TAG_SHIFT: u32 = 24;
    const TAG_INDEXED: u32 = 0b00;
    const TAG_NAMED: u32 = 0b01;
    const TAG_RGB: u32 = 0b10;

    #[inline]
    pub(crate) fn pack(color: Color) -> Self {
        let v = match color {
            Color::Indexed(i) => (Self::TAG_INDEXED << Self::TAG_SHIFT) | i as u32,
            Color::Named(n) => (Self::TAG_NAMED << Self::TAG_SHIFT) | (n as u32),
            Color::Rgb(r, g, b) => {
                (Self::TAG_RGB << Self::TAG_SHIFT)
                    | ((r as u32) << 16)
                    | ((g as u32) << 8)
                    | b as u32
            }
        };
        Self(v)
    }

    #[inline]
    pub(crate) fn unpack(self) -> Color {
        match self.0 >> Self::TAG_SHIFT {
            Self::TAG_INDEXED => Color::Indexed((self.0 & 0xFF) as u8),
            Self::TAG_NAMED => {
                Color::Named(crate::color::NamedColor::from_u8((self.0 & 0x0F) as u8))
            }
            _ => Color::Rgb(
                ((self.0 >> 16) & 0xFF) as u8,
                ((self.0 >> 8) & 0xFF) as u8,
                (self.0 & 0xFF) as u8,
            ),
        }
    }
}

/// The `Option<Color>` underline color packed into 4 bytes: top bit set
/// means `Some`, the low 31 bits carry the [`PackedColor`] verbatim (which
/// only uses 26 bits, so nothing overlaps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedOptionColor(u32);

impl PackedOptionColor {
    const PRESENT: u32 = 1 << 31;
    const PAYLOAD_MASK: u32 = 0x7FFF_FFFF;

    #[inline]
    pub(crate) fn pack(color: Option<Color>) -> Self {
        match color {
            None => Self(0),
            Some(c) => {
                let PackedColor(v) = PackedColor::pack(c);
                debug_assert_eq!(v & Self::PRESENT, 0, "color must fit 31 bits");
                Self(Self::PRESENT | v)
            }
        }
    }

    #[inline]
    pub(crate) fn unpack(self) -> Option<Color> {
        if self.0 & Self::PRESENT == 0 {
            return None;
        }
        Some(PackedColor(self.0 & Self::PAYLOAD_MASK).unpack())
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            combining: None,
            fg: PackedColor::pack(Color::Named(crate::color::NamedColor::White)),
            bg: PackedColor::pack(Color::Named(crate::color::NamedColor::Black)),
            underline_color: PackedOptionColor::pack(None),
            flags: CellFlags::default(),
            width: 1, // Space has width 1
        }
    }
}

impl Cell {
    /// Create a new cell with a character
    ///
    /// Uses the default width configuration.
    pub fn new(c: char) -> Self {
        let width = char_width(c, &WidthConfig::default()) as u8;
        Self {
            c,
            combining: None,
            width,
            ..Default::default()
        }
    }

    /// Create a new cell with character and colors
    ///
    /// Uses the default width configuration.
    pub fn with_colors(c: char, fg: Color, bg: Color) -> Self {
        let width = char_width(c, &WidthConfig::default()) as u8;
        Self {
            c,
            combining: None,
            fg: PackedColor::pack(fg),
            bg: PackedColor::pack(bg),
            underline_color: PackedOptionColor::pack(None),
            flags: CellFlags::default(),
            width,
        }
    }

    // --- Field accessors (ARC-012) ---
    // Cell fields are `pub(crate)` (encapsulated from the public API); external
    // consumers read them through these accessors so the representation can
    // change without breaking the rlib API.

    /// The base character stored in this cell.
    #[inline]
    pub fn c(&self) -> char {
        self.c
    }

    /// Foreground color.
    #[inline]
    pub fn fg(&self) -> Color {
        self.fg.unpack()
    }

    /// Background color.
    #[inline]
    pub fn bg(&self) -> Color {
        self.bg.unpack()
    }

    /// Underline color (SGR 58/59); `None` means use the foreground color.
    #[inline]
    pub fn underline_color(&self) -> Option<Color> {
        self.underline_color.unpack()
    }

    /// Combining marks following the base character (empty for most cells).
    #[inline]
    pub fn combining(&self) -> &[char] {
        static EMPTY: [char; 0] = [];
        match &self.combining {
            Some(v) => v.as_slice(),
            None => &EMPTY,
        }
    }

    /// The text attributes/flags.
    #[inline]
    pub fn flags(&self) -> &CellFlags {
        &self.flags
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

    /// Get the full grapheme cluster as a String
    ///
    /// This reconstructs the complete grapheme cluster by combining the base character
    /// with all combining characters (variation selectors, ZWJ, modifiers, etc.)
    ///
    /// **Performance Note**: This method allocates a new String on every call.
    /// For performance-critical code, consider using `base_char()` and `has_combining_chars()`
    /// to avoid allocations when possible.
    /// Append this cell's grapheme (base char + combining marks) to `buf`
    /// without allocating a per-cell `String` (QA-006).
    #[inline]
    pub fn push_grapheme(&self, buf: &mut String) {
        buf.push(self.c);
        if let Some(marks) = &self.combining {
            for &ch in marks.iter() {
                buf.push(ch);
            }
        }
    }

    /// Return the base character plus any combining characters as a String.
    pub fn get_grapheme(&self) -> String {
        let n = self.combining.as_ref().map_or(0, |v| v.len());
        let mut result = String::with_capacity(1 + n);
        self.push_grapheme(&mut result);
        result
    }

    /// Check if this cell has combining characters
    ///
    /// Returns true if the cell has variation selectors, ZWJ, skin tone modifiers,
    /// or other combining characters.
    ///
    /// This is useful for optimization - if false, you can use just `base_char()`
    /// without allocating a String.
    #[inline]
    pub fn has_combining_chars(&self) -> bool {
        self.combining.is_some()
    }

    /// Get the base character without combining characters
    ///
    /// This returns just the base character and avoids String allocation.
    /// For cells with combining characters, use `get_grapheme()` instead
    /// to get the complete grapheme cluster.
    #[inline]
    pub fn base_char(&self) -> char {
        self.c
    }

    // --- pub(crate) combining mutation helpers ---
    // Callers inside the crate mutate combining marks during grapheme
    // assembly (write.rs); these keep the Arc-based representation private.

    /// Append a combining mark, allocating the spill store on first use.
    #[inline]
    pub(crate) fn combining_push(&mut self, ch: char) {
        let marks = self.combining.get_or_insert_with(|| Arc::new(Vec::new()));
        Arc::make_mut(marks).push(ch);
    }

    /// Replace the whole combining sequence (normalization rewrite).
    #[inline]
    pub(crate) fn combining_replace(&mut self, marks: Vec<char>) {
        self.combining = if marks.is_empty() {
            None
        } else {
            Some(Arc::new(marks))
        };
    }

    /// Whether the combining sequence contains `ch`.
    #[inline]
    pub(crate) fn combining_has(&self, ch: char) -> bool {
        self.combining.as_ref().is_some_and(|m| m.contains(&ch))
    }

    // Test-only writes: production code paths build cells through the
    // constructors, so the setters exist purely for test seeding.
    /// Set the foreground color.
    #[inline]
    #[cfg(test)]
    pub(crate) fn set_fg(&mut self, color: Color) {
        self.fg = PackedColor::pack(color);
    }

    /// Set the background color.
    #[inline]
    #[cfg(test)]
    pub(crate) fn set_bg(&mut self, color: Color) {
        self.bg = PackedColor::pack(color);
    }

    /// Set the underline color.
    #[inline]
    #[cfg(test)]
    pub(crate) fn set_underline_color(&mut self, color: Option<Color>) {
        self.underline_color = PackedOptionColor::pack(color);
    }

    /// Whether the cell carries no combining marks.
    #[inline]
    pub(crate) fn combining_is_empty(&self) -> bool {
        self.combining.is_none()
    }

    /// Create a cell from a grapheme cluster (base char + combining chars)
    ///
    /// Uses the default width configuration.
    pub fn from_grapheme(grapheme: &str) -> Self {
        let mut chars = grapheme.chars();
        let base_char = chars.next().unwrap_or(' ');
        let rest: Vec<char> = chars.collect();
        let combining = if rest.is_empty() {
            None
        } else {
            Some(Arc::new(rest))
        };
        let width = str_width(grapheme, &WidthConfig::default()).max(1) as u8;

        Self {
            c: base_char,
            combining,
            width,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::NamedColor;

    /// Card: slim the Cell struct. 56B -> 40B via packed colors and a
    /// heap-spilled `Option<Arc<Vec<char>>> combining store.
    #[test]
    fn cell_fits_in_40_bytes() {
        assert!(
            std::mem::size_of::<Cell>() <= 40,
            "Cell grew to {} bytes; the packed layout contract is 40",
            std::mem::size_of::<Cell>()
        );
    }

    /// Named, indexed and RGB colors must round-trip bit-exact through the
    /// packed u32 representation, for every tag.
    #[test]
    fn packed_colors_round_trip_bit_exact() {
        let colors = [
            Color::Named(NamedColor::Black),
            Color::Named(NamedColor::BrightWhite),
            Color::Indexed(0),
            Color::Indexed(255),
            Color::Rgb(0, 0, 0),
            Color::Rgb(255, 255, 255),
            Color::Rgb(0x12, 0x34, 0x56),
        ];
        for color in colors {
            assert_eq!(PackedColor::pack(color).unpack(), color, "{:?}", color);
            assert_eq!(
                PackedOptionColor::pack(Some(color)).unpack(),
                Some(color),
                "{:?} through the option wrapper",
                color
            );
        }
        assert_eq!(PackedOptionColor::pack(None).unpack(), None);
    }

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

        assert_eq!(cell.c(), 'X');
        assert_eq!(cell.fg(), fg);
        assert_eq!(cell.bg(), bg);
        assert_eq!(cell.width(), 1);
    }

    #[test]
    fn test_cell_reset() {
        let mut cell = Cell::new('A');
        cell.set_fg(Color::Rgb(255, 0, 0));
        cell.set_bg(Color::Rgb(0, 255, 0));
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

        let id = NonZeroU32::new(42).unwrap();
        flags.hyperlink_id = Some(id);
        assert_eq!(flags.hyperlink_id, Some(id));

        flags.hyperlink_id = None;
        assert_eq!(flags.hyperlink_id, None);
    }

    #[test]
    fn test_cell_underline_color() {
        let mut cell = Cell::default();
        assert_eq!(cell.underline_color(), None);

        cell.set_underline_color(Some(Color::Rgb(255, 0, 0)));
        assert_eq!(cell.underline_color(), Some(Color::Rgb(255, 0, 0)));

        cell.set_underline_color(None);
        assert_eq!(cell.underline_color(), None);
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
        cell1.set_fg(Color::Rgb(255, 0, 0));
        cell1.flags.set_bold(true);

        let cell2 = cell1.clone();

        assert_eq!(cell1.c, cell2.c);
        assert_eq!(cell1.fg, cell2.fg);
        assert_eq!(cell1.flags, cell2.flags);
    }

    /// ARC-005/QA-004: bulk-cloning cells that carry combining marks must stay
    /// cheap. With the packed layout, marks live in a refcount-shared
    /// `Arc<[char]>` inside the cell, so each clone is an atomic increment
    /// rather than a per-clone deep copy — the same allocation-free
    /// characteristic SmallVec's inline buffer gave the 4-mark case.
    #[test]
    fn cloning_combining_cells_is_fast_at_scale() {
        // base char + 3 combining marks
        let cell = Cell::from_grapheme("e\u{0301}\u{0302}\u{0303}");
        assert_eq!(cell.combining().len(), 3);

        // ~800k clones ≈ a 10k-line × 80-col reflow/scroll (the audit's worst case).
        let start = std::time::Instant::now();
        let clones: Vec<Cell> = std::iter::repeat_with(|| cell.clone())
            .take(80 * 10_000)
            .collect();
        let elapsed = start.elapsed();
        assert!(clones.iter().all(|c| c == &cell));
        drop(clones);

        // Coarse sanity net only. The budget must absorb scheduler load while
        // staying far below the seconds-scale allocation storm of a per-clone
        // deep copy.
        assert!(
            elapsed < std::time::Duration::from_millis(5000),
            "cloning 800k combining-bearing cells took {:?}, expected < 5000ms",
            elapsed
        );
    }
}
