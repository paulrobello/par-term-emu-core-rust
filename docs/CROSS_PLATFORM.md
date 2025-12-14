# Cross-Platform Compatibility Guide

This document details the cross-platform compatibility status of par-term-emu-core-rust and provides guidance for maintaining platform compatibility.

## Table of Contents

- [Overview](#overview)
- [Supported Platforms](#supported-platforms)
- [Platform Compatibility Status](#platform-compatibility-status)
  - [Terminal Emulation Core](#terminal-emulation-core)
  - [PTY Support](#pty-support)
  - [Screenshot Module](#screenshot-module)
  - [Debug Infrastructure](#debug-infrastructure)
- [Platform-Specific Considerations](#platform-specific-considerations)
  - [Windows](#windows)
  - [macOS](#macos)
  - [Linux](#linux)
- [Build Dependencies](#build-dependencies)
- [Emoji Font Support](#emoji-font-support)
- [Testing on Multiple Platforms](#testing-on-multiple-platforms)
- [Best Practices for Contributors](#best-practices-for-contributors)
- [CI/CD Configuration](#cicd-configuration)
- [Known Limitations](#known-limitations)
- [Potential Enhancements](#potential-enhancements)
- [Related Documentation](#related-documentation)

## Overview

par-term-emu-core-rust is designed for maximum cross-platform compatibility, using pure Rust implementations wherever possible to minimize platform-specific dependencies. The library leverages well-tested cross-platform crates like `portable-pty` for PTY operations and `swash` for font rendering.

## Supported Platforms

par-term-emu-core-rust officially supports:
- **Linux** (x86_64, aarch64)
- **macOS** (x86_64, Apple Silicon)
- **Windows** (x86_64)

## Platform Compatibility Status

### Terminal Emulation Core

**Platform Independence: Excellent**

The core terminal emulation is completely platform-agnostic:

- **VT Sequence Parsing**: Uses the `vte` crate (pure Rust, no platform dependencies)
- **Grid Management**: Pure Rust implementation with no platform-specific code
- **Color Handling**: Consistent RGB color model across all platforms
- **Unicode Support**: Full Unicode support including wide characters, combining marks, and emoji

All VT100/VT220/VT320/VT420 sequences work identically across platforms.

### PTY Support

**Platform Independence: Excellent (via portable-pty)**

PTY operations in `src/pty_session.rs` use the `portable-pty` crate for cross-platform compatibility:

**Shell Detection:**
- **Windows**: Uses `%COMSPEC%` environment variable (typically `cmd.exe`), fallback to `cmd.exe`
- **Unix/macOS**: Uses `$SHELL` environment variable, fallback to `/bin/bash`

Implementation in `src/pty_session.rs`:
```rust
pub fn get_default_shell() -> String {
    if cfg!(windows) {
        // Use %COMSPEC% (typically cmd.exe), fall back to cmd.exe
        if let Ok(comspec) = std::env::var("COMSPEC") {
            comspec
        } else {
            "cmd.exe".to_string()
        }
    } else {
        // Unix-like: check $SHELL, fall back to /bin/bash
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}
```

**Environment Variables:**
- Inherits parent environment variables properly on all platforms
- Automatically drops `COLUMNS` and `LINES` to prevent resize issues (apps should query PTY size via ioctl)
- Sets `TERM=xterm-256color` and `COLORTERM=truecolor` consistently
- Sets Kitty-specific variables (`TERM_PROGRAM=kitty`, `KITTY_WINDOW_ID`, `KITTY_PID`) for protocol detection

**Process Management:**
- Uses `portable-pty::native_pty_system()` for platform-appropriate PTY implementation
- Thread-safe design with Arc/Mutex for shared state
- Proper cleanup on all platforms

### Screenshot Module

**Platform Independence: Excellent (Pure Rust)**

The screenshot module in `src/screenshot/` uses pure Rust implementations for maximum portability:

**Font Rendering:**
- Uses `swash` crate (pure Rust, no FreeType/HarfBuzz dependencies)
- No C libraries required on any platform
- Consistent rendering quality across all platforms

**Embedded Fonts:**
- **JetBrains Mono** (~268KB) - Primary monospace font
- **Noto Emoji** (~409KB) - Monochrome emoji fallback

**System Font Paths** (searched in priority order):

**macOS:**
```rust
"/System/Library/Fonts/Apple Color Emoji.ttc",      // Color emoji
"/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
"/Library/Fonts/Arial Unicode.ttf",
"/System/Library/Fonts/Supplemental/DejaVu Sans.ttf",
"/System/Library/Fonts/Supplemental/DejaVuSans.ttf",
"/System/Library/Fonts/AppleSDGothicNeo.ttc",       // Korean
"/System/Library/Fonts/CJKSymbolsFallback.ttc",
"/System/Library/Fonts/PingFang.ttc",               // Chinese
"/System/Library/Fonts/Hiragino Sans GB.ttc",
"/System/Library/Fonts/Apple Symbols.ttf",
```

**Linux:**
```rust
"/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
"/usr/share/fonts/truetype/noto-color-emoji/NotoColorEmoji.ttf",
"/usr/share/fonts/noto-color-emoji/NotoColorEmoji.ttf",
"/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
"/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
"/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
"/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
```

**Windows:**
```rust
"C:\\Windows\\Fonts\\seguiemj.ttf",  // Segoe UI Emoji
"C:\\Windows\\Fonts\\seguisym.ttf",  // Segoe UI Symbol
"C:\\Windows\\Fonts\\arial.ttf",     // Arial (basic coverage)
"C:\\Windows\\Fonts\\msgothic.ttc",  // MS Gothic (Japanese)
"C:\\Windows\\Fonts\\msyh.ttc",      // Microsoft YaHei (Chinese)
```

**Image Encoding:**
- PNG, JPEG, BMP, SVG formats supported via `image` crate
- Consistent output across all platforms

### Debug Infrastructure

**Platform Independence: Excellent**

Debug logging in `src/debug.rs` handles platform differences transparently:

**Log File Locations:**
- **Unix/macOS**: `/tmp/par_term_emu_core_rust_debug_rust.log`
- **Windows**: `%TEMP%\par_term_emu_core_rust_debug_rust.log`

Implementation:
```rust
#[cfg(unix)]
let log_path = std::path::PathBuf::from("/tmp/par_term_emu_core_rust_debug_rust.log");
#[cfg(windows)]
let log_path = std::env::temp_dir().join("par_term_emu_core_rust_debug_rust.log");
```

**Control:**
- `DEBUG_LEVEL` environment variable (0-4) works identically on all platforms
- All debug output goes to log files (never stdout/stderr) to avoid corrupting TUI apps

## Platform-Specific Considerations

### Windows

**Shell Behavior:**
- Default shell: `cmd.exe` (via `%COMSPEC%` environment variable)
- Not bash or PowerShell by default
- Path separators: Backslash (`\`) instead of forward slash (`/`)
- Environment variable syntax: `%VARIABLE%` instead of `$VARIABLE`
- Line endings: CRLF (`\r\n`) instead of LF (`\n`)

**Font System:**
- System fonts located in `C:\Windows\Fonts\`
- Emoji font: `seguiemj.ttf` (Segoe UI Emoji) - color emoji on Windows 10+
- CJK fonts: `msgothic.ttc`, `msyh.ttc`

**PTY Implementation:**
- Uses ConPTY on Windows 10 1809+ (via portable-pty)
- Fallback to winpty on older Windows versions
- SIGWINCH not available (resize handled differently)

**Testing Notes:**
- Some tests use Unix-specific paths like `/bin/echo` or `/home/user`
- These tests are informational and don't break Windows builds
- Windows-specific tests should use `#[cfg(windows)]` guards

### macOS

**Shell Environment:**
- Default shell: `zsh` on macOS 10.15 (Catalina) and later
- Previous versions used `bash` by default
- Users may configure custom shells via `$SHELL` environment variable
- Common shells: zsh, bash, fish

**Font System:**
- System fonts in `/System/Library/Fonts/` and `/Library/Fonts/`
- Emoji font: `Apple Color Emoji.ttc` (excellent color emoji coverage)
- Built-in CJK support: PingFang (Chinese), Hiragino Sans GB (Chinese), Hiragino Kaku Gothic ProN (Japanese), AppleSDGothicNeo (Korean)
- Additional fonts: CJKSymbolsFallback, Apple Symbols, STHeiti, AppleMyungjo, STSong
- Arial Unicode provides comprehensive fallback coverage (both in `/System/Library/Fonts/Supplemental/` and `/Library/Fonts/`)
- DejaVu Sans available in Supplemental folder for Unicode fallback

**Platform Features:**
- Excellent out-of-the-box emoji rendering with Apple Color Emoji
- Wide range of CJK fonts pre-installed (Chinese, Japanese, Korean)
- May require Full Disk Access permission for some system fonts in certain security contexts
- PTY implementation uses Unix domain sockets with SIGWINCH for resize signals

### Linux

**Shell Environment:**
- Varies by distribution (bash, zsh, dash, fish, etc.)
- Always respects `$SHELL` environment variable
- Fallback: `/bin/bash` (most widely available)

**Font System:**
- Distribution-dependent font locations
- Common paths:
  - Debian/Ubuntu: `/usr/share/fonts/`
  - Fedora/RHEL: `/usr/share/fonts/`
  - Arch: `/usr/share/fonts/`
- Emoji font installation:
  ```bash
  # Debian/Ubuntu
  sudo apt install fonts-noto-color-emoji

  # Fedora
  sudo dnf install google-noto-emoji-color-fonts

  # Arch
  sudo pacman -S noto-fonts-emoji
  ```

**Distribution Differences:**
- Font packages may have different names
- Some distros include emoji fonts by default, others don't
- Embedded Noto Emoji font ensures basic emoji support everywhere

## Build Dependencies

### Pure Rust Implementation

par-term-emu-core-rust is built entirely in Rust with no C dependencies required. This significantly simplifies the build process across all platforms.

**Key Advantages:**
- No C library dependencies (no FreeType, HarfBuzz, Fontconfig, etc.)
- No platform-specific build tools beyond Rust toolchain
- Simplified build process across all platforms
- Better cross-compilation support
- Consistent behavior everywhere

**Required Tools:**
- **Rust**: Version 1.75 or later (minimum specified in `Cargo.toml: rust-version = "1.75"`)
- **Cargo**: Comes with Rust
- **Python**: 3.12+ for Python bindings (if building with `python` feature)
- **uv**: Python package manager for dependency management (recommended)

**Quick Start:**
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the project
cargo build --release

# Or build Python bindings
uv run maturin develop --release
```

**Platform-Specific Setup:**

**Windows:**
- MSVC Build Tools required (Visual Studio 2019+ or Build Tools for Visual Studio)
- PowerShell or cmd.exe for build commands

**macOS:**
- Xcode Command Line Tools: `xcode-select --install`
- No additional dependencies

**Linux:**
- GCC or Clang (usually pre-installed)
- No additional dependencies

## Emoji Font Support

### Font Fallback Strategy

The screenshot module implements a sophisticated multi-tier font fallback system for emoji and Unicode characters:

**Tier 1: System Color Emoji Fonts**
- **Linux**: NotoColorEmoji (if installed)
- **macOS**: Apple Color Emoji (built-in, excellent coverage)
- **Windows**: Segoe UI Emoji (Windows 10+)

**Tier 2: System Unicode Fonts**
- Arial Unicode, Noto Sans, DejaVu Sans, Liberation Sans
- Platform-specific CJK fonts for Asian characters (see below)

**Tier 3: Embedded Fonts**
- **JetBrains Mono** (~268KB) - Primary monospace font for terminal text
- **Noto Emoji** (~409KB) - Monochrome emoji fallback for universal compatibility

**Tier 4: Graceful Degradation**
- Characters not found in any font render as tofu boxes (□)
- This only occurs if embedded fonts fail to load (extremely rare)

### CJK Font Paths

The screenshot module searches for CJK fonts in the following locations (in priority order):

**macOS:**
```rust
"/System/Library/Fonts/Supplemental/Arial Unicode.ttf",  // Excellent CJK coverage
"/Library/Fonts/Arial Unicode.ttf",
"/System/Library/Fonts/PingFang.ttc",                    // Modern Chinese
"/System/Library/Fonts/Hiragino Sans GB.ttc",            // Chinese
"/System/Library/Fonts/AppleSDGothicNeo.ttc",            // Korean
"/System/Library/Fonts/Hiragino Kaku Gothic ProN.ttc",   // Japanese
"/System/Library/Fonts/CJKSymbolsFallback.ttc",          // CJK punctuation & symbols
"/System/Library/Fonts/STHeiti Medium.ttc",              // Legacy Chinese
"/System/Library/Fonts/AppleMyungjo.ttf",                // Korean serif
"/System/Library/Fonts/STSong.ttf",                      // Chinese serif
```

**Linux:**
```rust
"/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/truetype/noto-cjk/NotoSansCJK-Regular.ttc",
"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
"/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
```

**Windows:**
```rust
"C:\\Windows\\Fonts\\msgothic.ttc",  // Japanese
"C:\\Windows\\Fonts\\msyh.ttc",      // Chinese (Microsoft YaHei)
"C:\\Windows\\Fonts\\malgun.ttf",    // Korean (Malgun Gothic)
"C:\\Windows\\Fonts\\arial.ttf",     // Has some CJK coverage
```

**Note:** The CJK font loader loads **ALL** available fonts from the above list, not just the first one. This provides comprehensive CJK coverage through multi-font fallback. Each character is cached with its preferred font for performance.

### Font Loading Implementation

The font cache in `src/screenshot/font_cache.rs` implements lazy loading:

```rust
// Embedded fonts are always available
const DEFAULT_FONT: &[u8] = include_bytes!("JetBrainsMono-Regular.ttf");  // ~268KB
const EMOJI_FALLBACK_FONT: &[u8] = include_bytes!("NotoEmoji-Regular.ttf");  // ~409KB

// System fonts loaded on-demand when emoji/CJK characters detected
fn try_load_emoji_font(&mut self) { /* searches system emoji font paths */ }
fn try_load_cjk_font(&mut self) { /* loads ALL available CJK fonts in priority order */ }
```

**Behavior:**
1. Regular ASCII text uses JetBrains Mono (embedded, always available)
2. Emoji detection (`is_emoji()`) triggers lazy loading of system color emoji fonts
3. If no system emoji font found, falls back to embedded Noto Emoji (monochrome)
4. CJK detection (`is_cjk()`) triggers lazy loading of ALL available system CJK fonts in priority order
5. CJK font fallback: tries each loaded font until glyph is found
6. Font choices cached per character (`cjk_font_cache: HashMap<char, usize>`) for performance
7. Glyph cache: `HashMap<(char, u32, bool, bool), CachedGlyph>` for rendered glyphs

### Potential Enhancement: Optional Color Emoji Embedding

Future versions could add an optional feature for embedding color emoji:

```toml
[features]
default = ["python"]
embed-color-emoji = []  # Would add ~10-15MB for NotoColorEmoji
```

This is not currently implemented because:
- Most systems have adequate emoji fonts
- Embedded monochrome fallback provides universal compatibility
- Binary size remains reasonable (~1MB currently)

## Testing on Multiple Platforms

### Rust Tests

Run Rust tests on all platforms:

```bash
# Basic test suite (library tests only, no default features)
cargo test --lib --no-default-features --features pyo3/auto-initialize

# All tests including integration tests
cargo test

# Platform-specific tests use cfg guards
#[cfg(unix)]
#[test]
fn test_unix_pty_spawn() {
    // Unix-specific PTY test
}

#[cfg(windows)]
#[test]
fn test_windows_pty_spawn() {
    // Windows-specific PTY test
}
```

### Python Tests

Run Python tests on all platforms using pytest:

```bash
# All tests with timeout protection
uv run pytest tests/ -v --timeout=5 --timeout-method=thread

# Exclude platform-specific PTY tests (if problematic in CI)
uv run pytest tests/ --ignore=tests/test_pty.py

# Run specific test file
uv run pytest tests/test_terminal.py -v
```

**Note**: Some PTY tests may be excluded in CI due to platform-specific behavior.

### Code Quality Checks

**Rust:**
```bash
cargo fmt -- --check      # Format check
cargo clippy -- -D warnings  # Linting
```

**Python:**
```bash
uv run ruff format --check .  # Format check
uv run ruff check .           # Linting
uv run pyright .              # Type checking
```

**All checks:**
```bash
make checkall  # Runs all quality checks
```

### Manual Testing Checklist

When testing on a new platform, verify:

**Screenshot Module:**
- [ ] PNG screenshot generation works
- [ ] Emoji render correctly (color with system fonts, monochrome fallback)
- [ ] CJK characters render correctly
- [ ] Custom fonts can be loaded
- [ ] JPEG, BMP, SVG formats work

**PTY Module:**
- [ ] Default shell spawns correctly
- [ ] Custom commands execute
- [ ] Window resize works (SIGWINCH on Unix, ConPTY on Windows)
- [ ] Environment variables inherited properly
- [ ] `TERM=xterm-256color` and `COLORTERM=truecolor` set correctly
- [ ] Kitty protocol environment variables set (`TERM_PROGRAM`, `KITTY_WINDOW_ID`, `KITTY_PID`)

**Graphics Module:**
- [ ] Kitty graphics protocol works
- [ ] Sixel graphics render
- [ ] Animation playback functions

**Debug Infrastructure:**
- [ ] Log file created in correct temp directory
- [ ] `DEBUG_LEVEL` environment variable controls verbosity
- [ ] Debug output doesn't corrupt terminal display

## Best Practices for Contributors

### 1. Avoid Hardcoded Paths
❌ **Bad:**
```rust
let path = "/tmp/myfile.log";
```

✅ **Good:**
```rust
let path = std::env::temp_dir().join("myfile.log");
```

### 2. Use Cross-Platform Path APIs
❌ **Bad:**
```rust
let path = format!("{}/{}", dir, file);  // Unix-specific separator
```

✅ **Good:**
```rust
use std::path::Path;
let path = Path::new(dir).join(file);
```

### 3. Handle Platform Differences with cfg

Use Rust's conditional compilation for platform-specific code:

```rust
// Good: Clear platform-specific handling
pub fn get_default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

// Good: Platform-specific tests
#[cfg(unix)]
#[test]
fn test_unix_signal_handling() {
    // Unix-specific test
}

#[cfg(windows)]
#[test]
fn test_windows_conpty() {
    // Windows-specific test
}
```

### 4. Test on Multiple Platforms

**Local Testing:**
- Test on your development platform before committing
- If possible, test on at least two platforms (e.g., macOS + Linux)
- Use VMs or WSL for testing other platforms locally

**CI/CD Testing:**
- All PRs automatically run on Linux, macOS, and Windows via GitHub Actions
- Check CI results before merging
- Fix any platform-specific failures

**Test Matrix:**
- Tests run on Python 3.12, 3.13, and 3.14
- Tests run on all three major platforms
- Total: 9 test combinations per PR

### 5. Document Platform-Specific Behavior

**Code Comments:**
```rust
// Windows uses ConPTY (Windows 10 1809+) with fallback to winpty
// Unix uses traditional PTY with SIGWINCH for resize notification
let pty_system = portable_pty::native_pty_system();
```

**User Documentation:**
- Update relevant docs when adding platform-specific features
- Note platform limitations clearly
- Provide platform-specific examples when behavior differs

**This Document:**
- Update `CROSS_PLATFORM.md` when adding new platform-specific code
- Document workarounds for platform-specific issues
- Keep compatibility matrix up to date

## CI/CD Configuration

### GitHub Actions Implementation

The project uses comprehensive cross-platform CI via GitHub Actions (`.github/workflows/ci.yml`).

**Trigger**: Workflow runs on `workflow_dispatch` (manual trigger)

**Test Matrix:**
```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
    python-version: ["3.12", "3.13", "3.14"]
```

This creates **9 test combinations** (3 platforms × 3 Python versions).

### CI Pipeline Jobs

**1. Test Job**
- Runs on all platforms (Linux, macOS, Windows)
- Tests all Python versions (3.12, 3.13, 3.14)
- 15-minute timeout per job
- Executes:
  - Rust library tests: `cargo test --lib`
  - Python tests: `pytest tests/` (with 5-second per-test timeout)

**2. Lint Job**
- Runs on Ubuntu only (linting is platform-independent)
- Checks:
  - Rust formatting: `cargo fmt --check`
  - Rust linting: `cargo clippy --all-targets --all-features`
  - Python formatting: `ruff format --check`
  - Python linting: `ruff check`
  - Python type checking: `pyright`

**3. Build Job**
- Runs on all platforms (Linux, macOS, Windows)
- Builds Python wheels using maturin
- 15-minute timeout per job
- Uploads wheel artifacts for each platform

### Platform-Specific CI Steps

**All Platforms:**
```yaml
- name: Build Python package
  run: uv run maturin develop --release

- name: Run Rust tests
  run: cargo test --lib --no-default-features --features pyo3/auto-initialize
```

**Windows Only:**
```yaml
- name: Set up MSVC
  if: runner.os == 'Windows'
  uses: ilammy/msvc-dev-cmd@v1
```

**Platform-Specific Test Commands:**

Linux uses `timeout` wrapper:
```yaml
- name: Run Python tests (Linux)
  if: runner.os == 'Linux'
  run: timeout 300 uv run pytest tests/ -v --timeout=5
```

macOS and Windows run pytest directly:
```yaml
- name: Run Python tests (macOS)
  if: runner.os == 'macOS'
  run: uv run pytest tests/ -v --timeout=5
```

### CI Best Practices

**Timeouts:**
- Job timeout: 15 minutes
- Individual test timeout: 5 seconds
- Prevents hanging tests from blocking CI

**Exclusions:**
- PTY tests excluded in CI: `test_pty.py`, `test_ioctl_size.py`, `test_pty_resize_sigwinch.py`, `test_nested_shell_resize.py`
- These are excluded due to platform-specific behavior and CI environment limitations
- Graphics tests may be excluded if they require display access

**Test Execution:**
- All platforms run the same test suite with platform-specific exclusions
- Tests excluded: PTY-related tests that depend on terminal control behavior

## Known Limitations

### Windows

**PTY Behavior:**
- ConPTY available on Windows 10 1809+ (October 2018 Update)
- Older Windows versions fall back to winpty (via portable-pty)
- SIGWINCH signal not available (resize handled via ConPTY API)
- Some Unix-specific tests use hardcoded paths like `/bin/echo`

**Font System:**
- Color emoji require Windows 10+ with Segoe UI Emoji
- Older Windows versions may have limited emoji support
- Falls back to embedded monochrome emoji font

**Shell:**
- Default shell is `cmd.exe`, not PowerShell
- PowerShell can be used via explicit path: `term.spawn("powershell.exe", [])`

### macOS

**Font System:**
- Apple Color Emoji font is ~30MB (not embedded due to size)
- System always has excellent color emoji support
- Some system fonts may require Full Disk Access permission in System Settings

**Platform Behavior:**
- Default shell changed to zsh in macOS 10.15 (Catalina)
- Older macOS versions use bash by default
- PTY implementation uses traditional Unix PTY with SIGWINCH

### Linux

**Font System:**
- NotoColorEmoji not installed by default on many distributions
- Font paths vary by distribution (Debian/Ubuntu vs Fedora vs Arch)
- Users may need to install emoji fonts manually
- Embedded Noto Emoji provides fallback for missing system fonts

**Distribution Differences:**
- Font package names vary (`fonts-noto-color-emoji` vs `google-noto-emoji-color-fonts` vs `noto-fonts-emoji`)
- Font locations vary:
  - NotoColorEmoji: `/usr/share/fonts/truetype/noto/`, `/usr/share/fonts/truetype/noto-color-emoji/`, `/usr/share/fonts/noto-color-emoji/`
  - NotoSansCJK: `/usr/share/fonts/opentype/noto/`, `/usr/share/fonts/truetype/noto/`, `/usr/share/fonts/noto-cjk/`, `/usr/share/fonts/truetype/noto-cjk/`
- Some minimal distributions may lack basic Unicode fonts and need manual installation
- Liberation and DejaVu fonts widely available as fallbacks

**Shell Variations:**
- Wide variety of shells (bash, zsh, fish, dash, etc.)
- Shell configuration files vary by distro
- Always respects `$SHELL` environment variable

### General Limitations

**Test Coverage:**
- Some platform-specific tests are informational only
- PTY tests may behave differently on different platforms
- Graphics tests may require display access (excluded in headless CI)

**Font Embedding:**
- Only monochrome emoji embedded (color emoji would add 10-15MB)
- Custom fonts must be provided by user or system
- Font fallback may not cover all Unicode ranges

## Potential Enhancements

Future versions could add:

**1. Optional Color Emoji Embedding**
```toml
[features]
embed-color-emoji = []  # Add ~10-15MB for guaranteed color emoji
```

**2. PowerShell Detection on Windows**
- Auto-detect PowerShell Core or Windows PowerShell
- Make PowerShell the default on modern Windows systems
- Keep cmd.exe as fallback

**3. XDG Directory Support**
- Respect `XDG_CONFIG_HOME` for custom font directories
- Support user font directories on all platforms
- Better font discovery on Linux

**4. Enhanced Platform-Specific Tests**
- More comprehensive platform-specific test coverage
- Automated testing of font rendering on all platforms
- Better CI coverage for edge cases

**5. Cross-Compilation Support**
- Document cross-compilation procedures
- Provide Docker images for consistent builds
- Support ARM platforms more comprehensively

## Related Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture and design decisions
- [BUILDING.md](BUILDING.md) - Build and installation instructions for all platforms
- [README.md](../README.md) - User-facing documentation and quick start
- [SECURITY.md](SECURITY.md) - Security considerations for PTY operations
- [FONTS.md](FONTS.md) - Font system and emoji rendering details
- [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) - Configuration options reference

## External Resources

- [Rust Platform Support](https://doc.rust-lang.org/rustc/platform-support.html) - Official Rust platform tiers
- [portable-pty documentation](https://docs.rs/portable-pty/) - Cross-platform PTY abstraction
- [swash documentation](https://docs.rs/swash/) - Pure Rust font rendering library
- [image crate documentation](https://docs.rs/image/) - Image encoding and decoding
- [vte crate documentation](https://docs.rs/vte/) - VT sequence parser
