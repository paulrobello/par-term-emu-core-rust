//! Replay session for the Instant Replay feature.
//!
//! Provides [`ReplaySession`] which allows navigating through captured
//! terminal history, reconstructing terminal frames at arbitrary positions
//! within the snapshot timeline.

use super::snapshot_manager::SnapshotManager;
use super::Terminal;

/// Result of a seek or navigation operation on a [`ReplaySession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekResult {
    /// Successfully navigated to the requested position.
    Ok,
    /// Reached the beginning of the timeline (cannot go further back).
    AtStart,
    /// Reached the end of the timeline (cannot go further forward).
    AtEnd,
    /// No entries are available in the snapshot manager.
    Empty,
}

/// A replay session that allows navigating through captured terminal history.
///
/// The session maintains a current position (entry index + byte offset) and
/// a reconstructed terminal frame at that position. Navigation methods move
/// through the timeline and update the frame accordingly.
#[derive(Debug)]
pub struct ReplaySession {
    /// Current entry index in the snapshot manager.
    current_index: usize,
    /// Byte offset within the current entry's input_bytes.
    current_byte_offset: usize,
    /// Total number of entries at the time the session was created.
    total_entries: usize,
    /// Reconstructed terminal at the current position.
    current_frame: Terminal,
    /// Metadata for each entry: (timestamp, input_bytes_len).
    entry_metadata: Vec<(u64, usize)>,
    /// Cloned snapshot manager used for reconstruction.
    manager: SnapshotManager,
}

/// Clone a [`SnapshotManager`] by iterating its entries and pushing them
/// into a fresh manager with the same configuration.
fn clone_manager(mgr: &SnapshotManager) -> SnapshotManager {
    let mut new_mgr = SnapshotManager::new(mgr.max_memory(), mgr.snapshot_interval());
    for i in 0..mgr.entry_count() {
        if let Some(entry) = mgr.get_entry(i) {
            new_mgr.push_entry(entry.clone());
        }
    }
    new_mgr
}

impl ReplaySession {
    /// Create a new replay session starting at the end of the timeline
    /// (the latest captured state).
    ///
    /// Returns `None` if the snapshot manager has no entries.
    pub fn new(manager: &SnapshotManager) -> Option<Self> {
        if manager.entry_count() == 0 {
            return None;
        }

        let total_entries = manager.entry_count();
        let cloned = clone_manager(manager);

        // Build metadata for each entry.
        let mut entry_metadata = Vec::with_capacity(total_entries);
        for i in 0..total_entries {
            if let Some(entry) = cloned.get_entry(i) {
                entry_metadata.push((entry.snapshot.timestamp, entry.input_bytes.len()));
            }
        }

        // Start at the end: last entry, last byte offset.
        let last_index = total_entries - 1;
        let last_byte_offset = entry_metadata[last_index].1;

        let current_frame = cloned.reconstruct_at(last_index, last_byte_offset)?;

        Some(Self {
            current_index: last_index,
            current_byte_offset: last_byte_offset,
            total_entries,
            current_frame,
            entry_metadata,
            manager: cloned,
        })
    }

    /// Return a reference to the reconstructed terminal at the current position.
    pub fn current_frame(&self) -> &Terminal {
        &self.current_frame
    }

    /// Return the current entry index.
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Return the current byte offset within the current entry.
    pub fn current_byte_offset(&self) -> usize {
        self.current_byte_offset
    }

    /// Return the total number of entries in this replay session.
    pub fn total_entries(&self) -> usize {
        self.total_entries
    }

    /// Return the timestamp of the current entry's snapshot.
    pub fn current_timestamp(&self) -> u64 {
        self.entry_metadata[self.current_index].0
    }

    /// Seek to a specific entry index and byte offset.
    ///
    /// The entry index and byte offset are clamped to valid ranges.
    /// Reconstructs the terminal frame at the new position.
    pub fn seek_to(&mut self, entry_index: usize, byte_offset: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        // Clamp entry index.
        let clamped_index = entry_index.min(self.total_entries - 1);

        // Clamp byte offset to the available input bytes for this entry.
        let max_bytes = self.entry_metadata[clamped_index].1;
        let clamped_offset = byte_offset.min(max_bytes);

        // Reconstruct frame at the new position.
        if let Some(frame) = self.manager.reconstruct_at(clamped_index, clamped_offset) {
            self.current_index = clamped_index;
            self.current_byte_offset = clamped_offset;
            self.current_frame = frame;

            if clamped_index == 0 && clamped_offset == 0 {
                SeekResult::AtStart
            } else if clamped_index == self.total_entries - 1 && clamped_offset == max_bytes {
                SeekResult::AtEnd
            } else {
                SeekResult::Ok
            }
        } else {
            SeekResult::Empty
        }
    }

    /// Seek to the entry closest to the given Unix-millisecond timestamp.
    ///
    /// Uses [`SnapshotManager::find_entry_for_timestamp`] to locate the entry,
    /// then seeks to byte offset 0 of that entry.
    pub fn seek_to_timestamp(&mut self, timestamp: u64) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        if let Some(index) = self.manager.find_entry_for_timestamp(timestamp) {
            self.seek_to(index, 0)
        } else {
            SeekResult::Empty
        }
    }

    /// Seek to the very beginning of the timeline (entry 0, byte offset 0).
    pub fn seek_to_start(&mut self) -> SeekResult {
        self.seek_to(0, 0)
    }

    /// Seek to the very end of the timeline (last entry, last byte).
    pub fn seek_to_end(&mut self) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }
        let last_index = self.total_entries - 1;
        let last_offset = self.entry_metadata[last_index].1;
        self.seek_to(last_index, last_offset)
    }

    /// Step forward by `n_bytes` bytes in the input stream.
    ///
    /// If the step overflows the current entry's input bytes, it continues
    /// into the next entry. Returns [`SeekResult::AtEnd`] if already at
    /// or reaching the end of the timeline.
    pub fn step_forward(&mut self, n_bytes: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        let mut remaining = n_bytes;
        let mut index = self.current_index;
        let mut offset = self.current_byte_offset;

        while remaining > 0 {
            let entry_len = self.entry_metadata[index].1;
            let available = entry_len - offset;

            if remaining <= available {
                offset += remaining;
                remaining = 0;
            } else {
                // Consume the rest of this entry and move to the next.
                remaining -= available;
                if index + 1 < self.total_entries {
                    index += 1;
                    offset = 0;
                } else {
                    // At the last entry; clamp to end.
                    offset = entry_len;
                    remaining = 0;
                }
            }
        }

        self.seek_to(index, offset)
    }

    /// Step backward by `n_bytes` bytes in the input stream.
    ///
    /// If the step underflows the current entry's input bytes, it continues
    /// into the previous entry. Returns [`SeekResult::AtStart`] if already at
    /// or reaching the beginning of the timeline.
    pub fn step_backward(&mut self, n_bytes: usize) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        let mut remaining = n_bytes;
        let mut index = self.current_index;
        let mut offset = self.current_byte_offset;

        while remaining > 0 {
            if remaining <= offset {
                offset -= remaining;
                remaining = 0;
            } else {
                // Consume back to the start of this entry and move to previous.
                remaining -= offset;
                if index > 0 {
                    index -= 1;
                    offset = self.entry_metadata[index].1;
                } else {
                    // At the first entry; clamp to start.
                    offset = 0;
                    remaining = 0;
                }
            }
        }

        self.seek_to(index, offset)
    }

    /// Navigate to the start of the previous entry.
    ///
    /// Returns [`SeekResult::AtStart`] if already at the first entry.
    pub fn previous_entry(&mut self) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        if self.current_index == 0 {
            return self.seek_to(0, 0);
        }

        self.seek_to(self.current_index - 1, 0)
    }

    /// Navigate to the start of the next entry.
    ///
    /// Returns [`SeekResult::AtEnd`] if already at the last entry.
    pub fn next_entry(&mut self) -> SeekResult {
        if self.total_entries == 0 {
            return SeekResult::Empty;
        }

        if self.current_index + 1 >= self.total_entries {
            // Already at or past the last entry; seek to end.
            let last_index = self.total_entries - 1;
            let last_offset = self.entry_metadata[last_index].1;
            return self.seek_to(last_index, last_offset);
        }

        self.seek_to(self.current_index + 1, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::snapshot_manager::SnapshotEntry;
    use std::time::Duration;

    /// Helper: create a small terminal.
    fn make_terminal() -> Terminal {
        Terminal::new(20, 5)
    }

    /// Helper: create a manager with some entries and known content.
    fn make_populated_manager() -> SnapshotManager {
        let mut term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));

        // Entry 0: snapshot of empty terminal, then "Hello" as input
        mgr.take_snapshot(&term);
        let input0 = b"Hello";
        term.process(input0);
        mgr.record_input(input0);

        // Entry 1: snapshot after "Hello", then "World" as input
        mgr.take_snapshot(&term);
        let input1 = b"World";
        term.process(input1);
        mgr.record_input(input1);

        // Entry 2: snapshot after "HelloWorld", then "!" as input
        mgr.take_snapshot(&term);
        let input2 = b"!";
        term.process(input2);
        mgr.record_input(input2);

        mgr
    }

    #[test]
    fn test_creation_starts_at_end() {
        let mgr = make_populated_manager();
        let session = ReplaySession::new(&mgr).unwrap();

        assert_eq!(session.total_entries(), 3);
        assert_eq!(session.current_index(), 2);
        // Last entry has 1 byte of input ("!")
        assert_eq!(session.current_byte_offset(), 1);

        // Metadata should be correct
        assert_eq!(session.entry_metadata.len(), 3);
        assert_eq!(session.entry_metadata[0].1, 5); // "Hello"
        assert_eq!(session.entry_metadata[1].1, 5); // "World"
        assert_eq!(session.entry_metadata[2].1, 1); // "!"
    }

    #[test]
    fn test_empty_manager_returns_none() {
        let mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        assert!(ReplaySession::new(&mgr).is_none());
    }

    #[test]
    fn test_seek_to_start() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        let result = session.seek_to_start();
        assert_eq!(result, SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);

        // Frame should be the empty terminal (snapshot 0 with 0 bytes replayed)
        let grid = session.current_frame().grid();
        assert_eq!(grid.get(0, 0).unwrap().c, ' ');
    }

    #[test]
    fn test_seek_to_end() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // First seek to start, then back to end.
        session.seek_to_start();
        let result = session.seek_to_end();
        assert_eq!(result, SeekResult::AtEnd);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 1);
    }

    #[test]
    fn test_step_forward_within_entry() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Seek to start first.
        session.seek_to_start();
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);

        // Step forward 3 bytes within entry 0 ("Hel" from "Hello")
        let result = session.step_forward(3);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 3);

        // Verify frame content: "Hel" should be visible
        let grid = session.current_frame().grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'H');
        assert_eq!(grid.get(1, 0).unwrap().c, 'e');
        assert_eq!(grid.get(2, 0).unwrap().c, 'l');
        assert_eq!(grid.get(3, 0).unwrap().c, ' '); // not yet processed
    }

    #[test]
    fn test_step_forward_across_entries() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        session.seek_to_start();

        // Step forward 7 bytes: 5 from entry 0 ("Hello") + 2 from entry 1 ("Wo")
        let result = session.step_forward(7);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 1);
        assert_eq!(session.current_byte_offset(), 2);
    }

    #[test]
    fn test_step_forward_at_end_returns_at_end() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Already at end, step forward
        let result = session.step_forward(10);
        assert_eq!(result, SeekResult::AtEnd);
    }

    #[test]
    fn test_step_backward_within_entry() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Start at entry 0, offset 5 (end of "Hello")
        session.seek_to(0, 5);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 5);

        // Step backward 2 bytes => offset 3 ("Hel")
        let result = session.step_backward(2);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 3);

        // Verify frame content
        let grid = session.current_frame().grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'H');
        assert_eq!(grid.get(1, 0).unwrap().c, 'e');
        assert_eq!(grid.get(2, 0).unwrap().c, 'l');
        assert_eq!(grid.get(3, 0).unwrap().c, ' ');
    }

    #[test]
    fn test_step_backward_across_entries() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Start at entry 1, offset 2
        session.seek_to(1, 2);

        // Step backward 4 bytes: 2 from entry 1 + 2 from entry 0 (offset 5-2=3)
        let result = session.step_backward(4);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 3);
    }

    #[test]
    fn test_step_backward_at_start_returns_at_start() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        session.seek_to_start();

        let result = session.step_backward(10);
        assert_eq!(result, SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);
    }

    #[test]
    fn test_next_entry() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        session.seek_to_start();

        // Move to next entry (entry 1)
        let result = session.next_entry();
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 1);
        assert_eq!(session.current_byte_offset(), 0);

        // Move to next entry (entry 2)
        let result = session.next_entry();
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 0);

        // Move to next entry — already at last, should go to end
        let result = session.next_entry();
        assert_eq!(result, SeekResult::AtEnd);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 1);
    }

    #[test]
    fn test_previous_entry() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Start at the end (entry 2, offset 1)
        assert_eq!(session.current_index(), 2);

        // Move to previous entry (entry 1)
        let result = session.previous_entry();
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 1);
        assert_eq!(session.current_byte_offset(), 0);

        // Move to previous entry (entry 0)
        let result = session.previous_entry();
        assert_eq!(result, SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);

        // Try again — already at start
        let result = session.previous_entry();
        assert_eq!(result, SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);
    }

    #[test]
    fn test_seek_to_timestamp() {
        let term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));

        // Entry 0
        mgr.take_snapshot(&term);
        mgr.record_input(b"aaa");

        // Entry 1 with a known later timestamp
        let mut snap1 = term.capture_snapshot();
        let ts0 = mgr.get_entry(0).unwrap().snapshot.timestamp;
        snap1.timestamp = ts0 + 1000;
        snap1.estimated_size_bytes = snap1.estimate_size();
        mgr.push_entry(SnapshotEntry {
            snapshot: snap1,
            input_bytes: b"bbb".to_vec(),
        });

        // Entry 2 with an even later timestamp
        let mut snap2 = term.capture_snapshot();
        snap2.timestamp = ts0 + 2000;
        snap2.estimated_size_bytes = snap2.estimate_size();
        mgr.push_entry(SnapshotEntry {
            snapshot: snap2,
            input_bytes: b"ccc".to_vec(),
        });

        let mut session = ReplaySession::new(&mgr).unwrap();

        // Seek to a timestamp between entry 0 and entry 1
        let result = session.seek_to_timestamp(ts0 + 500);
        assert_eq!(result, SeekResult::AtStart);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 0);

        // Seek to a timestamp between entry 1 and entry 2
        let result = session.seek_to_timestamp(ts0 + 1500);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 1);
        assert_eq!(session.current_byte_offset(), 0);

        // Seek to the exact timestamp of entry 2
        let result = session.seek_to_timestamp(ts0 + 2000);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 0);
    }

    #[test]
    fn test_seek_to_clamps_out_of_range() {
        let mgr = make_populated_manager();
        let mut session = ReplaySession::new(&mgr).unwrap();

        // Seek to an entry index beyond the total — clamped to last entry, offset 0
        let result = session.seek_to(100, 0);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 0);

        // Seek to a byte offset beyond the entry's input bytes — clamped to max
        let result = session.seek_to(0, 9999);
        // entry 0 has 5 bytes, so offset clamped to 5
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 5);

        // Seek with both index and offset clamped to the very end
        let result = session.seek_to(100, 9999);
        assert_eq!(result, SeekResult::AtEnd);
        assert_eq!(session.current_index(), 2);
        assert_eq!(session.current_byte_offset(), 1);
    }

    #[test]
    fn test_current_timestamp() {
        let mgr = make_populated_manager();
        let session = ReplaySession::new(&mgr).unwrap();

        // current_timestamp should return the timestamp of the current entry
        let ts = session.current_timestamp();
        assert!(ts > 0);
    }

    #[test]
    fn test_single_entry_navigation() {
        let mut term = make_terminal();
        let mut mgr = SnapshotManager::new(1024 * 1024, Duration::from_secs(0));
        mgr.take_snapshot(&term);
        let input = b"AB";
        term.process(input);
        mgr.record_input(input);

        let mut session = ReplaySession::new(&mgr).unwrap();
        assert_eq!(session.total_entries(), 1);
        // Starts at end: index 0, offset 2
        assert_eq!(session.current_index(), 0);
        assert_eq!(session.current_byte_offset(), 2);

        // seek_to_start
        let result = session.seek_to_start();
        assert_eq!(result, SeekResult::AtStart);

        // step forward 1 byte
        let result = session.step_forward(1);
        assert_eq!(result, SeekResult::Ok);
        assert_eq!(session.current_byte_offset(), 1);
        let grid = session.current_frame().grid();
        assert_eq!(grid.get(0, 0).unwrap().c, 'A');
        assert_eq!(grid.get(1, 0).unwrap().c, ' ');

        // step forward to end
        let result = session.step_forward(1);
        assert_eq!(result, SeekResult::AtEnd);
        assert_eq!(session.current_byte_offset(), 2);

        // previous_entry at entry 0 should return AtStart
        let result = session.previous_entry();
        assert_eq!(result, SeekResult::AtStart);

        // next_entry at last entry should return AtEnd
        let result = session.next_entry();
        assert_eq!(result, SeekResult::AtEnd);
    }
}
