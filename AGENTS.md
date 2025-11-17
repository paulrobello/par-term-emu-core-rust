# Repository Guidelines

## Project Structure & Modules
- Core Rust library lives in `src/` (terminal, grid, PTY, screenshot). Higher-level Rust tests are in `src/tests/*.rs`.
- Python bindings and public API are in `python/par_term_emu_core_rust/` (PyO3 `_native` module).
- Python tests are in `tests/` (pytest), with runnable examples in `examples/`.
- Documentation is in `docs/` (start with `ARCHITECTURE.md`, `BUILDING.md`, `SECURITY.md`). Shell integration scripts live in `shell_integration/`.

## Build, Test & Development Commands
- First-time setup: `make setup-venv` then `make dev` (uses `uv` + `maturin`). Always use `uv` (never raw `pip`) and prefer `make dev` over `cargo build` for PyO3 work.
- Build only: `make build` (debug) or `make build-release`.
- Run tests: `make test` (Rust + Python), or `make test-rust` / `make test-python`.
- Full quality gate: `make checkall` (format, lint, type-check, tests). Run and fix issues before any commit or PR.
- Pre-commit hooks: `make pre-commit-install` to enable, `make pre-commit-run` to run on all files.

## Coding Style & Naming Conventions
- Rust: use `rustfmt` via `make fmt`; snake_case for functions and fields, CamelCase for types, SCREAMING_SNAKE_CASE for constants.
- Python: run `make fmt-python` and `make lint-python` (ruff format + ruff + pyright). Prefer type hints and pytest-style tests.
- Keep most logic in Rust, with thin Python wrappers (see `python/par_term_emu_core_rust/debug.py` for patterns).

## Implementation & API Conventions
- ANSI sequences: add handlers in `src/terminal/sequences/{csi,osc,esc,dcs}.rs`, update grid/cursor as needed, and add Rust + Python tests. VT parameter `0` or missing should usually default to `1`.
- PTY features: modify `src/pty_session.rs` and `src/python_bindings/pty.rs`, keep operations thread-safe (Arc/Mutex or atomics), and update any generation counters that track state.
- Python API: return tuples `(col, row)` for coordinates and `(r, g, b)` for colors, return `None` for invalid positions instead of raising, and keep Python wrappers thin over Rust logic.

## Testing Guidelines
- Rust tests should live alongside modules and in `src/tests/`, using descriptive `#[test]` names.
- Python tests go in `tests/test_*.py` with `test_*` functions using pytest.
- When fixing bugs or adding features, add or update tests that reproduce the behavior and update relevant docs. At minimum run `make test`; ideally run `make checkall`.

## Commit & Pull Request Guidelines
- Follow Conventional Commit-style prefixes: `feat:`, `fix:`, `docs:`, `ci:`, `chore:`, etc. (see `git log` for examples).
- Each PR should include a clear summary, list of key changes, tests executed (e.g. `make checkall`), and linked issues where relevant.
- For behavior changes (terminal output, PTY behavior, screenshots), include short examples or screenshots when helpful.

## Architecture & Security Notes
- Keep the PyO3 module definition consistent across `pyproject.toml`, `src/lib.rs`, and `python/par_term_emu_core_rust/__init__.py`.
- Respect core invariants: do not hold Rust mutexes while calling into Python (GIL), use `unicode-width` for character widths, validate row/col bounds, and remember VT mouse coordinates are 1-indexed while internals are 0-indexed.
- Review `docs/ARCHITECTURE.md` before large refactors and `docs/SECURITY.md` before PTY- or shell-related changes; keep behavior in sync with the sister project `par-term-emu-tui-rust` where configuration, options, or user-facing features overlap.
