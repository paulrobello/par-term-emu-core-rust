# OSC 9;4 Progress Bar Protocol - Implementation Guide

**Research Date**: 2026-02-09
**Last Updated**: 2026-02-09
**Project**: par-term-emu-core-rust
**Purpose**: Parser implementation reference

**Related Research**: `/Users/probello/Repos/research/terminal-emulators/osc-9-4-progress-bars-2026-02-09.md`

## Critical Note: OSC 9;4 vs OSC 934

**IMPORTANT**: The correct sequence is `OSC 9;4` (NOT `OSC 934`).

- Correct: `ESC ] 9 ; 4 ; [state] ; [progress] ST`
- Incorrect: `ESC ] 934 ; [state] ; [progress] ST`

The confusion arises because the semicolons are part of the OSC parameter syntax, not the sequence number itself.

## Protocol Summary

OSC 9;4 is a terminal escape sequence for displaying progress bars. Originally created by ConEmu for Windows, it has been adopted by:
- iTerm2 (macOS)
- Ghostty (cross-platform)
- Windows Terminal
- WezTerm (cross-platform)
- Konsole (KDE)
- mintty (Cygwin/MSYS2/WSL)
- xterm.js (web-based terminals)

## Escape Sequence Format

### Full Syntax

```
ESC ] 9 ; 4 ; [state] ; [progress] ST
```

Where:
- `ESC` = `\x1b` (escape character)
- `]` = Introduces OSC sequence
- `9` = OSC 9 sequence family (ConEmu-specific)
- `;` = Parameter separator
- `4` = Progress bar sub-command
- `[state]` = Single digit 0-4 (required)
- `[progress]` = Integer 0-100 (optional, depends on state)
- `ST` = String Terminator: either `BEL` (`\x07`) or `ESC \` (`\x1b\x5c`)

### Abbreviated Form

To clear the progress bar:

```
ESC ] 9 ; 4 ST
```

This is equivalent to `ESC ] 9 ; 4 ; 0 ST` (remove/clear state).

## State Values

| State | Name          | Description                                      | Progress Required | Progress Optional |
|-------|---------------|--------------------------------------------------|-------------------|-------------------|
| 0     | Remove/Clear  | Removes/hides the progress bar                   | No                | Ignored           |
| 1     | Set/Normal    | Normal/success state (typically green)           | **Yes**           | No                |
| 2     | Error         | Error state (typically red)                      | No                | Yes (indeterminate if omitted) |
| 3     | Indeterminate | Indeterminate/pulsing/animated state             | No                | Ignored           |
| 4     | Warning/Pause | Warning/paused state (typically yellow/orange)   | **Yes**           | No                |

### State Details

#### State 0: Remove/Clear
- **Purpose**: Hides and removes the progress bar
- **Progress**: Not required, ignored if provided
- **Use case**: Operation completed, cancelled, or no longer needs display
- **Example**: `ESC ] 9 ; 4 ; 0 \x07` or `ESC ] 9 ; 4 \x07`

#### State 1: Set/Normal (Success)
- **Purpose**: Display normal/success progress
- **Progress**: **REQUIRED** (0-100)
- **Visual**: Typically rendered in green or terminal's success color
- **Use case**: Normal ongoing operation with known progress
- **Example**: `ESC ] 9 ; 4 ; 1 ; 50 \x07` (50% progress)
- **Error handling**: If progress missing, parser should reject or default to 0

#### State 2: Error
- **Purpose**: Display error state
- **Progress**: **OPTIONAL**
  - If provided: Shows error at specific percentage (e.g., failed at 75%)
  - If omitted: Shows indeterminate error state (animated/pulsing red bar)
- **Visual**: Typically rendered in red
- **Use case**: Operation failed or encountered errors
- **Examples**:
  - `ESC ] 9 ; 4 ; 2 ; 75 \x07` (error at 75%)
  - `ESC ] 9 ; 4 ; 2 \x07` (indeterminate error)

#### State 3: Indeterminate
- **Purpose**: Display animated/pulsing progress indicator
- **Progress**: Not required, ignored if provided
- **Visual**: Animated horizontal bar (no percentage shown)
- **Use case**: Operation in progress but duration/completion unknown
- **Example**: `ESC ] 9 ; 4 ; 3 \x07`

#### State 4: Warning/Pause
- **Purpose**: Display warning or paused state
- **Progress**: **REQUIRED** (0-100)
- **Visual**: Typically rendered in yellow/orange
- **Use case**: Paused operations or operations with warnings
- **Example**: `ESC ] 9 ; 4 ; 4 ; 25 \x07` (paused at 25%)
- **Error handling**: If progress missing, parser should reject or default to 0

## Progress Value Specification

### Valid Range
- **Type**: Integer (decimal representation)
- **Range**: 0 to 100 (inclusive)
- **Meaning**: Percentage of completion (0% to 100%)

### Value Clamping
Terminal implementations **MUST** clamp values outside the valid range:
- Values < 0: Clamp to 0
- Values > 100: Clamp to 100
- Decimal/float values: Parse integer portion only

### Required vs Optional
| State | Progress Requirement                                    |
|-------|---------------------------------------------------------|
| 0     | Ignored (not needed)                                    |
| 1     | **Required** - sequence invalid without it              |
| 2     | Optional - creates indeterminate error state if omitted |
| 3     | Ignored (not needed)                                    |
| 4     | **Required** - sequence invalid without it              |

## Parser Implementation

### Parsing State Machine

```rust
enum ProgressState {
    Remove,        // 0
    Normal,        // 1
    Error,         // 2
    Indeterminate, // 3
    Warning,       // 4
}

struct ProgressBar {
    state: ProgressState,
    progress: Option<u8>,  // 0-100, None for indeterminate
}
```

### Parsing Algorithm

1. **Detect Sequence Start**: Watch for `ESC ] 9 ; 4`
2. **Parse State Parameter**:
   - Next parameter (after `4`) is the state (0-4)
   - If missing or not followed by `;`, treat as state 0 (remove)
   - If invalid (not 0-4), reject entire sequence
3. **Parse Progress Parameter**:
   - Next parameter (after state) is progress (if present)
   - Parse as decimal integer
   - Clamp to 0-100 range
   - Validate against state requirements
4. **Validate Terminator**: Accept either BEL (`\x07`) or `ESC \` (`\x1b\x5c`)

### Pseudocode

```rust
fn parse_osc_9_4(params: &[&str]) -> Result<ProgressBar, ParseError> {
    // First param after "9;4" is state
    let state = match params.get(0) {
        Some(&"0") | None => ProgressState::Remove,
        Some(&"1") => ProgressState::Normal,
        Some(&"2") => ProgressState::Error,
        Some(&"3") => ProgressState::Indeterminate,
        Some(&"4") => ProgressState::Warning,
        _ => return Err(ParseError::InvalidState),
    };

    // Second param is progress (if present)
    let progress = params.get(1)
        .and_then(|p| p.parse::<i32>().ok())
        .map(|p| p.clamp(0, 100) as u8);

    // Validate required progress for certain states
    match (state, progress) {
        (ProgressState::Normal, None) => return Err(ParseError::MissingProgress),
        (ProgressState::Warning, None) => return Err(ParseError::MissingProgress),
        _ => {}
    }

    Ok(ProgressBar { state, progress })
}
```

### Edge Cases

| Case | Behavior |
|------|----------|
| `ESC ] 9 ; 4 ST` | Valid: Remove progress bar (state 0) |
| `ESC ] 9 ; 4 ; 0 ST` | Valid: Remove progress bar (explicit state 0) |
| `ESC ] 9 ; 4 ; 1 ST` | **Invalid**: State 1 requires progress parameter |
| `ESC ] 9 ; 4 ; 1 ; 150 ST` | Valid: Clamp 150 to 100 |
| `ESC ] 9 ; 4 ; 1 ; -10 ST` | Valid: Clamp -10 to 0 |
| `ESC ] 9 ; 4 ; 2 ST` | Valid: Indeterminate error state |
| `ESC ] 9 ; 4 ; 2 ; 50 ST` | Valid: Error at 50% |
| `ESC ] 9 ; 4 ; 3 ; 50 ST` | Valid: Indeterminate state (50 ignored) |
| `ESC ] 9 ; 4 ; 5 ST` | **Invalid**: State 5 does not exist |
| `ESC ] 9 ; 4 ; 1 ; abc ST` | **Invalid**: Progress must be numeric |

### Conflict with OSC 9 Notifications

**CRITICAL**: OSC 9 without `;4` is a different sequence (desktop notification).

- `ESC ] 9 ; 4 ; ...` → Progress bar (this protocol)
- `ESC ] 9 ; [message] ST` → Desktop notification (different protocol)

Parser MUST distinguish between these by checking for `;4` immediately after `9`.

```rust
// Parser state machine
if osc_params[0] == "9" {
    if osc_params.get(1) == Some(&"4") {
        // Progress bar: ESC ] 9 ; 4 ; state ; progress ST
        parse_progress_bar(&osc_params[2..])
    } else {
        // Desktop notification: ESC ] 9 ; message ST
        parse_desktop_notification(&osc_params[1..])
    }
}
```

## Example Sequences (Bash)

```bash
# Show indeterminate progress (start of unknown-duration task)
echo -e "\033]9;4;3\a"

# Update to 25% normal progress
echo -e "\033]9;4;1;25\a"

# Update to 50%
echo -e "\033]9;4;1;50\a"

# Update to 75%
echo -e "\033]9;4;1;75\a"

# Show error at current progress
echo -e "\033]9;4;2;75\a"

# Clear progress bar
echo -e "\033]9;4;0\a"
# or
echo -e "\033]9;4\a"
```

## Example Sequences (Python)

```python
import sys

def show_progress(percent: int) -> None:
    """Show normal progress at given percentage (0-100)."""
    sys.stdout.write(f"\033]9;4;1;{percent}\007")
    sys.stdout.flush()

def show_indeterminate() -> None:
    """Show indeterminate progress (duration unknown)."""
    sys.stdout.write("\033]9;4;3\007")
    sys.stdout.flush()

def show_error(percent: int | None = None) -> None:
    """Show error state, optionally at specific percentage."""
    if percent is not None:
        sys.stdout.write(f"\033]9;4;2;{percent}\007")
    else:
        sys.stdout.write("\033]9;4;2\007")
    sys.stdout.flush()

def show_warning(percent: int) -> None:
    """Show warning/pause state at given percentage."""
    sys.stdout.write(f"\033]9;4;4;{percent}\007")
    sys.stdout.flush()

def clear_progress() -> None:
    """Clear/remove the progress bar."""
    sys.stdout.write("\033]9;4;0\007")
    sys.stdout.flush()
```

## Example Sequences (Rust)

```rust
use std::io::{self, Write};

pub fn show_progress(percent: u8) -> io::Result<()> {
    print!("\x1b]9;4;1;{}\x07", percent.min(100));
    io::stdout().flush()
}

pub fn show_indeterminate() -> io::Result<()> {
    print!("\x1b]9;4;3\x07");
    io::stdout().flush()
}

pub fn show_error(percent: Option<u8>) -> io::Result<()> {
    match percent {
        Some(p) => print!("\x1b]9;4;2;{}\x07", p.min(100)),
        None => print!("\x1b]9;4;2\x07"),
    }
    io::stdout().flush()
}

pub fn show_warning(percent: u8) -> io::Result<()> {
    print!("\x1b]9;4;4;{}\x07", percent.min(100));
    io::stdout().flush()
}

pub fn clear_progress() -> io::Result<()> {
    print!("\x1b]9;4;0\x07");
    io::stdout().flush()
}
```

## Integration with Existing OSC Parser

### Recommended Approach

The OSC 9;4 sequence should be parsed in the existing OSC handler, likely in `src/terminal/sequences/osc.rs`:

```rust
pub fn handle_osc(&mut self, params: &[&str]) -> Result<(), Error> {
    if params.is_empty() {
        return Ok(());
    }

    match params[0] {
        "0" | "1" | "2" => self.handle_title(params),
        "4" => self.handle_color_query(params),
        "8" => self.handle_hyperlink(params),
        "9" => {
            // ConEmu-specific sequences
            if params.get(1) == Some(&"4") {
                // Progress bar: OSC 9 ; 4 ; state ; progress
                self.handle_progress_bar(&params[2..])
            } else {
                // Desktop notification: OSC 9 ; message
                self.handle_desktop_notification(&params[1..])
            }
        }
        "52" => self.handle_clipboard(params),
        "133" => self.handle_shell_integration(params),
        "1337" => self.handle_iterm2(params),
        _ => Ok(()), // Ignore unknown sequences
    }
}

fn handle_progress_bar(&mut self, params: &[&str]) -> Result<(), Error> {
    let state = match params.get(0) {
        Some(&"0") | None => ProgressState::Remove,
        Some(&"1") => ProgressState::Normal,
        Some(&"2") => ProgressState::Error,
        Some(&"3") => ProgressState::Indeterminate,
        Some(&"4") => ProgressState::Warning,
        _ => return Ok(()), // Invalid state, ignore
    };

    let progress = params.get(1)
        .and_then(|p| p.parse::<i32>().ok())
        .map(|p| p.clamp(0, 100) as u8);

    // Validate required progress
    match (state, progress) {
        (ProgressState::Normal, None) => return Ok(()), // Invalid, ignore
        (ProgressState::Warning, None) => return Ok(()), // Invalid, ignore
        _ => {}
    }

    // Emit event or update terminal state
    self.emit_progress_event(state, progress);
    Ok(())
}
```

### Data Structures

Add to `src/terminal/mod.rs` or create `src/terminal/progress.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Remove,
    Normal,
    Error,
    Indeterminate,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressBar {
    pub state: ProgressState,
    pub progress: Option<u8>, // 0-100, None for indeterminate states
}

impl ProgressBar {
    pub fn remove() -> Self {
        Self {
            state: ProgressState::Remove,
            progress: None,
        }
    }

    pub fn normal(percent: u8) -> Self {
        Self {
            state: ProgressState::Normal,
            progress: Some(percent.min(100)),
        }
    }

    pub fn error(percent: Option<u8>) -> Self {
        Self {
            state: ProgressState::Error,
            progress: percent.map(|p| p.min(100)),
        }
    }

    pub fn indeterminate() -> Self {
        Self {
            state: ProgressState::Indeterminate,
            progress: None,
        }
    }

    pub fn warning(percent: u8) -> Self {
        Self {
            state: ProgressState::Warning,
            progress: Some(percent.min(100)),
        }
    }

    /// Returns true if this is an indeterminate state (no percentage)
    pub fn is_indeterminate(&self) -> bool {
        matches!(self.state, ProgressState::Indeterminate)
            || (matches!(self.state, ProgressState::Error) && self.progress.is_none())
    }
}
```

### Event Emission

Add to terminal events (in streaming protocol or direct events):

```rust
pub enum TerminalEvent {
    // ... existing events ...
    ProgressBar(ProgressBar),
}
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remove() {
        // Abbreviated form
        assert_eq!(parse_osc_9_4(&[]).unwrap(), ProgressBar::remove());
        // Explicit form
        assert_eq!(parse_osc_9_4(&["0"]).unwrap(), ProgressBar::remove());
    }

    #[test]
    fn test_parse_normal() {
        assert_eq!(parse_osc_9_4(&["1", "50"]).unwrap(), ProgressBar::normal(50));
        // Missing progress should error
        assert!(parse_osc_9_4(&["1"]).is_err());
    }

    #[test]
    fn test_parse_error() {
        // With progress
        assert_eq!(parse_osc_9_4(&["2", "75"]).unwrap(), ProgressBar::error(Some(75)));
        // Without progress (indeterminate error)
        assert_eq!(parse_osc_9_4(&["2"]).unwrap(), ProgressBar::error(None));
    }

    #[test]
    fn test_parse_indeterminate() {
        assert_eq!(parse_osc_9_4(&["3"]).unwrap(), ProgressBar::indeterminate());
        // Progress should be ignored
        assert_eq!(parse_osc_9_4(&["3", "50"]).unwrap(), ProgressBar::indeterminate());
    }

    #[test]
    fn test_parse_warning() {
        assert_eq!(parse_osc_9_4(&["4", "25"]).unwrap(), ProgressBar::warning(25));
        // Missing progress should error
        assert!(parse_osc_9_4(&["4"]).is_err());
    }

    #[test]
    fn test_clamping() {
        // Above max
        assert_eq!(parse_osc_9_4(&["1", "150"]).unwrap(), ProgressBar::normal(100));
        // Below min
        assert_eq!(parse_osc_9_4(&["1", "-10"]).unwrap(), ProgressBar::normal(0));
    }

    #[test]
    fn test_invalid_state() {
        assert!(parse_osc_9_4(&["5"]).is_err());
        assert!(parse_osc_9_4(&["abc"]).is_err());
    }
}
```

### Integration Tests (Python)

```python
def test_progress_bar_sequences():
    """Test OSC 9;4 progress bar parsing."""
    term = Terminal(80, 24)

    # Test normal progress
    term.write("\x1b]9;4;1;50\x07")
    events = term.poll_events()
    assert len(events) == 1
    assert events[0]["type"] == "ProgressBar"
    assert events[0]["state"] == "Normal"
    assert events[0]["progress"] == 50

    # Test indeterminate
    term.write("\x1b]9;4;3\x07")
    events = term.poll_events()
    assert events[0]["state"] == "Indeterminate"
    assert events[0]["progress"] is None

    # Test error with progress
    term.write("\x1b]9;4;2;75\x07")
    events = term.poll_events()
    assert events[0]["state"] == "Error"
    assert events[0]["progress"] == 75

    # Test error without progress (indeterminate)
    term.write("\x1b]9;4;2\x07")
    events = term.poll_events()
    assert events[0]["state"] == "Error"
    assert events[0]["progress"] is None

    # Test remove
    term.write("\x1b]9;4;0\x07")
    events = term.poll_events()
    assert events[0]["state"] == "Remove"

    # Test abbreviated remove
    term.write("\x1b]9;4\x07")
    events = term.poll_events()
    assert events[0]["state"] == "Remove"
```

## Best Practices for Applications

1. **Always Clear**: Send `OSC 9;4;0` when operation completes or is cancelled
2. **Flush Output**: Always flush stdout after sending the sequence
3. **Rate Limiting**: Update progress at reasonable intervals (every 1-10% or 100ms-500ms minimum)
4. **Start Indeterminate**: If duration is unknown, start with state 3, then switch to state 1 when percentage is known
5. **Error Handling**: Use state 2 for failures, optionally showing where it failed
6. **Value Range**: Always send progress values in 0-100 range (don't rely on terminal clamping)

## Terminal Implementation Best Practices

1. **Clamp Values**: Automatically clamp progress to 0-100 range
2. **Graceful Degradation**: Ignore malformed sequences without disrupting display
3. **Visual Feedback**: Use platform-native progress indicators when available
4. **Persistence**: Clear progress bar when session/tab closes
5. **User Control**: Consider allowing users to disable progress indicators in settings
6. **Thread Safety**: Progress bar updates may come from any thread

## References

- **Full Research Document**: `/Users/probello/Repos/research/terminal-emulators/osc-9-4-progress-bars-2026-02-09.md`
- **ConEmu Specification**: https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC
- **iTerm2 Documentation**: https://iterm2.com/documentation-escape-codes.html
- **rockorager.dev Spec**: https://rockorager.dev/misc/osc-9-4-progress-bars/
- **par-term Feature Matrix**: `/Users/probello/Repos/par-term/MATRIX.md` (section 39)

## Implementation Checklist

- [ ] Add `ProgressState` and `ProgressBar` types to terminal module
- [ ] Implement `parse_osc_9_4()` parser function
- [ ] Add OSC 9 handler that distinguishes between `;4` (progress) and notification
- [ ] Add progress bar event to `TerminalEvent` enum
- [ ] Update streaming protocol to include progress bar events
- [ ] Add Python bindings for progress bar events
- [ ] Write Rust unit tests for parser
- [ ] Write Python integration tests
- [ ] Update API documentation
- [ ] Update README with progress bar support
