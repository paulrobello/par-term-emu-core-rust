//! Unicode placeholder support for Kitty graphics protocol
//!
//! This module handles parsing of Unicode diacritics (combining characters)
//! used with the U+10EEEE placeholder character to specify row, column,
//! and most significant byte of image ID for virtual placement rendering.
//!
//! Reference: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders

/// The Unicode placeholder character for graphics
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';

/// Number to diacritic mapping for row/column encoding
///
/// Maps numeric values (0-63) to Unicode combining characters
/// as specified in the Kitty graphics protocol.
pub fn number_to_diacritic(n: u8) -> Option<char> {
    if n > 63 {
        return None;
    }

    // Mapping from rowcolumn-diacritics.txt in Kitty spec
    Some(match n {
        0 => '\u{0305}',  // Combining Overline
        1 => '\u{030D}',  // Combining Vertical Line Above
        2 => '\u{030E}',  // Combining Double Vertical Line Above
        3 => '\u{0310}',  // Combining Candrabindu
        4 => '\u{0312}',  // Combining Turned Comma Above
        5 => '\u{033D}',  // Combining X Above
        6 => '\u{033E}',  // Combining Vertical Tilde
        7 => '\u{033F}',  // Combining Double Overline
        8 => '\u{0346}',  // Combining Bridge Above
        9 => '\u{034A}',  // Combining Not Tilde Above
        10 => '\u{034B}', // Combining Homothetic Above
        11 => '\u{034C}', // Combining Almost Equal To Above
        12 => '\u{0350}', // Combining Right Arrowhead Above
        13 => '\u{0351}', // Combining Left Half Ring Above
        14 => '\u{0352}', // Combining Fermata
        15 => '\u{0357}', // Combining Right Half Ring Above
        16 => '\u{035B}', // Combining Zigzag Above
        17 => '\u{0363}', // Combining Latin Small Letter A
        18 => '\u{0364}', // Combining Latin Small Letter E
        19 => '\u{0365}', // Combining Latin Small Letter I
        20 => '\u{0366}', // Combining Latin Small Letter O
        21 => '\u{0367}', // Combining Latin Small Letter U
        22 => '\u{0368}', // Combining Latin Small Letter C
        23 => '\u{0369}', // Combining Latin Small Letter D
        24 => '\u{036A}', // Combining Latin Small Letter H
        25 => '\u{036B}', // Combining Latin Small Letter M
        26 => '\u{036C}', // Combining Latin Small Letter R
        27 => '\u{036D}', // Combining Latin Small Letter T
        28 => '\u{036E}', // Combining Latin Small Letter V
        29 => '\u{036F}', // Combining Latin Small Letter X
        30 => '\u{0483}', // Combining Cyrillic Titlo
        31 => '\u{0484}', // Combining Cyrillic Palatalization
        32 => '\u{0485}', // Combining Cyrillic Dasia Pneumata
        33 => '\u{0486}', // Combining Cyrillic Psili Pneumata
        34 => '\u{0487}', // Combining Cyrillic Pokrytie
        35 => '\u{0592}', // Hebrew Accent Segol
        36 => '\u{0593}', // Hebrew Accent Shalshelet
        37 => '\u{0594}', // Hebrew Accent Zaqef Qatan
        38 => '\u{0595}', // Hebrew Accent Zaqef Gadol
        39 => '\u{0597}', // Hebrew Accent Revia
        40 => '\u{0598}', // Hebrew Accent Zarqa
        41 => '\u{0599}', // Hebrew Accent Pashta
        42 => '\u{059C}', // Hebrew Accent Geresh
        43 => '\u{059D}', // Hebrew Accent Geresh Muqdam
        44 => '\u{059E}', // Hebrew Accent Gershayim
        45 => '\u{059F}', // Hebrew Accent Qarney Para
        46 => '\u{05A0}', // Hebrew Accent Telisha Gedola
        47 => '\u{05A1}', // Hebrew Accent Pazer
        48 => '\u{05A8}', // Hebrew Accent Qadma
        49 => '\u{05A9}', // Hebrew Accent Telisha Qetana
        50 => '\u{05AB}', // Hebrew Accent Ole
        51 => '\u{05AC}', // Hebrew Accent Iluy
        52 => '\u{05AF}', // Hebrew Mark Masora Circle
        53 => '\u{05C4}', // Hebrew Mark Upper Dot
        54 => '\u{0610}', // Arabic Sign Sallallahou Alayhe Wassallam
        55 => '\u{0611}', // Arabic Sign Alayhe Assallam
        56 => '\u{0612}', // Arabic Sign Rahmatullah Alayhe
        57 => '\u{0613}', // Arabic Sign Radi Allahou Anhu
        58 => '\u{0614}', // Arabic Sign Takhallus
        59 => '\u{0615}', // Arabic Small High Tah
        60 => '\u{0616}', // Arabic Small High Ligature Alef with Lam with Yeh
        61 => '\u{0617}', // Arabic Small High Zain
        62 => '\u{0657}', // Arabic Inverted Damma
        63 => '\u{0658}', // Arabic Mark Noon Ghunna
        _ => unreachable!(),
    })
}

/// Diacritic to number mapping for row/column encoding
///
/// Maps Unicode combining characters to their numeric values (0-63)
/// as specified in the Kitty graphics protocol.
pub fn diacritic_to_number(c: char) -> Option<u8> {
    // Mapping from rowcolumn-diacritics.txt in Kitty spec
    match c {
        '\u{0305}' => Some(0),  // Combining Overline
        '\u{030D}' => Some(1),  // Combining Vertical Line Above
        '\u{030E}' => Some(2),  // Combining Double Vertical Line Above
        '\u{0310}' => Some(3),  // Combining Candrabindu
        '\u{0312}' => Some(4),  // Combining Turned Comma Above
        '\u{033D}' => Some(5),  // Combining X Above
        '\u{033E}' => Some(6),  // Combining Vertical Tilde
        '\u{033F}' => Some(7),  // Combining Double Overline
        '\u{0346}' => Some(8),  // Combining Bridge Above
        '\u{034A}' => Some(9),  // Combining Not Tilde Above
        '\u{034B}' => Some(10), // Combining Homothetic Above
        '\u{034C}' => Some(11), // Combining Almost Equal To Above
        '\u{0350}' => Some(12), // Combining Right Arrowhead Above
        '\u{0351}' => Some(13), // Combining Left Half Ring Above
        '\u{0352}' => Some(14), // Combining Fermata
        '\u{0357}' => Some(15), // Combining Right Half Ring Above
        '\u{035B}' => Some(16), // Combining Zigzag Above
        '\u{0363}' => Some(17), // Combining Latin Small Letter A
        '\u{0364}' => Some(18), // Combining Latin Small Letter E
        '\u{0365}' => Some(19), // Combining Latin Small Letter I
        '\u{0366}' => Some(20), // Combining Latin Small Letter O
        '\u{0367}' => Some(21), // Combining Latin Small Letter U
        '\u{0368}' => Some(22), // Combining Latin Small Letter C
        '\u{0369}' => Some(23), // Combining Latin Small Letter D
        '\u{036A}' => Some(24), // Combining Latin Small Letter H
        '\u{036B}' => Some(25), // Combining Latin Small Letter M
        '\u{036C}' => Some(26), // Combining Latin Small Letter R
        '\u{036D}' => Some(27), // Combining Latin Small Letter T
        '\u{036E}' => Some(28), // Combining Latin Small Letter V
        '\u{036F}' => Some(29), // Combining Latin Small Letter X
        '\u{0483}' => Some(30), // Combining Cyrillic Titlo
        '\u{0484}' => Some(31), // Combining Cyrillic Palatalization
        '\u{0485}' => Some(32), // Combining Cyrillic Dasia Pneumata
        '\u{0486}' => Some(33), // Combining Cyrillic Psili Pneumata
        '\u{0487}' => Some(34), // Combining Cyrillic Pokrytie
        '\u{0592}' => Some(35), // Hebrew Accent Segol
        '\u{0593}' => Some(36), // Hebrew Accent Shalshelet
        '\u{0594}' => Some(37), // Hebrew Accent Zaqef Qatan
        '\u{0595}' => Some(38), // Hebrew Accent Zaqef Gadol
        '\u{0597}' => Some(39), // Hebrew Accent Revia
        '\u{0598}' => Some(40), // Hebrew Accent Zarqa
        '\u{0599}' => Some(41), // Hebrew Accent Pashta
        '\u{059C}' => Some(42), // Hebrew Accent Geresh
        '\u{059D}' => Some(43), // Hebrew Accent Geresh Muqdam
        '\u{059E}' => Some(44), // Hebrew Accent Gershayim
        '\u{059F}' => Some(45), // Hebrew Accent Qarney Para
        '\u{05A0}' => Some(46), // Hebrew Accent Telisha Gedola
        '\u{05A1}' => Some(47), // Hebrew Accent Pazer
        '\u{05A8}' => Some(48), // Hebrew Accent Qadma
        '\u{05A9}' => Some(49), // Hebrew Accent Telisha Qetana
        '\u{05AB}' => Some(50), // Hebrew Accent Ole
        '\u{05AC}' => Some(51), // Hebrew Accent Iluy
        '\u{05AF}' => Some(52), // Hebrew Mark Masora Circle
        '\u{05C4}' => Some(53), // Hebrew Mark Upper Dot
        '\u{0610}' => Some(54), // Arabic Sign Sallallahou Alayhe Wassallam
        '\u{0611}' => Some(55), // Arabic Sign Alayhe Assallam
        '\u{0612}' => Some(56), // Arabic Sign Rahmatullah Alayhe
        '\u{0613}' => Some(57), // Arabic Sign Radi Allahou Anhu
        '\u{0614}' => Some(58), // Arabic Sign Takhallus
        '\u{0615}' => Some(59), // Arabic Small High Tah
        '\u{0616}' => Some(60), // Arabic Small High Ligature Alef with Lam with Yeh
        '\u{0617}' => Some(61), // Arabic Small High Zain
        '\u{0657}' => Some(62), // Arabic Inverted Damma
        '\u{0658}' => Some(63), // Arabic Mark Noon Ghunna
        _ => None,
    }
}

/// Information extracted from a Unicode placeholder cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaceholderInfo {
    /// Image ID (from foreground color)
    pub image_id: u32,
    /// Placement ID (from underline color, 0 if not specified)
    pub placement_id: u32,
    /// Row position (from first diacritic)
    pub row: Option<u8>,
    /// Column position (from second diacritic)
    pub col: Option<u8>,
    /// Most significant byte of image ID (from third diacritic)
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
    pub fn with_diacritics(mut self, row: Option<u8>, col: Option<u8>, msb: Option<u8>) -> Self {
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
    pub fn can_inherit_from(&self, prev: &PlaceholderInfo, expected_col: u8) -> bool {
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
/// - First diacritic: row (0-63)
/// - Second diacritic: column (0-63)
/// - Third diacritic: MSB of image ID (0-63, optional)
///
/// If MSB is 0 or None, it is omitted.
pub fn create_placeholder_with_diacritics(row: u8, col: u8, msb: Option<u8>) -> String {
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
            if let Some(msb_diacritic) = number_to_diacritic(msb_val) {
                result.push(msb_diacritic);
            }
        }
    }

    result
}

/// Parse diacritics from a string of combining characters
///
/// Returns (row, col, msb) as parsed from the diacritics
pub fn parse_diacritics(diacritics: &str) -> (Option<u8>, Option<u8>, Option<u8>) {
    let mut chars: Vec<char> = diacritics.chars().collect();

    // Remove any non-diacritic characters
    chars.retain(|&c| diacritic_to_number(c).is_some());

    let row = chars.first().and_then(|&c| diacritic_to_number(c));
    let col = chars.get(1).and_then(|&c| diacritic_to_number(c));
    let msb = chars.get(2).and_then(|&c| diacritic_to_number(c));

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
    fn test_number_to_diacritic() {
        assert_eq!(number_to_diacritic(0), Some('\u{0305}'));
        assert_eq!(number_to_diacritic(1), Some('\u{030D}'));
        assert_eq!(number_to_diacritic(2), Some('\u{030E}'));
        assert_eq!(number_to_diacritic(63), Some('\u{0658}'));
        assert_eq!(number_to_diacritic(64), None);
    }

    #[test]
    fn test_diacritic_mapping() {
        assert_eq!(diacritic_to_number('\u{0305}'), Some(0));
        assert_eq!(diacritic_to_number('\u{030D}'), Some(1));
        assert_eq!(diacritic_to_number('\u{030E}'), Some(2));
        assert_eq!(diacritic_to_number('\u{0658}'), Some(63));
        assert_eq!(diacritic_to_number('a'), None);
    }

    #[test]
    fn test_roundtrip_diacritic_conversion() {
        // Test that number -> diacritic -> number works
        for n in 0..=63 {
            let diacritic = number_to_diacritic(n).unwrap();
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
}
