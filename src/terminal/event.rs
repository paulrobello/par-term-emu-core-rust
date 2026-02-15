//! Terminal events and notifications
//!
//! This module defines the various events that can be emitted by the terminal
//! to notify observers of state changes, user interactions, or protocol-specific actions.

use crate::terminal::file_transfer::TransferDirection;
use crate::terminal::progress::{ProgressBarAction, ProgressState};
use crate::terminal::trigger::TriggerMatch;
use crate::zone::ZoneType;

/// Bell event type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BellEvent {
    /// Standard visual bell
    VisualBell,
    /// Warning bell with volume (0-8, where 0 is off)
    WarningBell(u8),
    /// Margin bell with volume (0-8, where 0 is off)
    MarginBell(u8),
}

/// Current working directory change information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwdChange {
    /// Previous working directory
    pub old_cwd: Option<String>,
    /// New working directory
    pub new_cwd: String,
    /// Hostname associated with new working directory (if remote)
    pub hostname: Option<String>,
    /// Username associated with new working directory (if provided)
    pub username: Option<String>,
    /// Timestamp of change (unix millis)
    pub timestamp: u64,
}

/// Terminal change event
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEvent {
    /// Bell event occurred
    BellRang(BellEvent),
    /// Terminal title changed
    TitleChanged(String),
    /// Terminal was resized
    SizeChanged(usize, usize),
    /// A terminal mode changed
    ModeChanged(String, bool),
    /// Graphics added at row
    GraphicsAdded(usize),
    /// Hyperlink added with URL, position, and optional internal ID
    HyperlinkAdded {
        /// The URL of the hyperlink
        url: String,
        /// Row where hyperlink starts
        row: usize,
        /// Column where hyperlink starts
        col: usize,
        /// Internal hyperlink ID
        id: Option<u32>,
    },
    /// Dirty region (first_row, last_row)
    DirtyRegion(usize, usize),
    /// Current working directory changed (from OSC 7 or manual record)
    CwdChanged(CwdChange),
    /// A trigger pattern matched terminal output
    TriggerMatched(TriggerMatch),
    /// A user variable changed (from OSC 1337 SetUserVar)
    UserVarChanged {
        /// Variable name
        name: String,
        /// New value (base64-decoded)
        value: String,
        /// Previous value if the variable already existed
        old_value: Option<String>,
    },
    /// A named progress bar was created, updated, or removed (from OSC 934)
    ProgressBarChanged {
        /// The action that occurred
        action: ProgressBarAction,
        /// Progress bar ID
        id: String,
        /// Progress bar state (only for Set action)
        state: Option<ProgressState>,
        /// Progress percentage 0-100 (only for Set action)
        percent: Option<u8>,
        /// Optional label (only for Set action)
        label: Option<String>,
    },
    /// Badge text changed (from OSC 1337 SetBadgeFormat)
    BadgeChanged(Option<String>),
    /// Shell integration event (FinalTerm sequences)
    ShellIntegrationEvent {
        /// Event type: "prompt_start", "command_start", "command_executed", "command_finished"
        event_type: String,
        /// The command text (for command_start)
        command: Option<String>,
        /// Exit code (for command_finished)
        exit_code: Option<i32>,
        /// Timestamp (Unix epoch milliseconds)
        timestamp: Option<u64>,
        /// Absolute cursor line (scrollback_len + cursor_row) at the time the marker was emitted.
        cursor_line: Option<usize>,
    },
    /// A zone was opened (prompt, command, or output block started)
    ZoneOpened {
        /// Unique zone identifier
        zone_id: usize,
        /// Type of zone
        zone_type: ZoneType,
        /// Absolute row where zone starts
        abs_row_start: usize,
    },
    /// A zone was closed (prompt, command, or output block ended)
    ZoneClosed {
        /// Unique zone identifier
        zone_id: usize,
        /// Type of zone
        zone_type: ZoneType,
        /// Absolute row where zone starts
        abs_row_start: usize,
        /// Absolute row where zone ends
        abs_row_end: usize,
        /// Exit code (for output zones only)
        exit_code: Option<i32>,
    },
    /// A zone was evicted from scrollback
    ZoneScrolledOut {
        /// Unique zone identifier
        zone_id: usize,
        /// Type of zone that was evicted
        zone_type: ZoneType,
    },
    /// An environment variable changed (CWD, hostname, username)
    EnvironmentChanged {
        /// The key that changed ("cwd", "hostname", "username")
        key: String,
        /// The new value
        value: String,
        /// The previous value (if any)
        old_value: Option<String>,
    },
    /// Remote host transition detected (hostname changed)
    RemoteHostTransition {
        /// New hostname
        hostname: String,
        /// New username (if known)
        username: Option<String>,
        /// Previous hostname (if any)
        old_hostname: Option<String>,
        /// Previous username (if any)
        old_username: Option<String>,
    },
    /// Sub-shell detected (shell nesting depth changed)
    SubShellDetected {
        /// Current shell nesting depth
        depth: usize,
        /// Shell type if known (e.g., "bash", "zsh")
        shell_type: Option<String>,
    },
    /// A file transfer has started (download or upload)
    FileTransferStarted {
        /// Unique transfer identifier
        id: u64,
        /// Transfer direction (download or upload)
        direction: TransferDirection,
        /// Name of the file being transferred (if known)
        filename: Option<String>,
        /// Total expected size in bytes (if known)
        total_bytes: Option<usize>,
    },
    /// Progress update for an active file transfer
    FileTransferProgress {
        /// Unique transfer identifier
        id: u64,
        /// Number of bytes transferred so far
        bytes_transferred: usize,
        /// Total expected size in bytes (if known)
        total_bytes: Option<usize>,
    },
    /// A file transfer completed successfully
    FileTransferCompleted {
        /// Unique transfer identifier
        id: u64,
        /// Name of the file that was transferred (if known)
        filename: Option<String>,
        /// Total size of the transferred data in bytes
        size: usize,
    },
    /// A file transfer failed
    FileTransferFailed {
        /// Unique transfer identifier
        id: u64,
        /// Reason for the failure
        reason: String,
    },
    /// An upload was requested by the remote application
    UploadRequested {
        /// Upload format (e.g., "base64")
        format: String,
    },
}

impl TerminalEvent {
    /// Get the kind of this event
    pub fn kind(&self) -> TerminalEventKind {
        match self {
            TerminalEvent::BellRang(_) => TerminalEventKind::BellRang,
            TerminalEvent::TitleChanged(_) => TerminalEventKind::TitleChanged,
            TerminalEvent::SizeChanged(_, _) => TerminalEventKind::SizeChanged,
            TerminalEvent::ModeChanged(_, _) => TerminalEventKind::ModeChanged,
            TerminalEvent::GraphicsAdded(_) => TerminalEventKind::GraphicsAdded,
            TerminalEvent::HyperlinkAdded { .. } => TerminalEventKind::HyperlinkAdded,
            TerminalEvent::DirtyRegion(_, _) => TerminalEventKind::DirtyRegion,
            TerminalEvent::CwdChanged(_) => TerminalEventKind::CwdChanged,
            TerminalEvent::TriggerMatched(_) => TerminalEventKind::TriggerMatched,
            TerminalEvent::UserVarChanged { .. } => TerminalEventKind::UserVarChanged,
            TerminalEvent::ProgressBarChanged { .. } => TerminalEventKind::ProgressBarChanged,
            TerminalEvent::BadgeChanged(_) => TerminalEventKind::BadgeChanged,
            TerminalEvent::ShellIntegrationEvent { .. } => TerminalEventKind::ShellIntegrationEvent,
            TerminalEvent::ZoneOpened { .. } => TerminalEventKind::ZoneOpened,
            TerminalEvent::ZoneClosed { .. } => TerminalEventKind::ZoneClosed,
            TerminalEvent::ZoneScrolledOut { .. } => TerminalEventKind::ZoneScrolledOut,
            TerminalEvent::EnvironmentChanged { .. } => TerminalEventKind::EnvironmentChanged,
            TerminalEvent::RemoteHostTransition { .. } => TerminalEventKind::RemoteHostTransition,
            TerminalEvent::SubShellDetected { .. } => TerminalEventKind::SubShellDetected,
            TerminalEvent::FileTransferStarted { .. } => TerminalEventKind::FileTransferStarted,
            TerminalEvent::FileTransferProgress { .. } => TerminalEventKind::FileTransferProgress,
            TerminalEvent::FileTransferCompleted { .. } => TerminalEventKind::FileTransferCompleted,
            TerminalEvent::FileTransferFailed { .. } => TerminalEventKind::FileTransferFailed,
            TerminalEvent::UploadRequested { .. } => TerminalEventKind::UploadRequested,
        }
    }
}

/// Kind of terminal event for subscription filters
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TerminalEventKind {
    BellRang,
    TitleChanged,
    SizeChanged,
    ModeChanged,
    GraphicsAdded,
    HyperlinkAdded,
    DirtyRegion,
    CwdChanged,
    TriggerMatched,
    UserVarChanged,
    ProgressBarChanged,
    BadgeChanged,
    ShellIntegrationEvent,
    ZoneOpened,
    ZoneClosed,
    ZoneScrolledOut,
    EnvironmentChanged,
    RemoteHostTransition,
    SubShellDetected,
    FileTransferStarted,
    FileTransferProgress,
    FileTransferCompleted,
    FileTransferFailed,
    UploadRequested,
}

/// A drained shell integration event: (event_type, command, exit_code, timestamp, cursor_line).
pub type ShellEvent = (
    String,
    Option<String>,
    Option<i32>,
    Option<u64>,
    Option<usize>,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    #[test]
    fn test_event_kind_bell_rang() {
        let event = TerminalEvent::BellRang(BellEvent::VisualBell);
        assert_eq!(event.kind(), TerminalEventKind::BellRang);

        let event2 = TerminalEvent::BellRang(BellEvent::WarningBell(5));
        assert_eq!(event2.kind(), TerminalEventKind::BellRang);

        let event3 = TerminalEvent::BellRang(BellEvent::MarginBell(3));
        assert_eq!(event3.kind(), TerminalEventKind::BellRang);
    }

    #[test]
    fn test_event_kind_title_changed() {
        let event = TerminalEvent::TitleChanged("New Title".to_string());
        assert_eq!(event.kind(), TerminalEventKind::TitleChanged);
    }

    #[test]
    fn test_event_kind_size_changed() {
        let event = TerminalEvent::SizeChanged(100, 50);
        assert_eq!(event.kind(), TerminalEventKind::SizeChanged);
    }

    #[test]
    fn test_event_kind_mode_changed() {
        let event = TerminalEvent::ModeChanged("DECCKM".to_string(), true);
        assert_eq!(event.kind(), TerminalEventKind::ModeChanged);
    }

    #[test]
    fn test_event_kind_graphics_added() {
        let event = TerminalEvent::GraphicsAdded(10);
        assert_eq!(event.kind(), TerminalEventKind::GraphicsAdded);
    }

    #[test]
    fn test_event_kind_hyperlink_added() {
        let event = TerminalEvent::HyperlinkAdded {
            url: "https://example.com".to_string(),
            row: 5,
            col: 10,
            id: Some(42),
        };
        assert_eq!(event.kind(), TerminalEventKind::HyperlinkAdded);
    }

    #[test]
    fn test_event_kind_dirty_region() {
        let event = TerminalEvent::DirtyRegion(0, 23);
        assert_eq!(event.kind(), TerminalEventKind::DirtyRegion);
    }

    #[test]
    fn test_event_kind_cwd_changed() {
        let event = TerminalEvent::CwdChanged(CwdChange {
            old_cwd: Some("/old/path".to_string()),
            new_cwd: "/new/path".to_string(),
            hostname: None,
            username: None,
            timestamp: 1234567890,
        });
        assert_eq!(event.kind(), TerminalEventKind::CwdChanged);
    }

    #[test]
    fn test_event_kind_trigger_matched() {
        let event = TerminalEvent::TriggerMatched(TriggerMatch {
            trigger_id: 1,
            row: 10,
            col: 0,
            end_col: 15,
            text: "error occurred".to_string(),
            captures: vec!["error occurred".to_string()],
            timestamp: 1234567890,
        });
        assert_eq!(event.kind(), TerminalEventKind::TriggerMatched);
    }

    #[test]
    fn test_event_kind_user_var_changed() {
        let event = TerminalEvent::UserVarChanged {
            name: "MY_VAR".to_string(),
            value: "new_value".to_string(),
            old_value: Some("old_value".to_string()),
        };
        assert_eq!(event.kind(), TerminalEventKind::UserVarChanged);
    }

    #[test]
    fn test_event_kind_progress_bar_changed() {
        let event = TerminalEvent::ProgressBarChanged {
            action: ProgressBarAction::Set,
            id: "download".to_string(),
            state: Some(ProgressState::Normal),
            percent: Some(50),
            label: Some("Downloading".to_string()),
        };
        assert_eq!(event.kind(), TerminalEventKind::ProgressBarChanged);
    }

    #[test]
    fn test_event_kind_badge_changed() {
        let event = TerminalEvent::BadgeChanged(Some("Important".to_string()));
        assert_eq!(event.kind(), TerminalEventKind::BadgeChanged);
    }

    #[test]
    fn test_event_kind_shell_integration_event() {
        let event = TerminalEvent::ShellIntegrationEvent {
            event_type: "prompt_start".to_string(),
            command: None,
            exit_code: None,
            timestamp: Some(1234567890),
            cursor_line: Some(5),
        };
        assert_eq!(event.kind(), TerminalEventKind::ShellIntegrationEvent);
    }

    #[test]
    fn test_event_kind_zone_opened() {
        let event = TerminalEvent::ZoneOpened {
            zone_id: 1,
            zone_type: ZoneType::Prompt,
            abs_row_start: 10,
        };
        assert_eq!(event.kind(), TerminalEventKind::ZoneOpened);
    }

    #[test]
    fn test_event_kind_zone_closed() {
        let event = TerminalEvent::ZoneClosed {
            zone_id: 1,
            zone_type: ZoneType::Output,
            abs_row_start: 10,
            abs_row_end: 15,
            exit_code: Some(0),
        };
        assert_eq!(event.kind(), TerminalEventKind::ZoneClosed);
    }

    #[test]
    fn test_event_kind_zone_scrolled_out() {
        let event = TerminalEvent::ZoneScrolledOut {
            zone_id: 1,
            zone_type: ZoneType::Command,
        };
        assert_eq!(event.kind(), TerminalEventKind::ZoneScrolledOut);
    }

    #[test]
    fn test_event_kind_environment_changed() {
        let event = TerminalEvent::EnvironmentChanged {
            key: "cwd".to_string(),
            value: "/home/user".to_string(),
            old_value: Some("/home".to_string()),
        };
        assert_eq!(event.kind(), TerminalEventKind::EnvironmentChanged);
    }

    #[test]
    fn test_event_kind_remote_host_transition() {
        let event = TerminalEvent::RemoteHostTransition {
            hostname: "server.example.com".to_string(),
            username: Some("user".to_string()),
            old_hostname: Some("localhost".to_string()),
            old_username: Some("localuser".to_string()),
        };
        assert_eq!(event.kind(), TerminalEventKind::RemoteHostTransition);
    }

    #[test]
    fn test_event_kind_sub_shell_detected() {
        let event = TerminalEvent::SubShellDetected {
            depth: 2,
            shell_type: Some("bash".to_string()),
        };
        assert_eq!(event.kind(), TerminalEventKind::SubShellDetected);
    }

    #[test]
    fn test_event_kind_file_transfer_started() {
        let event = TerminalEvent::FileTransferStarted {
            id: 123,
            direction: TransferDirection::Download,
            filename: Some("file.txt".to_string()),
            total_bytes: Some(1024),
        };
        assert_eq!(event.kind(), TerminalEventKind::FileTransferStarted);
    }

    #[test]
    fn test_event_kind_file_transfer_progress() {
        let event = TerminalEvent::FileTransferProgress {
            id: 123,
            bytes_transferred: 512,
            total_bytes: Some(1024),
        };
        assert_eq!(event.kind(), TerminalEventKind::FileTransferProgress);
    }

    #[test]
    fn test_event_kind_file_transfer_completed() {
        let event = TerminalEvent::FileTransferCompleted {
            id: 123,
            filename: Some("file.txt".to_string()),
            size: 1024,
        };
        assert_eq!(event.kind(), TerminalEventKind::FileTransferCompleted);
    }

    #[test]
    fn test_event_kind_file_transfer_failed() {
        let event = TerminalEvent::FileTransferFailed {
            id: 123,
            reason: "Network error".to_string(),
        };
        assert_eq!(event.kind(), TerminalEventKind::FileTransferFailed);
    }

    #[test]
    fn test_event_kind_upload_requested() {
        let event = TerminalEvent::UploadRequested {
            format: "base64".to_string(),
        };
        assert_eq!(event.kind(), TerminalEventKind::UploadRequested);
    }

    #[test]
    fn test_event_queuing_through_process() {
        let mut term = Terminal::new(80, 24);

        // Process a bell sequence
        term.process(b"\x07");

        // Poll events
        let events = term.poll_events();

        // Should have a bell event
        let bell_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.kind(), TerminalEventKind::BellRang))
            .collect();

        assert!(!bell_events.is_empty(), "Should have received a bell event");

        // Polling again should return empty (events are consumed)
        let events2 = term.poll_events();
        assert!(events2.is_empty(), "Events should be cleared after polling");
    }

    #[test]
    fn test_event_queuing_title_change() {
        let mut term = Terminal::new(80, 24);

        // Process OSC 0 sequence to change title
        term.process(b"\x1b]0;New Title\x07");

        let events = term.poll_events();

        // Should have a title changed event
        let title_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.kind(), TerminalEventKind::TitleChanged))
            .collect();

        assert_eq!(title_events.len(), 1);

        if let TerminalEvent::TitleChanged(title) = &title_events[0] {
            assert_eq!(title, "New Title");
        } else {
            panic!("Expected TitleChanged event");
        }
    }

    #[test]
    fn test_event_queuing_multiple_events() {
        let mut term = Terminal::new(80, 24);

        // Process multiple sequences that generate events
        term.process(b"\x07"); // Bell
        term.process(b"\x1b]0;Title1\x07"); // Title change
        term.process(b"\x07"); // Another bell

        let events = term.poll_events();

        // Should have multiple events
        assert!(events.len() >= 3, "Should have at least 3 events");

        let bell_count = events
            .iter()
            .filter(|e| matches!(e.kind(), TerminalEventKind::BellRang))
            .count();
        let title_count = events
            .iter()
            .filter(|e| matches!(e.kind(), TerminalEventKind::TitleChanged))
            .count();

        assert_eq!(bell_count, 2, "Should have 2 bell events");
        assert_eq!(title_count, 1, "Should have 1 title change event");
    }

    #[test]
    fn test_bell_event_variants() {
        let visual = BellEvent::VisualBell;
        let warning = BellEvent::WarningBell(5);
        let margin = BellEvent::MarginBell(3);

        assert_eq!(visual, BellEvent::VisualBell);
        assert_eq!(warning, BellEvent::WarningBell(5));
        assert_eq!(margin, BellEvent::MarginBell(3));

        // Test inequality
        assert_ne!(visual, warning);
        assert_ne!(visual, margin);
        assert_ne!(warning, margin);
    }

    #[test]
    fn test_cwd_change_struct() {
        let cwd_change = CwdChange {
            old_cwd: Some("/home/user".to_string()),
            new_cwd: "/home/user/projects".to_string(),
            hostname: Some("server".to_string()),
            username: Some("user".to_string()),
            timestamp: 1234567890,
        };

        assert_eq!(cwd_change.old_cwd, Some("/home/user".to_string()));
        assert_eq!(cwd_change.new_cwd, "/home/user/projects");
        assert_eq!(cwd_change.hostname, Some("server".to_string()));
        assert_eq!(cwd_change.username, Some("user".to_string()));
        assert_eq!(cwd_change.timestamp, 1234567890);
    }
}
