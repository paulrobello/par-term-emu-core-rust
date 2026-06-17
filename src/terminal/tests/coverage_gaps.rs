//! Coverage gap-fillers for the `Terminal` god-object.
//!
//! These tests target PUBLIC methods on `Terminal` (in `src/terminal/mod.rs`)
//! that previously had no direct unit coverage. Each test exercises a method
//! with real branching logic (bounds checks, accumulation, reset, stack ops,
//! dirty-region folding, etc.) rather than trivial field getters.
//!
//! Scope rules followed (see task brief):
//! - Deterministic only — no timing, no PTY, no threads.
//! - Drive state with `Terminal::new(..)` / `Terminal::with_scrollback(..)` and
//!   `term.process(b"...")` then assert on query methods.
//! - No production code changes.

use crate::cell::Cell;
use crate::terminal::{cells_to_text, Terminal};

// =============================================================================
// Dirty-region tracking: mark_row_dirty / mark_clean / get_dirty_rows /
// get_dirty_region (mod.rs ~2710-2745)
// =============================================================================

#[test]
fn dirty_region_empty_when_nothing_marked() {
    let term = Terminal::new(80, 24);
    // Fresh terminal has no dirty rows: get_dirty_region must short-circuit to None.
    assert!(term.get_dirty_region().is_none());
    assert!(term.get_dirty_rows().is_empty());
}

#[test]
fn dirty_region_single_row_folds_to_itself() {
    let mut term = Terminal::new(80, 24);
    term.mark_row_dirty(5);

    let rows = term.get_dirty_rows();
    assert_eq!(rows, vec![5]);

    // Bounding box (first_row, 0, last_row, cols-1) for a single row.
    assert_eq!(term.get_dirty_region(), Some((5, 0, 5, 79)));
}

#[test]
fn dirty_region_bounding_box_takes_min_max_row() {
    let mut term = Terminal::new(40, 10);
    // Mark out of order — the fold takes min/max, not insertion order.
    term.mark_row_dirty(7);
    term.mark_row_dirty(1);
    term.mark_row_dirty(4);

    // get_dirty_rows must return sorted (uses sort_unstable).
    assert_eq!(term.get_dirty_rows(), vec![1, 4, 7]);
    assert_eq!(term.get_dirty_region(), Some((1, 0, 7, 39)));
}

#[test]
fn mark_clean_clears_dirty_rows_and_region() {
    let mut term = Terminal::new(80, 24);
    term.mark_row_dirty(2);
    term.mark_row_dirty(9);
    assert!(term.get_dirty_region().is_some());

    term.mark_clean();

    assert!(term.get_dirty_rows().is_empty());
    assert!(term.get_dirty_region().is_none());
}

// =============================================================================
// count_non_whitespace_lines (mod.rs ~2748)
// =============================================================================

#[test]
fn count_non_whitespace_lines_counts_rows_with_content() {
    let mut term = Terminal::new(20, 5);
    // Rows 0..3 get content, row 4 stays as spaces.
    term.process(b"aa\r\nbb\r\ncc\r\ndd\r\n      ");

    // Only the 4 rows with non-space content count.
    assert_eq!(term.count_non_whitespace_lines(), 4);
}

#[test]
fn count_non_whitespace_lines_zero_on_blank_screen() {
    let term = Terminal::new(20, 5);
    assert_eq!(term.count_non_whitespace_lines(), 0);
}

#[test]
fn count_non_whitespace_lines_uses_active_grid_on_alt_screen() {
    let mut term = Terminal::new(20, 3);
    term.process(b"primary content");
    term.process(b"\x1b[?1049h"); // enter alt screen (cleared)
                                  // On a freshly-entered alt screen there is no content.
    assert_eq!(term.count_non_whitespace_lines(), 0);

    term.process(b"alt content");
    assert_eq!(term.count_non_whitespace_lines(), 1);
}

// =============================================================================
// Programmatic tab-stop API: get_tab_stops / set_tab_stop / clear_tab_stop /
// clear_all_tab_stops (mod.rs ~2070-2099).
// (basic.rs only exercises these via VT sequences, not the direct API.)
// =============================================================================

#[test]
fn get_tab_stops_reports_every_8th_column_by_default() {
    let term = Terminal::new(40, 5);
    let stops = term.get_tab_stops();
    assert_eq!(stops, vec![0, 8, 16, 24, 32]);
}

#[test]
fn set_tab_stop_adds_position_and_is_idempotent() {
    let mut term = Terminal::new(40, 5);
    term.clear_all_tab_stops();

    term.set_tab_stop(3);
    term.set_tab_stop(3); // setting twice is idempotent
    term.set_tab_stop(11);

    assert_eq!(term.get_tab_stops(), vec![3, 11]);
}

#[test]
fn set_tab_stop_out_of_bounds_is_ignored() {
    let mut term = Terminal::new(10, 5);
    // Column index past tab_stops.len() must be a no-op (not a panic).
    term.set_tab_stop(99);
    // Only the default 8-column stops remain.
    assert_eq!(term.get_tab_stops(), vec![0, 8]);
}

#[test]
fn clear_tab_stop_removes_only_targeted_column() {
    let mut term = Terminal::new(24, 5);
    // Defaults: 0, 8, 16.
    term.clear_tab_stop(8);
    assert_eq!(term.get_tab_stops(), vec![0, 16]);
}

#[test]
fn clear_tab_stop_out_of_bounds_is_ignored() {
    let mut term = Terminal::new(24, 5);
    term.clear_tab_stop(500);
    assert_eq!(term.get_tab_stops(), vec![0, 8, 16]);
}

#[test]
fn clear_all_tab_stops_empties_every_position() {
    let mut term = Terminal::new(40, 5);
    term.clear_all_tab_stops();
    assert!(term.get_tab_stops().is_empty());
    // Setting a fresh stop afterwards still works.
    term.set_tab_stop(0);
    assert_eq!(term.get_tab_stops(), vec![0]);
}

// =============================================================================
// Title API + title stack (mod.rs ~1285, title_state/title_stack fields).
// The OSC sequence-driven push/pop is parsed elsewhere, but the plain setter
// + getter + RIS reset interaction is not directly tested.
// =============================================================================

#[test]
fn set_title_updates_title_getter() {
    let mut term = Terminal::new(80, 24);
    assert_eq!(term.title(), "");

    term.set_title("hello world".to_string());
    assert_eq!(term.title(), "hello world");

    // Overwriting replaces the previous title.
    term.set_title("second".to_string());
    assert_eq!(term.title(), "second");
}

#[test]
fn reset_clears_title_back_to_default() {
    let mut term = Terminal::new(80, 24);
    term.set_title("transient".to_string());
    term.process(b"\x1bc"); // RIS — full reset

    // RIS reconstructs the terminal via Self::with_scrollback; title is empty.
    assert_eq!(term.title(), "");
}

// =============================================================================
// bell_count (mod.rs ~2298)
// =============================================================================

#[test]
fn bell_count_increments_per_bel_char() {
    let mut term = Terminal::new(80, 24);
    assert_eq!(term.bell_count(), 0);

    term.process(b"a\x07b\x07\x07");
    assert_eq!(term.bell_count(), 3);
}

#[test]
fn bell_count_survives_reset_only_if_reset_recreates_progress_state() {
    // RIS recreates the terminal, so the counter returns to 0.
    let mut term = Terminal::new(80, 24);
    term.process(b"\x07\x07");
    assert_eq!(term.bell_count(), 2);
    term.process(b"\x1bc");
    assert_eq!(term.bell_count(), 0);
}

// =============================================================================
// scrollback() accessor + scrollback accumulation (mod.rs ~1140, grid/mod.rs).
// =============================================================================

#[test]
fn scrollback_returns_lines_in_oldest_first_order() {
    let mut term = Terminal::with_scrollback(10, 2, 100);
    // Push enough lines to force at least 2 into scrollback.
    term.process(b"first\r\n");
    term.process(b"second\r\n");
    term.process(b"third"); // stays on the visible screen

    let sb = term.scrollback();
    // Order documented by scrollback(): index 0 is the oldest line.
    assert!(sb.iter().any(|l| l.starts_with("first")));
    // The currently-visible line ("third") must not appear in scrollback.
    assert!(!sb.iter().any(|l| l.starts_with("third")));
    // Oldest-first ordering: "first" should come before "second" if both present.
    if let (Some(fi), Some(si)) = (
        sb.iter().position(|l| l.starts_with("first")),
        sb.iter().position(|l| l.starts_with("second")),
    ) {
        assert!(fi < si);
    }
}

#[test]
fn scrollback_empty_on_fresh_terminal() {
    let term = Terminal::new(80, 24);
    assert!(term.scrollback().is_empty());
}

// =============================================================================
// get_row_range (mod.rs ~3014) — end-exclusive, out-of-bounds tolerated.
// =============================================================================

#[test]
fn get_row_range_returns_end_exclusive_slice() {
    let mut term = Terminal::new(6, 5);
    term.process(b"AAAAAA\r\nBBBBBB\r\nCCCCCC\r\nDDDDDD\r\nEEEEEE");

    // Range [1, 3) -> rows 1 and 2.
    let rows = term.get_row_range(1, 3);
    assert_eq!(rows.len(), 2);
    assert_eq!(cells_to_text(&rows[0]), "BBBBBB");
    assert_eq!(cells_to_text(&rows[1]), "CCCCCC");
}

#[test]
fn get_row_range_out_of_bounds_rows_are_omitted_not_panicked() {
    let term = Terminal::new(6, 3);
    // Request rows beyond the grid — only the in-bounds rows are returned.
    let rows = term.get_row_range(0, 99);
    assert_eq!(rows.len(), 3);
}

#[test]
fn get_row_range_empty_when_start_at_end() {
    let term = Terminal::new(6, 3);
    let rows = term.get_row_range(2, 2);
    assert!(rows.is_empty());
}

// =============================================================================
// get_visible_region (mod.rs ~3008)
// =============================================================================

#[test]
fn get_visible_region_matches_screen_dimensions() {
    let term = Terminal::new(80, 24);
    assert_eq!(term.get_visible_region(), (0, 0, 23, 79));
}

#[test]
fn get_visible_region_follows_active_screen_after_alt_switch() {
    // Resize the alt screen indirectly isn't possible (both grids share size on
    // Terminal::new), but size() delegates to the active grid, so verify the
    // region reflects the current dimensions regardless of which screen is up.
    let mut term = Terminal::new(40, 10);
    term.process(b"\x1b[?1049h"); // enter alt screen
    assert_eq!(term.get_visible_region(), (0, 0, 9, 39));
}

// =============================================================================
// Programmatic rectangle ops: fill_rectangle / erase_rectangle / get_rectangle
// / calculate_rectangle_checksum (mod.rs ~2926-3005).
// ffi_tests covers checksum via Python only; the Rust API needs direct tests.
// =============================================================================

#[test]
fn fill_rectangle_writes_char_into_targeted_region() {
    let mut term = Terminal::new(10, 5);
    term.fill_rectangle(1, 1, 3, 4, 'X');

    let region = term.get_rectangle(1, 1, 3, 4);
    // Every cell in the rectangle should be 'X'.
    for row in &region {
        for cell in row {
            assert_eq!(cell.c, 'X');
        }
    }
    assert_eq!(region.len(), 3); // rows 1..=3
    assert_eq!(region[0].len(), 4); // cols 1..=4
}

#[test]
fn fill_rectangle_does_not_touch_cells_outside_region() {
    let mut term = Terminal::new(10, 3);
    term.fill_rectangle(0, 0, 0, 0, 'X');

    // (1,0) should remain a space.
    let cell = term.active_grid().get(1, 0).unwrap();
    assert_eq!(cell.c, ' ');
}

#[test]
fn fill_rectangle_marks_targeted_rows_dirty() {
    let mut term = Terminal::new(10, 5);
    term.fill_rectangle(1, 1, 3, 4, 'X');

    // Rows 1, 2, 3 were marked dirty; row 0 and 4 were not.
    let dirty = term.get_dirty_rows();
    assert_eq!(dirty, vec![1, 2, 3]);
}

#[test]
fn erase_rectangle_clears_cells_to_spaces() {
    let mut term = Terminal::new(10, 3);
    term.process(b"AAAAAAAAAA\r\nBBBBBBBBBB\r\nCCCCCCCCCC");
    term.erase_rectangle(0, 0, 1, 4);

    // Rows 0 and 1 cols 0..=4 should now be spaces.
    let region = term.get_rectangle(0, 0, 1, 4);
    for row in &region {
        for cell in row {
            assert_eq!(cell.c, ' ', "expected space, got {:?}", cell.c);
        }
    }
}

#[test]
fn calculate_rectangle_checksum_sums_char_codes_masked_to_u16() {
    let mut term = Terminal::new(10, 2);
    // Fill row 0 cols 0..=2 with 'A' (65) each => 3*65 = 195.
    term.fill_rectangle(0, 0, 0, 2, 'A');
    assert_eq!(term.calculate_rectangle_checksum(0, 0, 0, 2), 195);
}

#[test]
fn calculate_rectangle_checksum_wraps_at_16_bits() {
    // Use cells whose raw code sum exceeds 16 bits, then verify the result is
    // the wrapping_add truncated to u16. '\u{FFFF}' = 65535, ten of them would
    // be 655350 raw; mod 0x10000 == 655350 - 10*0x10000 == 655350 - 655360 == -10
    // == 65526 (wrapping).
    let mut term = Terminal::new(10, 1);
    term.fill_rectangle(0, 0, 0, 9, '\u{FFFF}');
    let checksum = term.calculate_rectangle_checksum(0, 0, 0, 9);

    let expected = (10u32 * 0xFFFF) & 0xFFFF;
    assert_eq!(checksum, expected as u16);
    assert_eq!(expected as u16, 65526);
}

#[test]
fn calculate_rectangle_checksum_empty_region_is_zero() {
    let term = Terminal::new(10, 3);
    // Single-space cell has code 0x20, but a totally-empty (out-of-bounds)
    // region contributes nothing.
    assert_eq!(term.calculate_rectangle_checksum(100, 100, 100, 100), 0);
}

// =============================================================================
// get_ansi_color (mod.rs ~1578) — bounds behavior returns None out of range.
// =============================================================================

#[test]
fn get_ansi_color_in_range_returns_some() {
    let term = Terminal::new(80, 24);
    // Standard 16-color palette indices 0..16 are always populated.
    assert!(term.get_ansi_color(0).is_some());
    assert!(term.get_ansi_color(15).is_some());
}

#[test]
fn get_ansi_color_out_of_range_returns_none() {
    let term = Terminal::new(80, 24);
    assert!(term.get_ansi_color(usize::MAX).is_none());
}

// =============================================================================
// save_cursor / restore_cursor (programmatic API, mod.rs ~1504).
// cursor.rs covers the ESC 7 / ESC 8 path; this hits the direct methods.
// =============================================================================

#[test]
fn save_then_restore_cursor_round_trips_position_and_attrs() {
    use crate::color::{Color, NamedColor};

    let mut term = Terminal::new(80, 24);
    term.cursor.goto(3, 7);
    term.fg = Color::Named(NamedColor::Red);

    term.save_cursor();

    // Mutate state after saving.
    term.cursor.goto(0, 0);
    term.fg = Color::default();

    term.restore_cursor();

    assert_eq!(term.cursor.col, 3);
    assert_eq!(term.cursor.row, 7);
    assert_eq!(term.fg, Color::Named(NamedColor::Red));
}

#[test]
fn restore_cursor_without_prior_save_is_a_noop() {
    let mut term = Terminal::new(80, 24);
    term.cursor.goto(4, 4);
    // No save_cursor() call — restore must leave state untouched.
    term.restore_cursor();
    assert_eq!(term.cursor.col, 4);
    assert_eq!(term.cursor.row, 4);
}

// =============================================================================
// cell_dimensions / set_cell_dimensions (mod.rs ~1809) — clamps to >= 1.
// =============================================================================

#[test]
fn set_cell_dimensions_clamps_zero_to_one() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(0, 0);
    let (w, h) = term.cell_dimensions();
    assert_eq!((w, h), (1, 1));
}

#[test]
fn set_cell_dimensions_round_trips_positive_values() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(9, 18);
    assert_eq!(term.cell_dimensions(), (9, 18));
}

// =============================================================================
// size() delegates to ACTIVE grid (mod.rs ~1182).
// =============================================================================

#[test]
fn size_reflects_active_screen_after_resize() {
    let mut term = Terminal::new(80, 24);
    assert_eq!(term.size(), (80, 24));

    term.resize(100, 30);
    assert_eq!(term.size(), (100, 30));

    // size() must agree on the alt screen too.
    term.process(b"\x1b[?1049h");
    assert_eq!(term.size(), (100, 30));
}

// =============================================================================
// reset() (RIS, mod.rs ~2697) — preserves tab stops, clears runtime state.
// =============================================================================

#[test]
fn reset_preserves_tab_stops_but_clears_content() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5G\x1bH"); // HTS at column 5 (0-indexed 4)
    term.process(b"Some content here");

    // Sanity: stop exists and screen has content.
    assert!(term.get_tab_stops().contains(&4));
    assert!(term.count_non_whitespace_lines() > 0);

    term.process(b"\x1bc"); // RIS

    // Tab stop survives reset...
    assert!(
        term.get_tab_stops().contains(&4),
        "RIS must preserve tab stops"
    );
    // ...but content is cleared.
    assert_eq!(term.count_non_whitespace_lines(), 0);
    // Cursor returns to home.
    assert_eq!(term.cursor.col, 0);
    assert_eq!(term.cursor.row, 0);
}

#[test]
fn reset_clears_alt_screen_state() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());

    term.process(b"\x1bc");
    assert!(!term.is_alt_screen_active());
}

// =============================================================================
// get_rectangle + Cell round-trip via Cell::new construction (sanity).
// =============================================================================

#[test]
fn get_rectangle_returns_cells_cloned_not_references() {
    let mut term = Terminal::new(5, 2);
    term.fill_rectangle(0, 0, 1, 4, 'Q');

    let before = term.get_rectangle(0, 0, 1, 4);
    // Mutate the returned clone; the grid must be unaffected.
    let mut mutated = before.clone();
    mutated[0][0] = Cell::new('Z');

    let after = term.get_rectangle(0, 0, 1, 4);
    assert_eq!(after[0][0].c, 'Q');
    assert_eq!(mutated[0][0].c, 'Z');
}
