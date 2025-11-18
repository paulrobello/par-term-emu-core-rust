//! Terminal conformance level support (VT100/VT220/VT320/VT420/VT520)
//!
//! This module defines conformance levels for DEC VT terminal emulation.
//! The conformance level determines which control sequences and features are available.

use std::fmt;

/// Terminal conformance level
///
/// Determines which VT terminal features and sequences are supported.
/// Higher levels are backward compatible with lower levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ConformanceLevel {
    /// VT100 level (level 1)
    /// - Basic cursor movement (CUU, CUD, CUF, CUB, CUP, HVP)
    /// - Basic editing (ED, EL)
    /// - SGR (basic colors and attributes)
    /// - Simple scrolling
    VT100 = 1,

    /// VT220 level (level 2)
    /// - All VT100 features
    /// - Line/character editing (IL, DL, ICH, DCH, ECH)
    /// - 8-bit controls
    /// - ANSI color support
    /// - Device attributes
    VT220 = 2,

    /// VT320 level (level 3)
    /// - All VT220 features
    /// - Additional text attributes
    /// - Enhanced mode support
    VT320 = 3,

    /// VT420 level (level 4)
    /// - All VT320 features
    /// - Rectangle operations (DECFRA, DECCRA, DECERA, DECSERA, DECCARA, DECRARA)
    /// - Character protection (DECSCA, DECSED, DECSEL)
    /// - Left/right margins (DECSLRM, DECLRMM)
    VT420 = 4,

    /// VT520 level (level 5)
    /// - All VT420 features
    /// - Enhanced conformance level control (DECSCL)
    /// - Bell volume control (DECSWBV, DECSMBV)
    /// - Additional status reports
    #[default]
    VT520 = 5,
}

impl ConformanceLevel {
    /// Get the level number (1-5)
    pub fn level(&self) -> u8 {
        *self as u8
    }

    /// Get the Device Attributes (DA) identifier
    ///
    /// Returns the identifier used in Primary DA response:
    /// - VT100: 1
    /// - VT220: 62
    /// - VT320: 63
    /// - VT420: 64
    /// - VT520: 65
    pub fn da_identifier(&self) -> u8 {
        match self {
            ConformanceLevel::VT100 => 1,
            ConformanceLevel::VT220 => 62,
            ConformanceLevel::VT320 => 63,
            ConformanceLevel::VT420 => 64,
            ConformanceLevel::VT520 => 65,
        }
    }

    /// Parse conformance level from DECSCL parameter
    ///
    /// DECSCL format: CSI Pl ; Pc " p
    /// Where Pl is the conformance level:
    /// - 61 or 1 = VT100
    /// - 62 or 2 = VT220
    /// - 63 or 3 = VT320
    /// - 64 or 4 = VT420
    /// - 65 or 5 = VT520
    pub fn from_decscl_param(param: u16) -> Option<Self> {
        match param {
            1 | 61 => Some(ConformanceLevel::VT100),
            2 | 62 => Some(ConformanceLevel::VT220),
            3 | 63 => Some(ConformanceLevel::VT320),
            4 | 64 => Some(ConformanceLevel::VT420),
            5 | 65 => Some(ConformanceLevel::VT520),
            _ => None,
        }
    }

    /// Check if a feature is supported at this conformance level
    pub fn supports(&self, feature: Feature) -> bool {
        let required_level = feature.required_level();
        *self >= required_level
    }
}

impl fmt::Display for ConformanceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConformanceLevel::VT100 => write!(f, "VT100"),
            ConformanceLevel::VT220 => write!(f, "VT220"),
            ConformanceLevel::VT320 => write!(f, "VT320"),
            ConformanceLevel::VT420 => write!(f, "VT420"),
            ConformanceLevel::VT520 => write!(f, "VT520"),
        }
    }
}

/// Terminal features that require specific conformance levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// Basic cursor movement
    CursorMovement,
    /// Line/character editing (IL, DL, ICH, DCH, ECH)
    LineEditing,
    /// Rectangle operations
    RectangleOperations,
    /// Character protection
    CharacterProtection,
    /// Left/right margins
    LeftRightMargins,
    /// Bell volume control
    BellVolumeControl,
}

impl Feature {
    /// Get the minimum conformance level required for this feature
    pub fn required_level(&self) -> ConformanceLevel {
        match self {
            Feature::CursorMovement => ConformanceLevel::VT100,
            Feature::LineEditing => ConformanceLevel::VT220,
            Feature::RectangleOperations => ConformanceLevel::VT420,
            Feature::CharacterProtection => ConformanceLevel::VT420,
            Feature::LeftRightMargins => ConformanceLevel::VT420,
            Feature::BellVolumeControl => ConformanceLevel::VT520,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conformance_level_ordering() {
        assert!(ConformanceLevel::VT100 < ConformanceLevel::VT220);
        assert!(ConformanceLevel::VT220 < ConformanceLevel::VT320);
        assert!(ConformanceLevel::VT320 < ConformanceLevel::VT420);
        assert!(ConformanceLevel::VT420 < ConformanceLevel::VT520);
    }

    #[test]
    fn test_level_numbers() {
        assert_eq!(ConformanceLevel::VT100.level(), 1);
        assert_eq!(ConformanceLevel::VT220.level(), 2);
        assert_eq!(ConformanceLevel::VT320.level(), 3);
        assert_eq!(ConformanceLevel::VT420.level(), 4);
        assert_eq!(ConformanceLevel::VT520.level(), 5);
    }

    #[test]
    fn test_da_identifiers() {
        assert_eq!(ConformanceLevel::VT100.da_identifier(), 1);
        assert_eq!(ConformanceLevel::VT220.da_identifier(), 62);
        assert_eq!(ConformanceLevel::VT320.da_identifier(), 63);
        assert_eq!(ConformanceLevel::VT420.da_identifier(), 64);
        assert_eq!(ConformanceLevel::VT520.da_identifier(), 65);
    }

    #[test]
    fn test_from_decscl_param() {
        // Short form
        assert_eq!(
            ConformanceLevel::from_decscl_param(1),
            Some(ConformanceLevel::VT100)
        );
        assert_eq!(
            ConformanceLevel::from_decscl_param(2),
            Some(ConformanceLevel::VT220)
        );
        assert_eq!(
            ConformanceLevel::from_decscl_param(5),
            Some(ConformanceLevel::VT520)
        );

        // Long form
        assert_eq!(
            ConformanceLevel::from_decscl_param(61),
            Some(ConformanceLevel::VT100)
        );
        assert_eq!(
            ConformanceLevel::from_decscl_param(62),
            Some(ConformanceLevel::VT220)
        );
        assert_eq!(
            ConformanceLevel::from_decscl_param(65),
            Some(ConformanceLevel::VT520)
        );

        // Invalid
        assert_eq!(ConformanceLevel::from_decscl_param(0), None);
        assert_eq!(ConformanceLevel::from_decscl_param(6), None);
        assert_eq!(ConformanceLevel::from_decscl_param(99), None);
    }

    #[test]
    fn test_feature_support() {
        let vt100 = ConformanceLevel::VT100;
        let vt220 = ConformanceLevel::VT220;
        let vt420 = ConformanceLevel::VT420;
        let vt520 = ConformanceLevel::VT520;

        // VT100 only supports basic features
        assert!(vt100.supports(Feature::CursorMovement));
        assert!(!vt100.supports(Feature::LineEditing));
        assert!(!vt100.supports(Feature::RectangleOperations));

        // VT220 supports VT100 + editing
        assert!(vt220.supports(Feature::CursorMovement));
        assert!(vt220.supports(Feature::LineEditing));
        assert!(!vt220.supports(Feature::RectangleOperations));

        // VT420 supports rectangles
        assert!(vt420.supports(Feature::CursorMovement));
        assert!(vt420.supports(Feature::LineEditing));
        assert!(vt420.supports(Feature::RectangleOperations));
        assert!(vt420.supports(Feature::CharacterProtection));
        assert!(!vt420.supports(Feature::BellVolumeControl));

        // VT520 supports everything
        assert!(vt520.supports(Feature::CursorMovement));
        assert!(vt520.supports(Feature::LineEditing));
        assert!(vt520.supports(Feature::RectangleOperations));
        assert!(vt520.supports(Feature::CharacterProtection));
        assert!(vt520.supports(Feature::BellVolumeControl));
    }

    #[test]
    fn test_default_conformance_level() {
        assert_eq!(ConformanceLevel::default(), ConformanceLevel::VT520);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ConformanceLevel::VT100), "VT100");
        assert_eq!(format!("{}", ConformanceLevel::VT220), "VT220");
        assert_eq!(format!("{}", ConformanceLevel::VT520), "VT520");
    }
}
