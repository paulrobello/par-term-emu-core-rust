//! Server-to-client messages: the [`ServerMessage`] enum, its constructors,
//! and [`ConnectedBuilder`].

use super::{
    AgentEntry, CpuStats, DamageRange, DiskStats, InlineImageFrame, LoadAverage, MemoryStats,
    MuxTreeWorkspace, NetworkInterfaceStats, ThemeInfo,
};
use serde::{Deserialize, Serialize};

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    all(
        any(feature = "python", feature = "python-test"),
        feature = "streaming"
    ),
    derive(par_term_emu_derive::PyDictConvert)
)]
#[cfg_attr(feature = "streaming", derive(par_term_emu_derive::ProtoConvert))]
#[cfg_attr(
    feature = "streaming",
    proto(
        wire = "crate::streaming::proto::pb::ServerMessage",
        module = "crate::streaming::proto::pb",
        oneof = "crate::streaming::proto::pb::server_message::Message",
        empty = "Empty server message"
    )
)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    /// Terminal output data (raw ANSI escape sequences)
    Output {
        /// Raw terminal output data including ANSI sequences
        data: String,
        /// Optional timestamp (Unix epoch in milliseconds)
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        timestamp: Option<u64>,
        /// Viewport rows the accompanying `data` changed relative to the
        /// pane's previous state (ascending, in-bounds). Empty means
        /// "no damage info" ONLY together with an absent generation:
        /// present generation + empty list = the chunk changed no viewport
        /// row. JSON omits the key when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        damage: Vec<DamageRange>,
        /// The pane's damage generation after `data` applied (monotonic per
        /// pane). Present marks the damage fields authoritative —
        /// distinguishes "no change" (present, `damage` empty) from "no
        /// damage info" (both absent = full scan).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        damage_generation: Option<u64>,
    },

    /// Terminal size changed
    Resize {
        /// Number of columns
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 80)
        )]
        cols: u16,
        /// Number of rows
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 24)
        )]
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
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 80)
        )]
        cols: u16,
        /// Current terminal height in rows
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 24)
        )]
        rows: u16,
        /// Optional initial screen content
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_screen: Option<String>,
        /// Session ID for this connection
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = uuid::Uuid::new_v4().to_string()))]
        session_id: String,
        /// Optional theme information
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(
                to_with = "crate::streaming::py_convert::theme_to_py",
                from_with = "crate::streaming::py_convert::theme_from_py"
            )
        )]
        theme: Option<ThemeInfo>,
        /// Current badge text (from OSC 1337)
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        badge: Option<String>,
        /// Faint text alpha for SGR 2 dim text (0.0-1.0)
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        faint_text_alpha: Option<f32>,
        /// Current working directory
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        cwd: Option<String>,
        /// modifyOtherKeys mode (0=disabled, 1=special keys, 2=all keys)
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        modify_other_keys: Option<u32>,
        /// Unique client identifier for this connection
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        client_id: Option<String>,
        /// Whether this connection is read-only
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        readonly: Option<bool>,
        /// Pointer shape requested via OSC 22 (`ew-resize`, `text`, …);
        /// absent = the client's own pointer applies
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        pointer_shape: Option<String>,
        /// Frames already on screen, replayed so a late joiner sees the
        /// graphics too
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        graphics: Vec<InlineImageFrame>,
        /// Server admits [`ClientMessage::PaneCreateRequest`](super::ClientMessage::PaneCreateRequest): the
        /// `allow_pane_create` config is on AND this connection is not
        /// read-only. Always set by `build_connect_message` so a client can
        /// tell "new server, disabled" from "old server" (field absent).
        /// `None` (absent on the wire) for hand-built defaults.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        pane_create_allowed: Option<bool>,
        /// The OSC 7 hostname accompanying the `cwd` when the cwd came
        /// from an OSC 7 report; `None` when the cwd has no OSC
        /// 7 host (a kernel-cwd or spawn-floor cwd is local by
        /// construction). Pairs with `cwd_is_local` so a client can both
        /// display the host and decide locality without guessing.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        cwd_hostname: Option<String>,
        /// Server-computed locality of `cwd` (SEC-128 rule): `true` when
        /// the path lives on the streaming host. `None` = field absent on
        /// the wire (pre-feature server).
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(encode_skip)
        )]
        cwd_is_local: Option<bool>,
    },

    /// Screen refresh response (full screen content)
    Refresh {
        /// Current terminal width in columns
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 80)
        )]
        cols: u16,
        /// Current terminal height in rows
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 24)
        )]
        rows: u16,
        /// Full screen content with ANSI styling
        screen_content: String,
    },

    /// Cursor position changed (optional optimization)
    #[serde(rename = "cursor")]
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(type = "cursor")
    )]
    #[cfg_attr(feature = "streaming", proto(oneof_variant = "Cursor"))]
    CursorPosition {
        /// Column position (0-indexed)
        col: u16,
        /// Row position (0-indexed)
        row: u16,
        /// Whether cursor is visible
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = true)
        )]
        visible: bool,
    },

    /// Bell event occurred
    Bell,

    /// Current working directory changed (OSC 7)
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(from = "crate::streaming::py_convert::cwd_changed_from")
    )]
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
        /// Server-computed locality of `new_cwd` (SEC-128 rule): `true`
        /// when the path lives on the streaming host. `None` = field
        /// absent on the wire (pre-feature sender).
        #[serde(skip_serializing_if = "Option::is_none")]
        is_local: Option<bool>,
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "Unknown error".to_string()))]
        message: String,
        /// Optional error code
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },

    /// Server is shutting down
    Shutdown {
        /// Reason for shutdown
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "Server shutdown".to_string()))]
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
        /// Serving session id (`pane-12`); absent = the streamed terminal itself
        #[serde(skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "set".to_string()))]
        action: String,
        /// Progress bar identifier
        id: String,
        /// State name (only for "set"): normal, indeterminate, warning, error
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        /// Progress percentage 0-100 (only for "set")
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            feature = "streaming",
            proto(with = "crate::streaming::proto::wire::percent")
        )]
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "chars".to_string()))]
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
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(type = "shell_integration")
    )]
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
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(from = "crate::streaming::py_convert::system_stats_from")
    )]
    SystemStats {
        /// CPU statistics
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        cpu: Option<CpuStats>,
        /// Memory statistics
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        memory: Option<MemoryStats>,
        /// Disk statistics
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        disks: Vec<DiskStats>,
        /// Network interface statistics
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        networks: Vec<NetworkInterfaceStats>,
        /// System load averages
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
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
    #[cfg_attr(feature = "streaming", proto(message = "SemanticSnapshotData"))]
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "download".to_string()))]
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "unknown".to_string()))]
        reason: String,
    },

    /// Upload requested by terminal application
    #[serde(rename = "upload_requested")]
    UploadRequested {
        /// Upload format
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "base64".to_string()))]
        format: String,
    },

    /// Screen cleared (ED 2J or ED 3J)
    #[serde(rename = "screen_cleared")]
    ScreenCleared {
        /// Whether scrollback was also cleared (ED 3J vs ED 2J)
        include_scrollback: bool,
    },

    /// Full par-mux agent roster snapshot (pane-id ascending). Sent once per
    /// client connect and again after the roster watcher reconnects.
    #[serde(rename = "agent_roster")]
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(from = "crate::streaming::py_convert::agent_roster_from")
    )]
    AgentRoster {
        /// Every roster entry
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested(keep_empty))
        )]
        agents: Vec<AgentEntry>,
    },

    /// One roster delta: upsert `agent`, or remove its pane when `released`.
    #[serde(rename = "agent_state_changed")]
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(from = "crate::streaming::py_convert::agent_state_changed_from")
    )]
    AgentStateChanged {
        /// The changed entry (only `pane_id` is meaningful when released)
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested)
        )]
        #[cfg_attr(
            feature = "streaming",
            proto(with = "crate::streaming::proto::wire::required_agent")
        )]
        agent: AgentEntry,
        /// True when the pane's agent was released or the pane exited
        released: bool,
    },

    /// Effective pane grid and independent minimum-size owners.
    #[serde(rename = "pane_size_status")]
    PaneSizeStatus {
        /// Effective columns.
        cols: u16,
        /// Effective rows.
        rows: u16,
        /// Column constraint owner, `attach:N` or `stream:N`.
        column_viewer: Option<String>,
        /// Row constraint owner, `attach:N` or `stream:N`.
        row_viewer: Option<String>,
    },
    /// Full replacement pane viewer set, including the receiver.
    #[serde(rename = "viewer_presence")]
    ViewerPresence {
        /// Total number of viewers.
        count: u32,
        /// Sorted `kind:id` entries. Kind is attach, web, or mobile.
        viewers: Vec<String>,
    },

    /// Pointer shape change (OSC 22)
    #[serde(rename = "pointer_shape_changed")]
    PointerShapeChanged {
        /// Requested shape name (`ew-resize`, `text`, …); `None` = the client's own pointer applies
        #[serde(skip_serializing_if = "Option::is_none")]
        shape: Option<String>,
    },

    /// Inline image frame: one image delivered to a pane via OSC 1337 (or a
    /// pane graphic forwarded by the mux streamer). Subscribes under
    /// [`EventType::Graphics`](super::EventType::Graphics).
    ///
    /// The payload is size-bounded: [`MAX_INLINE_IMAGE_BYTES`](super::MAX_INLINE_IMAGE_BYTES) is enforced
    /// on both wire boundaries — `encode_server_message` rejects oversize
    /// frames server-side, and the wire decode rejects them client-side —
    /// so an image can never enter or leave the wire unbounded.
    #[serde(rename = "inline_image")]
    InlineImage {
        /// Terminal graphic id, for replace/delete tracking
        image_id: u64,
        /// par-mux pane the image belongs to; `None` = the streamed terminal itself
        #[serde(skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        /// Payload format tag (`"rgba"`, `"png"`, `"jpeg"`, …)
        format: String,
        /// Pixel width
        width: u32,
        /// Pixel height
        height: u32,
        /// Encoded image bytes; at most [`MAX_INLINE_IMAGE_BYTES`](super::MAX_INLINE_IMAGE_BYTES)
        #[cfg_attr(
            feature = "streaming",
            proto(with = "crate::streaming::proto::wire::inline_image_data")
        )]
        data: Vec<u8>,
        /// Cell column of the image's top-left corner
        col: u32,
        /// Cell row of the image's top-left corner
        row: u32,
        /// Serving session id (`pane-12`); absent = no pane attribution
        #[serde(skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
    },

    /// A client upload began staging daemon-side; `upload_id` keys the
    /// matching `upload_finished`. Sent only to the uploading session.
    #[serde(rename = "upload_started")]
    UploadStarted {
        /// The daemon-side upload id (the `upload-begin` reply)
        upload_id: u64,
    },

    /// An upload settled: committed (`path` set — the bracket-pasted
    /// file), or failed/aborted (`error` set; a failed commit leaves the
    /// materialized file in place and names it in the message).
    #[serde(rename = "upload_finished")]
    UploadFinished {
        /// The daemon-side upload id (`0` when none was created)
        upload_id: u64,
        /// The materialized absolute path on a commit
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// The failure or abort reason
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Full par-mux tree snapshot: every workspace, session, window and
    /// pane (including inactive and non-agent ones). Pushed on connect and
    /// again after every structural change; the reply to
    /// `ClientMessage::MuxTreeRequest`; served from the watcher's cache.
    #[serde(rename = "mux_tree_snapshot")]
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(from = "crate::streaming::py_convert::mux_tree_snapshot_from")
    )]
    MuxTreeSnapshot {
        /// Every workspace, in workspace-id order
        #[serde(skip_serializing_if = "Vec::is_empty")]
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(nested(keep_empty))
        )]
        workspaces: Vec<MuxTreeWorkspace>,
    },

    /// A mirrored pane's process lifecycle transition. `exited` is the
    /// daemon's held-pane (remain-on-exit) state: the pane, its window
    /// and its frozen screen stay in the tree, and the stream stays open
    /// — this event names the pane and carries the status when known.
    /// `running` clears the exited state on `respawn-pane`, on the same
    /// connection. Late joiners receive the current state from the
    /// pane-info-derived snapshot at connect, never inferred from output
    /// silence. Pane deletion and daemon disconnect are DIFFERENT
    /// terminal conditions: deletion tears the pane out of the tree (the
    /// mirror session closes, `window_close` follows), a daemon
    /// disconnect closes the stream with `error`/`shutdown` — neither is
    /// ever reported as this event.
    #[serde(rename = "pane_process_state")]
    PaneProcessState {
        /// par-mux pane id (`%N`)
        pane_id: String,
        /// `"running"` | `"exited"`
        state: String,
        /// The captured exit status while `"exited"`; absent while
        /// running and when the daemon had no status to read
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },

    /// A requested pane was created daemon-side: one new default-shell
    /// window in the daemon session the request named. Echoes the
    /// request's `correlation_id` so a client can match a reply that
    /// arrives late — a drop between the daemon create and this reply
    /// leaves the pane existing, so a client that saw neither this reply
    /// nor an error must re-query the mux tree rather than retry blind.
    /// Opt-in server-side (`allow-pane-create`); refusals arrive as the
    /// error message with codes `pane_create_disabled`,
    /// `pane_create_read_only`, `pane_create_no_daemon`, or
    /// `pane_create_failed`.
    #[serde(rename = "pane_created")]
    PaneCreated {
        /// The request's correlation id, echoed
        correlation_id: String,
        /// The daemon session the window was created in (echoed)
        session_id: String,
        /// The new window (`@N`)
        window_id: String,
        /// The window's single pane (`%N`, default shell)
        pane_id: String,
    },
}

impl From<InlineImageFrame> for ServerMessage {
    fn from(frame: InlineImageFrame) -> Self {
        Self::InlineImage {
            image_id: frame.image_id,
            pane_id: frame.pane_id,
            format: frame.format,
            width: frame.width,
            height: frame.height,
            data: frame.data,
            col: frame.col,
            row: frame.row,
            pane: frame.pane,
        }
    }
}

/// Builder for [`ServerMessage::Connected`] (QA-003).
///
/// The next optional field is one method here, not another constructor.
/// Start from [`ServerMessage::connected_builder`] and chain the fields
/// you have; unset fields stay `None` on the wire.
#[derive(Debug, Clone, Default)]
pub struct ConnectedBuilder {
    /// Terminal width in columns
    pub cols: u16,
    /// Terminal height in rows
    pub rows: u16,
    /// Session identifier
    pub session_id: String,
    /// Optional initial screen content (ANSI-styled)
    pub initial_screen: Option<String>,
    /// Optional theme information
    pub theme: Option<ThemeInfo>,
    /// Optional badge text
    pub badge: Option<String>,
    /// Optional faint-text alpha override
    pub faint_text_alpha: Option<f32>,
    /// Optional current working directory
    pub cwd: Option<String>,
    /// Optional modifyOtherKeys level
    pub modify_other_keys: Option<u32>,
    /// Optional assigned client id
    pub client_id: Option<String>,
    /// Optional read-only flag
    pub readonly: Option<bool>,
    /// Optional pointer shape requested via OSC 22
    pub pointer_shape: Option<String>,
    /// Frames already on screen, replayed for a late joiner
    pub graphics: Vec<InlineImageFrame>,
    /// Pane-create capability advertised to the client; `None` = field
    /// absent on the wire (pre-feature servers omit it)
    pub pane_create_allowed: Option<bool>,
    /// The OSC 7 hostname accompanying `cwd`; `None` = no OSC 7 host
    /// (a kernel-cwd or spawn-floor cwd is local by construction)
    pub cwd_hostname: Option<String>,
    /// Server-computed locality of `cwd` (SEC-128); `None` = field
    /// absent on the wire (pre-feature servers omit it)
    pub cwd_is_local: Option<bool>,
}

impl ConnectedBuilder {
    /// Set terminal width in columns
    pub fn cols(mut self, cols: u16) -> Self {
        self.cols = cols;
        self
    }

    /// Set terminal height in rows
    pub fn rows(mut self, rows: u16) -> Self {
        self.rows = rows;
        self
    }

    /// Set session identifier
    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    /// Set initial screen content (ANSI-styled)
    pub fn initial_screen(mut self, screen: Option<String>) -> Self {
        self.initial_screen = screen;
        self
    }

    /// Set theme information
    pub fn theme(mut self, theme: Option<ThemeInfo>) -> Self {
        self.theme = theme;
        self
    }

    /// Set badge text
    pub fn badge(mut self, badge: Option<String>) -> Self {
        self.badge = badge;
        self
    }

    /// Set faint-text alpha override
    pub fn faint_text_alpha(mut self, alpha: Option<f32>) -> Self {
        self.faint_text_alpha = alpha;
        self
    }

    /// Set current working directory
    pub fn cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }

    /// Set modifyOtherKeys level
    pub fn modify_other_keys(mut self, mode: Option<u32>) -> Self {
        self.modify_other_keys = mode;
        self
    }

    /// Set assigned client id
    pub fn client_id(mut self, client_id: Option<String>) -> Self {
        self.client_id = client_id;
        self
    }

    /// Set read-only flag
    pub fn readonly(mut self, readonly: Option<bool>) -> Self {
        self.readonly = readonly;
        self
    }

    /// Set the pointer shape requested via OSC 22
    pub fn pointer_shape(mut self, shape: Option<String>) -> Self {
        self.pointer_shape = shape;
        self
    }

    /// Set the replayed inline-image frames
    pub fn graphics(mut self, graphics: Vec<InlineImageFrame>) -> Self {
        self.graphics = graphics;
        self
    }

    /// Advertise the pane-create capability (allow + writable); `None`
    /// leaves the field absent on the wire
    pub fn pane_create_allowed(mut self, allowed: Option<bool>) -> Self {
        self.pane_create_allowed = allowed;
        self
    }

    /// Set the OSC 7 hostname accompanying `cwd`; `None` leaves the
    /// field absent on the wire
    pub fn cwd_hostname(mut self, host: Option<String>) -> Self {
        self.cwd_hostname = host;
        self
    }

    /// Set the server-computed locality of `cwd`; `None` leaves the
    /// field absent on the wire
    pub fn cwd_is_local(mut self, local: Option<bool>) -> Self {
        self.cwd_is_local = local;
        self
    }

    /// Finish and produce the `Connected` message
    pub fn build(self) -> ServerMessage {
        ServerMessage::Connected {
            cols: self.cols,
            rows: self.rows,
            initial_screen: self.initial_screen,
            session_id: self.session_id,
            theme: self.theme,
            badge: self.badge,
            faint_text_alpha: self.faint_text_alpha,
            cwd: self.cwd,
            modify_other_keys: self.modify_other_keys,
            client_id: self.client_id,
            readonly: self.readonly,
            pointer_shape: self.pointer_shape,
            graphics: self.graphics,
            pane_create_allowed: self.pane_create_allowed,
            cwd_hostname: self.cwd_hostname,
            cwd_is_local: self.cwd_is_local,
        }
    }
}

impl ServerMessage {
    /// Create a new output message
    pub fn output(data: String) -> Self {
        Self::Output {
            data,
            timestamp: None,
            damage: Vec::new(),
            damage_generation: None,
        }
    }

    /// Create a new output message with timestamp
    pub fn output_with_timestamp(data: String, timestamp: u64) -> Self {
        Self::Output {
            data,
            timestamp: Some(timestamp),
            damage: Vec::new(),
            damage_generation: None,
        }
    }

    /// Create a new output message carrying damage: the inclusive viewport
    /// row ranges `data` changed (ascending, in-bounds) and the pane's
    /// damage generation after `data` applied. With `generation` `Some`,
    /// present ranges are authoritative; `Some` generation with empty
    /// ranges says the chunk changed no viewport row. `None` (or empty
    /// ranges with `None`) degrades to "no damage info — full scan".
    pub fn output_with_damage(
        data: String,
        generation: Option<u64>,
        damage: Vec<DamageRange>,
    ) -> Self {
        Self::Output {
            data,
            timestamp: None,
            damage,
            damage_generation: generation,
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

    /// Start building a `Connected` message (QA-003)
    ///
    /// The next optional field on `Connected` is one method on the
    /// returned [`ConnectedBuilder`], not another constructor.
    pub fn connected_builder(cols: u16, rows: u16, session_id: String) -> ConnectedBuilder {
        ConnectedBuilder {
            cols,
            rows,
            session_id,
            ..Default::default()
        }
    }

    /// Create a new connected message
    #[deprecated(note = "use connected_builder()")]
    pub fn connected(cols: u16, rows: u16, session_id: String) -> Self {
        Self::connected_builder(cols, rows, session_id).build()
    }

    /// Create a new connected message with initial screen
    #[deprecated(note = "use connected_builder()")]
    pub fn connected_with_screen(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
    ) -> Self {
        Self::connected_builder(cols, rows, session_id)
            .initial_screen(Some(initial_screen))
            .build()
    }

    /// Create a new connected message with theme
    #[deprecated(note = "use connected_builder()")]
    pub fn connected_with_theme(
        cols: u16,
        rows: u16,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::connected_builder(cols, rows, session_id)
            .theme(Some(theme))
            .build()
    }

    /// Create a new connected message with initial screen and theme
    #[deprecated(note = "use connected_builder()")]
    pub fn connected_with_screen_and_theme(
        cols: u16,
        rows: u16,
        initial_screen: String,
        session_id: String,
        theme: ThemeInfo,
    ) -> Self {
        Self::connected_builder(cols, rows, session_id)
            .initial_screen(Some(initial_screen))
            .theme(Some(theme))
            .build()
    }

    /// Create a fully-specified connected message with all terminal state
    #[deprecated(note = "use connected_builder()")]
    #[allow(
        clippy::too_many_arguments,
        reason = "deprecated public API, one argument per `Connected` field, kept for embedders; `ConnectedBuilder` replaces it"
    )]
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
        Self::connected_builder(cols, rows, session_id)
            .initial_screen(initial_screen)
            .theme(theme)
            .badge(badge)
            .faint_text_alpha(faint_text_alpha)
            .cwd(cwd)
            .modify_other_keys(modify_other_keys)
            .client_id(client_id)
            .readonly(readonly)
            .build()
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
            is_local: None,
        }
    }

    /// Create a fully-specified CWD changed message. `is_local` is the
    /// sender's SEC-128 locality verdict for `new_cwd` (the OSC 7
    /// hostname against the streaming host); the mux mirror and the
    /// standalone server both compute it with
    /// `mux::pane::osc7_host_is_local`.
    pub fn cwd_changed_full(
        old_cwd: Option<String>,
        new_cwd: String,
        hostname: Option<String>,
        username: Option<String>,
        timestamp: u64,
        is_local: bool,
    ) -> Self {
        Self::CwdChanged {
            old_cwd,
            new_cwd,
            hostname,
            username,
            timestamp: Some(timestamp),
            is_local: Some(is_local),
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
        Self::GraphicsAdded {
            row,
            format: None,
            pane: None,
        }
    }

    /// Create a graphics added message with format
    pub fn graphics_added_with_format(row: u16, format: String) -> Self {
        Self::GraphicsAdded {
            row,
            format: Some(format),
            pane: None,
        }
    }

    /// Create a graphics added message with pane attribution
    pub fn graphics_added_with_pane(row: u16, pane: Option<String>) -> Self {
        Self::GraphicsAdded {
            row,
            format: None,
            pane,
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

    /// Create a pointer shape changed message
    pub fn pointer_shape_changed(shape: Option<String>) -> Self {
        Self::PointerShapeChanged { shape }
    }

    /// Create an inline image frame.
    ///
    /// `data` must be at most [`MAX_INLINE_IMAGE_BYTES`](super::MAX_INLINE_IMAGE_BYTES); oversize payloads
    /// are rejected at the wire boundaries (`encode_server_message` and the
    /// wire decode), never truncated.
    #[allow(
        clippy::too_many_arguments,
        reason = "public API, one argument per `InlineImage` field; a parameter struct would break embedders"
    )]
    pub fn inline_image(
        image_id: u64,
        pane_id: Option<u32>,
        format: String,
        width: u32,
        height: u32,
        data: Vec<u8>,
        col: u32,
        row: u32,
        pane: Option<String>,
    ) -> Self {
        Self::InlineImage {
            image_id,
            pane_id,
            format,
            width,
            height,
            data,
            col,
            row,
            pane,
        }
    }

    /// Create a selection changed message
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
    #[allow(
        clippy::too_many_arguments,
        reason = "public API, one argument per `SystemStats` field; a parameter struct would break embedders"
    )]
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

    /// Create a screen cleared message
    pub fn screen_cleared(include_scrollback: bool) -> Self {
        Self::ScreenCleared { include_scrollback }
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

// ---------------------------------------------------------------------------
// Variant-name registries (QA-005)
//
// The golden fixture-coverage test (proto_golden_tests.rs) asserts the
// fixtures cover these name lists EXACTLY (both directions). The
// `variant_name` matches below are the compile-side tie: they are
// exhaustive with no wildcard arm (repo rule), so adding a variant is a
// compile error until it is named there, and the coverage test then
// demands its registry entry and a fixture. Keep registry and match in
// the same order.

/// Every [`ServerMessage`] variant name, in `variant_name` order.
#[cfg(all(test, feature = "streaming"))]
pub(crate) const SERVER_MESSAGE_VARIANT_NAMES: &[&str] = &[
    "Output",
    "Resize",
    "Title",
    "Connected",
    "Refresh",
    "CursorPosition",
    "Bell",
    "CwdChanged",
    "TriggerMatched",
    "ActionNotify",
    "ActionMarkLine",
    "Error",
    "Shutdown",
    "Pong",
    "ModeChanged",
    "GraphicsAdded",
    "HyperlinkAdded",
    "UserVarChanged",
    "ProgressBarChanged",
    "BadgeChanged",
    "SelectionChanged",
    "ClipboardSync",
    "ShellIntegrationEvent",
    "SystemStats",
    "ZoneOpened",
    "ZoneClosed",
    "ZoneScrolledOut",
    "EnvironmentChanged",
    "RemoteHostTransition",
    "SubShellDetected",
    "SemanticSnapshot",
    "FileTransferStarted",
    "FileTransferProgress",
    "FileTransferCompleted",
    "FileTransferFailed",
    "UploadRequested",
    "ScreenCleared",
    "AgentRoster",
    "AgentStateChanged",
    "PointerShapeChanged",
    "PaneSizeStatus",
    "ViewerPresence",
    "InlineImage",
    "UploadStarted",
    "UploadFinished",
    "MuxTreeSnapshot",
    "PaneProcessState",
    "PaneCreated",
];

#[cfg(feature = "streaming")]
impl ServerMessage {
    /// The variant's own name (`"Output"` for [`ServerMessage::Output`]),
    /// the fixture-coverage key. Exhaustive with no wildcard arm (repo
    /// rule), so an unnamed new variant fails to compile here.
    #[cfg(all(test, feature = "streaming"))]
    pub(crate) fn variant_name(&self) -> &'static str {
        match self {
            Self::Output { .. } => "Output",
            Self::Resize { .. } => "Resize",
            Self::Title { .. } => "Title",
            Self::Connected { .. } => "Connected",
            Self::Refresh { .. } => "Refresh",
            Self::CursorPosition { .. } => "CursorPosition",
            Self::Bell => "Bell",
            Self::CwdChanged { .. } => "CwdChanged",
            Self::TriggerMatched { .. } => "TriggerMatched",
            Self::ActionNotify { .. } => "ActionNotify",
            Self::ActionMarkLine { .. } => "ActionMarkLine",
            Self::Error { .. } => "Error",
            Self::Shutdown { .. } => "Shutdown",
            Self::Pong => "Pong",
            Self::ModeChanged { .. } => "ModeChanged",
            Self::GraphicsAdded { .. } => "GraphicsAdded",
            Self::HyperlinkAdded { .. } => "HyperlinkAdded",
            Self::UserVarChanged { .. } => "UserVarChanged",
            Self::ProgressBarChanged { .. } => "ProgressBarChanged",
            Self::BadgeChanged { .. } => "BadgeChanged",
            Self::SelectionChanged { .. } => "SelectionChanged",
            Self::ClipboardSync { .. } => "ClipboardSync",
            Self::ShellIntegrationEvent { .. } => "ShellIntegrationEvent",
            Self::SystemStats { .. } => "SystemStats",
            Self::ZoneOpened { .. } => "ZoneOpened",
            Self::ZoneClosed { .. } => "ZoneClosed",
            Self::ZoneScrolledOut { .. } => "ZoneScrolledOut",
            Self::EnvironmentChanged { .. } => "EnvironmentChanged",
            Self::RemoteHostTransition { .. } => "RemoteHostTransition",
            Self::SubShellDetected { .. } => "SubShellDetected",
            Self::SemanticSnapshot { .. } => "SemanticSnapshot",
            Self::FileTransferStarted { .. } => "FileTransferStarted",
            Self::FileTransferProgress { .. } => "FileTransferProgress",
            Self::FileTransferCompleted { .. } => "FileTransferCompleted",
            Self::FileTransferFailed { .. } => "FileTransferFailed",
            Self::UploadRequested { .. } => "UploadRequested",
            Self::ScreenCleared { .. } => "ScreenCleared",
            Self::AgentRoster { .. } => "AgentRoster",
            Self::AgentStateChanged { .. } => "AgentStateChanged",
            Self::PointerShapeChanged { .. } => "PointerShapeChanged",
            Self::PaneSizeStatus { .. } => "PaneSizeStatus",
            Self::ViewerPresence { .. } => "ViewerPresence",
            Self::InlineImage { .. } => "InlineImage",
            Self::UploadStarted { .. } => "UploadStarted",
            Self::UploadFinished { .. } => "UploadFinished",
            Self::MuxTreeSnapshot { .. } => "MuxTreeSnapshot",
            Self::PaneProcessState { .. } => "PaneProcessState",
            Self::PaneCreated { .. } => "PaneCreated",
        }
    }
}
