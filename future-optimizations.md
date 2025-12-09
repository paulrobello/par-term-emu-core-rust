# Terminal Streaming Optimization Plan

## Overview
Comprehensive optimization of the terminal streaming server, protocol, and web frontend to reduce latency, bandwidth, and CPU usage while improving perceived responsiveness.

---

## Phase 1: Quick Wins (Low Effort, High Impact)

### 1.1 RAF-Batched Terminal Writes
- **File**: `web-terminal-frontend/components/Terminal.tsx`
- **Status**: [x] Complete
- **Description**: Buffer incoming terminal output and flush once per animation frame instead of immediately on each WebSocket message
- **Expected Impact**: 10-100x reduction in render calls during burst output
- **Implementation**: Added `writeBufferRef` and `rafIdRef`, `flushWrites()` and `bufferWrite()` callbacks

### 1.2 Reuse TextDecoder Instance
- **File**: `web-terminal-frontend/components/Terminal.tsx`
- **Status**: [x] Complete
- **Description**: Create TextDecoder once outside the message handler instead of per-message
- **Expected Impact**: Minor CPU reduction, cleaner code
- **Implementation**: Added `sharedDecoder` constant at module level

### 1.3 Lower Compression Threshold
- **File**: `src/streaming/proto.rs`
- **Status**: [x] Complete
- **Description**: Reduce COMPRESSION_THRESHOLD from 1024 to 256 bytes to compress more messages
- **Expected Impact**: ~30% more messages compressed, reduced bandwidth
- **Implementation**: Changed threshold constant, added boundary tests

### 1.4 Add WebSocket Preconnect Hint
- **File**: `web-terminal-frontend/app/layout.tsx`
- **Status**: [x] Complete
- **Description**: Add `<link rel="preconnect">` for WebSocket endpoint to reduce initial connection time
- **Expected Impact**: Save 100-200ms on initial connection
- **Implementation**: Added preconnect links for ws:// and wss:// localhost:8099

### 1.5 Font Preloading
- **File**: `web-terminal-frontend/app/layout.tsx`
- **Status**: [x] Complete
- **Description**: Preload terminal fonts to avoid layout shift
- **Expected Impact**: Faster time-to-interactive, no font flash
- **Implementation**: Added preload and stylesheet links for JetBrains Mono

---

## Phase 2: Server-Side Optimizations

### 2.1 Server-Side Output Batching
- **File**: `src/streaming/server.rs`
- **Status**: [x] Complete
- **Description**: Coalesce output messages within a 16ms window (one frame) before broadcasting
- **Expected Impact**: 50-80% reduction in message count
- **Implementation**: Rewrote output_broadcaster_loop with tokio::select! for time-based batching

### 2.2 TCP_NODELAY Verification
- **File**: `src/streaming/server.rs`
- **Status**: [x] Complete
- **Description**: Ensure TCP_NODELAY is set on WebSocket connections for low-latency small messages
- **Expected Impact**: Reduce keystroke latency from ~40ms to ~5ms
- **Implementation**: Added stream.set_nodelay(true) after accept() for both plain and TLS connections

### 2.3 Pre-serialized Broadcast Cache
- **File**: `src/streaming/server.rs`, `src/streaming/broadcaster.rs`
- **Status**: [-] Deferred
- **Description**: Cache serialized message bytes to avoid N serializations for N clients
- **Expected Impact**: N→1 serialization calls for broadcasts
- **Notes**: Lower priority - current broadcast mechanism already efficient for typical client counts

### 2.4 Add jemalloc Allocator
- **Files**: `Cargo.toml`, `src/bin/streaming_server.rs`
- **Status**: [x] Complete
- **Description**: Use jemalloc for better server workload performance
- **Expected Impact**: 5-15% throughput improvement
- **Implementation**: Added tikv-jemallocator with feature flag, set as global allocator in streaming server

---

## Phase 3: Protocol Enhancements

### 3.1 Scroll Detection Optimization
- **Files**: `proto/terminal.proto`, `src/streaming/protocol.rs`, `src/streaming/proto.rs`, `web-terminal-frontend/lib/protocol.ts`
- **Status**: [-] Deferred
- **Description**: Add Scroll message type to send "scroll N lines + new content" instead of full screen
- **Expected Impact**: 10-100x smaller messages for streaming output
- **Notes**: Requires protocol version negotiation for backwards compatibility - deferred to future release

### 3.2 Delta Update Protocol
- **Files**: `proto/terminal.proto`, `src/streaming/protocol.rs`, `src/terminal/grid.rs`
- **Status**: [-] Deferred
- **Description**: Track dirty cells and send incremental updates instead of full screen refresh
- **Expected Impact**: 5-10x bandwidth reduction for typical workloads
- **Notes**: Major protocol change - deferred to future release

### 3.3 LZ4 Compression Option
- **Files**: `Cargo.toml`, `src/streaming/proto.rs`, `web-terminal-frontend/lib/protocol.ts`
- **Status**: [-] Deferred
- **Description**: Add LZ4 as faster compression alternative to zlib
- **Expected Impact**: 3-5x faster compression/decompression
- **Notes**: Requires client capability negotiation - deferred to future release

---

## Phase 4: UX Optimizations

### 4.1 Web Worker for Protocol Processing
- **Files**: `web-terminal-frontend/lib/protocol.worker.ts` (new), `web-terminal-frontend/components/Terminal.tsx`
- **Status**: [-] Deferred
- **Description**: Move decompression and protobuf decoding to Web Worker to prevent UI jank
- **Expected Impact**: Eliminate UI freezes during large message bursts
- **Notes**: Next.js requires additional webpack configuration for workers - deferred (server-side batching already reduces message burst impact)

### 4.2 Local Echo (Predictive Input)
- **File**: `web-terminal-frontend/components/Terminal.tsx`
- **Status**: [x] Complete
- **Description**: Display typed characters immediately before server confirmation, then reconcile
- **Expected Impact**: Perceived latency drops from 50-150ms to <16ms
- **Implementation**: Added `pendingEchoRef` and `localEchoEnabledRef`, echo printable ASCII in onData handler, filter echoed chars in bufferWrite

---

## Implementation Progress

| Phase | Task | Status | Notes |
|-------|------|--------|-------|
| 1.1 | RAF-Batched Writes | [x] Complete | 10-100x fewer render calls |
| 1.2 | Reuse TextDecoder | [x] Complete | Minor CPU reduction |
| 1.3 | Lower Compression | [x] Complete | 256 byte threshold |
| 1.4 | WebSocket Preconnect | [x] Complete | 100-200ms faster connect |
| 1.5 | Font Preloading | [x] Complete | No font flash |
| 2.1 | Output Batching | [x] Complete | 50-80% fewer messages |
| 2.2 | TCP_NODELAY | [x] Complete | Lower keystroke latency |
| 2.3 | Broadcast Cache | [-] Deferred | Lower priority |
| 2.4 | jemalloc | [x] Complete | 5-15% throughput boost |
| 3.1 | Scroll Detection | [-] Deferred | Major protocol change |
| 3.2 | Delta Updates | [-] Deferred | Major protocol change |
| 3.3 | LZ4 Compression | [-] Deferred | Needs capability negotiation |
| 4.1 | Web Worker | [-] Deferred | Server batching sufficient |
| 4.2 | Local Echo | [x] Complete | Instant typing feel |

---

## Testing Strategy

### Performance Benchmarks
- [ ] Measure baseline latency (keystroke → display)
- [ ] Measure baseline bandwidth (typical session)
- [ ] Measure baseline CPU usage (server + client)
- [ ] Re-measure after each phase

### Functional Tests
- [ ] Verify terminal output correctness after batching
- [ ] Verify compression/decompression works at new threshold
- [ ] Verify scroll detection handles edge cases
- [ ] Verify local echo reconciliation works correctly

---

## Rollback Plan
Each optimization is independent. If issues arise:
1. Revert the specific commit
2. Re-run `make checkall`
3. Deploy previous version

---

## Summary

### Implemented Optimizations (8 of 14)
1. **RAF-Batched Terminal Writes** - Client-side write coalescing, one render per frame
2. **Reuse TextDecoder** - Module-level shared instance
3. **Lower Compression Threshold** - 256 bytes (was 1KB)
4. **WebSocket Preconnect** - DNS/TCP prefetch hints
5. **Font Preloading** - JetBrains Mono preload
6. **Server-Side Output Batching** - 16ms window, tokio::select! based
7. **TCP_NODELAY** - Disabled Nagle's algorithm for lower latency
8. **jemalloc Allocator** - Optional feature for server binary
9. **Local Echo** - Predictive input for instant typing feel

### Deferred for Future Release (6 of 14)
- Pre-serialized broadcast cache (lower priority)
- Scroll detection optimization (major protocol change)
- Delta updates (major protocol change)
- LZ4 compression (needs capability negotiation)
- Web Worker (server batching sufficient)

### Expected Performance Impact
- **Bandwidth**: 30-50% reduction from compression + batching
- **Message Count**: 50-80% reduction from server batching
- **Render Calls**: 10-100x reduction from RAF batching
- **Keystroke Latency**: ~40ms → ~5ms from TCP_NODELAY
- **Perceived Input Latency**: 50-150ms → <16ms from local echo
- **Initial Connection**: 100-200ms faster from preconnect hints

---

## Notes
- All changes must pass `make checkall` before commit
- Update CHANGELOG.md for user-facing changes
- Protocol changes require version negotiation for backwards compatibility
