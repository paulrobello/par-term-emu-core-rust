//! Macro recording and playback functionality
//!
//! This module provides keyboard macro recording and playback with YAML serialization.
//! Macros can contain keyboard events, delays, and screenshot triggers.
//!
//! ## Example
//!
//! YAML save/load needs the `macro-yaml` feature (enabled by `python` and
//! `streaming-bin`).
//!
//! ```rust,no_run
//! use par_term_emu_core_rust::macros::{Macro, MacroEvent};
//! # fn main() -> std::io::Result<()> {
//! # #[cfg(feature = "macro-yaml")]
//! # {
//! let mut macro_seq = Macro::new("Test Macro");
//! macro_seq.add_key("ctrl+c");
//! macro_seq.add_delay(100);
//! macro_seq.add_screenshot();
//!
//! // Save to YAML
//! macro_seq.save_yaml("/path/to/macro.yaml")?;
//!
//! // Load from YAML
//! let loaded = Macro::load_yaml("/path/to/macro.yaml")?;
//! # }
//! # Ok(())
//! # }
//! ```

use crate::keyboard::{self, modifiers, TermKey, TermKeyEvent};
use crate::terminal::Terminal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "macro-yaml")]
use std::{fs, io, path::Path};

/// A single macro event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MacroEvent {
    /// Keyboard input with friendly key name
    #[serde(rename = "key")]
    KeyPress {
        /// Friendly key combination (e.g., "ctrl+shift+s", "a", "enter")
        key: String,
        /// Timestamp offset from macro start (milliseconds)
        timestamp: u64,
    },
    /// Delay/pause in playback
    #[serde(rename = "delay")]
    Delay {
        /// Duration in milliseconds
        duration: u64,
        /// Timestamp offset from macro start (milliseconds)
        timestamp: u64,
    },
    /// Trigger a screenshot
    #[serde(rename = "screenshot")]
    Screenshot {
        /// Optional screenshot filename/label
        label: Option<String>,
        /// Timestamp offset from macro start (milliseconds)
        timestamp: u64,
    },
}

/// A macro recording session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macro {
    /// Macro name/title
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Creation timestamp (UNIX epoch milliseconds)
    pub created: u64,
    /// Terminal size when recorded (cols, rows)
    pub terminal_size: Option<(usize, usize)>,
    /// Environment variables captured during recording
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// Recorded events
    pub events: Vec<MacroEvent>,
    /// Total duration (milliseconds)
    pub duration: u64,
}

impl Macro {
    /// Create a new empty macro
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            terminal_size: None,
            env: HashMap::new(),
            events: Vec::new(),
            duration: 0,
        }
    }

    /// Add a key press event with friendly key name
    ///
    /// Supported formats:
    /// - Single keys: "a", "enter", "escape", "tab", "backspace", "space"
    /// - Modified keys: "ctrl+c", "shift+tab", "alt+f4", "ctrl+shift+s"
    /// - Function keys: "f1", "f12"
    /// - Arrow keys: "up", "down", "left", "right"
    pub fn add_key(&mut self, key: impl Into<String>) -> &mut Self {
        let timestamp = self.events.last().map(|e| e.timestamp()).unwrap_or(0);
        self.events.push(MacroEvent::KeyPress {
            key: key.into(),
            timestamp,
        });
        self
    }

    /// Add a delay event
    pub fn add_delay(&mut self, duration_ms: u64) -> &mut Self {
        let timestamp = self.events.last().map(|e| e.timestamp()).unwrap_or(0);
        self.events.push(MacroEvent::Delay {
            duration: duration_ms,
            timestamp: timestamp + duration_ms,
        });
        self.duration = timestamp + duration_ms;
        self
    }

    /// Add a screenshot trigger
    pub fn add_screenshot(&mut self) -> &mut Self {
        self.add_screenshot_labeled(None)
    }

    /// Add a screenshot trigger with a label
    pub fn add_screenshot_labeled(&mut self, label: Option<String>) -> &mut Self {
        let timestamp = self.events.last().map(|e| e.timestamp()).unwrap_or(0);
        self.events
            .push(MacroEvent::Screenshot { label, timestamp });
        self
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the terminal size
    pub fn with_terminal_size(mut self, cols: usize, rows: usize) -> Self {
        self.terminal_size = Some((cols, rows));
        self
    }

    /// Add an environment variable
    pub fn add_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

/// YAML persistence (ARC-116): only with the `macro-yaml` feature, so slim
/// profiles do not compile `serde_yaml_ng`.
#[cfg(feature = "macro-yaml")]
impl Macro {
    /// Save the macro to a YAML file
    pub fn save_yaml<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let yaml = serde_yaml_ng::to_string(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, yaml)
    }

    /// Load a macro from a YAML file
    pub fn load_yaml<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        serde_yaml_ng::from_str(&contents)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Convert to YAML string
    pub fn to_yaml(&self) -> Result<String, serde_yaml_ng::Error> {
        serde_yaml_ng::to_string(self)
    }

    /// Parse from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(yaml)
    }
}

impl MacroEvent {
    /// Get the timestamp of this event
    pub fn timestamp(&self) -> u64 {
        match self {
            MacroEvent::KeyPress { timestamp, .. } => *timestamp,
            MacroEvent::Delay { timestamp, .. } => *timestamp,
            MacroEvent::Screenshot { timestamp, .. } => *timestamp,
        }
    }
}

/// Key name parser and converter.
///
/// Key names resolve to a [`TermKeyEvent`] and are encoded by the shared
/// key encoder ([`crate::keyboard`]), so macro playback sends the bytes a
/// real keypress would. Unknown names are sent as their literal bytes.
pub struct KeyParser;

impl KeyParser {
    /// Parse a friendly key name into the bytes a freshly reset terminal
    /// expects (legacy xterm encoding, normal cursor keys).
    ///
    /// Playback into a live terminal should use [`Self::encode_key`], which
    /// honors the application's negotiated modes.
    pub fn parse_key(key: &str) -> Vec<u8> {
        match Self::key_event(key) {
            Some(ev) => keyboard::encode_key_default(&ev),
            None => key.as_bytes().to_vec(),
        }
    }

    /// Encode a friendly key name against `term`'s negotiated input state:
    /// application cursor keys (DECCKM), kitty keyboard flags and
    /// modifyOtherKeys, exactly as [`keyboard::encode_key`] does for a
    /// frontend keypress.
    pub fn encode_key(key: &str, term: &Terminal) -> Vec<u8> {
        match Self::key_event(key) {
            Some(ev) => keyboard::encode_key(&ev, term),
            None => key.as_bytes().to_vec(),
        }
    }

    /// Resolve `ctrl+`/`alt+`/`shift+` modifiers (case-insensitive) and the
    /// final key name. `None` means an unknown name, sent literally.
    fn key_event(key: &str) -> Option<TermKeyEvent> {
        let key_lower = key.to_lowercase();
        let parts: Vec<&str> = key_lower.split('+').collect();

        let mut mods = 0;
        for (name, bit) in [
            ("ctrl", modifiers::CTRL),
            ("alt", modifiers::ALT),
            ("shift", modifiers::SHIFT),
        ] {
            if parts.contains(&name) {
                mods |= bit;
            }
        }

        // The main key is the last part.
        let main_key = parts.last().copied().unwrap_or("");

        // Single-byte (ASCII) key. A multi-byte character is not a key
        // name, so it is sent as the original bytes.
        if let [byte] = *main_key.as_bytes() {
            return Some(TermKeyEvent::char_(char::from(byte), mods));
        }

        let key = match main_key {
            "f1" => TermKey::F1,
            "f2" => TermKey::F2,
            "f3" => TermKey::F3,
            "f4" => TermKey::F4,
            "f5" => TermKey::F5,
            "f6" => TermKey::F6,
            "f7" => TermKey::F7,
            "f8" => TermKey::F8,
            "f9" => TermKey::F9,
            "f10" => TermKey::F10,
            "f11" => TermKey::F11,
            "f12" => TermKey::F12,
            "up" => TermKey::Up,
            "down" => TermKey::Down,
            "right" => TermKey::Right,
            "left" => TermKey::Left,
            "home" => TermKey::Home,
            "end" => TermKey::End,
            "pageup" | "pgup" => TermKey::PageUp,
            "pagedown" | "pgdn" => TermKey::PageDown,
            "insert" | "ins" => TermKey::Insert,
            "delete" | "del" => TermKey::Delete,
            "enter" | "return" => TermKey::Enter,
            "tab" => TermKey::Tab,
            "backspace" => TermKey::Backspace,
            "escape" | "esc" => TermKey::Escape,
            "space" => return Some(TermKeyEvent::char_(' ', mods)),
            _ => return None,
        };
        Some(TermKeyEvent::functional(key, mods))
    }
}

/// Macro playback state machine
#[derive(Debug, Clone)]
pub struct MacroPlayback {
    /// The macro being played
    macro_data: Macro,
    /// Current event index
    current_index: usize,
    /// Playback start time (milliseconds)
    start_time: u64,
    /// Speed multiplier (1.0 = normal, 2.0 = double speed, 0.5 = half speed)
    speed: f64,
    /// Whether playback is paused
    paused: bool,
    /// Time spent paused (milliseconds)
    paused_time: u64,
    /// When pause started (milliseconds)
    pause_start: Option<u64>,
}

impl MacroPlayback {
    /// Create a new playback session
    pub fn new(macro_data: Macro) -> Self {
        Self {
            macro_data,
            current_index: 0,
            start_time: Self::current_time_ms(),
            speed: 1.0,
            paused: false,
            paused_time: 0,
            pause_start: None,
        }
    }

    /// Create a playback session with custom speed
    pub fn with_speed(macro_data: Macro, speed: f64) -> Self {
        let mut playback = Self::new(macro_data);
        playback.speed = speed;
        playback
    }

    /// Get current time in milliseconds
    fn current_time_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Get the next event that should be executed now, if any
    pub fn next_event(&mut self) -> Option<MacroEvent> {
        if self.paused || self.current_index >= self.macro_data.events.len() {
            return None;
        }

        let current_time = Self::current_time_ms();
        let elapsed = current_time - self.start_time - self.paused_time;
        let event = &self.macro_data.events[self.current_index];
        let event_time = (event.timestamp() as f64 / self.speed) as u64;

        if elapsed >= event_time {
            let event = event.clone();
            self.current_index += 1;
            Some(event)
        } else {
            None
        }
    }

    /// Pause playback
    pub fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            self.pause_start = Some(Self::current_time_ms());
        }
    }

    /// Resume playback
    pub fn resume(&mut self) {
        if self.paused {
            if let Some(pause_start) = self.pause_start {
                let current_time = Self::current_time_ms();
                self.paused_time += current_time - pause_start;
            }
            self.paused = false;
            self.pause_start = None;
        }
    }

    /// Set playback speed
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(0.1, 10.0); // Clamp between 0.1x and 10x
    }

    /// Check if playback is finished
    pub fn is_finished(&self) -> bool {
        self.current_index >= self.macro_data.events.len()
    }

    /// Check if playback is paused
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Get current progress (current_index, total_events)
    pub fn progress(&self) -> (usize, usize) {
        (self.current_index, self.macro_data.events.len())
    }

    /// Get the macro name
    pub fn name(&self) -> &str {
        &self.macro_data.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_creation() {
        let mut macro_seq = Macro::new("Test");
        macro_seq.add_key("ctrl+c").add_delay(100).add_screenshot();

        assert_eq!(macro_seq.events.len(), 3);
        assert_eq!(macro_seq.name, "Test");
    }

    #[test]
    fn test_key_parser() {
        assert_eq!(KeyParser::parse_key("ctrl+c"), vec![3]); // Ctrl+C
        assert_eq!(KeyParser::parse_key("enter"), vec![b'\r']);
        assert_eq!(KeyParser::parse_key("tab"), vec![b'\t']);
        assert_eq!(KeyParser::parse_key("a"), vec![b'a']);
    }

    #[test]
    fn test_key_parser_single_char_modifiers() {
        // Alt on top of Ctrl ESC-prefixes the control byte, as the shared
        // encoder does for a real Ctrl+Alt keypress (ARC-093).
        assert_eq!(KeyParser::parse_key("ctrl+alt+a"), vec![0x1b, 1]);
        assert_eq!(KeyParser::parse_key("Ctrl+Z"), vec![26]);
        assert_eq!(KeyParser::parse_key("alt+x"), vec![0x1b, b'x']);
        // Ctrl on a non-alphabetic key sends the key itself
        assert_eq!(KeyParser::parse_key("ctrl+1"), vec![b'1']);
        // Multi-byte characters are passed through as the original key bytes
        assert_eq!(KeyParser::parse_key("é"), "é".as_bytes().to_vec());
        assert_eq!(KeyParser::parse_key("alt+é"), "alt+é".as_bytes().to_vec());
    }

    #[test]
    fn key_parser_named_keys_carry_modifiers() {
        // Modifiers on named keys take the xterm parameter form.
        assert_eq!(KeyParser::parse_key("alt+f4"), b"\x1b[1;3S".to_vec());
        assert_eq!(KeyParser::parse_key("ctrl+alt+del"), b"\x1b[3;7~".to_vec());
        assert_eq!(KeyParser::parse_key("shift+enter"), b"\n".to_vec());
        assert_eq!(KeyParser::parse_key("shift+tab"), b"\x1b[Z".to_vec());
        assert_eq!(KeyParser::parse_key("up"), b"\x1b[A".to_vec());
        assert_eq!(KeyParser::parse_key("f12"), b"\x1b[24~".to_vec());
        assert_eq!(KeyParser::parse_key("ctrl+space"), vec![0x00]);
        assert_eq!(KeyParser::parse_key("ls"), b"ls".to_vec());
    }

    #[test]
    fn macro_arrow_honors_application_cursor_mode() {
        let mut term = Terminal::new(80, 24);
        assert_eq!(KeyParser::encode_key("up", &term), b"\x1b[A".to_vec());
        term.process(b"\x1b[?1h");
        assert_eq!(KeyParser::encode_key("up", &term), b"\x1bOA".to_vec());
        assert_eq!(KeyParser::encode_key("home", &term), b"\x1bOH".to_vec());
        // Unknown names stay literal whatever the modes.
        assert_eq!(KeyParser::encode_key("é", &term), "é".as_bytes().to_vec());
    }

    #[test]
    fn macro_keys_follow_kitty_flags() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[>1u");
        assert_eq!(
            KeyParser::encode_key("ctrl+c", &term),
            b"\x1b[99;5u".to_vec()
        );
        assert_eq!(KeyParser::encode_key("enter", &term), b"\x1b[13u".to_vec());
        // Plain text keys stay text under kitty level 1.
        assert_eq!(KeyParser::encode_key("a", &term), b"a".to_vec());
    }

    #[cfg(feature = "macro-yaml")]
    #[test]
    fn test_yaml_serialization() {
        let mut macro_seq = Macro::new("Test Macro");
        macro_seq
            .add_key("ctrl+shift+s")
            .add_delay(100)
            .add_screenshot_labeled(Some("test.png".to_string()));

        let yaml = macro_seq.to_yaml().unwrap();
        let loaded = Macro::from_yaml(&yaml).unwrap();

        assert_eq!(loaded.name, macro_seq.name);
        assert_eq!(loaded.events.len(), macro_seq.events.len());
    }

    #[test]
    fn test_playback() {
        let mut macro_seq = Macro::new("Test");
        macro_seq.add_key("a").add_delay(100).add_key("b");

        let mut playback = MacroPlayback::new(macro_seq);
        playback.set_speed(100.0); // Very fast for testing

        // Should get first event immediately
        assert!(playback.next_event().is_some());
        assert!(!playback.is_finished());
    }

    #[test]
    fn test_pause_resume() {
        let mut macro_seq = Macro::new("Test");
        macro_seq.add_key("a");

        let mut playback = MacroPlayback::new(macro_seq);
        playback.pause();
        assert!(playback.is_paused());

        playback.resume();
        assert!(!playback.is_paused());
    }
}
