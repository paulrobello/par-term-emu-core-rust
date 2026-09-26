# ENH-017 — Ship streaming-enabled Python wheels

> Filed from the 2026-09-26 /opus-audit enhancement pass (derived from audit ARC-025's
> analysis; the audit finding itself is only the capability constant). Board card: `[ENH-017]`
> (priority medium, estimate M). Consumer: `/enhancement-all` / `/enhancement-next`.

## Goal

Users who `pip install par-term-emu-core-rust` cannot actually use the streaming server: wheels
are built without the `streaming` feature (`pyproject.toml [tool.maturin] features`,
`deployment.yml` maturin args), `StreamingServer`/`StreamingConfig` register as stubs whose
constructors raise `RuntimeError`, and `_has_streaming` is always `True` because the stubs still
import (audit ARC-025). Ship a wheel variant with streaming compiled in, plus honest capability
detection.

## Current state

- `python/par_term_emu_core_rust/__init__.py:75-95,166-177`: `except ImportError` feature
  detection never fires; `__all__` advertises `StreamingServer`/`StreamingConfig` unconditionally.
- `src/python_bindings/streaming.rs:1023-1059` registers stub classes in non-streaming builds;
  `:1215-1254` registers non-streaming codec functions.
- Publish pipeline: `.github/workflows/deployment.yml` (maturin builds per-platform wheels).
- The audit's ARC-025 remedy (native `HAS_STREAMING` constant via `register_constants`) is a
  prerequisite — land it first if not already done.

## Implementation

1. **Prerequisite**: ARC-025's `HAS_STREAMING: bool` native constant, and `__init__.py` deriving
   `_has_streaming` from it. Skip if already shipped.
2. **Decide the delivery shape** (recommendation: variant wheels):
   - **Option A (recommended)** — build all wheels WITH streaming. The streaming feature pulls
     tokio/axum/TLS deps and grows the wheel, but streaming is a headline feature and every
     platform in the matrix already builds it for `par-term-streamer`. One wheel set, no user
     confusion, `_has_streaming` honestly True everywhere. Check wheel size delta first
     (`cargo tree` for the streaming dep closure; if the size delta is >~40%, reconsider).
   - **Option B** — two wheel sets: `par-term-emu-core-rust` (no streaming) and
     `par-term-emu-core-rust-streaming` or an extrasrequire mapping to a `-streaming` wheel.
     Doubles publish complexity; only worth it if Option A's size cost is real.
3. **Build wiring**: in `deployment.yml`, add `--features streaming` (and keep
   `python`) to the maturin build args for the chosen shape; for Option B add a second matrix
   leg or a second maturin invocation producing suffixed distributions.
4. **Docs**: `docs/STREAMING.md` install section, `README.md` install/feature table, and the
   `docs/API_REFERENCE.md` streaming intro — state which variant carries streaming and how to
   check (`HAS_STREAMING`). Update the capability-detection snippet.
5. **Changelog + README What's New** entry (release process requires both).

## Files to touch

- `.github/workflows/deployment.yml` (maturin args / matrix)
- `pyproject.toml` (if Option B: distribution naming; else likely unchanged)
- `python/par_term_emu_core_rust/__init__.py` (capability docstring; `__all__` already gated)
- `docs/STREAMING.md`, `README.md`, `docs/API_REFERENCE.md` (install + capability docs)
- `CHANGELOG.md`

## Verify (acceptance criteria on the card)

1. A locally built wheel of the chosen variant installs into a clean venv and
   `StreamingServer(config)` constructs (bind a loopback port, then stop it) — not a stub
   RuntimeError.
2. `HAS_STREAMING`/`_has_streaming` reflects reality on both the dev build and the built wheel;
   the documented capability check works.
3. Publish pipeline updated (dry-run locally where possible; the actual publish stays behind the
   normal release process); `make checkall` green.

## Rollback

Revert the workflow args; stub behavior returns. No API break (the constant stays).
