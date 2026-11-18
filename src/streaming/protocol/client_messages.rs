//! Client-to-server messages: the [`ClientMessage`] enum, its constructors,
//! and the [`MouseEventType`] of a mouse event.

use super::EventType;
use serde::{Deserialize, Serialize};

/// Messages sent from client to server
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
        wire = "crate::streaming::proto::pb::ClientMessage",
        module = "crate::streaming::proto::pb",
        oneof = "crate::streaming::proto::pb::client_message::Message",
        empty = "Empty client message"
    )
)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    /// Declare this connection's presence kind (web or mobile).
    #[serde(rename = "viewer_identity")]
    ViewerIdentity {
        /// Only web and mobile are accepted.
        kind: String,
    },
    /// User input (keyboard)
    Input {
        /// Input data (can include escape sequences)
        data: String,
    },

    /// Terminal resize request
    #[cfg_attr(feature = "streaming", proto(message = "ClientResize"))]
    Resize {
        /// Requested number of columns
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 80)
        )]
        cols: u16,
        /// Requested number of rows
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = 24)
        )]
        rows: u16,
    },

    /// Ping for keepalive
    Ping,

    /// Request full screen refresh
    #[serde(rename = "refresh")]
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(type = "refresh")
    )]
    #[cfg_attr(feature = "streaming", proto(oneof_variant = "Refresh"))]
    RequestRefresh,

    /// Subscribe to specific events
    Subscribe {
        /// Event types to subscribe to
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(
                to_with = "crate::streaming::py_convert::events_to_py",
                from_with = "crate::streaming::py_convert::events_from_py"
            )
        )]
        events: Vec<EventType>,
    },

    /// Mouse input from client. `button` is the raw SGR code space: low
    /// bits 0=left, 1=middle, 2=right, 3=none/release, plus the motion bit
    /// 32 and the wheel bit 64 ORed on top. A `move` event may carry the
    /// logical button (0 left-held, 3 buttonless); the server normalizes it
    /// with an idempotent `| 32` before encoding.
    #[cfg_attr(feature = "streaming", proto(message = "MouseInput"))]
    Mouse {
        /// Column position
        col: u16,
        /// Row position
        row: u16,
        /// Raw SGR code: 0=left, 1=middle, 2=right, 3=none/release,
        /// +32 motion flag, +64 wheel flag. Scroll uses 64/65; a `move`
        /// event's button is normalized to include the motion bit (0→32,
        /// 3→35) before the report is encoded.
        #[cfg_attr(
            feature = "streaming",
            proto(with = "crate::streaming::proto::wire::mouse_button")
        )]
        button: u8,
        /// Shift key held
        shift: bool,
        /// Ctrl key held
        ctrl: bool,
        /// Alt key held
        alt: bool,
        /// Press, release, move or scroll
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(
                to_with = "crate::streaming::py_convert::mouse_event_type_to_py",
                from_with = "crate::streaming::py_convert::mouse_event_type_from_py"
            )
        )]
        event_type: MouseEventType,
    },

    /// Focus change from client
    #[cfg_attr(feature = "streaming", proto(oneof_variant = "Focus"))]
    FocusChange {
        /// Whether the terminal is focused
        #[cfg_attr(
            all(
                any(feature = "python", feature = "python-test"),
                feature = "streaming"
            ),
            pydict(default = true)
        )]
        focused: bool,
    },

    /// Paste content from client
    #[cfg_attr(feature = "streaming", proto(message = "PasteInput"))]
    Paste {
        /// Content to paste
        content: String,
    },

    /// Selection request from client
    #[cfg_attr(feature = "streaming", proto(oneof_variant = "Selection"))]
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "chars".to_string()))]
        mode: String,
    },

    /// Clipboard request from client
    #[cfg_attr(feature = "streaming", proto(oneof_variant = "Clipboard"))]
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
        #[cfg_attr(all(any(feature = "python", feature = "python-test"), feature = "streaming"), pydict(default = "visible".to_string()))]
        scope: String,
        /// Max commands for "recent" scope
        #[serde(skip_serializing_if = "Option::is_none")]
        max_commands: Option<u32>,
    },

    /// Begin a chunked upload into this session's pane (mux-backed
    /// sessions; see MUX.md "Client uploads"): the server opens bounded
    /// daemon-side staging and reports the outcome through
    /// `upload_started` / `upload_finished`. A rejected chunk aborts the
    /// upload daemon-side.
    #[serde(rename = "upload_begin")]
    UploadBegin {
        /// The file's bare name (a path prefix is stripped daemon-side)
        filename: String,
        /// The byte total the chunks must land exactly
        total_bytes: u64,
    },

    /// Append one chunk to this session's in-progress upload. The bytes
    /// ride the protobuf `bytes` field; the payload is capped at
    /// [`MAX_UPLOAD_CHUNK_BYTES`](super::MAX_UPLOAD_CHUNK_BYTES) at the wire boundary.
    #[serde(rename = "upload_chunk")]
    UploadChunk {
        /// The chunk bytes; at most [`MAX_UPLOAD_CHUNK_BYTES`](super::MAX_UPLOAD_CHUNK_BYTES)
        #[cfg_attr(
            feature = "streaming",
            proto(with = "crate::streaming::proto::wire::upload_chunk_data")
        )]
        data: Vec<u8>,
    },

    /// Seal this session's staged upload and bracket-paste the
    /// materialized file's PATH into the pane (the documented input
    /// semantic: the bytes themselves never enter pane input).
    #[serde(rename = "upload_commit")]
    UploadCommit,

    /// Abandon this session's in-progress upload; its spool file is
    /// deleted daemon-side.
    #[serde(rename = "upload_abort")]
    UploadAbort,

    /// Request the current par-mux tree snapshot. Served from the
    /// streaming server's cache (no daemon round trip); structural changes
    /// arrive afterwards as fresh `mux_tree_snapshot` pushes.
    #[serde(rename = "mux_tree_request")]
    MuxTreeRequest,

    /// Create one new default-shell window (a single new pane) in the
    /// daemon session named by `session_id` (`$N` or name). Opt-in
    /// server-side (`allow-pane-create`, default off); read-only
    /// connections are refused regardless of the flag; non-mux sessions
    /// are refused with `pane_create_no_daemon`. `correlation_id` is
    /// client-chosen and echoed in the `pane_created` reply, so a client
    /// can match a reply that arrives late: a drop between the daemon
    /// create and the reply leaves the pane existing — re-query the mux
    /// tree rather than retrying blind.
    #[serde(rename = "pane_create_request")]
    PaneCreateRequest {
        /// Client-chosen id echoed in the `pane_created` reply
        correlation_id: String,
        /// Daemon session (`$N` or name) to create the window in
        session_id: String,
        /// Optional window name (card 01a1d0d5): the streamer validates it
        /// (bounded length, no control characters) and hands it to the
        /// daemon's `new-window -n`. Absent keeps the daemon's default
        /// name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// The kind of a [`ClientMessage::Mouse`] event.
///
/// On the protobuf wire and in the Python dict API it is the lowercase
/// name (`"press"`, `"release"`, `"move"`, `"scroll"`).
///
/// A `move` event's `button` is normalized: the server ORs in the SGR
/// motion bit 32 (button 0→32, 3→35; already-raw 32/35 pass unchanged).
/// `press`/`release`/`scroll` pass `button` through verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseEventType {
    /// Button pressed
    Press,
    /// Button released
    Release,
    /// Pointer moved
    Move,
    /// Wheel scrolled
    Scroll,
}

impl MouseEventType {
    /// The wire name of this event type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Release => "release",
            Self::Move => "move",
            Self::Scroll => "scroll",
        }
    }

    /// Parse a wire name; `None` for anything but the four exact names.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "press" => Some(Self::Press),
            "release" => Some(Self::Release),
            "move" => Some(Self::Move),
            "scroll" => Some(Self::Scroll),
            _ => None,
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
    pub fn mouse(
        col: u16,
        row: u16,
        button: u8,
        shift: bool,
        ctrl: bool,
        alt: bool,
        event_type: MouseEventType,
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

/// Every [`ClientMessage`] variant name, in `variant_name` order.
#[cfg(all(test, feature = "streaming"))]
pub(crate) const CLIENT_MESSAGE_VARIANT_NAMES: &[&str] = &[
    "ViewerIdentity",
    "Input",
    "Resize",
    "Ping",
    "RequestRefresh",
    "Subscribe",
    "Mouse",
    "FocusChange",
    "Paste",
    "SelectionRequest",
    "ClipboardRequest",
    "SnapshotRequest",
    "UploadBegin",
    "UploadChunk",
    "UploadCommit",
    "UploadAbort",
    "MuxTreeRequest",
    "PaneCreateRequest",
];

#[cfg(feature = "streaming")]
impl ClientMessage {
    /// The variant's own name (`"Input"` for [`ClientMessage::Input`]),
    /// the fixture-coverage key. Exhaustive with no wildcard arm (repo
    /// rule), so an unnamed new variant fails to compile here.
    #[cfg(all(test, feature = "streaming"))]
    pub(crate) fn variant_name(&self) -> &'static str {
        match self {
            Self::ViewerIdentity { .. } => "ViewerIdentity",
            Self::Input { .. } => "Input",
            Self::Resize { .. } => "Resize",
            Self::Ping => "Ping",
            Self::RequestRefresh => "RequestRefresh",
            Self::Subscribe { .. } => "Subscribe",
            Self::Mouse { .. } => "Mouse",
            Self::FocusChange { .. } => "FocusChange",
            Self::Paste { .. } => "Paste",
            Self::SelectionRequest { .. } => "SelectionRequest",
            Self::ClipboardRequest { .. } => "ClipboardRequest",
            Self::SnapshotRequest { .. } => "SnapshotRequest",
            Self::UploadBegin { .. } => "UploadBegin",
            Self::UploadChunk { .. } => "UploadChunk",
            Self::UploadCommit => "UploadCommit",
            Self::UploadAbort => "UploadAbort",
            Self::MuxTreeRequest => "MuxTreeRequest",
            Self::PaneCreateRequest { .. } => "PaneCreateRequest",
        }
    }
}
