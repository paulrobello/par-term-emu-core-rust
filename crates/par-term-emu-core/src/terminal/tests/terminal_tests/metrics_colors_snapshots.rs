//! Clipboard history, metrics, color utilities, snapshots, and bookmarks.

use super::*;

#[test]
fn test_search_clipboard_history() {
    let mut term = Terminal::new(80, 24);

    // Add clipboard entries (history is recorded by the set_clipboard*
    // API; the raw OSC 52 path only updates the current content)
    term.set_clipboard_with_slot("Hello".to_string(), ClipboardSlot::Clipboard);
    term.set_clipboard_with_slot("World".to_string(), ClipboardSlot::Clipboard);

    // Both writes land in the Clipboard slot's history
    let history = term.get_clipboard_history(ClipboardSlot::Clipboard);
    assert_eq!(history.len(), 2, "both writes recorded: {history:?}");

    let results = term.search_clipboard_history("Hello", None);
    assert_eq!(results.len(), 1, "only the Hello entry matches");
    assert_eq!(results[0].content, "Hello");
}

#[test]
fn test_clipboard_history() {
    let mut term = Terminal::new(80, 24);

    term.set_clipboard_with_slot("Hello".to_string(), ClipboardSlot::Clipboard);
    // The recording API lands entries in the slot's history
    let history = term.get_clipboard_history(ClipboardSlot::Clipboard);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "Hello");
}

#[test]
fn test_performance_metrics_basic() {
    let mut term = Terminal::new(80, 24);

    let metrics = term.get_performance_metrics();
    assert_eq!(metrics.frames_rendered, 0);
    assert_eq!(metrics.cells_updated, 0);

    term.process(b"Hello");
    term.record_frame_timing(100, 5, 5);

    let metrics = term.get_performance_metrics();
    assert!(metrics.frames_rendered > 0);
}

#[test]
fn test_performance_metrics_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Test");
    term.record_frame_timing(100, 4, 4);

    term.reset_performance_metrics();
    let metrics = term.get_performance_metrics();
    assert_eq!(metrics.frames_rendered, 0);
    assert_eq!(metrics.cells_updated, 0);
}

#[test]
fn test_frame_timing() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 10);

    let timings = term.get_frame_timings(Some(1));
    assert!(!timings.is_empty());
    assert_eq!(timings[0].processing_us, 1000);
}

#[test]
fn test_color_hsv_conversion() {
    let hsv = rgb_to_hsv(255, 0, 0); // Red
    assert!((hsv.h - 0.0).abs() < 1.0); // Hue should be ~0
    assert!((hsv.s - 1.0).abs() < 0.01); // Full saturation
    assert!((hsv.v - 1.0).abs() < 0.01); // Full value

    let (r, g, b) = hsv_to_rgb(hsv);
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_color_hsl_conversion() {
    let hsl = rgb_to_hsl(0, 0, 255); // Blue
    assert!((hsl.h - 240.0).abs() < 1.0); // Blue hue
    assert!((hsl.s - 1.0).abs() < 0.01); // Full saturation
    assert!((hsl.l - 0.5).abs() < 0.01); // 50% lightness

    let (r, g, b) = hsl_to_rgb(hsl);
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_generate_color_palette_complementary() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(255, 0, 0, ThemeMode::Complementary);

    assert_eq!(palette.base, (255, 0, 0));
    assert_eq!(palette.mode, ThemeMode::Complementary);
    assert!(!palette.colors.is_empty());
}

#[test]
fn test_generate_color_palette_analogous() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(0, 255, 0, ThemeMode::Analogous);

    assert_eq!(palette.base, (0, 255, 0));
    assert_eq!(palette.mode, ThemeMode::Analogous);
    assert!(palette.colors.len() >= 2);
}

#[test]
fn test_generate_color_palette_triadic() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(128, 64, 192, ThemeMode::Triadic);

    assert_eq!(palette.mode, ThemeMode::Triadic);
    assert!(palette.colors.len() >= 2);
}

#[test]
fn test_color_palette_generation() {
    let term = Terminal::new(80, 24);
    let palette = term.generate_color_palette(255, 128, 64, ThemeMode::Monochromatic);

    assert_eq!(palette.mode, ThemeMode::Monochromatic);
    assert!(!palette.colors.is_empty());
}

#[test]
fn test_snapshot_diff() {
    let old_lines = vec![
        "Line 1".to_string(),
        "Line 2".to_string(),
        "Line 3".to_string(),
    ];
    let new_lines = vec![
        "Line 1".to_string(),
        "Modified Line 2".to_string(),
        "Line 3".to_string(),
    ];

    let diff = diff_screen_lines(&old_lines, &new_lines);
    assert!(diff.modified > 0);
    assert_eq!(diff.unchanged, 2);
}

#[test]
fn test_snapshot_diff_no_changes() {
    let old_lines = vec!["Same".to_string(), "Lines".to_string()];
    let new_lines = vec!["Same".to_string(), "Lines".to_string()];

    let diff = diff_screen_lines(&old_lines, &new_lines);
    assert_eq!(diff.modified, 0);
    assert_eq!(diff.unchanged, 2);
}

#[test]
fn test_diff_snapshots_semantic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"alpha\r\nbeta\r\ngamma");
    let old = term.get_semantic_snapshot(SnapshotScope::Visible);
    term.process(b"\x1b[2;1HBETA\x1b[4;1Hdelta");
    let new = term.get_semantic_snapshot(SnapshotScope::Visible);

    let diff = diff_snapshots(&old, &new);
    // content() pads blank rows, so the empty row gaining "delta" is a
    // modified row alongside the beta -> BETA overwrite
    assert_eq!(diff.modified, 2);
    let modified: Vec<&LineDiff> = diff
        .diffs
        .iter()
        .filter(|d| d.change_type == DiffChangeType::Modified)
        .collect();
    assert_eq!(modified.len(), 2);
    let beta_row = modified
        .iter()
        .find(|d| d.old_content.as_deref() == Some("beta"))
        .expect("beta row diff present");
    assert_eq!(beta_row.new_content.as_deref(), Some("BETA"));
    assert_eq!(beta_row.old_row, Some(1));
    assert_eq!(beta_row.new_row, Some(1));
    let delta_row = modified
        .iter()
        .find(|d| d.new_content.as_deref() == Some("delta"))
        .expect("delta row diff present");
    assert_eq!(delta_row.old_content.as_deref(), Some(""));
    assert_eq!(delta_row.new_row, Some(3));
}

#[test]
fn test_add_bookmark() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    term.process(b"Some content\r\n");

    let _id = term.add_bookmark(0, Some("Test Bookmark".to_string()));

    let bookmarks = term.get_bookmarks();
    assert!(!bookmarks.is_empty());
    assert_eq!(bookmarks[0].label, "Test Bookmark");
}

#[test]
fn test_remove_bookmark() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    let id = term.add_bookmark(0, Some("Test".to_string()));

    assert_eq!(term.get_bookmarks().len(), 1);

    let removed = term.remove_bookmark(id);
    assert!(removed);
    assert_eq!(term.get_bookmarks().len(), 0);
}

#[test]
fn test_clear_all_bookmarks() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    term.add_bookmark(0, Some("First".to_string()));
    term.add_bookmark(1, Some("Second".to_string()));

    assert_eq!(term.get_bookmarks().len(), 2);

    term.clear_bookmarks();
    assert_eq!(term.get_bookmarks().len(), 0);
}

#[test]
fn test_scrollback_stats() {
    let mut term = Terminal::with_scrollback(80, 5, 100);

    // Fill scrollback
    for i in 0..20 {
        term.process(format!("Line {}\r\n", i).as_bytes());
    }

    let stats = term.scrollback_stats();
    assert!(stats.total_lines > 0);
    assert!(stats.memory_bytes > 0);
}
