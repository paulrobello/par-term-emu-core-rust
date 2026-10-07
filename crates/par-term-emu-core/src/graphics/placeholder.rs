//! Unicode placeholder support for Kitty graphics protocol
//!
//! This module handles parsing of Unicode diacritics (combining characters)
//! used with the U+10EEEE placeholder character to specify row, column,
//! and most significant byte of image ID for virtual placement rendering.
//!
//! Reference: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders
//! Diacritic table: https://github.com/kovidgoyal/kitty/blob/master/kittens/unicode_input/rowcolumn-diacritics.txt
//!
//! Per the spec there are 297 diacritics indexed 0..=296.  The earlier
//! 64-entry table caused images wider/taller than 64 cells to lose
//! placeholders past the 64th column/row — `decode_placeholder_cell` would
//! reject the col-diacritic on cell 64+, the bounding-box scan would stop at
//! 63, and clients (e.g. par-textual-image) saw a Kitty image visibly smaller
//! than the same image rendered via Sixel/iTerm2.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The Unicode placeholder character for graphics
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';

/// Maximum index supported by the diacritic table (Kitty spec: 297 entries).
pub const MAX_DIACRITIC_INDEX: u16 = 296;

/// Full diacritic table (297 entries) from the Kitty graphics protocol spec.
/// Index = numeric value, value = combining character used in the placeholder.
pub const DIACRITICS: &[char] = &[
    '\u{0305}',
    '\u{030D}',
    '\u{030E}',
    '\u{0310}',
    '\u{0312}',
    '\u{033D}',
    '\u{033E}',
    '\u{033F}',
    '\u{0346}',
    '\u{034A}',
    '\u{034B}',
    '\u{034C}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035B}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036A}',
    '\u{036B}',
    '\u{036C}',
    '\u{036D}',
    '\u{036E}',
    '\u{036F}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059C}',
    '\u{059D}',
    '\u{059E}',
    '\u{059F}',
    '\u{05A0}',
    '\u{05A1}',
    '\u{05A8}',
    '\u{05A9}',
    '\u{05AB}',
    '\u{05AC}',
    '\u{05AF}',
    '\u{05C4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065A}',
    '\u{065B}',
    '\u{065D}',
    '\u{065E}',
    '\u{06D6}',
    '\u{06D7}',
    '\u{06D8}',
    '\u{06D9}',
    '\u{06DA}',
    '\u{06DB}',
    '\u{06DC}',
    '\u{06DF}',
    '\u{06E0}',
    '\u{06E1}',
    '\u{06E2}',
    '\u{06E4}',
    '\u{06E7}',
    '\u{06E8}',
    '\u{06EB}',
    '\u{06EC}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073A}',
    '\u{073D}',
    '\u{073F}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074A}',
    '\u{07EB}',
    '\u{07EC}',
    '\u{07ED}',
    '\u{07EE}',
    '\u{07EF}',
    '\u{07F0}',
    '\u{07F1}',
    '\u{07F3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081B}',
    '\u{081C}',
    '\u{081D}',
    '\u{081E}',
    '\u{081F}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082A}',
    '\u{082B}',
    '\u{082C}',
    '\u{082D}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0F82}',
    '\u{0F83}',
    '\u{0F86}',
    '\u{0F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// Build the inverse `char -> u16` lookup table on first use.  Cheaper than a
/// 297-arm match (which the compiler does not always lower to a jump table for
/// chars in widely-separated planes).
fn diacritic_lookup() -> &'static HashMap<char, u16> {
    static LOOKUP: OnceLock<HashMap<char, u16>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        DIACRITICS
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, i as u16))
            .collect()
    })
}

/// Number to diacritic mapping for row/column/MSB encoding.
///
/// Returns `None` for indices outside the 0..=296 spec range.
pub fn number_to_diacritic(n: u16) -> Option<char> {
    DIACRITICS.get(n as usize).copied()
}

/// Diacritic to number mapping for row/column/MSB decoding.
///
/// Returns the spec index in 0..=296 for any of the 297 valid diacritics, or
/// `None` for any other character.
pub fn diacritic_to_number(c: char) -> Option<u16> {
    diacritic_lookup().get(&c).copied()
}

/// Information extracted from a Unicode placeholder cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaceholderInfo {
    /// Image ID (from foreground color)
    pub image_id: u32,
    /// Placement ID (from underline color, 0 if not specified)
    pub placement_id: u32,
    /// Row position (from first diacritic; 0..=296 per spec)
    pub row: Option<u16>,
    /// Column position (from second diacritic; 0..=296 per spec)
    pub col: Option<u16>,
    /// Most significant byte of image ID (from third diacritic; spec uses
    /// indices 0..=255 here so this stays a `u8`)
    pub msb: Option<u8>,
}

impl PlaceholderInfo {
    /// Create placeholder info from foreground color (image ID)
    pub fn from_color(image_id: u32) -> Self {
        Self {
            image_id,
            placement_id: 0,
            row: None,
            col: None,
            msb: None,
        }
    }

    /// Set the placement ID from underline color
    pub fn with_placement_id(mut self, placement_id: u32) -> Self {
        self.placement_id = placement_id;
        self
    }

    /// Set row/column/MSB from diacritics
    pub fn with_diacritics(mut self, row: Option<u16>, col: Option<u16>, msb: Option<u8>) -> Self {
        self.row = row;
        self.col = col;
        self.msb = msb;
        self
    }

    /// Get the full image ID including MSB
    pub fn full_image_id(&self) -> u32 {
        if let Some(msb) = self.msb {
            // Combine MSB with lower bytes from color
            let lower_24 = self.image_id & 0x00FFFFFF;
            ((msb as u32) << 24) | lower_24
        } else {
            self.image_id
        }
    }

    /// Check if this placeholder can inherit from the previous cell
    pub fn can_inherit_from(&self, prev: &PlaceholderInfo, expected_col: u16) -> bool {
        // Same image ID and placement ID
        if self.image_id != prev.image_id || self.placement_id != prev.placement_id {
            return false;
        }

        match (self.row, self.col, self.msb) {
            // No diacritics: inherit row, col+1, msb
            (None, None, None) => true,
            // Only row: inherit col+1 and msb if same row
            (Some(row), None, None) => row == prev.row.unwrap_or(0),
            // Row and col: inherit msb if col is prev.col + 1
            (Some(row), Some(col), None) => row == prev.row.unwrap_or(0) && col == expected_col,
            _ => false,
        }
    }

    /// Inherit values from previous placeholder
    pub fn inherit_from(&mut self, prev: &PlaceholderInfo) {
        if self.row.is_none() {
            self.row = prev.row;
        }
        if self.col.is_none() {
            self.col = prev.col.map(|c| c + 1);
        }
        if self.msb.is_none() {
            self.msb = prev.msb;
        }
    }
}

/// Create a placeholder character with diacritics for row/column/MSB encoding
///
/// Returns a String containing U+10EEEE followed by up to 3 combining diacritics.
/// - First diacritic: row (0..=296)
/// - Second diacritic: column (0..=296)
/// - Third diacritic: MSB of image ID (0..=255, optional)
///
/// If MSB is 0 or None, it is omitted.
pub fn create_placeholder_with_diacritics(row: u16, col: u16, msb: Option<u8>) -> String {
    let mut result = String::from(PLACEHOLDER_CHAR);

    // Add row diacritic
    if let Some(row_diacritic) = number_to_diacritic(row) {
        result.push(row_diacritic);
    }

    // Add column diacritic
    if let Some(col_diacritic) = number_to_diacritic(col) {
        result.push(col_diacritic);
    }

    // Add MSB diacritic if present and non-zero
    if let Some(msb_val) = msb {
        if msb_val > 0 {
            if let Some(msb_diacritic) = number_to_diacritic(msb_val as u16) {
                result.push(msb_diacritic);
            }
        }
    }

    result
}

/// Parse diacritics from a string of combining characters
///
/// Returns (row, col, msb) as parsed from the diacritics
pub fn parse_diacritics(diacritics: &str) -> (Option<u16>, Option<u16>, Option<u8>) {
    let mut chars: Vec<char> = diacritics.chars().collect();

    // Remove any non-diacritic characters
    chars.retain(|&c| diacritic_to_number(c).is_some());

    let row = chars.first().and_then(|&c| diacritic_to_number(c));
    let col = chars.get(1).and_then(|&c| diacritic_to_number(c));
    let msb = chars
        .get(2)
        .and_then(|&c| diacritic_to_number(c))
        .map(|n| n as u8);

    (row, col, msb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder_char() {
        assert_eq!(PLACEHOLDER_CHAR, '\u{10EEEE}');
    }

    #[test]
    fn test_diacritic_table_size() {
        assert_eq!(DIACRITICS.len(), 297, "Kitty spec defines 297 diacritics");
        assert_eq!(MAX_DIACRITIC_INDEX as usize, DIACRITICS.len() - 1);
    }

    #[test]
    fn test_number_to_diacritic() {
        assert_eq!(number_to_diacritic(0), Some('\u{0305}'));
        assert_eq!(number_to_diacritic(1), Some('\u{030D}'));
        assert_eq!(number_to_diacritic(2), Some('\u{030E}'));
        assert_eq!(number_to_diacritic(63), Some('\u{0658}'));
        // Past the old 64-entry table — must now resolve.
        assert_eq!(number_to_diacritic(64), Some('\u{0659}'));
        assert_eq!(number_to_diacritic(296), Some('\u{1D244}'));
        assert_eq!(number_to_diacritic(297), None);
    }

    #[test]
    fn test_diacritic_mapping() {
        assert_eq!(diacritic_to_number('\u{0305}'), Some(0));
        assert_eq!(diacritic_to_number('\u{030D}'), Some(1));
        assert_eq!(diacritic_to_number('\u{030E}'), Some(2));
        assert_eq!(diacritic_to_number('\u{0658}'), Some(63));
        // Past the old table — must now resolve.
        assert_eq!(diacritic_to_number('\u{0659}'), Some(64));
        assert_eq!(diacritic_to_number('\u{1D244}'), Some(296));
        assert_eq!(diacritic_to_number('a'), None);
    }

    #[test]
    fn test_roundtrip_diacritic_conversion() {
        // Test that number -> diacritic -> number works for the full range.
        for n in 0..=MAX_DIACRITIC_INDEX {
            let diacritic = number_to_diacritic(n).expect("index in spec range");
            assert_eq!(diacritic_to_number(diacritic), Some(n));
        }
    }

    #[test]
    fn test_parse_diacritics() {
        // Row 0, col 0
        let (row, col, msb) = parse_diacritics("\u{0305}\u{0305}");
        assert_eq!(row, Some(0));
        assert_eq!(col, Some(0));
        assert_eq!(msb, None);

        // Row 1, col 0
        let (row, col, msb) = parse_diacritics("\u{030D}\u{0305}");
        assert_eq!(row, Some(1));
        assert_eq!(col, Some(0));
        assert_eq!(msb, None);

        // Row 0, col 1, msb 2
        let (row, col, msb) = parse_diacritics("\u{0305}\u{030D}\u{030E}");
        assert_eq!(row, Some(0));
        assert_eq!(col, Some(1));
        assert_eq!(msb, Some(2));

        // Row/col past the old 64-entry boundary — must now parse cleanly.
        let row_diac = number_to_diacritic(0).unwrap();
        let col_diac = number_to_diacritic(120).unwrap();
        let s: String = [row_diac, col_diac].iter().collect();
        let (row, col, _) = parse_diacritics(&s);
        assert_eq!(row, Some(0));
        assert_eq!(col, Some(120));
    }

    #[test]
    fn test_placeholder_info_full_image_id() {
        let info = PlaceholderInfo {
            image_id: 42,
            placement_id: 0,
            row: Some(0),
            col: Some(0),
            msb: None,
        };
        assert_eq!(info.full_image_id(), 42);

        let info_with_msb = PlaceholderInfo {
            image_id: 42,
            placement_id: 0,
            row: Some(0),
            col: Some(0),
            msb: Some(2),
        };
        // 2 << 24 | 42 = 33554474
        assert_eq!(info_with_msb.full_image_id(), 33554474);
    }

    #[test]
    fn test_placeholder_inheritance() {
        let prev = PlaceholderInfo {
            image_id: 42,
            placement_id: 0,
            row: Some(0),
            col: Some(0),
            msb: Some(2),
        };

        // Cell with no diacritics should inherit
        let mut current = PlaceholderInfo::from_color(42);
        assert!(current.can_inherit_from(&prev, 1));
        current.inherit_from(&prev);
        assert_eq!(current.row, Some(0));
        assert_eq!(current.col, Some(1));
        assert_eq!(current.msb, Some(2));

        // Cell with only row should inherit col and msb
        let mut current2 = PlaceholderInfo::from_color(42).with_diacritics(Some(0), None, None);
        assert!(current2.can_inherit_from(&prev, 1));
        current2.inherit_from(&prev);
        assert_eq!(current2.row, Some(0));
        assert_eq!(current2.col, Some(1));
        assert_eq!(current2.msb, Some(2));
    }

    #[test]
    fn test_create_placeholder_with_diacritics() {
        // Test with row=0, col=0, no MSB
        let placeholder = create_placeholder_with_diacritics(0, 0, None);
        assert!(placeholder.starts_with(PLACEHOLDER_CHAR));
        assert_eq!(placeholder.chars().count(), 3); // Char + 2 diacritics

        // Test with row=1, col=2, MSB=3
        let placeholder = create_placeholder_with_diacritics(1, 2, Some(3));
        assert!(placeholder.starts_with(PLACEHOLDER_CHAR));
        assert_eq!(placeholder.chars().count(), 4); // Char + 3 diacritics

        // Verify round-trip
        let diacritics: String = placeholder.chars().skip(1).collect();
        let (row, col, msb) = parse_diacritics(&diacritics);
        assert_eq!(row, Some(1));
        assert_eq!(col, Some(2));
        assert_eq!(msb, Some(3));
    }

    #[test]
    fn test_create_placeholder_past_64() {
        // Index 120 used to be unencodable — now it must round-trip.
        let placeholder = create_placeholder_with_diacritics(120, 200, None);
        assert!(placeholder.starts_with(PLACEHOLDER_CHAR));
        let diacritics: String = placeholder.chars().skip(1).collect();
        let (row, col, _) = parse_diacritics(&diacritics);
        assert_eq!(row, Some(120));
        assert_eq!(col, Some(200));
    }
}
