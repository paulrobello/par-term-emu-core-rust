//! Macro recording and playback
//!
//! Provides types and Terminal implementation for recording and playing back macros.

use crate::terminal::Terminal;

impl Terminal {
    // === Macro Management ===

    /// Load a macro into the library
    pub fn load_macro(&mut self, name: String, m: crate::macros::Macro) {
        self.macro_library.insert(name, m);
    }

    /// Get a macro from the library
    pub fn get_macro(&self, name: &str) -> Option<&crate::macros::Macro> {
        self.macro_library.get(name)
    }

    /// Remove a macro from the library
    pub fn remove_macro(&mut self, name: &str) -> Option<crate::macros::Macro> {
        self.macro_library.remove(name)
    }

    /// List all macros in the library
    pub fn list_macros(&self) -> Vec<String> {
        self.macro_library.keys().cloned().collect()
    }

    // === Feature 38: Macro Recording and Playback ===

    /// Start playing a macro by name
    pub fn play_macro(&mut self, name: &str) -> Result<(), String> {
        if let Some(m) = self.macro_library.get(name).cloned() {
            self.macro_playback = Some(crate::macros::MacroPlayback::new(m));
            Ok(())
        } else {
            Err(format!("Macro '{}' not found", name))
        }
    }

    /// Stop macro playback
    pub fn stop_macro(&mut self) {
        self.macro_playback = None;
        self.macro_screenshot_triggers.clear();
    }

    /// Pause macro playback
    pub fn pause_macro(&mut self) {
        if let Some(ref mut playback) = self.macro_playback {
            playback.pause();
        }
    }

    /// Resume macro playback
    pub fn resume_macro(&mut self) {
        if let Some(ref mut playback) = self.macro_playback {
            playback.resume();
        }
    }

    /// Set macro playback speed
    pub fn set_macro_speed(&mut self, speed: f64) {
        if let Some(ref mut playback) = self.macro_playback {
            playback.set_speed(speed);
        }
    }

    /// Check if a macro is currently playing
    pub fn is_macro_playing(&self) -> bool {
        self.macro_playback
            .as_ref()
            .map(|p| !p.is_finished())
            .unwrap_or(false)
    }

    /// Check if macro playback is paused
    pub fn is_macro_paused(&self) -> bool {
        self.macro_playback
            .as_ref()
            .map(|p| p.is_paused())
            .unwrap_or(false)
    }

    /// Get macro playback progress
    pub fn get_macro_progress(&self) -> Option<(usize, usize)> {
        self.macro_playback.as_ref().map(|p| p.progress())
    }

    /// Get the name of the currently playing macro
    pub fn get_current_macro_name(&self) -> Option<String> {
        self.macro_playback.as_ref().map(|p| p.name().to_string())
    }

    /// Tick macro playback and return events that should be processed now
    ///
    /// Returns bytes to send to PTY for KeyPress events, None for others
    /// Screenshot events are stored in macro_screenshot_triggers
    pub fn tick_macro(&mut self) -> Option<Vec<u8>> {
        if let Some(ref mut playback) = self.macro_playback {
            if let Some(event) = playback.next_event() {
                match event {
                    crate::macros::MacroEvent::KeyPress { key, .. } => {
                        let bytes = crate::macros::KeyParser::parse_key(&key);
                        return Some(bytes);
                    }
                    crate::macros::MacroEvent::Screenshot { label, .. } => {
                        self.macro_screenshot_triggers
                            .push(label.unwrap_or_else(|| "screenshot".to_string()));
                    }
                    crate::macros::MacroEvent::Delay { .. } => {
                        // Delays are handled by timing in the playback state machine
                    }
                }
            }

            // Check if playback is finished and clean up
            if playback.is_finished() {
                self.macro_playback = None;
            }
        }
        None
    }

    /// Get and clear screenshot triggers
    pub fn get_macro_screenshot_triggers(&mut self) -> Vec<String> {
        std::mem::take(&mut self.macro_screenshot_triggers)
    }

    /// Convert a RecordingSession to a Macro
    pub fn recording_to_macro(
        &self,
        session: &crate::terminal::RecordingSession,
        name: String,
    ) -> crate::macros::Macro {
        let mut macro_data = crate::macros::Macro::new(name)
            .with_terminal_size(session.initial_size.0, session.initial_size.1);

        // Copy environment variables
        for (k, v) in &session.env {
            macro_data = macro_data.add_env(k.clone(), v.clone());
        }

        // Convert input events to key presses
        let mut last_timestamp = 0u64;
        for event in &session.events {
            if event.event_type == crate::terminal::RecordingEventType::Input {
                // Add delay if there's a gap
                if event.timestamp > last_timestamp {
                    let delay = event.timestamp - last_timestamp;
                    macro_data.add_delay(delay);
                }

                // Convert raw bytes to a key string (basic conversion)
                let key_string = String::from_utf8_lossy(&event.data).to_string();
                macro_data.add_key(key_string);

                last_timestamp = event.timestamp;
            }
        }

        macro_data.duration = session.duration;
        macro_data
    }
}
