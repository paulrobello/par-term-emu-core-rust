# TODO: Variation Selector and Emoji Sequence Preservation

**Date**: 2025-11-23
**Priority**: HIGH
**Impact**: Affects colored emoji rendering (⚠️ ❤️ ℹ️) and complex emoji sequences

---

## 🎯 Problem Statement

The terminal currently **discards variation selectors and joining characters** when processing incoming text, causing several emoji rendering issues:

### 1. Variation Selectors Lost (U+FE0E, U+FE0F)
**Example**: `⚠️` (U+26A0 + U+FE0F) → Only `⚠` (U+26A0) stored
- **Result**: Symbol renders in monochrome text style instead of colored emoji style
- **Affects**: ⚠️ ❤️ ℹ️ ☑️ and many other dual-style symbols

### 2. Zero Width Joiner Lost (U+200D)
**Example**: `👨‍👩‍👧‍👦` (MAN + ZWJ + WOMAN + ZWJ + GIRL + ZWJ + BOY)
- **Result**: Four separate overlapping emoji instead of single family glyph
- **Affects**: All ZWJ sequences (family, profession, skin tone combinations)

### 3. Skin Tone Modifiers Separated
**Example**: `👋🏽` (WAVE + MEDIUM SKIN TONE)
- **Result**: Hand emoji + separate brown square instead of combined brown hand
- **Affects**: All emoji with skin tone modifiers (U+1F3FB-U+1F3FF)

---

## 🔍 Root Cause Analysis

### Current Behavior
When processing incoming UTF-8 text, the terminal:
1. Decodes characters one by one
2. Treats variation selectors and ZWJ as "formatting" or "zero-width" characters
3. Discards them or doesn't store them in the cell grid
4. Each visible emoji component gets its own cell

### Why This Happens
The terminal emulator was likely designed for **ASCII-era terminals** where:
- One character = one cell
- No concept of grapheme clusters
- Combining characters handled differently

### Unicode Reality
Modern emoji use **grapheme clusters** (multiple codepoints forming one visual unit):
- `🇺🇸` = U+1F1E6 (🇺) + U+1F1F8 (🇸) → Single flag glyph
- `👋🏽` = U+1F44B (👋) + U+1F3FD (🏽) → Single brown hand
- `👨‍👩‍👧‍👦` = 4 emoji + 3 ZWJ → Single family emoji

---

## ✅ Solution: Preserve Emoji Sequences

### Approach 1: Store Grapheme Clusters (Recommended)

**Concept**: Detect grapheme cluster boundaries and store entire cluster in first cell

#### Implementation Steps:

1. **Add Unicode Segmentation**
   ```toml
   # Add to Cargo.toml dependencies
   unicode-segmentation = "1.12"
   ```

2. **Detect Grapheme Clusters During Text Processing**
   ```rust
   use unicode_segmentation::UnicodeSegmentation;

   // In character processing code
   fn process_incoming_text(&mut self, text: &str) {
       // Split into grapheme clusters instead of characters
       for grapheme in text.graphemes(true) {
           self.handle_grapheme_cluster(grapheme);
       }
   }
   ```

3. **Modify Cell Storage**
   ```rust
   pub struct Cell {
       pub character: char,           // Base character
       pub combining: Vec<char>,      // Variation selectors, ZWJ, modifiers
       pub width: u8,                 // Display width (1 or 2)
       // ... other fields
   }
   ```

4. **Handle Grapheme Clusters**
   ```rust
   fn handle_grapheme_cluster(&mut self, grapheme: &str) {
       let chars: Vec<char> = grapheme.chars().collect();

       if chars.is_empty() {
           return;
       }

       // Base character goes in current cell
       let base_char = chars[0];
       let combining_chars: Vec<char> = chars[1..].to_vec();

       // Calculate display width
       let width = if is_wide_emoji(grapheme) { 2 } else { 1 };

       let cell = Cell {
           character: base_char,
           combining: combining_chars,
           width,
           // ...
       };

       self.grid.set_cell(self.cursor_x, self.cursor_y, cell);

       // If wide character, mark next cell as spacer
       if width == 2 {
           self.grid.set_spacer(self.cursor_x + 1, self.cursor_y);
       }

       self.cursor_x += width as usize;
   }
   ```

5. **Extract Full Grapheme for Rendering**
   ```rust
   impl Cell {
       pub fn get_grapheme(&self) -> String {
           let mut result = String::new();
           result.push(self.character);
           for c in &self.combining {
               result.push(*c);
           }
           result
       }
   }
   ```

#### Key Detection Functions:

```rust
fn is_variation_selector(c: char) -> bool {
    c == '\u{FE0E}' || c == '\u{FE0F}'
}

fn is_zwj(c: char) -> bool {
    c == '\u{200D}'
}

fn is_skin_tone_modifier(c: char) -> bool {
    let code = c as u32;
    (0x1F3FB..=0x1F3FF).contains(&code)
}

fn is_regional_indicator(c: char) -> bool {
    let code = c as u32;
    (0x1F1E6..=0x1F1FF).contains(&code)
}

fn is_wide_emoji(grapheme: &str) -> bool {
    // Regional Indicator pairs (flags)
    if grapheme.chars().filter(|c| is_regional_indicator(*c)).count() == 2 {
        return true;
    }

    // ZWJ sequences
    if grapheme.contains('\u{200D}') {
        return true;
    }

    // Emoji with skin tone modifiers
    if grapheme.chars().any(is_skin_tone_modifier) {
        return true;
    }

    false
}
```

---

### Approach 2: Extended Cell Attributes (Alternative)

**Concept**: Add flags to cells indicating they're part of emoji sequence

```rust
pub struct Cell {
    pub character: char,
    pub emoji_sequence_type: EmojiSequenceType,
    pub sequence_position: u8,  // 0 = base, 1+ = combining
    // ...
}

pub enum EmojiSequenceType {
    None,
    RegionalIndicator,   // Flag emoji
    SkinToneModifier,    // Skin tone sequence
    ZwjSequence,         // Zero-width joiner sequence
    VariationSelector,   // Text vs emoji style
}
```

**Pros**: Minimal storage overhead
**Cons**: More complex to reconstruct sequences

---

## 🧪 Testing Strategy

### Test Cases to Implement:

```rust
#[test]
fn test_variation_selector_preservation() {
    let mut term = Terminal::new(80, 24);

    // Warning sign with emoji style variation selector
    term.process_text("⚠️");  // U+26A0 + U+FE0F

    let cell = term.get_cell(0, 0);
    assert_eq!(cell.get_grapheme(), "⚠️");
    assert!(cell.combining.contains(&'\u{FE0F}'));
}

#[test]
fn test_zwj_sequence_preservation() {
    let mut term = Terminal::new(80, 24);

    // Family emoji
    term.process_text("👨‍👩‍👧‍👦");

    let cell = term.get_cell(0, 0);
    let grapheme = cell.get_grapheme();
    assert!(grapheme.contains('\u{200D}'));  // Contains ZWJ
    assert_eq!(cell.width, 2);  // Wide emoji
}

#[test]
fn test_skin_tone_preservation() {
    let mut term = Terminal::new(80, 24);

    // Waving hand with medium skin tone
    term.process_text("👋🏽");  // U+1F44B + U+1F3FD

    let cell = term.get_cell(0, 0);
    assert_eq!(cell.character, '👋');
    assert!(cell.combining.contains(&'🏽'));
    assert_eq!(cell.width, 2);
}

#[test]
fn test_regional_indicator_preservation() {
    let mut term = Terminal::new(80, 24);

    // US flag
    term.process_text("🇺🇸");  // U+1F1FA + U+1F1F8

    let cell = term.get_cell(0, 0);
    let grapheme = cell.get_grapheme();
    assert_eq!(grapheme, "🇺🇸");
    assert_eq!(cell.width, 2);
}
```

---

## 📁 Files to Modify

### Primary Changes:
1. **`src/terminal.rs`** (or wherever character processing happens)
   - Add grapheme cluster detection
   - Modify cell storage logic

2. **`src/grid.rs`** (or cell grid implementation)
   - Update Cell struct
   - Add `get_grapheme()` method
   - Handle wide character spacing

3. **`src/vte_processor.rs`** (if VT parsing is separate)
   - Ensure grapheme clusters aren't split during parsing

### Dependencies:
```toml
[dependencies]
unicode-segmentation = "1.12"
unicode-width = "0.2"  # For width calculation
```

---

## 🚧 Implementation Phases

### Phase 1: Basic Variation Selector Support (2-4 hours)
- [ ] Add unicode-segmentation dependency
- [ ] Detect variation selectors (FE0E, FE0F)
- [ ] Store them with base character
- [ ] Test with ⚠️ ❤️ ℹ️

### Phase 2: ZWJ Sequence Support (4-6 hours)
- [ ] Detect ZWJ characters (U+200D)
- [ ] Keep emoji+ZWJ+emoji together
- [ ] Mark as wide (2 cells)
- [ ] Test with 👨‍👩‍👧‍👦 🏳️‍🌈

### Phase 3: Skin Tone Support (2-4 hours)
- [ ] Detect skin tone modifiers (U+1F3FB-U+1F3FF)
- [ ] Combine with base emoji
- [ ] Test with 👋🏽 👍🏿

### Phase 4: Regional Indicators (Already Working?)
- [ ] Verify flags work correctly
- [ ] Ensure stored as wide characters
- [ ] Test with 🇺🇸 🇬🇧 🇯🇵

---

## 🎨 Rendering Integration

The frontend (par-term) is already prepared to handle these:

```rust
// Frontend already has this logic in cell_renderer.rs
// Just needs the full grapheme from terminal core

let grapheme = cell.get_grapheme();  // "⚠️" with FE0F

// Text shaper receives full grapheme cluster
let shaped = text_shaper.shape_text(&grapheme, font_index);

// HarfBuzz will properly shape:
// - "⚠️" → Colored warning emoji glyph
// - "👨‍👩‍👧‍👦" → Single family glyph
// - "👋🏽" → Single brown hand glyph
```

---

## 📊 Success Criteria

### Must Work:
- [x] Simple emoji: 😀 🎉 🔥 (already working)
- [ ] Colored symbols: ⚠️ ❤️ ℹ️ (needs variation selectors)
- [x] Flags: 🇺🇸 🇬🇧 🇯🇵 (already working with frontend workaround)
- [ ] Skin tones: 👋🏽 👍🏿 (needs modifier preservation)
- [ ] ZWJ sequences: 👨‍👩‍👧‍👦 🏳️‍🌈 (needs ZWJ preservation)

### Performance:
- Grapheme cluster detection should add < 5% overhead
- No impact on ASCII text processing
- Memory: +8 bytes per cell with combining chars (Vec<char>)

---

## 🔗 References

### Unicode Standards:
- [TR51: Unicode Emoji](https://unicode.org/reports/tr51/)
- [UAX29: Unicode Text Segmentation](https://unicode.org/reports/tr29/)
- [Emoji ZWJ Sequences](https://unicode.org/emoji/charts/emoji-zwj-sequences.html)

### Rust Crates:
- [unicode-segmentation](https://docs.rs/unicode-segmentation/)
- [unicode-width](https://docs.rs/unicode-width/)

### Testing Resources:
- [Emoji Test File](https://unicode.org/Public/emoji/latest/emoji-test.txt)
- [Full Emoji List](https://unicode.org/emoji/charts/full-emoji-list.html)

---

## 💡 Alternative: Minimal Fix

If full implementation is too complex, a **minimal fix** could be:

```rust
// Just preserve variation selectors specifically
fn handle_variation_selector(&mut self, c: char) {
    if c == '\u{FE0F}' || c == '\u{FE0E}' {
        if let Some(cell) = self.grid.get_cell_mut(self.cursor_x - 1, self.cursor_y) {
            cell.variation_selector = Some(c);
        }
    }
}
```

This would at least fix the colored emoji issue without full grapheme cluster support.

---

## 🤝 Coordination with Frontend

Frontend (par-term) changes needed:
- [x] Per-font text shaping (DONE)
- [x] Emoji sequence detection (DONE)
- [x] Run splitting logic (DONE)
- [ ] Read `cell.get_grapheme()` instead of `cell.character`

Once terminal core provides full graphemes, frontend just needs to call:
```rust
let grapheme = cell.get_grapheme();  // Instead of cell.character
```

All the emoji sequence handling is already implemented in the frontend's text shaping system.

---

**Estimated Total Time**: 1-2 days for full implementation
**Recommended Start**: Phase 1 (Variation Selectors) - highest impact, simplest fix
