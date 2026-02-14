# Command Output Capture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add APIs to extract text from specific command execution blocks by linking CommandExecution records to their Output zone row ranges.

**Architecture:** Store output zone's absolute row range (start/end) on CommandExecution when the command finishes. Refactor `get_zone_text` into a reusable `extract_text_from_row_range` helper. New `get_command_output(index)` and `get_command_outputs()` methods use the stored range with that helper.

**Tech Stack:** Rust (terminal core), PyO3 (Python bindings), pytest (Python tests)

---

### Task 1: Add output row range fields to CommandExecution

**Files:**
- Modify: `src/terminal/mod.rs:986-1001` (CommandExecution struct)
- Modify: `src/terminal/mod.rs:6164-6178` (start_command_execution — add None for new fields)

**Step 1: Add fields to CommandExecution struct**

In `src/terminal/mod.rs`, add two fields after `success`:

```rust
pub struct CommandExecution {
    // ... existing fields ...
    /// Whether command succeeded (exit code 0)
    pub success: Option<bool>,
    /// Absolute start row of the output zone
    pub output_start_row: Option<usize>,
    /// Absolute end row of the output zone
    pub output_end_row: Option<usize>,
}
```

**Step 2: Update start_command_execution to initialize new fields**

In `start_command_execution` (line 6170), add the new fields to the struct literal:

```rust
self.current_command = Some(CommandExecution {
    command,
    cwd: self.shell_integration.cwd().map(String::from),
    start_time: timestamp,
    end_time: None,
    exit_code: None,
    duration_ms: None,
    success: None,
    output_start_row: None,
    output_end_row: None,
});
```

**Step 3: Build and verify compilation**

Run: `cargo check --lib --no-default-features --features pyo3/auto-initialize`
Expected: PASS (may have warnings about unused fields, that's OK)

**Step 4: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat(command-output): add output row range fields to CommandExecution"
```

---

### Task 2: Wire output zone range into end_command_execution

**Files:**
- Modify: `src/terminal/mod.rs:6182-6201` (end_command_execution)

**Step 1: Capture output zone range before pushing to history**

In `end_command_execution`, after setting `cmd.success` and before `self.command_history.push(cmd)`, add:

```rust
pub fn end_command_execution(&mut self, exit_code: i32) {
    if let Some(mut cmd) = self.current_command.take() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        cmd.end_time = Some(timestamp);
        cmd.exit_code = Some(exit_code);
        cmd.duration_ms = Some(timestamp.saturating_sub(cmd.start_time));
        cmd.success = Some(exit_code == 0);

        // Capture output zone range from the most recent Output zone
        if let Some(zone) = self.grid.zones().last() {
            if zone.zone_type == crate::zone::ZoneType::Output {
                cmd.output_start_row = Some(zone.abs_row_start);
                cmd.output_end_row = Some(zone.abs_row_end);
            }
        }

        self.command_history.push(cmd);

        // Limit history size
        if self.command_history.len() > self.max_command_history {
            self.command_history
                .drain(0..self.command_history.len() - self.max_command_history);
        }
    }
}
```

**Step 2: Build and verify**

Run: `cargo check --lib --no-default-features --features pyo3/auto-initialize`
Expected: PASS

**Step 3: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat(command-output): wire output zone range into end_command_execution"
```

---

### Task 3: Refactor get_zone_text into reusable extract_text_from_row_range

**Files:**
- Modify: `src/terminal/mod.rs:2245-2313` (get_zone_text)

**Step 1: Create extract_text_from_row_range helper**

Add a new private method right before `get_zone_text`:

```rust
/// Extract text content from a range of absolute rows.
/// Returns None if the range is entirely outside the current buffer.
fn extract_text_from_row_range(&self, abs_start: usize, abs_end: usize) -> Option<String> {
    let scrollback_len = self.grid.scrollback_len();
    let total_rows = scrollback_len + self.grid.rows();

    // Check if the range is entirely evicted
    if abs_end < self.grid.total_lines_scrolled().saturating_sub(self.grid.max_scrollback()) {
        return None;
    }

    let mut text = String::new();
    let mut found_any = false;

    for row in abs_start..=abs_end {
        if row < scrollback_len {
            // Row is in scrollback
            if let Some(line) = self.grid.scrollback_line(row) {
                found_any = true;
                let line_text: String = line
                    .iter()
                    .filter(|c| !c.flags.wide_char_spacer())
                    .map(|c| {
                        let mut s = String::new();
                        s.push(c.c);
                        for &combining in &c.combining {
                            s.push(combining);
                        }
                        s
                    })
                    .collect();
                let trimmed = line_text.trim_end();
                if !text.is_empty() {
                    if row > abs_start && self.grid.is_scrollback_wrapped(row - 1) {
                        // Wrapped line - no newline
                    } else {
                        text.push('\n');
                    }
                }
                text.push_str(trimmed);
            }
        } else {
            let grid_row = row - scrollback_len;
            if grid_row < self.grid.rows() {
                if let Some(line) = self.grid.row(grid_row) {
                    found_any = true;
                    let line_text: String = line
                        .iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in &c.combining {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() && row > abs_start {
                        let prev_row = row - 1;
                        if prev_row < scrollback_len {
                            if !self.grid.is_scrollback_wrapped(prev_row) {
                                text.push('\n');
                            }
                        } else {
                            let prev_grid_row = prev_row - scrollback_len;
                            if !self.grid.is_line_wrapped(prev_grid_row) {
                                text.push('\n');
                            }
                        }
                    }
                    text.push_str(trimmed);
                }
            }
        }
    }

    if found_any { Some(text) } else { None }
}
```

**Step 2: Refactor get_zone_text to use the helper**

Replace `get_zone_text` body:

```rust
pub fn get_zone_text(&self, abs_row: usize) -> Option<String> {
    let zone = self.grid.zone_at(abs_row)?;
    self.extract_text_from_row_range(zone.abs_row_start, zone.abs_row_end)
}
```

**Step 3: Run existing tests to verify refactor is correct**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize test_zone`
Expected: All existing zone tests PASS

**Step 4: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "refactor: extract reusable text extraction helper from get_zone_text"
```

---

### Task 4: Add CommandOutput struct and get_command_output / get_command_outputs methods

**Files:**
- Modify: `src/terminal/mod.rs` (add struct near CommandExecution, add methods near get_command_history)

**Step 1: Write Rust unit tests for get_command_output**

Add tests in `src/terminal/sequences/osc.rs` test module (after the existing zone tests):

```rust
#[test]
fn test_get_command_output_basic() {
    let mut term = Terminal::new(80, 24);

    // Set up shell integration command
    term.shell_integration_mut().set_command("ls".to_string());
    term.start_command_execution("ls".to_string());

    // Full OSC 133 cycle
    term.process(b"\x1b]133;A\x07$ \r\n");
    term.process(b"\x1b]133;B\x07ls\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"file1.txt\r\nfile2.txt\r\n");
    term.process(b"\x1b]133;D;0\x07");

    term.end_command_execution(0);

    // Index 0 = most recent command
    let output = term.get_command_output(0);
    assert!(output.is_some());
    let text = output.unwrap();
    assert!(text.contains("file1.txt"));
    assert!(text.contains("file2.txt"));
}

#[test]
fn test_get_command_output_out_of_bounds() {
    let mut term = Terminal::new(80, 24);
    assert!(term.get_command_output(0).is_none());
    assert!(term.get_command_output(100).is_none());
}

#[test]
fn test_get_command_output_no_zone() {
    let mut term = Terminal::new(80, 24);
    // Start and end command without OSC 133 zones
    term.start_command_execution("echo hi".to_string());
    term.end_command_execution(0);

    // Should return None since no output zone exists
    assert!(term.get_command_output(0).is_none());
}

#[test]
fn test_get_command_output_multiple_commands() {
    let mut term = Terminal::new(80, 24);

    // First command
    term.shell_integration_mut().set_command("cmd1".to_string());
    term.start_command_execution("cmd1".to_string());
    term.process(b"\x1b]133;A\x07$ \r\n");
    term.process(b"\x1b]133;B\x07cmd1\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"output1\r\n");
    term.process(b"\x1b]133;D;0\x07");
    term.end_command_execution(0);

    // Second command
    term.shell_integration_mut().set_command("cmd2".to_string());
    term.start_command_execution("cmd2".to_string());
    term.process(b"\x1b]133;A\x07$ \r\n");
    term.process(b"\x1b]133;B\x07cmd2\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"output2\r\n");
    term.process(b"\x1b]133;D;0\x07");
    term.end_command_execution(0);

    // Index 0 = most recent (cmd2), index 1 = older (cmd1)
    let out0 = term.get_command_output(0).unwrap();
    assert!(out0.contains("output2"));
    let out1 = term.get_command_output(1).unwrap();
    assert!(out1.contains("output1"));
}

#[test]
fn test_get_command_outputs_filters_evicted() {
    let mut term = Terminal::with_scrollback(80, 5, 10);

    // First command - will be evicted
    term.shell_integration_mut().set_command("old".to_string());
    term.start_command_execution("old".to_string());
    term.process(b"\x1b]133;A\x07$ \r\n");
    term.process(b"\x1b]133;B\x07old\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"old output\r\n");
    term.process(b"\x1b]133;D;0\x07");
    term.end_command_execution(0);

    // Generate lots of output to push old command into eviction
    for i in 0..30 {
        term.process(format!("filler line {}\r\n", i).as_bytes());
    }

    // Second command - recent
    term.shell_integration_mut().set_command("new".to_string());
    term.start_command_execution("new".to_string());
    term.process(b"\x1b]133;A\x07$ \r\n");
    term.process(b"\x1b]133;B\x07new\r\n");
    term.process(b"\x1b]133;C\x07");
    term.process(b"new output\r\n");
    term.process(b"\x1b]133;D;0\x07");
    term.end_command_execution(0);

    let outputs = term.get_command_outputs();
    // Only the recent command should have extractable output
    assert!(!outputs.is_empty());
    assert!(outputs.iter().any(|o| o.output.contains("new output")));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize test_get_command_output`
Expected: FAIL — methods don't exist yet

**Step 3: Add CommandOutput struct and implement methods**

Add `CommandOutput` struct near `CommandExecution` (around line 1002):

```rust
/// Command output record combining execution metadata with extracted output text
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Command that was executed
    pub command: String,
    /// Current working directory when command was run
    pub cwd: Option<String>,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Extracted output text
    pub output: String,
}
```

Add methods near `get_command_history` (around line 6212):

```rust
/// Get command output text by index (0 = most recent completed command).
/// Returns None if index is out of bounds or output has been evicted from scrollback.
pub fn get_command_output(&self, index: usize) -> Option<String> {
    let history = &self.command_history;
    if history.is_empty() || index >= history.len() {
        return None;
    }
    let cmd = &history[history.len() - 1 - index];
    let start = cmd.output_start_row?;
    let end = cmd.output_end_row?;
    self.extract_text_from_row_range(start, end)
}

/// Get all commands with extractable output text.
/// Commands whose output has been evicted from scrollback are excluded.
pub fn get_command_outputs(&self) -> Vec<CommandOutput> {
    self.command_history
        .iter()
        .filter_map(|cmd| {
            let start = cmd.output_start_row?;
            let end = cmd.output_end_row?;
            let output = self.extract_text_from_row_range(start, end)?;
            Some(CommandOutput {
                command: cmd.command.clone(),
                cwd: cmd.cwd.clone(),
                exit_code: cmd.exit_code,
                output,
            })
        })
        .collect()
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize test_get_command_output`
Expected: All PASS

**Step 5: Commit**

```bash
git add src/terminal/mod.rs src/terminal/sequences/osc.rs
git commit -m "feat(command-output): add get_command_output and get_command_outputs APIs"
```

---

### Task 5: Add Python bindings

**Files:**
- Modify: `src/python_bindings/types.rs:2476-2514` (PyCommandExecution)
- Modify: `src/python_bindings/terminal.rs` (near line 4757, after get_command_history)

**Step 1: Add output row fields to PyCommandExecution**

In `src/python_bindings/types.rs`, add fields to `PyCommandExecution`:

```rust
#[pyclass(name = "CommandExecution", from_py_object)]
#[derive(Clone)]
pub struct PyCommandExecution {
    #[pyo3(get)]
    pub command: String,
    #[pyo3(get)]
    pub cwd: Option<String>,
    #[pyo3(get)]
    pub start_time: u64,
    #[pyo3(get)]
    pub end_time: Option<u64>,
    #[pyo3(get)]
    pub exit_code: Option<i32>,
    #[pyo3(get)]
    pub duration_ms: Option<u64>,
    #[pyo3(get)]
    pub success: Option<bool>,
    #[pyo3(get)]
    pub output_start_row: Option<usize>,
    #[pyo3(get)]
    pub output_end_row: Option<usize>,
}
```

Update the `From` impl to include new fields:

```rust
impl From<&crate::terminal::CommandExecution> for PyCommandExecution {
    fn from(cmd: &crate::terminal::CommandExecution) -> Self {
        PyCommandExecution {
            command: cmd.command.clone(),
            cwd: cmd.cwd.clone(),
            start_time: cmd.start_time,
            end_time: cmd.end_time,
            exit_code: cmd.exit_code,
            duration_ms: cmd.duration_ms,
            success: cmd.success,
            output_start_row: cmd.output_start_row,
            output_end_row: cmd.output_end_row,
        }
    }
}
```

**Step 2: Add Python bindings for new Terminal methods**

In `src/python_bindings/terminal.rs`, after `get_current_command` (around line 4775):

```rust
    /// Get command output text by index (0 = most recent completed command).
    ///
    /// Args:
    ///     index: Command index (0 = most recent)
    ///
    /// Returns:
    ///     Output text if available, None if index out of bounds or output evicted
    ///
    /// Example:
    ///     >>> term.get_command_output(0)  # Get most recent command's output
    ///     'file1.txt\nfile2.txt'
    fn get_command_output(&self, index: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_command_output(index))
    }

    /// Get all commands with extractable output text.
    /// Commands whose output has been evicted from scrollback are excluded.
    ///
    /// Returns:
    ///     List of dicts with keys: command, cwd, exit_code, output
    ///
    /// Example:
    ///     >>> outputs = term.get_command_outputs()
    ///     >>> outputs[0]['command']
    ///     'ls -la'
    fn get_command_outputs(&self) -> PyResult<Vec<pyo3::Py<pyo3::types::PyDict>>> {
        let outputs = self.inner.get_command_outputs();
        Python::attach(|py| {
            let mut result = Vec::with_capacity(outputs.len());
            for out in &outputs {
                let dict = pyo3::types::PyDict::new(py);
                dict.set_item("command", &out.command)?;
                dict.set_item("cwd", out.cwd.as_deref())?;
                dict.set_item("exit_code", out.exit_code)?;
                dict.set_item("output", &out.output)?;
                result.push(dict.into());
            }
            Ok(result)
        })
    }
```

**Step 3: Build with maturin to verify**

Run: `make dev`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add src/python_bindings/types.rs src/python_bindings/terminal.rs
git commit -m "feat(command-output): add Python bindings for command output capture"
```

---

### Task 6: Add Python tests

**Files:**
- Create: `tests/test_command_output.py`

**Step 1: Write Python tests**

```python
"""Tests for command output capture (issue #36)."""

from par_term_emu_core_rust import Terminal


class TestGetCommandOutput:
    """Test get_command_output() method."""

    def test_basic_output_capture(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"file1.txt\r\nfile2.txt\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        output = term.get_command_output(0)
        assert output is not None
        assert "file1.txt" in output
        assert "file2.txt" in output

    def test_returns_none_for_empty_history(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_command_output(0) is None

    def test_returns_none_for_out_of_bounds(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)
        assert term.get_command_output(5) is None

    def test_command_without_zones_returns_none(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("echo hi")
        term.end_command_execution(0)
        assert term.get_command_output(0) is None

    def test_multiple_commands_indexed_correctly(self) -> None:
        term = Terminal(80, 24, scrollback=100)

        # First command
        term.start_command_execution("cmd1")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07cmd1\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output1\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        # Second command
        term.start_command_execution("cmd2")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07cmd2\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output2\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        # 0 = most recent
        out0 = term.get_command_output(0)
        assert out0 is not None
        assert "output2" in out0

        out1 = term.get_command_output(1)
        assert out1 is not None
        assert "output1" in out1

    def test_empty_output_returns_empty_string(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("true")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        output = term.get_command_output(0)
        # Output zone exists but may have no content
        assert output is not None


class TestGetCommandOutputs:
    """Test get_command_outputs() bulk method."""

    def test_empty_history(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_command_outputs() == []

    def test_returns_dict_with_expected_keys(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"hello\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        outputs = term.get_command_outputs()
        assert len(outputs) == 1
        assert "command" in outputs[0]
        assert "cwd" in outputs[0]
        assert "exit_code" in outputs[0]
        assert "output" in outputs[0]
        assert outputs[0]["command"] == "ls"
        assert "hello" in outputs[0]["output"]

    def test_filters_evicted_output(self) -> None:
        term = Terminal(80, 5, scrollback=10)

        # First command
        term.start_command_execution("old")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07old\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"old output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        # Push old output past scrollback
        for i in range(30):
            term.process(f"filler {i}\r\n".encode())

        # Second command
        term.start_command_execution("new")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07new\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"new output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        outputs = term.get_command_outputs()
        # Old command's output should be evicted
        commands = [o["command"] for o in outputs]
        assert "new" in commands

    def test_excludes_commands_without_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # Command without zones
        term.start_command_execution("no-zones")
        term.end_command_execution(0)

        # Command with zones
        term.start_command_execution("with-zones")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        outputs = term.get_command_outputs()
        commands = [o["command"] for o in outputs]
        assert "no-zones" not in commands
        assert "with-zones" in commands


class TestCommandExecutionFields:
    """Test that CommandExecution has the new output row fields."""

    def test_output_rows_on_command_execution(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        history = term.get_command_history()
        assert len(history) == 1
        assert history[0].output_start_row is not None
        assert history[0].output_end_row is not None
        assert history[0].output_start_row <= history[0].output_end_row

    def test_no_output_rows_without_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("echo")
        term.end_command_execution(0)

        history = term.get_command_history()
        assert len(history) == 1
        assert history[0].output_start_row is None
        assert history[0].output_end_row is None
```

**Step 2: Run Python tests**

Run: `make dev && uv run pytest tests/test_command_output.py -v`
Expected: All PASS

**Step 3: Commit**

```bash
git add tests/test_command_output.py
git commit -m "test(command-output): add Python tests for command output capture"
```

---

### Task 7: Run full check suite and fix any issues

**Files:** None (verification only)

**Step 1: Run make checkall**

Run: `make checkall`
Expected: All checks pass (fmt, lint, clippy, pyright, tests)

**Step 2: Fix any issues found**

If clippy or pyright complain, fix the issues.

**Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: address lint/type check issues from command output capture"
```

---

### Task 8: Update documentation

**Files:**
- Modify: `docs/API_REFERENCE.md` (add new methods)
- Modify: `README.md` (mention new feature in What's New if applicable)

**Step 1: Add API reference entries**

Add entries for:
- `Terminal.get_command_output(index: int) -> Optional[str]`
- `Terminal.get_command_outputs() -> list[dict]`
- `CommandExecution.output_start_row` and `CommandExecution.output_end_row` fields

**Step 2: Commit**

```bash
git add docs/API_REFERENCE.md README.md
git commit -m "docs: add command output capture API reference"
```

---

### Task 9: Create PR

**Step 1: Push branch and create PR**

```bash
gh pr create --title "feat: Command Output Capture - extract text from command execution blocks" --body "$(cat <<'EOF'
## Summary
- Adds `output_start_row` / `output_end_row` fields to `CommandExecution` linking commands to their Output zone row range
- Adds `get_command_output(index)` to extract text for a specific completed command (0 = most recent)
- Adds `get_command_outputs()` to get all commands with extractable output
- Refactors `get_zone_text` into reusable `extract_text_from_row_range` helper
- Python bindings and tests for all new APIs

Closes #36

## Test plan
- [x] Rust unit tests for basic output capture, multiple commands, eviction, no-zone fallback
- [x] Python tests mirroring Rust tests
- [x] `make checkall` passes
EOF
)"
```
