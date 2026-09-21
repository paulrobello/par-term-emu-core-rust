//! Terminal multiplexer server.
//!
//! Owns PTYs and a session/window/pane tree, and speaks tmux control-mode
//! protocol over a local Unix socket so that control-mode clients — including
//! `par-term-tmux` — can attach to it in place of tmux.
//!
//! This module is the *emitter* side of the format that [`crate::tmux_control`]
//! parses. That parser is the format reference and the conformance oracle for
//! everything here; see `par-mux.md`.

pub mod emit;
pub mod ids;
pub mod pane;

pub use emit::{emit, emit_block, escape_output};
pub use ids::{IdAllocator, PaneId, ParseIdError, SessionId, WindowId};
pub use pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory};

/// Marker type used by the feature-isolation test to prove this module is
/// reachable exactly when the `mux` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxMarker;
