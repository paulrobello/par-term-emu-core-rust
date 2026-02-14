//! Protocol definitions for terminal streaming
//!
//! This module defines the message formats used for WebSocket-based
//! terminal streaming between the server and web clients.

use serde::{Deserialize, Serialize};

/// Theme information for terminal color scheme
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    /// Theme name (e.g., "iterm2-dark", "monokai")
    pub name: String,
    /// Background color (RGB)
    pub background: (u8, u8, u8),
    /// Foreground color (RGB)
    pub foreground: (u8, u8, u8),
    /// Normal ANSI colors 0-7 (RGB)
    pub normal: [(u8, u8, u8); 8],
    /// Bright ANSI colors 8-15 (RGB)
    pub bright: [(u8, u8, u8); 8],
}

/// CPU statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuStats {
    pub overall_usage_percent: f64,
    pub physical_core_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub per_core_usage_percent: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_mhz: Option<u64>,
}

/// Memory statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

/// Individual disk statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskStats {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub kind: String,
    pub file_system: String,
    pub is_removable: bool,
}

/// Network interface statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceStats {
    pub name: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub total_received_bytes: u64,
    pub total_transmitted_bytes: u64,
    pub packets_received: u64,
    pub packets_transmitted: u64,
    pub errors_received: u64,
    pub errors_transmitted: u64,
}

/// System load averages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadAverage {
    pub one_minute: f64,
    pub five_minutes: f64,
    pub fifteen_minutes: f64,
}

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    /// Terminal output data (raw ANSI escape sequences)
    Output {
        /// Raw terminal output data including ANSI sequences
        data: String,
        /// Optional timestamp (Unix epoch in milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Terminal size changed
    Resize {
        /// Number of columns
        cols: u16,
        /// Number of rows
        rows: u16,
    },

    /// Terminal title changed
    Title {
        /// New terminal title
        title: String,
    },

    /// Connection established successfully
    Connected {
        /// Current terminal width in columns
        cols: u16,
        /// Current terminal height in rows
        rows: u16,
        /// Optional initial screen content
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_screen: Option<String>,
        /// Session ID for this connection
        session_id: String,
        /// Optional theme information
        #[serde(skip_serializing_if = "Option::is_none")]
        theme: Option<ThemeInfo>,
        /// Current badge text (from OSC 1337)
        #[serde(skip_serializing_if = "Option::is_none")]
        badge: Option<String>,
        /// Faint text alpha for SGR 2 dim text (0.0-1.0)
        #[serde(skip_serializing_if = "Option::is_none")]
        faint_text_alpha: Option<f32>,
        /// Current working directory
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// modifyOtherKeys mode (0=disabled, 1=special keys, 2=all keys)
        #[serde(skip_serializing_if = "Option::is_none")]
        modify_other_keys: Option<u32>,
        /// Unique client identifier for this connection
        #[serde(skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        /// Whether this connection is read-only
        #[serde(skip_serializing_if = "Option::is_none")]
        readonly: Option<bool>,
    },

    /// Screen refresh response (full screen content)
    Refresh {
        /// Current terminal width in columns
        cols: u16,
        /// Current terminal height in rows
        rows: u16,
        /// Full screen content with ANSI styling
        screen_content: String,
    },

    /// Cursor position changed (optional optimization)
    #[serde(rename = "cursor")]
    CursorPosition {
        /// Column position (0-indexed)
        col: u16,
        /// Row position (0-indexed)
        row: u16,
        /// Whether cursor is visible
        visible: bool,
    },

    /// Bell event occurred
    Bell,

    /// Current working directory changed (OSC 7)
    CwdChanged {
        /// Previous working directory
        #[serde(skip_serializing_if = "Option::is_none")]
        old_cwd: Option<String>,
        /// New working directory
        new_cwd: String,
        /// Hostname (if remote)
        #[serde(skip_serializing_if = "Option::is_none")]
        hostname: Option<String>,
        /// Username (if provided)
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// Timestamp of change (Unix epoch milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Trigger pattern matched terminal output
    TriggerMatched {
        /// ID of the trigger that matched
        trigger_id: u64,
        /// Row where the match occurred
        row: u16,
        /// Column where match starts
        col: u16,
        /// Column where match ends (exclusive)
        end_col: u16,
        /// Matched text
        text: String,
        /// Capture groups
        captures: Vec<String>,
        /// Timestamp when match occurred (Unix epoch milliseconds)
        timestamp: u64,
    },

    /// Trigger action result: display notification
    ActionNotify {
        /// ID of the trigger that produced this action
        trigger_id: u64,
        /// Notification title
        title: String,
        /// Notification message
        message: String,
    },

    /// Trigger action result: mark/bookmark a line
    ActionMarkLine {
        /// ID of the trigger that produced this action
        trigger_id: u64,
        /// Row to mark
        row: u16,
        /// Optional label for the mark
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Optional RGB color for the mark
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<(u8, u8, u8)>,
    },

    /// Error occurred
    Error {
        /// Error message
        message: String,
        /// Optional error code
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },

    /// Server is shutting down
    Shutdown {
        /// Reason for shutdown
        reason: String,
    },

    /// Keepalive pong response
    Pong,

    /// Terminal mode changed (cursor visibility, mouse tracking, etc.)
    ModeChanged {
        /// Mode name (e.g., "cursor_visible", "mouse_tracking", "bracketed_paste")
        mode: String,
        /// Whether the mode is enabled
        enabled: bool,
    },

    /// Graphics/image added to terminal (Sixel, iTerm2, Kitty)
    GraphicsAdded {
        /// Row where graphics were added
        row: u16,
        /// Graphics format ("sixel", "iterm2", "kitty")
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },

    /// Hyperlink added (OSC 8)
    HyperlinkAdded {
        /// The URL of the hyperlink
        url: String,
        /// Row where the hyperlink appears
        row: u16,
        /// Column where hyperlink starts
        col: u16,
        /// Optional hyperlink ID from OSC 8
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },

    /// User variable changed (OSC 1337 SetUserVar)
    #[serde(rename = "user_var_changed")]
    UserVarChanged {
        /// Variable name
        name: String,
        /// New value (base64-decoded)
        value: String,
        /// Previous value if the variable already existed
        #[serde(skip_serializing_if = "Option::is_none")]
        old_value: Option<String>,
    },

    /// Named progress bar changed (OSC 934)
    #[serde(rename = "progress_bar_changed")]
    ProgressBarChanged {
        /// Action: "set", "remove", or "remove_all"
        action: String,
        /// Progress bar identifier
        id: String,
        /// State name (only for "set"): normal, indeterminate, warning, error
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        /// Progress percentage 0-100 (only for "set")
        #[serde(skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
        /// Descriptive label (only for "set")
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },

    /// Badge text changed (OSC 1337 SetBadgeFormat)
    #[serde(rename = "badge_changed")]
    BadgeChanged {
        /// New badge text (None if cleared)
        #[serde(skip_serializing_if = "Option::is_none")]
        badge: Option<String>,
    },

    /// Selection changed
    #[serde(rename = "selection_changed")]
    SelectionChanged {
        /// Start column (None if cleared)
        #[serde(skip_serializing_if = "Option::is_none")]
        start_col: Option<u16>,
        /// Start row (None if cleared)
        #[serde(skip_serializing_if = "Option::is_none")]
        start_row: Option<u16>,
        /// End column (None if cleared)
        #[serde(skip_serializing_if = "Option::is_none")]
        end_col: Option<u16>,
        /// End row (None if cleared)
        #[serde(skip_serializing_if = "Option::is_none")]
        end_row: Option<u16>,
        /// Selected text content
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// Selection mode: "chars", "line", "block"
        mode: String,
        /// True if selection was cleared
        cleared: bool,
    },

    /// Clipboard sync event (OSC 52)
    #[serde(rename = "clipboard_sync")]
    ClipboardSync {
        /// Operation: "set", "get_response"
        operation: String,
        /// Clipboard content
        content: String,
        /// Clipboard target: "clipboard", "primary", "select"
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },

    /// Shell integration event (FinalTerm sequences)
    #[serde(rename = "shell_integration")]
    ShellIntegrationEvent {
        /// Event type: "prompt_start", "command_start", "command_executed", "command_finished"
        event_type: String,
        /// The command text (for command_start)
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// Exit code (for command_finished)
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        /// Timestamp (Unix epoch milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
        /// Absolute cursor line (scrollback_len + cursor_row) at marker time
        #[serde(skip_serializing_if = "Option::is_none")]
        cursor_line: Option<u64>,
    },

    /// System resource statistics (CPU, memory, disk, network)
    #[serde(rename = "system_stats")]
    SystemStats {
        /// CPU statistics
        #[serde(skip_serializing_if = "Option::is_none")]
        cpu: Option<CpuStats>,
        /// Memory statistics
        #[serde(skip_serializing_if = "Option::is_none")]
        memory: Option<MemoryStats>,
        /// Disk statistics
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        disks: Vec<DiskStats>,
        /// Network interface statistics
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        networks: Vec<NetworkInterfaceStats>,
        /// System load averages
        #[serde(skip_serializing_if = "Option::is_none")]
        load_average: Option<LoadAverage>,
        /// Hostname
        #[serde(skip_serializing_if = "Option::is_none")]
        hostname: Option<String>,
        /// Operating system name
        #[serde(skip_serializing_if = "Option::is_none")]
        os_name: Option<String>,
        /// Operating system version
        #[serde(skip_serializing_if = "Option::is_none")]
        os_version: Option<String>,
        /// Kernel version
        #[serde(skip_serializing_if = "Option::is_none")]
        kernel_version: Option<String>,
        /// System uptime in seconds
        #[serde(skip_serializing_if = "Option::is_none")]
        uptime_secs: Option<u64>,
        /// Timestamp (Unix epoch milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<u64>,
    },

    /// Zone opened (prompt, command, or output block started)
    #[serde(rename = "zone_opened")]
    ZoneOpened {
        /// Unique zone identifier
        zone_id: u64,
        /// Zone type: "prompt", "command", "output"
        zone_type: String,
        /// Absolute row where zone starts
        abs_row_start: u64,
    },

    /// Zone closed (prompt, command, or output block ended)
    #[serde(rename = "zone_closed")]
    ZoneClosed {
        /// Unique zone identifier
        zone_id: u64,
        /// Zone type
        zone_type: String,
        /// Absolute row where zone starts
        abs_row_start: u64,
        /// Absolute row where zone ends
        abs_row_end: u64,
        /// Exit code (for output zones only)
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },

    /// Zone evicted from scrollback
    #[serde(rename = "zone_scrolled_out")]
    ZoneScrolledOut {
        /// Unique zone identifier
        zone_id: u64,
        /// Zone type
        zone_type: String,
    },

    /// Environment variable changed
    #[serde(rename = "environment_changed")]
    EnvironmentChanged {
        /// Key that changed ("cwd", "hostname", "username")
        key: String,
        /// New value
        value: String,
        /// Previous value
        #[serde(skip_serializing_if = "Option::is_none")]
        old_value: Option<String>,
    },

    /// Remote host transition detected
    #[serde(rename = "remote_host_transition")]
    RemoteHostTransition {
        /// New hostname
        hostname: String,
        /// New username
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// Previous hostname
        #[serde(skip_serializing_if = "Option::is_none")]
        old_hostname: Option<String>,
        /// Previous username
        #[serde(skip_serializing_if = "Option::is_none")]
        old_username: Option<String>,
    },

    /// Sub-shell detected
    #[serde(rename = "sub_shell_detected")]
    SubShellDetected {
        /// Current shell nesting depth
        depth: u64,
        /// Shell type if known
        #[serde(skip_serializing_if = "Option::is_none")]
        shell_type: Option<String>,
    },

    /// Semantic snapshot of terminal state
    #[serde(rename = "semantic_snapshot")]
    SemanticSnapshot {
        /// JSON-encoded SemanticSnapshot struct
        snapshot_json: String,
    },

    /// File transfer started (download or upload)
    #[serde(rename = "file_transfer_started")]
    FileTransferStarted {
        /// Transfer ID
        id: u64,
        /// Direction: "download" or "upload"
        direction: String,
        /// Filename if known
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Total bytes if known
        #[serde(skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
    },

    /// File transfer progress update
    #[serde(rename = "file_transfer_progress")]
    FileTransferProgress {
        /// Transfer ID
        id: u64,
        /// Bytes transferred so far
        bytes_transferred: u64,
        /// Total bytes if known
        #[serde(skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
    },

    /// File transfer completed successfully
    #[serde(rename = "file_transfer_completed")]
    FileTransferCompleted {
        /// Transfer ID
        id: u64,
        /// Filename if known
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Total bytes transferred
        size: u64,
    },

    /// File transfer failed
    #[serde(rename = "file_transfer_failed")]
    FileTransferFailed {
        /// Transfer ID
        id: u64,
        /// Failure reason
        reason: String,
    },

    /// Upload requested by terminal application
    #[serde(rename = "upload_requested")]
    UploadRequested {
        /// Upload format
        format: String,
    },
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    /// User input (keyboard)
    Input {
        /// Input data (can include escape sequences)
        data: String,
    },

    /// Terminal resize request
    Resize {
        /// Requested number of columns
        cols: u16,
        /// Requested number of rows
        rows: u16,
    },

    /// Ping for keepalive
    Ping,

    /// Request full screen refresh
    #[serde(rename = "refresh")]
    RequestRefresh,

    /// Subscribe to specific events
    Subscribe {
        /// Event types to subscribe to
        events: Vec<EventType>,
    },

    /// Mouse input from client
    Mouse {
        /// Column position
        col: u16,
        /// Row position
        row: u16,
        /// Button: 0=left, 1=middle, 2=right, 3=release, 4=scroll_up, 5=scroll_down
        button: u8,
        /// Shift key held
        shift: bool,
        /// Ctrl key held
        ctrl: bool,
        /// Alt key held
        alt: bool,
        /// Event type: "press", "release", "move", "scroll"
        event_type: String,
    },

    /// Focus change from client
    FocusChange {
        /// Whether the terminal is focused
        focused: bool,
    },

    /// Paste content from client
    Paste {
        /// Content to paste
        content: String,
    },

    /// Selection request from client
    SelectionRequest {
        /// Start column
        start_col: u16,
        /// Start row
        start_row: u16,
        /// End column
        end_col: u16,
        /// End row
        end_row: u16,
        /// Selection mode: "chars", "line", "block", "word", "clear"
        mode: String,
    },

    /// Clipboard request from client
    ClipboardRequest {
        /// Operation: "set", "get"
        operation: String,
        /// Content for "set" operations
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        /// Clipboard target: "clipboard", "primary", "select"
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },

    /// Request a semantic snapshot
    #[serde(rename = "snapshot_request")]
    SnapshotRequest {
        /// Scope: "visible", "recent", "full"
        scope: String,
        /// Max commands for "recent" scope
        #[serde(skip_serializing_if = "Option::is_none")]
        max_commands: Option<u32>,
    },
}

/// Event types that clients can subscribe to
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// Terminal output
    Output,
    /// Cursor position changes
    Cursor,
    /// Bell events
    Bell,
    /// Title changes
    Title,
    /// Resize events
    Resize,
    /// CWD change events
    Cwd,
    /// Trigger match events
    Trigger,
    /// Trigger action result events (Notify, MarkLine)
    Action,
    /// Terminal mode change events
    Mode,
    /// Graphics/image events
    Graphics,
    /// Hyperlink events
    Hyperlink,
    /// User variable change events
    #[serde(rename = "user_var")]
    UserVar,
    /// Named progress bar events
    #[serde(rename = "progress_bar")]
    ProgressBar,
    /// Badge change events
    Badge,
    /// Selection events
    Selection,
    /// Clipboard sync events
    Clipboard,
    /// Shell integration events
    Shell,
    /// System resource statistics events
    #[serde(rename = "system_stats")]
    SystemStats,
    /// Zone events (opened, closed, scrolled out)
    Zone,
    /// Environment change events
    Environment,
    /// Remote host transition events
    #[serde(rename = "remote_host")]
    RemoteHost,
    /// Sub-shell detection events
    #[serde(rename = "sub_shell")]
    SubShell,
    /// Semantic snapshot events
    #[serde(rename = "snapshot")]
    Snapshot,
    /// File transfer events (started, progress, completed, failed)
    #[serde(rename = "file_transfer")]
    FileTransfer,
    /// Upload request events
    #[serde(rename = "upload_request")]
    UploadRequest,
}

impl ServerMessage {
    /// Create a new output message
    pub fn output(data: String) -> Self {
        Self::Output {
            data,
            timestamp: None,
        }
    }

    /// Create a new output message with timestamp
    pub fn output_with_timestamp(data: String, timestamp: u64) -> Self {
        Self::Output {
            data,
            timestamp: Some(timestamp),
        }
    }

    /// Create a new resize message
    pub fn resize(cols: u16, rows: u16) -> Self {
        Self::Resize { cols, rows }
    }

    /// Create a new title message
    pub fn title(title: String) -> Self {
        Self::Title { title }
    }

    /// Create a new connected message
    pub fn connected(cols: u16, rows: u16, session_id: String) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: None,
            session_id,
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with initial screen
    pub fn connected_with_screen(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: Some(initial_screen),
            session_id,
            theme: None,
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with theme
    pub fn connected_with_theme(
        cols: u16,
        rows: u16,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: None,
            session_id,
            theme: Some(theme),
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a new connected message with initial screen and theme
    pub fn connected_with_screen_and_theme(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen: Some(initial_screen),
            session_id,
            theme: Some(theme),
            badge: None,
            faint_text_alpha: None,
            cwd: None,
            modify_other_keys: None,
            client_id: None,
            readonly: None,
        }
    }

    /// Create a fully-specified connected message with all terminal state
    #[allow(clippy::too_many_arguments)]
    pub fn connected_full(
        cols: u16,
        rows: u16,
        initial_screen: Option<String>,
        session_id: String,
        theme: Option<ThemeInfo>,
        badge: Option<String>,
        faint_text_alpha: Option<f32>,
        cwd: Option<String>,
        modify_other_keys: Option<u32>,
        client_id: Option<String>,
        readonly: Option<bool>,
    ) -> Self {
        Self::Connected {
            cols,
            rows,
            initial_screen,
            session_id,
            theme,
            badge,
            faint_text_alpha,
            cwd,
            modify_other_keys,
            client_id,
            readonly,
        }
    }

    /// Create a new refresh message with screen content
    pub fn refresh(cols: u16, rows: u16, screen_content: String) -> Self {
        Self::Refresh {
            cols,
            rows,
            screen_content,
        }
    }

    /// Create a new error message
    pub fn error(message: String) -> Self {
        Self::Error {
            message,
            code: None,
        }
    }

    /// Create a new error message with code
    pub fn error_with_code(message: String, code: String) -> Self {
        Self::Error {
            message,
            code: Some(code),
        }
    }

    /// Create a new cursor position message
    pub fn cursor(col: u16, row: u16, visible: bool) -> Self {
        Self::CursorPosition { col, row, visible }
    }

    /// Create a bell event message
    pub fn bell() -> Self {
        Self::Bell
    }

    /// Create a shutdown message
    pub fn shutdown(reason: String) -> Self {
        Self::Shutdown { reason }
    }

    /// Create a CWD changed message
    pub fn cwd_changed(new_cwd: String) -> Self {
        Self::CwdChanged {
            old_cwd: None,
            new_cwd,
            hostname: None,
            username: None,
            timestamp: None,
        }
    }

    /// Create a fully-specified CWD changed message
    pub fn cwd_changed_full(
        old_cwd: Option<String>,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
    ) -> Self {
        Self::CwdChanged {
            old_cwd,
            new_cwd,
            hostname,
            username,
            timestamp: Some(timestamp),
        }
    }

    /// Create a trigger matched message
    pub fn trigger_matched(
        trigger_id: u64,
        row: u16,
        col: u16,
        end_col: u16,
        text: String,
        captures: Vec<String>,
        timestamp: u64,
    ) -> Self {
        Self::TriggerMatched {
            trigger_id,
            row,
            col,
            end_col,
            text,
            captures,
            timestamp,
        }
    }

    /// Create an action notify message
    pub fn action_notify(trigger_id: u64, title: String, message: String) -> Self {
        Self::ActionNotify {
            trigger_id,
            title,
            message,
        }
    }

    /// Create an action mark line message
    pub fn action_mark_line(
        trigger_id: u64,
        row: u16,
        label: Option<String>,
        color: Option<(u8, u8, u8)>,
    ) -> Self {
        Self::ActionMarkLine {
            trigger_id,
            row,
            label,
            color,
        }
    }

    /// Create a pong message (keepalive response)
    pub fn pong() -> Self {
        Self::Pong
    }

    /// Create a mode changed message
    pub fn mode_changed(mode: String, enabled: bool) -> Self {
        Self::ModeChanged { mode, enabled }
    }

    /// Create a graphics added message
    pub fn graphics_added(row: u16) -> Self {
        Self::GraphicsAdded { row, format: None }
    }

    /// Create a graphics added message with format
    pub fn graphics_added_with_format(row: u16, format: String) -> Self {
        Self::GraphicsAdded {
            row,
            format: Some(format),
        }
    }

    /// Create a hyperlink added message
    pub fn hyperlink_added(url: String, row: u16, col: u16) -> Self {
        Self::HyperlinkAdded {
            url,
            row,
            col,
            id: None,
        }
    }

    /// Create a hyperlink added message with ID
    pub fn hyperlink_added_with_id(url: String, row: u16, col: u16, id: String) -> Self {
        Self::HyperlinkAdded {
            url,
            row,
            col,
            id: Some(id),
        }
    }

    /// Create a user variable changed message
    pub fn user_var_changed(name: String, value: String) -> Self {
        Self::UserVarChanged {
            name,
            value,
            old_value: None,
        }
    }

    /// Create a user variable changed message with old value
    pub fn user_var_changed_full(name: String, value: String, old_value: Option<String>) -> Self {
        Self::UserVarChanged {
            name,
            value,
            old_value,
        }
    }

    /// Create a badge changed message
    pub fn badge_changed(badge: Option<String>) -> Self {
        Self::BadgeChanged { badge }
    }

    /// Create a selection changed message
    #[allow(clippy::too_many_arguments)]
    pub fn selection_changed(
        start_col: Option<u16>,
        start_row: Option<u16>,
        end_col: Option<u16>,
        end_row: Option<u16>,
        text: Option<String>,
        mode: String,
        cleared: bool,
    ) -> Self {
        Self::SelectionChanged {
            start_col,
            start_row,
            end_col,
            end_row,
            text,
            mode,
            cleared,
        }
    }

    /// Create a selection cleared message
    pub fn selection_cleared() -> Self {
        Self::SelectionChanged {
            start_col: None,
            start_row: None,
            end_col: None,
            end_row: None,
            text: None,
            mode: "chars".to_string(),
            cleared: true,
        }
    }

    /// Create a clipboard sync message
    pub fn clipboard_sync(operation: String, content: String, target: Option<String>) -> Self {
        Self::ClipboardSync {
            operation,
            content,
            target,
        }
    }

    /// Create a shell integration event message
    pub fn shell_integration_event(
        event_type: String,
        command: Option<String>,
        exit_code: Option<i32>,
        timestamp: Option<u64>,
        cursor_line: Option<u64>,
    ) -> Self {
        Self::ShellIntegrationEvent {
            event_type,
            command,
            exit_code,
            timestamp,
            cursor_line,
        }
    }

    /// Create a system stats message
    #[allow(clippy::too_many_arguments)]
    pub fn system_stats(
        cpu: Option<CpuStats>,
        memory: Option<MemoryStats>,
        disks: Vec<DiskStats>,
        networks: Vec<NetworkInterfaceStats>,
        load_average: Option<LoadAverage>,
        hostname: Option<String>,
        os_name: Option<String>,
        os_version: Option<String>,
        kernel_version: Option<String>,
        uptime_secs: Option<u64>,
        timestamp: Option<u64>,
    ) -> Self {
        Self::SystemStats {
            cpu,
            memory,
            disks,
            networks,
            load_average,
            hostname,
            os_name,
            os_version,
            kernel_version,
            uptime_secs,
            timestamp,
        }
    }

    /// Create a zone opened message
    pub fn zone_opened(zone_id: u64, zone_type: String, abs_row_start: u64) -> Self {
        Self::ZoneOpened {
            zone_id,
            zone_type,
            abs_row_start,
        }
    }

    /// Create a zone closed message
    pub fn zone_closed(
        zone_id: u64,
        zone_type: String,
        abs_row_start: u64,
        abs_row_end: u64,
        exit_code: Option<i32>,
    ) -> Self {
        Self::ZoneClosed {
            zone_id,
            zone_type,
            abs_row_start,
            abs_row_end,
            exit_code,
        }
    }

    /// Create a zone scrolled out message
    pub fn zone_scrolled_out(zone_id: u64, zone_type: String) -> Self {
        Self::ZoneScrolledOut { zone_id, zone_type }
    }

    /// Create an environment changed message
    pub fn environment_changed(key: String, value: String, old_value: Option<String>) -> Self {
        Self::EnvironmentChanged {
            key,
            value,
            old_value,
        }
    }

    /// Create a remote host transition message
    pub fn remote_host_transition(
        hostname: String,
        username: Option<String>,
        old_hostname: Option<String>,
        old_username: Option<String>,
    ) -> Self {
        Self::RemoteHostTransition {
            hostname,
            username,
            old_hostname,
            old_username,
        }
    }

    /// Create a sub-shell detected message
    pub fn sub_shell_detected(depth: u64, shell_type: Option<String>) -> Self {
        Self::SubShellDetected { depth, shell_type }
    }

    /// Create a semantic snapshot message
    pub fn semantic_snapshot(snapshot_json: String) -> Self {
        Self::SemanticSnapshot { snapshot_json }
    }

    /// Create a file transfer started message
    pub fn file_transfer_started(
        id: u64,
        direction: String,
        filename: Option<String>,
        total_bytes: Option<u64>,
    ) -> Self {
        Self::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        }
    }

    /// Create a file transfer progress message
    pub fn file_transfer_progress(
        id: u64,
        bytes_transferred: u64,
        total_bytes: Option<u64>,
    ) -> Self {
        Self::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        }
    }

    /// Create a file transfer completed message
    pub fn file_transfer_completed(id: u64, filename: Option<String>, size: u64) -> Self {
        Self::FileTransferCompleted { id, filename, size }
    }

    /// Create a file transfer failed message
    pub fn file_transfer_failed(id: u64, reason: String) -> Self {
        Self::FileTransferFailed { id, reason }
    }

    /// Create an upload requested message
    pub fn upload_requested(format: String) -> Self {
        Self::UploadRequested { format }
    }

    /// Create a progress bar changed message from terminal event data
    pub fn progress_bar_changed(
        action: crate::terminal::ProgressBarAction,
        id: String,
        state: Option<crate::terminal::ProgressState>,
        percent: Option<u8>,
        label: Option<String>,
    ) -> Self {
        let action_str = match action {
            crate::terminal::ProgressBarAction::Set => "set",
            crate::terminal::ProgressBarAction::Remove => "remove",
            crate::terminal::ProgressBarAction::RemoveAll => "remove_all",
        };
        Self::ProgressBarChanged {
            action: action_str.to_string(),
            id,
            state: state.map(|s| s.description().to_string()),
            percent,
            label,
        }
    }
}

impl ClientMessage {
    /// Create a new input message
    pub fn input(data: String) -> Self {
        Self::Input { data }
    }

    /// Create a new resize message
    pub fn resize(cols: u16, rows: u16) -> Self {
        Self::Resize { cols, rows }
    }

    /// Create a ping message
    pub fn ping() -> Self {
        Self::Ping
    }

    /// Create a refresh request message
    pub fn request_refresh() -> Self {
        Self::RequestRefresh
    }

    /// Create a subscribe message
    pub fn subscribe(events: Vec<EventType>) -> Self {
        Self::Subscribe { events }
    }

    /// Create a mouse input message
    #[allow(clippy::too_many_arguments)]
    pub fn mouse(
        col: u16,
        row: u16,
        button: u8,
        shift: bool,
        ctrl: bool,
        alt: bool,
        event_type: String,
    ) -> Self {
        Self::Mouse {
            col,
            row,
            button,
            shift,
            ctrl,
            alt,
            event_type,
        }
    }

    /// Create a focus change message
    pub fn focus_change(focused: bool) -> Self {
        Self::FocusChange { focused }
    }

    /// Create a paste message
    pub fn paste(content: String) -> Self {
        Self::Paste { content }
    }

    /// Create a selection request message
    pub fn selection_request(
        start_col: u16,
        start_row: u16,
        end_col: u16,
        end_row: u16,
        mode: String,
    ) -> Self {
        Self::SelectionRequest {
            start_col,
            start_row,
            end_col,
            end_row,
            mode,
        }
    }

    /// Create a clipboard request message
    pub fn clipboard_request(
        operation: String,
        content: Option<String>,
        target: Option<String>,
    ) -> Self {
        Self::ClipboardRequest {
            operation,
            content,
            target,
        }
    }

    /// Create a snapshot request message
    pub fn snapshot_request(scope: String, max_commands: Option<u32>) -> Self {
        Self::SnapshotRequest {
            scope,
            max_commands,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_message_output_serialization() {
        let msg = ServerMessage::output("Hello, World!".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"output"#));
        assert!(json.contains(r#""data":"Hello, World!"#));

        // Deserialize back
        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerMessage::Output { data, timestamp } => {
                assert_eq!(data, "Hello, World!");
                assert_eq!(timestamp, None);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_server_message_resize_serialization() {
        let msg = ServerMessage::resize(80, 24);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"resize"#));
        assert!(json.contains(r#""cols":80"#));
        assert!(json.contains(r#""rows":24"#));
    }

    #[test]
    fn test_client_message_input_serialization() {
        let msg = ClientMessage::input("ls\n".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"input"#));
        assert!(json.contains(r#""data":"ls\n"#));

        // Deserialize back
        let deserialized: ClientMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ClientMessage::Input { data } => {
                assert_eq!(data, "ls\n");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_client_message_resize_serialization() {
        let msg = ClientMessage::resize(100, 30);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"resize"#));
        assert!(json.contains(r#""cols":100"#));
        assert!(json.contains(r#""rows":30"#));
    }

    #[test]
    fn test_server_message_error_serialization() {
        let msg = ServerMessage::error("Something went wrong".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error"#));
        assert!(json.contains(r#""message":"Something went wrong"#));
    }

    #[test]
    fn test_server_message_connected_serialization() {
        let msg = ServerMessage::connected(80, 24, "session-123".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"connected"#));
        assert!(json.contains(r#""session_id":"session-123"#));
        assert!(!json.contains(r#""initial_screen"#)); // Should be omitted when None
    }

    #[test]
    fn test_event_type_serialization() {
        let events = vec![EventType::Output, EventType::Bell];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""output"#));
        assert!(json.contains(r#""bell"#));
    }

    #[test]
    fn test_theme_info_serialization() {
        let theme = ThemeInfo {
            name: "test-theme".to_string(),
            background: (0, 0, 0),
            foreground: (255, 255, 255),
            normal: [
                (0, 0, 0),
                (255, 0, 0),
                (0, 255, 0),
                (255, 255, 0),
                (0, 0, 255),
                (255, 0, 255),
                (0, 255, 255),
                (255, 255, 255),
            ],
            bright: [
                (128, 128, 128),
                (255, 128, 128),
                (128, 255, 128),
                (255, 255, 128),
                (128, 128, 255),
                (255, 128, 255),
                (128, 255, 255),
                (255, 255, 255),
            ],
        };

        let json = serde_json::to_string(&theme).unwrap();
        assert!(json.contains(r#""name":"test-theme"#));
        assert!(json.contains(r#""background":[0,0,0]"#));
        assert!(json.contains(r#""foreground":[255,255,255]"#));

        // Deserialize back
        let deserialized: ThemeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-theme");
        assert_eq!(deserialized.background, (0, 0, 0));
        assert_eq!(deserialized.foreground, (255, 255, 255));
    }

    #[test]
    fn test_connected_message_with_theme() {
        let theme = ThemeInfo {
            name: "test-theme".to_string(),
            background: (0, 0, 0),
            foreground: (255, 255, 255),
            normal: [
                (0, 0, 0),
                (255, 0, 0),
                (0, 255, 0),
                (255, 255, 0),
                (0, 0, 255),
                (255, 0, 255),
                (0, 255, 255),
                (255, 255, 255),
            ],
            bright: [
                (128, 128, 128),
                (255, 128, 128),
                (128, 255, 128),
                (255, 255, 128),
                (128, 128, 255),
                (255, 128, 255),
                (128, 255, 255),
                (255, 255, 255),
            ],
        };

        let msg = ServerMessage::connected_with_theme(80, 24, "session-123".to_string(), theme);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"connected"#));
        assert!(json.contains(r#""session_id":"session-123"#));
        assert!(json.contains(r#""theme":{"#));
        assert!(json.contains(r#""name":"test-theme"#));
    }

    #[test]
    fn test_connected_full_serialization() {
        let msg = ServerMessage::connected_full(
            120,
            40,
            None,
            "session-full".to_string(),
            None,
            Some("mybadge".to_string()),
            Some(0.5),
            Some("/home/user".to_string()),
            Some(2),
            None,
            None,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""badge":"mybadge"#));
        assert!(json.contains(r#""faint_text_alpha":0.5"#));
        assert!(json.contains(r#""cwd":"/home/user"#));
        assert!(json.contains(r#""modify_other_keys":2"#));
    }

    #[test]
    fn test_cwd_changed_serialization() {
        let msg = ServerMessage::cwd_changed_full(
            Some("/old/dir".to_string()),
            "/new/dir".to_string(),
            Some("myhost".to_string()),
            Some("user".to_string()),
            1234567890,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"cwdchanged"#));
        assert!(json.contains(r#""new_cwd":"/new/dir"#));
        assert!(json.contains(r#""old_cwd":"/old/dir"#));
    }

    #[test]
    fn test_trigger_matched_serialization() {
        let msg = ServerMessage::trigger_matched(
            42,
            10,
            5,
            15,
            "matched text".to_string(),
            vec!["group1".to_string()],
            9999999,
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"triggermatched"#));
        assert!(json.contains(r#""trigger_id":42"#));
        assert!(json.contains(r#""text":"matched text"#));
    }

    #[test]
    fn test_event_type_cwd_trigger_serialization() {
        let events = vec![EventType::Cwd, EventType::Trigger];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""cwd"#));
        assert!(json.contains(r#""trigger"#));
    }

    #[test]
    fn test_semantic_snapshot_serialization() {
        let msg = ServerMessage::semantic_snapshot("{\"cols\":80}".to_string());
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"semantic_snapshot""#));
        assert!(json.contains(r#""snapshot_json"#));

        let deserialized: ServerMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerMessage::SemanticSnapshot { snapshot_json } => {
                assert_eq!(snapshot_json, "{\"cols\":80}");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_snapshot_request_serialization() {
        let msg = ClientMessage::snapshot_request("recent".to_string(), Some(5));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"snapshot_request""#));

        let deserialized: ClientMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            ClientMessage::SnapshotRequest {
                scope,
                max_commands,
            } => {
                assert_eq!(scope, "recent");
                assert_eq!(max_commands, Some(5));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_event_type_snapshot_serialization() {
        let events = vec![EventType::Snapshot];
        let json = serde_json::to_string(&events).unwrap();
        assert!(json.contains(r#""snapshot""#));
    }
}
