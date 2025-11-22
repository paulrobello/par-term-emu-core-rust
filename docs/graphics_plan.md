# Graphics Architecture Plan

Multi-protocol graphics support for Sixel, iTerm2 inline images, and Kitty graphics protocol.

## Implementation Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Core Refactoring | **Complete** | `src/graphics/mod.rs`, `TerminalGraphic`, `GraphicsStore`, `GraphicsLimits` |
| Phase 2: Scrollback Persistence | **Complete** | Terminal uses `GraphicsStore` with scrollback support. Graphics visible in scrollback! |
| Phase 3: iTerm2 Support | **Complete** | Single-sequence and chunked transfers (MultipartFile/FilePart) both working |
| Phase 4: Kitty Support | **Complete** | APC G handling in `dcs.rs`, `KittyParser` in `src/graphics/kitty.rs` |
| Phase 5: Advanced Features | **Complete** (core) | Virtual placements, relative placements, Unicode placeholder infrastructure, full animation support |

### Recent Updates (2025-11-21)

**Debug Output Cleanup - All Projects TUI-Safe** ✅

All three sister projects now have proper debug-to-file logging infrastructure, eliminating all stdout/stderr output that could interfere with TUI applications:

1. **par-term-emu-core-rust (Backend)** ✅
   - Removed/converted 12+ eprintln! statements across 7 files
   - All debug output now uses debug framework (`debug_error!`, `debug_info!`, `debug_log!`, `debug_trace!`)
   - Log file: `/tmp/par_term_emu_core_rust_debug_rust.log`
   - Controlled by `DEBUG_LEVEL` environment variable (0-4)
   - Files updated: `streaming/server.rs`, `terminal/mod.rs`, `screenshot/font_cache.rs`, `streaming/broadcaster.rs`, `python_bindings/streaming.rs`, `pty_session.rs`, `debug.rs`

2. **par-term (Frontend)** ✅
   - Created new `src/debug.rs` module
   - Converted 11 eprintln! statements across 5 files
   - Log file: `/tmp/par_term_debug.log`
   - Controlled by `DEBUG_LEVEL` environment variable (0-4)
   - Files updated: `app.rs`, `terminal.rs`, `graphics_renderer.rs`, `cell_renderer.rs`, `renderer.rs`
   - Build successful with clean compilation

3. **par-term-emu-tui-rust (TUI)** ✅
   - Already properly configured with debug logging
   - Uses `setup_debug_logging()` in `app.py`
   - Timestamped log files in `debug_logs/` directory
   - Print statements in `config.py` are intentional user-facing recovery prompts (run before TUI starts)
   - No changes needed

**Result**: All three projects are now production-ready with no spurious debug output interfering with terminal UIs.

### Recent Updates (2025-01-21)

**Phase 5: Advanced Kitty Graphics Features** ✅ **CORE COMPLETE**

All core backend features are **fully implemented and tested**. Only frontend rendering integration remains.
- ✅ **Virtual Placements (U=1)**: Implemented support for virtual placements
  - Virtual placements serve as prototypes for Unicode placeholder-based images
  - Stored separately in `GraphicsStore.virtual_placements`
  - Do not display directly; used as templates for U+10EEEE placeholders
- ✅ **Relative Placements (P=, Q=, H=, V=)**: Implemented relative positioning
  - `P=<image_id>` and `Q=<placement_id>` specify parent placement
  - `H=<pixels>` and `V=<pixels>` specify offset from parent
  - Allows positioning images relative to other placements
  - Useful for complex layouts and image composition
- ✅ **Unicode Placeholder Infrastructure**: Diacritics parsing implemented
  - ✅ Created `src/graphics/placeholder.rs` module
  - ✅ Implemented diacritic-to-number mapping (0-63 range)
  - ✅ `PlaceholderInfo` struct for extracting image ID, placement ID, row, column, MSB
  - ✅ Placeholder inheritance logic for omitted diacritics
  - ✅ `get_placeholder_graphic()` method in GraphicsStore for rendering lookup
  - 🔄 Cell-level detection pending (need to integrate with Grid/rendering)
  - 🔄 Rendering logic pending (frontend implementation needed)
- ✅ **Animation Support**: Backend complete, frontend partially integrated
  - ✅ Created `src/graphics/animation.rs` module
  - ✅ `AnimationFrame` struct with frame data, delay, offset, composition mode
  - ✅ `Animation` struct managing frame collection, playback state, timing
  - ✅ `AnimationControl` enum for play/pause/stop/loop control
  - ✅ Frame storage in `GraphicsStore.animations` HashMap
  - ✅ `KittyAction::Frame` parser support for adding frames
  - ✅ `KittyAction::AnimationControl` parser support for playback control
  - ✅ Frame timing and automatic advancement with `update_animations()`
  - ✅ Alpha blend and overwrite composition modes
  - ✅ Loop count support (0 = infinite loops)
  - ✅ Frontend `update_animations()` called in render loop (app.rs:1595)
  - ✅ Frontend `get_graphics_with_animations()` implemented (terminal.rs:548)
  - 🔄 **ISSUE**: Static graphics work, animations don't display (see handoff.md)

### Recent Updates (2025-11-21)

**Graphics Scrollback - FULLY WORKING** ✅
- Graphics now properly persist in scrollback and render when viewing scrollback
- ✅ **FIXED: 2x Scroll Speed Bug** - Graphics were scrolling at 2x speed when entering scrollback
  - **Root Cause**: Double clipping bug in par-term's `graphics_renderer.rs`
  - Graphic height was being reduced by `scroll_offset_rows`, but position calculation already accounted for scrolling
  - **Solution**: Removed height reduction in `graphics_renderer.rs:338`
  - Graphics now maintain full height; only position changes as they scroll
  - **Fix Location**: `/Users/probello/Repos/par-term/src/graphics_renderer.rs:336-338`
- Position calculation formula is correct: `absolute_row = scrollback_len - scroll_offset + row`
- Texture cropping (tex_v_start) correctly uses `scroll_offset_rows` for partial visibility
- par-term frontend updated to use new `TerminalGraphic` type and unified renderer

**Kitty Graphics Protocol** ✅
- ✅ Kitty graphics protocol (APC G) fully implemented
- ✅ File transmission modes (`t=f`, `t=t`) with security validation
  - Directory traversal prevention
  - File size limits (100MB max)
  - Temp file auto-deletion
- ✅ Image ID reuse and placement support
- ✅ Query response handling for protocol detection
- ✅ PNG format support
- See `docs/SECURITY.md` for file loading security considerations

**iTerm2 Inline Images** ✅
- ✅ OSC 1337 handler receives and parses sequences
- ✅ Single-sequence format: `File=inline=1:<base64_data>`
- ✅ Chunked transfer (`imgcat` protocol): `MultipartFile` → multiple `FilePart` → finalize
  - Size validation and limit enforcement
  - Automatic completion detection
  - Fallback to single-file processing after assembly

## Reference Specifications

- **Sixel**: <https://vt100.net/docs/vt3xx-gp/chapter14.html>
- **iTerm2**: <https://iterm2.com/documentation-images.html>
- **Kitty**: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

## Current State

### Existing Files
- `src/sixel.rs` - Sixel parser, `SixelGraphic`, `SixelParser`, `SixelLimits`
- `src/terminal/graphics.rs` - Graphics management methods on Terminal
- `src/terminal/sequences/dcs.rs` - DCS sequence handling (Sixel entry point)
- `src/terminal/mod.rs` - `Terminal.graphics: Vec<SixelGraphic>`
- `src/python_bindings/types.rs` - `PyGraphic` wrapper

### Current Architecture
- **Storage**: `Terminal.graphics_store: GraphicsStore` - unified storage for all protocols
- **Cell Association**: None - graphics overlay by (col, row) coordinates
- **Scrollback**: Graphics are moved to scrollback when scrolled off (preserved)
- **Rendering**: Single RGBA pixel format via `TerminalGraphic`, supports all protocols

## Goals

1. Support Sixel, iTerm2 (OSC 1337), and Kitty graphics protocols
2. Graphics persist in scrollback buffer
3. Unified rendering abstraction for TUI (half-blocks) and pixel-based terminals
4. Kitty image ID reuse (multiple placements reference same image data)

---

## 1. Unified Graphics Types

### New Module: `src/graphics/mod.rs`

```rust
pub mod kitty;
pub mod iterm;

use std::collections::HashMap;
use std::sync::Arc;

/// Graphics protocol identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicProtocol {
    Sixel,
    ITermInline,  // OSC 1337
    Kitty,        // APC graphics protocol
}

/// Protocol-agnostic graphic representation
#[derive(Debug, Clone)]
pub struct TerminalGraphic {
    pub id: u64,                              // Unique placement ID
    pub protocol: GraphicProtocol,
    pub position: (usize, usize),             // (col, row) in terminal
    pub width: usize,                         // Width in pixels
    pub height: usize,                        // Height in pixels
    pub pixels: Arc<Vec<u8>>,                 // RGBA pixel data (Arc for Kitty sharing)
    pub cell_dimensions: Option<(u32, u32)>,  // (cell_width, cell_height) for rendering
    pub scroll_offset_rows: usize,            // Rows scrolled off visible area

    // Kitty-specific (None for other protocols)
    pub kitty_image_id: Option<u32>,
    pub kitty_placement_id: Option<u32>,
}

/// Centralized graphics storage supporting image reuse
pub struct GraphicsStore {
    /// Kitty shared images: image_id -> pixel data
    shared_images: HashMap<u32, Arc<Vec<u8>>>,

    /// All active placements (visible area)
    placements: Vec<TerminalGraphic>,

    /// Graphics in scrollback (keyed by scrollback row)
    scrollback: Vec<TerminalGraphic>,

    /// Next unique placement ID
    next_id: u64,
}
```

### GraphicsStore Methods

```rust
impl GraphicsStore {
    pub fn new() -> Self;

    // Placement management
    pub fn add_graphic(&mut self, graphic: TerminalGraphic) -> u64;
    pub fn remove_graphic(&mut self, id: u64);
    pub fn graphics_at_row(&self, row: usize) -> Vec<&TerminalGraphic>;
    pub fn all_graphics(&self) -> &[TerminalGraphic];

    // Kitty image management
    pub fn store_kitty_image(&mut self, image_id: u32, pixels: Vec<u8>);
    pub fn get_kitty_image(&self, image_id: u32) -> Option<Arc<Vec<u8>>>;
    pub fn remove_kitty_image(&mut self, image_id: u32);

    // Scrolling
    pub fn adjust_for_scroll_up(&mut self, lines: usize, visible_rows: usize);
    pub fn adjust_for_scroll_down(&mut self, lines: usize);

    // Scrollback
    pub fn graphics_in_scrollback(&self, start_row: usize, end_row: usize) -> Vec<&TerminalGraphic>;
    pub fn clear_scrollback_graphics(&mut self);
}
```

---

## 2. Escape Sequences

### Sixel (DCS)
```
DCS P1 ; P2 ; P3 q <sixel-data> ST
ESC P ... ESC \
0x90 ... 0x9C (8-bit)
```
- Already implemented in `src/terminal/sequences/dcs.rs`

### iTerm2 (OSC 1337)
```
OSC 1337 ; File=[params]:<base64-data> ST
ESC ] 1337 ; File=name=foo.png;inline=1:BASE64DATA ESC \
```
- Entry point: `src/terminal/sequences/osc.rs`
- Parameters semicolon-separated key=value pairs
- Data after colon is base64-encoded image

### Kitty (APC)
```
APC G <key>=<value>,<key>=<value>;<base64-data> ST
ESC _ G a=T,f=100,... ; BASE64DATA ESC \
0x9F G ... 0x9C (8-bit)
```
- New entry point needed: handle APC in main parser
- Chunked: `m=1` means more chunks follow, `m=0` is final
- Keys: `a`=action, `f`=format, `t`=transmission, `i`=image_id, `p`=placement_id, etc.

---

## 3. Protocol Parsers

### Sixel (Modify existing)

Update `src/sixel.rs`:
- `SixelParser.build_graphic()` returns `TerminalGraphic` instead of `SixelGraphic`
- Keep Sixel-specific parsing logic, convert output to unified format
- Move `SixelGraphic` to internal parser state only

### iTerm2 Inline Images

New file: `src/graphics/iterm.rs`

```rust
/// Parse iTerm2 OSC 1337 inline image
/// Format: OSC 1337 ; File=name=<base64>;size=<bytes>;inline=1:<base64 data> ST
pub struct ITermParser {
    params: HashMap<String, String>,
    data: Vec<u8>,
}

impl ITermParser {
    pub fn new() -> Self;
    pub fn parse_params(&mut self, params: &str) -> Result<(), ITermError>;
    pub fn decode_image(&self) -> Result<TerminalGraphic, ITermError>;
}
```

Supported parameters:
- `name` - filename (optional)
- `size` - byte size hint
- `width`, `height` - size in cells or pixels
- `preserveAspectRatio` - boolean
- `inline` - must be 1 for inline display

Image decoding: Use `image` crate to decode PNG, JPEG, GIF to RGBA.

### Kitty Graphics Protocol

New file: `src/graphics/kitty.rs`

```rust
/// Kitty graphics transmission action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyAction {
    Transmit,           // t - transmit image data
    TransmitDisplay,    // T - transmit and display
    Query,              // q - query terminal support
    Put,                // p - display previously transmitted image
    Delete,             // d - delete images
    Frame,              // f - animation frame
    AnimationControl,   // a - animation control
}

/// Kitty transmission format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyFormat {
    Rgba,    // 32-bit RGBA
    Rgb,     // 24-bit RGB
    Png,     // PNG compressed
}

/// Kitty transmission medium
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyMedium {
    Direct,      // d - direct in-band data
    File,        // f - read from file
    TempFile,    // t - read from temp file and delete
    SharedMem,   // s - read from shared memory
}

pub struct KittyParser {
    // Parsing state for chunked transmission
    image_id: Option<u32>,
    placement_id: Option<u32>,
    action: KittyAction,
    format: KittyFormat,
    medium: KittyMedium,
    width: Option<u32>,
    height: Option<u32>,
    // ... additional parameters
    data_chunks: Vec<Vec<u8>>,
    more_chunks: bool,
}

impl KittyParser {
    pub fn new() -> Self;
    pub fn parse_chunk(&mut self, payload: &str) -> Result<Option<KittyCommand>, KittyError>;
    pub fn build_graphic(&self, store: &mut GraphicsStore) -> Result<TerminalGraphic, KittyError>;
}
```

Key Kitty features to support:
1. **Image IDs**: Store image data once, reference by ID
2. **Placement IDs**: Multiple placements of same image
3. **Chunked transmission**: Images sent in multiple APC sequences
4. **Virtual placements**: Unicode placeholder characters (U+10EEEE range)
5. **Delete commands**: By ID, position, or all

---

## 3. Scrollback Persistence

### Design

When graphics scroll off the visible area:
1. Don't delete - transfer to `GraphicsStore.scrollback`
2. Adjust `scroll_offset_rows` to track position relative to scrollback
3. When scrollback is viewed, include scrollback graphics in render

### Implementation

Modify `GraphicsStore.adjust_for_scroll_up()`:

```rust
pub fn adjust_for_scroll_up(&mut self, lines: usize, visible_rows: usize) {
    let mut to_scrollback = Vec::new();

    self.placements.retain_mut(|g| {
        g.position.1 = g.position.1.saturating_sub(lines);

        let graphic_bottom = g.position.1 + g.height_in_cells();
        if graphic_bottom == 0 {
            // Completely scrolled off - move to scrollback
            g.scrollback_row = Some(self.current_scrollback_row);
            to_scrollback.push(g.clone());
            false
        } else {
            // Partially visible or fully visible
            if g.position.1 == 0 {
                g.scroll_offset_rows += lines.min(g.height_in_cells());
            }
            true
        }
    });

    self.scrollback.extend(to_scrollback);
}
```

### Scrollback Limits

- Apply same limits as text scrollback (configurable max lines)
- When scrollback exceeds limit, remove oldest graphics too
- Track scrollback row indices for proper positioning

---

## 4. Rendering Abstraction

### Trait for Backends

```rust
/// Rendering mode for graphics
pub enum RenderMode {
    /// Half-block characters (TUI) - sample 2 vertical pixels per character
    HalfBlock,
    /// Per-pixel rendering (GPU terminals)
    PerPixel,
}

impl TerminalGraphic {
    /// Get RGBA color at pixel coordinates
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y * self.width + x) * 4;
        Some((
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ))
    }

    /// Sample color for half-block cell rendering
    /// Returns (top_half_rgba, bottom_half_rgba) for the cell at (col, row)
    pub fn sample_half_block(
        &self,
        cell_col: usize,
        cell_row: usize,
        cell_width: u32,
        cell_height: u32,
    ) -> Option<((u8, u8, u8, u8), (u8, u8, u8, u8))> {
        let px_x = (cell_col - self.position.0) * cell_width as usize;
        let px_y = (cell_row - self.position.1) * cell_height as usize;

        // Sample center of top and bottom halves
        let top_y = px_y + cell_height as usize / 4;
        let bottom_y = px_y + (cell_height as usize * 3) / 4;
        let center_x = px_x + cell_width as usize / 2;

        let top = self.pixel_at(center_x, top_y)?;
        let bottom = self.pixel_at(center_x, bottom_y)?;

        Some((top, bottom))
    }

    /// Get dimensions in terminal cells
    pub fn cell_size(&self, cell_width: u32, cell_height: u32) -> (usize, usize) {
        let cols = (self.width + cell_width as usize - 1) / cell_width as usize;
        let rows = (self.height + cell_height as usize - 1) / cell_height as usize;
        (cols, rows)
    }
}
```

### TUI Half-Block Rendering

For Python TUI using Rich/Textual:
- Use Unicode half-block character: `▀` (U+2580)
- Set foreground = top half color, background = bottom half color
- Sample graphic at cell positions using `sample_half_block()`

### Per-Pixel Rendering

For Rust terminal (par-term):
- Pass raw RGBA pixel data
- Render as texture/image in GPU shader
- Position based on cell coordinates × cell dimensions

---

## 5. Python Bindings

### Updated PyGraphic

```rust
#[pyclass]
pub struct PyGraphic {
    #[pyo3(get)]
    pub id: u64,
    #[pyo3(get)]
    pub protocol: String,  // "sixel", "iterm", "kitty"
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub width: usize,
    #[pyo3(get)]
    pub height: usize,
    #[pyo3(get)]
    pub cell_dimensions: Option<(u32, u32)>,

    inner: Arc<Vec<u8>>,
}

#[pymethods]
impl PyGraphic {
    /// Get raw RGBA pixel data as bytes
    pub fn pixels(&self) -> &[u8];

    /// Get RGBA color at pixel (x, y)
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)>;

    /// Sample for half-block rendering at cell (col, row)
    pub fn sample_half_block(
        &self,
        cell_col: usize,
        cell_row: usize,
        cell_width: u32,
        cell_height: u32,
    ) -> Option<((u8, u8, u8, u8), (u8, u8, u8, u8))>;

    /// Get size in terminal cells
    pub fn cell_size(&self, cell_width: u32, cell_height: u32) -> (usize, usize);
}
```

---

## 6. Implementation Order

### Phase 1: Core Refactoring
1. Create `src/graphics/mod.rs` with `TerminalGraphic`, `GraphicsStore`
2. Migrate Sixel to output `TerminalGraphic`
3. Replace `Terminal.graphics` with `GraphicsStore`
4. Update Python bindings

### Phase 2: Scrollback Persistence
1. Implement scrollback transfer in `GraphicsStore`
2. Add scrollback graphics retrieval methods
3. Update Python API for scrollback graphics

### Phase 3: iTerm2 Support
1. Add OSC 1337 parsing in `src/terminal/sequences/osc.rs`
2. Implement `ITermParser` for base64 image decode
3. Add `image` crate dependency for PNG/JPEG decode

### Phase 4: Kitty Support
1. Add APC sequence handling
2. Implement `KittyParser` with chunked transmission
3. Support image ID storage and placement
4. Implement delete commands

### Phase 5: Advanced Features
1. Kitty Unicode placeholders (optional)
2. Animation support (optional)
3. Performance optimization (lazy decode, caching)

---

## 7. Dependencies

Add to `Cargo.toml`:
```toml
[dependencies]
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "gif"] }
base64 = "0.22"
```

---

## 8. Error Types

```rust
// src/graphics/mod.rs
#[derive(Debug, thiserror::Error)]
pub enum GraphicsError {
    #[error("Invalid image dimensions: {0}x{1}")]
    InvalidDimensions(u32, u32),
    #[error("Image too large: {0} bytes (max {1})")]
    ImageTooLarge(usize, usize),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("Invalid base64: {0}")]
    Base64Error(#[from] base64::DecodeError),
    #[error("Image decode failed: {0}")]
    ImageError(#[from] image::ImageError),
    #[error("Kitty protocol error: {0}")]
    KittyError(String),
    #[error("iTerm protocol error: {0}")]
    ITermError(String),
}
```

---

## 9. File Change Summary

### New Files
| File | Purpose |
|------|---------|
| `src/graphics/mod.rs` | `TerminalGraphic`, `GraphicsStore`, `GraphicProtocol`, `GraphicsError` |
| `src/graphics/iterm.rs` | `ITermParser` for OSC 1337 |
| `src/graphics/kitty.rs` | `KittyParser`, `KittyAction`, `KittyFormat`, `KittyMedium` |

### Modified Files
| File | Changes |
|------|---------|
| `src/lib.rs` | Add `pub mod graphics;` |
| `src/sixel.rs` | Output `TerminalGraphic`, keep internal `SixelGraphic` for parsing state |
| `src/terminal/mod.rs` | Replace `graphics: Vec<SixelGraphic>` with `graphics: GraphicsStore` |
| `src/terminal/graphics.rs` | Delegate to `GraphicsStore` methods |
| `src/terminal/sequences/osc.rs` | Add OSC 1337 handling |
| `src/terminal/parser.rs` | Add APC sequence handling for Kitty |
| `src/python_bindings/types.rs` | Update `PyGraphic` with protocol field and new methods |
| `Cargo.toml` | Add `image`, `base64` dependencies |

---

## 10. Backward Compatibility

- `PyGraphic` gains new fields but existing fields unchanged
- `Terminal.graphics()` Python method returns same structure (enhanced)
- Existing Sixel tests must continue passing
- Add feature flag `graphics-iterm` and `graphics-kitty` if conditional compilation needed

---

## 11. Resource Limits

```rust
/// Limits for graphics to prevent resource exhaustion
pub struct GraphicsLimits {
    pub max_width: u32,           // Default: 10000 pixels
    pub max_height: u32,          // Default: 10000 pixels
    pub max_pixels: usize,        // Default: 25_000_000 (25MP)
    pub max_total_memory: usize,  // Default: 256MB across all graphics
    pub max_graphics_count: usize, // Default: 1000 placements
    pub max_scrollback_graphics: usize, // Default: 500
}
```

Apply limits:
- Reject images exceeding dimensions
- Evict oldest graphics when memory exceeded
- Limit scrollback graphics separately

---

## 12. Testing Strategy

### Unit Tests
- Parse/encode each protocol
- Graphics positioning calculations
- Scroll adjustments
- Half-block sampling

### Integration Tests
- Full terminal sequences → rendered graphics
- Scrollback persistence through scroll operations
- Multiple graphics interactions

### Visual Tests
- Sixel test images
- iTerm2 inline images
- Kitty protocol test suite

### Test Files Location
- `tests/graphics/` - Rust integration tests
- `tests/test_graphics.py` - Python binding tests
- `tests/fixtures/images/` - Sample images for each protocol
