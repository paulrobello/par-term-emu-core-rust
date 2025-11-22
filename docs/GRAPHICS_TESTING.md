# Graphics Testing Guide

Quick reference for testing graphics support in par-term-emu-core-rust.

## Supported Protocols

| Protocol | Status | Format | Test Script |
|----------|--------|--------|-------------|
| Sixel | ✅ Complete | DCS | Built-in examples |
| iTerm2 Inline | ✅ Complete | OSC 1337 | Use `imgcat` |
| Kitty Graphics | ✅ Complete | APC G | See below |
| Kitty Animation | 🔄 Backend only | APC G | [`test_kitty_animation.py`](../scripts/test_kitty_animation.py) |

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
cd /Users/probello/Repos/par-term && cargo run
# In the terminal:
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

**Frontend integration incomplete** - Animations are stored in backend but need:
1. Periodic `update_animations()` calls to advance frames
2. Rendering current animation frame instead of static image

See: [TESTING_KITTY_ANIMATIONS.md](TESTING_KITTY_ANIMATIONS.md#frontend-integration-todo)

## Performance Testing

```bash
# Send many graphics quickly
for i in {1..100}; do
    echo -e '\x1bPq"1;1;50;50#0;2;0;0;0#1;2;100;0;0#1!50~-\x1b\\'
done

# Check memory usage
ps aux | grep par-term

# Check graphics count
# (Requires Python bindings)
python -c "
from par_term_emu_core_rust import Terminal
term = Terminal(80, 24)
print(f'Graphics count: {term.graphics_count()}')
print(f'Scrollback graphics: {len(term.all_scrollback_graphics())}')
"
```

## Architecture Documentation

For implementation details, see:
- [Graphics Architecture Plan](graphics_plan.md) - Full design and implementation status
- [Testing Kitty Animations](TESTING_KITTY_ANIMATIONS.md) - Animation-specific testing
- [Security Considerations](SECURITY.md) - File loading and resource limits

## Example Images

Test images are available in the repository:

```
images/
├── test.png          # Basic test image
├── test.jpg          # JPEG test image
└── animated.gif      # GIF test (iTerm2)
```

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
3. 🔄 **Animation playback** - Needs frontend integration
4. ⏳ **Unicode placeholders** - Backend ready, needs testing
5. ⏳ **Performance optimization** - Texture caching, frame pooling

## Contributing

When adding graphics tests:
1. Add test script to `scripts/`
2. Document protocol specifics
3. Include expected output
4. Test with both par-term (GPU) and par-term-emu-tui-rust (TUI)
