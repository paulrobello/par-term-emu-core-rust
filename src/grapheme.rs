/// Utilities for handling grapheme clusters, emoji sequences, and Unicode modifiers
/// Check if a character is a variation selector (U+FE0E or U+FE0F)
///
/// Variation selectors control whether a character is rendered as text or emoji:
/// - U+FE0E (VS15) = Text style
/// - U+FE0F (VS16) = Emoji style
///
/// Examples:
/// - ⚠ (U+26A0) + U+FE0E = ⚠ (text style, black & white)
/// - ⚠ (U+26A0) + U+FE0F = ⚠️ (emoji style, colored)
#[inline]
pub fn is_variation_selector(c: char) -> bool {
    c == '\u{FE0E}' || c == '\u{FE0F}'
}

/// Check if a character is a Zero Width Joiner (U+200D)
///
/// ZWJ is used to combine multiple emoji into a single glyph:
/// - 👨‍👩‍👧‍👦 = MAN + ZWJ + WOMAN + ZWJ + GIRL + ZWJ + BOY
/// - 🏳️‍🌈 = WHITE FLAG + ZWJ + RAINBOW
#[inline]
pub fn is_zwj(c: char) -> bool {
    c == '\u{200D}'
}

/// Check if a character is a skin tone modifier (U+1F3FB-U+1F3FF)
///
/// Skin tone modifiers (Emoji Modifier Fitzpatrick Type) modify the preceding emoji:
/// - U+1F3FB = Light skin tone
/// - U+1F3FC = Medium-light skin tone
/// - U+1F3FD = Medium skin tone
/// - U+1F3FE = Medium-dark skin tone
/// - U+1F3FF = Dark skin tone
///
/// Example: 👋🏽 = WAVING HAND (U+1F44B) + MEDIUM SKIN TONE (U+1F3FD)
#[inline]
pub fn is_skin_tone_modifier(c: char) -> bool {
    let code = c as u32;
    (0x1F3FB..=0x1F3FF).contains(&code)
}

/// Check if a character is a Regional Indicator Symbol (U+1F1E6-U+1F1FF)
///
/// Regional indicators are used in pairs to form flag emoji:
/// - 🇺🇸 = U+1F1FA (🇺) + U+1F1F8 (🇸)
/// - 🇬🇧 = U+1F1EC (🇬) + U+1F1E7 (🇧)
#[inline]
pub fn is_regional_indicator(c: char) -> bool {
    let code = c as u32;
    (0x1F1E6..=0x1F1FF).contains(&code)
}

/// Check if a character is a combining mark (diacritics, accents, etc.)
///
/// Combining marks modify the preceding base character.
/// This includes:
/// - Combining Diacritical Marks (U+0300-U+036F)
/// - Combining Marks for Symbols (U+20D0-U+20FF)
/// - And other Unicode combining character categories
#[inline]
pub fn is_combining_mark(c: char) -> bool {
    let code = c as u32;
    matches!(code,
        0x0300..=0x036F | // Combining Diacritical Marks
        0x1AB0..=0x1AFF | // Combining Diacritical Marks Extended
        0x1DC0..=0x1DFF | // Combining Diacritical Marks Supplement
        0x20D0..=0x20FF | // Combining Diacritical Marks for Symbols
        0xFE20..=0xFE2F   // Combining Half Marks
    )
}

/// Check if a grapheme cluster should be rendered with width 2 (wide character)
///
/// Wide emoji include:
/// - Regional indicator pairs (flags): 🇺🇸 🇬🇧
/// - ZWJ sequences: 👨‍👩‍👧‍👦 🏳️‍🌈
/// - Emoji with skin tone modifiers: 👋🏽 👍🏿
/// - Most emoji by default
///
/// # Arguments
///
/// * `grapheme` - The grapheme cluster to check
///
/// # Returns
///
/// true if the grapheme should occupy 2 cells, false if 1 cell
pub fn is_wide_grapheme(grapheme: &str) -> bool {
    // Regional Indicator pairs (flags) are always wide
    let regional_indicators: Vec<char> = grapheme
        .chars()
        .filter(|c| is_regional_indicator(*c))
        .collect();
    if regional_indicators.len() == 2 {
        return true;
    }

    // ZWJ sequences are wide
    if grapheme.contains('\u{200D}') {
        return true;
    }

    // Emoji with skin tone modifiers are wide
    if grapheme.chars().any(is_skin_tone_modifier) {
        return true;
    }

    // Emoji with variation selectors are typically wide
    if grapheme.contains('\u{FE0F}') {
        return true;
    }

    // Fallback to unicode-width for other cases
    unicode_width::UnicodeWidthStr::width(grapheme) >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variation_selector_detection() {
        assert!(is_variation_selector('\u{FE0E}')); // Text style
        assert!(is_variation_selector('\u{FE0F}')); // Emoji style
        assert!(!is_variation_selector('a'));
        assert!(!is_variation_selector('⚠'));
    }

    #[test]
    fn test_zwj_detection() {
        assert!(is_zwj('\u{200D}'));
        assert!(!is_zwj('a'));
        assert!(!is_zwj(' '));
    }

    #[test]
    fn test_skin_tone_modifier_detection() {
        assert!(is_skin_tone_modifier('\u{1F3FB}')); // Light
        assert!(is_skin_tone_modifier('\u{1F3FC}')); // Medium-light
        assert!(is_skin_tone_modifier('\u{1F3FD}')); // Medium
        assert!(is_skin_tone_modifier('\u{1F3FE}')); // Medium-dark
        assert!(is_skin_tone_modifier('\u{1F3FF}')); // Dark
        assert!(!is_skin_tone_modifier('a'));
        assert!(!is_skin_tone_modifier('👋'));
    }

    #[test]
    fn test_regional_indicator_detection() {
        assert!(is_regional_indicator('\u{1F1FA}')); // 🇺
        assert!(is_regional_indicator('\u{1F1F8}')); // 🇸
        assert!(!is_regional_indicator('a'));
    }

    #[test]
    fn test_combining_mark_detection() {
        assert!(is_combining_mark('\u{0301}')); // Combining acute accent
        assert!(is_combining_mark('\u{0300}')); // Combining grave accent
        assert!(!is_combining_mark('a'));
    }

    #[test]
    fn test_wide_grapheme_detection() {
        // ZWJ sequences
        assert!(is_wide_grapheme("👨‍👩‍👧‍👦")); // Family

        // Skin tone modifiers
        assert!(is_wide_grapheme("👋🏽")); // Waving hand with medium skin tone

        // Variation selectors
        assert!(is_wide_grapheme("⚠️")); // Warning with emoji variation

        // ASCII should not be wide
        assert!(!is_wide_grapheme("a"));
    }
}
