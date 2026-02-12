# Codebase Cleanup Report

Generated: 2026-02-12

## Summary

| Category | Count | Priority |
|---|---|---|
| TODO/FIXME comments | 3 | Medium |
| Dead code (`#[allow(dead_code)]`) | 6 annotations | Low-Medium |
| Unused `bold` parameters | 2 | Low |
| Unused struct (never instantiated) | 1 | Medium |

Clippy reports zero warnings with `--features streaming`. No unused dependencies detected in `Cargo.toml`. No deprecated items found.

---

## 1. TODO Comments

### 1.1 Kitty Graphics — Incomplete Delete Targets

- **File**: `src/graphics/kitty.rs:456`
- **Code**: `_ => {} // TODO: implement other delete targets`
- **Context**: The `KittyDeleteTarget` enum defines 8 variants (`All`, `ById`, `ByPlacement`, `AtCursor`, `InCell`, `OnScreen`, `ByColumn`, `ByRow`), but only `All`, `ById`, and `ByPlacement` are handled. The remaining 4 (`AtCursor`, `InCell`, `ByColumn`, `ByRow`) are silently ignored.
- **Recommendation**: Implement the missing delete targets using `GraphicsStore` position-based queries, or at minimum log a warning when an unimplemented target is requested so users know the operation was a no-op.

### 1.2 Kitty Graphics — Chunked Transmission

- **File**: `src/terminal/sequences/dcs.rs:337`
- **Code**: `// TODO: Support chunked transmission by storing parser state`
- **Context**: Kitty graphics protocol supports multi-chunk payloads for large images. Currently, the first chunk is processed but subsequent chunks are logged and discarded.
- **Recommendation**: Store `KittyParser` state between DCS chunks to support large image uploads. This is important for real-world usage where images exceed single-payload limits.

### 1.3 Graphics Placeholders — Diacritics Handling

- **File**: `src/terminal/graphics.rs:135`
- **Code**: `// TODO: Handle diacritics properly in Cell structure`
- **Context**: Kitty Unicode placeholder protocol uses combining diacritics to encode row/column offsets. Currently the base placeholder character is inserted but diacritics aren't added to the `Cell.combining` vector.
- **Recommendation**: Append the row/column/MSB diacritics to `Cell.combining` so that frontends reading the cell buffer can reconstruct the full placeholder sequence for proper rendering.

---

## 2. Dead Code

### 2.1 `DefaultSessionFactory` — Never Instantiated

- **File**: `src/streaming/server.rs:910-924`
- **Annotation**: `#[allow(dead_code)]`
- **Context**: Struct + `SessionFactory` impl for backward-compatible single-terminal sessions. Defined but never constructed anywhere in the codebase.
- **Recommendation**: **Remove entirely.** If backward compatibility is needed in the future, it can be re-added. Keeping dead code with a suppression annotation masks real warnings.

### 2.2 `GlyphMetrics` — Unused Fields

- **File**: `src/screenshot/font_cache.rs:42-49`
- **Fields**:
  - `xmin: i32` (line 42) — `#[allow(dead_code)]`
  - `advance_height: f32` (line 48) — `#[allow(dead_code)]`
- **Context**: These fields are populated during glyph rasterization but never read.
- **Recommendation**: **Remove both fields.** If vertical text layout or precise glyph positioning is added later, they can be re-introduced. Currently they add noise to the struct.

### 2.3 `ShapedGlyph` — Unused Fields

- **File**: `src/screenshot/shaper.rs:21-25`
- **Fields**:
  - `x_advance: i32` (line 21) — `#[allow(dead_code)]`
  - `y_advance: i32` (line 24) — `#[allow(dead_code)]`
- **Context**: Set during text shaping but never consumed by the renderer.
- **Recommendation**: **Remove both fields.** The renderer uses its own advance calculations. If shaping-based advance is needed, re-add with actual consumers.

### 2.4 `PyPtyTerminal` Rust-Only Methods

- **File**: `src/python_bindings/pty.rs:3238`
- **Annotation**: `#[allow(dead_code)] // Used by streaming feature`
- **Methods**: `get_terminal_arc()`, `set_output_callback()`
- **Recommendation**: **Keep as-is.** These are legitimately used by the streaming feature. The annotation is correct and documented. Consider gating with `#[cfg(feature = "streaming")]` instead of `#[allow(dead_code)]` for clarity.

---

## 3. Unused Parameters

### 3.1 `bold` Parameter in Font Cache

- **File**: `src/screenshot/font_cache.rs:389` and `src/screenshot/font_cache.rs:668`
- **Code**: `let _ = bold;`
- **Context**: Two font rasterization methods accept a `bold: bool` parameter but immediately discard it. The comment at line 388 says "Apply bold if requested (not used currently, but kept for API compatibility)".
- **Recommendation**: Either implement bold font rendering (synthetic bolding via stroke widening or weight selection) or remove the parameter from the function signature and all call sites. The `let _ = bold;` pattern is a code smell.

---

## 4. Unimplemented Enum Variants

### 4.1 `KittyDeleteTarget` Variants

- **File**: `src/graphics/kitty.rs:113-122`
- **Unhandled variants** (in `kitty.rs:456` match arm):
  - `AtCursor` — delete image at cursor position
  - `InCell` — delete image at specific cell
  - `ByColumn(u32)` — delete images in column
  - `ByRow(u32)` — delete images in row
- **Recommendation**: Implement using `GraphicsStore::graphics_at_row()` and cursor position queries. These are part of the Kitty graphics protocol spec and needed for full compliance.

---

## 5. Suppressed Result Values (`let _ =`)

The codebase has ~40 instances of `let _ = expr;` to suppress unused `Result` values. Most are appropriate (fire-and-forget channel sends, cleanup operations). A few warrant review:

| File | Line | Expression | Note |
|---|---|---|---|
| `screenshot/renderer.rs` | 61 | `let _ = shaper.set_emoji_font(emoji_data)` | Font load failure silently ignored |
| `screenshot/renderer.rs` | 66 | `let _ = shaper.set_cjk_font(cjk_data)` | Font load failure silently ignored |
| `streaming/server.rs` | 1159 | `let _ = session;` | Variable bound solely to suppress warning |

- **Recommendation**: For renderer font loading, consider logging a warning on failure so users know emoji/CJK rendering may be degraded. For `server.rs:1159`, the binding is purely to silence an unused variable warning — add a `_` prefix to the binding name instead (`let _session = ...`).

---

## Priority Recommendations

### Do Now (Quick Wins)
1. Remove `DefaultSessionFactory` — dead code, never instantiated
2. Remove unused `GlyphMetrics` fields (`xmin`, `advance_height`)
3. Remove unused `ShapedGlyph` fields (`x_advance`, `y_advance`)
4. Replace `#[allow(dead_code)]` on `PyPtyTerminal` methods with `#[cfg(feature = "streaming")]`

### Do Soon (Feature Completeness)
5. Implement remaining `KittyDeleteTarget` variants
6. Add chunked Kitty graphics transmission support
7. Handle diacritics in graphics placeholder cells

### Consider Later
8. Implement or remove `bold` parameter in font cache
9. Add logging for silently-dropped font load errors in renderer
