# Contributing to par-term-emu-core-rust

This guide covers the development workflow, sync rules, and review expectations for the par-term-emu-core-rust terminal emulator library. Read it before opening a pull request.

## Table of Contents

- [Development Setup](#development-setup)
- [Build Rules](#build-rules)
- [Verification](#verification)
- [Version Sync](#version-sync)
- [Rust to Python Binding Sync](#rust-to-python-binding-sync)
- [Streaming Protocol Changes](#streaming-protocol-changes)
- [Pull Request Workflow](#pull-request-workflow)
- [Related Documentation](#related-documentation)

## Development Setup

Set up a working build environment.

```bash
make setup-venv   # Create .venv and install Python dependencies
make dev          # Build the library (release mode via maturin)
```

`make dev` rebuilds the Python extension in place. Run it after every Rust change you want to test from Python.

## Build Rules

> **Warning:** Never use `cargo build` directly for this PyO3 module. It fails at the link stage because the `extension-module` feature produces a Python extension that cannot be linked as a normal binary. Always build with `make dev` (maturin).

The only time you invoke `cargo` directly is for Rust tests, which require the `--no-default-features --features pyo3/auto-initialize` workaround so the test harness can bootstrap a Python interpreter. See `docs/BUILDING.md` for the full rationale.

## Verification

Run the full quality gate before every commit.

```bash
make checkall     # All checks: clippy, fmt, ruff, pyright, Rust + Python tests
```

Targeted checks during development:

```bash
make test         # All tests (Rust + Python)
make test-rust    # Rust tests only
make test-python  # Python tests only (rebuilds first)
make lint         # Rust clippy + fmt (auto-fix)
make lint-python  # Python ruff format + check + pyright
```

Do not push until `make checkall` passes cleanly. When fixing a failing test, confirm you are fixing the actual bug and not papering over a real issue in the code.

## Version Sync

When bumping the project version, update all three files to the same value in one commit:

1. `Cargo.toml` (`version = "X.Y.Z"`)
2. `pyproject.toml` (`version = "X.Y.Z"`)
3. `python/par_term_emu_core_rust/__init__.py` (`__version__ = "X.Y.Z"`)

Also update `CHANGELOG.md` and note breaking changes in both `CHANGELOG.md` and the README "What's New" section.

## Rust to Python Binding Sync

When you add or modify a Rust method on `Terminal` or `PtySession`, keep the layers in sync:

1. Add the Python binding in `src/python_bindings/terminal.rs` or `src/python_bindings/pty.rs`.
2. Add docstrings with `Args`, `Returns`, and `Example` sections (Google style).
3. Update `docs/API_REFERENCE.md`.
4. Update `README.md` if the change is user-facing.
5. Add Python tests in `tests/` when the feature is reachable from Python.

Files that must stay in lockstep:

- Rust impl (`src/terminal/mod.rs`) ↔ Python binding (`src/python_bindings/terminal.rs`)
- Rust impl (`src/pty_session.rs`) ↔ Python binding (`src/python_bindings/pty.rs`)
- Python binding ↔ API reference (`docs/API_REFERENCE.md`)

## Streaming Protocol Changes

The streaming protocol has three layers. A protocol change touches all of them:

1. `proto/terminal.proto` generates `src/streaming/terminal.pb.rs`. Never edit the generated file directly.
2. `src/streaming/protocol.rs` defines app-level types (`ServerMessage`, `ClientMessage`, `EventType` enums).
3. `src/streaming/proto.rs` converts between app types and the protobuf wire format.

Also update:

- `src/python_bindings/streaming.rs` (dict conversion + event type matching)
- `tests/test_streaming.rs` (use `..` in destructuring for forward compatibility)
- `src/streaming/server.rs` (`build_connect_message()` helper when extending `Connected`)

When extending the `Connected` message, update every existing constructor, add `connected_full()`, and update `build_connect_message()` in `server.rs`.

## Pull Request Workflow

- Branch from `main` and use a descriptive branch name.
- Use [Conventional Commits](https://www.conventionalcommits.org/) messages (for example `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`).
- Keep changes surgical: touch only what the task requires and match the surrounding style.
- Run `make checkall` before pushing. Fix all lint, type, and test failures.
- Do not push or open a PR unless the maintainer requests it.
- Keep the Python and Rust sides in sync (see the sync rules above).
- Keep sister projects (`par-term-emu-tui-rust`, `par-term`) in mind when changing shared CLI options, features, or config.

## Related Documentation

- [CLAUDE.md](CLAUDE.md) - Full build commands, architecture notes, and project conventions
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - Internal architecture with diagrams
- [docs/BUILDING.md](docs/BUILDING.md) - Detailed build and test instructions
- [docs/DOCUMENTATION_STYLE_GUIDE.md](docs/DOCUMENTATION_STYLE_GUIDE.md) - Documentation standards
- [docs/SECURITY.md](docs/SECURITY.md) - PTY security considerations
