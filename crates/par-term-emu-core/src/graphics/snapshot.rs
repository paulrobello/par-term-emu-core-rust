//! Lock-light graphics capture (ARC-004 remainder).
//!
//! Reading a terminal's graphics for a screenshot or a graphics export while
//! holding a shared `Arc<RwLock<Terminal>>` (`PtySession`) stalls the PTY
//! reader's write lock for the whole render/export. A [`GraphicsStoreSnapshot`]
//! copies the placements the render/export reads — their RGBA buffers are
//! `Arc`-shared, so the copy is reference bumps rather than buffer clones —
//! and the caller then works with no access to the live store.

use super::GraphicsStore;
use crate::graphics::TerminalGraphic;

/// The graphics state a screenshot render or graphics export reads, detached
/// from the live [`GraphicsStore`]. Captured with
/// [`GraphicsStore::snapshot`].
#[derive(Debug, Clone, Default)]
pub struct GraphicsStoreSnapshot {
    /// Visible-area placements in store order. This order is the effective
    /// z-order: the screenshot renderer draws them back-to-front in exactly
    /// this sequence (the per-placement `ImagePlacement::z_index` is metadata
    /// the renderer does not sort by).
    placements: Vec<TerminalGraphic>,
    /// Graphics promoted to scrollback, oldest first.
    scrollback: Vec<TerminalGraphic>,
    /// The store's scrollback row counter at capture time (see
    /// [`GraphicsStore::scrollback_position`]).
    scrollback_position: usize,
}

impl GraphicsStoreSnapshot {
    /// Capture from a live store. Pixel buffers are `Arc`-shared with the
    /// store's graphics; later store mutations (new graphics, scrolling,
    /// eviction) do not change a captured snapshot.
    pub fn from_store(store: &GraphicsStore) -> Self {
        GraphicsStoreSnapshot {
            placements: store.all_graphics().to_vec(),
            scrollback: store.scrollback_entries().clone(),
            scrollback_position: store.scrollback_position(),
        }
    }

    /// Visible-area placements in render (z) order.
    pub fn placements(&self) -> &[TerminalGraphic] {
        &self.placements
    }

    /// Graphics promoted to scrollback, oldest first.
    pub fn scrollback(&self) -> &[TerminalGraphic] {
        &self.scrollback
    }

    /// The store's scrollback row counter at capture time.
    pub fn scrollback_position(&self) -> usize {
        self.scrollback_position
    }

    /// True when the store held no visible or scrollback graphics at capture
    /// time.
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty() && self.scrollback.is_empty()
    }
}

impl GraphicsStore {
    /// Copy the graphics state a screenshot render or graphics export reads.
    /// Cost is one `Vec` clone per placement list with the pixel buffers
    /// `Arc`-shared (see [`GraphicsStoreSnapshot::from_store`]).
    pub fn snapshot(&self) -> GraphicsStoreSnapshot {
        GraphicsStoreSnapshot::from_store(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::graphics::{next_graphic_id, GraphicProtocol};

    /// Two graphics in the visible area plus one in scrollback, promoted the
    /// way the terminal promotes them: scrolled off the top of the region.
    fn populated_store() -> GraphicsStore {
        let mut store = GraphicsStore::new();
        for position in [(0usize, 8usize), (5, 10)] {
            store.add_graphic(TerminalGraphic::new(
                next_graphic_id(),
                GraphicProtocol::Sixel,
                position,
                8,
                6,
                vec![0xAA; 8 * 6 * 4],
            ));
        }
        store.add_graphic(TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::Sixel,
            (0, 0),
            8,
            6,
            vec![0xBB; 8 * 6 * 4],
        ));
        store.adjust_for_scroll_up_with_scrollback(5, 0, 11, 100);
        store
    }

    #[test]
    fn snapshot_captures_placements_scrollback_and_position() {
        let mut store = populated_store();
        store.notify_scrollback_advance(7);
        let snap = store.snapshot();

        assert_eq!(snap.placements().len(), 2);
        assert_eq!(snap.placements()[0].position, (0, 3));
        assert_eq!(snap.placements()[1].position, (5, 5));
        assert_eq!(snap.scrollback().len(), 1);
        assert_eq!(snap.scrollback()[0].scrollback_row, Some(100));
        assert_eq!(snap.scrollback_position(), 7);
        assert!(!snap.is_empty());
    }

    #[test]
    fn snapshot_is_detached_from_later_store_mutations() {
        let mut store = populated_store();
        let snap = store.snapshot();
        let before = snap.placements().len();

        store.add_graphic(TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::Kitty,
            (1, 1),
            4,
            4,
            vec![0xCC; 4 * 4 * 4],
        ));
        store.remove_graphic(snap.placements()[0].id);

        assert_eq!(snap.placements().len(), before);
        assert_eq!(snap.placements()[0].position, (0, 3));
        assert_eq!(snap.scrollback().len(), 1);
    }

    #[test]
    fn empty_store_snapshots_to_empty() {
        let snap = GraphicsStore::new().snapshot();
        assert!(snap.is_empty());
        assert_eq!(snap.placements().len(), 0);
        assert_eq!(snap.scrollback().len(), 0);
        assert_eq!(snap.scrollback_position(), 0);
    }

    /// Pixel buffers are shared, not copied: the snapshot's graphic exposes
    /// the same buffer address as the store's live graphic.
    #[test]
    fn pixel_buffers_are_arc_shared_with_the_store() {
        let store = populated_store();
        let live_ptr = Arc::as_ptr(&store.all_graphics()[0].pixels);
        let snap = store.snapshot();
        assert_eq!(Arc::as_ptr(&snap.placements()[0].pixels), live_ptr);
    }
}
