# Repository Guidelines

## Project Structure & Modules
- Core Rust crate in `src/` (`terminal/`, `sixel/`, `pty_session.rs`, `html_export.rs`) with Python bindings in `src/python_bindings` and packaged Python shim under `python/`.
- Integration and unit tests: Rust tests co-located in `src/tests`, Python tests in `tests/`. Example scripts in `examples/` (basic, PTY, streaming).
- Docs and references live in `docs/`; helper scripts in `scripts/`; optional shell/terminfo add‑ons in `shell_integration/` and `terminfo/`.

## Build, Test, and Development Commands
- `make setup-venv` → create `.venv` with all dev deps (uv + maturin) before building.
- `make build` / `make build-release` → develop/install the Rust crate (debug vs release) via maturin.
- `make build-streaming` or `make dev-streaming` → enable the `streaming` feature; pair with `make examples-streaming` to run the WebSocket demo.
- `make test` → full Rust + Python suite; `make test-rust` runs `cargo test --lib --no-default-features --features pyo3/auto-initialize`; `make test-python` runs `pytest tests/ -v` via uv.
- `make fmt`, `make lint`, `make check` → format, clippy+fmt with autofix, and `cargo check`; `make checkall` runs format, lint, typecheck, and both test suites.
- Web frontend (Next.js) in `web-terminal-frontend/`: `make web-install`, `make web-dev`, `make web-build`, `make web-start`.

## Coding Style & Naming Conventions
- Rust: keep `rustfmt` clean; prefer explicit enums/structs; use `?` over `unwrap`; feature flags kept minimal (`streaming`).
- Python: Ruff formatting (`ruff format`) and lint (`ruff check --fix`) plus `pyright` types; modules and tests in `snake_case`.
- Naming: commits and PR titles use imperative, present-tense; prefer `feat:`, `fix:`, `chore:` prefixes seen in git log.

## Testing Guidelines
- Default expectation: `make test` green before pushing. For quick checks, run `make test-rust` when touching core and `make test-python` for bindings or examples.
- Add regression cases beside the touched code: Rust tests under `src/tests`, Python tests under `tests/` named `test_*.py`.
- Streaming or surface changes should be validated with `make examples-streaming` to ensure server/client handoff still works.

## Commit & Pull Request Guidelines
- Commits: small, focused, conventional prefix (`feat: improve cursor wrap`, `fix: reset kitty flags`).
- PRs: include scope, behavior change, and risk notes; list test commands executed; attach screenshots for web UI tweaks; link related issues/CHANGELOG entry when user-facing.
- Keep PRs draft until `make checkall` (or at least format + relevant tests) have run locally.

## Security & Configuration Tips
- Avoid adding default-privileged PTY or shell hooks; keep environment overrides explicit in examples.
- If adjusting terminfo or shell integration, document required exports (`TERM=par-term`, `COLORTERM=truecolor`) and avoid enabling system-wide changes by default.

## Agent Notes
- Use `Makefile` targets instead of ad-hoc cargo/pytest invocations to stay consistent with tooling (uv, maturin, feature flags).
- Clean builds with `make clean`; avoid removing user-created artifacts outside `target/`, `.next/`, and build caches.
