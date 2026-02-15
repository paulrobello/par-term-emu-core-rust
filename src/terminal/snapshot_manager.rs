//! Snapshot manager for the Instant Replay feature.
//!
//! Manages a rolling buffer of [`TerminalSnapshot`] entries with associated
//! input bytes, providing size-based eviction and input-stream reconstruction.
//! Each entry captures a point-in-time snapshot plus the bytes that were fed
//! to `Terminal::process()` *after* the snapshot was taken.

use super::terminal_snapshot::TerminalSnapshot;
use super::Terminal;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default maximum memory budget for stored snapshots (4 MiB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024;

/// Default interval between automatic snapshots (30 seconds).
pub const DEFAULT_SNAPSHOT_INTERVAL_SECS: u64 = 30;

/// A single entry in the snapshot ring buffer.
///
/// Contains a terminal snapshot and the input bytes that were fed to
/// `Terminal::process()` after this snapshot was captured.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// The captured terminal state.
    pub snapshot: TerminalSnapshot,
    /// Bytes fed to `Terminal::process()` after this snapshot was taken.
    pub input_bytes: Vec<u8>,
}

impl SnapshotEntry {
    /// Estimate the total memory footprint of this entry in bytes.
    pub fn size_bytes(&self) -> usize {
        self.snapshot.estimated_size_bytes + self.input_bytes.len()
    }
}

/// Manages a rolling buffer of terminal snapshots with size-based eviction.
///
/// The manager captures periodic snapshots of terminal state and records
/// the input bytes processed between snapshots. This allows reconstructing
/// terminal state at any point by restoring a snapshot and replaying the
/// subsequent input bytes.
#[derive(Debug)]
pub struct SnapshotManager {
    /// Ring buffer of snapshot entries, oldest first.
    entries: VecDeque<SnapshotEntry>,
    /// Maximum memory budget for all stored entries.
    max_memory_bytes: usize,
    /// Current total memory usage of all stored entries.
    current_memory_bytes: usize,
    /// Minimum interval between automatic snapshots.
    snapshot_interval: Duration,
    /// When the last snapshot was taken (wall-clock).
    last_snapshot_time: Option<Instant>,
    /// Whether the manager is enabled.
    enabled: bool,
}

impl SnapshotManager {
    /// Create a new snapshot manager with the given memory budget and interval.
    pub fn new(max_memory_bytes: usize, snapshot_interval: Duration) -> Self {
        Self {
            entries: VecDeque::new(),
            max_memory_bytes,
            current_memory_bytes: 0,
            snapshot_interval,
            last_snapshot_time: None,
            enabled: true,
        }
    }

    /// Create a new snapshot manager with default settings.
    pub fn with_defaults() -> Self {
        Self::new(
            DEFAULT_MAX_MEMORY_BYTES,
            Duration::from_secs(DEFAULT_SNAPSHOT_INTERVAL_SECS),
        )
    }

    /// Check whether enough time has elapsed since the last snapshot.
    ///
    /// Returns `true` if the manager is enabled and the snapshot interval
    /// has elapsed (or no snapshot has been taken yet).
    pub fn should_snapshot(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.last_snapshot_time {
            None => true,
            Some(last) => last.elapsed() >= self.snapshot_interval,
        }
    }

    /// Capture a snapshot of the current terminal state and append it.
    ///
    /// Returns the index of the newly added entry. Evicts oldest entries
    /// if the memory budget is exceeded (always keeps at least one entry).
    pub fn take_snapshot(&mut self, terminal: &Terminal) -> usize {
        let snapshot = terminal.capture_snapshot();
        let entry = SnapshotEntry {
            snapshot,
            input_bytes: Vec::new(),
        };
        self.current_memory_bytes += entry.size_bytes();
        self.entries.push_back(entry);
        self.last_snapshot_time = Some(Instant::now());
        self.evict();
        self.entries.len() - 1
    }

    /// Record input bytes that were fed to `Terminal::process()`.
    ///
    /// The bytes are appended to the most recent entry's `input_bytes`.
    /// No-op if the manager is disabled or has no entries.
    pub fn record_input(&mut self, bytes: &[u8]) {
        if !self.enabled || self.entries.is_empty() {
            return;
        }
        if let Some(entry) = self.entries.back_mut() {
            self.current_memory_bytes += bytes.len();
            entry.input_bytes.extend_from_slice(bytes);
        }
        self.evict();
    }

    /// Return the number of stored entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Get a reference to the entry at the given index.
    pub fn get_entry(&self, index: usize) -> Option<&SnapshotEntry> {
        self.entries.get(index)
    }

    /// Return the current total memory usage of all entries.
    pub fn memory_usage(&self) -> usize {
        self.current_memory_bytes
    }

    /// Return the maximum memory budget.
    pub fn max_memory(&self) -> usize {
        self.max_memory_bytes
    }

    /// Set the maximum memory budget, triggering eviction if needed.
    pub fn set_max_memory(&mut self, max_bytes: usize) {
        self.max_memory_bytes = max_bytes;
        self.evict();
    }

    /// Return the current snapshot interval.
    pub fn snapshot_interval(&self) -> Duration {
        self.snapshot_interval
    }

    /// Set the snapshot interval.
    pub fn set_snapshot_interval(&mut self, interval: Duration) {
        self.snapshot_interval = interval;
    }

    /// Set whether the manager is enabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Return whether the manager is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Clear all entries and reset memory tracking.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_memory_bytes = 0;
        self.last_snapshot_time = None;
    }

    /// Return the time range of stored snapshots as `(oldest_ms, newest_ms)`.
    ///
    /// Returns `None` if there are no entries.
    pub fn time_range(&self) -> Option<(u64, u64)> {
        let oldest = self.entries.front()?.snapshot.timestamp;
        let newest = self.entries.back()?.snapshot.timestamp;
        Some((oldest, newest))
    }

    /// Push a pre-built entry into the buffer.
    ///
    /// Useful for cloning or rebuilding a manager from serialized data.
    pub fn push_entry(&mut self, entry: SnapshotEntry) {
        self.current_memory_bytes += entry.size_bytes();
        self.entries.push_back(entry);
        self.evict();
    }

    /// Reconstruct a terminal at the state represented by `entry_index`
    /// with input replayed up to `byte_offset`.
    ///
    /// Creates a fresh terminal, restores the snapshot, then replays
    /// `input_bytes[..byte_offset]`. The byte offset is clamped to the
    /// available input length.
    ///
    /// Returns `None` if the entry index is out of bounds.
    pub fn reconstruct_at(&self, entry_index: usize, byte_offset: usize) -> Option<Terminal> {
        let entry = self.entries.get(entry_index)?;
        let snap = &entry.snapshot;

        let mut terminal =
            Terminal::with_scrollback(snap.cols, snap.rows, snap.grid.max_scrollback);
        terminal.restore_from_snapshot(snap.clone());

        let clamped = byte_offset.min(entry.input_bytes.len());
        if clamped > 0 {
            terminal.process(&entry.input_bytes[..clamped]);
        }

        Some(terminal)
    }

    /// Find the entry index whose snapshot timestamp is closest to (but not
    /// after) the given Unix-millisecond timestamp.
    ///
    /// Uses binary search. Returns `Some(0)` if the timestamp predates all
    /// entries but entries exist. Returns `None` only when the manager is empty.
    pub fn find_entry_for_timestamp(&self, timestamp: u64) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        // Binary search: find the rightmost entry with ts <= timestamp.
        let mut lo: usize = 0;
        let mut hi: usize = self.entries.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.entries[mid].snapshot.timestamp <= timestamp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        // lo is now the first entry with ts > timestamp.
        // We want the entry just before that, or 0 if timestamp < all entries.
        if lo == 0 {
            Some(0)
        } else {
            Some(lo - 1)
        }
    }

    /// Evict oldest entries while over budget, keeping at least one entry.
    fn evict(&mut self) {
        while self.current_memory_bytes > self.max_memory_bytes && self.entries.len() > 1 {
            if let Some(removed) = self.entries.pop_front() {
                self.current_memory_bytes = self
                    .current_memory_bytes
                    .saturating_sub(removed.size_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a small terminal and return it.
    fn make_terminal() -> Terminal {
        Terminal::new(20, 5)
    }

    // ---- Task 5 Tests ----

    #[test]
    fn test_creation() {
        let mgr = SnapshotManager::new(1024, Duration::from_secs(10));
        assert_eq!(mgr.entry_count(), 0);
        assert_eq!(mgr.memory_usage(), 0);
        assert_eq!(mgr.max_memory(), 1024);
        assert!(mgr.is_enabled());
        assert_eq!(mgr.snapshot_interval(), Duration::from_secs(10));
    }

    #[test]
    fn test_with_defaults() {
        let mgr = SnapshotManager::with_defaults();
        assert_eq!(mgr.max_memory(), DEFAULT_MAX_MEMORY_BYTES);
        assert_eq!(
            mgr.snapshot_interval(),
            Duration::from_secs(DEFAULT_SNAPSHOT_INTERVAL_SECS)
        );
    }

    #[test]
    fn test_take_snapshot() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(1));
        let idx = mgr.take_snapshot(&term);
        assert_eq!(idx, 0);
        assert_eq!(mgr.entry_count(), 1);
        assert!(mgr.memory_usage() > 0);

        let entry = mgr.get_entry(0).unwrap();
        assert!(entry.input_bytes.is_empty());
        assert_eq!(entry.snapshot.cols, 20);
        assert_eq!(entry.snapshot.rows, 5);
    }

    #[test]
    fn test_record_input() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(1));
        mgr.take_snapshot(&term);

        let data = b"hello world";
        mgr.record_input(data);

        let entry = mgr.get_entry(0).unwrap();
        assert_eq!(entry.input_bytes, b"hello world");
        // Memory should include the input bytes
        assert!(mgr.memory_usage() >= data.len());
    }

    #[test]
    fn test_record_input_no_entry() {
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(1));
        // Should be a no-op, not panic
        mgr.record_input(b"orphan bytes");
        assert_eq!(mgr.entry_count(), 0);
    }

    #[test]
    fn test_multiple_snapshots() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        let i0 = mgr.take_snapshot(&term);
        mgr.record_input(b"first");
        let i1 = mgr.take_snapshot(&term);
        mgr.record_input(b"second");
        let i2 = mgr.take_snapshot(&term);

        assert_eq!(i0, 0);
        assert_eq!(i1, 1);
        assert_eq!(i2, 2);
        assert_eq!(mgr.entry_count(), 3);
        assert_eq!(mgr.get_entry(0).unwrap().input_bytes, b"first");
        assert_eq!(mgr.get_entry(1).unwrap().input_bytes, b"second");
        assert!(mgr.get_entry(2).unwrap().input_bytes.is_empty());
    }

    #[test]
    fn test_eviction_keeps_at_least_one() {
        let term = make_terminal();
        // Very small budget — smaller than a single snapshot
        let mut mgr = SnapshotManager::new(1, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        // Should keep at least 1 entry even though over budget
        assert_eq!(mgr.entry_count(), 1);
    }

    #[test]
    fn test_eviction_reduces_memory() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));

        // Take some snapshots
        mgr.take_snapshot(&term);
        mgr.record_input(&vec![0u8; 1024]);
        mgr.take_snapshot(&term);
        mgr.record_input(&vec![0u8; 1024]);
        mgr.take_snapshot(&term);

        let mem_before = mgr.memory_usage();
        let count_before = mgr.entry_count();
        assert_eq!(count_before, 3);

        // Now reduce budget to force eviction
        let single_snap_size = mgr.get_entry(0).unwrap().size_bytes();
        mgr.set_max_memory(single_snap_size + 1);

        assert!(mgr.entry_count() < count_before);
        assert!(mgr.memory_usage() < mem_before);
    }

    #[test]
    fn test_should_snapshot_initially_true() {
        let mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(30));
        assert!(mgr.should_snapshot());
    }

    #[test]
    fn test_should_snapshot_false_after_recent() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(60));
        mgr.take_snapshot(&term);
        // Just took a snapshot, interval is 60s, so should be false
        assert!(!mgr.should_snapshot());
    }

    #[test]
    fn test_disabled_manager() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.set_enabled(false);
        assert!(!mgr.is_enabled());
        assert!(!mgr.should_snapshot());

        // record_input should be a no-op when disabled
        mgr.take_snapshot(&term); // take_snapshot doesn't check enabled
        mgr.set_enabled(false);
        mgr.record_input(b"ignored");
        assert!(mgr.get_entry(0).unwrap().input_bytes.is_empty());
    }

    #[test]
    fn test_clear() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        mgr.record_input(b"data");
        assert!(mgr.entry_count() > 0);
        assert!(mgr.memory_usage() > 0);

        mgr.clear();
        assert_eq!(mgr.entry_count(), 0);
        assert_eq!(mgr.memory_usage(), 0);
        // After clearing, should_snapshot should be true again
        assert!(mgr.should_snapshot());
    }

    #[test]
    fn test_time_range() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));

        assert!(mgr.time_range().is_none());

        mgr.take_snapshot(&term);
        let (oldest, newest) = mgr.time_range().unwrap();
        assert_eq!(oldest, newest); // Only one entry

        mgr.take_snapshot(&term);
        let (oldest2, newest2) = mgr.time_range().unwrap();
        assert!(newest2 >= oldest2);
    }

    #[test]
    fn test_set_max_memory_triggers_eviction() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);
        assert_eq!(mgr.entry_count(), 3);

        // Shrink to force eviction (keep at least 1)
        mgr.set_max_memory(1);
        assert_eq!(mgr.entry_count(), 1);
    }

    #[test]
    fn test_push_entry() {
        let term = make_terminal();
        let snap = term.capture_snapshot();
        let entry = SnapshotEntry {
            snapshot: snap,
            input_bytes: vec![1, 2, 3],
        };
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.push_entry(entry);
        assert_eq!(mgr.entry_count(), 1);
        assert_eq!(mgr.get_entry(0).unwrap().input_bytes, vec![1, 2, 3]);
    }

    #[test]
    fn test_snapshot_entry_size_bytes() {
        let term = make_terminal();
        let snap = term.capture_snapshot();
        let entry = SnapshotEntry {
            snapshot: snap.clone(),
            input_bytes: vec![0u8; 100],
        };
        assert_eq!(entry.size_bytes(), snap.estimated_size_bytes + 100);
    }

    // ---- Task 6 Tests ----

    #[test]
    fn test_reconstruct_at_entry_start() {
        // Reconstruct at byte_offset=0 means just restore the snapshot, no replay
        let mut term = make_terminal();
        term.process(b"Hello");

        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        mgr.record_input(b"World");

        let reconstructed = mgr.reconstruct_at(0, 0).unwrap();
        // Should match the snapshot state (before "World" was processed)
        let grid = reconstructed.grid();
        let cell = grid.get(0, 0).unwrap();
        assert_eq!(cell.c, 'H');
    }

    #[test]
    fn test_reconstruct_with_full_input_replay() {
        let mut term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);

        let input = b"ABCDE";
        term.process(input);
        mgr.record_input(input);

        // Replay all input bytes
        let reconstructed = mgr.reconstruct_at(0, input.len()).unwrap();
        let grid = reconstructed.grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'A');
        assert_eq!(grid.get(1, 0).unwrap().c, 'B');
        assert_eq!(grid.get(4, 0).unwrap().c, 'E');
    }

    #[test]
    fn test_reconstruct_with_partial_replay() {
        let mut term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);

        let input = b"ABCDE";
        term.process(input);
        mgr.record_input(input);

        // Replay only first 3 bytes => should see "ABC" but not "DE"
        let reconstructed = mgr.reconstruct_at(0, 3).unwrap();
        let grid = reconstructed.grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'A');
        assert_eq!(grid.get(1, 0).unwrap().c, 'B');
        assert_eq!(grid.get(2, 0).unwrap().c, 'C');
        // Column 3 should still be the default (space)
        assert_eq!(grid.get(3, 0).unwrap().c, ' ');
    }

    #[test]
    fn test_reconstruct_invalid_index() {
        let mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        assert!(mgr.reconstruct_at(0, 0).is_none());
        assert!(mgr.reconstruct_at(99, 0).is_none());
    }

    #[test]
    fn test_reconstruct_clamps_byte_offset() {
        let mut term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);

        let input = b"XY";
        term.process(input);
        mgr.record_input(input);

        // Request offset way beyond available bytes — should clamp
        let reconstructed = mgr.reconstruct_at(0, 9999).unwrap();
        let grid = reconstructed.grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'X');
        assert_eq!(grid.get(1, 0).unwrap().c, 'Y');
    }

    #[test]
    fn test_find_entry_for_timestamp_exact_match() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);

        let ts = mgr.get_entry(0).unwrap().snapshot.timestamp;
        assert_eq!(mgr.find_entry_for_timestamp(ts), Some(0));
    }

    #[test]
    fn test_find_entry_for_timestamp_between() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        // Manually push an entry with a known later timestamp
        let mut snap2 = term.capture_snapshot();
        snap2.timestamp = mgr.get_entry(0).unwrap().snapshot.timestamp + 1000;
        snap2.estimated_size_bytes = snap2.estimate_size();
        mgr.push_entry(SnapshotEntry {
            snapshot: snap2,
            input_bytes: Vec::new(),
        });

        let ts0 = mgr.get_entry(0).unwrap().snapshot.timestamp;
        let ts1 = mgr.get_entry(1).unwrap().snapshot.timestamp;
        let between = ts0 + 500;
        assert!(between < ts1);

        // Should return index 0 (last entry with ts <= between)
        assert_eq!(mgr.find_entry_for_timestamp(between), Some(0));
    }

    #[test]
    fn test_find_entry_for_timestamp_before_all() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);

        // Timestamp before any entry
        assert_eq!(mgr.find_entry_for_timestamp(0), Some(0));
    }

    #[test]
    fn test_find_entry_for_timestamp_empty() {
        let mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        assert_eq!(mgr.find_entry_for_timestamp(12345), None);
    }

    #[test]
    fn test_find_entry_for_timestamp_after_all() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        mgr.take_snapshot(&term);

        // Timestamp far in the future
        assert_eq!(
            mgr.find_entry_for_timestamp(u64::MAX),
            Some(mgr.entry_count() - 1)
        );
    }
}
