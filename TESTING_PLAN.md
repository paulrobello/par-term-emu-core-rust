# Test Coverage Improvement Plan

## Priority 1: Sequence Handler Tests (High Impact, Low Effort)

### CSI Sequence Tests (`src/terminal/sequences/csi.rs`)
**Status**: 1,511 lines, 0 unit tests
**Recommended Tests**: 40-50 tests

#### What to Test (Using Public API Only):
- **Cursor Movement**: Verify `cursor.row` and `cursor.col` after CUU/CUD/CUF/CUB/CUP
- **SGR Attributes**: Test `flags`, `fg`, `bg`, `underline_color` state changes
- **Mode Changes**: Verify `auto_wrap`, `application_cursor`, `mouse_mode`, etc.
- **Scroll Regions**: Test `scroll_region_top`, `scroll_region_bottom`
- **Device Responses**: Verify `drain_responses()` output for DSR/DA commands
- **Tab Stops**: Check `tab_stops` vector state
- **Keyboard Protocol**: Test `keyboard_flags` and `keyboard_stack`

#### Example Test Pattern:
```rust
#[test]
fn test_cursor_movement_up() {
    let mut term = Terminal::new(80, 24);
    term.cursor.goto(10, 10);

    // Simulate CSI 5 A (move up 5 rows)
    term.write(b"\x1b[5A");

    assert_eq!(term.cursor.row, 5);
    assert_eq!(term.cursor.col, 10);
}
```

**Effort**: 4-6 hours
**Files to Create**: Add `#[cfg(test)] mod tests` to `src/terminal/sequences/csi.rs`

---

### OSC Sequence Tests (`src/terminal/sequences/osc.rs`)
**Status**: 465 lines, 0 unit tests
**Recommended Tests**: 25-30 tests

#### What to Test:
- **Window Title**: Verify `title` field changes (OSC 0/2)
- **Title Stack**: Test `title_stack` push/pop operations (OSC 21/22/23)
- **Color Parsing**: Unit test `parse_color_spec()` function directly
- **Default Colors**: Test `default_fg`, `default_bg`, `cursor_color` changes
- **ANSI Palette**: Verify `ansi_palette` modifications (OSC 4/104)
- **Hyperlinks**: Check `hyperlinks` HashMap and `current_hyperlink_id`
- **Clipboard**: Test `clipboard_content` (OSC 52)
- **Shell Integration**: Verify `shell_integration.marker()`, `exit_code()`, `cwd()`
- **Notifications**: Check `notifications` vector
- **Security**: Test `disable_insecure_sequences` blocking

#### Example Test Pattern:
```rust
#[test]
fn test_osc_set_title() {
    let mut term = Terminal::new(80, 24);

    term.write(b"\x1b]0;Test Title\x1b\\");

    assert_eq!(term.title, "Test Title");
}

#[test]
fn test_parse_color_spec() {
    assert_eq!(
        Terminal::parse_color_spec("rgb:FF/00/AA"),
        Some((255, 0, 170))
    );
    assert_eq!(
        Terminal::parse_color_spec("#123456"),
        Some((18, 52, 86))
    );
}
```

**Effort**: 3-4 hours
**Files to Create**: Add `#[cfg(test)] mod tests` to `src/terminal/sequences/osc.rs`

---

### ESC Sequence Tests (`src/terminal/sequences/esc.rs`)
**Status**: 129 lines, 0 unit tests
**Recommended Tests**: 8-12 tests

#### What to Test:
- **Cursor Save/Restore**: Verify `saved_cursor`, `saved_fg`, `saved_bg`, etc.
- **Tab Stops**: Test `tab_stops` after HTS (ESC H)
- **Terminal Reset**: Verify full state reset (ESC c)
- **Character Protection**: Test `char_protected` flag

#### Example Test Pattern:
```rust
#[test]
fn test_save_restore_cursor() {
    let mut term = Terminal::new(80, 24);
    term.cursor.goto(15, 10);
    term.fg = Color::Rgb(255, 0, 0);

    term.write(b"\x1b7");  // Save cursor
    term.cursor.goto(50, 20);
    term.write(b"\x1b8");  // Restore cursor

    assert_eq!(term.cursor.col, 15);
    assert_eq!(term.cursor.row, 10);
    assert_eq!(term.fg, Color::Rgb(255, 0, 0));
}
```

**Effort**: 1-2 hours
**Files to Create**: Add `#[cfg(test)] mod tests` to `src/terminal/sequences/esc.rs`

---

## Priority 2: Integration Tests for Grid Operations

Since Grid cells are private, create integration tests that verify behavior through the Terminal API:

### Grid Interaction Tests
**Location**: `src/tests/grid_integration_tests.rs` (new file)

```rust
#[test]
fn test_erase_in_display() {
    let mut term = Terminal::new(80, 24);

    // Fill screen with 'X'
    for _ in 0..24 {
        term.write(b"X".repeat(80).as_slice());
    }

    // Position cursor and erase
    term.write(b"\x1b[10;20H");  // Move to (20, 10)
    term.write(b"\x1b[J");       // Erase from cursor to end

    // Verify cursor position
    assert_eq!(term.cursor.row, 9);
    assert_eq!(term.cursor.col, 19);

    // Verify specific cells using public API
    assert_eq!(term.get_cell(19, 9).map(|c| c.c), Some(' '));
    assert_eq!(term.get_cell(0, 0).map(|c| c.c), Some('X'));
}
```

**Effort**: 3-4 hours for 15-20 tests

---

## Priority 3: Python Binding Tests

### PTY Binding Tests
**Location**: Add to `tests/test_pty.py`

Current coverage: 6 basic tests
Recommended: Add 20-30 more tests for:
- Session lifecycle edge cases
- Large I/O operations
- Environment variable handling
- Error conditions (invalid sizes, closed sessions)
- Thread safety scenarios

**Effort**: 4-6 hours

### Terminal Binding Tests
**Location**: `tests/test_terminal_bindings.py` (new file)

Test Python API for:
- Grid access methods
- Cursor position queries
- Title/mode getters
- Screenshot generation
- Cell attribute queries

**Effort**: 3-4 hours for 20-25 tests

---

## Priority 4: Screenshot Renderer Tests

**Location**: `src/screenshot/renderer.rs`

Add tests for:
- HTML generation with various cell attributes
- ANSI text format output
- Color rendering (RGB, 256-color, named)
- Unicode character handling
- Style attribute combinations

**Effort**: 4-5 hours for 20-25 tests

---

## Implementation Strategy

### Phase 1 (Week 1): Sequence Handlers
1. Add CSI tests (40-50 tests) - 6 hours
2. Add OSC tests (25-30 tests) - 4 hours
3. Add ESC tests (8-12 tests) - 2 hours
4. Run `make checkall` and fix issues - 2 hours

**Total**: ~14 hours, +73-92 tests

### Phase 2 (Week 2): Integration & Python
1. Grid integration tests (15-20 tests) - 4 hours
2. Python PTY tests (20-30 tests) - 5 hours
3. Python Terminal tests (20-25 tests) - 4 hours
4. Fix issues and verify - 2 hours

**Total**: ~15 hours, +55-75 tests

### Phase 3 (Week 3): Renderer & Polish
1. Screenshot renderer tests (20-25 tests) - 5 hours
2. Additional edge case coverage - 3 hours
3. Documentation and CI integration - 2 hours

**Total**: ~10 hours, +20-25 tests

---

## Expected Outcomes

### Coverage Improvement
- **Current**: ~44% of files have tests (20/45 files)
- **After Phase 1**: ~54% of files (+3 critical files)
- **After Phase 2**: ~60% of files (+2 test files)
- **After Phase 3**: ~62% of files (+1 file)

### Test Count
- **Current**: ~175 tests
- **After Phase 1**: ~248-267 tests (+42%)
- **After Phase 2**: ~303-342 tests (+74%)
- **After Phase 3**: ~323-367 tests (+110%)

### Lines of Tested Code
- **Current**: ~6,000 lines untested in critical paths
- **After all phases**: ~2,000-3,000 lines untested (50-67% reduction)

---

## Key Principles for Test Implementation

1. **Use Public APIs Only**: Don't access private fields like `grid.cells`
2. **Test State, Not Implementation**: Verify terminal state after sequences
3. **Integration Over Unit**: When private APIs block unit tests, use integration tests
4. **Security Testing**: Always test `disable_insecure_sequences` flag
5. **Error Cases**: Test invalid parameters, edge cases, boundary conditions
6. **Helper Functions**: Create test helpers for common operations

---

## Quick Wins (Immediate Action Items)

### Can Be Done Today (2-3 hours):
1. Add 10-15 OSC color parsing tests (pure functions, no dependencies)
2. Add 10-15 cursor movement CSI tests (simple state verification)
3. Add 5-8 ESC save/restore tests

### Example Implementation:

Create `src/terminal/sequences/csi_tests.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::terminal::Terminal;
    use crate::color::Color;

    #[test]
    fn test_cursor_up() {
        let mut term = Terminal::new(80, 24);
        term.write(b"\x1b[10;10H");  // Move to (10, 10)
        term.write(b"\x1b[5A");      // Move up 5
        assert_eq!(term.cursor.row, 4);
    }

    #[test]
    fn test_sgr_bold() {
        let mut term = Terminal::new(80, 24);
        term.write(b"\x1b[1m");  // Bold on
        assert!(term.flags.bold());
        term.write(b"\x1b[22m"); // Bold off
        assert!(!term.flags.bold());
    }

    #[test]
    fn test_sgr_foreground_color() {
        let mut term = Terminal::new(80, 24);
        term.write(b"\x1b[31m");  // Red foreground
        assert_eq!(term.fg, Color::Named(NamedColor::Red));
    }
}
```

Then run: `cargo test csi_tests`

---

## Conclusion

The test coverage gaps are significant but addressable through systematic implementation of the phases outlined above. The key is to:

1. Focus on public API testing
2. Use integration tests when unit tests are blocked by private fields
3. Prioritize high-impact areas (sequence handlers, Python bindings)
4. Maintain good test quality over quantity

**Estimated Total Effort**: 35-40 hours spread over 3 weeks
**Expected Outcome**: 110% increase in test coverage, 50-67% reduction in untested critical code
