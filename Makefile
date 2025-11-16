.PHONY: help build build-release test test-rust test-python clean install dev fmt lint check \
        examples examples-basic examples-pty examples-all setup-venv watch \
        fmt-python lint-python checkall pre-commit-install pre-commit-uninstall \
        pre-commit-run pre-commit-update

help:
	@echo "==================================================================="
	@echo "  par-term-emu-core-rust Makefile"
	@echo "==================================================================="
	@echo ""
	@echo "  Rust terminal emulator library with Python bindings"
	@echo ""
	@echo "==================================================================="
	@echo ""
	@echo "Setup & Installation:"
	@echo "  setup-venv      - Create virtual environment and install tools"
	@echo "  dev             - Install library in development mode (release)"
	@echo "  install         - Build and install the package"
	@echo ""
	@echo "Building:"
	@echo "  build           - Build the library in development mode (debug)"
	@echo "  build-release   - Build the library in development mode (release)"
	@echo "  watch           - Auto-rebuild on file changes (requires cargo-watch)"
	@echo ""
	@echo "Testing:"
	@echo "  test            - Run all tests (Rust + Python)"
	@echo "  test-rust       - Run Rust tests only"
	@echo "  test-python     - Run Python tests only"
	@echo ""
	@echo "Code Quality:"
	@echo "  fmt             - Format Rust code"
	@echo "  fmt-python      - Format Python code with ruff"
	@echo "  lint            - Run Rust linters (clippy + fmt, auto-fix)"
	@echo "  lint-python     - Run Python linters (format + ruff + pyright, auto-fix)"
	@echo "  check           - Check Rust code without building"
	@echo "  checkall        - Run ALL checks: tests, format, lint, typecheck (auto-fix all)"
	@echo ""
	@echo "Pre-commit Hooks:"
	@echo "  pre-commit-install   - Install pre-commit hooks"
	@echo "  pre-commit-uninstall - Uninstall pre-commit hooks"
	@echo "  pre-commit-run       - Run pre-commit on all files"
	@echo "  pre-commit-update    - Update pre-commit hook versions"
	@echo ""
	@echo "Examples:"
	@echo "  examples        - Run basic terminal examples"
	@echo "  examples-pty    - Run PTY/shell examples"
	@echo "  examples-all    - Run all examples (basic + PTY)"
	@echo ""
	@echo "Cleanup:"
	@echo "  clean           - Clean all build artifacts"
	@echo ""
	@echo "==================================================================="

# ============================================================================
# Setup & Installation
# ============================================================================

setup-venv:
	@echo "Creating virtual environment and syncing dependencies..."
	uv venv .venv
	uv sync --all-extras
	@echo ""
	@echo "Virtual environment created and dependencies synced!"
	@echo "Activate it with:"
	@echo "  source .venv/bin/activate"
	@echo ""
	@echo "Then run 'make dev' to build the library"

dev:
	@echo "Syncing dependencies and building library in development mode..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv sync
	uv run maturin develop --release

install:
	uv run maturin build --release
	uv pip install target/wheels/*.whl --force-reinstall

# ============================================================================
# Building
# ============================================================================

build:
	@echo "Building library in development mode..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv run maturin develop

build-release:
	@echo "Building library in release mode..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv run maturin develop --release

watch:
	@if ! command -v cargo-watch > /dev/null; then \
		echo "cargo-watch not found. Install with:"; \
		echo "  cargo install cargo-watch"; \
		exit 1; \
	fi
	cargo watch -x "build --release" -s "uv run maturin develop --release"

# ============================================================================
# Testing
# ============================================================================

test: test-rust test-python

test-rust:
	@echo "Running Rust tests..."
	cargo test --lib

test-python: dev
	@echo "Running Python tests..."
	uv run pytest tests/ -v

# ============================================================================
# Code Quality
# ============================================================================

fmt:
	@echo "Formatting Rust code..."
	cargo fmt

fmt-python:
	@echo "Formatting Python code..."
	uv run ruff format .

lint:
	@echo "Running Rust linters and auto-fixing issues..."
	cargo clippy --fix --allow-dirty --allow-staged -- -D warnings
	cargo fmt

lint-python:
	@echo "Running Python linters and auto-fixing issues..."
	uv run ruff format .
	uv run ruff check --fix .
	uv run pyright .

check:
	@echo "Checking Rust code..."
	cargo check

checkall: test-rust lint lint-python test-python
	@echo ""
	@echo "======================================================================"
	@echo "  All code quality checks passed!"
	@echo "======================================================================"
	@echo ""
	@echo "Summary:"
	@echo "  ✓ Rust tests"
	@echo "  ✓ Rust format (auto-fixed)"
	@echo "  ✓ Rust lint (clippy auto-fixed)"
	@echo "  ✓ Python format (auto-fixed)"
	@echo "  ✓ Python lint (ruff auto-fixed)"
	@echo "  ✓ Python type check (pyright)"
	@echo "  ✓ Python tests"
	@echo ""

# ============================================================================
# Pre-commit Hooks
# ============================================================================

pre-commit-install:
	@echo "Installing pre-commit hooks..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv sync
	uv run pre-commit install
	@echo ""
	@echo "======================================================================"
	@echo "  Pre-commit hooks installed successfully!"
	@echo "======================================================================"
	@echo ""
	@echo "Hooks will now run automatically on 'git commit'."
	@echo "To run hooks manually: make pre-commit-run"
	@echo "To skip hooks on commit: git commit --no-verify"
	@echo ""

pre-commit-uninstall:
	@echo "Uninstalling pre-commit hooks..."
	uv run pre-commit uninstall
	@echo "Pre-commit hooks uninstalled."

pre-commit-run:
	@echo "Running pre-commit on all files..."
	uv run pre-commit run --all-files

pre-commit-update:
	@echo "Updating pre-commit hook versions..."
	uv run pre-commit autoupdate
	@echo ""
	@echo "Hook versions updated. Review changes in .pre-commit-config.yaml"

# ============================================================================
# Examples
# ============================================================================

examples: examples-basic

examples-basic: dev
	@echo "======================================================================"
	@echo "  Running Basic Terminal Examples"
	@echo "======================================================================"
	@echo ""
	@echo "Running basic_usage_improved.py..."
	uv run python examples/basic_usage_improved.py
	@echo ""
	@echo "Running colors_demo.py..."
	uv run python examples/colors_demo.py
	@echo ""
	@echo "Running cursor_movement.py..."
	uv run python examples/cursor_movement.py
	@echo ""
	@echo "Running scrollback_demo.py..."
	uv run python examples/scrollback_demo.py
	@echo ""
	@echo "Running text_attributes.py..."
	uv run python examples/text_attributes.py
	@echo ""
	@echo "======================================================================"
	@echo "  Basic examples completed!"
	@echo "======================================================================"

examples-pty: dev
	@echo "======================================================================"
	@echo "  Running PTY/Shell Examples"
	@echo "======================================================================"
	@echo ""
	@echo "Running pty_basic.py..."
	uv run python examples/pty_basic.py
	@echo ""
	@echo "Running pty_shell.py..."
	uv run python examples/pty_shell.py
	@echo ""
	@echo "Running pty_custom_env.py..."
	uv run python examples/pty_custom_env.py
	@echo ""
	@echo "Running pty_resize.py..."
	uv run python examples/pty_resize.py
	@echo ""
	@echo "Running pty_multiple.py..."
	uv run python examples/pty_multiple.py
	@echo ""
	@echo "Running pty_event_loop.py..."
	uv run python examples/pty_event_loop.py
	@echo ""
	@echo "Running pty_mouse_events.py..."
	uv run python examples/pty_mouse_events.py
	@echo ""
	@echo "======================================================================"
	@echo "  PTY examples completed!"
	@echo "======================================================================"

examples-all: examples-basic examples-pty
	@echo ""
	@echo "======================================================================"
	@echo "  All examples completed!"
	@echo "======================================================================"

# ============================================================================
# Cleanup
# ============================================================================

clean:
	@echo "Cleaning build artifacts..."
	cargo clean
	rm -rf target/
	rm -rf dist/
	rm -rf build/
	rm -rf *.egg-info
	find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
	find . -type f -name "*.pyc" -delete
	find . -type f -name "*.so" -delete
	@echo "Clean complete!"
