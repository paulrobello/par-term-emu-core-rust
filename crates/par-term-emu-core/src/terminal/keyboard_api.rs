//! Kitty keyboard protocol flags and modifyOtherKeys mode (ARC-003).

use super::Terminal;

impl Terminal {
    /// Process a buffered Sixel command (color, raster, repeat)
    /// Get current Kitty keyboard protocol flags
    pub fn keyboard_flags(&self) -> u16 {
        self.keyboard_state.keyboard_flags
    }

    /// Push keyboard flags to stack
    pub fn push_keyboard_flags(&mut self, flags: u16) {
        self.keyboard_state
            .keyboard_stack
            .push(self.keyboard_state.keyboard_flags);
        self.keyboard_state.keyboard_flags = flags;
    }

    /// Pop keyboard flags from stack
    pub fn pop_keyboard_flags(&mut self, count: usize) {
        for _ in 0..count {
            if let Some(flags) = self.keyboard_state.keyboard_stack.pop() {
                self.keyboard_state.keyboard_flags = flags;
            }
        }
    }

    /// Set Kitty keyboard protocol flags (for testing/direct control)
    pub fn set_keyboard_flags(&mut self, flags: u16) {
        self.keyboard_state.keyboard_flags = flags;
    }

    /// Get modifyOtherKeys mode (XTerm extension)
    /// 0 = disabled, 1 = report modifiers for special keys, 2 = report modifiers for all keys
    pub fn modify_other_keys_mode(&self) -> u8 {
        self.keyboard_state.modify_other_keys_mode
    }

    /// Set modifyOtherKeys mode (for testing/direct control)
    pub fn set_modify_other_keys_mode(&mut self, mode: u8) {
        // Clamp to valid range (0-2)
        self.keyboard_state.modify_other_keys_mode = mode.min(2);
    }
}
