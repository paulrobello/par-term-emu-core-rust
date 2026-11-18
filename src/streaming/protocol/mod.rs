//! Protocol definitions for terminal streaming
//!
//! This module defines the message formats used for WebSocket-based
//! terminal streaming between the server and web clients.
//!
//! The app-type layer is split by direction and topic; every public item is
//! re-exported here so `crate::streaming::protocol::X` paths are stable:
//!
//! - `server_messages`: [`ServerMessage`], its constructors, [`ConnectedBuilder`]
//! - `client_messages`: [`ClientMessage`], its constructors, [`MouseEventType`]
//! - `events`: [`EventType`] and the `TerminalEvent` -> `ServerMessage` mapping
//! - `common`: shared leaf types (theme, system stats, damage ranges, size caps)
//! - `mux_types`: par-mux agent roster and session-tree payloads

mod client_messages;
mod common;
mod events;
mod mux_types;
mod server_messages;

#[cfg(test)]
mod tests;

pub use client_messages::{ClientMessage, MouseEventType};
pub use common::{
    CpuStats, DamageRange, DiskStats, InlineImageFrame, LoadAverage, MemoryStats,
    NetworkInterfaceStats, ThemeInfo, MAX_INLINE_IMAGE_BYTES, MAX_UPLOAD_CHUNK_BYTES,
};
pub use events::{terminal_event_to_server_message, EventType};
pub use mux_types::{AgentEntry, MuxTreePane, MuxTreeSession, MuxTreeWindow, MuxTreeWorkspace};
pub use server_messages::{ConnectedBuilder, ServerMessage};

#[cfg(all(test, feature = "streaming"))]
pub(crate) use client_messages::CLIENT_MESSAGE_VARIANT_NAMES;
#[cfg(all(test, feature = "streaming"))]
pub(crate) use server_messages::SERVER_MESSAGE_VARIANT_NAMES;
