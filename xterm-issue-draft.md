# xterm.js Issue Draft

**Title:** Block Element characters (U+2580-U+259F) render incorrectly during scroll operations

---

## Description

When Unicode Block Element characters (U+2580-U+259F range) are output to the terminal and scroll occurs during rendering, visual artifacts appear. The artifacts manifest as corrupted/displaced content on lines where scroll happened during output.

Regular ASCII characters and emoji render correctly under the same conditions. The issue specifically affects the Block Elements Unicode range.

## Affected Characters

All characters in the Block Elements range (U+2580-U+259F):
- `█` U+2588 (Full block)
- `▄` U+2584 (Lower half block)
- `▀` U+2580 (Upper half block)
- `░` U+2591 (Light shade)
- `▒` U+2592 (Medium shade)
- `▓` U+2593 (Dark shade)
- And other block element characters

## Not Affected

- Regular ASCII characters (space, letters, numbers, symbols)
- Wide emoji characters (e.g., 🟥 🟧 🟨)
- Box Drawing characters (U+2500-U+257F) - need to verify
- Block Elements when no scroll occurs during rendering

## Environment

- **xterm.js version:** 5.5.0
- **Addons:** @xterm/addon-webgl 0.18.0, @xterm/addon-fit 0.10.0
- **Browser:** Chrome (also tested in other browsers)
- **OS:** macOS (also affects other platforms)
- **Renderers tested:** Both WebGL and DOM renderers exhibit the issue

## Steps to Reproduce

1. Create a terminal with fewer rows than the content to be rendered
2. Output multiple rows of Block Element characters with colors
3. Ensure output exceeds terminal height to trigger scroll
4. Observe artifacts on lines where scroll occurred during rendering

### Minimal Reproduction Code

```html
<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@xterm/xterm@5.5.0/css/xterm.css">
</head>
<body>
  <div id="terminal"></div>
  <button onclick="runTest()">Run Block Element Test</button>
  <script src="https://cdn.jsdelivr.net/npm/@xterm/xterm@5.5.0/lib/xterm.js"></script>
  <script>
    const term = new Terminal({ rows: 24, cols: 80 });
    term.open(document.getElementById('terminal'));

    function runTest() {
      // Generate more rows than terminal height to trigger scroll
      const numRows = 40;
      const cols = term.cols;

      for (let y = 0; y < numRows; y++) {
        let line = '';
        for (let x = 0; x < cols; x++) {
          // Create gradient colors
          const h = x / cols;
          const r = Math.floor(255 * (1 - h));
          const g = Math.floor(255 * h);
          const b = 0;

          // Use lower half block with FG+BG colors (like Rich library does)
          line += `\x1b[38;2;${r};${g};${b}m\x1b[48;2;${r};${g};0m▄\x1b[0m`;
        }
        term.write(line + '\r\n');
      }
    }
  </script>
</body>
</html>
```

### Python Reproduction (if using a PTY backend)

```python
import sys
import colorsys

cols, rows = 80, 24  # Adjust to your terminal size

# Output more rows than terminal height
for y in range(rows + 15):
    for x in range(cols):
        h = x / cols
        r1, g1, b1 = colorsys.hls_to_rgb(h, 0.4, 1.0)
        r2, g2, b2 = colorsys.hls_to_rgb(h, 0.5, 1.0)

        # Block element with FG+BG colors
        sys.stdout.write(f"\x1b[38;2;{int(r2*255)};{int(g2*255)};{int(b2*255)}m")
        sys.stdout.write(f"\x1b[48;2;{int(r1*255)};{int(g1*255)};{int(b1*255)}m")
        sys.stdout.write("▄")
        sys.stdout.write("\x1b[0m")
    sys.stdout.write("\n")
    sys.stdout.flush()
```

## Expected Behavior

Block Element characters should render correctly regardless of whether scroll occurs during output, just like regular ASCII characters do.

## Actual Behavior

When scroll occurs during Block Element output:
- Visual artifacts appear on affected lines
- Content appears corrupted or displaced
- Artifacts persist in scrollback (not just a rendering glitch)

## What Works (No Artifacts)

For comparison, these scenarios work correctly:
- Spaces with background colors during scroll - **works**
- Regular letters (X, A, etc.) with FG+BG colors during scroll - **works**
- Wide emoji (🟥) with colors during scroll - **works**
- Block Elements when terminal doesn't scroll - **works**

## Additional Context

This issue was discovered when using the [Rich](https://github.com/Textualize/rich) Python library, which uses `▄` (lower half block) with FG+BG colors to create smooth color gradients. The Rich demo (`python -m rich`) consistently shows artifacts when the output causes terminal scroll.

The issue appears to be specific to the custom glyph rendering path for Block Elements (introduced in #2409 / PR #3416) interacting with scroll operations.

## Possibly Related Issues

- #2409 - Manually draw pixel-perfect glyphs for Box Drawing and Block Elements characters
- #3617 - Window contents not cleared properly (WebGL caching issue)
- #4180 - Fix glyphs becoming garbled or invisible

---

## Diagnostic Tests Performed

| Test | Character | Colors | Result |
|------|-----------|--------|--------|
| Space | U+0020 | BG only | PASS |
| Space | U+0020 | FG+BG | PASS |
| Letter X | U+0058 | FG+BG | PASS |
| Full block █ | U+2588 | FG+BG | **FAIL** |
| Lower half ▄ | U+2584 | FG+BG | **FAIL** |
| Upper half ▀ | U+2580 | FG+BG | **FAIL** |
| Lower half ▄ | U+2584 | BG only | **FAIL** |
| Light shade ░ | U+2591 | FG+BG | **FAIL** |
| Medium shade ▒ | U+2592 | FG+BG | **FAIL** |
| Red square 🟥 | U+1F7E5 | FG+BG | PASS |
