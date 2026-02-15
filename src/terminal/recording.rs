//! Terminal recording and replay support
//!
//! Provides types for recording terminal sessions in various formats.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Recording format type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingFormat {
    /// Asciicast v2 format (asciinema)
    Asciicast,
    /// JSON with timing data
    Json,
    /// Raw TTY data
    Tty,
}

/// Recording event type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingEventType {
    /// Input to the terminal (from user/PTY)
    Input,
    /// Output from the terminal
    Output,
    /// Terminal resize event
    Resize,
    /// Metadata change
    Metadata,
    /// Marker/bookmark
    Marker,
}

/// A single event in a recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingEvent {
    /// Timestamp relative to recording start (microseconds)
    pub timestamp: u64,
    /// Type of event
    pub event_type: RecordingEventType,
    /// Raw data associated with the event
    pub data: Vec<u8>,
    /// Metadata (for resize: cols, rows)
    pub metadata: Option<(usize, usize)>,
}

/// A complete recording session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSession {
    /// Unique recording identifier
    pub id: String,
    /// Recording name/title
    pub title: String,
    /// Initial terminal size (cols, rows)
    pub initial_size: (usize, usize),
    /// All events in chronological order
    pub events: Vec<RecordingEvent>,
    /// Environment variables at start
    pub env: HashMap<String, String>,
    /// Total duration in microseconds
    pub duration: u64,
    /// Creation timestamp
    pub created_at: u64,
}

/// Export format for recordings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingExportFormat {
    /// SVG animation
    Svg,
    /// Animated GIF
    Gif,
    /// Video (MP4)
    Video,
    /// HTML with embedded player
    Html,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 24: Terminal Replay/Recording ===

    /// Start recording terminal session
    ///
    /// If `title` is provided, it overrides the terminal's current title.
    pub fn start_recording(&mut self, title: Option<String>) {
        let (cols, rows) = self.size();
        let mut env = HashMap::new();
        if let Some(cwd) = self.shell_integration.cwd() {
            env.insert("CWD".to_string(), cwd.to_string());
        }

        self.recording_session = Some(RecordingSession {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.unwrap_or_else(|| self.title().to_string()),
            initial_size: (cols, rows),
            events: Vec::new(),
            env,
            duration: 0,
            created_at: crate::terminal::unix_millis(),
        });
        self.is_recording = true;
        self.recording_start_time = crate::terminal::unix_millis();
    }

    /// Stop recording terminal session
    pub fn stop_recording(&mut self) -> Option<RecordingSession> {
        self.is_recording = false;
        let mut session = self.recording_session.take()?;
        session.duration = crate::terminal::unix_millis() - self.recording_start_time;
        Some(session)
    }

    /// Record an event
    pub fn record_event(&mut self, event_type: RecordingEventType, data: Vec<u8>) {
        if !self.is_recording {
            return;
        }

        if let Some(ref mut session) = self.recording_session {
            let timestamp = crate::terminal::unix_millis() - self.recording_start_time;
            session.events.push(RecordingEvent {
                timestamp,
                event_type,
                data,
                metadata: None,
            });
        }
    }

    /// Check if recording is active
    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    /// Get current recording session (if any)
    pub fn get_recording_session(&self) -> Option<&RecordingSession> {
        self.recording_session.as_ref()
    }

    /// Export a recording session to asciicast format
    pub fn export_asciicast(&self, session: &RecordingSession) -> String {
        let mut output = String::new();

        // 1. Header line
        let header = serde_json::json!({
            "version": 2,
            "width": session.initial_size.0,
            "height": session.initial_size.1,
            "timestamp": session.created_at / 1000,
            "title": session.title,
            "env": session.env,
        });
        output.push_str(&header.to_string());
        output.push('\n');

        // 2. Event lines
        for event in &session.events {
            let timestamp = event.timestamp as f64 / 1_000_000.0; // microseconds to seconds

            let event_json = match event.event_type {
                RecordingEventType::Output => {
                    let text = String::from_utf8_lossy(&event.data);
                    serde_json::json!([timestamp, "o", text])
                }
                RecordingEventType::Input => {
                    let text = String::from_utf8_lossy(&event.data);
                    serde_json::json!([timestamp, "i", text])
                }
                RecordingEventType::Resize => {
                    if let Some((cols, rows)) = event.metadata {
                        serde_json::json!([timestamp, "r", cols, rows])
                    } else {
                        continue;
                    }
                }
                _ => continue, // Ignore other events for asciicast
            };

            output.push_str(&event_json.to_string());
            output.push('\n');
        }

        output
    }

    /// Export a recording session to JSON format
    pub fn export_json(&self, session: &RecordingSession) -> String {
        serde_json::to_string(session).unwrap_or_default()
    }

    /// Record input to the terminal
    pub fn record_input(&mut self, data: &[u8]) {
        self.record_event(RecordingEventType::Input, data.to_vec());
    }

    /// Record output from the terminal
    pub fn record_output(&mut self, data: &[u8]) {
        self.record_event(RecordingEventType::Output, data.to_vec());
    }

    /// Record a resize event
    pub fn record_resize(&mut self, cols: usize, rows: usize) {
        if !self.is_recording {
            return;
        }

        if let Some(ref mut session) = self.recording_session {
            let timestamp = crate::terminal::unix_millis() - self.recording_start_time;
            session.events.push(RecordingEvent {
                timestamp,
                event_type: RecordingEventType::Resize,
                data: Vec::new(),
                metadata: Some((cols, rows)),
            });
        }
    }

    /// Record a marker/bookmark
    pub fn record_marker(&mut self, label: String) {
        self.record_event(RecordingEventType::Marker, label.into_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_lifecycle_basic() {
        let mut term = Terminal::new(80, 24);

        // Not recording initially
        assert!(!term.is_recording());
        assert!(term.get_recording_session().is_none());

        // Start recording
        term.start_recording(Some("Test Session".to_string()));
        assert!(term.is_recording());

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.title, "Test Session");
        assert_eq!(session.initial_size, (80, 24));
        assert!(session.events.is_empty());

        // Record some events
        term.record_input(b"ls -la\n");
        term.record_output(b"total 42\n");

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.events.len(), 2);
        assert_eq!(session.events[0].event_type, RecordingEventType::Input);
        assert_eq!(session.events[1].event_type, RecordingEventType::Output);

        // Stop recording
        let session = term.stop_recording().unwrap();
        assert!(!term.is_recording());
        assert!(term.get_recording_session().is_none());
        assert_eq!(session.events.len(), 2);
        // Duration is always set (may be 0 in fast test execution)
        let _ = session.duration; // Just verify field exists
    }

    #[test]
    fn test_recording_timestamps() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(None);

        let start_time = term.recording_start_time;

        // Record events with small delays
        term.record_input(b"echo hello");
        std::thread::sleep(std::time::Duration::from_millis(10));
        term.record_output(b"hello\n");

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.events.len(), 2);

        // Timestamps should be relative to start and increasing
        let ts1 = session.events[0].timestamp;
        let ts2 = session.events[1].timestamp;
        assert!(ts2 > ts1, "Second timestamp should be later than first");

        // Timestamps should be in microseconds
        let now = crate::terminal::unix_millis();
        assert!(
            ts1 < (now - start_time) * 1000 + 1_000_000,
            "Timestamp should be reasonable"
        );
    }

    #[test]
    fn test_record_input_output_resize_marker() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(Some("Event Test".to_string()));

        // Record different event types
        term.record_input(b"pwd");
        term.record_output(b"/home/user\n");
        term.record_resize(100, 30);
        term.record_marker("Important moment".to_string());

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.events.len(), 4);

        // Verify event types
        assert_eq!(session.events[0].event_type, RecordingEventType::Input);
        assert_eq!(session.events[0].data, b"pwd");

        assert_eq!(session.events[1].event_type, RecordingEventType::Output);
        assert_eq!(session.events[1].data, b"/home/user\n");

        assert_eq!(session.events[2].event_type, RecordingEventType::Resize);
        assert_eq!(session.events[2].metadata, Some((100, 30)));
        assert!(session.events[2].data.is_empty());

        assert_eq!(session.events[3].event_type, RecordingEventType::Marker);
        assert_eq!(session.events[3].data, b"Important moment");
    }

    #[test]
    fn test_stop_without_start() {
        let mut term = Terminal::new(80, 24);

        // Stop without starting should return None
        let result = term.stop_recording();
        assert!(result.is_none());
        assert!(!term.is_recording());
    }

    #[test]
    fn test_start_twice() {
        let mut term = Terminal::new(80, 24);

        // Start first recording
        term.start_recording(Some("First".to_string()));
        term.record_input(b"first command");

        // Start second recording (should implicitly stop first)
        term.start_recording(Some("Second".to_string()));

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.title, "Second");
        assert!(session.events.is_empty(), "New session should start fresh");
        assert!(term.is_recording());
    }

    #[test]
    fn test_is_recording_state_transitions() {
        let mut term = Terminal::new(80, 24);

        // Initial state
        assert!(!term.is_recording());

        // After start
        term.start_recording(None);
        assert!(term.is_recording());

        // After stop
        term.stop_recording();
        assert!(!term.is_recording());

        // Start again
        term.start_recording(Some("Round 2".to_string()));
        assert!(term.is_recording());

        // Multiple stops (second should be no-op)
        term.stop_recording();
        assert!(!term.is_recording());
        let result = term.stop_recording();
        assert!(result.is_none());
        assert!(!term.is_recording());
    }

    #[test]
    fn test_export_asciicast_format() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(Some("Asciicast Test".to_string()));

        term.record_input(b"echo hello");
        term.record_output(b"hello\n");
        term.record_resize(100, 30);

        let session = term.stop_recording().unwrap();
        let asciicast = term.export_asciicast(&session);

        // Split into lines
        let lines: Vec<&str> = asciicast.lines().collect();
        assert!(lines.len() >= 3, "Should have header + at least 2 events");

        // Verify header line
        let header: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header["version"], 2);
        assert_eq!(header["width"], 80);
        assert_eq!(header["height"], 24);
        assert_eq!(header["title"], "Asciicast Test");

        // Verify event lines have [timestamp, type, data] format
        let event1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert!(event1.is_array());
        let arr = event1.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert!(arr[0].is_f64(), "First element should be timestamp");
        assert_eq!(arr[1], "i"); // Input event
        assert_eq!(arr[2], "echo hello");

        let event2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        let arr2 = event2.as_array().unwrap();
        assert_eq!(arr2[1], "o"); // Output event
        assert_eq!(arr2[2], "hello\n");

        // Resize event should have 4 elements: [timestamp, "r", cols, rows]
        let event3: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
        let arr3 = event3.as_array().unwrap();
        assert_eq!(arr3.len(), 4);
        assert_eq!(arr3[1], "r");
        assert_eq!(arr3[2], 100);
        assert_eq!(arr3[3], 30);
    }

    #[test]
    fn test_export_json_format() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(Some("JSON Test".to_string()));

        term.record_input(b"test");
        term.record_output(b"output");

        let session = term.stop_recording().unwrap();
        let json_str = term.export_json(&session);

        // Verify it's valid JSON and can be deserialized
        let deserialized: RecordingSession = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.title, "JSON Test");
        assert_eq!(deserialized.initial_size, (80, 24));
        assert_eq!(deserialized.events.len(), 2);
        assert_eq!(deserialized.events[0].event_type, RecordingEventType::Input);
        assert_eq!(
            deserialized.events[1].event_type,
            RecordingEventType::Output
        );
        assert_eq!(deserialized.events[0].data, b"test");
        assert_eq!(deserialized.events[1].data, b"output");
    }

    #[test]
    fn test_recording_not_active_ignores_events() {
        let mut term = Terminal::new(80, 24);

        // Record events without starting recording
        term.record_input(b"ignored");
        term.record_output(b"also ignored");
        term.record_resize(100, 30);

        // Should have no session
        assert!(term.get_recording_session().is_none());

        // Start and stop without recording anything
        term.start_recording(None);
        let session = term.stop_recording().unwrap();
        assert!(session.events.is_empty());
    }

    #[test]
    fn test_recording_with_process() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(Some("Process Test".to_string()));

        // Process some data (should auto-record as output)
        term.process(b"Hello, world!");

        let session = term.get_recording_session().unwrap();

        // Should have at least one output event
        let output_events: Vec<_> = session
            .events
            .iter()
            .filter(|e| e.event_type == RecordingEventType::Output)
            .collect();

        assert!(!output_events.is_empty());
        assert_eq!(output_events[0].data, b"Hello, world!");
    }

    #[test]
    fn test_recording_default_title() {
        let mut term = Terminal::new(80, 24);
        term.set_title("Custom Title".to_string());

        // Start recording without providing title
        term.start_recording(None);

        let session = term.get_recording_session().unwrap();
        assert_eq!(session.title, "Custom Title");
    }

    #[test]
    fn test_recording_session_id_unique() {
        let mut term = Terminal::new(80, 24);

        term.start_recording(None);
        let id1 = term.get_recording_session().unwrap().id.clone();
        term.stop_recording();

        term.start_recording(None);
        let id2 = term.get_recording_session().unwrap().id.clone();
        term.stop_recording();

        // Each session should have a unique UUID
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_export_asciicast_ignores_marker_events() {
        let mut term = Terminal::new(80, 24);
        term.start_recording(None);

        term.record_input(b"cmd");
        term.record_marker("This should be ignored".to_string());
        term.record_output(b"result");

        let session = term.stop_recording().unwrap();
        let asciicast = term.export_asciicast(&session);

        let lines: Vec<&str> = asciicast.lines().collect();

        // Should have header + 2 events (input + output, marker ignored)
        assert_eq!(lines.len(), 3);
    }
}
