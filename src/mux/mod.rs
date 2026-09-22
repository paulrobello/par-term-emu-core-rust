//! Terminal multiplexer server.
//!
//! Owns PTYs and a session/window/pane tree, and speaks tmux control-mode
//! protocol over a local Unix socket so that control-mode clients — including
//! `par-term-tmux` — can attach to it in place of tmux.
//!
//! This module is the *emitter* side of the format that [`crate::tmux_control`]
//! parses. That parser is the format reference and the conformance oracle for
//! everything here; see `par-mux.md`.

pub mod client;
pub mod command;
pub mod emit;
pub mod hooks;
pub mod ids;
pub mod ipc;
pub mod layout;
pub mod pane;
pub mod persist;
pub mod server;
pub mod tree;

pub use client::MuxClient;
pub use command::{parse_command, MuxCommand};
pub use emit::{emit, emit_block, escape_output};
pub use ids::{IdAllocator, PaneId, ParseIdError, SessionId, WindowId};
pub use ipc::{
    bind_local_listener, connect_local_stream, default_socket_path, prepare_socket_path,
    LocalListener, LocalStream,
};
pub use layout::{LayoutTree, NoSuchLeaf, PaneGeometry, SplitDirection};
pub use pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory};
pub use persist::{PersistError, PersistState, FORMAT_VERSION};
pub use server::MuxServer;
pub use tree::{MuxSession, MuxTree, MuxWindow};

/// Marker type used by the feature-isolation test to prove this module is
/// reachable exactly when the `mux` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxMarker;
