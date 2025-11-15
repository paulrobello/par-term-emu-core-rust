//! Python bindings module
//!
//! This module contains all Python-facing bindings organized into submodules:
//! - `terminal`: PyTerminal struct and its implementation
//! - `pty`: PyPtyTerminal struct and its implementation (PTY support)
//! - `types`: Data types (PyAttributes, PyScreenSnapshot, PyShellIntegration, PyGraphic)
//! - `enums`: Enum types (PyCursorStyle, PyUnderlineStyle)
//! - `conversions`: Type conversions and parsing utilities

pub mod conversions;
pub mod enums;
pub mod pty;
pub mod terminal;
pub mod types;

// Re-export public types for convenience
pub use enums::{PyCursorStyle, PyUnderlineStyle};
pub use pty::PyPtyTerminal;
pub use terminal::PyTerminal;
pub use types::{PyAttributes, PyGraphic, PyScreenSnapshot, PyShellIntegration};
