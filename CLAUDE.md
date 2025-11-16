# CLAUDE.md

Guidance for Claude Code when working with this repository.

## Project Overview

Rust terminal emulator library with Python 3.12+ bindings (PyO3). Provides VT100/VT220/VT320/VT420 compatibility with PTY support.

**Sister Project**: `../par-term-emu-tui-rust` - Full-featured TUI application ([GitHub](https://github.com/paulrobello/par-term-emu-tui-rust), [PyPI](https://pypi.org/project/par-term-emu-tui-rust/)). Keep config files, CLI options, and features in sync.

## Quick Start

```bash
make setup-venv  # Initial setup
make dev         # Build after Rust changes
make checkall    # All quality checks (run before commits)
```

## Key Principles

1. **Build System**: Always use `uv` for Python (never pip). Use `make dev` not `cargo build` for PyO3 modules.
2. **Architecture**: Modular design - see `docs/ARCHITECTURE.md` for component details
3. **Testing**: Run `make checkall` before commits. Install hooks: `make pre-commit-install`
4. **PyO3 Module**: Must match across `pyproject.toml`, `src/lib.rs`, and `python/par_term_emu_core_rust/__init__.py`

## Development Workflows

### Adding ANSI Sequences
1. Add handler in `src/terminal/sequences/{csi,osc,esc,dcs}.rs`
2. Implement grid/cursor changes if needed
3. Add tests (Rust + Python)
4. VT parameter 0 or missing defaults to 1

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

- **Threading**: Never hold mutex while calling Python (GIL deadlock)
- **Unicode**: Use `unicode-width` crate (wide chars = 2 cols)
- **Bounds**: Validate col/row before grid access
- **Mouse**: VT coords are 1-indexed, internal are 0-indexed
- **Python Version**: Requires Python 3.12+

## Resources

- **README.md** - API documentation
- **docs/ARCHITECTURE.md** - Internal architecture details
- **docs/SECURITY.md** - PTY security considerations
- **docs/DOCUMENTATION_STYLE_GUIDE.md** - Documentation standards
- [PyO3 guide](https://pyo3.rs/) - Python bindings reference
- [xterm sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - VT spec
- never push unless the user requests it
- always run `make checkall` and fix all isues before pushing
- always add tests and update documentation for new features
