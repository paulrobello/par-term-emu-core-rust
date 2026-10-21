//! Terminal multiplexer server.
//!
//! Owns PTYs and a session/window/pane tree, and speaks tmux control-mode
//! protocol over a local Unix socket so that control-mode clients — including
//! `par-term-tmux` — can attach to it in place of tmux.
//!
//! This module is the *emitter* side of the format that [`par_term_emu_core::tmux_control`]
//! parses. That parser is the format reference and the conformance oracle for
//! everything here; see `par-mux.md`.

pub mod agent_resume;
#[cfg(feature = "attach")]
pub mod attach;
pub mod client;
pub mod command;
pub mod config;
pub mod discovery;
pub mod emit;
pub(crate) mod foreground;
pub mod hooks;
pub(crate) mod host_probe;
pub mod ids;
// The installers back the binary's one-shot flags only (ARC-143, the
// ARC-106 binary-only rule).
#[cfg(feature = "mux-bin")]
pub mod integrations;
pub mod ipc;
pub mod layout;
#[cfg(feature = "mux-bin")]
pub mod machine_config;
pub mod pane;
pub mod pane_stream;
pub mod persist;
/// Saved-machine profiles and the ssh relay command (mux-bin: the CLI's
/// machine subcommands and --machine relay read it; toml_edit rides mux-bin).
pub mod plugins;
pub mod popup;
pub mod scrape;
pub mod server;
#[cfg(all(target_os = "linux", feature = "shutdown-inhibit"))]
pub mod shutdown_inhibit;
#[cfg(feature = "mux-bin")]
pub mod skill_installer;
#[cfg(test)]
pub(crate) mod test_wait;
pub mod theme;
pub mod tree;
pub mod win_resume;
pub(crate) mod worktree;

/// Per-command dispatch — private, reached only through `server` (the
/// socket loop) and its line-level test shims.
mod dispatch;

pub use client::{MuxClient, Reply};
pub use command::{parse_command, MuxCommand};
pub use emit::{emit, emit_block, escape_output};
pub use ids::{
    AnyTarget, IdAllocator, PaneId, ParseIdError, SessionId, Target, WindowId, WorkspaceId,
};
pub use ipc::{
    accept_connection, bind_local_listener, connect_local_stream, default_socket_path,
    prepare_socket_path, resolve_socket_path, LocalListener, LocalStream,
};
pub use layout::{LayoutTree, NoSuchLeaf, PaneGeometry, SplitDirection};
pub use pane::{MuxError, MuxPane, PaneFactory, ShellPaneFactory};
pub use persist::{PersistError, PersistState, FORMAT_VERSION};
pub use scrape::{scrape_tick, ScrapeEngine};
pub use server::MuxServer;
pub use tree::{MuxSession, MuxTree, MuxWindow, MuxWorkspace, WorktreeLink};

/// `s` without its control characters (C0, DEL, C1 — `char::is_control`).
/// User-supplied names and titles are echoed into the status row and
/// `par-mux -c` stdout verbatim, so an embedded ESC or BEL would reach the
/// viewer's terminal as a live sequence (SEC-209). Borrowed when `s` is
/// already clean.
pub(crate) fn strip_controls(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().any(char::is_control) {
        std::borrow::Cow::Owned(s.chars().filter(|c| !c.is_control()).collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// Invisible format characters that reorder or hide text: the bidi
/// embeddings/overrides (U+202A–U+202E), isolates (U+2066–U+2069),
/// marks (U+200E, U+200F, U+061C), U+200B ZERO WIDTH SPACE and U+FEFF
/// BOM. A pane can put them in a title or agent label so chrome reads
/// as another pane (SEC-212). ZWJ/ZWNJ (U+200D/U+200C) are NOT here:
/// emoji sequences and Persian/Indic text need them.
pub(crate) fn is_spoofing_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
            | '\u{200B}'
            | '\u{FEFF}'
    )
}

/// The build identity of THIS crate compilation: the crate version plus the
/// git sha it was built from (`0.50.0+a02b2b3`, `-dirty` appended when the
/// checkout had uncommitted tracked changes). Outside a repository (a
/// crates.io tarball) the sha is a content digest of the crate source
/// (`0.52.0+src-0123456789abcdef`), so same-version drift stays detectable
/// in release builds; only a build with neither identity available stamps
/// `+unknown`.
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

/// Why starting a daemon here would nest it under another: this process runs
/// inside a par-mux pane (`PAR_MUX_ENV=1` in the pane env contract) and
/// nesting is not explicitly allowed. `None` when starting is fine.
///
/// The daemon's serve path and [`MuxClient`]'s auto-spawn both consult this
/// so their refusals cannot drift. One-shot client mode (`--cmd`), `--stop`
/// and `--restart` never do — a pane must keep reaching and restarting its
/// own daemon, as tmux allows `kill-server` from inside.
pub fn nested_daemon_refusal() -> Option<&'static str> {
    std::env::var_os("PAR_MUX_ENV")?;
    if std::env::var("PAR_MUX_ALLOW_NESTED").ok().as_deref() == Some("1") {
        return None;
    }
    Some(
        "refusing to start a nested daemon: PAR_MUX_ENV is set, so this process runs \
         inside a par-mux pane — set PAR_MUX_ALLOW_NESTED=1 to override",
    )
}

/// Marker type used by the feature-isolation test to prove this module is
/// reachable exactly when the `mux` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuxMarker;
