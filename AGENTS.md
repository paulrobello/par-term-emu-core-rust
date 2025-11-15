# Repository Guidelines

## Project Structure & Module Organization
- `src/` — Rust library modules: `terminal.rs`, `grid.rs`, `sixel.rs`, `pty_session.rs`, etc.
- `python/par_term_emu/` — Python package and PyO3 `_native` extension.
- `examples/` — Runnable Python demos (ANSI, PTY, sixel, TUI helpers).
- `tests/` — Pytest suite; integration-heavy terminal/PTY tests.
- `tui/` — Optional Textual-based demo app.
- `docs/` — Architecture, security, and debugging notes.
- `Makefile` — Primary developer entry points.

## Tooling Rules (Very Important)
- Always use `uv` for Python. Never call `pip` directly; prefer `uv run <cmd>`.
- Build the PyO3 extension via `maturin develop` (already wired in `make dev`), not bare `cargo build`.
- macOS/Windows shells: run everything from a normal terminal, not inside our TUI.

## Build, Test, and Development Commands
- `make setup-venv` — Create `.venv` via `uv` and install tools.
- `make dev` — Build and develop-install via `maturin` (release mode).
- `make build` | `make build-release` — Debug/release builds.
- `make test` | `make test-rust` | `make test-python` — Run tests.
- `make fmt` | `make lint` | `make checkall` — Format/lint Rust+Python.
- `make examples` | `make tui` — Run demos (`make tui-install` first for TUI).
- `make pre-commit-install` — Install hooks; recommended before committing.

Quick workflow (from a clean checkout):
- `make setup-venv && make dev` — sync deps + build
- `make test-python` — run Python integration tests (preferred)
- `cargo test` — run Rust unit tests
- `make checkall` — run all quality checks (fmt+lint+types+tests)

## Coding Style & Naming Conventions
- Rust: `rustfmt` defaults; `clippy -D warnings` is enforced. Types/traits/enums `PascalCase`; functions/modules `snake_case`; constants `SCREAMING_SNAKE_CASE`. Keep modules focused; prefer `#[cfg(test)]` unit tests near code.
- Python (3.13+): `ruff format`; pass `ruff` and `pyright`. 4‑space indent. Modules/functions `snake_case`, classes `PascalCase`, constants `UPPER_CASE`. Public APIs should be type‑hinted.

## VT Parsing Gotchas
- CSI parameter semantics: per VT spec, parameter 0 or a missing parameter defaults to 1. Always normalize with something equivalent to:
  - Rust: `let n = params.iter().next().and_then(|p| p.first()).copied().unwrap_or(1) as usize; let n = if n == 0 { 1 } else { n };`

## TUI Integration Notes
- Do not run the Textual TUI from within the TUI itself (causes display corruption). Always launch it from a regular terminal: `make tui`.
- PTY reader runs in Rust; Textual polls an atomic generation counter. Use `update_generation()` and avoid blocking the event loop.

## Debugging Cheatsheet
- Clear logs: `make debug-clear`
- Run with logs: `DEBUG_LEVEL=3 make tui` (levels 0–4; 3 recommended)
- Tail logs: `make debug-tail`
- Log locations:
  - Rust: `/tmp/par_term_emu_debug_rust.log`
  - Python: `/tmp/par_term_emu_debug_python.log`

## Testing Guidelines
- Frameworks: `cargo test` and `pytest` (default timeout 30s). Python tests live in `tests/` and are named `test_*.py`. Use `pytest.skipif` for platform specifics. For PTY tests, avoid brittle sleeps—prefer event‑driven waits and deterministic assertions.

Recommended emphasis for coverage:
- VT220 editing sequences (IL, DL, ICH, DCH, ECH), scroll regions (DECSTBM), alt‑screen transitions, mouse modes, and the “parameter 0 defaults to 1” rule.

## Commit & Pull Request Guidelines
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, etc.); optional scope; imperative subject ≤ 72 chars. Reference issues/PRs (e.g., `(#23)`).
- PRs: include clear description, architecture impact, repro steps, and OS/shell used; add screenshots/GIFs for TUI changes. Run `make checkall && make test` before requesting review.

### Pre‑Push Checklist
- Rust: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`
- Python: `make fmt-python`, `make lint-python`, `make test-python`
- Or simply: `make checkall && make test-python`

## Security & Configuration Tips
- PTY: never pass untrusted input to shells; document environment changes. OSC‑52 clipboard reads are disabled by default—enable explicitly when testing.
- Debugging: set `DEBUG_LEVEL=2|3|4`; tail logs with `make debug-tail`. See `docs/SECURITY.md` for security considerations.

## PyO3 Binding Notes
- Module naming must match across files or Python import will fail:
  - `pyproject.toml`: `module-name = "par_term_emu._native"`
  - `src/lib.rs`: `#[pymodule] fn _native(...)`
  - `python/par_term_emu/__init__.py`: `from ._native import ...`

## Common Pitfalls
- Borrow checker issues: the `Terminal` owns `Grid`/`Cursor`; prefer method calls to borrowing internals.
- Index bounds: validate `(col, row)` and dimensions before writes.
- Don’t use `cargo build` for the Python extension; use `make dev` (maturin).
- Parameter 0 handling in CSI: normalize to 1 to match VT behavior.
