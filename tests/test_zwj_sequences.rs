// Comprehensive tests for ZWJ (Zero Width Joiner) sequence support
use par_term_emu_core_rust::terminal::Terminal;

#[test]
fn test_simple_zwj_sequence() {
    let mut term = Terminal::new(80, 24);
    term.process("👨‍💻".as_bytes()); // Man + ZWJ + laptop = technologist

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have ZWJ
    assert!(grapheme.contains('\u{200D}'), "Should contain ZWJ");
    // Should have both emoji
    assert!(grapheme.contains('👨'), "Should contain man emoji");
    assert!(grapheme.contains('💻'), "Should contain laptop emoji");
}

#[test]
fn test_family_zwj_sequence() {
    let mut term = Terminal::new(80, 24);
    term.process("👨‍👩‍👧‍👦".as_bytes()); // Family: man + woman + girl + boy

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have multiple ZWJ
    let zwj_count = grapheme.chars().filter(|c| *c == '\u{200D}').count();
    assert!(zwj_count >= 3, "Should have at least 3 ZWJs for family");
}

#[test]
fn test_rainbow_flag() {
    let mut term = Terminal::new(80, 24);
    term.process("🏳️‍🌈".as_bytes()); // White flag + variation selector + ZWJ + rainbow

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have ZWJ
    assert!(grapheme.contains('\u{200D}'), "Should contain ZWJ");
    // Should have variation selector
    assert!(
        grapheme.contains('\u{FE0F}'),
        "Should contain variation selector"
    );
}

#[test]
fn test_zwj_with_skin_tone() {
    let mut term = Terminal::new(80, 24);
    term.process("👨🏻‍💻".as_bytes()); // Man + light skin + ZWJ + laptop

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have ZWJ
    assert!(grapheme.contains('\u{200D}'), "Should contain ZWJ");
    // Should have skin tone
    assert!(
        grapheme.contains('\u{1F3FB}'),
        "Should contain light skin tone"
    );
    // Should have both emoji
    assert!(grapheme.contains('👨'), "Should contain man emoji");
    assert!(grapheme.contains('💻'), "Should contain laptop emoji");
}

#[test]
fn test_multiple_zwj_sequences() {
    let mut term = Terminal::new(80, 24);

    // Test first sequence
    term.process("👨‍💻".as_bytes());
    let cell1 = term.active_grid().get(0, 0).unwrap();
    let grapheme1 = cell1.get_grapheme();
    assert!(
        grapheme1.contains('\u{200D}'),
        "First sequence should have ZWJ"
    );
    assert!(grapheme1.contains('👨'), "First should have man");
    assert!(grapheme1.contains('💻'), "First should have laptop");

    // Test second sequence on new line
    let mut term2 = Terminal::new(80, 24);
    term2.process("👩‍🔬".as_bytes());
    let cell2 = term2.active_grid().get(0, 0).unwrap();
    let grapheme2 = cell2.get_grapheme();
    assert!(
        grapheme2.contains('\u{200D}'),
        "Second sequence should have ZWJ"
    );
    assert!(grapheme2.contains('👩'), "Second should have woman");
}

#[test]
fn test_zwj_sequence_width() {
    let mut term = Terminal::new(80, 24);
    term.process("👨‍💻".as_bytes());

    // ZWJ sequence should still be wide (2 cells)
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.width(), 2, "ZWJ sequence should be wide");
    assert!(cell.flags.wide_char(), "Should be marked as wide char");

    // Next cell should be a spacer
    let spacer = term.active_grid().get(1, 0).unwrap();
    assert!(
        spacer.flags.wide_char_spacer(),
        "Next cell should be spacer"
    );
}

#[test]
fn test_profession_emojis() {
    let test_cases = vec![
        ("👨‍💻", "Technologist"),
        ("👨‍🔬", "Scientist"),
        ("👨‍⚕️", "Health worker"),
        ("👨‍🚀", "Astronaut"),
    ];

    for (emoji, _name) in test_cases {
        let mut term = Terminal::new(80, 24);
        term.process(emoji.as_bytes());

        let cell = term.active_grid().get(0, 0).unwrap();
        let grapheme = cell.get_grapheme();

        assert!(
            grapheme.contains('\u{200D}'),
            "Profession emoji should have ZWJ: {}",
            emoji
        );
    }
}

#[test]
fn test_profession_with_skin_tones() {
    let test_cases = vec![
        ("👨🏻‍💻", "Light skin technologist"),
        ("👩🏽‍🔬", "Medium skin scientist"),
        ("👨🏿‍🚀", "Dark skin astronaut"),
    ];

    for (emoji, name) in test_cases {
        let mut term = Terminal::new(80, 24);
        term.process(emoji.as_bytes());

        let cell = term.active_grid().get(0, 0).unwrap();
        let grapheme = cell.get_grapheme();

        assert!(grapheme.contains('\u{200D}'), "{} should have ZWJ", name);

        // Check for skin tone modifier
        let has_skin_tone = grapheme.chars().any(|c| {
            let code = c as u32;
            (0x1F3FB..=0x1F3FF).contains(&code)
        });
        assert!(has_skin_tone, "{} should have skin tone", name);
    }
}

#[test]
fn test_couple_emojis() {
    let mut term = Terminal::new(80, 24);
    term.process("👨‍❤️‍👨".as_bytes()); // Man + heart + man

    let cell = term.active_grid().get(0, 0).unwrap();
    let grapheme = cell.get_grapheme();

    // Should have multiple ZWJ
    let zwj_count = grapheme.chars().filter(|c| *c == '\u{200D}').count();
    assert!(zwj_count >= 2, "Couple emoji should have at least 2 ZWJs");
}

#[test]
fn test_zwj_sequence_preservation() {
    let _term = Terminal::new(80, 24);

    // Process multiple different ZWJ sequences
    let sequences = vec!["👨‍💻", "👩‍🔬", "👨‍⚕️"];

    for emoji in sequences {
        let mut term2 = Terminal::new(80, 24);
        term2.process(emoji.as_bytes());
        let cell = term2.active_grid().get(0, 0).unwrap();
        assert!(
            cell.get_grapheme().contains('\u{200D}'),
            "Sequence {} should preserve ZWJ",
            emoji
        );
    }
}
