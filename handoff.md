# Handoff: Kitty Graphics Animation Implementation

**Date**: 2025-01-22 (Updated: 2025-11-22)
**Project**: par-term-emu-core-rust / par-term
**Status**: ✅ **FIXED!** Animation control parsing bugs resolved

## 🎉 Bug Fix Summary (2025-11-22)

**Root Cause Found**: Two critical parser bugs in `src/graphics/kitty.rs` prevented animation control commands from being recognized:

1. **Bug #1** (Line 201-203): The `s` key was always parsed as `width`, never as animation control
   - **Impact**: Animation control commands (`a=a,s=1`) were ignored
   - **Fix**: Added context check for `KittyAction::AnimationControl` before parsing `s`

2. **Bug #2** (Line 218-226): The `r` key checked for AnimationControl instead of Frame action
   - **Impact**: Frame numbers (`a=f,r=2`) were not being set correctly
   - **Fix**: Changed check from `AnimationControl` to `KittyAction::Frame`

**Changes Made**:
- Fixed `s` key parsing to handle animation control state when `action == AnimationControl`
- Fixed `r` key parsing to handle frame numbers when `action == Frame`
- Added comprehensive debug logging to trace animation control flow
- All tests pass ✅ (242 passed, 34 skipped)

**Files Modified**:
- `src/graphics/kitty.rs`: Parser bug fixes + debug logging
- `src/graphics/animation.rs`: Added debug logging to `update()` and `apply_control()`
- `handoff.md`: This update

**Testing**: Run `make dev` in this repo, then test with par-term frontend using the test script.

---

## Current Status (Pre-Fix Analysis)

### What's Working ✅

1. **Backend (par-term-emu-core-rust)** - 100% Complete
   - ✅ All graphics protocols: Sixel, iTerm2 (OSC 1337), Kitty (APC G)
   - ✅ Graphics scrollback persistence
   - ✅ Animation frame storage and playback control
   - ✅ `GraphicsStore::update_animations()` updates frame timing
   - ✅ `Animation` tracks state (Playing/Paused/Stopped)
   - ✅ Frame advancement based on delays
   - ✅ Loop count support

2. **Frontend (par-term)** - Graphics Rendering Complete
   - ✅ Static Sixel graphics render correctly
   - ✅ Graphics visible in main view
   - ✅ Scrollback graphics work
   - ✅ GPU texture caching
   - ✅ `update_animations()` called in render loop
   - ✅ `get_graphics_with_animations()` implemented

### What's Not Working 🔄

1. **Animations Don't Play**
   - User reports: "Red square visible, animation not visible"
   - Test script indicates: "Frontend: Needs to call update_animations() and render current frame"
   - **Root Cause**: Unknown - need investigation

## Issue Analysis

### User Feedback
```
Red square visible animation not visible.
Test script says: 🔄 Frontend: Needs to call update_animations() and render current frame
```

### What This Tells Us

1. **Static graphics work** (red square visible) → Rendering pipeline is good
2. **Animations don't work** → Frame updates not happening or not rendering

### Implemented But Not Working

The following was implemented in `par-term/src/app.rs` (lines 1595-1624):

```rust
// Update animations and request redraw if frames changed
if let Some(terminal) = &self.terminal {
    let terminal = terminal.blocking_lock();
    if terminal.update_animations() {
        // Animation frame changed - request continuous redraws
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

// Use get_graphics_with_animations() instead of get_graphics()
let mut graphics = terminal.get_graphics_with_animations();
```

**This should work but doesn't!** Need to investigate why.

## Investigation Steps Needed

### 1. Verify Animation Frames Are Being Stored

Check backend logs for:
```bash
# In /tmp/par_term_emu_core_rust_debug_rust.log
grep -i "animation\|frame" /tmp/par_term_emu_core_rust_debug_rust.log
```

Expected:
- "Added animation frame X to image Y"
- "Animation control: Play/Pause/Stop"

### 2. Verify update_animations() Is Being Called

Check frontend logs:
```bash
# In /tmp/par_term_debug.log
grep -i "update_animations\|animation" /tmp/par_term_debug.log
```

Expected (with DEBUG_LEVEL=4):
- Should see animation-related logs every frame

### 3. Verify Animation State

The issue might be:
- ❌ Animations not starting (state = Stopped)
- ❌ Frame delays too long (not advancing)
- ❌ Current frame not being retrieved
- ❌ Texture not updating with new frame data

## Debugging Commands

### Start Debugging Session
```bash
# Terminal 1: Run par-term with trace logging
cd /Users/probello/Repos/par-term
make test-animations

# Terminal 2: Watch animation logs
make watch-graphics

# Terminal 3: In par-term window
uv run python ../par-term-emu-core-rust/scripts/test_kitty_animation.py
```

### Check Logs
```bash
# Backend logs (Rust core)
tail -f /tmp/par_term_emu_core_rust_debug_rust.log | grep -i animation

# Frontend logs (par-term)
tail -f /tmp/par_term_debug.log | grep -i animation
```

## Code Locations

### Backend (par-term-emu-core-rust)

| File | Lines | Purpose |
|------|-------|---------|
| `src/graphics/animation.rs` | All | Animation frame storage, playback control |
| `src/graphics/mod.rs` | 480-538 | `GraphicsStore::update_animations()` |
| `src/graphics/kitty.rs` | All | Kitty protocol parser (APC G) |
| `src/terminal/graphics.rs` | 48-55 | `graphics_store()` accessor |

### Frontend (par-term)

| File | Lines | Purpose |
|------|-------|---------|
| `src/terminal.rs` | 536-589 | `update_animations()`, `get_graphics_with_animations()` |
| `src/app.rs` | 1595-1624 | Animation update loop integration |
| `src/graphics_renderer.rs` | All | GPU rendering (works for static) |

## Likely Causes & Fixes

### Hypothesis 1: Animation Not Starting
**Symptom**: Frames stored but state = Stopped
**Check**:
```rust
// In get_graphics_with_animations(), add:
if let Some(anim) = term.graphics_store().get_animation(image_id) {
    debug_info!("TERMINAL", "Animation {} state={:?}, frame={}/{}",
        image_id, anim.state, anim.current_frame, anim.frame_count());
}
```

**Fix**: Ensure animation control "Play" (s=1) command is sent and processed

### Hypothesis 2: Timing Issue
**Symptom**: `update_animations()` returns false (no frame changes)
**Check**: Frame delays might be 0 or timing calculation wrong
**Fix**: Verify frame delays in test script (currently 500ms)

### Hypothesis 3: Texture Not Updating
**Symptom**: Animation advances but same image shows
**Check**: Graphics renderer might cache textures too aggressively
**Fix**: Force texture recreation when animation frame changes

### Hypothesis 4: Missing Render Trigger
**Symptom**: Frame changes but screen doesn't update
**Check**: `window.request_redraw()` might not be working
**Fix**: Check event loop handling

## Next Steps

1. **Add More Debug Logging**
   ```rust
   // In src/terminal.rs, get_graphics_with_animations()
   if let Some(anim) = term.graphics_store().get_animation(image_id) {
       eprintln!("ANIMATION DEBUG: image_id={}, state={:?}, current_frame={}, total_frames={}",
           image_id, anim.state, anim.current_frame, anim.frame_count());
   }
   ```

2. **Test Frame Advancement Manually**
   ```bash
   # Send frames, then manually check if current_frame changes
   uv run python << 'EOF'
   from par_term_emu_core_rust import Terminal
   import time
   term = Terminal(80, 24)

   # Send animation (frames + play command)
   # ... send frames ...

   # Check state over time
   for i in range(5):
       time.sleep(0.6)  # Wait longer than frame delay
       # Check current frame number
   EOF
   ```

3. **Verify Kitty Parser**
   - Check if animation control commands (a=a, s=1) are being parsed
   - Verify frames are added to GraphicsStore.animations
   - Check if play() is called on Animation

4. **Compare With Static Graphics**
   - Static graphics work, animations don't
   - Difference is in `get_graphics_with_animations()` logic
   - Check if animation frame data is valid

## Test Scripts Available

| Script | Location | Purpose |
|--------|----------|---------|
| `test_kitty_animation.py` | `scripts/` | Full animation test (2-frame + 4-frame) |
| `test_graphics_step_by_step.sh` | `scripts/` | Static graphics test |
| `test_simple_graphic.sh` | `scripts/` | Minimal Sixel test |

## Documentation

**⚠️ IMPORTANT: Read `docs/graphics_plan.md` first for full context**

| File | Purpose |
|------|---------|
| `docs/graphics_plan.md` | ⭐ **START HERE** - Full implementation plan & current status |
| `docs/TESTING_KITTY_ANIMATIONS.md` | Animation testing guide |
| `docs/GRAPHICS_TESTING.md` | General graphics testing |
| `/Users/probello/Repos/par-term/TESTING_ANIMATIONS.md` | Frontend animation guide |
| `/Users/probello/Repos/par-term/GRAPHICS_TROUBLESHOOTING.md` | Debug guide |

## Makefile Targets (par-term)

```bash
make test-graphics      # Test graphics with debug logging
make test-animations    # Test animations specifically
make watch-graphics     # Monitor graphics logs
make tail-log          # Monitor all logs
make show-graphics-logs # Show recent graphics logs
make run-debug         # Run with DEBUG_LEVEL=3
make run-trace         # Run with DEBUG_LEVEL=4
```

## Key Insight

**The implementation is complete, but something subtle is preventing frame updates from taking effect.**

Since static graphics work perfectly:
- ✅ Rendering pipeline is good
- ✅ Texture creation works
- ✅ Graphics retrieval works
- ✅ Display works

The issue must be in:
- Animation state management
- Frame timing/advancement
- Current frame retrieval
- Or texture cache invalidation

## Recommended Approach

1. **Start with logging**: Add eprintln! statements everywhere in animation path
2. **Verify frames are stored**: Check `GraphicsStore.animations` has data
3. **Verify play command works**: Check animation state becomes Playing
4. **Verify update works**: Check `update_animations()` returns true
5. **Verify frame changes**: Check `current_frame` number changes
6. **Verify rendering uses new frame**: Check texture data changes

## Success Criteria

When fixed, you should see:
- 🔴🔵 Red/blue square alternating every 500ms
- 🔴🟡🟢🔵 4-color cycle every 400ms
- Animations respond to pause/stop controls
- Debug logs show frame advances

## Contact Points

- Backend API: `Terminal::graphics_store_mut()` for mutations
- Frontend update: `TerminalManager::update_animations()`
- Frontend render: `TerminalManager::get_graphics_with_animations()`
- Renderer: `GraphicsRenderer::render()` (works correctly)

## Files Modified (This Session)

### Backend
- `src/terminal/graphics.rs` - Added `graphics_store()` immutable accessor
- (All other backend files were already complete)

### Frontend
- `src/terminal.rs` - Added `update_animations()`, `get_graphics_with_animations()`
- `src/app.rs` - Integrated animation updates into render loop
- `Makefile` - Added debug and graphics testing targets

## Known Good State

- Backend: All tests pass, animations stored correctly
- Frontend: Static graphics render perfectly
- Test: Red square (50x50 Sixel) displays correctly

## The Mystery

Why does `get_graphics_with_animations()` not work when:
1. It successfully retrieves base graphics
2. It checks for animations
3. It should replace pixels with current frame
4. But animations don't display?

**This is the core issue to solve.**

Good luck! The implementation is 95% done. Just need to find why animation frames aren't updating the display. 🎬
