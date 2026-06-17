//! Macro recording and playback
//!
//! Provides types and Terminal implementation for recording and playing back macros.

use crate::terminal::Terminal;

impl Terminal {
    // === Macro Management ===

    /// Load a macro into the library
    pub fn load_macro(&mut self, name: String, m: crate::macros::Macro) {
        self.macros.macro_library.insert(name, m);
    }

    /// Get a macro from the library
    pub fn get_macro(&self, name: &str) -> Option<&crate::macros::Macro> {
        self.macros.macro_library.get(name)
    }

    /// Remove a macro from the library
    pub fn remove_macro(&mut self, name: &str) -> Option<crate::macros::Macro> {
        self.macros.macro_library.remove(name)
    }

    /// List all macros in the library
    pub fn list_macros(&self) -> Vec<String> {
        self.macros.macro_library.keys().cloned().collect()
    }

    // === Feature 38: Macro Recording and Playback ===

    /// Start playing a macro by name
    pub fn play_macro(&mut self, name: &str) -> Result<(), String> {
        if let Some(m) = self.macros.macro_library.get(name).cloned() {
            self.macros.macro_playback = Some(crate::macros::MacroPlayback::new(m));
            Ok(())
        } else {
            Err(format!("Macro '{}' not found", name))
        }
    }

    /// Stop macro playback
    pub fn stop_macro(&mut self) {
        self.macros.macro_playback = None;
        self.macros.macro_screenshot_triggers.clear();
    }

    /// Pause macro playback
    pub fn pause_macro(&mut self) {
        if let Some(ref mut playback) = self.macros.macro_playback {
            playback.pause();
        }
    }

    /// Resume macro playback
    pub fn resume_macro(&mut self) {
        if let Some(ref mut playback) = self.macros.macro_playback {
            playback.resume();
        }
    }

    /// Set macro playback speed
    pub fn set_macro_speed(&mut self, speed: f64) {
        if let Some(ref mut playback) = self.macros.macro_playback {
            playback.set_speed(speed);
        }
    }

    /// Check if a macro is currently playing
    pub fn is_macro_playing(&self) -> bool {
        self.macros
            .macro_playback
            .as_ref()
            .map(|p| !p.is_finished())
            .unwrap_or(false)
    }

    /// Check if macro playback is paused
    pub fn is_macro_paused(&self) -> bool {
        self.macros
            .macro_playback
            .as_ref()
            .map(|p| p.is_paused())
            .unwrap_or(false)
    }

    /// Get macro playback progress
    pub fn get_macro_progress(&self) -> Option<(usize, usize)> {
        self.macros.macro_playback.as_ref().map(|p| p.progress())
    }

    /// Get the name of the currently playing macro
    pub fn get_current_macro_name(&self) -> Option<String> {
        self.macros
            .macro_playback
            .as_ref()
            .map(|p| p.name().to_string())
    }

    /// Tick macro playback and return events that should be processed now
    ///
    /// Returns bytes to send to PTY for KeyPress events, None for others
    /// Screenshot events are stored in macro_screenshot_triggers
    pub fn tick_macro(&mut self) -> Option<Vec<u8>> {
        if let Some(ref mut playback) = self.macros.macro_playback {
            if let Some(event) = playback.next_event() {
                match event {
                    crate::macros::MacroEvent::KeyPress { key, .. } => {
                        let bytes = crate::macros::KeyParser::parse_key(&key);
                        return Some(bytes);
                    }
                    crate::macros::MacroEvent::Screenshot { label, .. } => {
                        self.macros
                            .macro_screenshot_triggers
                            .push(label.unwrap_or_else(|| "screenshot".to_string()));
                    }
                    crate::macros::MacroEvent::Delay { .. } => {
                        // Delays are handled by timing in the playback state machine
                    }
                }
            }

            // Check if playback is finished and clean up
            if playback.is_finished() {
                self.macros.macro_playback = None;
            }
        }
        None
    }

    /// Get and clear screenshot triggers
    pub fn get_macro_screenshot_triggers(&mut self) -> Vec<String> {
        std::mem::take(&mut self.macros.macro_screenshot_triggers)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::Macro;

    /// A macro with a key, a delay, another key, and a screenshot trigger.
    fn make_macro(name: &str) -> Macro {
        let mut m = Macro::new(name);
        m.add_key("a");
        m.add_delay(10);
        m.add_key("enter");
        m.add_screenshot();
        m
    }

    #[test]
    fn macro_library_load_get_remove_list() {
        let mut term = Terminal::new(80, 24);
        assert!(term.list_macros().is_empty());

        term.load_macro("greet".to_string(), make_macro("greet"));
        assert!(term.get_macro("greet").is_some());
        assert!(term.get_macro("missing").is_none());
        assert_eq!(term.list_macros(), vec!["greet".to_string()]);

        assert!(term.remove_macro("greet").is_some());
        assert!(term.remove_macro("greet").is_none()); // already gone
        assert!(term.list_macros().is_empty());
    }

    #[test]
    fn play_macro_unknown_name_errors() {
        let mut term = Terminal::new(80, 24);
        assert!(term.play_macro("nope").is_err());
        assert!(!term.is_macro_playing());
    }

    #[test]
    fn playback_lifecycle_pause_resume_speed_stop() {
        let mut term = Terminal::new(80, 24);
        term.load_macro("m".to_string(), make_macro("m"));

        assert!(term.play_macro("m").is_ok());
        assert!(term.is_macro_playing());
        assert_eq!(term.get_current_macro_name().as_deref(), Some("m"));

        let (done, total) = term.get_macro_progress().unwrap();
        assert!(total >= done);

        // Pause / resume toggle the paused flag.
        term.pause_macro();
        assert!(term.is_macro_paused());
        term.resume_macro();
        assert!(!term.is_macro_paused());

        // set_macro_speed only affects an active playback (no panic here).
        term.set_macro_speed(2.0);

        term.stop_macro();
        assert!(!term.is_macro_playing());
        assert!(term.get_macro_progress().is_none());
        assert!(term.get_macro_screenshot_triggers().is_empty());
    }

    #[test]
    fn pause_resume_speed_are_noops_without_active_playback() {
        let mut term = Terminal::new(80, 24);
        // None of these should panic when no macro is playing.
        term.pause_macro();
        term.resume_macro();
        term.set_macro_speed(0.5);
        assert!(!term.is_macro_playing());
        assert!(!term.is_macro_paused());
    }

    #[test]
    fn tick_macro_emits_key_bytes_then_screenshot_trigger() {
        let mut term = Terminal::new(80, 24);
        let mut m = Macro::new("tick");
        m.add_key("a");
        m.add_screenshot();
        term.load_macro("tick".to_string(), m);
        term.play_macro("tick").unwrap();

        // First event is the key press -> Some(bytes).
        let key_bytes = term.tick_macro();
        assert!(key_bytes.is_some(), "key event must emit bytes");

        // Second event is the screenshot -> queues a trigger, no bytes.
        assert!(term.get_macro_screenshot_triggers().is_empty());
        assert!(term.tick_macro().is_none());
        let triggers = term.get_macro_screenshot_triggers();
        assert_eq!(triggers.len(), 1);

        // Both events consumed -> playback auto-clears.
        assert!(!term.is_macro_playing());
    }

    #[test]
    fn tick_macro_with_no_playback_returns_none() {
        let mut term = Terminal::new(80, 24);
        assert!(term.tick_macro().is_none());
    }
}
