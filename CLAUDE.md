# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rust terminal emulator library with Python 3.12+ bindings (PyO3). Provides VT100/VT220/VT320/VT420/VT520 compatibility with PTY support, streaming server, screenshot rendering, and graphics protocols (Sixel, iTerm2, Kitty).

**Sister Projects** (keep config files, CLI options, and features in sync):
- `../par-term-emu-tui-rust` - Python TUI application ([GitHub](https://github.com/paulrobello/par-term-emu-tui-rust), [PyPI](https://pypi.org/project/par-term-emu-tui-rust/))
- `../par-term` - Rust terminal frontend ([GitHub](https://github.com/probello/par-term))

## Build Commands

```bash
make setup-venv          # Initial setup: creates .venv, installs deps
make dev                 # Build library (release mode via maturin) - USE THIS, not cargo build
make dev-streaming       # Build with streaming feature enabled
make checkall            # All quality checks (run before every commit)
```

**Never use `cargo build` directly** for PyO3 modules - it will fail at link stage. Always use `make dev` (maturin).

### Running Tests

```bash
make test                # Run all tests (Rust + Python)
make test-rust           # Rust tests only
make test-python         # Python tests only (rebuilds first)

# Single Rust test
cargo test --lib --no-default-features --features pyo3/auto-initialize test_name

# Single Python test file
uv run pytest tests/test_terminal.py -v

# Single Python test function
uv run pytest tests/test_terminal.py::test_function_name -v

# Rust streaming tests
cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming test_name
```

The `--no-default-features --features pyo3/auto-initialize` flags are required because:
- Default feature enables `extension-module` which prevents linking during tests
- `auto-initialize` bootstraps the Python interpreter for Rust test environment

### Code Quality

```bash
make lint                # Rust clippy + fmt (auto-fix)
make lint-python         # Python ruff format + check + pyright
make fmt                 # Rust format only
make fmt-python          # Python format only
```

### Streaming Server & Web Frontend

```bash
make streamer-run        # Build + run streaming server on ws://127.0.0.1:8099
make streamer-run-http   # With HTTP server serving web_term/
make web-build-static    # Build Next.js frontend → web_term/ (run after frontend changes)

# Proto regeneration (after editing proto/terminal.proto)
make proto-rust          # Generate terminal.pb.rs
make proto-typescript    # Generate TypeScript proto code
make web-build-static    # Rebuild frontend with new proto
```

## Architecture

### Crate Structure

The crate produces three artifacts:
- **Python extension** (`cdylib`): Built via maturin, exposes `par_term_emu_core_rust._native` module
- **Rust library** (`rlib`): For use by other Rust projects (e.g., `par-term`)
- **Streaming server binary** (`par-term-streamer`): Requires `streaming` feature flag

### Feature Flags

| Feature | Purpose |
|---------|---------|
| `python` (default) | PyO3 bindings with `extension-module` |
| `streaming` | WebSocket server, protobuf, TLS, HTTP, CLI deps |
| `rust-only` | No Python bindings |
| `full` | `python` + `streaming` |
| `regenerate-proto` | Rebuild protobuf from `proto/terminal.proto` |
| `jemalloc` | Better server performance (non-Windows) |

### Data Flow

```
Input bytes → VTE Parser → Perform trait callbacks → Terminal state (Grid/Cursor) → Python API queries
                           (src/terminal/sequences/)   (src/terminal/mod.rs)        (src/python_bindings/)
```

### Key Source Layout

- `src/terminal/mod.rs` - Main `Terminal` struct with all state
- `src/terminal/sequences/{csi,osc,esc,dcs}.rs` - VTE escape sequence handlers
- `src/terminal/write.rs` - Character writing logic
- `src/terminal/trigger.rs` - Regex-based output pattern matching
- `src/grid.rs` - 2D terminal buffer with scrollback (flat Vec, row-major)
- `src/pty_session.rs` - PTY session with background reader thread
- `src/python_bindings/` - PyO3 wrappers (`terminal.rs`, `pty.rs`, `streaming.rs`, `types.rs`, `enums.rs`)
- `src/streaming/` - WebSocket streaming protocol
- `src/screenshot/` - Terminal-to-image rendering (embedded JetBrains Mono + Noto Emoji fonts)
- `src/graphics/` - Unified Sixel/iTerm2/Kitty graphics (all normalized to `TerminalGraphic` with RGBA)
- `src/lib.rs` - Module declarations, re-exports, and `_native` PyO3 module registration

### Streaming Protocol Layers

When modifying the streaming protocol, changes flow through 3 layers:

1. **`proto/terminal.proto`** → `src/streaming/terminal.pb.rs` (generated, don't edit directly)
2. **`src/streaming/protocol.rs`** - App-level types (`ServerMessage`, `ClientMessage`, `EventType` enums)
3. **`src/streaming/proto.rs`** - Conversion between app types ↔ protobuf wire format

Also update:
- `src/python_bindings/streaming.rs` - Dict conversion + event type matching
- `tests/test_streaming.rs` - Integration tests (use `..` in destructuring for forward compat)
- `src/streaming/server.rs` - `build_connect_message()` helper when extending `Connected`

### PTY Architecture

`PtySession` wraps `Arc<Mutex<Terminal>>` (using `parking_lot::Mutex` for performance/no poisoning). A background reader thread reads from the PTY master and calls `term.process(..)` while holding the lock. The `running` flag (`Arc<AtomicBool>`) is best-effort; use `try_wait()`/`wait()` for precise exit status.

## Development Workflows

### Adding ANSI Sequences
1. Add handler in `src/terminal/sequences/{csi,osc,esc,dcs}.rs`
2. Implement grid/cursor changes if needed
3. Add tests (Rust + Python)
4. VT parameter 0 or missing defaults to 1

### Adding Streaming Message Types
1. Update `src/streaming/protocol.rs` (enum variant + constructors)
2. Update `src/streaming/proto.rs` (both conversion directions)
3. Update `src/python_bindings/streaming.rs` (dict conversion + event type match)
4. Update `tests/test_streaming.rs`
5. If extending `Connected`: update all existing constructors, add `connected_full()`, update `build_connect_message()` in `server.rs`

### Adding PTY Features
1. Modify `PtySession` in `src/pty_session.rs`
2. Add Python wrapper in `src/python_bindings/pty.rs`
3. Ensure thread safety (Arc/Mutex or atomics)
4. Update generation counter for state changes

### Python API Conventions
- Return tuples: `(col, row)` for coordinates, `(r, g, b)` for colors
- Return `None` for invalid positions (no exceptions)
- Keep logic in Rust, Python wrappers thin

## Critical Reminders

- **Build**: Never use `cargo build` for PyO3 modules; use `make dev` (maturin)
- **Threading**: Never hold mutex while calling Python (GIL deadlock)
- **Unicode**: Use `unicode-width` crate (wide chars = 2 cols)
- **Bounds**: Validate col/row before grid access
- **Mouse**: VT coords are 1-indexed, internal are 0-indexed
- **Terminal events**: Queued in `terminal_events` vec, polled via `poll_events()` which takes/clears them

## Workflow Rules

- Never push unless the user requests it
- Always run `make checkall` and fix all issues before pushing
- Always add tests and update documentation for new features
- Always ensure the Python bindings are in sync with method signatures
- When bumping project version make sure CHANGELOG.md is updated
- Note breaking changes in changelog and README "What's New" sections
- Always run `make web-build-static` after updating the web terminal frontend

### Version and Python Binding Sync (CRITICAL)

**Version Sync**: When bumping version, update ALL of these files to the same version:
- `Cargo.toml` (line 3: `version = "X.Y.Z"`)
- `pyproject.toml` (line 9: `version = "X.Y.Z"`)
- `python/par_term_emu_core_rust/__init__.py` (`__version__ = "X.Y.Z"`)

**Python Binding Sync**: When adding/modifying Rust methods on `Terminal` or `PtySession`:
1. **Add Python binding** in `src/python_bindings/terminal.rs` or `src/python_bindings/pty.rs`
2. **Add docstrings** with Args, Returns, and Example sections (Google style)
3. **Update API docs** in `docs/API_REFERENCE.md`
4. **Update README.md** if it's a user-facing feature
5. **Add Python tests** in `tests/` if the feature is testable from Python

Files that must stay in sync:
- Rust impl (`src/terminal/mod.rs`) ↔ Python binding (`src/python_bindings/terminal.rs`)
- Rust impl (`src/pty_session.rs`) ↔ Python binding (`src/python_bindings/pty.rs`)
- Python binding ↔ API Reference (`docs/API_REFERENCE.md`)

## Resources

- **docs/ARCHITECTURE.md** - Detailed internal architecture with diagrams
- **docs/SECURITY.md** - PTY security considerations
- **docs/DOCUMENTATION_STYLE_GUIDE.md** - Documentation standards
- [PyO3 guide](https://pyo3.rs/) - Python bindings reference
- [xterm sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - VT spec
