# Building par-term-emu-core-rust

This guide explains how to build and install the par-term-emu-core-rust library (version 0.9.0).

## Table of Contents

- [Prerequisites](#prerequisites)
  - [Rust](#rust)
  - [Python](#python)
  - [uv Package Manager](#uv-package-manager)
- [Building from Source](#building-from-source)
  - [Quick Start](#quick-start)
  - [Development Build](#development-build)
  - [Production Build](#production-build)
  - [Auto-rebuild on Changes](#auto-rebuild-on-changes)
- [Running Tests](#running-tests)
  - [Rust Tests](#rust-tests)
  - [Python Tests](#python-tests)
  - [Code Quality Checks](#code-quality-checks)
  - [Pre-commit Hooks](#pre-commit-hooks)
- [Running Examples](#running-examples)
- [Cross-Compilation](#cross-compilation)
  - [Linux](#linux)
  - [macOS](#macos)
  - [Windows](#windows)
- [Publishing to PyPI](#publishing-to-pypi)
- [Troubleshooting](#troubleshooting)
- [Docker Build](#docker-build)
- [Related Documentation](#related-documentation)

## Prerequisites

### Rust

You need Rust 1.75 or later. The project is currently tested with Rust 1.91.1. Install Rust from [rustup.rs](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Python

You need Python 3.12 or later. The project officially supports Python 3.12, 3.13, and 3.14. Check your version:

```bash
python --version
```

### uv Package Manager

**This project uses `uv` for Python package management.** Install it from [astral.sh](https://astral.sh/):

```bash
# macOS/Linux
curl -LsSf https://astral.sh/uv/install.sh | sh

# Windows
powershell -c "irm https://astral.sh/uv/install.ps1 | iex"
```

**Do not use `pip` directly** - always use `uv` commands as shown throughout this guide.

## Building from Source

### Quick Start

For first-time setup:

```bash
# Create virtual environment and install all dependencies
make setup-venv

# Build in release mode and install
make dev
```

The `setup-venv` target creates a `.venv` directory and syncs all development dependencies from `pyproject.toml`, including:
- **maturin** (≥1.9, <2.0 for build; ≥1.10.1 for dev) - Build tool for PyO3 projects
- **pytest** (≥9.0.1) and **pytest-timeout** (≥2.4.0) - Testing framework
- **ruff** (≥0.14.5) - Fast Python linter and formatter
- **pyright** (≥1.1.407) - Static type checker
- **pre-commit** (≥4.4.0) - Git hook framework

### Development Build

For development, use `uv run maturin develop` to build and install the package in editable mode:

```bash
# Debug build (faster compilation, slower runtime)
uv run maturin develop

# Release build (slower compilation, faster runtime) - RECOMMENDED
uv run maturin develop --release

# Or use the make target
make dev
```

This installs the package in your virtual environment, allowing you to import it:

```python
from par_term_emu_core_rust import Terminal
```

### Production Build

To create a wheel for distribution:

```bash
uv run maturin build --release

# Or use the make target
make build-release
```

The wheel will be created in `target/wheels/`.

Install it with:

```bash
uv add --find-links target/wheels par-term-emu-core-rust
# or
pip install target/wheels/par_term_emu_core_rust-*.whl
```

### Auto-rebuild on Changes

For faster development, use `cargo-watch` to automatically rebuild when files change:

```bash
# Install cargo-watch (one-time setup)
cargo install cargo-watch

# Watch for changes and rebuild
make watch
```

> **📝 Note:** The `watch` target automatically rebuilds and reinstalls the package whenever Rust source files change.

## Running Tests

The project includes comprehensive test coverage:
- **Rust unit tests** covering all modules and features
- **Python integration tests** covering VT sequences, grid operations, PTY sessions, screenshots, and Python bindings
- Tests are organized into separate test files for different feature areas

### Rust Tests

Run the Rust unit tests with the correct PyO3 feature flags:

```bash
# Correct command (required for PyO3 compatibility)
cargo test --lib --no-default-features --features pyo3/auto-initialize

# Or use the make target
make test-rust
```

> **⚠️ Important:** The simple `cargo test` command will fail due to PyO3's `extension-module` feature. Tests require the `auto-initialize` feature instead. The Makefile target handles this automatically.

**Why different features?**
- **Production builds** use `pyo3/extension-module` (configured in `pyproject.toml` - tells linker NOT to link against libpython)
- **Rust tests** use `pyo3/auto-initialize` (configured in `dev-dependencies` - initializes Python interpreter for testing)

### Python Tests

First, install the package in development mode, then run pytest:

```bash
make dev
make test-python

# Or manually
uv run maturin develop --release
uv run pytest tests/ -v
```

**Test configuration:**
- Default timeout: 5 seconds per test (configured in `pyproject.toml`)
- Pytest warning filters suppress expected PyPtyTerminal unsendable warnings
- Some PTY tests may need special handling in CI environments due to signal handling

### Code Quality Checks

Run all code quality checks (format, lint, type check, tests):

```bash
# Run all checks with auto-fix
make checkall

# Individual checks
make fmt              # Format Rust code
make fmt-python       # Format Python code
make lint             # Lint Rust code (clippy)
make lint-python      # Lint and type-check Python code
```

### Pre-commit Hooks

Install pre-commit hooks to automatically run checks before each commit:

```bash
# Install hooks
make pre-commit-install

# Run hooks manually on all files
make pre-commit-run

# Update hook versions
make pre-commit-update

# Uninstall hooks
make pre-commit-uninstall
```

**Pre-commit hooks include:**
- **File checks**: Trailing whitespace, end-of-file fixer, YAML/TOML validation, merge conflicts
- **Rust**: `cargo fmt`, `cargo clippy`, `cargo test` (with PyO3 auto-initialize)
- **Python**: `ruff format`, `ruff check --fix`, `pyright`, `pytest`

The pytest hook runs `uv sync && maturin develop && uv run pytest tests/ -v` to ensure the package is built before running tests.

> **📝 Note:** Pre-commit hooks will run automatically on `git commit`. To skip hooks temporarily, use `git commit --no-verify`.

## Running Examples

After installing the package, run the example scripts:

```bash
# Run all examples (basic + PTY examples)
make examples-all

# Run only basic terminal examples
make examples-basic

# Run only PTY/shell examples
make examples-pty

# Or run individual examples with uv
uv run python examples/basic_usage_improved.py
uv run python examples/colors_demo.py
uv run python examples/cursor_movement.py
uv run python examples/scrollback_demo.py
uv run python examples/text_attributes.py
uv run python examples/screenshot_demo.py
uv run python examples/pty_basic.py
uv run python examples/pty_shell.py
# ... and many more in the examples/ directory
```

> **📝 Note:** The project includes 33 example scripts demonstrating various features including:
> - **Basic Terminal**: colors, cursor movement, scrollback, text attributes, rectangle operations, alt screen, Unicode/emoji
> - **PTY/Shell**: basic PTY, shell sessions, resize, custom environments, multiple PTYs, event loops, mouse events
> - **Graphics**: Sixel display and rendering, image display
> - **Advanced Features**: mouse tracking, hyperlinks, notifications, shell integration, bracketed paste, synchronized updates
> - **Testing**: underline styles, keyboard protocols, clipboard (OSC 52), TUI integration
> - **Streaming**: WebSocket streaming demo (requires streaming feature)
> See the `examples/` directory for the complete list.

## Cross-Compilation

Maturin supports cross-compilation for different platforms:

### Linux

```bash
# For x86_64
uv run maturin build --release --target x86_64-unknown-linux-gnu

# For aarch64 (ARM64)
uv run maturin build --release --target aarch64-unknown-linux-gnu
```

### macOS

```bash
# For x86_64 (Intel)
uv run maturin build --release --target x86_64-apple-darwin

# For aarch64 (Apple Silicon)
uv run maturin build --release --target aarch64-apple-darwin

# Universal binary (both architectures)
uv run maturin build --release --universal2
```

### Windows

```bash
# For x86_64
uv run maturin build --release --target x86_64-pc-windows-msvc
```

> **📝 Note:** Cross-compilation may require additional toolchains. See [CROSS_PLATFORM.md](CROSS_PLATFORM.md) for detailed setup instructions.

## Publishing to PyPI

To publish the package to PyPI:

```bash
# Build wheels for the current platform
uv run maturin build --release

# Upload to PyPI (requires PyPI credentials)
uv run maturin publish

# Or build for multiple platforms and upload
uv run maturin build --release --target x86_64-unknown-linux-gnu
uv run maturin build --release --target aarch64-apple-darwin
uv run maturin publish
```

> **📝 Note:** You'll need to configure PyPI credentials first. Use `maturin publish --help` for authentication options.

## Troubleshooting

### Error: "cannot find -lpython3.x"

Make sure Python development headers are installed:

**Ubuntu/Debian:**
```bash
sudo apt-get install python3-dev
```

**Fedora/RHEL:**
```bash
sudo dnf install python3-devel
```

**macOS:**
```bash
# Install Python via Homebrew (3.12, 3.13, or 3.14 supported)
brew install python@3.14
```

### Error: "uv: command not found"

Install uv from [astral.sh](https://astral.sh/):

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

### Error: "no preset version for pyo3"

Make sure you're using a compatible Python version (3.12+):

```bash
python --version
```

### Slow Build Times

Use debug builds during development (faster compilation):

```bash
make build  # Debug build
```

Use release builds for testing performance or creating distributions:

```bash
make dev  # Release build (recommended for most development)
```

## Docker Build

> **📝 Note:** This project does not currently include a Dockerfile. If you need to build in a containerized environment, you can create a Dockerfile based on this example:

```dockerfile
FROM rust:latest

# Install system dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    python3-dev \
    && rm -rf /var/lib/apt/lists/*

# Install uv
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"

WORKDIR /build
COPY . .

# Sync dependencies and build
RUN uv sync
RUN uv run maturin build --release
```

Build and extract the wheel:

```bash
docker build -t par-term-emu-builder .
docker create --name builder par-term-emu-builder
docker cp builder:/build/target/wheels/ ./wheels/
docker rm builder
```

> **✅ Note:** Screenshot functionality uses pure Rust libraries (Swash for font rendering), so no external font library dependencies (FreeType, HarfBuzz) are required.

## Related Documentation

- [README.md](../README.md) - Project overview and API reference
- [QUICKSTART.md](../QUICKSTART.md) - Quick start guide for new users
- [CLAUDE.md](../CLAUDE.md) - Project development guide for contributors
- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture and design
- [CROSS_PLATFORM.md](CROSS_PLATFORM.md) - Cross-platform build instructions
- [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) - Configuration file reference
- [SECURITY.md](SECURITY.md) - Security considerations for PTY operations
- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced features documentation
- [VT_SEQUENCES.md](VT_SEQUENCES.md) - VT sequence reference and implementation status
- [Sister Project: par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust) - Full-featured TUI application
