//! VT520 conformance levels, device attributes, and bell volumes.

use super::*;

// ============================================================================
// VT520 Tests
// ============================================================================

#[test]
fn test_vt520_conformance_level_default() {
    let term = Terminal::new(80, 24);
    // Default conformance level should be VT520
    assert_eq!(term.conformance_level.level(), 5);
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );
}

#[test]
fn test_vt520_decscl_set_conformance_level() {
    let mut term = Terminal::new(80, 24);

    // Set to VT100 (short form)
    term.process(b"\x1b[1\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT100
    );

    // Set to VT220 (long form)
    term.process(b"\x1b[62\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT220
    );

    // Set to VT320 (short form)
    term.process(b"\x1b[3\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT320
    );

    // Set to VT420 (long form)
    term.process(b"\x1b[64\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT420
    );

    // Set to VT520 (short form)
    term.process(b"\x1b[5\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );

    // Set to VT520 (long form)
    term.process(b"\x1b[65\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT520
    );
}

#[test]
fn test_vt520_decscl_with_8bit_mode() {
    let mut term = Terminal::new(80, 24);

    // Set to VT220 with 8-bit mode
    term.process(b"\x1b[62;2\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT220
    );

    // Set to VT420 with 7-bit mode
    term.process(b"\x1b[64;0\"p");
    assert_eq!(
        term.conformance_level,
        crate::conformance_level::ConformanceLevel::VT420
    );
}

#[test]
fn test_vt520_decscl_invalid_level() {
    let mut term = Terminal::new(80, 24);
    let original_level = term.conformance_level;

    // Invalid level should be ignored
    term.process(b"\x1b[99\"p");
    assert_eq!(term.conformance_level, original_level);

    term.process(b"\x1b[0\"p");
    assert_eq!(term.conformance_level, original_level);
}

#[test]
fn test_vt520_device_attributes_vt100() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1\"p"); // Set to VT100

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT100 (id=1)
    assert!(response.starts_with("\x1b[?1;"));
}

#[test]
fn test_vt520_device_attributes_vt220() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p"); // Set to VT220

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT220 (id=62)
    assert!(response.starts_with("\x1b[?62;"));
}

#[test]
fn test_vt520_device_attributes_vt520() {
    let mut term = Terminal::new(80, 24);
    // Default is VT520

    // Request Primary DA
    term.process(b"\x1b[c");

    let responses = term.drain_responses();
    let response = String::from_utf8(responses).unwrap();
    // Should report VT520 (id=65)
    assert!(response.starts_with("\x1b[?65;"));
    // Should include features: 1;4;6;9;15;22
    assert!(response.contains("1;4;6;9;15;22;52c"));
}

#[test]
fn test_vt520_device_attributes_all_levels() {
    let mut term = Terminal::new(80, 24);

    let test_cases = vec![
        (b"\x1b[61\"p", "\x1b[?1;"),  // VT100
        (b"\x1b[62\"p", "\x1b[?62;"), // VT220
        (b"\x1b[63\"p", "\x1b[?63;"), // VT320
        (b"\x1b[64\"p", "\x1b[?64;"), // VT420
        (b"\x1b[65\"p", "\x1b[?65;"), // VT520
    ];

    for (set_level, expected_da) in test_cases {
        term.process(set_level);
        term.process(b"\x1b[c");

        let responses = term.drain_responses();
        let response = String::from_utf8(responses).unwrap();
        assert!(
            response.starts_with(expected_da),
            "Expected DA to start with {} but got {}",
            expected_da,
            response
        );
    }
}

#[test]
fn test_vt520_decswbv_set_warning_bell_volume() {
    let mut term = Terminal::new(80, 24);

    // Default should be 4 (moderate)
    assert_eq!(term.bell_volumes.warning, 4);

    // Set to off (0)
    term.process(b"\x1b[0 t");
    assert_eq!(term.bell_volumes.warning, 0);

    // Set to low (1)
    term.process(b"\x1b[1 t");
    assert_eq!(term.bell_volumes.warning, 1);

    // Set to medium (4)
    term.process(b"\x1b[4 t");
    assert_eq!(term.bell_volumes.warning, 4);

    // Set to high (8)
    term.process(b"\x1b[8 t");
    assert_eq!(term.bell_volumes.warning, 8);
}

#[test]
fn test_vt520_decswbv_clamps_to_valid_range() {
    let mut term = Terminal::new(80, 24);

    // Values above 8 should be clamped to 8
    term.process(b"\x1b[99 t");
    assert_eq!(term.bell_volumes.warning, 8);

    term.process(b"\x1b[255 t");
    assert_eq!(term.bell_volumes.warning, 8);
}

#[test]
fn test_vt520_decsmbv_set_margin_bell_volume() {
    let mut term = Terminal::new(80, 24);

    // Default should be 4 (moderate)
    assert_eq!(term.bell_volumes.margin, 4);

    // Set to off (0)
    term.process(b"\x1b[0 u");
    assert_eq!(term.bell_volumes.margin, 0);

    // Set to low (1)
    term.process(b"\x1b[1 u");
    assert_eq!(term.bell_volumes.margin, 1);

    // Set to medium (4)
    term.process(b"\x1b[4 u");
    assert_eq!(term.bell_volumes.margin, 4);

    // Set to high (8)
    term.process(b"\x1b[8 u");
    assert_eq!(term.bell_volumes.margin, 8);
}

#[test]
fn test_vt520_decsmbv_clamps_to_valid_range() {
    let mut term = Terminal::new(80, 24);

    // Values above 8 should be clamped to 8
    term.process(b"\x1b[99 u");
    assert_eq!(term.bell_volumes.margin, 8);

    term.process(b"\x1b[200 u");
    assert_eq!(term.bell_volumes.margin, 8);
}

#[test]
fn test_vt520_bell_volumes_independent() {
    let mut term = Terminal::new(80, 24);

    // Set different values for each bell
    term.process(b"\x1b[2 t"); // Warning bell = 2
    term.process(b"\x1b[6 u"); // Margin bell = 6

    assert_eq!(term.bell_volumes.warning, 2);
    assert_eq!(term.bell_volumes.margin, 6);

    // Change warning bell without affecting margin bell
    term.process(b"\x1b[7 t");
    assert_eq!(term.bell_volumes.warning, 7);
    assert_eq!(term.bell_volumes.margin, 6); // Should remain unchanged
}

#[test]
fn test_vt520_conformance_level_feature_support() {
    use crate::conformance_level::{ConformanceLevel, Feature};

    let vt100 = ConformanceLevel::VT100;
    let vt220 = ConformanceLevel::VT220;
    let vt420 = ConformanceLevel::VT420;
    let vt520 = ConformanceLevel::VT520;

    // VT100 supports basic features only
    assert!(vt100.supports(Feature::CursorMovement));
    assert!(!vt100.supports(Feature::LineEditing));
    assert!(!vt100.supports(Feature::RectangleOperations));
    assert!(!vt100.supports(Feature::BellVolumeControl));

    // VT220 adds line editing
    assert!(vt220.supports(Feature::CursorMovement));
    assert!(vt220.supports(Feature::LineEditing));
    assert!(!vt220.supports(Feature::RectangleOperations));
    assert!(!vt220.supports(Feature::BellVolumeControl));

    // VT420 adds rectangles and character protection
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
fn test_vt520_conformance_level_ordering() {
    use crate::conformance_level::ConformanceLevel;

    assert!(ConformanceLevel::VT100 < ConformanceLevel::VT220);
    assert!(ConformanceLevel::VT220 < ConformanceLevel::VT320);
    assert!(ConformanceLevel::VT320 < ConformanceLevel::VT420);
    assert!(ConformanceLevel::VT420 < ConformanceLevel::VT520);
}
