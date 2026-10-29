//! On-disk persistence envelope for the mux tree (par-mux.md Phase 3, D3.2).
//!
//! The artifact is a versioned envelope, not a bare tree: `format_version`
//! lets a future format change migrate old files rather than refuse them,
//! and each window's pane list rides beside its
//! [`LayoutTree`](crate::mux::layout::LayoutTree) so restore
//! can spawn every pane's replacement process before re-hanging content
//! (D3.5: spawn first, restore second — startup bytes must not overwrite a
//! restored screen).
//!
//! Layout: `format` holds the serde schema (its field order is the
//! on-disk format), `save` the capture and atomic write, `restore` the
//! load tiers and the rebuild, `state_file` the state path and daemon
//! lock, and `layouts` the named-layout store. Every public item is
//! re-exported here at its historical `crate::mux::persist::` path.

mod format;
mod layouts;
mod restore;
mod save;
mod state_file;

pub use format::{
    PersistAgentSession, PersistError, PersistPane, PersistSession, PersistState, PersistWindow,
    PersistWorkspace, PersistWorktreeLink, FORMAT_VERSION,
};
pub use layouts::{
    kill_layout_in, kill_layout_in_dir, layout_file_in, layout_file_in_dir, layouts_dir,
    layouts_dir_in, list_layouts_in, list_layouts_in_dir, load_layout_in, load_layout_in_dir,
    save_layout_in, save_layout_state_in, save_layout_state_in_dir, valid_layout_name,
};
pub use restore::{load_or_quarantine, peek_state, Loaded, StatePeek};
#[allow(deprecated)]
pub use save::save_to_with_origin;
pub use save::{save_off_lock, save_to, write_job, write_state, PersistCapture, SaveOrigin};
pub use state_file::{
    acquire_state_lock, platform_state_dir, state_file_in, state_file_path, StateLock,
};

#[cfg(test)]
mod tests;

/// The envelope must survive serialize → deserialize whole — LayoutTree and
/// the id newtypes serialize inside it (D3.2), which no other test exercises.
#[cfg(all(test, feature = "serde"))]
mod serde_tests;
