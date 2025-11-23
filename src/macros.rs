//! Macro recording and playback functionality
//!
//! This module provides keyboard macro recording and playback with YAML serialization.
//! Macros can contain keyboard events, delays, and screenshot triggers.
//!
//! ## Example
//!
//! ```rust
//! use par_term_emu_core_rust::macros::{Macro, MacroEvent};
//!
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
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
                .unwrap()
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

    /// Save the macro to a YAML file
    pub fn save_yaml<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let yaml = serde_yaml::to_string(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, yaml)
    }

    /// Load a macro from a YAML file
    pub fn load_yaml<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        serde_yaml::from_str(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Convert to YAML string
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Parse from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
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

/// Key name parser and converter
pub struct KeyParser;

impl KeyParser {
    /// Parse a friendly key name into terminal escape sequences
    ///
    /// Returns the bytes to send to the terminal
    pub fn parse_key(key: &str) -> Vec<u8> {
        let key_lower = key.to_lowercase();
        let parts: Vec<&str> = key_lower.split('+').collect();

        // Check for modifiers
        let has_ctrl = parts.contains(&"ctrl");
        let has_alt = parts.contains(&"alt");
        let has_shift = parts.contains(&"shift");

        // Get the main key (last part)
        let main_key = parts.last().copied().unwrap_or("");

        // Handle special keys
        match main_key {
            // Control characters (Ctrl+key)
            k if has_ctrl && k.len() == 1 && k.chars().next().unwrap().is_ascii_alphabetic() => {
                let ch = k.chars().next().unwrap();
                let ctrl_code = ch as u8 - b'a' + 1;
                vec![ctrl_code]
            }

            // Function keys
            "f1" => vec![0x1b, b'O', b'P'],
            "f2" => vec![0x1b, b'O', b'Q'],
            "f3" => vec![0x1b, b'O', b'R'],
            "f4" => vec![0x1b, b'O', b'S'],
            "f5" => vec![0x1b, b'[', b'1', b'5', b'~'],
            "f6" => vec![0x1b, b'[', b'1', b'7', b'~'],
            "f7" => vec![0x1b, b'[', b'1', b'8', b'~'],
            "f8" => vec![0x1b, b'[', b'1', b'9', b'~'],
            "f9" => vec![0x1b, b'[', b'2', b'0', b'~'],
            "f10" => vec![0x1b, b'[', b'2', b'1', b'~'],
            "f11" => vec![0x1b, b'[', b'2', b'3', b'~'],
            "f12" => vec![0x1b, b'[', b'2', b'4', b'~'],

            // Arrow keys
            "up" => vec![0x1b, b'[', b'A'],
            "down" => vec![0x1b, b'[', b'B'],
            "right" => vec![0x1b, b'[', b'C'],
            "left" => vec![0x1b, b'[', b'D'],

            // Special keys
            "enter" | "return" => vec![b'\r'],
            "tab" => {
                if has_shift {
                    vec![0x1b, b'[', b'Z'] // Shift+Tab
                } else {
                    vec![b'\t']
                }
            }
            "backspace" => vec![0x7f],
            "delete" | "del" => vec![0x1b, b'[', b'3', b'~'],
            "escape" | "esc" => vec![0x1b],
            "space" => vec![b' '],
            "home" => vec![0x1b, b'[', b'H'],
            "end" => vec![0x1b, b'[', b'F'],
            "pageup" | "pgup" => vec![0x1b, b'[', b'5', b'~'],
            "pagedown" | "pgdn" => vec![0x1b, b'[', b'6', b'~'],
            "insert" | "ins" => vec![0x1b, b'[', b'2', b'~'],

            // Regular character
            k if k.len() == 1 => {
                let ch = k.chars().next().unwrap();
                if has_alt {
                    vec![0x1b, ch as u8]
                } else if has_ctrl && ch.is_ascii_alphabetic() {
                    let ctrl_code = ch as u8 - b'a' + 1;
                    vec![ctrl_code]
                } else {
                    vec![ch as u8]
                }
            }

            // Unknown key - return it as-is
            _ => key.as_bytes().to_vec(),
        }
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
            .unwrap()
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

    /// Reset playback to the beginning
    pub fn reset(&mut self) {
        self.current_index = 0;
        self.start_time = Self::current_time_ms();
        self.paused_time = 0;
        self.pause_start = None;
        self.paused = false;
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
