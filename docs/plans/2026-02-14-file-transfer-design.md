# General-Purpose File Transfer Support (OSC 1337)

**Issue**: #45
**Date**: 2026-02-14
**Approach**: Minimal FileTransfer Manager (Approach 1)

## Overview

Extend the existing iTerm2 inline image transfer implementation to support general-purpose file upload/download via the OSC 1337 `File=` protocol. The library emits events for file transfer lifecycle; frontends decide where/whether to save files.

Implements all three iTerm2 file transfer capabilities:
- **Download** (`inline=0`): Host sends file to terminal, frontend receives raw bytes via events
- **Inline display** (`inline=1`): Existing image path, unchanged
- **Upload** (`RequestUpload=format=tgz`): Host requests files, frontend shows picker and responds

## Core Data Model

New file: `src/terminal/file_transfer.rs`

### Types

- `TransferId` (u64): Unique transfer identifier
- `TransferDirection`: `Download` | `Upload`
- `TransferStatus`: `Pending` | `InProgress { bytes_transferred, total_bytes }` | `Completed` | `Failed(String)` | `Cancelled`
- `FileTransfer`: Full transfer state (id, direction, filename, status, data bytes, params, timestamps)
- `FileTransferManager`: Manages active + completed transfers with bounded ring buffer (default 32 completed, 50MB max size)

### Key Decisions

- `FileTransfer.data` stores raw decoded bytes, not base64
- Completed transfers kept in bounded ring buffer for frontend retrieval
- `max_transfer_size` is separate from graphics limits
- Original OSC params preserved for frontend inspection

## TerminalEvent Variants

Five new variants added to `TerminalEvent`:

| Variant | When Emitted | Data |
|---------|-------------|------|
| `FileTransferStarted` | File= or MultipartFile= received with inline=0 | id, direction, filename, total_bytes |
| `FileTransferProgress` | Each chunk in multipart download | id, bytes_transferred, total_bytes |
| `FileTransferCompleted` | Transfer finished successfully | id, filename, size |
| `FileTransferFailed` | Decode error, size exceeded, cancelled | id, reason |
| `UploadRequested` | RequestUpload= received | format |

All route to `on_screen_event()` in the observer system.

### Download Flow

1. `FileTransferStarted` emitted when File= received with `inline=0`
2. `FileTransferProgress` emitted per chunk (multipart only)
3. `FileTransferCompleted` emitted when done
4. Frontend calls `take_completed_transfer(id)` to get raw bytes
5. Frontend decides save location

### Upload Flow

1. `UploadRequested` emitted when `RequestUpload=format=tgz` received
2. Frontend shows file picker, builds tgz archive
3. Frontend calls `send_upload_data(data)` (writes `ok\n` + base64 to PTY) or `cancel_upload()` (writes abort)

## Terminal API

### New Public Methods

```rust
// Query
fn get_active_transfers(&self) -> Vec<&FileTransfer>;
fn get_completed_transfers(&self) -> Vec<&FileTransfer>;
fn get_transfer(&self, id: TransferId) -> Option<&FileTransfer>;

// Retrieve & consume
fn take_completed_transfer(&mut self, id: TransferId) -> Option<FileTransfer>;

// Control
fn cancel_transfer(&mut self, id: TransferId) -> bool;
fn send_upload_data(&mut self, data: &[u8]);
fn cancel_upload(&mut self);

// Config
fn set_max_transfer_size(&mut self, bytes: usize);
fn get_max_transfer_size(&self) -> usize;
```

## Handler Routing Changes

Existing `handle_iterm_image()` entry point unchanged. Routing splits:

```
handle_iterm_image(data)
  ├─ MultipartFile=... → handle_multipart_file_start(params)
  │   ├─ inline=1 → existing image path (unchanged)
  │   └─ inline=0 → FileTransferManager.start_download(), emit FileTransferStarted
  │
  ├─ FilePart=... → handle_file_part(chunk)
  │   └─ routes based on ITermMultipartState.is_file_transfer flag
  │
  └─ File=...:... → handle_single_file_transfer(data)
      ├─ inline=1 → existing image decode (unchanged)
      └─ inline=0 → decode base64, store in FileTransfer, emit Started+Completed
```

New OSC 1337 route:
```
"RequestUpload=..." → handle_request_upload(params), emit UploadRequested
```

### ITermMultipartState Change

Add `is_file_transfer: bool` field. When `true`, `handle_file_part()` routes chunks to `FileTransferManager` instead of graphics pipeline.

## Python Bindings

### New Methods on PyTerminal

```python
# Query
get_active_transfers() -> list[dict]
get_completed_transfers() -> list[dict]
get_transfer(id: int) -> dict | None

# Retrieve & consume
take_completed_transfer(id: int) -> dict | None  # includes "data": bytes

# Control
cancel_transfer(id: int) -> bool
send_upload_data(data: bytes) -> None
cancel_upload() -> None

# Config
set_max_transfer_size(bytes: int) -> None
get_max_transfer_size() -> int
```

### Observer Event Dicts

- `file_transfer_started`: id, direction, filename, total_bytes
- `file_transfer_progress`: id, bytes_transferred, total_bytes
- `file_transfer_completed`: id, filename, size
- `file_transfer_failed`: id, reason
- `upload_requested`: format

## Streaming Protocol

### New Proto Messages

```protobuf
message FileTransferStarted { id, direction, filename?, total_bytes? }
message FileTransferProgress { id, bytes_transferred, total_bytes? }
message FileTransferCompleted { id, filename?, size }
message FileTransferFailed { id, reason }
message UploadRequested { format }
```

### New EventType Enum Values

```
EVENT_TYPE_FILE_TRANSFER_STARTED = 20
EVENT_TYPE_FILE_TRANSFER_PROGRESS = 21
EVENT_TYPE_FILE_TRANSFER_COMPLETED = 22
EVENT_TYPE_FILE_TRANSFER_FAILED = 23
EVENT_TYPE_UPLOAD_REQUESTED = 24
```

With corresponding conversions in `protocol.rs` and `proto.rs`.

## Testing

### Rust Unit Tests (file_transfer.rs)
- FileTransferManager lifecycle (start/progress/complete/cancel)
- Size limit enforcement
- Ring buffer eviction
- take_completed_transfer removes and returns

### Rust Integration Tests (graphics.rs)
- inline=0 single transfer emits events and stores bytes
- inline=0 multipart transfer routes chunks correctly
- inline=1 path unchanged (regression)
- RequestUpload OSC parsing

### Streaming Tests (test_streaming.rs)
- Roundtrip serialization for all 5 new event types

### Python Tests (test_file_transfer.py)
- Transfer lifecycle via Python API
- Observer events
- take_completed_transfer returns bytes
- Upload request/response
- Config methods

## Files Modified

| File | Change |
|------|--------|
| `src/terminal/file_transfer.rs` | **New** — FileTransfer, FileTransferManager |
| `src/terminal/mod.rs` | Add TerminalEvent/Kind variants, FileTransferManager field, ITermMultipartState.is_file_transfer |
| `src/terminal/graphics.rs` | Route inline=0 to FileTransferManager |
| `src/terminal/sequences/osc.rs` | Add RequestUpload handler |
| `src/graphics/iterm.rs` | Remove inline=1 hard requirement from parse_params |
| `src/observer.rs` | Route new events to Screen category |
| `src/python_bindings/terminal.rs` | New Python methods |
| `src/python_bindings/observer.rs` | New event dict conversions |
| `src/python_bindings/types.rs` | PyFileTransfer type (if needed) |
| `proto/terminal.proto` | New messages and event types |
| `src/streaming/protocol.rs` | New ServerMessage variants |
| `src/streaming/proto.rs` | Proto ↔ app conversions |
| `src/python_bindings/streaming.rs` | New event type handling |
| `tests/test_streaming.rs` | New roundtrip tests |
| `tests/test_file_transfer.py` | **New** — Python integration tests |
| `docs/API_REFERENCE.md` | Document new methods |
| `README.md` | Update features |
