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
