# Handoff: Kitty Graphics Animation Debugging

**Date**: 2025-11-22
**Project**: par-term-emu-core-rust / par-term
**Status**: 🔄 **Animation frames load but not displayed - architectural issue found**

## Critical Issue Found

**Problem**: Animation frames are being stored correctly, but there's no placement to display them.

### Root Cause
The Kitty graphics protocol has two separate concepts:
1. **Shared Images** (`shared_images` HashMap) - Reusable image data referenced by ID
2. **Placements** (`placements` Vec) - Visual instances of images on screen
3. **Animations** (`animations` HashMap) - Frame data for animated images

**Current Bug**: Animation frames (`a=f`) only add data to the `animations` HashMap. They don't:
- Create a shared image entry
- Create a placement to display
- Have any visual representation

When we call `display_image()` with `a=p`, it fails with "Image not found" because there's no shared image with that ID.

### Evidence from Logs
```
[INFO ] [ANIMATION] Adding frame 1 to image_id=42 (delay=500ms, size=100x100)
[INFO ] [GRAPHICS] Added animation frame 1 to image_id=42 (total frames: 1)
[DEBUG] [KITTY] Kitty command processed (no graphic created)  ← No placement!
[DEBUG] [KITTY] Started Kitty graphics sequence
[DEBUG] [KITTY] Failed to build Kitty graphic: Kitty protocol error: Image not found
```

## What's Working ✅

1. **Parser Fixes** - All committed
   - ✅ Fixed `s` key to handle AnimationControl action
   - ✅ Fixed `r` key to handle Frame action (frame numbers)
   - ✅ Base64 decoder accepts both padded and unpadded data
   - ✅ Animation frames are correctly parsed and stored
   - ✅ Animation control commands (play/pause/stop) work
   - ✅ Frame timing and `update_animations()` work

2. **Test Script Fixed**
   - ✅ Changed from `a=T` to `a=f` for all frames
   - ✅ Calls `display_image()` after first frame

3. **Debug Logging**
   - ✅ Comprehensive logging added throughout animation path
   - ✅ Can trace frame additions, control commands, and updates

## What's NOT Working 🔄

**No visual display of animations** because:

1. **Animation frames don't create placements** - They only store frame data
2. **No shared image created** - `a=p` (Put) command fails
3. **No first-frame display** - Nothing visible to animate

## Architecture Issue

### Current Flow (BROKEN)
```
a=f,r=1 → Add frame 1 to animations[42] → No placement created
a=f,r=2 → Add frame 2 to animations[42] → No placement created
a=p,i=42 → Try to display image 42 → ERROR: Image not found
```

### What Should Happen
According to Kitty spec, animations need:
1. **First frame** creates both:
   - Animation entry in `animations`
   - Shared image in `shared_images`
   - Placement in `placements` (with `a=T` or first `a=f` + `a=p`)

2. **Subsequent frames** add to animation only

3. **Display command** (`a=p`) references the existing placement

### Possible Solutions

#### Option A: First Frame Creates Placement
Modify `KittyAction::Frame` handler to:
- If `frame_number == 1`: Create a shared image + placement
- Store ALL frames (including 1) in animations
- `a=p` then works because shared image exists

#### Option B: Explicit Display Command
Keep frames separate, but:
- Add `a=T` or `a=t` before animation frames to create base image
- Then `a=f` commands add animation frames
- `a=p` displays the animated placement

#### Option C: Auto-Create on First Frame
When first animation frame arrives:
- Automatically create TerminalGraphic placement
- Use frame 1 pixel data as initial display
- Store in both `placements` and `animations`

## Recommended Approach

**Option A** seems most correct per Kitty spec. Modify `/Users/probello/Repos/par-term-emu-core-rust/src/graphics/kitty.rs` around line 475-520:

```rust
KittyAction::Frame => {
    // ... existing frame decode logic ...

    let frame_num = self.frame_number.unwrap_or(1);
    let mut frame = AnimationFrame::new(frame_num, pixels.clone(), width, height);

    // ... set delays, offsets, composition ...

    // Add frame to animation
    store.add_animation_frame(image_id, frame);

    // NEW: If this is frame 1, also create a placement for display
    if frame_num == 1 {
        // Store as shared image
        store.store_kitty_image(image_id, pixels.clone());

        // Create placement to display the animation
        let graphic = TerminalGraphic {
            id: next_graphic_id(),
            protocol: GraphicProtocol::Kitty,
            position: (cursor_col, cursor_row),
            width,
            height,
            pixels: Arc::new(pixels),
            // ... other fields ...
            kitty_image_id: Some(image_id),
            kitty_placement_id: self.placement_id,
        };

        store.add_graphic(graphic);
        return Ok(Some(created_graphic_with_cursor_advance));
    }

    Ok(None)
}
```

## Code Locations

### Backend (par-term-emu-core-rust)
| File | Lines | Purpose |
|------|-------|---------|
| `src/graphics/kitty.rs` | 475-520 | **FIX HERE**: Frame action handler |
| `src/graphics/mod.rs` | 505-513 | `add_animation_frame()` |
| `src/graphics/animation.rs` | 172-178 | `Animation::add_frame()` |
| `src/terminal/graphics.rs` | 48-55 | Graphics store accessor |

### Test Scripts
| File | Status |
|------|--------|
| `scripts/test_kitty_animation.py` | ✅ Fixed - uses `a=f` for all frames |

## Testing After Fix

1. Apply Option A changes to `kitty.rs`
2. Rebuild: `make dev`
3. Rebuild par-term: `cd /Users/probello/Repos/par-term && cargo build`
4. Test: `DEBUG_LEVEL=2 make run-debug`
5. Run: `uv run python ../par-term-emu-core-rust/scripts/test_kitty_animation.py`

Expected behavior:
- ✅ Red square appears (frame 1 placement)
- ✅ Animation plays: red ↔ blue alternating every 500ms
- ✅ `update_animations()` advances frames
- ✅ Frontend renders current frame from animation

## Debug Logs

Check `/tmp/par_term_emu_core_rust_debug_rust.log` for:
```bash
grep -i "adding frame\|kitty.*image\|placement" /tmp/par_term_emu_core_rust_debug_rust.log
```

Should see:
- "Adding frame 1 to image_id=42"
- "Kitty image at (col, row)" ← NEW after fix
- "Animation advanced frame 1 -> 2" ← When playing

## Related Documentation

- **Kitty Spec**: https://sw.kovidgoyal.net/kitty/graphics-protocol/#animation
- `docs/graphics_plan.md` - Architecture overview
- `docs/TESTING_KITTY_ANIMATIONS.md` - Animation testing guide
- `/Users/probello/Repos/par-term/TESTING_ANIMATIONS.md` - Frontend guide

## Commits Made

1. `d67998d` - Fixed animation control parsing (`s` and `r` keys)
2. `ee45e81` - Fixed base64 to support padded and unpadded
3. `b921797` - Fixed test script to use `a=f` for all frames + added debug logging

## Next Developer: Start Here

1. **Read `docs/graphics_plan.md`** for full architecture context
2. **Review this handoff** for current issue
3. **Implement Option A** in `src/graphics/kitty.rs` (Frame handler)
4. **Test with the fixed test script**
5. **Verify animations play** in par-term frontend

The parser bugs are fixed. The architecture just needs frame 1 to create a placement. That's the missing piece! 🎯
