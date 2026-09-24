//! Terminal multiplexer server.
//!
//! Owns PTYs and a session/window/pane tree, and speaks tmux control-mode
//! protocol over a local Unix socket so that control-mode clients — including
//! `par-term-tmux` — can attach to it in place of tmux.
//!
//! This module is the *emitter* side of the format that [`crate::tmux_control`]
//! parses. That parser is the format reference and the conformance oracle for
//! everything here; see `par-mux.md`.

pub mod agent_resume;
pub mod client;
pub mod command;
pub mod emit;
pub mod hooks;
pub mod ids;
pub mod ipc;
pub mod layout;
pub mod pane;
pub mod persist;
pub mod scrape;
pub mod server;
pub mod tree;

/// Per-command dispatch — private, reached only through `server` (the
/// socket loop) and its line-level test shims.
mod dispatch;

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
pub use scrape::{scrape_tick, ScrapeEngine};
pub use server::MuxServer;
pub use tree::{MuxSession, MuxTree, MuxWindow};

/// The build identity of THIS crate compilation: the crate version plus the
/// git sha it was built from (`0.50.0+a02b2b3`, `-dirty` appended when the
/// checkout had uncommitted tracked changes; `+unknown` when built outside a
/// repository, e.g. from a crates.io tarball).
///
/// Both sides of a daemon/client pair read this same function — the daemon
/// serves it as the `version` command's reply, clients compare their own
/// linked value against that reply — because the env var is baked into the
/// rlib at compile time: a client's stamp is the stamp of the core IT
/// linked, which is exactly what a daemon comparison needs to be against.
pub fn build_stamp() -> &'static str {
    concat!(
        env!("CARGO_PKG_VERSION"),
        "+",
        env!("PAR_TERM_CORE_BUILD_SHA")
    )
}

/// Marker type used by the feature-isolation test to prove this module is
/// reachable exactly when the `mux` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxMarker;
