# ENH-001 — Implement `Terminal.diff_snapshots()` and make `PySnapshotDiff` reachable

## Goal

Expose a working snapshot-diff API from Python. `PySnapshotDiff` is registered in the
module (`src/lib.rs:222`) but no binding returns it — the class is unobtainable, and
`docs/API_REFERENCE.md` documents a `diff_snapshots()` that raises AttributeError today
(audit finding DOC-004 removes that doc entry; this enhancement restores it honestly).

## Current State

- Snapshot support exists: the Python API has snapshot capture (see the snapshot
  bindings in `src/python_bindings/` and `PySnapshotDiff` registration at `src/lib.rs:222`).
- Locate the Rust-side diff type first: `grep -rn "SnapshotDiff" src/` — find the core
  struct the Py wrapper wraps, and whether a core `diff` function already exists (if the
  wrapper was registered, the core type almost certainly exists with no producer wired).

## Implementation Steps

1. **Ground**: `grep -rn "SnapshotDiff\|snapshot" src/python_bindings/ src/terminal/ src/grid/`
   to map: the core snapshot type, the core diff type, existing diff logic (if any).
2. If core diff logic exists: add `Terminal::diff_snapshots(&a, &b) -> SnapshotDiff` (or
   confirm it exists) in the core; if not, implement it: compare two snapshots
   row-by-row producing changed-row indices, cursor delta, size delta — match whatever
   fields `PySnapshotDiff` already declares (read its `#[pyclass]` definition; the field
   set defines scope — do not invent fields).
3. Add the Python binding: a method on the snapshot class or `Terminal`
   (`diff_snapshots(self, other)` on the snapshot type is the most discoverable; follow
   the API convention — return the wrapper, `None` never needed here). Put it in the
   themed `*_api.rs` file that owns snapshots, or `common.rs` if both wrapper classes
   need it.
4. Docstrings per convention (Args/Returns/Example, Google style).
5. Restore the `docs/API_REFERENCE.md` entry (removed by DOC-004) with the real
   signature; add a row to the class inventory for `SnapshotDiff` usage.
6. Tests: Python test creating two snapshots around a known change (write text, snapshot,
   write more, snapshot, diff) asserting the changed rows/fields; Rust unit test for the
   core diff.

## Files to Touch

- `src/terminal/` or `src/grid/` (core diff logic — locate in step 1)
- `src/python_bindings/` (the snapshot-owning `*_api.rs` or `common.rs`)
- `src/python_bindings/types/` (PySnapshotDiff wrapper — read-only unless fields missing)
- `docs/API_REFERENCE.md`
- `tests/test_snapshots*.py` (or the existing snapshot test file)
- `CHANGELOG.md` (Added)

## Verify

- `make dev && uv run pytest tests/ -k "snapshot and diff" -v` — new tests pass
- `uv run python -c "import par_term_emu_core_rust as p; t=p.Terminal(80,24); t.process(b'a'); s1=t.snapshot(); t.process(b'b'); s2=t.snapshot(); d=s1.diff_snapshots(s2) if hasattr(s1,'diff_snapshots') else None; print(type(d))"`
  prints the SnapshotDiff type (adjust to the actual snapshot-capture method name)
- `make checkall` passes

## Rollback

Pure addition — revert the commit. No existing behavior changes; `PySnapshotDiff` was
already registered, so no import surface changes either way.
