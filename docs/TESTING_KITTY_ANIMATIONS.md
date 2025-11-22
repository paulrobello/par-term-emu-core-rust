# Testing Kitty Graphics Animations

This guide explains how to test Kitty graphics protocol animation support in par-term-emu-core-rust.

## Current Status

| Component | Status | Notes |
|-----------|--------|-------|
| Backend Animation Storage | ✅ Complete | `src/graphics/animation.rs` |
| Backend Frame Management | ✅ Complete | Frames stored in `GraphicsStore.animations` |
| Backend Playback Control | ✅ Complete | Play/Pause/Stop/SetLoops |
| Frontend Rendering | 🔄 Pending | Needs integration in par-term |

## Prerequisites

Install Pillow for PNG generation:

```bash
pip install Pillow
# or with uv:
uv pip install Pillow
```

## Quick Test

Run the test script in par-term:

```bash
cd /Users/probello/Repos/par-term && cargo run
# In the terminal that opens, run:
python /Users/probello/Repos/par-term-emu-core-rust/scripts/test_kitty_animation.py
```

## What the Test Does

The test script creates two animations:

1. **Simple 2-frame animation**: Red ↔ Blue squares
   - Frame 1: Red (500ms delay)
   - Frame 2: Blue (500ms delay)
   - Demonstrates: Play, Pause, Resume, Stop

2. **Multi-frame color cycle**: Red → Yellow → Green → Blue
   - 4 frames, 400ms each
   - Demonstrates: Loop count (2 loops)

## Kitty Animation Protocol

### Frame Transmission

```
ESC _ G a=f,i=<id>,r=<frame>,z=<delay>,f=100,t=d ; <base64_png> ESC \
```

Parameters:
- `a=f` - Action: frame
- `i=<id>` - Image ID
- `r=<frame>` - Frame number (1-indexed)
- `z=<delay>` - Frame delay in milliseconds
- `f=100` - Format: PNG
- `t=d` - Transmission: direct

### Animation Control

```
ESC _ G a=a,i=<id>,s=<control> ESC \
```

Parameters:
- `a=a` - Action: animation control
- `i=<id>` - Image ID
- `s=<control>` - Control value:
  - `1` = Play/Resume
  - `2` = Pause
  - `3` = Stop
  - `0` or other number = Set loop count (0 = infinite)

## Backend Verification

Check debug logs for animation events:

```bash
# Enable debug logging
export DEBUG_LEVEL=4

# Run par-term
cd /Users/probello/Repos/par-term && cargo run

# In another terminal, check logs
tail -f /tmp/par_term_emu_core_rust_debug_rust.log
```

You should see:
- Animation frame additions
- Animation control commands
- Frame timing updates (if frontend calls `update_animations()`)

## Frontend Integration (TODO)

The frontend (par-term) needs the following integration to enable animation playback:

### 1. Call `update_animations()` periodically

In `app.rs` redraw loop (around line 1595), add:

```rust
// Update animations and get list of images with frame changes
if let Some(terminal) = &self.terminal {
    let terminal = terminal.blocking_lock();
    let pty = terminal.pty_session.lock().unwrap();
    let term_arc = pty.terminal();
    let mut term = term_arc.lock().unwrap();
    let changed_images = term.graphics_store_mut().update_animations();

    if !changed_images.is_empty() {
        // Request redraw when animation frames change
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
```

### 2. Render current animation frame

Modify graphics retrieval to use current animation frame:

```rust
// Get current graphics with animation frames
let mut graphics = Vec::new();
for graphic in terminal.all_graphics() {
    // If this graphic has an animation, get current frame
    if let Some(image_id) = graphic.kitty_image_id {
        if let Some(anim) = term.graphics_store().get_animation(image_id) {
            if let Some(current_frame) = anim.current_frame() {
                // Create graphic from current animation frame
                let mut animated_graphic = graphic.clone();
                animated_graphic.pixels = current_frame.pixels.clone();
                animated_graphic.width = current_frame.width;
                animated_graphic.height = current_frame.height;
                graphics.push(animated_graphic);
                continue;
            }
        }
    }
    // Not animated or no current frame
    graphics.push(graphic.clone());
}
```

### 3. Terminal API additions needed

The Terminal struct needs to expose `graphics_store_mut()`:

```rust
// In src/terminal/mod.rs
impl Terminal {
    pub fn graphics_store_mut(&mut self) -> &mut GraphicsStore {
        &mut self.graphics_store
    }
}
```

## Manual Testing

You can also test animations manually by sending escape sequences:

```bash
# Create a simple red PNG (requires Pillow)
python3 -c "
from PIL import Image
import io, base64
img = Image.new('RGB', (100, 100), (255, 0, 0))
buf = io.BytesIO()
img.save(buf, format='PNG')
print(base64.standard_b64encode(buf.getvalue()).decode())
" > /tmp/red.b64

# Send frame 1
printf '\x1b_Ga=T,i=1,r=1,z=500,f=100,t=d;'
cat /tmp/red.b64
printf '\x1b\\'

# Create and send blue frame 2
python3 -c "
from PIL import Image
import io, base64
img = Image.new('RGB', (100, 100), (0, 0, 255))
buf = io.BytesIO()
img.save(buf, format='PNG')
print(base64.standard_b64encode(buf.getvalue()).decode())
" | (printf '\x1b_Ga=f,i=1,r=2,z=500,f=100,t=d;'; cat; printf '\x1b\\')

# Play animation
printf '\x1b_Ga=a,i=1,s=1\x1b\\'
```

## Debugging

### Check if frames are being stored

Add debug logging in `src/graphics/kitty.rs`:

```rust
debug_info!("KITTY", "Added animation frame {} to image {}", frame_number, image_id);
```

### Check animation state

In terminal, you can query animation state (if exposed via Python bindings):

```python
from par_term_emu_core_rust import Terminal
term = Terminal(80, 24)
# ... load animations ...
anim = term.graphics_store().get_animation(1)
print(f"State: {anim.state}, Frame: {anim.current_frame}/{anim.frame_count()}")
```

## References

- [Kitty Graphics Protocol - Animation](https://sw.kovidgoyal.net/kitty/graphics-protocol/#animation)
- [Animation Implementation](../src/graphics/animation.rs)
- [Graphics Store](../src/graphics/mod.rs)
- [Kitty Parser](../src/graphics/kitty.rs)

## Known Limitations

1. **Frontend rendering**: Animations are stored but not yet rendered in par-term
2. **Frame composition**: Alpha blend vs overwrite modes need testing
3. **Performance**: Large animations may need frame caching optimization
4. **Cleanup**: Old animation data needs garbage collection strategy

## Next Steps

1. ✅ Backend animation storage - Complete
2. ✅ Backend playback control - Complete
3. 🔄 Frontend integration - In progress
4. ⏳ Performance optimization - Pending
5. ⏳ Garbage collection - Pending
