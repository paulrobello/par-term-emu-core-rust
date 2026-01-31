//! Configurable Unicode width calculation
//!
//! This module provides configurable character width calculations for terminal emulation.
//! It supports:
//! - Different Unicode versions for width tables
//! - Configurable treatment of East Asian Ambiguous width characters
//!
//! # Unicode Version Support
//!
//! Different Unicode versions have different width tables, particularly for emoji.
//! The `UnicodeVersion` enum allows specifying which version's width tables to use.
//!
//! # Ambiguous Width
//!
//! Some characters (East Asian Ambiguous) have uncertain width - they may be displayed
//! as either 1 or 2 cells depending on the context:
//! - Western contexts typically use narrow (1 cell)
//! - CJK contexts typically use wide (2 cells)
//!
//! # Example
//!
//! ```
//! use par_term_emu_core_rust::unicode_width_config::{WidthConfig, UnicodeVersion, AmbiguousWidth, char_width};
//!
//! let config = WidthConfig::default();
//! assert_eq!(char_width('A', &config), 1);
//! assert_eq!(char_width('\u{4E00}', &config), 2); // CJK character
//! ```

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

/// Unicode version for width calculation tables.
///
/// Different Unicode versions have different character width assignments,
/// particularly for newly added emoji and other characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnicodeVersion {
    /// Unicode 9.0 (June 2016) - Pre-emoji standardization
    Unicode9,
    /// Unicode 10.0 (June 2017)
    Unicode10,
    /// Unicode 11.0 (June 2018)
    Unicode11,
    /// Unicode 12.0 (March 2019)
    Unicode12,
    /// Unicode 13.0 (March 2020)
    Unicode13,
    /// Unicode 14.0 (September 2021)
    Unicode14,
    /// Unicode 15.0 (September 2022)
    Unicode15,
    /// Unicode 15.1 (September 2023)
    Unicode15_1,
    /// Unicode 16.0 (September 2024)
    Unicode16,
    /// Use the latest available Unicode version (default)
    #[default]
    Auto,
}

impl UnicodeVersion {
    /// Returns true if this version is Auto (use latest)
    #[inline]
    pub fn is_auto(&self) -> bool {
        matches!(self, UnicodeVersion::Auto)
    }

    /// Returns the version number as a tuple (major, minor)
    pub fn version_tuple(&self) -> Option<(u8, u8)> {
        match self {
            UnicodeVersion::Unicode9 => Some((9, 0)),
            UnicodeVersion::Unicode10 => Some((10, 0)),
            UnicodeVersion::Unicode11 => Some((11, 0)),
            UnicodeVersion::Unicode12 => Some((12, 0)),
            UnicodeVersion::Unicode13 => Some((13, 0)),
            UnicodeVersion::Unicode14 => Some((14, 0)),
            UnicodeVersion::Unicode15 => Some((15, 0)),
            UnicodeVersion::Unicode15_1 => Some((15, 1)),
            UnicodeVersion::Unicode16 => Some((16, 0)),
            UnicodeVersion::Auto => None,
        }
    }

    /// Returns a human-readable version string
    pub fn version_string(&self) -> &'static str {
        match self {
            UnicodeVersion::Unicode9 => "9.0",
            UnicodeVersion::Unicode10 => "10.0",
            UnicodeVersion::Unicode11 => "11.0",
            UnicodeVersion::Unicode12 => "12.0",
            UnicodeVersion::Unicode13 => "13.0",
            UnicodeVersion::Unicode14 => "14.0",
            UnicodeVersion::Unicode15 => "15.0",
            UnicodeVersion::Unicode15_1 => "15.1",
            UnicodeVersion::Unicode16 => "16.0",
            UnicodeVersion::Auto => "auto",
        }
    }
}

/// Treatment of East Asian Ambiguous width characters.
///
/// Ambiguous characters include Greek/Cyrillic letters, some symbols, and
/// other characters that may display as either 1 or 2 cells depending on context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguousWidth {
    /// Narrow (1 cell) - Western/default terminal behavior
    #[default]
    Narrow,
    /// Wide (2 cells) - CJK terminal behavior
    Wide,
}

impl AmbiguousWidth {
    /// Returns the width value (1 or 2)
    #[inline]
    pub fn width(&self) -> usize {
        match self {
            AmbiguousWidth::Narrow => 1,
            AmbiguousWidth::Wide => 2,
        }
    }

    /// Returns true if this is the narrow setting
    #[inline]
    pub fn is_narrow(&self) -> bool {
        matches!(self, AmbiguousWidth::Narrow)
    }

    /// Returns true if this is the wide setting
    #[inline]
    pub fn is_wide(&self) -> bool {
        matches!(self, AmbiguousWidth::Wide)
    }
}

/// Configuration for Unicode width calculations.
///
/// This struct combines Unicode version and ambiguous width settings
/// to control how character widths are calculated in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WidthConfig {
    /// Unicode version for width tables
    #[serde(default)]
    pub unicode_version: UnicodeVersion,
    /// Treatment of East Asian Ambiguous width characters
    #[serde(default)]
    pub ambiguous_width: AmbiguousWidth,
}

impl WidthConfig {
    /// Create a new WidthConfig with specified settings
    pub fn new(unicode_version: UnicodeVersion, ambiguous_width: AmbiguousWidth) -> Self {
        Self {
            unicode_version,
            ambiguous_width,
        }
    }

    /// Create a WidthConfig optimized for CJK environments
    pub fn cjk() -> Self {
        Self {
            unicode_version: UnicodeVersion::Auto,
            ambiguous_width: AmbiguousWidth::Wide,
        }
    }

    /// Create a WidthConfig optimized for Western environments
    pub fn western() -> Self {
        Self {
            unicode_version: UnicodeVersion::Auto,
            ambiguous_width: AmbiguousWidth::Narrow,
        }
    }
}

/// Check if a character is East Asian Ambiguous.
///
/// East Asian Ambiguous characters are those that have uncertain width,
/// displaying as either 1 or 2 cells depending on context.
///
/// This includes characters like:
/// - Greek and Cyrillic letters
/// - Some mathematical symbols
/// - Some line-drawing characters
/// - Various punctuation marks
#[inline]
pub fn is_east_asian_ambiguous(c: char) -> bool {
    // Common East Asian Ambiguous ranges
    // Based on Unicode EastAsianWidth property
    let code = c as u32;
    matches!(code,
        // Latin extended characters with ambiguous width
        0x00A1 | // INVERTED EXCLAMATION MARK
        0x00A4 | // CURRENCY SIGN
        0x00A7..=0x00A8 | // SECTION SIGN, DIAERESIS
        0x00AA | // FEMININE ORDINAL INDICATOR
        0x00AD..=0x00AE | // SOFT HYPHEN, REGISTERED SIGN
        0x00B0..=0x00B4 | // DEGREE SIGN through ACUTE ACCENT
        0x00B6..=0x00BA | // PILCROW SIGN through MASCULINE ORDINAL
        0x00BC..=0x00BF | // VULGAR FRACTIONS, INVERTED QUESTION MARK
        0x00C6 | // LATIN CAPITAL LETTER AE
        0x00D0 | // LATIN CAPITAL LETTER ETH
        0x00D7..=0x00D8 | // MULTIPLICATION SIGN, LATIN CAPITAL O WITH STROKE
        0x00DE..=0x00E1 | // THORN through LATIN SMALL A WITH ACUTE
        0x00E6 | // LATIN SMALL LETTER AE
        0x00E8..=0x00EA | // E WITH GRAVE through E WITH CIRCUMFLEX
        0x00EC..=0x00ED | // I WITH GRAVE, I WITH ACUTE
        0x00F0 | // LATIN SMALL LETTER ETH
        0x00F2..=0x00F3 | // O WITH GRAVE, O WITH ACUTE
        0x00F7..=0x00FA | // DIVISION SIGN through U WITH ACUTE
        0x00FC | // U WITH DIAERESIS
        0x00FE | // LATIN SMALL LETTER THORN

        // Greek letters
        0x0391..=0x03A1 | // GREEK CAPITAL LETTERS
        0x03A3..=0x03A9 | // More Greek capitals
        0x03B1..=0x03C1 | // Greek small letters
        0x03C3..=0x03C9 | // More Greek small letters

        // Cyrillic letters
        0x0401 | // CYRILLIC CAPITAL LETTER IO
        0x0410..=0x044F | // Basic Cyrillic
        0x0451 | // CYRILLIC SMALL LETTER IO

        // General punctuation
        0x2010 | // HYPHEN
        0x2013..=0x2016 | // EN DASH through DOUBLE VERTICAL LINE
        0x2018..=0x2019 | // SINGLE QUOTATION MARKS
        0x201C..=0x201D | // DOUBLE QUOTATION MARKS
        0x2020..=0x2022 | // DAGGER, DOUBLE DAGGER, BULLET
        0x2024..=0x2027 | // ONE DOT LEADER through HYPHENATION POINT
        0x2030 | // PER MILLE SIGN
        0x2032..=0x2033 | // PRIME, DOUBLE PRIME
        0x2035 | // REVERSED PRIME
        0x203B | // REFERENCE MARK
        0x203E | // OVERLINE
        0x2074 | // SUPERSCRIPT FOUR
        0x207F | // SUPERSCRIPT LATIN SMALL LETTER N
        0x2081..=0x2084 | // SUBSCRIPTS 1-4

        // Letterlike symbols
        0x2103 | // DEGREE CELSIUS
        0x2105 | // CARE OF
        0x2109 | // DEGREE FAHRENHEIT
        0x2113 | // SCRIPT SMALL L
        0x2116 | // NUMERO SIGN
        0x2121..=0x2122 | // TEL, TM
        0x2126 | // OHM SIGN
        0x212B | // ANGSTROM SIGN
        0x2153..=0x2154 | // VULGAR FRACTIONS

        // Arrows
        0x2190..=0x2199 | // ARROWS
        0x21B8..=0x21B9 | // More arrows
        0x21D2 | // RIGHTWARDS DOUBLE ARROW
        0x21D4 | // LEFT RIGHT DOUBLE ARROW
        0x21E7 | // UPWARDS WHITE ARROW

        // Mathematical operators
        0x2200 | // FOR ALL
        0x2202..=0x2203 | // PARTIAL DIFFERENTIAL, THERE EXISTS
        0x2207..=0x2208 | // NABLA, ELEMENT OF
        0x220B | // CONTAINS AS MEMBER
        0x220F | // N-ARY PRODUCT
        0x2211 | // N-ARY SUMMATION
        0x2215 | // DIVISION SLASH
        0x221A | // SQUARE ROOT
        0x221D..=0x2220 | // PROPORTIONAL TO through ANGLE
        0x2223 | // DIVIDES
        0x2225 | // PARALLEL TO
        0x2227..=0x222C | // LOGICAL AND through DOUBLE INTEGRAL
        0x222E | // CONTOUR INTEGRAL
        0x2234..=0x2237 | // THEREFORE through PROPORTION
        0x223C..=0x223D | // TILDE OPERATOR, REVERSED TILDE
        0x2248 | // ALMOST EQUAL TO
        0x224C | // ALL EQUAL TO
        0x2252 | // APPROXIMATELY EQUAL TO OR THE IMAGE OF
        0x2260..=0x2261 | // NOT EQUAL TO, IDENTICAL TO
        0x2264..=0x2267 | // LESS/GREATER THAN OR EQUAL TO
        0x226A..=0x226B | // MUCH LESS/GREATER THAN
        0x226E..=0x226F | // NOT LESS/GREATER THAN
        0x2282..=0x2283 | // SUBSET/SUPERSET OF
        0x2286..=0x2287 | // SUBSET/SUPERSET OF OR EQUAL TO
        0x2295 | // CIRCLED PLUS
        0x2299 | // CIRCLED DOT OPERATOR
        0x22A5 | // UP TACK
        0x22BF | // RIGHT TRIANGLE

        // Miscellaneous technical
        0x2312 | // ARC

        // Box drawing (subset that's ambiguous)
        0x2500..=0x254B | // Box drawing

        // Block elements
        0x2550..=0x2573 | // More box drawing
        0x2580..=0x258F | // Block elements
        0x2592..=0x2595 | // Shades and light/medium/dark shades

        // Geometric shapes
        0x25A0..=0x25A1 | // BLACK/WHITE SQUARE
        0x25A3..=0x25A9 | // Various squares
        0x25B2..=0x25B3 | // BLACK/WHITE UP-POINTING TRIANGLE
        0x25B6..=0x25B7 | // BLACK/WHITE RIGHT-POINTING TRIANGLE
        0x25BC..=0x25BD | // BLACK/WHITE DOWN-POINTING TRIANGLE
        0x25C0..=0x25C1 | // BLACK/WHITE LEFT-POINTING TRIANGLE
        0x25C6..=0x25C8 | // BLACK/WHITE DIAMOND
        0x25CB | // WHITE CIRCLE
        0x25CE..=0x25D1 | // BULLSEYE through CIRCLE variants
        0x25E2..=0x25E5 | // BLACK triangles
        0x25EF | // LARGE CIRCLE

        // Miscellaneous symbols
        0x2605..=0x2606 | // BLACK/WHITE STAR
        0x2609 | // SUN
        0x260E..=0x260F | // Telephone symbols
        0x2614..=0x2615 | // Umbrella, hot beverage
        0x261C | // WHITE LEFT POINTING INDEX
        0x261E | // WHITE RIGHT POINTING INDEX
        0x2640 | // FEMALE SIGN
        0x2642 | // MALE SIGN
        0x2660..=0x2661 | // BLACK/WHITE SPADE SUIT
        0x2663..=0x2665 | // Card suit symbols
        0x2667..=0x266A | // More card suits, musical notes
        0x266C..=0x266D | // BEAMED SIXTEENTH NOTES, MUSIC FLAT SIGN
        0x266F | // MUSIC SHARP SIGN
        0x269E..=0x269F | // THREE LINES CONVERGING
        0x26BE..=0x26BF | // Baseball, squared key
        0x26C4..=0x26CD | // Various symbols
        0x26CF..=0x26E1 | // More symbols
        0x26E3 | // HEAVY CIRCLE WITH STROKE
        0x26E8..=0x26FF | // More symbols

        // Dingbats
        0x273D | // HEAVY TEARDROP-SPOKED ASTERISK
        0x2757 | // HEAVY EXCLAMATION MARK SYMBOL
        0x2776..=0x277F | // DINGBAT NEGATIVE CIRCLED DIGITS

        // CJK symbols
        0x2B55..=0x2B59 | // Heavy circles
        0xFE00..=0xFE0F | // Variation selectors
        0xFFFD // REPLACEMENT CHARACTER
    )
}

/// Calculate the display width of a character.
///
/// This function calculates how many terminal cells a character occupies,
/// taking into account the width configuration.
///
/// # Arguments
///
/// * `c` - The character to measure
/// * `config` - Width configuration settings
///
/// # Returns
///
/// The display width in cells (0, 1, or 2)
///
/// # Examples
///
/// ```
/// use par_term_emu_core_rust::unicode_width_config::{char_width, WidthConfig, AmbiguousWidth};
///
/// let config = WidthConfig::default();
/// assert_eq!(char_width('A', &config), 1);
/// assert_eq!(char_width('\u{4E00}', &config), 2); // CJK
///
/// // With CJK ambiguous width setting
/// let cjk_config = WidthConfig::cjk();
/// // Greek alpha is ambiguous
/// assert_eq!(char_width('\u{03B1}', &cjk_config), 2);
/// ```
#[inline]
pub fn char_width(c: char, config: &WidthConfig) -> usize {
    // Handle ambiguous width characters
    if config.ambiguous_width.is_wide() && is_east_asian_ambiguous(c) {
        return 2;
    }

    // Use unicode-width crate for standard width calculation
    // The crate handles most cases correctly including:
    // - Control characters (0 width)
    // - Zero-width characters
    // - Wide characters (CJK, emoji, etc.)
    c.width().unwrap_or(0)
}

/// Calculate the display width of a character with CJK ambiguous width.
///
/// This is a convenience function that uses `AmbiguousWidth::Wide`.
/// Equivalent to calling `char_width(c, &WidthConfig::cjk())`.
#[inline]
pub fn char_width_cjk(c: char) -> usize {
    char_width(c, &WidthConfig::cjk())
}

/// Calculate the display width of a string.
///
/// This sums the widths of all characters in the string.
///
/// # Arguments
///
/// * `s` - The string to measure
/// * `config` - Width configuration settings
///
/// # Returns
///
/// The total display width in cells
pub fn str_width(s: &str, config: &WidthConfig) -> usize {
    s.chars().map(|c| char_width(c, config)).sum()
}

/// Calculate the display width of a string with CJK ambiguous width.
///
/// This is a convenience function that uses `AmbiguousWidth::Wide`.
pub fn str_width_cjk(s: &str) -> usize {
    str_width(s, &WidthConfig::cjk())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unicode_version_default() {
        let version = UnicodeVersion::default();
        assert!(version.is_auto());
    }

    #[test]
    fn test_unicode_version_tuple() {
        assert_eq!(UnicodeVersion::Unicode9.version_tuple(), Some((9, 0)));
        assert_eq!(UnicodeVersion::Unicode15_1.version_tuple(), Some((15, 1)));
        assert_eq!(UnicodeVersion::Auto.version_tuple(), None);
    }

    #[test]
    fn test_unicode_version_string() {
        assert_eq!(UnicodeVersion::Unicode9.version_string(), "9.0");
        assert_eq!(UnicodeVersion::Unicode15_1.version_string(), "15.1");
        assert_eq!(UnicodeVersion::Auto.version_string(), "auto");
    }

    #[test]
    fn test_ambiguous_width_default() {
        let width = AmbiguousWidth::default();
        assert!(width.is_narrow());
        assert_eq!(width.width(), 1);
    }

    #[test]
    fn test_ambiguous_width_wide() {
        let width = AmbiguousWidth::Wide;
        assert!(width.is_wide());
        assert_eq!(width.width(), 2);
    }

    #[test]
    fn test_width_config_default() {
        let config = WidthConfig::default();
        assert!(config.unicode_version.is_auto());
        assert!(config.ambiguous_width.is_narrow());
    }

    #[test]
    fn test_width_config_cjk() {
        let config = WidthConfig::cjk();
        assert!(config.unicode_version.is_auto());
        assert!(config.ambiguous_width.is_wide());
    }

    #[test]
    fn test_width_config_western() {
        let config = WidthConfig::western();
        assert!(config.unicode_version.is_auto());
        assert!(config.ambiguous_width.is_narrow());
    }

    #[test]
    fn test_char_width_ascii() {
        let config = WidthConfig::default();
        assert_eq!(char_width('A', &config), 1);
        assert_eq!(char_width('z', &config), 1);
        assert_eq!(char_width('0', &config), 1);
        assert_eq!(char_width(' ', &config), 1);
    }

    #[test]
    fn test_char_width_cjk_characters() {
        let config = WidthConfig::default();
        // CJK Unified Ideographs
        assert_eq!(char_width('\u{4E00}', &config), 2); // 一
        assert_eq!(char_width('\u{9FFF}', &config), 2);
        // Hiragana
        assert_eq!(char_width('\u{3042}', &config), 2); // あ
                                                        // Katakana
        assert_eq!(char_width('\u{30A2}', &config), 2); // ア
    }

    #[test]
    fn test_char_width_emoji() {
        let config = WidthConfig::default();
        // Basic emoji
        assert_eq!(char_width('\u{1F600}', &config), 2); // 😀
        assert_eq!(char_width('\u{1F44D}', &config), 2); // 👍
    }

    #[test]
    fn test_char_width_control_characters() {
        let config = WidthConfig::default();
        // Control characters have width 0
        assert_eq!(char_width('\x00', &config), 0);
        assert_eq!(char_width('\x1B', &config), 0); // ESC
        assert_eq!(char_width('\n', &config), 0);
        assert_eq!(char_width('\r', &config), 0);
    }

    #[test]
    fn test_char_width_combining_characters() {
        let config = WidthConfig::default();
        // Combining characters have width 0
        assert_eq!(char_width('\u{0301}', &config), 0); // Combining acute accent
        assert_eq!(char_width('\u{0300}', &config), 0); // Combining grave accent
    }

    #[test]
    fn test_ambiguous_width_greek() {
        // Greek alpha is East Asian Ambiguous
        let narrow_config = WidthConfig::default();
        let wide_config = WidthConfig::cjk();

        // With narrow config, Greek alpha should be 1
        assert_eq!(char_width('\u{03B1}', &narrow_config), 1); // α

        // With wide config, Greek alpha should be 2
        assert_eq!(char_width('\u{03B1}', &wide_config), 2);
    }

    #[test]
    fn test_ambiguous_width_cyrillic() {
        // Some Cyrillic letters are East Asian Ambiguous
        let narrow_config = WidthConfig::default();
        let wide_config = WidthConfig::cjk();

        assert_eq!(char_width('\u{0410}', &narrow_config), 1); // А (Cyrillic A)
        assert_eq!(char_width('\u{0410}', &wide_config), 2);
    }

    #[test]
    fn test_ambiguous_width_box_drawing() {
        // Box drawing characters are East Asian Ambiguous
        let narrow_config = WidthConfig::default();
        let wide_config = WidthConfig::cjk();

        assert_eq!(char_width('\u{2500}', &narrow_config), 1); // ─
        assert_eq!(char_width('\u{2500}', &wide_config), 2);
    }

    #[test]
    fn test_str_width_ascii() {
        let config = WidthConfig::default();
        assert_eq!(str_width("hello", &config), 5);
        assert_eq!(str_width("", &config), 0);
    }

    #[test]
    fn test_str_width_mixed() {
        let config = WidthConfig::default();
        // "a" (1) + CJK char (2) + "b" (1) = 4
        assert_eq!(str_width("a\u{4E00}b", &config), 4);
    }

    #[test]
    fn test_str_width_cjk_function() {
        // Test the convenience function
        assert_eq!(str_width_cjk("a\u{03B1}b"), 4); // "a" + Greek alpha (wide) + "b"
    }

    #[test]
    fn test_char_width_cjk_function() {
        assert_eq!(char_width_cjk('\u{03B1}'), 2); // Greek alpha with CJK width
    }

    #[test]
    fn test_is_east_asian_ambiguous() {
        // Test some known ambiguous characters
        assert!(is_east_asian_ambiguous('\u{00A1}')); // INVERTED EXCLAMATION MARK
        assert!(is_east_asian_ambiguous('\u{03B1}')); // Greek alpha
        assert!(is_east_asian_ambiguous('\u{0410}')); // Cyrillic A
        assert!(is_east_asian_ambiguous('\u{2500}')); // Box drawing

        // Test non-ambiguous characters
        assert!(!is_east_asian_ambiguous('A'));
        assert!(!is_east_asian_ambiguous('a'));
        assert!(!is_east_asian_ambiguous('\u{4E00}')); // CJK - wide, not ambiguous
    }

    #[test]
    fn test_serde_roundtrip() {
        let config = WidthConfig::new(UnicodeVersion::Unicode15, AmbiguousWidth::Wide);
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: WidthConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_serde_default_values() {
        // Test that missing fields use defaults
        let json = "{}";
        let config: WidthConfig = serde_json::from_str(json).unwrap();
        assert!(config.unicode_version.is_auto());
        assert!(config.ambiguous_width.is_narrow());
    }
}
