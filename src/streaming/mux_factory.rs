//! Streaming sessions backed by par-mux panes.
//!
//! [`MuxSessionFactory`] is a [`SessionFactory`](crate::streaming::server::SessionFactory) whose sessions mirror a pane
//! the par-mux daemon owns instead of spawning a PTY of their own. The pane
//! keeps ONE terminal, in the daemon; each streaming session holds a mirror
//! `Terminal` fed by the daemon's screen replay plus its `%output` deltas, and
//! is never a second owner (par-mux.md D2/R7). Several streaming sessions may
//! mirror the same pane; each has its own daemon connection.
//!
//! One connection per session, read in order by one reader thread whose
//! lines feed first the handshake in `create_session`, then the drain thread.
//! The seed (`refresh-client -t %N -G` — the screen replay plus the pane's
//! graphics placements, with a plain-form retry when an older daemon
//! rejects the flag) is read in `create_session` against a reply deadline,
//! and `%output` pushed before its reply block is discarded (the replay
//! already reflects it); everything after is applied. Ordering is
//! the reason this does not use [`crate::mux::MuxClient`], which splits
//! replies and notifications into separate channels.
//!
//! `create_session` blocks on daemon I/O, so the streaming server calls it
//! off the async runtime (`spawn_blocking`).
//!
//! Size policy: the daemon's smallest-attached-client rule — a window's
//! extent is the componentwise minimum over the render clients displaying
//! it, and a streaming viewer's `Resize` becomes `refresh-client -t %N -C
//! WxH`, one sizing contribution per streaming server connection. The
//! mirror is seeded at the pane's current size (`pane-info`), never the
//! viewer's, and re-fits only when a `%layout-change` changes that pane's
//! rectangle.
//!
//! Layout: `lifecycle` (factory construction, pane selection, the
//! `SessionFactory` impl), `stream` (the drain thread and the input path),
//! `mirror` (the mirror link, viewer status, seed, workspace follow),
//! `control` (the daemon line reader and reply helper), and `upload` (the
//! client-upload pipe).

mod control;
mod lifecycle;
mod mirror;
mod stream;
mod upload;

pub use upload::MuxUploadPipe;

use crate::streaming::server::StreamingServer;
use mirror::MirrorLink;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How long `create_session` waits for each handshake reply block. Same
/// value as [`crate::mux::MuxClient`]'s reply timeout.
const MUX_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// The session id that makes a [`MuxPaneSelector::FromSessionId`] mirror
/// follow the daemon's active workspace: each `%workspaces-changed` the
/// daemon broadcasts re-resolves the mirror to the active workspace's
/// active pane and re-seeds the viewers in place, no reconnect.
pub const ACTIVE_WORKSPACE_SESSION_ID: &str = "workspace-active";

/// Which daemon pane a streaming session mirrors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxPaneSelector {
    /// The same pane for every session.
    Pane(u32),
    /// Parse the streaming session id: `pane-N` mirrors pane N;
    /// [`ACTIVE_WORKSPACE_SESSION_ID`] follows the daemon's active
    /// workspace; any other id falls back to the daemon's first pane
    /// (`list-panes`).
    FromSessionId,
}

/// A [`SessionFactory`](crate::streaming::server::SessionFactory) whose sessions mirror par-mux panes.
pub struct MuxSessionFactory {
    socket: PathBuf,
    selector: MuxPaneSelector,
    scrollback: usize,
    reply_timeout: Duration,
    sessions: RwLock<HashMap<String, Arc<MirrorLink>>>,
    /// One client-upload pipe per session (the upload verbs' own daemon
    /// connection); registered at session creation, removed at teardown.
    pipes: RwLock<HashMap<String, Arc<MuxUploadPipe>>>,
    // Weak on purpose: the server holds this factory strongly
    // (`with_factory`), so a strong back-reference is a reference cycle
    // that keeps every session — and each session's daemon-connection
    // drain thread holds the server strongly in turn — alive after the
    // last real owner drops. Measured as the stress test hanging at
    // runtime teardown: the enqueue drain's blocking task never sees the
    // channel close because the cycle keeps the session state alive.
    streaming_server: RwLock<Option<std::sync::Weak<StreamingServer>>>,
}

/// par-mux's load-independent waits: the hang ceiling every positive wait
/// in `tests` takes (one source of truth across the mux test suites).
#[cfg(test)]
#[path = "../../crates/par-mux/src/mux/test_wait.rs"]
mod test_wait;

#[cfg(test)]
mod tests;
