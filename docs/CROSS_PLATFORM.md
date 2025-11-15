# Cross-Platform Compatibility Guide

This document details the cross-platform compatibility status of par-term-emu and provides guidance for maintaining platform compatibility.

## Supported Platforms

par-term-emu officially supports:
- **Linux** (x86_64, aarch64)
- **macOS** (x86_64, Apple Silicon)
- **Windows** (x86_64)

## Platform Compatibility Status

### ✅ Excellent Cross-Platform Support

#### Terminal Emulation Core
- **VT Sequence Parsing**: Platform-agnostic (uses `vte` crate)
- **Grid Management**: Pure Rust, no platform dependencies
- **Color Handling**: Consistent across all platforms
- **Unicode Support**: Full Unicode including emoji on all platforms

#### PTY Support (`src/pty_session.rs`)
- **Shell Detection**: Platform-specific handling ✅
  - **Windows**: Uses `%COMSPEC%` environment variable, fallback to `cmd.exe`
  - **Unix**: Uses `$SHELL` environment variable, fallback to `/bin/bash`
- **Process Spawning**: Uses `portable-pty` crate for cross-platform PTY
- **Environment Variables**: Properly inherits and sets platform-appropriate variables

#### Screenshot Module (`src/screenshot/`)
- **Font Rendering**: Swash (pure Rust) - no C dependencies, works on all platforms
- **Image Encoding**: `image` crate supports all platforms
- **Font Paths**: Comprehensive coverage with system font paths:
  - **macOS**: Apple Color Emoji, Arial Unicode, system fonts
  - **Linux**: NotoColorEmoji, Noto, DejaVu, Liberation
  - **Windows**: Segoe UI Emoji, Symbol, CJK fonts
- **Embedded Fonts**: JetBrains Mono and Noto Emoji as fallbacks
- **Path Handling**: Uses `std::path::Path` (cross-platform abstraction)
- **Pure Rust**: No platform-specific build dependencies required

#### Debugging (`src/debug.rs`)
- **Log File Location**: Cross-platform ✅
  - **Unix/macOS**: `/tmp/par_term_emu_debug_rust.log`
  - **Windows**: `%TEMP%\par_term_emu_debug_rust.log`
- **Implementation**: Uses `std::env::temp_dir()` for platform-appropriate temp directory
- **Python**: Also uses `tempfile.gettempdir()` for cross-platform compatibility

## Platform-Specific Considerations

### Windows
**Shell Command Differences:**
- Default shell: PowerShell or cmd.exe (not bash)
- Path separators: `\\` instead of `/`
- Environment variables: `%VARIABLE%` syntax
- Line endings: CRLF (`\r\n`) vs LF (`\n`)

**Font Paths:**
- System fonts in `C:\Windows\Fonts\`
- Emoji font: `seguiemj.ttf` (Segoe UI Emoji)

**Testing:**
- Some tests use Unix-specific paths (`/bin/echo`, `/home/user`)
- These tests are informational and won't break Windows builds

### macOS
**Font Paths:**
- System fonts in `/System/Library/Fonts/`
- Emoji font: `Apple Color Emoji.ttc`
- Excellent emoji coverage out of the box

**Shell:**
- Default: zsh (macOS 10.15+) or bash
- Users may have custom shells in `$SHELL`

### Linux
**Font Paths:**
- Distro-dependent font locations
- Common: `/usr/share/fonts/`
- Emoji font: NotoColorEmoji (if installed)
- May require: `sudo apt install fonts-noto-color-emoji`

**Shell:**
- Varies by distro (bash, zsh, fish, etc.)
- Respects `$SHELL` environment variable

## Build Dependencies

### Pure Rust Implementation ✅

**No External Dependencies Required!**

The screenshot module uses **Swash**, a pure Rust font rendering library. This means:
- ✅ No C library dependencies (FreeType, HarfBuzz, etc.)
- ✅ No platform-specific build tools required
- ✅ Simpler build process across all platforms
- ✅ Better cross-compilation support
- ✅ Just Rust and Cargo - that's it!

**Installation:**
```bash
# All you need is Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the project - no additional dependencies!
cargo build --release
```

## Emoji Font Support

### System Font Detection
The screenshot module searches for emoji fonts in this priority order:

1. **Color emoji fonts** (highest priority)
   - Linux: NotoColorEmoji
   - macOS: Apple Color Emoji
   - Windows: Segoe UI Emoji

2. **Fallback Unicode fonts**
   - Arial Unicode, Noto Sans, DejaVu Sans

3. **Graceful degradation**
   - If no emoji font found, renders as grayscale placeholder boxes

### Embedded Font Implementation

**Current State:**
- ✅ JetBrains Mono (274KB) - Embedded for text rendering
- ✅ Noto Emoji (419KB) - Embedded for universal emoji support

**Implemented: Noto Emoji (Monochrome)**
- **Size**: 419KB (manageable)
- **Coverage**: All Unicode emoji
- **Quality**: Decent grayscale styling
- **Benefit**: Works on all platforms without system fonts

**Implementation:**
```rust
// src/screenshot/font_cache.rs
// Embedded fonts loaded via Swash

// Fallback order (as implemented):
// 1. System color emoji font (NotoColorEmoji, Apple Color Emoji, etc.)
// 2. Embedded emoji font (monochrome) ✅ IMPLEMENTED
// 3. Grayscale placeholder boxes (only if embedded font fails to load)
```

**Future Option: Optional Color Emoji Feature**
```toml
[features]
default = []
embed-color-emoji = []  # Would add 10-15MB for NotoColorEmoji
```

## Testing on Multiple Platforms

### Rust Tests
```bash
# All tests should pass on all platforms
cargo test

# Platform-specific tests (if needed)
cargo test --features windows-specific  # Example
```

### Python Tests
```bash
# Run on all platforms
uv run pytest tests/

# Some tests may need platform guards
#[cfg(unix)]
#[test]
fn test_unix_specific() { ... }
```

### Manual Testing Checklist

**Screenshot Module:**
- [ ] PNG screenshots work
- [ ] Emoji render (color if system font available, grayscale otherwise)
- [ ] Custom fonts load correctly
- [ ] Sixel graphics render

**PTY Module:**
- [ ] Shell spawns correctly
- [ ] Commands execute
- [ ] Resize works (SIGWINCH on Unix)
- [ ] Environment variables set properly

**Debugging:**
- [ ] Log file created in correct temp directory
- [ ] DEBUG_LEVEL environment variable works

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
```rust
pub fn get_default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}
```

### 4. Test on Multiple Platforms
- Run tests locally if possible
- Use CI/CD for automated cross-platform testing
- Check GitHub Actions for Linux/macOS/Windows builds

### 5. Document Platform-Specific Behavior
- Add comments explaining platform differences
- Update this document when adding platform-specific code
- Note any platform limitations in user-facing documentation

## CI/CD Configuration

### Recommended GitHub Actions Matrix
```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
    rust: [stable]
```

### Platform-Specific Build Steps

**No platform-specific dependencies needed!** ✅

Since the project uses pure Rust (Swash) for font rendering, CI/CD is simple:

```yaml
- name: Build
  run: cargo build --release

- name: Test
  run: cargo test

# No platform-specific setup required!
```

## Known Limitations

### Windows-Specific
1. Some tests use Unix paths (non-breaking, informational only)
2. Color emoji require Windows 10+ with Segoe UI Emoji installed
3. PTY implementation may have minor behavioral differences

### macOS-Specific
1. Apple Color Emoji font is large (~30MB), not embedded
2. Some system fonts require Full Disk Access in security settings

### Linux-Specific
1. NotoColorEmoji may not be installed by default
2. Font paths vary by distribution
3. Some distros may need `fonts-noto-color-emoji` package

## Potential Enhancements

The following enhancements could further improve cross-platform support:

1. **Feature flag**: Optional NotoColorEmoji embedding (~10-15MB)
2. **Platform-specific tests**: `#[cfg(target_os = "...")]` guards for platform-specific functionality
3. **Cross-platform CI**: Expanded CI/CD test matrix for Windows and macOS
4. **Font discovery**: XDG config support for custom font directories

## Resources

- [Rust Platform Support](https://doc.rust-lang.org/rustc/platform-support.html)
- [portable-pty documentation](https://docs.rs/portable-pty/)
- [Swash documentation](https://docs.rs/swash/) - Pure Rust font rendering
- [std::env documentation](https://doc.rust-lang.org/std/env/)
- [image crate documentation](https://docs.rs/image/) - Image encoding/decoding

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture
- [BUILDING.md](BUILDING.md) - Build and installation instructions
- [README.md](../README.md) - User-facing documentation
