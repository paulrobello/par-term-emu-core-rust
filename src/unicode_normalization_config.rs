//! Configurable Unicode normalization for terminal text
//!
//! This module provides configurable Unicode normalization forms for text
//! received from the PTY before storing in cells. Normalization ensures
//! consistent storage for search, comparison, and cursor movement.
//!
//! # Normalization Forms
//!
//! Unicode characters can have multiple binary representations that look identical.
//! For example, `é` can be:
//! - **Precomposed** (NFC): U+00E9 (single code point)
//! - **Decomposed** (NFD): U+0065 + U+0301 (base + combining mark)
//!
//! Without normalization, string comparison, search, and cursor positioning
//! can produce unexpected results.
//!
//! # Example
//!
//! ```
//! use par_term_emu_core_rust::unicode_normalization_config::NormalizationForm;
//!
//! let form = NormalizationForm::default();
//! assert_eq!(form, NormalizationForm::NFC);
//!
//! let nfc = "e\u{0301}"; // decomposed é
//! let normalized = form.normalize(nfc);
//! assert_eq!(normalized, "\u{00E9}"); // precomposed é
//! ```

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Unicode normalization form for terminal text.
///
/// Controls how Unicode text is normalized before being stored in terminal cells.
/// Normalization ensures consistent representation for search and comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NormalizationForm {
    /// No normalization - store text as received
    #[serde(rename = "none")]
    None,
    /// Canonical Decomposition, followed by Canonical Composition (default)
    ///
    /// Combines characters where possible (`e` + `́` → `é`).
    /// This is the most common form, used by most systems.
    #[default]
    NFC,
    /// Canonical Decomposition
    ///
    /// Splits into base + combining marks (`é` → `e` + `́`).
    /// Used by macOS HFS+ filesystem.
    NFD,
    /// Compatibility Decomposition, followed by Canonical Composition
    ///
    /// NFC + replaces compatibility characters (`ﬁ` → `fi`).
    NFKC,
    /// Compatibility Decomposition
    ///
    /// NFD + replaces compatibility characters.
    NFKD,
}

impl NormalizationForm {
    /// Returns true if normalization is disabled
    #[inline]
    pub fn is_none(&self) -> bool {
        matches!(self, NormalizationForm::None)
    }

    /// Returns a human-readable name for the normalization form
    pub fn name(&self) -> &'static str {
        match self {
            NormalizationForm::None => "none",
            NormalizationForm::NFC => "NFC",
            NormalizationForm::NFD => "NFD",
            NormalizationForm::NFKC => "NFKC",
            NormalizationForm::NFKD => "NFKD",
        }
    }

    /// Normalize a string using this normalization form
    ///
    /// Returns the normalized string. If normalization is `None`,
    /// returns the input unchanged.
    pub fn normalize(&self, s: &str) -> String {
        match self {
            NormalizationForm::None => s.to_string(),
            NormalizationForm::NFC => s.nfc().collect(),
            NormalizationForm::NFD => s.nfd().collect(),
            NormalizationForm::NFKC => s.nfkc().collect(),
            NormalizationForm::NFKD => s.nfkd().collect(),
        }
    }

    /// Normalize a single character, returning the result as a string
    ///
    /// Some normalization forms may decompose a single character into
    /// multiple characters, so the result is a String.
    pub fn normalize_char(&self, c: char) -> String {
        match self {
            NormalizationForm::None => c.to_string(),
            NormalizationForm::NFC => {
                let s: String = core::iter::once(c).collect();
                s.nfc().collect()
            }
            NormalizationForm::NFD => {
                let s: String = core::iter::once(c).collect();
                s.nfd().collect()
            }
            NormalizationForm::NFKC => {
                let s: String = core::iter::once(c).collect();
                s.nfkc().collect()
            }
            NormalizationForm::NFKD => {
                let s: String = core::iter::once(c).collect();
                s.nfkd().collect()
            }
        }
    }

    /// Check if a string is already in this normalization form
    pub fn is_normalized(&self, s: &str) -> bool {
        match self {
            NormalizationForm::None => true,
            _ => self.normalize(s) == s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_nfc() {
        assert_eq!(NormalizationForm::default(), NormalizationForm::NFC);
    }

    #[test]
    fn test_is_none() {
        assert!(NormalizationForm::None.is_none());
        assert!(!NormalizationForm::NFC.is_none());
        assert!(!NormalizationForm::NFD.is_none());
        assert!(!NormalizationForm::NFKC.is_none());
        assert!(!NormalizationForm::NFKD.is_none());
    }

    #[test]
    fn test_name() {
        assert_eq!(NormalizationForm::None.name(), "none");
        assert_eq!(NormalizationForm::NFC.name(), "NFC");
        assert_eq!(NormalizationForm::NFD.name(), "NFD");
        assert_eq!(NormalizationForm::NFKC.name(), "NFKC");
        assert_eq!(NormalizationForm::NFKD.name(), "NFKD");
    }

    #[test]
    fn test_nfc_composes() {
        // e + combining acute accent -> é (precomposed)
        let decomposed = "e\u{0301}";
        let result = NormalizationForm::NFC.normalize(decomposed);
        assert_eq!(result, "\u{00E9}");
    }

    #[test]
    fn test_nfd_decomposes() {
        // é (precomposed) -> e + combining acute accent
        let precomposed = "\u{00E9}";
        let result = NormalizationForm::NFD.normalize(precomposed);
        assert_eq!(result, "e\u{0301}");
    }

    #[test]
    fn test_nfkc_compatibility() {
        // ﬁ (ligature) -> fi
        let ligature = "\u{FB01}";
        let result = NormalizationForm::NFKC.normalize(ligature);
        assert_eq!(result, "fi");
    }

    #[test]
    fn test_nfkd_compatibility() {
        // ﬁ (ligature) -> fi
        let ligature = "\u{FB01}";
        let result = NormalizationForm::NFKD.normalize(ligature);
        assert_eq!(result, "fi");
    }

    #[test]
    fn test_none_passthrough() {
        let s = "e\u{0301}";
        let result = NormalizationForm::None.normalize(s);
        assert_eq!(result, s);
    }

    #[test]
    fn test_ascii_unchanged() {
        let s = "hello world";
        assert_eq!(NormalizationForm::NFC.normalize(s), s);
        assert_eq!(NormalizationForm::NFD.normalize(s), s);
        assert_eq!(NormalizationForm::NFKC.normalize(s), s);
        assert_eq!(NormalizationForm::NFKD.normalize(s), s);
    }

    #[test]
    fn test_normalize_char_nfc() {
        // NFC of a precomposed char is itself
        let result = NormalizationForm::NFC.normalize_char('\u{00E9}');
        assert_eq!(result, "\u{00E9}");
    }

    #[test]
    fn test_normalize_char_nfd() {
        // NFD decomposes precomposed é
        let result = NormalizationForm::NFD.normalize_char('\u{00E9}');
        assert_eq!(result, "e\u{0301}");
    }

    #[test]
    fn test_normalize_char_none() {
        let result = NormalizationForm::None.normalize_char('A');
        assert_eq!(result, "A");
    }

    #[test]
    fn test_is_normalized() {
        // NFC: precomposed should be normalized
        assert!(NormalizationForm::NFC.is_normalized("\u{00E9}"));
        // NFC: decomposed should not be normalized
        assert!(!NormalizationForm::NFC.is_normalized("e\u{0301}"));
        // None: everything is "normalized"
        assert!(NormalizationForm::None.is_normalized("e\u{0301}"));
    }

    #[test]
    fn test_serde_roundtrip() {
        for form in [
            NormalizationForm::None,
            NormalizationForm::NFC,
            NormalizationForm::NFD,
            NormalizationForm::NFKC,
            NormalizationForm::NFKD,
        ] {
            let json = serde_json::to_string(&form).unwrap();
            let deserialized: NormalizationForm = serde_json::from_str(&json).unwrap();
            assert_eq!(form, deserialized, "roundtrip failed for {:?}", form);
        }
    }

    #[test]
    fn test_serde_values() {
        assert_eq!(
            serde_json::to_string(&NormalizationForm::NFC).unwrap(),
            "\"NFC\""
        );
        assert_eq!(
            serde_json::to_string(&NormalizationForm::None).unwrap(),
            "\"none\""
        );
    }

    #[test]
    fn test_hangul_normalization() {
        // Korean Hangul syllable decomposition/composition
        // 한 (U+D55C) NFC == 한, NFD == ᄒ + ᅡ + ᆫ
        let hangul = "\u{D55C}";
        let nfc = NormalizationForm::NFC.normalize(hangul);
        let nfd = NormalizationForm::NFD.normalize(hangul);
        assert_eq!(nfc, hangul);
        assert_ne!(nfd, hangul);
        // Re-composing NFD should give back the original
        let recomposed = NormalizationForm::NFC.normalize(&nfd);
        assert_eq!(recomposed, hangul);
    }

    #[test]
    fn test_multiple_combining_marks() {
        // a + combining tilde + combining acute = NFC should handle
        let s = "a\u{0303}\u{0301}";
        let nfc = NormalizationForm::NFC.normalize(s);
        let nfd = NormalizationForm::NFD.normalize(s);
        // Both should produce valid output (exact result depends on Unicode tables)
        assert!(!nfc.is_empty());
        assert!(!nfd.is_empty());
    }
}
