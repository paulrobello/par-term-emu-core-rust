# File Transfer Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add general-purpose OSC 1337 file transfer support (download, upload, progress tracking) per the design in `docs/plans/2026-02-14-file-transfer-design.md`.

**Architecture:** New `FileTransferManager` on `Terminal` struct handles download (`inline=0`) and upload (`RequestUpload`) flows. Events emitted via existing `TerminalEvent` system. Existing inline image path (`inline=1`) unchanged.

**Tech Stack:** Rust, PyO3, protobuf (prost), base64 crate

---

### Task 1: Create FileTransfer module with core types

**Files:**
- Create: `src/terminal/file_transfer.rs`
- Modify: `src/terminal/mod.rs` (add `pub mod file_transfer` and re-exports)

**Step 1: Write failing test for FileTransferManager**

In `src/terminal/file_transfer.rs`, add the module with tests at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_download() {
        let mut mgr = FileTransferManager::new();
        let id = mgr.start_download(Some("test.txt".into()), Some(100), HashMap::new());
        let transfer = mgr.get_transfer(id).unwrap();
        assert_eq!(transfer.filename, Some("test.txt".into()));
        assert!(matches!(transfer.status, TransferStatus::Pending));
        assert_eq!(transfer.direction, TransferDirection::Download);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib file_transfer -- --nocapture 2>&1 | head -30`
Expected: FAIL — module doesn't exist yet

**Step 3: Write the core types and FileTransferManager**

```rust
//! File transfer management for OSC 1337 File= protocol
//!
//! Supports general-purpose file upload/download via iTerm2's OSC 1337 protocol.
//! The manager tracks active transfers, stores completed file data for frontend
//! retrieval, and provides lifecycle state for event emission.

use std::collections::HashMap;

/// Unique identifier for a file transfer
pub type TransferId = u64;

/// Direction of the file transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    /// Host → Terminal (OSC 1337 File= with inline=0)
    Download,
    /// Terminal → Host (OSC 1337 RequestUpload=format=tgz)
    Upload,
}

/// Current state of a file transfer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    /// Transfer initiated, waiting for data
    Pending,
    /// Actively receiving/sending chunks
    InProgress {
        bytes_transferred: usize,
        total_bytes: Option<usize>,
    },
    /// Transfer completed successfully
    Completed,
    /// Transfer failed
    Failed(String),
    /// Transfer cancelled
    Cancelled,
}

const DEFAULT_MAX_COMPLETED: usize = 32;
const DEFAULT_MAX_TRANSFER_SIZE: usize = 50 * 1024 * 1024; // 50MB

/// A file transfer (download or upload)
#[derive(Debug, Clone)]
pub struct FileTransfer {
    /// Unique transfer identifier
    pub id: TransferId,
    /// Direction of transfer
    pub direction: TransferDirection,
    /// Filename (decoded from base64 name= param)
    pub filename: Option<String>,
    /// Current transfer status
    pub status: TransferStatus,
    /// Accumulated raw bytes (decoded from base64)
    pub data: Vec<u8>,
    /// Original OSC parameters
    pub params: HashMap<String, String>,
    /// Transfer start time (unix millis)
    pub started_at: u64,
    /// Transfer completion time (unix millis)
    pub completed_at: Option<u64>,
}

/// Manages active and recently completed file transfers
#[derive(Debug)]
pub struct FileTransferManager {
    /// Active (in-progress) transfers keyed by ID
    active_transfers: HashMap<TransferId, FileTransfer>,
    /// Recently completed transfers (ring buffer)
    completed_transfers: Vec<FileTransfer>,
    /// Max completed transfers to retain
    max_completed: usize,
    /// Next transfer ID to assign
    next_id: TransferId,
    /// Maximum allowed transfer size in bytes
    max_transfer_size: usize,
}

impl Default for FileTransferManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTransferManager {
    /// Create a new file transfer manager with default limits
    pub fn new() -> Self {
        Self {
            active_transfers: HashMap::new(),
            completed_transfers: Vec::new(),
            max_completed: DEFAULT_MAX_COMPLETED,
            next_id: 1,
            max_transfer_size: DEFAULT_MAX_TRANSFER_SIZE,
        }
    }

    /// Start a new download transfer. Returns the transfer ID.
    pub fn start_download(
        &mut self,
        filename: Option<String>,
        total_bytes: Option<usize>,
        params: HashMap<String, String>,
    ) -> TransferId {
        let id = self.next_id;
        self.next_id += 1;

        let transfer = FileTransfer {
            id,
            direction: TransferDirection::Download,
            filename,
            status: TransferStatus::Pending,
            data: Vec::new(),
            params,
            started_at: crate::terminal::unix_millis(),
            completed_at: None,
        };

        self.active_transfers.insert(id, transfer);
        id
    }

    /// Append decoded bytes to an active transfer.
    /// Returns Ok(bytes_so_far) or Err(reason) if transfer not found or size exceeded.
    pub fn append_data(&mut self, id: TransferId, data: &[u8]) -> Result<usize, String> {
        let transfer = self
            .active_transfers
            .get_mut(&id)
            .ok_or_else(|| format!("Transfer {} not found", id))?;

        let new_size = transfer.data.len() + data.len();
        if new_size > self.max_transfer_size {
            transfer.status = TransferStatus::Failed(format!(
                "Transfer size {} exceeds limit {}",
                new_size, self.max_transfer_size
            ));
            return Err(format!(
                "Transfer size {} exceeds limit {}",
                new_size, self.max_transfer_size
            ));
        }

        transfer.data.extend_from_slice(data);
        let total_bytes = transfer.params.get("size").and_then(|s| s.parse().ok());
        transfer.status = TransferStatus::InProgress {
            bytes_transferred: transfer.data.len(),
            total_bytes,
        };

        Ok(transfer.data.len())
    }

    /// Mark a transfer as completed and move it to the completed list.
    pub fn complete_transfer(&mut self, id: TransferId) -> Option<&FileTransfer> {
        if let Some(mut transfer) = self.active_transfers.remove(&id) {
            transfer.status = TransferStatus::Completed;
            transfer.completed_at = Some(crate::terminal::unix_millis());

            // Ring buffer eviction
            if self.completed_transfers.len() >= self.max_completed {
                self.completed_transfers.remove(0);
            }
            self.completed_transfers.push(transfer);
            self.completed_transfers.last()
        } else {
            None
        }
    }

    /// Mark a transfer as failed and move it to the completed list.
    pub fn fail_transfer(&mut self, id: TransferId, reason: String) {
        if let Some(mut transfer) = self.active_transfers.remove(&id) {
            transfer.status = TransferStatus::Failed(reason);
            transfer.completed_at = Some(crate::terminal::unix_millis());

            if self.completed_transfers.len() >= self.max_completed {
                self.completed_transfers.remove(0);
            }
            self.completed_transfers.push(transfer);
        }
    }

    /// Cancel an active transfer. Returns true if found and cancelled.
    pub fn cancel_transfer(&mut self, id: TransferId) -> bool {
        if let Some(mut transfer) = self.active_transfers.remove(&id) {
            transfer.status = TransferStatus::Cancelled;
            transfer.completed_at = Some(crate::terminal::unix_millis());

            if self.completed_transfers.len() >= self.max_completed {
                self.completed_transfers.remove(0);
            }
            self.completed_transfers.push(transfer);
            true
        } else {
            false
        }
    }

    /// Get a reference to a transfer (active or completed) by ID.
    pub fn get_transfer(&self, id: TransferId) -> Option<&FileTransfer> {
        self.active_transfers
            .get(&id)
            .or_else(|| self.completed_transfers.iter().find(|t| t.id == id))
    }

    /// Get all active transfers.
    pub fn active_transfers(&self) -> Vec<&FileTransfer> {
        self.active_transfers.values().collect()
    }

    /// Get all completed transfers.
    pub fn completed_transfers(&self) -> Vec<&FileTransfer> {
        self.completed_transfers.iter().collect()
    }

    /// Take a completed transfer by ID (removes it from the completed list).
    /// Returns the transfer with its data for the frontend to save.
    pub fn take_completed_transfer(&mut self, id: TransferId) -> Option<FileTransfer> {
        if let Some(pos) = self.completed_transfers.iter().position(|t| t.id == id) {
            Some(self.completed_transfers.remove(pos))
        } else {
            None
        }
    }

    /// Get the maximum transfer size limit.
    pub fn max_transfer_size(&self) -> usize {
        self.max_transfer_size
    }

    /// Set the maximum transfer size limit.
    pub fn set_max_transfer_size(&mut self, bytes: usize) {
        self.max_transfer_size = bytes;
    }
}
```

Then add the module declaration and re-exports to `src/terminal/mod.rs`:

At line 18 (after `mod write;`), add:
```rust
pub mod file_transfer;
```

At line 31 (after the trigger re-exports), add:
```rust
// Re-export file transfer types as they're part of the public API
pub use file_transfer::{
    FileTransfer, FileTransferManager, TransferDirection, TransferId, TransferStatus,
};
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib file_transfer -- --nocapture`
Expected: PASS

**Step 5: Add remaining unit tests**

Add these tests to the `#[cfg(test)] mod tests` block in `file_transfer.rs`:

```rust
    #[test]
    fn test_append_data_and_complete() {
        let mut mgr = FileTransferManager::new();
        let id = mgr.start_download(Some("test.bin".into()), Some(10), HashMap::new());

        let result = mgr.append_data(id, b"hello");
        assert_eq!(result, Ok(5));

        let result = mgr.append_data(id, b"world");
        assert_eq!(result, Ok(10));

        mgr.complete_transfer(id);
        let transfer = mgr.get_transfer(id).unwrap();
        assert!(matches!(transfer.status, TransferStatus::Completed));
        assert_eq!(transfer.data, b"helloworld");
    }

    #[test]
    fn test_size_limit_enforcement() {
        let mut mgr = FileTransferManager::new();
        mgr.set_max_transfer_size(10);
        let id = mgr.start_download(None, None, HashMap::new());

        let result = mgr.append_data(id, &[0u8; 11]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel_transfer() {
        let mut mgr = FileTransferManager::new();
        let id = mgr.start_download(None, None, HashMap::new());

        assert!(mgr.cancel_transfer(id));
        let transfer = mgr.get_transfer(id).unwrap();
        assert!(matches!(transfer.status, TransferStatus::Cancelled));
    }

    #[test]
    fn test_take_completed_transfer() {
        let mut mgr = FileTransferManager::new();
        let id = mgr.start_download(Some("file.dat".into()), None, HashMap::new());
        mgr.append_data(id, b"data").unwrap();
        mgr.complete_transfer(id);

        let taken = mgr.take_completed_transfer(id);
        assert!(taken.is_some());
        assert_eq!(taken.unwrap().data, b"data");

        // Should be gone now
        assert!(mgr.get_transfer(id).is_none());
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut mgr = FileTransferManager::new();
        // Create and complete max_completed + 1 transfers
        let max = 32;
        for i in 0..=max {
            let id = mgr.start_download(Some(format!("file_{}.txt", i)), None, HashMap::new());
            mgr.complete_transfer(id);
        }
        // First transfer should have been evicted
        assert_eq!(mgr.completed_transfers().len(), max);
        assert!(mgr.get_transfer(1).is_none()); // ID 1 was evicted
    }

    #[test]
    fn test_fail_transfer() {
        let mut mgr = FileTransferManager::new();
        let id = mgr.start_download(None, None, HashMap::new());
        mgr.fail_transfer(id, "decode error".into());

        let transfer = mgr.get_transfer(id).unwrap();
        assert!(matches!(transfer.status, TransferStatus::Failed(ref r) if r == "decode error"));
    }

    #[test]
    fn test_active_transfers_list() {
        let mut mgr = FileTransferManager::new();
        let id1 = mgr.start_download(Some("a.txt".into()), None, HashMap::new());
        let id2 = mgr.start_download(Some("b.txt".into()), None, HashMap::new());

        assert_eq!(mgr.active_transfers().len(), 2);
        mgr.complete_transfer(id1);
        assert_eq!(mgr.active_transfers().len(), 1);
        assert_eq!(mgr.active_transfers()[0].id, id2);
    }
```

**Step 6: Run all tests**

Run: `cargo test --lib file_transfer -- --nocapture`
Expected: All PASS

**Step 7: Commit**

```bash
git add src/terminal/file_transfer.rs src/terminal/mod.rs
git commit -m "feat: add FileTransfer module with core types and manager"
```

---

### Task 2: Add TerminalEvent variants for file transfer lifecycle

**Files:**
- Modify: `src/terminal/mod.rs:97-243` (TerminalEvent and TerminalEventKind enums)
- Modify: `src/terminal/mod.rs:3149-3169` (event_kind mapping)

**Step 1: Add new TerminalEvent variants**

In `src/terminal/mod.rs`, after the `SubShellDetected` variant (line ~218), add before the closing `}`:

```rust
    /// A file transfer started (download or upload)
    FileTransferStarted {
        /// Unique transfer identifier
        id: u64,
        /// Direction: "download" or "upload"
        direction: TransferDirection,
        /// Filename if known
        filename: Option<String>,
        /// Total expected bytes if known
        total_bytes: Option<usize>,
    },
    /// File transfer progress update
    FileTransferProgress {
        /// Transfer identifier
        id: u64,
        /// Bytes transferred so far
        bytes_transferred: usize,
        /// Total expected bytes if known
        total_bytes: Option<usize>,
    },
    /// File transfer completed successfully
    FileTransferCompleted {
        /// Transfer identifier
        id: u64,
        /// Filename if known
        filename: Option<String>,
        /// Total bytes transferred
        size: usize,
    },
    /// File transfer failed or cancelled
    FileTransferFailed {
        /// Transfer identifier
        id: u64,
        /// Failure reason
        reason: String,
    },
    /// Host requested file upload via RequestUpload protocol
    UploadRequested {
        /// Upload format (e.g., "tgz")
        format: String,
    },
```

You'll need to add a use for `TransferDirection` at the top of the enum's containing scope. Since `file_transfer` is already `pub mod` and re-exported, the existing `use` in the re-exports handles this.

**Step 2: Add corresponding TerminalEventKind variants**

After `SubShellDetected` in the `TerminalEventKind` enum (line ~242), add:

```rust
    FileTransferStarted,
    FileTransferProgress,
    FileTransferCompleted,
    FileTransferFailed,
    UploadRequested,
```

**Step 3: Update event_kind() match**

In `src/terminal/mod.rs` at the `event_kind()` function (around line 3149), add before the closing `}` of the match:

```rust
            TerminalEvent::FileTransferStarted { .. } => TerminalEventKind::FileTransferStarted,
            TerminalEvent::FileTransferProgress { .. } => TerminalEventKind::FileTransferProgress,
            TerminalEvent::FileTransferCompleted { .. } => TerminalEventKind::FileTransferCompleted,
            TerminalEvent::FileTransferFailed { .. } => TerminalEventKind::FileTransferFailed,
            TerminalEvent::UploadRequested { .. } => TerminalEventKind::UploadRequested,
```

**Step 4: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: All existing tests PASS (no behavior change)

**Step 5: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat: add TerminalEvent variants for file transfer lifecycle"
```

---

### Task 3: Add FileTransferManager to Terminal struct and wire up ITermMultipartState

**Files:**
- Modify: `src/terminal/mod.rs:859-870` (ITermMultipartState)
- Modify: `src/terminal/mod.rs:1340-1370` (Terminal struct fields)
- Modify: `src/terminal/mod.rs:1800-1870` (Terminal::new initializer)

**Step 1: Add `is_file_transfer` field to ITermMultipartState**

In `src/terminal/mod.rs` at the `ITermMultipartState` struct (line ~861), add a new field after `accumulated_size`:

```rust
    /// Whether this is a non-inline file transfer (inline=0)
    is_file_transfer: bool,
    /// Transfer ID in FileTransferManager (only set when is_file_transfer=true)
    transfer_id: Option<u64>,
```

**Step 2: Add FileTransferManager field to Terminal struct**

In `src/terminal/mod.rs`, after `iterm_multipart_buffer` (line ~1365), add:

```rust
    /// File transfer manager for OSC 1337 File= downloads and RequestUpload uploads
    file_transfer_manager: FileTransferManager,
```

**Step 3: Initialize in Terminal::new**

In the Terminal::new function, after `iterm_multipart_buffer: None,` (line ~1814), add:

```rust
            file_transfer_manager: FileTransferManager::new(),
```

**Step 4: Add public accessor methods to Terminal**

Add these methods to a new impl block or within an existing one in `src/terminal/mod.rs`. Find a suitable location near other public API methods. Add after the existing graphics-related methods:

```rust
    // === File Transfer API ===

    /// Get all active file transfers
    pub fn get_active_transfers(&self) -> Vec<&FileTransfer> {
        self.file_transfer_manager.active_transfers()
    }

    /// Get all completed file transfers
    pub fn get_completed_transfers(&self) -> Vec<&FileTransfer> {
        self.file_transfer_manager.completed_transfers()
    }

    /// Get a file transfer by ID (active or completed)
    pub fn get_transfer(&self, id: TransferId) -> Option<&FileTransfer> {
        self.file_transfer_manager.get_transfer(id)
    }

    /// Take a completed transfer (removes from completed list, returns with data)
    pub fn take_completed_transfer(&mut self, id: TransferId) -> Option<FileTransfer> {
        self.file_transfer_manager.take_completed_transfer(id)
    }

    /// Cancel an active file transfer
    pub fn cancel_file_transfer(&mut self, id: TransferId) -> bool {
        let cancelled = self.file_transfer_manager.cancel_transfer(id);
        if cancelled {
            self.terminal_events
                .push(TerminalEvent::FileTransferFailed {
                    id,
                    reason: "cancelled".into(),
                });
        }
        cancelled
    }

    /// Set maximum file transfer size in bytes
    pub fn set_max_transfer_size(&mut self, bytes: usize) {
        self.file_transfer_manager.set_max_transfer_size(bytes);
    }

    /// Get maximum file transfer size in bytes
    pub fn get_max_transfer_size(&self) -> usize {
        self.file_transfer_manager.max_transfer_size()
    }
```

**Step 5: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: PASS

**Step 6: Commit**

```bash
git add src/terminal/mod.rs
git commit -m "feat: add FileTransferManager to Terminal struct with public API"
```

---

### Task 4: Modify graphics.rs handlers for inline=0 download support

**Files:**
- Modify: `src/terminal/graphics.rs:257-521` (handler functions)
- Modify: `src/graphics/iterm.rs:33-48` (remove inline=1 hard requirement from parse_params)

**Step 1: Update ITermParser::parse_params to accept inline=0**

In `src/graphics/iterm.rs`, change `parse_params` to not error on missing inline=1. Instead, just parse params and return Ok. The caller decides what to do based on the inline value.

Replace the `parse_params` method body (lines 33-48):

```rust
    pub fn parse_params(&mut self, params_str: &str) -> Result<(), GraphicsError> {
        self.params.clear();

        for part in params_str.split(';') {
            if let Some((key, value)) = part.split_once('=') {
                self.params.insert(key.to_string(), value.to_string());
            }
        }

        Ok(())
    }

    /// Check if inline display is requested (inline=1)
    pub fn is_inline(&self) -> bool {
        self.params.get("inline").map(|v| v == "1").unwrap_or(false)
    }
```

**Step 2: Update handle_multipart_file_start to support inline=0**

In `src/terminal/graphics.rs`, replace `handle_multipart_file_start` (lines 257-304):

```rust
    /// Handle MultipartFile command (start of chunked transfer)
    fn handle_multipart_file_start(&mut self, params_str: &str) {
        use std::collections::HashMap;

        // Parse parameters: inline=1;size=280459;name=...
        let mut params = HashMap::new();
        for part in params_str.split(';') {
            if let Some((key, value)) = part.split_once('=') {
                params.insert(key.to_string(), value.to_string());
            }
        }

        let is_inline = params.get("inline").map(|v| v == "1").unwrap_or(false);
        let is_file_transfer = !is_inline;

        // Get expected size if provided
        let total_size = params.get("size").and_then(|s| s.parse::<usize>().ok());

        if is_inline {
            // Inline image: check against graphics store limits
            if let Some(size) = total_size {
                let limits = self.graphics_store.limits();
                if size > limits.max_total_memory {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "ITERM",
                        &format!(
                            "MultipartFile rejected: size {} exceeds graphics limit {}",
                            size, limits.max_total_memory
                        ),
                    );
                    return;
                }
            }
        } else {
            // File transfer: check against file transfer limits
            if let Some(size) = total_size {
                if size > self.file_transfer_manager.max_transfer_size() {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "ITERM",
                        &format!(
                            "MultipartFile rejected: size {} exceeds transfer limit {}",
                            size,
                            self.file_transfer_manager.max_transfer_size()
                        ),
                    );
                    return;
                }
            }
        }

        // Decode filename from base64 name= param
        let filename = params.get("name").and_then(|n| {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, n.as_bytes())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        });

        // Start file transfer if non-inline
        let transfer_id = if is_file_transfer {
            let id = self.file_transfer_manager.start_download(
                filename.clone(),
                total_size,
                params.clone(),
            );
            self.terminal_events
                .push(crate::terminal::TerminalEvent::FileTransferStarted {
                    id,
                    direction: crate::terminal::TransferDirection::Download,
                    filename,
                    total_bytes: total_size,
                });
            Some(id)
        } else {
            None
        };

        // Initialize multipart state
        self.iterm_multipart_buffer = Some(crate::terminal::ITermMultipartState {
            params,
            chunks: Vec::new(),
            total_size,
            accumulated_size: 0,
            is_file_transfer,
            transfer_id,
        });
    }
```

**Step 3: Update handle_file_part to route chunks appropriately**

Replace `handle_file_part` (lines 307-377):

```rust
    /// Handle FilePart command (chunk of data in multipart transfer)
    fn handle_file_part(&mut self, base64_chunk: &str) {
        // Check if we have an active multipart transfer
        let state = match self.iterm_multipart_buffer.as_mut() {
            Some(s) => s,
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    "FilePart received without MultipartFile",
                );
                return;
            }
        };

        // Decode the chunk to check its size
        let decoded = match base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            base64_chunk.as_bytes(),
        ) {
            Ok(d) => d,
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!("FilePart base64 decode failed: {}", e),
                );
                // If this is a file transfer, fail it
                if let Some(tid) = state.transfer_id {
                    self.file_transfer_manager
                        .fail_transfer(tid, format!("base64 decode failed: {}", e));
                    self.terminal_events
                        .push(crate::terminal::TerminalEvent::FileTransferFailed {
                            id: tid,
                            reason: format!("base64 decode failed: {}", e),
                        });
                }
                self.iterm_multipart_buffer = None;
                return;
            }
        };

        let decoded_size = decoded.len();

        // Check if adding this chunk would exceed size limit
        let new_accumulated = state.accumulated_size + decoded_size;
        if let Some(expected_size) = state.total_size {
            if new_accumulated > expected_size {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "FilePart rejected: accumulated {} + chunk {} > expected {}",
                        state.accumulated_size, decoded_size, expected_size
                    ),
                );
                if let Some(tid) = state.transfer_id {
                    self.file_transfer_manager.fail_transfer(
                        tid,
                        format!(
                            "data exceeds declared size: {} > {}",
                            new_accumulated, expected_size
                        ),
                    );
                    self.terminal_events
                        .push(crate::terminal::TerminalEvent::FileTransferFailed {
                            id: tid,
                            reason: format!(
                                "data exceeds declared size: {} > {}",
                                new_accumulated, expected_size
                            ),
                        });
                }
                self.iterm_multipart_buffer = None;
                return;
            }
        }

        // For file transfers, append decoded bytes directly to the manager
        if state.is_file_transfer {
            if let Some(tid) = state.transfer_id {
                match self.file_transfer_manager.append_data(tid, &decoded) {
                    Ok(bytes_so_far) => {
                        self.terminal_events.push(
                            crate::terminal::TerminalEvent::FileTransferProgress {
                                id: tid,
                                bytes_transferred: bytes_so_far,
                                total_bytes: state.total_size,
                            },
                        );
                    }
                    Err(reason) => {
                        self.terminal_events
                            .push(crate::terminal::TerminalEvent::FileTransferFailed {
                                id: tid,
                                reason,
                            });
                        self.iterm_multipart_buffer = None;
                        return;
                    }
                }
            }
        }

        // For inline images, keep the base64 chunks for later assembly
        if !state.is_file_transfer {
            state.chunks.push(base64_chunk.to_string());
        }
        state.accumulated_size = new_accumulated;

        // Check if transfer is complete
        let is_complete = if let Some(expected_size) = state.total_size {
            state.accumulated_size >= expected_size
        } else {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                "MultipartFile missing size parameter - cannot determine completion",
            );
            if let Some(tid) = state.transfer_id {
                self.file_transfer_manager
                    .fail_transfer(tid, "missing size parameter".into());
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::FileTransferFailed {
                        id: tid,
                        reason: "missing size parameter".into(),
                    });
            }
            self.iterm_multipart_buffer = None;
            return;
        };

        if is_complete {
            self.finalize_multipart_transfer();
        }
    }
```

**Step 4: Update finalize_multipart_transfer**

Replace `finalize_multipart_transfer` (lines 380-400):

```rust
    /// Finalize multipart transfer and process the complete data
    fn finalize_multipart_transfer(&mut self) {
        let state = match self.iterm_multipart_buffer.take() {
            Some(s) => s,
            None => return,
        };

        if state.is_file_transfer {
            // File transfer: data already accumulated in FileTransferManager
            if let Some(tid) = state.transfer_id {
                if let Some(transfer) = self.file_transfer_manager.complete_transfer(tid) {
                    let filename = transfer.filename.clone();
                    let size = transfer.data.len();
                    self.terminal_events
                        .push(crate::terminal::TerminalEvent::FileTransferCompleted {
                            id: tid,
                            filename,
                            size,
                        });
                }
            }
        } else {
            // Inline image: join chunks and process as single transfer
            let complete_data = state.chunks.join("");

            let mut params_parts = Vec::new();
            for (key, value) in &state.params {
                params_parts.push(format!("{}={}", key, value));
            }
            let params_str = params_parts.join(";");
            let file_data = format!("File={}:{}", params_str, complete_data);

            self.handle_single_file_transfer(&file_data);
        }
    }
```

**Step 5: Update handle_single_file_transfer to support inline=0**

Replace `handle_single_file_transfer` (lines 403-521):

```rust
    /// Handle single-sequence File= transfer
    fn handle_single_file_transfer(&mut self, data: &str) {
        use crate::graphics::iterm::ITermParser;

        // Split into params and data at the colon
        let (params_str, file_data) = match data.split_once(':') {
            Some((p, d)) => (p, d),
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    "No colon separator in File= format",
                );
                return;
            }
        };

        // Must start with "File="
        if !params_str.starts_with("File=") {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                &format!("Unsupported OSC 1337 command: {}", params_str),
            );
            return;
        }

        let params_str = &params_str[5..]; // Remove "File=" prefix

        let mut parser = ITermParser::new();

        // Parse parameters (no longer requires inline=1)
        if let Err(e) = parser.parse_params(params_str) {
            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                &format!("Failed to parse iTerm params: {}", e),
            );
            return;
        }

        if parser.is_inline() {
            // === Inline image path (existing behavior, unchanged) ===
            parser.set_data(file_data.as_bytes());

            let position = (self.cursor.col, self.cursor.row);

            match parser.decode_image(position) {
                Ok(mut graphic) => {
                    let (cell_w, cell_h) = self.cell_dimensions;
                    graphic.set_cell_dimensions(cell_w, cell_h);

                    let graphic_height_in_rows = graphic.height.div_ceil(cell_h as usize);
                    let new_cursor_col = 0;
                    let new_cursor_row =
                        self.cursor.row.saturating_add(graphic_height_in_rows);

                    let (_, rows) = self.size();
                    if new_cursor_row >= rows {
                        let scroll_amount = new_cursor_row - rows + 1;
                        let scroll_top = self.scroll_region_top;
                        let scroll_bottom = self.scroll_region_bottom;

                        self.active_grid_mut().scroll_region_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );
                        self.adjust_graphics_for_scroll_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );

                        let original_row = graphic.position.1;
                        let new_row = original_row.saturating_sub(scroll_amount);
                        graphic.position.1 = new_row;

                        if scroll_amount > original_row {
                            graphic.scroll_offset_rows = scroll_amount - original_row;
                        }

                        self.cursor.row = rows - 1;
                        self.cursor.col = new_cursor_col;
                    } else {
                        self.cursor.row = new_cursor_row;
                        self.cursor.col = new_cursor_col;
                    }

                    self.graphics_store.add_graphic(graphic.clone());

                    debug::log(
                        debug::DebugLevel::Debug,
                        "ITERM",
                        &format!(
                            "Added iTerm image at ({}, {}), size {}x{}, cursor moved to ({}, {})",
                            position.0,
                            position.1,
                            graphic.width,
                            graphic.height,
                            self.cursor.col,
                            self.cursor.row
                        ),
                    );
                }
                Err(e) => {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "ITERM",
                        &format!("Failed to decode iTerm image: {}", e),
                    );
                }
            }
        } else {
            // === File download path (inline=0) ===
            // Decode base64 data
            let decoded = match base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                file_data.as_bytes(),
            ) {
                Ok(d) => d,
                Err(e) => {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "ITERM",
                        &format!("File download base64 decode failed: {}", e),
                    );
                    return;
                }
            };

            // Parse filename from name= param (base64-encoded)
            let filename = parser.get_param("name").and_then(|n| {
                base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    n.as_bytes(),
                )
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
            });

            // Build params map
            let mut params = std::collections::HashMap::new();
            for part in params_str.split(';') {
                if let Some((key, value)) = part.split_once('=') {
                    params.insert(key.to_string(), value.to_string());
                }
            }

            let size = decoded.len();

            // Check size limit
            if size > self.file_transfer_manager.max_transfer_size() {
                debug::log(
                    debug::DebugLevel::Debug,
                    "ITERM",
                    &format!(
                        "File download rejected: size {} exceeds limit {}",
                        size,
                        self.file_transfer_manager.max_transfer_size()
                    ),
                );
                return;
            }

            // Start and immediately complete the transfer
            let id = self.file_transfer_manager.start_download(
                filename.clone(),
                Some(size),
                params,
            );

            // Append all data at once
            if let Err(reason) = self.file_transfer_manager.append_data(id, &decoded) {
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::FileTransferFailed { id, reason });
                return;
            }

            self.file_transfer_manager.complete_transfer(id);

            self.terminal_events
                .push(crate::terminal::TerminalEvent::FileTransferStarted {
                    id,
                    direction: crate::terminal::TransferDirection::Download,
                    filename: filename.clone(),
                    total_bytes: Some(size),
                });
            self.terminal_events
                .push(crate::terminal::TerminalEvent::FileTransferCompleted {
                    id,
                    filename,
                    size,
                });

            debug::log(
                debug::DebugLevel::Debug,
                "ITERM",
                &format!(
                    "File download received: {} bytes, filename={:?}",
                    size,
                    self.file_transfer_manager
                        .get_transfer(id)
                        .and_then(|t| t.filename.clone())
                ),
            );
        }
    }
```

**Step 6: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: All PASS (including existing graphics tests)

**Step 7: Commit**

```bash
git add src/terminal/graphics.rs src/graphics/iterm.rs src/terminal/mod.rs
git commit -m "feat: wire up download handler for inline=0 file transfers"
```

---

### Task 5: Add RequestUpload handler in OSC dispatcher

**Files:**
- Modify: `src/terminal/sequences/osc.rs:707-734` (OSC 1337 handler)
- Modify: `src/terminal/mod.rs` (add upload response methods)

**Step 1: Add RequestUpload route in OSC 1337 handler**

In `src/terminal/sequences/osc.rs`, in the `"1337"` match arm (line ~729), add a new branch before the default `else` block. Change:

```rust
                        } else {
                            // Default to inline image handling
                            self.handle_iterm_image(&data);
                        }
```

to:

```rust
                        } else if let Some(params) = data.strip_prefix("RequestUpload=") {
                            self.handle_request_upload(params);
                        } else {
                            // Default to inline image handling
                            self.handle_iterm_image(&data);
                        }
```

**Step 2: Add handle_request_upload method**

Add a new method to `src/terminal/sequences/osc.rs` (at the end of the impl block, before the closing `}`):

```rust
    /// Handle OSC 1337 RequestUpload sequence
    ///
    /// Format: `OSC 1337 ; RequestUpload=format=tgz ST`
    ///
    /// The host is requesting the terminal to initiate a file upload.
    /// The terminal should show a file picker, then respond with:
    /// - "ok\n" followed by base64-encoded tgz data, or
    /// - Ctrl-C (0x03) to cancel
    fn handle_request_upload(&mut self, params_str: &str) {
        // Parse key=value pairs
        let mut format = String::from("tgz");
        for part in params_str.split(';') {
            if let Some((key, value)) = part.split_once('=') {
                if key == "format" {
                    format = value.to_string();
                }
            }
        }

        debug::log(
            debug::DebugLevel::Debug,
            "ITERM",
            &format!("RequestUpload received: format={}", format),
        );

        self.terminal_events
            .push(crate::terminal::TerminalEvent::UploadRequested { format });
    }
```

**Step 3: Add upload response methods to Terminal**

In `src/terminal/mod.rs`, in the file transfer API section you added in Task 3, add:

```rust
    /// Send upload data in response to an UploadRequested event.
    /// Writes "ok\n" followed by base64-encoded data to the response buffer.
    pub fn send_upload_data(&mut self, data: &[u8]) {
        use base64::Engine;
        // Write "ok\n" acknowledgment
        self.response_buffer.extend_from_slice(b"ok\n");
        // Base64-encode the data with line breaks (76 chars per line + \r)
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        self.response_buffer.extend_from_slice(encoded.as_bytes());
        // Terminate with double newline
        self.response_buffer.extend_from_slice(b"\n\n");
    }

    /// Cancel an upload request. Sends abort signal.
    pub fn cancel_upload(&mut self) {
        // Send Ctrl-C to abort
        self.response_buffer.push(0x03);
    }
```

**Step 4: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: PASS

**Step 5: Commit**

```bash
git add src/terminal/sequences/osc.rs src/terminal/mod.rs
git commit -m "feat: add RequestUpload handler and upload response API"
```

---

### Task 6: Update Python observer bindings for new events

**Files:**
- Modify: `src/python_bindings/observer.rs:18-229` (event_to_dict function)

**Step 1: Add new event conversions**

In `src/python_bindings/observer.rs`, in the `event_to_dict` function, before the closing `}` of the match block (line ~228), add:

```rust
        TerminalEvent::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            map.insert("type".to_string(), "file_transfer_started".to_string());
            map.insert("id".to_string(), id.to_string());
            let dir_str = match direction {
                crate::terminal::TransferDirection::Download => "download",
                crate::terminal::TransferDirection::Upload => "upload",
            };
            map.insert("direction".to_string(), dir_str.to_string());
            if let Some(name) = filename {
                map.insert("filename".to_string(), name.clone());
            }
            if let Some(total) = total_bytes {
                map.insert("total_bytes".to_string(), total.to_string());
            }
        }
        TerminalEvent::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => {
            map.insert("type".to_string(), "file_transfer_progress".to_string());
            map.insert("id".to_string(), id.to_string());
            map.insert(
                "bytes_transferred".to_string(),
                bytes_transferred.to_string(),
            );
            if let Some(total) = total_bytes {
                map.insert("total_bytes".to_string(), total.to_string());
            }
        }
        TerminalEvent::FileTransferCompleted { id, filename, size } => {
            map.insert("type".to_string(), "file_transfer_completed".to_string());
            map.insert("id".to_string(), id.to_string());
            if let Some(name) = filename {
                map.insert("filename".to_string(), name.clone());
            }
            map.insert("size".to_string(), size.to_string());
        }
        TerminalEvent::FileTransferFailed { id, reason } => {
            map.insert("type".to_string(), "file_transfer_failed".to_string());
            map.insert("id".to_string(), id.to_string());
            map.insert("reason".to_string(), reason.clone());
        }
        TerminalEvent::UploadRequested { format } => {
            map.insert("type".to_string(), "upload_requested".to_string());
            map.insert("format".to_string(), format.clone());
        }
```

**Step 2: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: PASS

**Step 3: Commit**

```bash
git add src/python_bindings/observer.rs
git commit -m "feat: add file transfer event conversions for Python observers"
```

---

### Task 7: Add Python terminal bindings for file transfer API

**Files:**
- Modify: `src/python_bindings/terminal.rs` (add new Python methods)

**Step 1: Add file transfer Python methods**

Add these methods to the `PyTerminal` impl block in `src/python_bindings/terminal.rs`. Place them near the end, after existing graphics methods. Find a suitable location:

```rust
    // === File Transfer API ===

    /// Get all active file transfers.
    ///
    /// Returns:
    ///     list[dict]: List of transfer dicts with keys: id, direction, filename,
    ///         status, bytes_transferred, total_bytes, started_at
    ///
    /// Example:
    ///     >>> transfers = terminal.get_active_transfers()
    ///     >>> for t in transfers:
    ///     ...     print(f"Transfer {t['id']}: {t['status']}")
    #[pyo3(text_signature = "($self)")]
    fn get_active_transfers(&self) -> Vec<HashMap<String, PyObject>> {
        Python::attach(|py| {
            self.terminal
                .get_active_transfers()
                .iter()
                .map(|t| transfer_to_py_dict(py, t, false))
                .collect()
        })
    }

    /// Get all completed file transfers.
    ///
    /// Returns:
    ///     list[dict]: List of completed transfer dicts
    ///
    /// Example:
    ///     >>> completed = terminal.get_completed_transfers()
    #[pyo3(text_signature = "($self)")]
    fn get_completed_transfers(&self) -> Vec<HashMap<String, PyObject>> {
        Python::attach(|py| {
            self.terminal
                .get_completed_transfers()
                .iter()
                .map(|t| transfer_to_py_dict(py, t, false))
                .collect()
        })
    }

    /// Get a file transfer by ID.
    ///
    /// Args:
    ///     transfer_id: The transfer ID to look up
    ///
    /// Returns:
    ///     dict | None: Transfer dict if found, None otherwise
    ///
    /// Example:
    ///     >>> t = terminal.get_transfer(42)
    ///     >>> if t: print(t['filename'])
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn get_transfer(&self, transfer_id: u64) -> Option<HashMap<String, PyObject>> {
        Python::attach(|py| {
            self.terminal
                .get_transfer(transfer_id)
                .map(|t| transfer_to_py_dict(py, t, false))
        })
    }

    /// Take a completed transfer, removing it from the completed list.
    /// Returns the transfer dict including the raw file data as bytes.
    ///
    /// Args:
    ///     transfer_id: The transfer ID to take
    ///
    /// Returns:
    ///     dict | None: Transfer dict with 'data' key containing bytes, or None
    ///
    /// Example:
    ///     >>> t = terminal.take_completed_transfer(42)
    ///     >>> if t:
    ///     ...     with open(t['filename'], 'wb') as f:
    ///     ...         f.write(t['data'])
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn take_completed_transfer(&mut self, transfer_id: u64) -> Option<HashMap<String, PyObject>> {
        Python::attach(|py| {
            self.terminal
                .take_completed_transfer(transfer_id)
                .map(|t| transfer_to_py_dict(py, &t, true))
        })
    }

    /// Cancel an active file transfer.
    ///
    /// Args:
    ///     transfer_id: The transfer ID to cancel
    ///
    /// Returns:
    ///     bool: True if transfer was found and cancelled
    ///
    /// Example:
    ///     >>> terminal.cancel_file_transfer(42)
    #[pyo3(text_signature = "($self, transfer_id)")]
    fn cancel_file_transfer(&mut self, transfer_id: u64) -> bool {
        self.terminal.cancel_file_transfer(transfer_id)
    }

    /// Send upload data in response to an upload_requested event.
    ///
    /// Args:
    ///     data: Raw file data bytes (will be base64-encoded for transmission)
    ///
    /// Example:
    ///     >>> with open('file.tar.gz', 'rb') as f:
    ///     ...     terminal.send_upload_data(f.read())
    #[pyo3(text_signature = "($self, data)")]
    fn send_upload_data(&mut self, data: &[u8]) {
        self.terminal.send_upload_data(data);
    }

    /// Cancel an upload request.
    ///
    /// Example:
    ///     >>> terminal.cancel_upload()
    #[pyo3(text_signature = "($self)")]
    fn cancel_upload(&mut self) {
        self.terminal.cancel_upload();
    }

    /// Set maximum file transfer size in bytes.
    ///
    /// Args:
    ///     max_bytes: Maximum allowed transfer size
    ///
    /// Example:
    ///     >>> terminal.set_max_transfer_size(100 * 1024 * 1024)  # 100MB
    #[pyo3(text_signature = "($self, max_bytes)")]
    fn set_max_transfer_size(&mut self, max_bytes: usize) {
        self.terminal.set_max_transfer_size(max_bytes);
    }

    /// Get maximum file transfer size in bytes.
    ///
    /// Returns:
    ///     int: Maximum allowed transfer size
    ///
    /// Example:
    ///     >>> limit = terminal.get_max_transfer_size()
    #[pyo3(text_signature = "($self)")]
    fn get_max_transfer_size(&self) -> usize {
        self.terminal.get_max_transfer_size()
    }
```

**Step 2: Add the helper function**

Add this helper function at the module level in `src/python_bindings/terminal.rs` (outside the impl block, near other helper functions):

```rust
/// Convert a FileTransfer to a Python dict
fn transfer_to_py_dict(
    py: Python<'_>,
    transfer: &crate::terminal::FileTransfer,
    include_data: bool,
) -> HashMap<String, PyObject> {
    let mut dict = HashMap::new();
    dict.insert("id".into(), transfer.id.into_pyobject(py).unwrap().into_any().unbind());
    let dir_str = match transfer.direction {
        crate::terminal::TransferDirection::Download => "download",
        crate::terminal::TransferDirection::Upload => "upload",
    };
    dict.insert("direction".into(), dir_str.into_pyobject(py).unwrap().into_any().unbind());
    dict.insert(
        "filename".into(),
        transfer
            .filename
            .as_ref()
            .map(|s| s.as_str())
            .into_pyobject(py)
            .unwrap()
            .into_any()
            .unbind(),
    );

    let (status_str, bytes_transferred, total_bytes) = match &transfer.status {
        crate::terminal::TransferStatus::Pending => ("pending", 0usize, None),
        crate::terminal::TransferStatus::InProgress {
            bytes_transferred,
            total_bytes,
        } => ("in_progress", *bytes_transferred, *total_bytes),
        crate::terminal::TransferStatus::Completed => {
            ("completed", transfer.data.len(), Some(transfer.data.len()))
        }
        crate::terminal::TransferStatus::Failed(_) => ("failed", transfer.data.len(), None),
        crate::terminal::TransferStatus::Cancelled => ("cancelled", transfer.data.len(), None),
    };
    dict.insert("status".into(), status_str.into_pyobject(py).unwrap().into_any().unbind());
    dict.insert(
        "bytes_transferred".into(),
        bytes_transferred.into_pyobject(py).unwrap().into_any().unbind(),
    );
    dict.insert(
        "total_bytes".into(),
        total_bytes.into_pyobject(py).unwrap().into_any().unbind(),
    );
    dict.insert("started_at".into(), transfer.started_at.into_pyobject(py).unwrap().into_any().unbind());
    dict.insert(
        "completed_at".into(),
        transfer.completed_at.into_pyobject(py).unwrap().into_any().unbind(),
    );

    if include_data {
        dict.insert("data".into(), pyo3::types::PyBytes::new(py, &transfer.data).into_any().unbind());
    }

    dict
}
```

**Step 3: Run tests**

Run: `cargo test --lib -- --nocapture 2>&1 | tail -20`
Expected: PASS

**Step 4: Commit**

```bash
git add src/python_bindings/terminal.rs
git commit -m "feat: add Python bindings for file transfer API"
```

---

### Task 8: Update streaming protocol (proto + conversions)

**Files:**
- Modify: `proto/terminal.proto` (new messages and event types)
- Modify: `src/streaming/protocol.rs` (new ServerMessage variants)
- Modify: `src/streaming/proto.rs` (conversions)
- Modify: `src/streaming/server.rs` (subscription filtering)

**Step 1: Add proto definitions**

In `proto/terminal.proto`, add new EventType values after `EVENT_TYPE_SNAPSHOT = 23;` (line ~49):

```protobuf
  EVENT_TYPE_FILE_TRANSFER = 24;
  EVENT_TYPE_UPLOAD_REQUEST = 25;
```

Add new messages to the ServerMessage oneof after `semantic_snapshot = 31;` (line ~88):

```protobuf
    FileTransferStarted file_transfer_started = 32;
    FileTransferProgress file_transfer_progress = 33;
    FileTransferCompleted file_transfer_completed = 34;
    FileTransferFailed file_transfer_failed = 35;
    UploadRequested upload_requested = 36;
```

Add new message definitions at the end of the file (before the closing comment or at the very end):

```protobuf
// File transfer started notification
message FileTransferStarted {
  uint64 id = 1;
  string direction = 2;                // "download" or "upload"
  optional string filename = 3;
  optional uint64 total_bytes = 4;
}

// File transfer progress update
message FileTransferProgress {
  uint64 id = 1;
  uint64 bytes_transferred = 2;
  optional uint64 total_bytes = 3;
}

// File transfer completed successfully
message FileTransferCompleted {
  uint64 id = 1;
  optional string filename = 2;
  uint64 size = 3;
}

// File transfer failed
message FileTransferFailed {
  uint64 id = 1;
  string reason = 2;
}

// Host requested file upload
message UploadRequested {
  string format = 1;
}
```

**Step 2: Regenerate protobuf Rust code**

Run: `make proto-rust`

**Step 3: Add ServerMessage variants to protocol.rs**

In `src/streaming/protocol.rs`, add new variants to the `ServerMessage` enum after `SemanticSnapshot` (line ~483):

```rust
    /// File transfer started
    #[serde(rename = "file_transfer_started")]
    FileTransferStarted {
        /// Unique transfer identifier
        id: u64,
        /// Direction: "download" or "upload"
        direction: String,
        /// Filename if known
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Total expected bytes if known
        #[serde(skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
    },

    /// File transfer progress update
    #[serde(rename = "file_transfer_progress")]
    FileTransferProgress {
        /// Transfer identifier
        id: u64,
        /// Bytes transferred so far
        bytes_transferred: u64,
        /// Total expected bytes if known
        #[serde(skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
    },

    /// File transfer completed
    #[serde(rename = "file_transfer_completed")]
    FileTransferCompleted {
        /// Transfer identifier
        id: u64,
        /// Filename if known
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Total bytes transferred
        size: u64,
    },

    /// File transfer failed
    #[serde(rename = "file_transfer_failed")]
    FileTransferFailed {
        /// Transfer identifier
        id: u64,
        /// Failure reason
        reason: String,
    },

    /// Host requested file upload
    #[serde(rename = "upload_requested")]
    UploadRequested {
        /// Upload format (e.g., "tgz")
        format: String,
    },
```

Add new EventType variants to the EventType enum:

```rust
    FileTransfer,
    UploadRequest,
```

Add constructor methods after the existing ones:

```rust
    /// Create a file transfer started message
    pub fn file_transfer_started(
        id: u64,
        direction: String,
        filename: Option<String>,
        total_bytes: Option<u64>,
    ) -> Self {
        Self::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        }
    }

    /// Create a file transfer progress message
    pub fn file_transfer_progress(id: u64, bytes_transferred: u64, total_bytes: Option<u64>) -> Self {
        Self::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        }
    }

    /// Create a file transfer completed message
    pub fn file_transfer_completed(id: u64, filename: Option<String>, size: u64) -> Self {
        Self::FileTransferCompleted { id, filename, size }
    }

    /// Create a file transfer failed message
    pub fn file_transfer_failed(id: u64, reason: String) -> Self {
        Self::FileTransferFailed { id, reason }
    }

    /// Create an upload requested message
    pub fn upload_requested(format: String) -> Self {
        Self::UploadRequested { format }
    }
```

**Step 4: Add proto.rs conversions**

In `src/streaming/proto.rs`, add app→proto conversions in the `to_proto_message` match (after the existing variants):

```rust
            AppServerMessage::FileTransferStarted {
                id,
                direction,
                filename,
                total_bytes,
            } => Some(Message::FileTransferStarted(pb::FileTransferStarted {
                id: *id,
                direction: direction.clone(),
                filename: filename.clone(),
                total_bytes: *total_bytes,
            })),
            AppServerMessage::FileTransferProgress {
                id,
                bytes_transferred,
                total_bytes,
            } => Some(Message::FileTransferProgress(pb::FileTransferProgress {
                id: *id,
                bytes_transferred: *bytes_transferred,
                total_bytes: *total_bytes,
            })),
            AppServerMessage::FileTransferCompleted { id, filename, size } => {
                Some(Message::FileTransferCompleted(pb::FileTransferCompleted {
                    id: *id,
                    filename: filename.clone(),
                    size: *size,
                }))
            }
            AppServerMessage::FileTransferFailed { id, reason } => {
                Some(Message::FileTransferFailed(pb::FileTransferFailed {
                    id: *id,
                    reason: reason.clone(),
                }))
            }
            AppServerMessage::UploadRequested { format } => {
                Some(Message::UploadRequested(pb::UploadRequested {
                    format: format.clone(),
                }))
            }
```

Add proto→app conversions in the `from_proto_message` match:

```rust
            Some(Message::FileTransferStarted(ft)) => Ok(AppServerMessage::FileTransferStarted {
                id: ft.id,
                direction: ft.direction,
                filename: ft.filename,
                total_bytes: ft.total_bytes,
            }),
            Some(Message::FileTransferProgress(ft)) => {
                Ok(AppServerMessage::FileTransferProgress {
                    id: ft.id,
                    bytes_transferred: ft.bytes_transferred,
                    total_bytes: ft.total_bytes,
                })
            }
            Some(Message::FileTransferCompleted(ft)) => {
                Ok(AppServerMessage::FileTransferCompleted {
                    id: ft.id,
                    filename: ft.filename,
                    size: ft.size,
                })
            }
            Some(Message::FileTransferFailed(ft)) => Ok(AppServerMessage::FileTransferFailed {
                id: ft.id,
                reason: ft.reason,
            }),
            Some(Message::UploadRequested(ur)) => Ok(AppServerMessage::UploadRequested {
                format: ur.format,
            }),
```

Add EventType conversions:

In app→proto:
```rust
            AppEventType::FileTransfer => pb::EventType::FileTransfer as i32,
            AppEventType::UploadRequest => pb::EventType::UploadRequest as i32,
```

In proto→app:
```rust
            pb::EventType::FileTransfer => AppEventType::FileTransfer,
            pb::EventType::UploadRequest => AppEventType::UploadRequest,
```

**Step 5: Update server.rs subscription filtering**

In `src/streaming/server.rs`, in the `should_send_message` function (line ~2843), before the always-send block, add:

```rust
        ServerMessage::FileTransferStarted { .. }
        | ServerMessage::FileTransferProgress { .. }
        | ServerMessage::FileTransferCompleted { .. }
        | ServerMessage::FileTransferFailed { .. } => subs.contains(&EventType::FileTransfer),
        ServerMessage::UploadRequested { .. } => subs.contains(&EventType::UploadRequest),
```

**Step 6: Run tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming -- --nocapture 2>&1 | tail -30`
Expected: PASS

**Step 7: Commit**

```bash
git add proto/terminal.proto src/streaming/terminal.pb.rs src/streaming/protocol.rs src/streaming/proto.rs src/streaming/server.rs
git commit -m "feat: add file transfer messages to streaming protocol"
```

---

### Task 9: Update Python streaming bindings

**Files:**
- Modify: `src/python_bindings/streaming.rs` (dict conversion for new ServerMessage variants)

**Step 1: Add dict→ServerMessage conversion**

In `src/python_bindings/streaming.rs`, find where other message types are matched by string name (e.g., `"graphics_added"`, `"badge_changed"`). Add new match arms:

```rust
        "file_transfer_started" => {
            let id = get_u64("id").unwrap_or(0);
            let direction = get_str("direction").unwrap_or_else(|| "download".to_string());
            let filename = get_str("filename");
            let total_bytes = get_u64("total_bytes");
            ServerMessage::file_transfer_started(id, direction, filename, total_bytes)
        }
        "file_transfer_progress" => {
            let id = get_u64("id").unwrap_or(0);
            let bytes_transferred = get_u64("bytes_transferred").unwrap_or(0);
            let total_bytes = get_u64("total_bytes");
            ServerMessage::file_transfer_progress(id, bytes_transferred, total_bytes)
        }
        "file_transfer_completed" => {
            let id = get_u64("id").unwrap_or(0);
            let filename = get_str("filename");
            let size = get_u64("size").unwrap_or(0);
            ServerMessage::file_transfer_completed(id, filename, size)
        }
        "file_transfer_failed" => {
            let id = get_u64("id").unwrap_or(0);
            let reason = get_str("reason").unwrap_or_default();
            ServerMessage::file_transfer_failed(id, reason)
        }
        "upload_requested" => {
            let format = get_str("format").unwrap_or_else(|| "tgz".to_string());
            ServerMessage::upload_requested(format)
        }
```

**Step 2: Add ServerMessage→dict conversion**

Find where other message types are converted to dicts (e.g., `ServerMessage::GraphicsAdded`). Add:

```rust
        ServerMessage::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            dict.set_item("type", "file_transfer_started")?;
            dict.set_item("id", id)?;
            dict.set_item("direction", direction)?;
            dict.set_item("filename", filename)?;
            dict.set_item("total_bytes", total_bytes)?;
        }
        ServerMessage::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => {
            dict.set_item("type", "file_transfer_progress")?;
            dict.set_item("id", id)?;
            dict.set_item("bytes_transferred", bytes_transferred)?;
            dict.set_item("total_bytes", total_bytes)?;
        }
        ServerMessage::FileTransferCompleted { id, filename, size } => {
            dict.set_item("type", "file_transfer_completed")?;
            dict.set_item("id", id)?;
            dict.set_item("filename", filename)?;
            dict.set_item("size", size)?;
        }
        ServerMessage::FileTransferFailed { id, reason } => {
            dict.set_item("type", "file_transfer_failed")?;
            dict.set_item("id", id)?;
            dict.set_item("reason", reason)?;
        }
        ServerMessage::UploadRequested { format } => {
            dict.set_item("type", "upload_requested")?;
            dict.set_item("format", format)?;
        }
```

**Step 3: Add EventType string conversion**

Find where event type strings are matched (e.g., `"graphics"`, `"badge"`). Add:

```rust
        "file_transfer" => EventType::FileTransfer,
        "upload_request" => EventType::UploadRequest,
```

And the reverse:

```rust
        EventType::FileTransfer => "file_transfer",
        EventType::UploadRequest => "upload_request",
```

**Step 4: Run tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming -- --nocapture 2>&1 | tail -20`
Expected: PASS

**Step 5: Commit**

```bash
git add src/python_bindings/streaming.rs
git commit -m "feat: add file transfer support to Python streaming bindings"
```

---

### Task 10: Add streaming protocol roundtrip tests

**Files:**
- Modify: `tests/test_streaming.rs` (add roundtrip tests for new message types)

**Step 1: Add roundtrip tests**

Add these tests to `tests/test_streaming.rs`:

```rust
#[test]
fn test_file_transfer_started_roundtrip() {
    let msg = ServerMessage::file_transfer_started(
        42,
        "download".into(),
        Some("test.pdf".into()),
        Some(1024),
    );
    let proto_bytes = server_message_to_proto(&msg);
    let decoded = proto_to_server_message(&proto_bytes).unwrap();
    match decoded {
        ServerMessage::FileTransferStarted {
            id,
            direction,
            filename,
            total_bytes,
        } => {
            assert_eq!(id, 42);
            assert_eq!(direction, "download");
            assert_eq!(filename, Some("test.pdf".into()));
            assert_eq!(total_bytes, Some(1024));
        }
        other => panic!("Expected FileTransferStarted, got {:?}", other),
    }
}

#[test]
fn test_file_transfer_progress_roundtrip() {
    let msg = ServerMessage::file_transfer_progress(42, 512, Some(1024));
    let proto_bytes = server_message_to_proto(&msg);
    let decoded = proto_to_server_message(&proto_bytes).unwrap();
    match decoded {
        ServerMessage::FileTransferProgress {
            id,
            bytes_transferred,
            total_bytes,
        } => {
            assert_eq!(id, 42);
            assert_eq!(bytes_transferred, 512);
            assert_eq!(total_bytes, Some(1024));
        }
        other => panic!("Expected FileTransferProgress, got {:?}", other),
    }
}

#[test]
fn test_file_transfer_completed_roundtrip() {
    let msg = ServerMessage::file_transfer_completed(42, Some("test.pdf".into()), 1024);
    let proto_bytes = server_message_to_proto(&msg);
    let decoded = proto_to_server_message(&proto_bytes).unwrap();
    match decoded {
        ServerMessage::FileTransferCompleted { id, filename, size } => {
            assert_eq!(id, 42);
            assert_eq!(filename, Some("test.pdf".into()));
            assert_eq!(size, 1024);
        }
        other => panic!("Expected FileTransferCompleted, got {:?}", other),
    }
}

#[test]
fn test_file_transfer_failed_roundtrip() {
    let msg = ServerMessage::file_transfer_failed(42, "size exceeded".into());
    let proto_bytes = server_message_to_proto(&msg);
    let decoded = proto_to_server_message(&proto_bytes).unwrap();
    match decoded {
        ServerMessage::FileTransferFailed { id, reason } => {
            assert_eq!(id, 42);
            assert_eq!(reason, "size exceeded");
        }
        other => panic!("Expected FileTransferFailed, got {:?}", other),
    }
}

#[test]
fn test_upload_requested_roundtrip() {
    let msg = ServerMessage::upload_requested("tgz".into());
    let proto_bytes = server_message_to_proto(&msg);
    let decoded = proto_to_server_message(&proto_bytes).unwrap();
    match decoded {
        ServerMessage::UploadRequested { format } => {
            assert_eq!(format, "tgz");
        }
        other => panic!("Expected UploadRequested, got {:?}", other),
    }
}
```

**Step 2: Run streaming tests**

Run: `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming test_file_transfer -- --nocapture`
Expected: All PASS

**Step 3: Commit**

```bash
git add tests/test_streaming.rs
git commit -m "test: add streaming roundtrip tests for file transfer messages"
```

---

### Task 11: Add Python integration tests

**Files:**
- Create: `tests/test_file_transfer.py`

**Step 1: Write Python tests**

```python
"""Tests for file transfer support (OSC 1337 File= protocol)."""

import base64

import par_term_emu_core_rust as core


def make_osc1337_file(
    data: bytes,
    filename: str | None = None,
    inline: int = 0,
) -> bytes:
    """Build an OSC 1337 File= sequence."""
    b64_data = base64.b64encode(data).decode("ascii")
    params = [f"inline={inline}", f"size={len(data)}"]
    if filename:
        b64_name = base64.b64encode(filename.encode()).decode("ascii")
        params.append(f"name={b64_name}")
    params_str = ";".join(params)
    return f"\x1b]1337;File={params_str}:{b64_data}\x07".encode()


class TestFileTransferBasics:
    """Test basic file transfer operations."""

    def test_default_max_transfer_size(self) -> None:
        term = core.Terminal(80, 24)
        # Default is 50MB
        assert term.get_max_transfer_size() == 50 * 1024 * 1024

    def test_set_max_transfer_size(self) -> None:
        term = core.Terminal(80, 24)
        term.set_max_transfer_size(100 * 1024 * 1024)
        assert term.get_max_transfer_size() == 100 * 1024 * 1024

    def test_no_active_transfers_initially(self) -> None:
        term = core.Terminal(80, 24)
        assert term.get_active_transfers() == []
        assert term.get_completed_transfers() == []


class TestFileDownload:
    """Test inline=0 file download handling."""

    def test_single_file_download_events(self) -> None:
        """Single-sequence File= with inline=0 should emit transfer events."""
        term = core.Terminal(80, 24)
        data = b"Hello, World!"
        seq = make_osc1337_file(data, filename="hello.txt", inline=0)
        term.process(seq)

        events = term.poll_events()
        event_types = [e["type"] for e in events]
        assert "file_transfer_started" in event_types
        assert "file_transfer_completed" in event_types

        # Find the completed event
        completed = [e for e in events if e["type"] == "file_transfer_completed"][0]
        assert completed["filename"] == "hello.txt"
        assert completed["size"] == str(len(data))

    def test_take_completed_transfer_returns_data(self) -> None:
        """take_completed_transfer should return file bytes."""
        term = core.Terminal(80, 24)
        data = b"file content here"
        seq = make_osc1337_file(data, filename="test.bin", inline=0)
        term.process(seq)

        events = term.poll_events()
        started = [e for e in events if e["type"] == "file_transfer_started"][0]
        transfer_id = int(started["id"])

        transfer = term.take_completed_transfer(transfer_id)
        assert transfer is not None
        assert transfer["data"] == data
        assert transfer["filename"] == "test.bin"
        assert transfer["status"] == "completed"

        # Should be gone after taking
        assert term.take_completed_transfer(transfer_id) is None

    def test_inline_1_still_works(self) -> None:
        """inline=1 should still go through the image path (regression)."""
        term = core.Terminal(80, 24)
        # Create a tiny 1x1 PNG
        import struct
        import zlib

        def make_1x1_png() -> bytes:
            signature = b"\x89PNG\r\n\x1a\n"
            # IHDR
            ihdr_data = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
            ihdr_crc = zlib.crc32(b"IHDR" + ihdr_data) & 0xFFFFFFFF
            ihdr = struct.pack(">I", 13) + b"IHDR" + ihdr_data + struct.pack(">I", ihdr_crc)
            # IDAT
            raw = zlib.compress(b"\x00\xff\x00\x00")
            idat_crc = zlib.crc32(b"IDAT" + raw) & 0xFFFFFFFF
            idat = struct.pack(">I", len(raw)) + b"IDAT" + raw + struct.pack(">I", idat_crc)
            # IEND
            iend_crc = zlib.crc32(b"IEND") & 0xFFFFFFFF
            iend = struct.pack(">I", 0) + b"IEND" + struct.pack(">I", iend_crc)
            return signature + ihdr + idat + iend

        png_data = make_1x1_png()
        seq = make_osc1337_file(png_data, filename="img.png", inline=1)
        term.process(seq)

        # Should NOT produce file transfer events
        events = term.poll_events()
        transfer_events = [e for e in events if e["type"].startswith("file_transfer")]
        assert len(transfer_events) == 0

        # Should have added a graphic
        assert term.graphics_count() >= 1


class TestUploadRequest:
    """Test RequestUpload handling."""

    def test_request_upload_event(self) -> None:
        """RequestUpload=format=tgz should emit upload_requested event."""
        term = core.Terminal(80, 24)
        seq = b"\x1b]1337;RequestUpload=format=tgz\x07"
        term.process(seq)

        events = term.poll_events()
        upload_events = [e for e in events if e["type"] == "upload_requested"]
        assert len(upload_events) == 1
        assert upload_events[0]["format"] == "tgz"

    def test_send_upload_data(self) -> None:
        """send_upload_data should write response to output buffer."""
        term = core.Terminal(80, 24)
        term.send_upload_data(b"test data")
        response = term.read_response()
        # Should contain "ok\n" prefix
        assert response.startswith(b"ok\n")

    def test_cancel_upload(self) -> None:
        """cancel_upload should write Ctrl-C to output."""
        term = core.Terminal(80, 24)
        term.cancel_upload()
        response = term.read_response()
        assert b"\x03" in response


class TestCancelTransfer:
    """Test transfer cancellation."""

    def test_cancel_nonexistent_returns_false(self) -> None:
        term = core.Terminal(80, 24)
        assert term.cancel_file_transfer(9999) is False
```

**Step 2: Build and run Python tests**

Run: `make dev && uv run pytest tests/test_file_transfer.py -v`
Expected: All PASS

**Step 3: Commit**

```bash
git add tests/test_file_transfer.py
git commit -m "test: add Python integration tests for file transfer"
```

---

### Task 12: Update documentation

**Files:**
- Modify: `docs/API_REFERENCE.md` (add file transfer section)
- Modify: `README.md` (update features list)
- Modify: `docs/CHANGELOG.md` (add entry)

**Step 1: Add file transfer section to API_REFERENCE.md**

Find the appropriate location in `docs/API_REFERENCE.md` and add a new section for file transfer methods. Document all new Python methods with signatures, args, returns, and examples.

**Step 2: Update README.md features**

Add "General-purpose file transfer (OSC 1337 File=)" to the features list.

**Step 3: Update CHANGELOG.md**

Add an entry under the appropriate version heading.

**Step 4: Commit**

```bash
git add docs/API_REFERENCE.md README.md docs/CHANGELOG.md
git commit -m "docs: add file transfer API documentation"
```

---

### Task 13: Run full verification and fix issues

**Step 1: Regenerate protos and rebuild frontend**

Run: `make proto-rust && make proto-typescript && make web-build-static`

**Step 2: Run full verification**

Run: `make checkall`
Expected: All checks pass

**Step 3: Fix any issues**

If any checks fail, fix them and re-run `make checkall` until clean.

**Step 4: Final commit if needed**

```bash
git add -A
git commit -m "fix: resolve checkall issues for file transfer feature"
```
