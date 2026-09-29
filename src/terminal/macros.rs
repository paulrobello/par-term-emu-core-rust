//! Macro recording and playback (ARC-039: `MacroEngine` service).
//!
//! `Terminal` carried the macro library and playback state machine as
//! inherent methods even though macros are not VT state-machine behavior —
//! every such method grows the god-object's four-way sync surface (Rust impl,
//! binding macros, .pyi, API_REFERENCE). The service below owns that logic,
//! operating on a borrowed `Terminal`.

use crate::terminal::Terminal;

/// Macro library and playback operations on a [`Terminal`].
///
/// Stateless by construction — call as `MacroEngine::play_macro(&mut term, ...)`.
pub struct MacroEngine;

impl MacroEngine {
    /// Load a macro into the library
    pub fn load_macro(term: &mut Terminal, name: String, m: crate::macros::Macro) {
        term.macros.macro_library.insert(name, m);
    }

    /// Get a macro from the library
    pub fn get_macro<'a>(term: &'a Terminal, name: &str) -> Option<&'a crate::macros::Macro> {
        term.macros.macro_library.get(name)
    }

    /// Remove a macro from the library
    pub fn remove_macro(term: &mut Terminal, name: &str) -> Option<crate::macros::Macro> {
        term.macros.macro_library.remove(name)
    }

    /// List all macros in the library
    pub fn list_macros(term: &Terminal) -> Vec<String> {
        term.macros.macro_library.keys().cloned().collect()
    }

    /// Start playing a macro by name
    pub fn play_macro(term: &mut Terminal, name: &str) -> Result<(), String> {
        if let Some(m) = term.macros.macro_library.get(name).cloned() {
            term.macros.macro_playback = Some(crate::macros::MacroPlayback::new(m));
            Ok(())
        } else {
            Err(format!("Macro '{}' not found", name))
        }
    }

    /// Stop macro playback
    pub fn stop_macro(term: &mut Terminal) {
        term.macros.macro_playback = None;
        term.macros.macro_screenshot_triggers.clear();
    }

    /// Pause macro playback
    pub fn pause_macro(term: &mut Terminal) {
        if let Some(ref mut playback) = term.macros.macro_playback {
            playback.pause();
        }
    }

    /// Resume macro playback
    pub fn resume_macro(term: &mut Terminal) {
        if let Some(ref mut playback) = term.macros.macro_playback {
            playback.resume();
        }
    }

    /// Set macro playback speed
    pub fn set_macro_speed(term: &mut Terminal, speed: f64) {
        if let Some(ref mut playback) = term.macros.macro_playback {
            playback.set_speed(speed);
        }
    }

    /// Check if a macro is currently playing
    pub fn is_macro_playing(term: &Terminal) -> bool {
        term.macros
            .macro_playback
            .as_ref()
            .map(|p| !p.is_finished())
            .unwrap_or(false)
    }

    /// Check if macro playback is paused
    pub fn is_macro_paused(term: &Terminal) -> bool {
        term.macros
            .macro_playback
            .as_ref()
            .map(|p| p.is_paused())
            .unwrap_or(false)
    }

    /// Get macro playback progress
    pub fn get_macro_progress(term: &Terminal) -> Option<(usize, usize)> {
        term.macros.macro_playback.as_ref().map(|p| p.progress())
    }

    /// Get the name of the currently playing macro
    pub fn get_current_macro_name(term: &Terminal) -> Option<String> {
        term.macros
            .macro_playback
            .as_ref()
            .map(|p| p.name().to_string())
    }

    /// Tick macro playback and return events that should be processed now
    ///
    /// Returns bytes to send to PTY for KeyPress events, None for others
    /// Screenshot events are stored in macro_screenshot_triggers
    pub fn tick_macro(term: &mut Terminal) -> Option<Vec<u8>> {
        if let Some(ref mut playback) = term.macros.macro_playback {
            if let Some(event) = playback.next_event() {
                match event {
                    crate::macros::MacroEvent::KeyPress { key, .. } => {
                        let bytes = crate::macros::KeyParser::parse_key(&key);
                        return Some(bytes);
                    }
                    crate::macros::MacroEvent::Screenshot { label, .. } => {
                        term.macros
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
                term.macros.macro_playback = None;
            }
        }
        None
    }

    /// Get and clear screenshot triggers
    pub fn get_macro_screenshot_triggers(term: &mut Terminal) -> Vec<String> {
        std::mem::take(&mut term.macros.macro_screenshot_triggers)
    }

    /// Convert a RecordingSession to a Macro
    pub fn recording_to_macro(
        _term: &Terminal,
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
        assert!(MacroEngine::list_macros(&term).is_empty());

        MacroEngine::load_macro(&mut term, "greet".to_string(), make_macro("greet"));
        assert!(MacroEngine::get_macro(&term, "greet").is_some());
        assert!(MacroEngine::get_macro(&term, "missing").is_none());
        assert_eq!(MacroEngine::list_macros(&term), vec!["greet".to_string()]);

        assert!(MacroEngine::remove_macro(&mut term, "greet").is_some());
        assert!(MacroEngine::remove_macro(&mut term, "greet").is_none()); // already gone
        assert!(MacroEngine::list_macros(&term).is_empty());
    }

    #[test]
    fn play_macro_unknown_name_errors() {
        let mut term = Terminal::new(80, 24);
        assert!(MacroEngine::play_macro(&mut term, "nope").is_err());
        assert!(!MacroEngine::is_macro_playing(&term));
    }

    #[test]
    fn playback_lifecycle_pause_resume_speed_stop() {
        let mut term = Terminal::new(80, 24);
        MacroEngine::load_macro(&mut term, "m".to_string(), make_macro("m"));

        assert!(MacroEngine::play_macro(&mut term, "m").is_ok());
        assert!(MacroEngine::is_macro_playing(&term));
        assert_eq!(
            MacroEngine::get_current_macro_name(&term).as_deref(),
            Some("m")
        );

        let (done, total) = MacroEngine::get_macro_progress(&term).unwrap();
        assert!(total >= done);

        // Pause / resume toggle the paused flag.
        MacroEngine::pause_macro(&mut term);
        assert!(MacroEngine::is_macro_paused(&term));
        MacroEngine::resume_macro(&mut term);
        assert!(!MacroEngine::is_macro_paused(&term));

        // set_macro_speed only affects an active playback (no panic here).
        MacroEngine::set_macro_speed(&mut term, 2.0);

        MacroEngine::stop_macro(&mut term);
        assert!(!MacroEngine::is_macro_playing(&term));
        assert!(MacroEngine::get_macro_progress(&term).is_none());
        assert!(MacroEngine::get_macro_screenshot_triggers(&mut term).is_empty());
    }

    #[test]
    fn pause_resume_speed_are_noops_without_active_playback() {
        let mut term = Terminal::new(80, 24);
        // None of these should panic when no macro is playing.
        MacroEngine::pause_macro(&mut term);
        MacroEngine::resume_macro(&mut term);
        MacroEngine::set_macro_speed(&mut term, 0.5);
        assert!(!MacroEngine::is_macro_playing(&term));
        assert!(!MacroEngine::is_macro_paused(&term));
    }

    #[test]
    fn tick_macro_emits_key_bytes_then_screenshot_trigger() {
        let mut term = Terminal::new(80, 24);
        let mut m = Macro::new("tick");
        m.add_key("a");
        m.add_screenshot();
        MacroEngine::load_macro(&mut term, "tick".to_string(), m);
        MacroEngine::play_macro(&mut term, "tick").unwrap();

        // First event is the key press -> Some(bytes).
        let key_bytes = MacroEngine::tick_macro(&mut term);
        assert!(key_bytes.is_some(), "key event must emit bytes");

        // Second event is the screenshot -> queues a trigger, no bytes.
        assert!(MacroEngine::get_macro_screenshot_triggers(&mut term).is_empty());
        assert!(MacroEngine::tick_macro(&mut term).is_none());
        let triggers = MacroEngine::get_macro_screenshot_triggers(&mut term);
        assert_eq!(triggers.len(), 1);

        // Both events consumed -> playback auto-clears.
        assert!(!MacroEngine::is_macro_playing(&term));
    }

    #[test]
    fn tick_macro_with_no_playback_returns_none() {
        let mut term = Terminal::new(80, 24);
        assert!(MacroEngine::tick_macro(&mut term).is_none());
    }
}
