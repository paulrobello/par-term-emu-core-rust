//! Core types for Rust embedders: `use par_term_emu_core_rust::prelude::*;`
//!
//! The prelude is the stable, curated tier of the public API (ARC-005). It
//! holds the types nearly every embedder touches: the terminal state machine,
//! its grid and cells, colors and cursor, the event/observer surface, the
//! graphics model, the tmux notification type, Unicode width configuration,
//! and (with `pty_session`) the PTY session.
//!
//! Everything else is peripheral and is reached through its module path:
//!
//! | Area | Module path |
//! |------|-------------|
//! | Terminal sub-features (triggers, recording, clipboard, search, progress, …) | [`crate::terminal`] |
//! | Graphics protocols (Sixel, iTerm2, Kitty) | [`crate::graphics`], [`crate::sixel`] |
//! | Badge formats | [`crate::badge`] |
//! | Coprocesses | [`crate::coprocess`] |
//! | Shell integration markers | [`crate::shell_integration`] |
//! | Streaming server and protocol (`streaming`) | `crate::streaming` |
//! | Multiplexer daemon and client (`mux`) | `crate::mux` |
//! | Screenshot renderer (`screenshot`) | `crate::screenshot` |
//! | PyO3 wrapper types (`python`) | `crate::python_bindings` |
//!
//! The crate-root re-exports that predate this tiering stay in place for
//! existing consumers; new code should import from the prelude or the module
//! paths above.

pub use crate::cell::{Cell, CellFlags};
pub use crate::color::Color;
pub use crate::cursor::{Cursor, CursorStyle};
pub use crate::graphics::TerminalGraphic;
pub use crate::grid::Grid;
pub use crate::mouse::{MouseEncoding, MouseMode};
pub use crate::observer::{ObserverId, TerminalObserver};
pub use crate::pty_error::PtyError;
pub use crate::terminal::{Terminal, TerminalEvent, TerminalEventKind};
pub use crate::tmux_control::TmuxNotification;
pub use crate::unicode_normalization_config::NormalizationForm;
pub use crate::unicode_width_config::{AmbiguousWidth, UnicodeVersion, WidthConfig};

#[cfg(feature = "pty_session")]
pub use crate::pty_session::PtySession;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelude_covers_the_embedder_core_loop() {
        let mut term = Terminal::new(10, 3);
        term.process(b"hi");
        let grid: &Grid = term.active_grid();
        let cell: &Cell = grid.get(0, 0).expect("cell in bounds");
        assert_eq!(cell.c, 'h');
        let _: CursorStyle = term.cursor().style;
        let _: Vec<TerminalEvent> = term.poll_events();
    }
}
