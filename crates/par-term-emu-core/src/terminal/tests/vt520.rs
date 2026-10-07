// VT520 conformance tests
use crate::conformance_level::ConformanceLevel;
use crate::terminal::*;

#[test]
fn test_vt520_conformance_level_default() {
    let term = Terminal::new(80, 24);
    assert_eq!(term.conformance_level, ConformanceLevel::VT520);
}

#[test]
fn test_vt520_decscl_set_conformance_level() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[62\"p");
    assert_eq!(term.conformance_level, ConformanceLevel::VT220);
}
