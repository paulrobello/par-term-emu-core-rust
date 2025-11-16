# Handoff Document

## Project Status

**Current Version**: 0.2.2
**Last Commit**: ad02ad1 - "feat: add title() method to PtyTerminal Python bindings"
**Branch**: main (up to date with origin)

## Completed Work

### Issue #2 - Add title() method to PtyTerminal ✅
Successfully implemented and merged:
- Added `title()` method to `PyPtyTerminal` class in `src/python_bindings/pty.rs` (lines 211-225)
- Added test case in `tests/test_pty.py` (lines 263-283)
- Bumped version to 0.2.2
- Issue automatically closed via commit message

**Quality**: All tests passing (267 Rust, 220 Python)

## Outstanding Work

### Task: Add Missing Methods to PtyTerminal

**Priority**: HIGH
**Estimated Effort**: 2-3 hours

#### Context
During the title() implementation, we discovered several useful methods that exist in `Terminal` but are missing from `PtyTerminal`. These methods would improve the TUI project's capabilities.

#### Methods to Add (in priority order):

### 1. HIGH PRIORITY - `get_hyperlink()` ✅ CRITICAL
```rust
/// Get hyperlink URL at the specified position (OSC 8)
///
/// Args:
///     col: Column position (0-based)
///     row: Row position (0-based)
///
/// Returns:
///     URL string if a hyperlink exists at that position, None otherwise
fn get_hyperlink(&self, col: usize, row: usize) -> PyResult<Option<String>> {
    let terminal = self.inner.terminal();
    let result = if let Ok(term) = terminal.lock() {
        if let Some(cell) = term.active_grid().get(col, row) {
            if let Some(id) = cell.flags.hyperlink_id {
                term.get_hyperlink_url(id)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    Ok(result)
}
```

**Use Case**: TUI can detect clickable URLs, show tooltips, open links on click
**Location**: Add after `get_attributes()` method around line 625 in `src/python_bindings/pty.rs`
**Reference**: See `src/python_bindings/terminal.rs:877-884`

### 2. MEDIUM PRIORITY - `flush_synchronized_updates()`
```rust
/// Flush synchronized updates (DEC 2026)
///
/// When synchronized update mode is active (CSI ? 2026 h), this flushes
/// all pending updates atomically for flicker-free rendering.
fn flush_synchronized_updates(&mut self) -> PyResult<()> {
    let terminal = self.inner.terminal();
    if let Ok(mut term) = terminal.lock() {
        term.flush_synchronized_updates();
    }
    Ok(())
}
```

**Use Case**: TUI can batch screen updates for smooth, flicker-free rendering
**Location**: Add near `synchronized_updates()` method around line 1385
**Reference**: See `src/python_bindings/terminal.rs:1061-1064`

### 3. MEDIUM PRIORITY - Focus Event Methods
```rust
/// Get focus in event sequence
///
/// Returns the escape sequence to send when terminal gains focus.
/// Only relevant when focus tracking is enabled (CSI ? 1004 h).
///
/// Returns:
///     Bytes for focus in event: b'\x1b[I'
fn get_focus_in_event(&self) -> PyResult<Vec<u8>> {
    let terminal = self.inner.terminal();
    let event = if let Ok(term) = terminal.lock() {
        term.report_focus_in()
    } else {
        Vec::new()
    };
    Ok(event)
}

/// Get focus out event sequence
///
/// Returns the escape sequence to send when terminal loses focus.
/// Only relevant when focus tracking is enabled (CSI ? 1004 h).
///
/// Returns:
///     Bytes for focus out event: b'\x1b[O'
fn get_focus_out_event(&self) -> PyResult<Vec<u8>> {
    let terminal = self.inner.terminal();
    let event = if let Ok(term) = terminal.lock() {
        term.report_focus_out()
    } else {
        Vec::new()
    };
    Ok(event)
}
```

**Use Case**: TUI sends focus events to child process when window focus changes
**Location**: Add after `focus_tracking()` method around line 1298
**Reference**: See `src/python_bindings/terminal.rs:1088-1101`

### 4. LOW PRIORITY - `simulate_mouse_event()` (Optional)
```rust
/// Simulate a mouse event and get the escape sequence
///
/// Generates the appropriate mouse event sequence based on current mouse mode.
///
/// Args:
///     button: Mouse button (0=left, 1=middle, 2=right)
///     col: Column position (0-based)
///     row: Row position (0-based)
///     pressed: True for press, False for release
///
/// Returns:
///     Bytes representing the mouse event sequence
fn simulate_mouse_event(
    &mut self,
    button: u8,
    col: usize,
    row: usize,
    pressed: bool,
) -> PyResult<Vec<u8>> {
    use crate::mouse::MouseEvent;
    let terminal = self.inner.terminal();
    let event_bytes = if let Ok(term) = terminal.lock() {
        let event = MouseEvent::new(button, col, row, pressed, 0);
        term.report_mouse(event)
    } else {
        Vec::new()
    };
    Ok(event_bytes)
}
```

**Use Case**: Testing, automation, accessibility features
**Location**: Add after `mouse_mode()` method around line 802
**Reference**: See `src/python_bindings/terminal.rs:1076-1086`

## Implementation Checklist

For each method added:
- [ ] Add method to `PyPtyTerminal` impl in `src/python_bindings/pty.rs`
- [ ] Add test case in `tests/test_pty.py`
- [ ] Run `make checkall` to verify all tests pass
- [ ] Update README.md if needed (most methods inherit from Terminal docs)
- [ ] Commit with descriptive message
- [ ] Bump patch version (0.2.2 → 0.2.3)

## Testing Strategy

Since PTY tests are skipped in CI, use this test pattern (see `test_pty_terminal_title` for reference):

```python
def test_pty_terminal_hyperlink():
    """Test getting hyperlink from PtyTerminal"""
    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference)
    term_regular = Terminal(80, 24)
    term_regular.process_str("\x1b]8;;https://example.com\x07Click\x1b]8;;\x07")
    assert term_regular.get_hyperlink(0, 0) == "https://example.com"

    # Verify PtyTerminal has the same API
    pty_term = PtyTerminal(80, 24)
    assert hasattr(pty_term, 'get_hyperlink'), "PtyTerminal should have get_hyperlink() method"
    assert pty_term.get_hyperlink(0, 0) is None  # Empty initially
```

## Documentation References

- **Terminal API**: `src/python_bindings/terminal.rs` - Reference implementation
- **PtyTerminal API**: `src/python_bindings/pty.rs` - Target file
- **Architecture**: `docs/ARCHITECTURE.md`
- **Advanced Features**: `docs/ADVANCED_FEATURES.md`
- **Development Guide**: `CLAUDE.md`

## Important Notes

### Methods NOT to Add
- ❌ `process()` / `process_str()` - These would bypass the PTY and cause desync with the child process
- ❌ `reset()` - Would reset terminal state without the child process knowing

### Thread Safety Pattern
All PtyTerminal methods that access the terminal must:
1. Get the terminal reference: `let terminal = self.inner.terminal();`
2. Lock the mutex: `if let Ok(term) = terminal.lock()`
3. Access terminal methods
4. Return default/empty value if lock fails

### Version Bumping
Update `pyproject.toml` line 9: `version = "0.2.3"`

### Git Workflow
```bash
make checkall           # Run ALL checks before committing
git add -A
git commit -m "feat: add hyperlink and focus methods to PtyTerminal"
git push origin main    # Only push when user requests it
```

## Known Issues
None currently.

## Next Steps

1. **Start with `get_hyperlink()`** - This is the most valuable method for the TUI project
2. Add `flush_synchronized_updates()` for smooth rendering
3. Add focus event methods for complete terminal emulation
4. Consider `simulate_mouse_event()` if time permits
5. Bump version to 0.2.3
6. Run full test suite
7. Commit and push when all tests pass

## Sister Project Context

The TUI project (par-term-emu-tui-rust) is waiting for these methods:
- Hyperlinks for clickable URLs in the terminal widget
- Focus events for proper window focus handling
- Synchronized updates for smooth rendering

These additions will enable full feature parity between Terminal and PtyTerminal for read-only query operations.

## Questions?

Refer to:
- Issue #2 (closed) for the title() implementation as a reference
- `src/python_bindings/terminal.rs` for exact method signatures
- `CLAUDE.md` for development workflow guidelines
