# ENH-054: Split `register_classes` into per-module `register(m)` functions

> Filed from the 2026-10-12 /opus-audit enhancement pass (AUDIT.md "Detailed Findings → Architecture & Design"). Board card: `[ENH-054]`.
> Sequencing: independent of every audit phase. It touches only the root crate (`src/lib.rs`, `src/python_bindings/`) and none of the files under `crates/par-mux/src/mux/attach/` that ARC-136…141, QA-243 and SEC-212 edit. It can land at any time.

**Priority**: low · **Estimate**: S

## Goal

`register_classes` (`src/lib.rs:220-291`) is the repo's highest-cyclomatic function (parsight `most_complex_functions` #1: complexity **67**, risk Critical). The complexity is not real branching. It comes from 66 `m.add_class::<…>()?` lines, each with a `?` early return. `src/lib.rs` also ranks as a god_objects entry: 7 functions, total complexity 138, max 67. Its second entry, `register_color_utils` (`:294`), has complexity 37 from the same `?` pattern.

Replace the flat list with one `register(m)` function per binding module, each registering the classes that module defines. `register_classes` then becomes a short call list. Behaviour stays exactly the same: the same 66 classes, the same functions, the same constants and an unchanged `_native.pyi`. A new Python test pins the registered class set against the shipped stub, so a future module that forgets a class fails CI instead of silently dropping it from the stub.

## Current state (HEAD `aaf35ef`; re-locate by symbol name if lines drift)

- **`src/lib.rs`** (341 lines):
  - The legacy `#[doc(hidden)] pub use python_bindings::{…}` re-export block (ARC-005), with its comment at `:147-150` and the list at `:151-174`. `register_*` uses these names unqualified.
  - `#[pymodule] fn _native` at `:192-202` calls `register_constants` (`:206`), `register_classes` (`:220`), `register_color_utils` (`:294`), `register_unicode_width` (`:320`) and `register_streaming_codec` (`:333`).
  - Every `register_*` is gated `#[cfg(any(feature = "python", feature = "python-test"))]`.
- **`register_classes` holds 66 `add_class` calls** (`grep -c add_class src/lib.rs` prints `66`). `grep -c '^class ' python/par_term_emu_core_rust/_native.pyi` also prints `66`.
- **Where each class is defined** (`#[pyclass]` sites):

  | Module | Classes |
  |---|---|
  | `python_bindings/terminal/mod.rs` | `PyTerminal` |
  | `python_bindings/pty.rs` | `PyPtyTerminal` |
  | `python_bindings/screenshot_config.rs` | `PyScreenshotConfig` |
  | `python_bindings/streaming.rs` | `PyStreamingServer`, `PyStreamingConfig`. Real types under `cfg(feature = "streaming")`, placeholder types at `:1098-1102+` under `cfg(not(feature = "streaming"))`, so both builds register the same names. |
  | `python_bindings/enums.rs` | `PyCursorStyle`, `PyUnderlineStyle`, `PyMouseEncoding`, `PyProgressState`, `PyUnicodeVersion`, `PyAmbiguousWidth`, `PyWidthConfig`, `PyNormalizationForm` |
  | `types/clipboard.rs` | `PyClipboardEntry`, `PyClipboardSyncEvent`, `PyClipboardHistoryEntry` |
  | `types/color.rs` | `PyColorHSV`, `PyColorHSL`, `PyColorPalette` |
  | `types/graphics.rs` | `PyImageDimension`, `PyImagePlacement`, `PyGraphic`, `PyInlineImage`, plus the enums `PyImageProtocol` (`:244`) and `PyImageFormat` (`:256`) |
  | `types/metrics.rs` | `PyPerformanceMetrics`, `PyFrameTiming`, `PyEscapeSequenceProfile`, `PyProfilingData`, `PyBenchmarkResult`, `PyBenchmarkSuite`, `PyComplianceTest`, `PyComplianceReport` |
  | `types/mouse.rs` | `PyMouseEvent`, `PyMousePosition` |
  | `types/notification.rs` | `PyTmuxNotification`, `PyNotification`, `PyNotificationEvent`, `PyNotificationConfig` |
  | `types/recording.rs` | `PyCoprocessConfig`, `PyRecordingEvent`, `PyRecordingSession`, `PyMacroEvent`, `PyMacro` |
  | `types/screen.rs` | `PyAttributes`, `PyScreenSnapshot`, `PyScrollbackStats`, `PyBookmark`, `PyJoinedLines`, `PyDamageRegion`, `PyRenderingHint`, `PyLineDiff`, `PySnapshotDiff` |
  | `types/selection.rs` | `PySearchMatch`, `PyDetectedItem`, `PySelection`, `PyRegexMatch`, plus the enum `PySelectionMode` (`:73`) |
  | `types/shell.rs` | `PyShellIntegration`, `PyProgressBar`, `PyCommandExecution`, `PyShellIntegrationStats`, `PyCwdChange` |
  | `types/trigger.rs` | `PyTrigger`, `PyTriggerMatch`, `PyTriggerAction` |

  That is 66 classes in total, which matches the `add_class` count.
- **`src/python_bindings/types/mod.rs`** declares the 11 submodules and `pub use`s each with a glob.
- **Stub tooling.**
  - `scripts/generate_stubs.py` (`main()` `:411`) iterates `dir(native)`. It classifies `type(obj).__name__ == "type"` as a class and emits classes **sorted by name** (`for cname in sorted(classes)`), so registration order does not affect the stub.
  - Under `Makefile:355` `stubs` and `:365` `stub-check` (pyright on the stub, `check_api_reference.py`, `check_docstring_examples.py`), `stub-check` is in `checkall` (`:525`).
  - `stub-drift` (`:379`) rebuilds, regenerates and runs `git diff --exit-code` on the stub. It is not in `checkall`.
- **No test asserts the registered class set.** `tests/test_capability.py` only checks `HAS_STREAMING` and the streaming exports.

## Implementation

1. **Re-confirm the mapping.** Run `grep -rn "pub struct Py\|pub enum Py" src/python_bindings --include=*.rs` and check that every one of the 66 `add_class` targets appears exactly once. A target can also be an alias, and none was seen at `aaf35ef`.
2. **Add one `register` per module.** Use the signature `pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()>`, gated by the same `#[cfg(any(feature = "python", feature = "python-test"))]` the module already compiles under. Each function contains only the `add_class` lines for the classes defined in that file, in the order they appear in the old list. Add one to each file below:
   - `terminal/mod.rs` and `pty.rs`.
   - `screenshot_config.rs`. It is a single-class module, so keep its function a single line.
   - `streaming.rs`. Write **one** unconditional `register`, because the real and placeholder types share the names `PyStreamingServer` / `PyStreamingConfig` under complementary cfgs.
   - `enums.rs`.
   - Each of the 11 `types/*.rs`.
3. **Add `types::register`** in `types/mod.rs`. Write it as a loop over the 11 submodule `register`s, `for register in [clipboard::register, color::register, graphics::register, metrics::register, mouse::register, notification::register, recording::register, screen::register, selection::register, shell::register, trigger::register] { register(m)?; }`, keeping that order. With 11 straight-line `?` calls this function alone would score 12, the same `?` count that gave `register_classes` its 67. The loop scores about 2.
4. **Add `python_bindings::register_classes`** in `python_bindings/mod.rs`. Use the same loop form over `terminal::register`, `pty::register`, `screenshot_config::register`, `enums::register`, `types::register` and `streaming::register`, then `Ok(())`. The largest per-file `register` is `types/screen.rs`, with 9 classes and an expected score of 10.
5. **Rewrite `src/lib.rs`.**
   - Replace the body of `register_classes` with `python_bindings::register_classes(m)`, or call that directly from `_native` and delete the local fn. Either is fine, but keep `_native`'s five-call shape readable.
   - **Keep** the ARC-005 `pub use python_bindings::{…}` block unchanged. It is a public, `doc(hidden)` API that par-term may import, and the rewrite only stops `register_classes` from depending on it.
   - Leave `register_color_utils`, `register_unicode_width`, `register_streaming_codec` and `register_constants` as they are. Their complexity is the same `?` pattern, but they are already per-topic. Out of scope.
6. **Add `tests/test_native_surface.py`.** It pins the registered class set against the shipped stub:
   ```python
   """The `_native` module registers exactly the classes `_native.pyi` declares (ENH-054)."""

   import ast
   from pathlib import Path

   from par_term_emu_core_rust import _native

   STUB = (
       Path(__file__).resolve().parents[1]
       / "python"
       / "par_term_emu_core_rust"
       / "_native.pyi"
   )


   def _stub_classes() -> set[str]:
       tree = ast.parse(STUB.read_text(encoding="utf-8"))
       return {node.name for node in tree.body if isinstance(node, ast.ClassDef)}


   def _registered_classes() -> set[str]:
       # Same classification rule as scripts/generate_stubs.py main().
       return {
           name
           for name in dir(_native)
           if not name.startswith("_") and type(getattr(_native, name)).__name__ == "type"
       }


   def test_registered_classes_match_the_stub() -> None:
       stub, live = _stub_classes(), _registered_classes()
       assert live - stub == set(), (
           f"registered but missing from the stub: {sorted(live - stub)}"
       )
       assert stub - live == set(), (
           f"in the stub but not registered: {sorted(stub - live)}"
       )


   def test_registered_class_count_is_pinned() -> None:
       assert len(_registered_classes()) == 66
   ```
   The count pin is deliberate. A new class must update it and the stub together, and that is the review signal the card is asking for.

## Files to touch

- `src/lib.rs` (`register_classes` body only)
- `src/python_bindings/mod.rs`, `terminal/mod.rs`, `pty.rs`, `screenshot_config.rs`, `streaming.rs`, `enums.rs` (one `register` each)
- `src/python_bindings/types/mod.rs` plus the 11 `types/*.rs` (one `register` each)
- `tests/test_native_surface.py` (new)

## Verify

Prefix every `make` with `PATH="$PWD/.venv/bin:$PATH"`. Never use `cargo build` for the extension.

- `PATH="$PWD/.venv/bin:$PATH" make dev > /tmp/enh054-dev.log 2>&1; echo EXIT=$?` prints `EXIT=0`. Then `uv run pytest tests/test_native_surface.py tests/test_capability.py -v > /tmp/enh054-py.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and the log shows `test_registered_classes_match_the_stub PASSED` and `test_registered_class_count_is_pinned PASSED`.
- **Stub unchanged.** `PATH="$PWD/.venv/bin:$PATH" make stub-drift > /tmp/enh054-drift.log 2>&1; echo EXIT=$?` prints `EXIT=0`, which means `git diff --exit-code -- python/par_term_emu_core_rust/_native.pyi` is clean.
- **Complexity gone.** `grep -c 'add_class' src/lib.rs` prints `0`. After reindexing, parsight `audit_repository` `cyclomatic_complexity` with `target: "register_classes"` and `file_path: "src/lib.rs"` reports complexity ≤ 3, and no `register` function under `src/python_bindings/` exceeds 10.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh054-ci.log 2>&1; echo EXIT=$?` prints `EXIT=0`. This covers `test-rust`'s `python-test` lib run, `stub-check` and `test-python`.

Acceptance criteria:
- `make stub-drift` exits 0 (the generated `_native.pyi` is byte-identical), and `grep -c add_class src/lib.rs` prints `0`.
- New `tests/test_native_surface.py` passes and fails when a class is removed from any `register` (spot-check: comment out one `add_class`, `make dev`, confirm the test fails naming the class, restore).
- parsight reports `register_classes` complexity ≤ 3 and no per-module `register` above 10.
- `make checkall-ci` exits 0.

## Rollback

Restore the flat `register_classes` body in `src/lib.rs` and delete the per-module `register` fns. The `pub use` re-exports were never touched, so nothing downstream breaks either way. The new test stays valid against either shape and can be kept.
