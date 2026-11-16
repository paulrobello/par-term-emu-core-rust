//! Unified untrusted-output memory budget (ENH-062).
//!
//! One aggregate cap on everything untrusted PTY output can make the
//! terminal retain: the graphics store (Sixel/iTerm2/Kitty pixel buffers),
//! the iTerm2 multipart transfer buffer, and the file-transfer manager.
//! Each store keeps its own SEC-001 caps; this budget is the aggregate
//! gate on top of them, and the single `OutputBudgetExceeded` event
//! vocabulary for "the budget said no".

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Default aggregate budget for everything retained from untrusted output:
/// decoded graphics pixels, the in-flight iTerm2 multipart buffer, and
/// file-transfer payloads (active plus completed-ring). 320 MiB covers the
/// 256 MiB graphics-store cap with shared headroom for transfers.
/// cap: Aggregate bytes of graphics pixels, iTerm2 multipart buffers, and file-transfer payloads retained from untrusted output.
pub const DEFAULT_OUTPUT_MEMORY_BUDGET: usize = 320 * 1024 * 1024;

/// Which untrusted-output store a budget decision applies to. The `store`
/// field of [`crate::terminal::TerminalEvent::OutputBudgetExceeded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStore {
    /// Decoded graphics pixels (Sixel/iTerm2/Kitty) in the graphics store.
    Graphics,
    /// The in-flight iTerm2 multipart transfer buffer.
    Multipart,
    /// File-transfer payloads (active transfers plus the completed ring).
    FileTransfer,
}

impl BudgetStore {
    /// Canonical string for event dicts and filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            BudgetStore::Graphics => "graphics",
            BudgetStore::Multipart => "multipart",
            BudgetStore::FileTransfer => "file_transfer",
        }
    }
}

/// Aggregate memory budget across the untrusted-output stores (ENH-062).
///
/// Byte counts are kept per store (`AtomicUsize` slices) so each store can
/// reconcile its slice without knowing about the others; [`used`](Self::used)
/// sums them, and every reservation is admitted against the aggregate
/// `total`, not the slice. All methods are lock-free and `&self`.
#[derive(Debug)]
pub struct OutputMemoryBudget {
    total: usize,
    graphics: AtomicUsize,
    multipart: AtomicUsize,
    transfers: AtomicUsize,
}

impl OutputMemoryBudget {
    /// Create a budget with the given aggregate total in bytes.
    pub fn new(total: usize) -> Self {
        Self {
            total,
            graphics: AtomicUsize::new(0),
            multipart: AtomicUsize::new(0),
            transfers: AtomicUsize::new(0),
        }
    }

    /// The aggregate total in bytes.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Bytes currently accounted to the graphics store.
    pub fn graphics_bytes(&self) -> usize {
        self.graphics.load(Ordering::Relaxed)
    }

    /// Bytes currently accounted to the multipart buffer.
    pub fn multipart_bytes(&self) -> usize {
        self.multipart.load(Ordering::Relaxed)
    }

    /// Bytes currently accounted to file transfers.
    pub fn transfer_bytes(&self) -> usize {
        self.transfers.load(Ordering::Relaxed)
    }

    /// Aggregate bytes currently accounted across all stores.
    pub fn used(&self) -> usize {
        self.graphics.load(Ordering::Relaxed)
            + self.multipart.load(Ordering::Relaxed)
            + self.transfers.load(Ordering::Relaxed)
    }

    /// Whether `bytes` more fit under the aggregate total. Admission is
    /// always against the aggregate, never the calling store's slice.
    pub fn admits(&self, bytes: usize) -> bool {
        self.used().saturating_add(bytes) <= self.total
    }

    /// Account `bytes` to `store`'s slice, refusing (and accounting
    /// nothing) when the aggregate would exceed the total. Returns `false`
    /// when the reservation was refused.
    pub fn try_reserve(&self, store: BudgetStore, bytes: usize) -> bool {
        let slice = self.slice(store);
        loop {
            let current = slice.load(Ordering::Relaxed);
            let Some(next) = current.checked_add(bytes) else {
                return false;
            };
            if !self.admits_for(store, next) {
                return false;
            }
            if slice
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Return `bytes` previously reserved by `store` to the budget
    /// (saturating at zero; releasing more than reserved clamps the slice).
    pub fn release(&self, store: BudgetStore, bytes: usize) {
        let slice = self.slice(store);
        loop {
            let current = slice.load(Ordering::Relaxed);
            let next = current.saturating_sub(bytes);
            if slice
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// [`try_reserve`](Self::try_reserve) returning an RAII guard that
    /// releases the reservation on drop. Reservations are admitted against
    /// the aggregate total.
    pub fn reserve_guard(
        self: &Arc<Self>,
        store: BudgetStore,
        bytes: usize,
    ) -> Option<BudgetGuard> {
        if self.try_reserve(store, bytes) {
            Some(BudgetGuard {
                budget: Arc::clone(self),
                store,
                bytes,
            })
        } else {
            None
        }
    }

    fn slice(&self, store: BudgetStore) -> &AtomicUsize {
        match store {
            BudgetStore::Graphics => &self.graphics,
            BudgetStore::Multipart => &self.multipart,
            BudgetStore::FileTransfer => &self.transfers,
        }
    }

    /// Bytes the two stores other than `store` currently hold.
    fn others_bytes(&self, store: BudgetStore) -> usize {
        match store {
            BudgetStore::Graphics => {
                self.multipart.load(Ordering::Relaxed) + self.transfers.load(Ordering::Relaxed)
            }
            BudgetStore::Multipart => {
                self.graphics.load(Ordering::Relaxed) + self.transfers.load(Ordering::Relaxed)
            }
            BudgetStore::FileTransfer => {
                self.graphics.load(Ordering::Relaxed) + self.multipart.load(Ordering::Relaxed)
            }
        }
    }

    /// Whether `next` bytes on `store`'s slice keep the aggregate under the
    /// total (the other two slices contribute what they currently hold).
    fn admits_for(&self, store: BudgetStore, next: usize) -> bool {
        self.others_bytes(store).saturating_add(next) <= self.total
    }
}

/// RAII reservation against [`OutputMemoryBudget`]: releasing `bytes` from
/// its store's slice when dropped.
#[derive(Debug)]
pub struct BudgetGuard {
    budget: Arc<OutputMemoryBudget>,
    store: BudgetStore,
    bytes: usize,
}

impl BudgetGuard {
    /// Bytes held by this guard.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Which store's slice this guard reserves against.
    pub fn store(&self) -> BudgetStore {
        self.store
    }

    /// Release the reservation early (the guard then releases nothing on
    /// drop).
    pub fn release_now(mut self) {
        if self.bytes > 0 {
            self.budget.release(self.store, self.bytes);
            self.bytes = 0;
        }
    }
}

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.budget.release(self.store, self.bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_reserve_and_release_track_slices() {
        let budget = OutputMemoryBudget::new(1000);
        assert!(budget.try_reserve(BudgetStore::Graphics, 400));
        assert!(budget.try_reserve(BudgetStore::FileTransfer, 300));
        assert_eq!(budget.graphics_bytes(), 400);
        assert_eq!(budget.transfer_bytes(), 300);
        assert_eq!(budget.used(), 700);

        budget.release(BudgetStore::Graphics, 400);
        assert_eq!(budget.graphics_bytes(), 0);
        assert_eq!(budget.used(), 300);
    }

    #[test]
    fn admission_is_against_the_aggregate_total() {
        let budget = OutputMemoryBudget::new(1000);
        assert!(budget.try_reserve(BudgetStore::Graphics, 600));
        // 600 + 500 > 1000 even though the transfers slice is empty.
        assert!(!budget.try_reserve(BudgetStore::FileTransfer, 500));
        assert!(budget.try_reserve(BudgetStore::FileTransfer, 400));
        assert_eq!(budget.used(), 1000);
        // Nothing more fits from any store.
        assert!(!budget.admits(1));
        assert!(!budget.try_reserve(BudgetStore::Graphics, 1));
    }

    #[test]
    fn release_saturates_at_zero() {
        let budget = OutputMemoryBudget::new(1000);
        budget.release(BudgetStore::Multipart, 50);
        assert_eq!(budget.multipart_bytes(), 0);
        assert!(budget.try_reserve(BudgetStore::Multipart, 100));
        budget.release(BudgetStore::Multipart, 500);
        assert_eq!(budget.multipart_bytes(), 0);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn guard_releases_on_drop() {
        let budget = Arc::new(OutputMemoryBudget::new(1000));
        {
            let guard = budget
                .reserve_guard(BudgetStore::Graphics, 700)
                .expect("700 of 1000 fits");
            assert_eq!(guard.bytes(), 700);
            assert_eq!(guard.store(), BudgetStore::Graphics);
            assert_eq!(budget.used(), 700);
            // The aggregate is now nearly full.
            assert!(!budget
                .reserve_guard(BudgetStore::FileTransfer, 400)
                .is_some());
        }
        assert_eq!(budget.used(), 0, "guard drop must release");

        // Refused reservations return None and account nothing.
        let guard = budget.reserve_guard(BudgetStore::Multipart, 2000);
        assert!(guard.is_none());
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn guard_release_now_releases_exactly_once() {
        let budget = Arc::new(OutputMemoryBudget::new(100));
        let guard = budget
            .reserve_guard(BudgetStore::FileTransfer, 100)
            .unwrap();
        guard.release_now();
        assert_eq!(budget.used(), 0);
        // release_now consumed the guard, so no drop-time double release.
    }

    // --- Store integration (ENH-062) ---

    use crate::graphics::{GraphicProtocol, GraphicsStore, TerminalGraphic};

    fn small_kitty_graphic(store_id: u64, image_id: u32, bytes: usize) -> TerminalGraphic {
        let mut g = TerminalGraphic::new(
            store_id,
            GraphicProtocol::Kitty,
            (0, 0),
            4,
            4,
            vec![0u8; bytes],
        );
        g.kitty_image_id = Some(image_id);
        g.kitty_placement_id = Some(1);
        g
    }

    #[test]
    fn graphics_store_admits_against_the_aggregate() {
        let budget = Arc::new(OutputMemoryBudget::new(1000));
        let mut store = GraphicsStore::default();
        store.set_shared_budget(budget.clone());

        // Ten 300-byte kitty images over a 1000-byte aggregate: oldest
        // kitty IDs evict until the total fits.
        for i in 0..10u32 {
            store.store_kitty_image(i, 4, 4, vec![0u8; 300]);
        }
        assert!(
            store.retained_bytes() <= 1000,
            "retained {} must stay at or under the aggregate",
            store.retained_bytes()
        );
        assert_eq!(budget.graphics_bytes(), store.retained_bytes());
        assert!(store.dropped_count() > 0, "evictions must be counted");

        // Removal releases the slice.
        let before = budget.graphics_bytes();
        let first_remaining = (0..10u32)
            .find(|i| store.get_kitty_image(*i).is_some())
            .expect("some image survives");
        store.remove_kitty_image(first_remaining);
        assert!(
            budget.graphics_bytes() < before,
            "release must shrink the slice"
        );
    }

    #[test]
    fn graphics_store_placement_growth_counts_once_per_buffer() {
        let budget = Arc::new(OutputMemoryBudget::new(100_000));
        let mut store = GraphicsStore::default();
        store.set_shared_budget(budget.clone());

        // A shared kitty buffer placed twice retains its bytes once.
        let pixels = Arc::new(vec![0u8; 500]);
        store.store_kitty_image_shared(7, 4, 4, pixels.clone());
        store.add_graphic(small_kitty_graphic(1, 7, 500));
        store.add_graphic(small_kitty_graphic(2, 7, 500));
        // The placement's buffer is a distinct Arc here, so it counts
        // separately from the shared image — dedup applies per Arc.
        assert_eq!(budget.graphics_bytes(), store.retained_bytes());
        assert!(store.retained_bytes() >= 500);
    }

    #[test]
    fn file_transfer_growth_refused_under_aggregate_pressure() {
        use crate::terminal::file_transfer::TransferStatus;
        use std::collections::HashMap;

        let budget = Arc::new(OutputMemoryBudget::new(1000));
        budget.try_reserve(BudgetStore::Graphics, 900);
        let mut mgr = crate::terminal::file_transfer::FileTransferManager::new();
        mgr.set_shared_budget(budget.clone());

        let id = mgr.start_download("f.bin".into(), None, HashMap::new());
        // 900 (graphics) + 200 (chunk) > 1000 total: refused, transfer failed.
        assert!(mgr.append_data(id, &[0u8; 200]).is_err());
        assert!(mgr.get_transfer(id).is_none());
        let completed = &mgr.completed_transfers()[0];
        assert!(matches!(
            &completed.status,
            TransferStatus::Failed(msg) if msg.contains("aggregate")
        ));

        // Releasing the graphics pressure lets a retry through.
        budget.release(BudgetStore::Graphics, 900);
        let id2 = mgr.start_download("g.bin".into(), None, HashMap::new());
        assert!(mgr.append_data(id2, &[0u8; 200]).is_ok());
    }
}
