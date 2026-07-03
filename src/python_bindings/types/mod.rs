//! Python data types and structures for the terminal API.
//!
//! Formerly one ~4,000-line file; now split by domain into submodules.
//! Every type is re-exported here so `python_bindings::types::PyX` and the
//! crate-level re-exports remain unchanged.

pub mod clipboard;
pub mod color;
pub mod graphics;
pub mod metrics;
pub mod mouse;
pub mod notification;
pub mod recording;
pub mod screen;
pub mod selection;
pub mod session;
pub mod shell;
pub mod trigger;

pub use clipboard::*;
pub use color::*;
pub use graphics::*;
pub use metrics::*;
pub use mouse::*;
pub use notification::*;
pub use recording::*;
pub use screen::*;
pub use selection::*;
pub use session::*;
pub use shell::*;
pub use trigger::*;

/// Type alias for a row of cell data returned by get_line_cells
/// Tuple contains: (character, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes)
pub type LineCellData = Vec<(String, (u8, u8, u8), (u8, u8, u8), PyAttributes)>;
