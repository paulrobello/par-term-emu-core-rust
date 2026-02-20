use crate::terminal::screen::{hsl_to_rgb, hsv_to_rgb, rgb_to_hsl, rgb_to_hsv, ColorHSL, ColorHSV};

// ─── Color Math ────────────────────────────────────────────────────────────

#[test]
fn test_rgb_to_hsv_red() {
    let hsv = rgb_to_hsv(255, 0, 0);
    assert!(
        (hsv.h - 0.0).abs() < 1.0,
        "hue should be ~0 for red, got {}",
        hsv.h
    );
    assert!(
        (hsv.s - 1.0).abs() < 0.01,
        "saturation should be 1.0, got {}",
        hsv.s
    );
    assert!(
        (hsv.v - 1.0).abs() < 0.01,
        "value should be 1.0, got {}",
        hsv.v
    );
}

#[test]
fn test_rgb_to_hsv_green() {
    let hsv = rgb_to_hsv(0, 255, 0);
    assert!(
        (hsv.h - 120.0).abs() < 1.0,
        "hue should be ~120 for green, got {}",
        hsv.h
    );
    assert!((hsv.s - 1.0).abs() < 0.01);
    assert!((hsv.v - 1.0).abs() < 0.01);
}

#[test]
fn test_rgb_to_hsv_blue() {
    let hsv = rgb_to_hsv(0, 0, 255);
    assert!(
        (hsv.h - 240.0).abs() < 1.0,
        "hue should be ~240 for blue, got {}",
        hsv.h
    );
    assert!((hsv.s - 1.0).abs() < 0.01);
    assert!((hsv.v - 1.0).abs() < 0.01);
}

#[test]
fn test_rgb_to_hsv_black() {
    let hsv = rgb_to_hsv(0, 0, 0);
    assert!(
        (hsv.s - 0.0).abs() < 0.01,
        "saturation of black should be 0"
    );
    assert!((hsv.v - 0.0).abs() < 0.01, "value of black should be 0");
}

#[test]
fn test_rgb_to_hsv_white() {
    let hsv = rgb_to_hsv(255, 255, 255);
    assert!(
        (hsv.s - 0.0).abs() < 0.01,
        "saturation of white should be 0"
    );
    assert!((hsv.v - 1.0).abs() < 0.01, "value of white should be 1.0");
}

#[test]
fn test_hsv_to_rgb_roundtrip_red() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 0.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_roundtrip_green() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 120.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 0);
    assert_eq!(g, 255);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_roundtrip_blue() {
    let (r, g, b) = hsv_to_rgb(ColorHSV {
        h: 240.0,
        s: 1.0,
        v: 1.0,
    });
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_rgb_hsv_full_roundtrip() {
    let (r_in, g_in, b_in) = (200u8, 100u8, 50u8);
    let hsv = rgb_to_hsv(r_in, g_in, b_in);
    let (r_out, g_out, b_out) = hsv_to_rgb(hsv);
    assert!(
        (r_in as i32 - r_out as i32).abs() <= 2,
        "r: {} vs {}",
        r_in,
        r_out
    );
    assert!(
        (g_in as i32 - g_out as i32).abs() <= 2,
        "g: {} vs {}",
        g_in,
        g_out
    );
    assert!(
        (b_in as i32 - b_out as i32).abs() <= 2,
        "b: {} vs {}",
        b_in,
        b_out
    );
}

#[test]
fn test_rgb_to_hsl_red() {
    let hsl = rgb_to_hsl(255, 0, 0);
    assert!(
        (hsl.h - 0.0).abs() < 1.0,
        "hue should be ~0 for red, got {}",
        hsl.h
    );
    assert!((hsl.s - 1.0).abs() < 0.01, "saturation should be 1.0");
    assert!(
        (hsl.l - 0.5).abs() < 0.01,
        "lightness of pure red should be 0.5"
    );
}

#[test]
fn test_rgb_to_hsl_white() {
    let hsl = rgb_to_hsl(255, 255, 255);
    assert!(
        (hsl.l - 1.0).abs() < 0.01,
        "lightness of white should be 1.0"
    );
    assert!(
        (hsl.s - 0.0).abs() < 0.01,
        "saturation of white should be 0"
    );
}

#[test]
fn test_rgb_to_hsl_black() {
    let hsl = rgb_to_hsl(0, 0, 0);
    assert!((hsl.l - 0.0).abs() < 0.01, "lightness of black should be 0");
    assert!(
        (hsl.s - 0.0).abs() < 0.01,
        "saturation of black should be 0"
    );
}

#[test]
fn test_hsl_to_rgb_roundtrip_red() {
    let (r, g, b) = hsl_to_rgb(ColorHSL {
        h: 0.0,
        s: 1.0,
        l: 0.5,
    });
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_rgb_hsl_full_roundtrip() {
    let (r_in, g_in, b_in) = (128u8, 64u8, 192u8);
    let hsl = rgb_to_hsl(r_in, g_in, b_in);
    let (r_out, g_out, b_out) = hsl_to_rgb(hsl);
    assert!(
        (r_in as i32 - r_out as i32).abs() <= 2,
        "r: {} vs {}",
        r_in,
        r_out
    );
    assert!(
        (g_in as i32 - g_out as i32).abs() <= 2,
        "g: {} vs {}",
        g_in,
        g_out
    );
    assert!(
        (b_in as i32 - b_out as i32).abs() <= 2,
        "b: {} vs {}",
        b_in,
        b_out
    );
}

// ─── Selection API ─────────────────────────────────────────────────────────

use crate::terminal::screen::SelectionMode;
use crate::terminal::Terminal;

#[test]
fn test_set_and_get_selection() {
    let mut term = Terminal::new(80, 24);
    term.set_selection((5, 2), (15, 2), SelectionMode::Character);
    let sel = term.get_selection().expect("selection should be set");
    assert_eq!(sel.start, (5, 2));
    assert_eq!(sel.end, (15, 2));
}

#[test]
fn test_get_selected_text() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello world");
    term.set_selection((6, 0), (11, 0), SelectionMode::Character);
    let text = term.get_selected_text().expect("should have selected text");
    assert!(
        text.contains("world"),
        "selected text should contain 'world', got: {:?}",
        text
    );
}

#[test]
fn test_clear_selection() {
    let mut term = Terminal::new(80, 24);
    term.set_selection((0, 0), (5, 0), SelectionMode::Character);
    assert!(term.get_selection().is_some());
    term.clear_selection();
    assert!(term.get_selection().is_none());
}

#[test]
fn test_select_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello");
    term.select_line(0);
    let sel = term
        .get_selection()
        .expect("should have selection after select_line");
    assert_eq!(sel.start.1, 0, "selection should be on row 0");
    assert_eq!(sel.end.1, 0, "selection should end on row 0");
}

#[test]
fn test_select_word() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello world");
    let result = term.select_word(6, 0, None);
    assert!(result.is_some(), "should find word at col 6");
}

#[test]
fn test_select_word_at_sets_selection() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello world");
    term.select_word_at(6, 0);
    let sel = term.get_selection();
    assert!(sel.is_some(), "select_word_at should set a selection");
}

#[test]
fn test_get_selection_initially_none() {
    let term = Terminal::new(80, 24);
    assert!(term.get_selection().is_none());
}

#[test]
fn test_select_semantic_region() {
    let mut term = Terminal::new(80, 24);
    term.process(b"(hello world)");
    let _result = term.select_semantic_region(1, 0, Some("()[]{}"));
    // Just verify it doesn't panic - result may be None or Some depending on implementation
}

// ─── Damage Regions ────────────────────────────────────────────────────────

#[test]
fn test_add_and_poll_damage_region() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(0, 0, 10, 5);
    let regions = term.poll_damage_regions();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].left, 0);
    assert_eq!(regions[0].top, 0);
    assert_eq!(regions[0].right, 10);
    assert_eq!(regions[0].bottom, 5);
}

#[test]
fn test_poll_damage_regions_clears_internal_state() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(0, 0, 5, 5);
    let first = term.poll_damage_regions();
    assert_eq!(first.len(), 1);
    let second = term.poll_damage_regions();
    assert_eq!(second.len(), 0, "poll should clear regions");
}

#[test]
fn test_get_damage_regions_non_destructive() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(1, 2, 3, 4);
    let first = term.get_damage_regions().to_vec();
    let second = term.get_damage_regions().to_vec();
    assert_eq!(
        first.len(),
        second.len(),
        "get_damage_regions should not clear"
    );
}

#[test]
fn test_clear_damage_regions() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(0, 0, 10, 10);
    assert!(!term.get_damage_regions().is_empty());
    term.clear_damage_regions();
    assert!(term.get_damage_regions().is_empty());
}

#[test]
fn test_add_multiple_damage_regions() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(0, 0, 5, 5);
    term.add_damage_region(10, 10, 20, 20);
    let regions = term.get_damage_regions();
    assert_eq!(regions.len(), 2);
}

#[test]
fn test_merge_damage_regions() {
    let mut term = Terminal::new(80, 24);
    term.add_damage_region(0, 0, 5, 5);
    term.add_damage_region(3, 3, 10, 10);
    term.merge_damage_regions();
    let regions = term.get_damage_regions();
    assert!(!regions.is_empty(), "merged regions should not be empty");
}

// ─── Rendering Hints ───────────────────────────────────────────────────────

#[test]
fn test_add_and_poll_rendering_hints() {
    use crate::terminal::screen::{
        AnimationHint, DamageRegion, RenderingHint, UpdatePriority, ZLayer,
    };
    let mut term = Terminal::new(80, 24);
    let hint = RenderingHint {
        damage: DamageRegion {
            left: 0,
            top: 0,
            right: 5,
            bottom: 5,
        },
        layer: ZLayer::Normal,
        animation: AnimationHint::None,
        priority: UpdatePriority::Normal,
    };
    term.add_rendering_hint(hint);
    let hints = term.poll_rendering_hints();
    assert_eq!(hints.len(), 1);
}

#[test]
fn test_poll_rendering_hints_clears() {
    use crate::terminal::screen::{
        AnimationHint, DamageRegion, RenderingHint, UpdatePriority, ZLayer,
    };
    let mut term = Terminal::new(80, 24);
    let hint = RenderingHint {
        damage: DamageRegion {
            left: 0,
            top: 0,
            right: 5,
            bottom: 5,
        },
        layer: ZLayer::Normal,
        animation: AnimationHint::None,
        priority: UpdatePriority::Normal,
    };
    term.add_rendering_hint(hint);
    let _ = term.poll_rendering_hints();
    let second = term.poll_rendering_hints();
    assert_eq!(second.len(), 0, "poll should clear hints");
}

#[test]
fn test_get_rendering_hints_non_destructive() {
    use crate::terminal::screen::{
        AnimationHint, DamageRegion, RenderingHint, UpdatePriority, ZLayer,
    };
    let mut term = Terminal::new(80, 24);
    let hint = RenderingHint {
        damage: DamageRegion {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        },
        layer: ZLayer::Overlay,
        animation: AnimationHint::Fade,
        priority: UpdatePriority::High,
    };
    term.add_rendering_hint(hint);
    let first = term.get_rendering_hints(false);
    let second = term.get_rendering_hints(false);
    assert_eq!(
        first.len(),
        second.len(),
        "get_rendering_hints should not clear"
    );
}

#[test]
fn test_clear_rendering_hints() {
    use crate::terminal::screen::{
        AnimationHint, DamageRegion, RenderingHint, UpdatePriority, ZLayer,
    };
    let mut term = Terminal::new(80, 24);
    let hint = RenderingHint {
        damage: DamageRegion {
            left: 0,
            top: 0,
            right: 5,
            bottom: 5,
        },
        layer: ZLayer::Normal,
        animation: AnimationHint::None,
        priority: UpdatePriority::Low,
    };
    term.add_rendering_hint(hint);
    term.clear_rendering_hints();
    assert!(term.get_rendering_hints(false).is_empty());
}

// ─── Text Extraction ───────────────────────────────────────────────────────

#[test]
fn test_get_word_at() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello world");
    let word = term.get_word_at(6, 0, None);
    assert_eq!(word, Some("world".to_string()));
}

#[test]
fn test_get_word_at_first_word() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello world");
    let word = term.get_word_at(0, 0, None);
    assert_eq!(word, Some("hello".to_string()));
}

#[test]
fn test_get_word_at_out_of_bounds_returns_none() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hi");
    let word = term.get_word_at(1000, 0, None);
    assert!(word.is_none());
}

#[test]
fn test_get_paragraph_at_single_paragraph() {
    let mut term = Terminal::new(80, 24);
    term.process(b"line one\r\nline two");
    let para = term.get_paragraph_at(0);
    assert!(
        para.contains("line one"),
        "paragraph should contain 'line one', got: {:?}",
        para
    );
}

#[test]
fn test_join_wrapped_lines_no_wrap() {
    let mut term = Terminal::new(80, 24);
    term.process(b"short line");
    let joined = term.join_wrapped_lines(0);
    assert!(joined.is_some());
    let j = joined.unwrap();
    assert!(j.text.contains("short line"));
    assert_eq!(j.start_row, 0);
}

#[test]
fn test_get_line_context() {
    let mut term = Terminal::new(80, 24);
    term.process(b"line 0\r\nline 1\r\nline 2\r\nline 3");
    let ctx = term.get_line_context(2, 1, 1);
    assert_eq!(ctx.len(), 3, "should return 3 lines: before, target, after");
}

#[test]
fn test_get_logical_lines() {
    let mut term = Terminal::new(80, 24);
    term.process(b"hello\r\nworld");
    let lines = term.get_logical_lines();
    assert!(!lines.is_empty());
}

#[test]
fn test_get_line_unwrapped() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test line");
    let line = term.get_line_unwrapped(0);
    assert!(line.is_some());
    assert!(line.unwrap().contains("test line"));
}

#[test]
fn test_is_line_start() {
    let term = Terminal::new(80, 24);
    assert!(term.is_line_start(0));
}
