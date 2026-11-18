//! par-mux agent roster and session-tree payload types carried by
//! [`ServerMessage`](super::ServerMessage) variants.

use serde::{Deserialize, Serialize};

/// One par-mux agent roster entry, verbatim from `list-agents`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    proto(wire = "crate::streaming::proto::pb::AgentEntry")
)]
pub struct AgentEntry {
    /// par-mux pane number (`%N`)
    pub pane_id: u32,
    /// Agent name
    pub agent: String,
    /// par-mux state string (working, blocked, ...)
    pub state: String,
    /// `hook`, `scrape`, `self`, or empty when the daemon reported no provenance
    pub source: String,
    /// Decoded blocked reason, empty when none
    pub reason: String,
    /// par-mux workspace id (`+N`) from the roster row's `ws=` token,
    /// empty when the daemon pre-dates it or the pane is unassigned
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(skip)
    )]
    pub workspace: String,
    /// Host-attributed project, empty when unknown — no streamer source
    /// attributes one today; consumers render absent as unknown
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(skip)
    )]
    pub project: String,
    /// Pane title/label, empty when unknown — the daemon knows titles but
    /// offers no global query a streamer can bind; consumers render
    /// absent as unknown
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(skip)
    )]
    pub pane_label: String,
    /// Unix ms of the pane's last processed output read or accepted hook
    /// report, from the row's `last_activity_at=` token (0 = absent)
    pub last_activity_at: u64,
    /// Unix ms of the pane's last accepted hook report, from
    /// `last_report_at=` (0 = absent; scrape classification never sets it)
    pub last_report_at: u64,
    /// Unix ms of the last CLIENT-sent keystroke or paste, from
    /// `last_input_at=` (0 = absent) — the human-presence clock
    pub last_input_at: u64,
    /// Attached clients whose focused window shows the pane, from
    /// `viewer_count=`
    pub viewer_count: u32,
    /// Unix ms a viewer last held the pane, from `last_viewed_at=`
    /// (0 = absent; live while viewed, frozen at the last viewer's leave)
    pub last_viewed_at: u64,
    /// Cumulative CPU milliseconds of the pane's child process tree, from
    /// `cpu_ms=` (0 = absent or dead pane)
    pub cpu_ms: u64,
    /// `working` with no activity for the daemon's stall window, from
    /// `stalled=1`
    pub stalled: bool,
}

/// One pane in a [`ServerMessage::MuxTreeSnapshot`](super::ServerMessage::MuxTreeSnapshot): `%N` id, its window's
/// active-pane marker, and the rectangle the window layout gives it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    proto(wire = "crate::streaming::proto::pb::MuxTreePane")
)]
pub struct MuxTreePane {
    /// par-mux pane id (`%N`)
    pub id: String,
    /// Effective pane title, empty when none
    pub title: String,
    /// The window's active pane
    pub active: bool,
    /// Layout-rect columns
    pub cols: u32,
    /// Layout-rect rows
    pub rows: u32,
    /// Layout-rect column origin
    pub x: u32,
    /// Layout-rect row origin
    pub y: u32,
    /// The pane's process lifecycle state: `"running"` | `"exited"`.
    /// An empty string comes from a producer that predates the field and
    /// reads as running.
    pub process_state: String,
    /// The captured exit status while `"exited"`; `None` while running
    /// and when the daemon had no status to read
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// One window in a [`ServerMessage::MuxTreeSnapshot`](super::ServerMessage::MuxTreeSnapshot): `@N` id, its
/// session's active-window marker, and the split geometry. `layout` is the
/// full (unzoomed) tmux layout string — the authoritative geometry — and
/// `panes` carry the same geometry pre-parsed into per-pane rectangles.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    proto(wire = "crate::streaming::proto::pb::MuxTreeWindow")
)]
pub struct MuxTreeWindow {
    /// par-mux window id (`@N`)
    pub id: String,
    /// Window name
    pub name: String,
    /// Its session's current window
    pub active: bool,
    /// Full unzoomed tmux layout string
    pub layout: String,
    /// The window's panes in layout leaf order
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(nested(keep_empty))
    )]
    pub panes: Vec<MuxTreePane>,
}

/// One session in a [`ServerMessage::MuxTreeSnapshot`](super::ServerMessage::MuxTreeSnapshot): `$N` id, its
/// workspace's active-session marker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    proto(wire = "crate::streaming::proto::pb::MuxTreeSession")
)]
pub struct MuxTreeSession {
    /// par-mux session id (`$N`)
    pub id: String,
    /// Session name
    pub name: String,
    /// Its workspace's active session
    pub active: bool,
    /// The session's windows in window order
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(nested(keep_empty))
    )]
    pub windows: Vec<MuxTreeWindow>,
}

/// One workspace in a [`ServerMessage::MuxTreeSnapshot`](super::ServerMessage::MuxTreeSnapshot): `+N` id, the
/// daemon's active-workspace marker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    proto(wire = "crate::streaming::proto::pb::MuxTreeWorkspace")
)]
pub struct MuxTreeWorkspace {
    /// par-mux workspace id (`+N`)
    pub id: String,
    /// Workspace name
    pub name: String,
    /// The daemon's active workspace
    pub active: bool,
    /// The workspace's sessions in session order
    #[cfg_attr(
        all(
            any(feature = "python", feature = "python-test"),
            feature = "streaming"
        ),
        pydict(nested(keep_empty))
    )]
    pub sessions: Vec<MuxTreeSession>,
}
