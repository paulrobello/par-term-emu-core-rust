//! Animation support for Kitty graphics protocol
//!
//! This module handles animation frames and playback control for
//! the Kitty graphics protocol.
//!
//! Reference: https://sw.kovidgoyal.net/kitty/graphics-protocol/#animation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Animation frame data
#[derive(Debug, Clone)]
pub struct AnimationFrame {
    /// Frame number (1-indexed)
    pub frame_number: u32,
    /// Frame pixel data (RGBA)
    pub pixels: Arc<Vec<u8>>,
    /// Frame width in pixels
    pub width: usize,
    /// Frame height in pixels
    pub height: usize,
    /// Frame delay in milliseconds (0 = use animation default)
    pub delay_ms: u32,
    /// Gap before this frame (left offset in pixels)
    pub x_offset: u32,
    /// Gap before this frame (top offset in pixels)
    pub y_offset: u32,
    /// Composition mode
    pub composition: CompositionMode,
}

impl AnimationFrame {
    /// Create a new animation frame
    pub fn new(frame_number: u32, pixels: Vec<u8>, width: usize, height: usize) -> Self {
        Self {
            frame_number,
            pixels: Arc::new(pixels),
            width,
            height,
            delay_ms: 0,
            x_offset: 0,
            y_offset: 0,
            composition: CompositionMode::default(),
        }
    }

    /// Set frame delay
    pub fn with_delay(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// Set frame offset
    pub fn with_offset(mut self, x: u32, y: u32) -> Self {
        self.x_offset = x;
        self.y_offset = y;
        self
    }

    /// Set composition mode
    pub fn with_composition(mut self, mode: CompositionMode) -> Self {
        self.composition = mode;
        self
    }
}

/// Frame composition mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositionMode {
    /// Alpha blend with previous frame
    #[default]
    AlphaBlend,
    /// Overwrite previous frame
    Overwrite,
}

impl CompositionMode {
    /// Parse composition mode from Kitty protocol value
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '0' => Some(CompositionMode::AlphaBlend),
            '1' => Some(CompositionMode::Overwrite),
            _ => None,
        }
    }
}

/// Animation state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationState {
    /// Animation is stopped (default)
    #[default]
    Stopped,
    /// Animation is playing
    Playing,
    /// Animation is paused
    Paused,
}

/// Animation playback control (per Kitty graphics protocol spec)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationControl {
    /// Stop the animation (s=1)
    Stop,
    /// Loading mode - wait for more frames instead of looping (s=2)
    LoadingMode,
    /// Enable normal looping after reaching final frame (s=3)
    EnableLooping,
}

impl AnimationControl {
    /// Parse animation control from Kitty protocol s= value
    /// According to spec:
    /// - s=1: stop animation
    /// - s=2: loading mode (wait for frames, don't loop)
    /// - s=3: enable looping
    pub fn from_value(value: &str) -> Option<Self> {
        match value {
            "1" => Some(AnimationControl::Stop),
            "2" => Some(AnimationControl::LoadingMode),
            "3" => Some(AnimationControl::EnableLooping),
            _ => None,
        }
    }
}

/// Animation data for an image
#[derive(Debug, Clone)]
pub struct Animation {
    /// Image ID this animation belongs to
    pub image_id: u32,
    /// All frames indexed by frame number
    pub frames: HashMap<u32, AnimationFrame>,
    /// Default frame delay in milliseconds
    pub default_delay_ms: u32,
    /// Current playback state
    pub state: AnimationState,
    /// Current frame number (1-indexed)
    pub current_frame: u32,
    /// Number of loops (0 = infinite)
    pub loop_count: u32,
    /// Loops completed
    pub loops_completed: u32,
    /// Time when current frame started (for timing)
    pub frame_start_time: Option<std::time::Instant>,
}

impl Animation {
    /// Create a new animation for an image
    pub fn new(image_id: u32, default_delay_ms: u32) -> Self {
        Self {
            image_id,
            frames: HashMap::new(),
            default_delay_ms,
            state: AnimationState::Stopped,
            current_frame: 1,
            loop_count: 0,
            loops_completed: 0,
            frame_start_time: None,
        }
    }

    /// Add a frame to the animation
    pub fn add_frame(&mut self, frame: AnimationFrame) {
        let frame_num = frame.frame_number;
        debug_info!(
            "ANIMATION",
            "Adding frame {} to image_id={} (delay={}ms, size={}x{})",
            frame_num,
            self.image_id,
            frame.delay_ms,
            frame.width,
            frame.height
        );
        self.frames.insert(frame.frame_number, frame);
        debug_info!(
            "ANIMATION",
            "Frame {} added. Total frames in animation: {}",
            frame_num,
            self.frames.len()
        );
    }

    /// Get a frame by number
    pub fn get_frame(&self, frame_number: u32) -> Option<&AnimationFrame> {
        self.frames.get(&frame_number)
    }

    /// Get the current frame
    pub fn current_frame(&self) -> Option<&AnimationFrame> {
        self.frames.get(&self.current_frame)
    }

    /// Get total number of frames
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Start or resume animation
    pub fn play(&mut self) {
        if self.state != AnimationState::Playing {
            self.state = AnimationState::Playing;
            if self.frame_start_time.is_none() {
                self.frame_start_time = Some(std::time::Instant::now());
            }
        }
    }

    /// Pause animation
    pub fn pause(&mut self) {
        self.state = AnimationState::Paused;
    }

    /// Stop animation and reset to first frame
    pub fn stop(&mut self) {
        self.state = AnimationState::Stopped;
        self.current_frame = 1;
        self.loops_completed = 0;
        self.frame_start_time = None;
    }

    /// Set loop count (0 = infinite)
    pub fn set_loops(&mut self, count: u32) {
        self.loop_count = count;
    }

    /// Update animation state and advance frame if needed
    ///
    /// Returns true if the frame changed
    pub fn update(&mut self) -> bool {
        if self.state != AnimationState::Playing {
            debug_trace!(
                "ANIMATION",
                "image_id={} update: not playing (state={:?})",
                self.image_id,
                self.state
            );
            return false;
        }

        let current_frame = match self.current_frame() {
            Some(f) => f,
            None => {
                debug_log!(
                    "ANIMATION",
                    "image_id={} update: no current frame (frame={})",
                    self.image_id,
                    self.current_frame
                );
                return false;
            }
        };

        let frame_delay = if current_frame.delay_ms > 0 {
            current_frame.delay_ms
        } else {
            self.default_delay_ms
        };

        if frame_delay == 0 {
            debug_log!(
                "ANIMATION",
                "image_id={} update: no frame delay",
                self.image_id
            );
            return false; // No animation timing
        }

        let frame_start = match self.frame_start_time {
            Some(t) => t,
            None => {
                debug_log!(
                    "ANIMATION",
                    "image_id={} update: no frame start time",
                    self.image_id
                );
                return false;
            }
        };

        let elapsed = frame_start.elapsed();
        if elapsed < Duration::from_millis(frame_delay as u64) {
            debug_trace!(
                "ANIMATION",
                "image_id={} update: not ready (elapsed={:?}, delay={}ms)",
                self.image_id,
                elapsed,
                frame_delay
            );
            return false; // Not time to advance yet
        }

        // Advance to next frame
        let old_frame = self.current_frame;
        self.current_frame += 1;

        // Check if we've completed all frames
        if self.current_frame > self.frames.len() as u32 {
            self.current_frame = 1;
            self.loops_completed += 1;

            // Check if we should stop looping
            // loop_count is set to (num_plays - 1), so we stop after (num_plays - 1) additional loops
            // which gives us num_plays total plays
            if self.loop_count > 0 && self.loops_completed > self.loop_count {
                debug_info!(
                    "ANIMATION",
                    "image_id={} completed all loops, stopping",
                    self.image_id
                );
                self.stop();
                return false;
            }
        }

        debug_info!(
            "ANIMATION",
            "image_id={} advanced frame {} -> {} (delay={}ms, elapsed={:?}ms)",
            self.image_id,
            old_frame,
            self.current_frame,
            frame_delay,
            elapsed.as_millis()
        );
        self.frame_start_time = Some(std::time::Instant::now());
        true
    }

    /// Apply animation control command
    /// Per Kitty spec:
    /// - Stop: stops animation and resets loop counter
    /// - LoadingMode: waits for more frames, doesn't loop
    /// - EnableLooping: starts/resumes normal looping playback
    pub fn apply_control(&mut self, control: AnimationControl) {
        debug_info!(
            "ANIMATION",
            "Applying control to image_id={}: {:?} (current state={:?}, frame={}/{})",
            self.image_id,
            control,
            self.state,
            self.current_frame,
            self.frame_count()
        );
        match control {
            AnimationControl::Stop => {
                // Stop animation and reset loop counter
                self.stop();
                self.loops_completed = 0;
            }
            AnimationControl::LoadingMode => {
                // Pause and wait for more frames
                self.pause();
            }
            AnimationControl::EnableLooping => {
                // Start/resume normal looping playback
                self.play();
            }
        }
        debug_info!(
            "ANIMATION",
            "After control: image_id={} state={:?}, frame={}/{}",
            self.image_id,
            self.state,
            self.current_frame,
            self.frame_count()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_composition_mode() {
        assert_eq!(
            CompositionMode::from_char('0'),
            Some(CompositionMode::AlphaBlend)
        );
        assert_eq!(
            CompositionMode::from_char('1'),
            Some(CompositionMode::Overwrite)
        );
        assert_eq!(CompositionMode::from_char('x'), None);
    }

    #[test]
    fn test_animation_control() {
        // Per Kitty spec: s=1 stop, s=2 loading, s=3 enable looping
        assert_eq!(
            AnimationControl::from_value("1"),
            Some(AnimationControl::Stop)
        );
        assert_eq!(
            AnimationControl::from_value("2"),
            Some(AnimationControl::LoadingMode)
        );
        assert_eq!(
            AnimationControl::from_value("3"),
            Some(AnimationControl::EnableLooping)
        );
        // Other values return None (loop counts use v= parameter now)
        assert_eq!(AnimationControl::from_value("5"), None);
    }

    #[test]
    fn test_animation_basic() {
        let mut anim = Animation::new(1, 100);
        assert_eq!(anim.state, AnimationState::Stopped);
        assert_eq!(anim.current_frame, 1);

        // Add frames
        anim.add_frame(AnimationFrame::new(1, vec![255u8; 16], 2, 2));
        anim.add_frame(AnimationFrame::new(2, vec![128u8; 16], 2, 2));
        assert_eq!(anim.frame_count(), 2);

        // Start animation
        anim.play();
        assert_eq!(anim.state, AnimationState::Playing);

        // Pause
        anim.pause();
        assert_eq!(anim.state, AnimationState::Paused);

        // Stop
        anim.stop();
        assert_eq!(anim.state, AnimationState::Stopped);
        assert_eq!(anim.current_frame, 1);
    }

    #[test]
    fn test_animation_frame_builder() {
        let frame = AnimationFrame::new(1, vec![255u8; 16], 2, 2)
            .with_delay(50)
            .with_offset(10, 20)
            .with_composition(CompositionMode::Overwrite);

        assert_eq!(frame.frame_number, 1);
        assert_eq!(frame.delay_ms, 50);
        assert_eq!(frame.x_offset, 10);
        assert_eq!(frame.y_offset, 20);
        assert_eq!(frame.composition, CompositionMode::Overwrite);
    }

    #[test]
    fn test_animation_loops() {
        let mut anim = Animation::new(1, 100);
        anim.add_frame(AnimationFrame::new(1, vec![255u8; 16], 2, 2));
        anim.add_frame(AnimationFrame::new(2, vec![128u8; 16], 2, 2));

        anim.set_loops(2);
        assert_eq!(anim.loop_count, 2);

        anim.play();
        assert_eq!(anim.state, AnimationState::Playing);
    }
}
