.PHONY: help build build-release build-streaming dev-streaming test test-rust test-python clean install dev fmt lint check \
        examples examples-basic examples-pty examples-streaming examples-all setup-venv watch \
        fmt-python lint-python checkall pre-commit-install pre-commit-uninstall \
        pre-commit-run pre-commit-update deploy \
        web-install web-dev web-build web-build-static web-start web-clean web-open \
        streamer-build streamer-build-release streamer-run streamer-run-auth streamer-run-http streamer-run-macro streamer-install

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
	@echo "  build            - Build the library in development mode (debug)"
	@echo "  build-release    - Build the library in development mode (release)"
	@echo "  build-streaming  - Build with streaming feature (debug)"
	@echo "  dev-streaming    - Build with streaming feature (release, for dev)"
	@echo "  watch            - Auto-rebuild on file changes (requires cargo-watch)"
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
	@echo "  examples           - Run basic terminal examples"
	@echo "  examples-pty       - Run PTY/shell examples"
	@echo "  examples-streaming - Run streaming demo (requires streaming feature)"
	@echo "  examples-all       - Run all examples (basic + PTY)"
	@echo ""
	@echo "Streaming Server (Standalone Rust Binary):"
	@echo "  streamer-build        - Build streaming server binary (debug)"
	@echo "  streamer-build-release - Build streaming server binary (release)"
	@echo "  streamer-run          - Build and run streaming server (WebSocket only)"
	@echo "  streamer-run-auth     - Build and run with authentication (API key: test-key)"
	@echo "  streamer-run-http     - Build and run with HTTP server (serves web_term)"
	@echo "  streamer-run-macro    - Build and run with macro playback demo"
	@echo "  streamer-install      - Install streaming server to ~/.cargo/bin"
	@echo ""
	@echo "Web Frontend (Next.js):"
	@echo "  web-install     - Install web frontend dependencies"
	@echo "  web-dev         - Start dev server and open in browser"
	@echo "  web-build       - Build web frontend for production (Next.js server)"
	@echo "  web-build-static - Build and copy static frontend to web_term/"
	@echo "  web-start       - Start production server"
	@echo "  web-clean       - Clean web frontend build artifacts"
	@echo ""
	@echo "Deployment:"
	@echo "  deploy          - Trigger GitHub 'Build and Deploy' workflow"
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

build-streaming:
	@echo "Building library with streaming feature (debug mode)..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv run maturin develop --features streaming

dev-streaming:
	@echo "Building library with streaming feature (release mode)..."
	@if [ ! -d ".venv" ]; then \
		echo "Warning: .venv not found. Run 'make setup-venv' first."; \
		exit 1; \
	fi
	uv sync
	uv run maturin develop --release --features streaming
	@echo ""
	@echo "======================================================================"
	@echo "  Streaming feature enabled!"
	@echo "======================================================================"
	@echo ""
	@echo "You can now run the streaming demo:"
	@echo "  make examples-streaming"
	@echo ""
	@echo "Or manually:"
	@echo "  python examples/streaming_demo.py"
	@echo ""
	@echo "Then open examples/streaming_client.html in your browser"
	@echo ""

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
	cargo test --lib --no-default-features --features pyo3/auto-initialize

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
	cargo clippy --all-targets --all-features --fix --allow-dirty --allow-staged -- -D warnings
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

examples-streaming: dev-streaming
	@echo "======================================================================"
	@echo "  Running Terminal Streaming Demo"
	@echo "======================================================================"
	@echo ""
	@echo "Starting WebSocket streaming server..."
	@echo ""
	@echo "Once the server starts, open examples/streaming_client.html"
	@echo "in your web browser and connect to ws://localhost:8080"
	@echo ""
	@echo "Press Ctrl+C to stop the server"
	@echo ""
	uv run python examples/streaming_demo.py

examples-all: examples-basic examples-pty
	@echo ""
	@echo "======================================================================"
	@echo "  All examples completed!"
	@echo "======================================================================"

# ============================================================================
# Streaming Server (Standalone Rust Binary)
# ============================================================================

streamer-build:
	@echo "======================================================================"
	@echo "  Building Streaming Server (Debug)"
	@echo "======================================================================"
	@echo ""
	cargo build --bin par-term-streamer --no-default-features --features streaming
	@echo ""
	@echo "Binary built: target/debug/par-term-streamer"
	@echo ""

streamer-build-release:
	@echo "======================================================================"
	@echo "  Building Streaming Server (Release)"
	@echo "======================================================================"
	@echo ""
	cargo build --bin par-term-streamer --no-default-features --features streaming --release
	@echo ""
	@echo "Binary built: target/release/par-term-streamer"
	@echo ""

streamer-run: streamer-build-release
	@echo "======================================================================"
	@echo "  Running Streaming Server"
	@echo "======================================================================"
	@echo ""
	@echo "Starting server on ws://127.0.0.1:8099"
	@echo "Open examples/streaming_client.html or run 'make web-dev' in browser"
	@echo ""
	@echo "Press Ctrl+C to stop"
	@echo ""
	./target/release/par-term-streamer --port 8099 --theme iterm2-dark

streamer-run-auth: streamer-build-release
	@echo "======================================================================"
	@echo "  Running Streaming Server with Authentication"
	@echo "======================================================================"
	@echo ""
	@echo "Starting server on ws://127.0.0.1:8099"
	@echo "Authentication: ENABLED"
	@echo "API Key: test-key"
	@echo ""
	@echo "Connect with:"
	@echo "  - Header: Authorization: Bearer test-key"
	@echo "  - URL: ws://127.0.0.1:8099?api_key=test-key"
	@echo ""
	@echo "Press Ctrl+C to stop"
	@echo ""
	./target/release/par-term-streamer --port 8099 --theme iterm2-dark --api-key test-key

streamer-run-http: streamer-build-release
	@echo "======================================================================"
	@echo "  Running Streaming Server with HTTP Support"
	@echo "======================================================================"
	@echo ""
	@if [ ! -d "web_term" ]; then \
		echo "Error: web_term directory not found. Run 'make web-build-static' first."; \
		exit 1; \
	fi
	@echo "Starting server on http://127.0.0.1:8099"
	@echo "HTTP Server: ENABLED"
	@echo "Web Root: ./web_term"
	@echo ""
	@echo "Open your browser to:"
	@echo "  http://127.0.0.1:8099"
	@echo ""
	@echo "WebSocket endpoint:"
	@echo "  ws://127.0.0.1:8099/ws"
	@echo ""
	@echo "Press Ctrl+C to stop"
	@echo ""
	./target/release/par-term-streamer --port 8099 --theme iterm2-dark --enable-http --web-root ./web_term

streamer-run-macro: streamer-build-release
	@echo "======================================================================"
	@echo "  Running Streaming Server with Macro Playback"
	@echo "======================================================================"
	@echo ""
	@if [ ! -f "examples/demo.yaml" ]; then \
		echo "Error: examples/demo.yaml not found."; \
		echo "Create a macro file first or use a different path."; \
		exit 1; \
	fi
	@echo "Starting server on ws://127.0.0.1:8099"
	@echo "Mode: MACRO PLAYBACK"
	@echo "Macro file: examples/demo.yaml"
	@echo ""
	@echo "The server will play back the macro in a loop."
	@echo "Connect with a WebSocket client or open examples/streaming_client.html"
	@echo ""
	@echo "Press Ctrl+C to stop"
	@echo ""
	./target/release/par-term-streamer --port 8099 --theme iterm2-dark --macro-file examples/demo.yaml --macro-loop

streamer-install: streamer-build-release
	@echo "======================================================================"
	@echo "  Installing Streaming Server"
	@echo "======================================================================"
	@echo ""
	cargo install --path . --bin par-term-streamer --no-default-features --features streaming --force
	@echo ""
	@echo "======================================================================"
	@echo "  Installation Complete!"
	@echo "======================================================================"
	@echo ""
	@echo "The streaming server has been installed to ~/.cargo/bin"
	@echo ""
	@echo "Run it with:"
	@echo "  par-term-streamer --help"
	@echo "  par-term-streamer --port 8080"
	@echo ""

# ============================================================================
# Web Frontend (Next.js)
# ============================================================================

web-install:
	@echo "======================================================================"
	@echo "  Installing Web Frontend Dependencies"
	@echo "======================================================================"
	@echo ""
	@if [ ! -d "web-terminal-frontend" ]; then \
		echo "Error: web-terminal-frontend directory not found!"; \
		exit 1; \
	fi
	cd web-terminal-frontend && npm install
	@echo ""
	@echo "Dependencies installed successfully!"
	@echo ""

web-dev: web-install
	@echo "======================================================================"
	@echo "  Starting Web Frontend Development Server"
	@echo "======================================================================"
	@echo ""
	@echo "Dev server will start at http://localhost:3000"
	@echo "Opening browser in 3 seconds..."
	@echo ""
	@echo "Make sure the streaming server is running:"
	@echo "  make examples-streaming"
	@echo ""
	@(sleep 3 && (command -v xdg-open > /dev/null && xdg-open http://localhost:3000 || \
	              command -v open > /dev/null && open http://localhost:3000 || \
	              echo "Please open http://localhost:3000 in your browser")) &
	cd web-terminal-frontend && npm run dev

web-build: web-install
	@echo "======================================================================"
	@echo "  Building Web Frontend for Production"
	@echo "======================================================================"
	@echo ""
	cd web-terminal-frontend && npm run build
	@echo ""
	@echo "======================================================================"
	@echo "  Production build complete!"
	@echo "======================================================================"
	@echo ""
	@echo "Start the production server with:"
	@echo "  make web-start"
	@echo ""

web-build-static: web-install
	@echo "======================================================================"
	@echo "  Building Static Web Frontend for HTTP Server"
	@echo "======================================================================"
	@echo ""
	@echo "Building static export..."
	cd web-terminal-frontend && npm run build
	@echo ""
	@echo "Copying to web_term/..."
	rm -rf web_term
	mkdir -p web_term
	cp -r web-terminal-frontend/out/* web_term/
	@echo ""
	@echo "======================================================================"
	@echo "  Static build complete!"
	@echo "======================================================================"
	@echo ""
	@echo "Files copied to: web_term/"
	@echo ""
	@echo "Run the streaming server with HTTP support:"
	@echo "  make streamer-run-http"
	@echo ""

web-start:
	@echo "======================================================================"
	@echo "  Starting Web Frontend Production Server"
	@echo "======================================================================"
	@echo ""
	@if [ ! -d "web-terminal-frontend/.next" ]; then \
		echo "Error: Production build not found. Run 'make web-build' first."; \
		exit 1; \
	fi
	@echo "Production server will start at http://localhost:3000"
	@echo "Opening browser in 3 seconds..."
	@echo ""
	@(sleep 3 && (command -v xdg-open > /dev/null && xdg-open http://localhost:3000 || \
	              command -v open > /dev/null && open http://localhost:3000 || \
	              echo "Please open http://localhost:3000 in your browser")) &
	cd web-terminal-frontend && npm run start

web-open:
	@echo "Opening web frontend in browser..."
	@(command -v xdg-open > /dev/null && xdg-open http://localhost:3000 || \
	  command -v open > /dev/null && open http://localhost:3000 || \
	  echo "Please open http://localhost:3000 in your browser")

web-clean:
	@echo "Cleaning web frontend build artifacts..."
	cd web-terminal-frontend && rm -rf .next out node_modules .turbo
	rm -rf web_term
	@echo "Web frontend clean complete!"

# ============================================================================
# Deployment
# ============================================================================

deploy:
	@echo "======================================================================"
	@echo "  Triggering GitHub 'Build and Deploy' workflow"
	@echo "======================================================================"
	@echo ""
	@if ! command -v gh > /dev/null; then \
		echo "Error: GitHub CLI (gh) not found. Install it from:"; \
		echo "  https://cli.github.com/"; \
		exit 1; \
	fi
	gh workflow run deployment.yml
	@echo ""
	@echo "Workflow triggered successfully!"
	@echo "Monitor progress at:"
	@echo "  https://github.com/paulrobello/par-term-emu-core-rust/actions"
	@echo ""
	@echo "Or use: gh run list --workflow=deployment.yml"
	@echo ""

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
	@if [ -d "web-terminal-frontend" ]; then \
		echo "Cleaning web frontend..."; \
		cd web-terminal-frontend && rm -rf .next out node_modules .turbo; \
	fi
	rm -rf web_term
	@echo "Clean complete!"
