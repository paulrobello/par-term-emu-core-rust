// Comprehensive tests for regional indicator flag emoji support
use par_term_emu_core_rust::terminal::Terminal;

#[test]
fn test_us_flag_basic() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸".as_bytes()); // US flag: U+1F1FA + U+1F1F8

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have both regional indicators combined
    assert_eq!(grapheme, "🇺🇸", "Should contain US flag");
    assert!(cell.has_combining_chars(), "Should have combining chars");
    assert_eq!(cell.combining.len(), 1, "Should have one combining char");
}

#[test]
fn test_uk_flag() {
    let mut term = Terminal::new(80, 24);
    term.process("🇬🇧".as_bytes()); // UK flag

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    assert_eq!(grapheme, "🇬🇧", "Should contain UK flag");
}

#[test]
fn test_japan_flag() {
    let mut term = Terminal::new(80, 24);
    term.process("🇯🇵".as_bytes()); // Japan flag

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    assert_eq!(grapheme, "🇯🇵", "Should contain Japan flag");
}

#[test]
fn test_flag_width() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸".as_bytes());

    // Flag should be wide (2 cells)
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.width(), 2, "Flag should be wide");
    assert!(cell.flags.wide_char(), "Should be marked as wide char");

    // Next cell should be a spacer
    let spacer = term.active_grid().get(1, 0).unwrap();
    assert!(
        spacer.flags.wide_char_spacer(),
        "Next cell should be spacer"
    );
}

#[test]
fn test_multiple_flags() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸🇬🇧🇯🇵".as_bytes()); // Three flags: US, UK, Japan

    // First flag at position 0
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "🇺🇸");

    // Second flag at position 2 (after wide char + spacer)
    assert_eq!(term.active_grid().get(2, 0).unwrap().get_grapheme(), "🇬🇧");

    // Third flag at position 4
    assert_eq!(term.active_grid().get(4, 0).unwrap().get_grapheme(), "🇯🇵");
}

#[test]
fn test_flags_separated_by_space() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸 🇬🇧 🇯🇵".as_bytes()); // Flags with spaces

    // US flag at position 0
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "🇺🇸");

    // Space at position 2
    assert_eq!(term.active_grid().get(2, 0).unwrap().c, ' ');

    // UK flag at position 3
    assert_eq!(term.active_grid().get(3, 0).unwrap().get_grapheme(), "🇬🇧");

    // Space at position 5
    assert_eq!(term.active_grid().get(5, 0).unwrap().c, ' ');

    // Japan flag at position 6
    assert_eq!(term.active_grid().get(6, 0).unwrap().get_grapheme(), "🇯🇵");
}

#[test]
fn test_cursor_position_after_flag() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸".as_bytes());

    // Cursor should be at position 2 (after wide flag + spacer)
    assert_eq!(term.cursor().col, 2, "Cursor should be at column 2");
    assert_eq!(term.cursor().row, 0, "Cursor should be at row 0");
}

#[test]
fn test_flags_with_text() {
    let mut term = Terminal::new(80, 24);
    term.process("Hello 🇺🇸 World".as_bytes());

    // "Hello " is 6 characters
    // Flag at position 6
    assert_eq!(term.active_grid().get(6, 0).unwrap().get_grapheme(), "🇺🇸");

    // " World" starts at position 8
    assert_eq!(term.active_grid().get(8, 0).unwrap().c, ' ');
    assert_eq!(term.active_grid().get(9, 0).unwrap().c, 'W');
}

#[test]
fn test_flag_at_line_end() {
    let mut term = Terminal::new(5, 24); // Small width
    term.process("ABC🇺🇸".as_bytes()); // "ABC" + flag = 3 + 2 = 5 cells

    // ABC at positions 0-2
    assert_eq!(term.active_grid().get(0, 0).unwrap().c, 'A');
    assert_eq!(term.active_grid().get(1, 0).unwrap().c, 'B');
    assert_eq!(term.active_grid().get(2, 0).unwrap().c, 'C');

    // Flag at position 3 (fits exactly in remaining 2 columns)
    assert_eq!(term.active_grid().get(3, 0).unwrap().get_grapheme(), "🇺🇸");
}

#[test]
fn test_row_text_with_flags() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸 USA".as_bytes());

    let row_text = term.active_grid().row_text(0);
    assert!(
        row_text.contains("🇺🇸"),
        "Row text should contain flag: {}",
        row_text
    );
    assert!(
        row_text.contains("USA"),
        "Row text should contain USA: {}",
        row_text
    );
}

#[test]
fn test_all_country_flags() {
    // Test various country flags to ensure they all work
    let flags = vec![
        "🇦🇺", // Australia
        "🇧🇷", // Brazil
        "🇨🇦", // Canada
        "🇨🇳", // China
        "🇩🇪", // Germany
        "🇪🇸", // Spain
        "🇫🇷", // France
        "🇮🇳", // India
        "🇮🇹", // Italy
        "🇰🇷", // South Korea
        "🇲🇽", // Mexico
        "🇳🇱", // Netherlands
        "🇷🇺", // Russia
        "🇺🇸", // United States
    ];

    for flag in flags {
        let mut term = Terminal::new(80, 24);
        term.process(flag.as_bytes());

        let cell = term.active_grid().get(0, 0).unwrap();
        let grapheme = cell.get_grapheme();

        assert_eq!(grapheme, flag, "Should correctly store flag: {}", flag);
        assert_eq!(cell.width(), 2, "Flag {} should be wide", flag);
        assert!(
            cell.flags.wide_char(),
            "Flag {} should be marked as wide",
            flag
        );
    }
}

#[test]
fn test_regional_indicator_base_char() {
    let mut term = Terminal::new(80, 24);
    term.process("🇺🇸".as_bytes());

    let cell = term.active_grid().get(0, 0).unwrap();

    // The base char should be the first regional indicator (U)
    assert_eq!(cell.base_char(), '🇺', "Base char should be first indicator");

    // The combining should have the second indicator (S)
    assert_eq!(
        cell.combining.len(),
        1,
        "Should have one combining character"
    );
    assert_eq!(
        cell.combining[0], '🇸',
        "Combining should be second indicator"
    );
}

#[test]
fn test_flag_overwrite() {
    let mut term = Terminal::new(80, 24);

    // Write US flag
    term.process("🇺🇸".as_bytes());
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "🇺🇸");

    // Move cursor back and overwrite with UK flag
    term.process("\x1b[H".as_bytes()); // Move to home position
    term.process("🇬🇧".as_bytes());
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "🇬🇧");
}
