# Graphics Testing Guide

Quick reference for testing graphics support in par-term-emu-core-rust.

## Table of Contents
- [Supported Protocols](#supported-protocols)
- [Quick Tests](#quick-tests)
- [Graphics in Scrollback](#graphics-in-scrollback)
- [Debug Logging](#debug-logging)
- [Python TUI Testing](#python-tui-testing)
- [Common Issues](#common-issues)
- [Performance Testing](#performance-testing)
- [Architecture Documentation](#architecture-documentation)
- [Test Images](#test-images)
- [Tools](#tools)
- [Next Steps](#next-steps)
- [Contributing](#contributing)

## Supported Protocols

| Protocol | Status | Format | Test Script |
|----------|--------|--------|-------------|
| Sixel | ✅ Complete | DCS | Built-in examples |
| iTerm2 Inline | ✅ Complete | OSC 1337 | Use `imgcat` |
| Kitty Graphics | ✅ Complete | APC G | See below |
| Kitty Animation | ✅ Complete | APC G | [`test_kitty_animation.py`](../scripts/test_kitty_animation.py) |

## Quick Tests

### Sixel Graphics

```bash
# In par-term terminal
echo -e '\x1bPq"1;1;100;100#0;2;0;0;0#1;2;100;0;0#1!100~-\x1b\\'
```

### iTerm2 Inline Images

```bash
# Install imgcat (iTerm2 utility)
# Works with any terminal supporting OSC 1337
imgcat /path/to/image.png
```

### Kitty Graphics (Static Image)

```python
# Simple test with Python
import base64
from PIL import Image
import io

# Create a test image
img = Image.new('RGB', (100, 100), (255, 0, 0))
buf = io.BytesIO()
img.save(buf, format='PNG')
data = base64.standard_b64encode(buf.getvalue()).decode()

# Send to terminal
print(f'\x1b_Ga=T,f=100,t=d;{data}\x1b\\')
```

### Kitty Graphics Animation

```bash
# Start the par-term frontend
cd /Users/probello/Repos/par-term && cargo run

# In the terminal that opens, run the test script
python /Users/probello/Repos/par-term-emu-core-rust/scripts/test_kitty_animation.py
```

See detailed guide: [TESTING_KITTY_ANIMATIONS.md](TESTING_KITTY_ANIMATIONS.md)

## Graphics in Scrollback

All graphics protocols support scrollback:

```bash
# Display a graphic
echo -e '\x1bPq"1;1;100;100#0;2;0;0;0#1;2;100;0;0#1!100~-\x1b\\'

# Scroll it off screen
for i in {1..50}; do echo "line $i"; done

# Scroll back up to see it
# Use mouse wheel or Shift+PageUp
```

## Debug Logging

Enable debug output to track graphics processing:

```bash
# Backend (Rust)
export DEBUG_LEVEL=4
cd /Users/probello/Repos/par-term && cargo run

# In another terminal, monitor logs:
tail -f /tmp/par_term_emu_core_rust_debug_rust.log | grep -i "GRAPHICS\|KITTY\|ITERM"
```

## Python TUI Testing

For the Python TUI (par-term-emu-tui-rust):

```bash
cd /Users/probello/Repos/par-term-emu-tui-rust
uv run par-term-emu-tui-rust

# Graphics render as half-block characters in TUI mode
```

## Common Issues

### Graphics not appearing

1. **Check protocol support**: Verify the terminal reports correct features
2. **Check image size**: Ensure within limits (default: 10000x10000, 25MP max)
3. **Check debug logs**: Look for parsing errors or resource limit rejections
4. **Verify cell dimensions**: Graphics renderer needs correct cell size

### Scrollback graphics rendering issues

1. **Check scroll offset calculation**: Graphics should track `scroll_offset_rows`
2. **Verify scrollback_row assignment**: Graphics moved to scrollback need proper row index
3. **Check texture clipping**: Partially visible graphics should clip correctly

### Animation not playing

**Frontend integration complete** - Animations are fully supported in both backend and frontend:

1. **Backend (Complete)**:
   - `update_animations()` is called periodically in par-term's render loop
   - Animation frames advance based on timing
   - State control (play/pause/stop/loop) is implemented

2. **Frontend (Complete)**:
   - Static graphics display correctly
   - Animation frame rendering is working
   - `get_graphics_with_animations()` method properly integrated in par-term

If animations aren't playing, check:
- Animation control sequences were sent correctly (action 'a', 'f', 's', etc.)
- Frame delays are appropriate (gap parameter)
- Debug logs show frame updates

See: [TESTING_KITTY_ANIMATIONS.md](TESTING_KITTY_ANIMATIONS.md) for detailed testing guide

## Performance Testing

```bash
# Send many graphics quickly
for i in {1..100}; do
    echo -e '\x1bPq"1;1;50;50#0;2;0;0;0#1;2;100;0;0#1!50~-\x1b\\'
done

# Check memory usage
ps aux | grep par-term

# Check graphics count using Python bindings
python3 << 'EOF'
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)
# Send some test graphics here if needed

print(f'Active graphics: {term.graphics_count()}')
# Note: all_scrollback_graphics() is not exposed in Python API
# Use the Rust API or debug logs to inspect scrollback graphics
EOF
```

## Architecture Documentation

For implementation details, see:
- [Graphics Architecture Plan](graphics_plan.md) - Full design and implementation status
- [Testing Kitty Animations](TESTING_KITTY_ANIMATIONS.md) - Animation-specific testing
- [Security Considerations](SECURITY.md) - File loading and resource limits

## Test Images

Test images are available in the repository:

```
images/
├── snake.png         # Snake game screenshot (280KB)
├── snake.sixel       # Snake as Sixel (271KB)
├── snake_tui.png     # Snake TUI version (4.7KB)
└── snake_tui.sixel   # Snake TUI as Sixel (8.8KB)
```

**Note**: Currently, test images are primarily snake game screenshots. Additional test images (basic colors, patterns, etc.) can be added for comprehensive protocol testing.

## Tools

### imgcat (iTerm2)

```bash
# Install
brew install iterm2

# Use
imgcat image.png
```

### kitty icat (Kitty terminal)

```bash
# Requires Kitty terminal
kitty +kitten icat image.png
```

Both tools work with par-term if the respective protocols are supported.

## Next Steps

1. ✅ **Sixel, iTerm2, Kitty** - All protocols working
2. ✅ **Scrollback persistence** - Graphics saved in scrollback
3. ✅ **Animation playback** - Fully integrated in backend and frontend
4. ✅ **Unicode placeholders** - Virtual placements insert placeholder characters into grid
5. ⏳ **Performance optimization** - Consider texture caching, frame pooling for large animations

## Contributing

When adding graphics tests:
1. Add test script to `scripts/`
2. Document protocol specifics
3. Include expected output
4. Test with both par-term (GPU) and par-term-emu-tui-rust (TUI)
