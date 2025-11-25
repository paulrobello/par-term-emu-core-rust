// Comprehensive tests for skin tone modifier support
use par_term_emu_core_rust::terminal::Terminal;

#[test]
fn test_skin_tone_light() {
    let mut term = Terminal::new(80, 24);
    term.process("👍🏻".as_bytes()); // Light skin
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.get_grapheme(), "👍🏻");
    assert_eq!(cell.combining.len(), 1);
    assert_eq!(cell.combining[0], '\u{1F3FB}');
}

#[test]
fn test_skin_tone_medium_light() {
    let mut term = Terminal::new(80, 24);
    term.process("👍🏼".as_bytes()); // Medium-light skin
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.get_grapheme(), "👍🏼");
    assert_eq!(cell.combining.len(), 1);
    assert_eq!(cell.combining[0], '\u{1F3FC}');
}

#[test]
fn test_skin_tone_medium() {
    let mut term = Terminal::new(80, 24);
    term.process("👋🏽".as_bytes()); // Medium skin
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.get_grapheme(), "👋🏽");
}

#[test]
fn test_skin_tone_medium_dark() {
    let mut term = Terminal::new(80, 24);
    term.process("✊🏾".as_bytes()); // Medium-dark skin
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.get_grapheme(), "✊🏾");
}

#[test]
fn test_skin_tone_dark() {
    let mut term = Terminal::new(80, 24);
    term.process("✊🏿".as_bytes()); // Dark skin
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.get_grapheme(), "✊🏿");
}

#[test]
fn test_all_skin_tones_sequence() {
    let mut term = Terminal::new(80, 24);
    // Default, light, medium-light, medium, medium-dark, dark
    term.process("👍 👍🏻 👍🏼 👍🏽 👍🏾 👍🏿".as_bytes());

    // Check each emoji (they're separated by spaces)
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "👍");
    assert_eq!(term.active_grid().get(3, 0).unwrap().get_grapheme(), "👍🏻");
    assert_eq!(term.active_grid().get(6, 0).unwrap().get_grapheme(), "👍🏼");
    assert_eq!(term.active_grid().get(9, 0).unwrap().get_grapheme(), "👍🏽");
    assert_eq!(term.active_grid().get(12, 0).unwrap().get_grapheme(), "👍🏾");
    assert_eq!(term.active_grid().get(15, 0).unwrap().get_grapheme(), "👍🏿");
}

#[test]
fn test_zwj_with_skin_tone() {
    let mut term = Terminal::new(80, 24);
    term.process("👨🏻‍💻".as_bytes()); // Man + light skin + ZWJ + laptop
    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Verify skin tone is present
    assert!(
        grapheme.contains('\u{1F3FB}'),
        "Should contain light skin tone modifier"
    );
    // Verify ZWJ is present
    assert!(
        grapheme.contains('\u{200D}'),
        "Should contain zero-width joiner"
    );
}

#[test]
fn test_skin_tone_with_variation_selector() {
    let mut term = Terminal::new(80, 24);
    // Some emoji can have both variation selector and skin tone
    term.process("☝🏽".as_bytes()); // Index pointing up + medium skin
    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    assert!(
        grapheme.contains('\u{1F3FD}'),
        "Should contain medium skin tone modifier"
    );
}

#[test]
fn test_multiple_emoji_with_skin_tones() {
    let mut term = Terminal::new(80, 24);
    term.process("👋🏽👍🏻".as_bytes()); // Two emoji with different skin tones

    // First emoji: waving hand with medium skin
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "👋🏽");

    // Second emoji: thumbs up with light skin
    assert_eq!(term.active_grid().get(2, 0).unwrap().get_grapheme(), "👍🏻");
}

#[test]
fn test_skin_tone_width() {
    let mut term = Terminal::new(80, 24);
    term.process("👍🏽".as_bytes());

    // Emoji with skin tone should still be wide (2 cells)
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.width(), 2);
    assert!(cell.flags.wide_char());

    // Next cell should be a spacer
    let spacer = term.active_grid().get(1, 0).unwrap();
    assert!(spacer.flags.wide_char_spacer());
}

#[test]
fn test_family_emoji_with_skin_tones() {
    let mut term = Terminal::new(80, 24);
    // Family emoji can have multiple skin tone modifiers
    term.process("👨🏾‍👩🏻‍👧🏽‍👦🏼".as_bytes());

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Verify it's a ZWJ sequence
    assert!(grapheme.contains('\u{200D}'), "Should contain ZWJ");

    // Verify at least one skin tone modifier is present
    let has_skin_tone = grapheme.chars().any(|c| {
        let code = c as u32;
        (0x1F3FB..=0x1F3FF).contains(&code)
    });
    assert!(
        has_skin_tone,
        "Should contain at least one skin tone modifier"
    );
}

#[test]
fn test_skin_tone_at_line_wrap() {
    let mut term = Terminal::new(5, 24); // Small width to force wrap
    term.process("👍🏽👋🏻".as_bytes()); // Two wide emoji = 4 cells

    // First emoji should be at (0, 0)
    assert_eq!(term.active_grid().get(0, 0).unwrap().get_grapheme(), "👍🏽");

    // Second emoji should be at (2, 0)
    assert_eq!(term.active_grid().get(2, 0).unwrap().get_grapheme(), "👋🏻");
}
