# Project Audit Report: par-term-emu-core-rust

**Date**: 2026-02-15
**Version Audited**: 0.38.0
**Codebase Size**: ~80,000 lines of Rust, ~9,300 lines of Python tests, 17 documentation files

---

## Executive Summary

par-term-emu-core-rust is a well-architected Rust terminal emulator library with Python bindings, demonstrating strong engineering practices overall. The codebase has comprehensive VT sequence support, a clean module structure, and good test coverage for core terminal functionality. However, the audit identified **1 critical**, **10 high**, **15 medium**, and **12 low** severity findings that should be addressed. The most urgent issues involve timing-attack vulnerabilities in the streaming server's authentication, missing input validation for DoS vectors, and significant test coverage gaps in the streaming infrastructure.

---

## Table of Contents

1. [Architecture Review](#1-architecture-review)
2. [Design Patterns](#2-design-patterns)
3. [Security Assessment](#3-security-assessment)
4. [Code Quality](#4-code-quality)
5. [Test Coverage](#5-test-coverage)
6. [Documentation Review](#6-documentation-review)
7. [Prioritized Remediation Plan](#7-prioritized-remediation-plan)

---

## 1. Architecture Review

### 1.1 Overall Structure

The project is cleanly decomposed into well-separated modules:

| Module | Lines | Purpose |
|--------|------:|---------|
| `terminal/` | 26,315 | Core VT emulation, state, sequence handling |
| `python_bindings/` | 18,326 | PyO3 Python API wrappers |
| `streaming/` | 8,441 | WebSocket server, protobuf protocol |
| `screenshot/` | 4,835 | Terminal-to-image rendering |
| `graphics/` | 4,436 | Sixel/iTerm2/Kitty graphics |
| `grid/` | 2,726 | 2D terminal buffer |
| Root modules | 12,522 | PTY, cursor, color, mouse, utilities |

**Verdict**: Good separation of concerns. Each module has a clear responsibility.

### 1.2 Strengths

- **Clean data flow**: Input bytes -> VTE parser -> Perform callbacks -> Terminal state -> Python API. This pipeline is well-defined and consistent.
- **Feature flags**: Proper use of Cargo features (`python`, `streaming`, `rust-only`, `jemalloc`) to control compilation scope.
- **Multi-artifact crate**: Produces Python extension (cdylib), Rust library (rlib), and streaming server binary cleanly.
- **Streaming protocol layers**: 3-layer design (proto -> protocol.rs -> proto.rs) provides good abstraction between wire format and application types.
- **CI/CD**: 7 GitHub Actions workflows with version consistency checks, multi-OS testing (Linux/macOS/Windows), and multi-Python-version support (3.12/3.13/3.14).
- **Version sync**: All three version locations (Cargo.toml, pyproject.toml, `__init__.py`) validated by CI.

### 1.3 Concerns

#### [A-1] God Object: Terminal Struct (Medium)
**File**: `src/terminal/mod.rs`, lines 164-495

The `Terminal` struct has **156+ fields** spanning grid state, cursor, colors, 60+ mode flags, event queues, graphics state, recording, macros, clipboard, notifications, triggers, metrics, search, shell integration, file transfers, and multiplexing.

**Risk**: Hard to reason about state consistency; high cognitive load; difficult to test features in isolation.

**Recommendation**: Extract sub-components:
- `TerminalGridState` (grid, cursor, scroll regions)
- `TerminalModeState` (mode flags)
- `TerminalGraphicsState` (all graphics protocols)
- `TerminalEventState` (events, observers, notifications)

This is a long-term refactor but would significantly improve maintainability.

#### [A-2] Rust Edition (Low)
**File**: `Cargo.toml`

Currently uses Rust edition 2021. Edition 2024 is the latest stable and could unlock newer language features. Not urgent but worth planning.

#### [A-3] Large Source Files (Low)
Several files exceed recommended size:
- `python_bindings/terminal/mod.rs`: ~9,000 lines
- `terminal/mod.rs`: ~7,500 lines (already being split based on git status)
- `streaming/proto.rs`: ~2,000 lines
- `streaming/server.rs`: ~1,800 lines

The ongoing refactoring visible in git status (splitting `grid.rs`, `terminal.rs`, CSI/OSC/DCS handlers) is moving in the right direction.

---

## 2. Design Patterns

### 2.1 Patterns in Use

| Pattern | Location | Assessment |
|---------|----------|------------|
| **Builder** | `ScreenshotConfig`, `StreamingConfig` | Good - clean configuration |
| **Observer** | `terminal/event.rs`, `observer.rs` | Good - decoupled event system |
| **Strategy** | `screenshot/formats/` | Good - pluggable image formats |
| **Facade** | `python_bindings/` | Good - thin wrappers over Rust API |
| **Singleton (lazy)** | `font_cache.rs` | Acceptable - embedded font loading |
| **State Machine** | VTE parser + Terminal modes | Good - well-structured state transitions |
| **Protocol Buffers** | `streaming/proto.rs` | Good - versioned wire format |

### 2.2 Pattern Concerns

#### [D-1] Observer Dispatch Race (Medium)
**File**: `src/terminal/mod.rs`, lines 2073-2108

`dispatch_events()` iterates over `terminal_events` without draining them. If `process()` is called multiple times without `poll_events()`, observers receive duplicate events. The code has a comment acknowledging this, but it should be formally resolved.

**Recommendation**: Use separate queues for observer dispatch vs. poll-based access, or add a `dispatched` flag to events.

#### [D-2] Lock Strategy Inconsistency (Low)
**Files**: `pty_session.rs` vs `streaming/`

Uses `parking_lot::Mutex` in streaming layer but `std::sync::Mutex` in the PTY session. While both work, `parking_lot::Mutex` is preferred throughout the codebase for no-poisoning semantics.

**Recommendation**: Standardize on `parking_lot::Mutex` everywhere, or document why `std::sync::Mutex` is used in the PTY path.

---

## 3. Security Assessment

### 3.1 Critical

#### [S-1] Timing Attack in API Key Authentication (Critical)
**File**: `src/streaming/server.rs`, lines 2907, 2915, 2924

```rust
if bearer_token.trim() == expected_key {  // Non-constant-time comparison
    return next.run(req).await;
}
```

All three API key comparison paths use standard `==`, enabling timing attacks. An attacker can measure response time to brute-force the key character-by-character.

**Fix**: Add `subtle` crate dependency and use `ct_eq()`:
```rust
use subtle::ConstantTimeEq;
if bool::from(bearer_token.trim().as_bytes().ct_eq(expected_key.as_bytes())) {
    return next.run(req).await;
}
```

### 3.2 High

#### [S-2] Non-Constant-Time Password Comparison (High)
**File**: `src/streaming/server.rs`, lines 237, 242

Clear-text password comparison and username comparison use non-constant-time `==`.

**Fix**: Apply `ct_eq()` to username and clear-text password comparisons.

#### [S-3] API Key in Query Parameters (High)
**File**: `src/streaming/server.rs`, lines 2920-2928

API keys passed via `?api_key=` are logged by proxies/firewalls, saved in browser history, and leaked via Referer headers. While documented, this should be disabled by default.

**Fix**: Add `allow_api_key_in_query: bool` config option, default `false`.

#### [S-4] Coprocess Command Injection (High)
**File**: `src/coprocess.rs`, lines 131-150

No validation of `config.command` path - could be arbitrary executable. No validation of `config.cwd` - could enable directory traversal. No environment variable name validation.

**Fix**: Validate command path (no `..`, no shell metacharacters), canonicalize `cwd`, validate env var names.

#### [S-5] Shell From Untrusted Environment Variable (High)
**File**: `src/pty_session.rs`, lines 205-215

`$SHELL` environment variable used without validation. Attacker with env control could set `SHELL=/tmp/malicious`.

**Fix**: Validate the shell path exists and is executable before using it.

#### [S-6] No Image Size Limits (High)
**Files**: `src/graphics/iterm.rs`, `src/graphics/kitty.rs`

Decoded image dimensions are not validated. A malicious image could specify dimensions like 999999x999999, causing OOM.

**Fix**: Add `MAX_IMAGE_WIDTH`/`MAX_IMAGE_HEIGHT` constants (e.g., 4096x2160) and reject oversized images.

#### [S-7] Unbounded Base64 Image Data (High)
**File**: `src/graphics/iterm.rs`, lines 65-66

`self.data: Vec<u8>` accumulates base64 data without any size limit. Could consume arbitrary memory.

**Fix**: Add `MAX_IMAGE_DATA` constant (e.g., 100MB) and reject oversized payloads.

#### [S-8] Unbounded Sixel Raster Dimensions (High)
**File**: `src/terminal/sequences/dcs/sixel.rs`, lines 44-54

Sixel raster width/height parsed as `usize` without upper bounds. Could cause massive memory allocation.

**Fix**: Add `MAX_SIXEL_WIDTH`/`MAX_SIXEL_HEIGHT` constants and validate before calling `set_raster_attributes()`.

#### [S-9] Missing Sixel Color Index Validation (High)
**File**: `src/terminal/sequences/dcs/sixel.rs`, lines 22-41

`color_idx` parsed from input but not validated against sixel color limits before use.

**Fix**: Validate `color_idx < MAX_SIXEL_COLORS` before calling `select_color()`/`define_color()`.

### 3.3 Medium

#### [S-10] OSC String Length Unbounded (Medium)
**Files**: `src/terminal/sequences/osc/shell.rs`, other OSC handlers

No maximum string length validation on OSC sequences. Extremely long strings could cause memory exhaustion.

**Fix**: Add `MAX_OSC_STRING_LENGTH` constant (e.g., 1MB) and reject oversized sequences.

#### [S-11] Clipboard Size Unbounded (Medium)
**File**: `src/terminal/clipboard.rs`

`ClipboardEntry.content: String` has no size limit. Combined with clipboard history (10 entries per slot, multiple slots), memory could grow several MB per session.

**Fix**: Add `MAX_CLIPBOARD_SIZE` constant (e.g., 10MB) and reject oversized content.

#### [S-12] TLS Private Key Permissions Not Validated (Medium)
**File**: `src/streaming/server.rs`, lines 71-120

Private key files loaded without checking file permissions. A world-readable private key should trigger a warning.

**Fix**: On Unix, check `mode & 0o077 == 0` before loading private keys.

#### [S-13] Password File Permissions Not Validated (Medium)
**File**: `src/bin/streaming_server.rs`, line 1878

htpasswd file loaded without permission validation.

**Fix**: Validate file permissions same as TLS private key.

#### [S-14] Password Stored in Plain Text in Memory (Medium)
**File**: `src/streaming/server.rs`, lines 220-251

`PasswordConfig::ClearText` keeps password in memory. Visible in debugger/core dumps.

**Fix**: Consider using the `zeroize` crate to clear memory after verification.

### 3.4 Low

#### [S-15] FFI Memory Safety Contract Under-documented (Low)
**File**: `src/ffi.rs`

Unsafe blocks have safety comments but the contract that callers must not access the `Terminal` struct while a `SharedState` is active needs stronger documentation.

#### [S-16] CString Double Unwrap (Low)
**File**: `src/ffi.rs`, lines 112, 120

```rust
let title_cstring = CString::new(title_str).unwrap_or_else(|_| CString::new("").unwrap());
```

The inner `unwrap()` on `CString::new("")` cannot fail in practice but violates the principle of avoiding unwrap in FFI code.

---

## 4. Code Quality

### 4.1 Strengths

- **Consistent error handling**: Uses `Result` types throughout with proper error propagation
- **Good bounds checking**: CSI cursor handlers use `saturating_sub()` and `min()` operations
- **Safe PTY implementation**: Proper cleanup in `Drop`, timeout handling for thread joins
- **Comprehensive Python docstrings**: PTY binding methods have Google-style docstrings with Args/Returns
- **Strong streaming error types**: `StreamingError` enum with proper Display impl and 20+ error tests

### 4.2 Issues

#### [Q-1] Debug Output in Production Code (High)
**File**: `src/grid/scroll.rs`, line 181

```rust
eprintln!(
    "Reflowing main grid: {}x{} -> {}x{}",
    old_cols, old_rows, cols, rows
);
```

Unconditional `eprintln!()` in grid reflow. Will produce unexpected stderr output in all production environments and may interfere with TUI applications.

**Fix**: Replace with conditional debug logging via the project's `debug::log()` system.

#### [Q-2] Mutex Unwrap in Streaming Server (High)
**File**: `src/streaming/server.rs`, lines 1609, 1627, 1719, 1739

```rust
*uri_query_clone.lock().unwrap() = Some(q.to_string());
```

Direct `.unwrap()` on mutex lock. If the lock holder panics, the entire server deadlocks or crashes.

**Fix**: Use `if let Ok(mut guard) = lock()` with error logging fallback.

#### [Q-3] Font Cache Unwrap (High)
**File**: `src/screenshot/font_cache.rs`, lines 408, 471, 503, 568, 591

```rust
return self.cache.get(&key).unwrap();
```

5 instances of unwrap on glyph cache lookup. Assumes entry was previously cached - will panic if cache logic changes.

**Fix**: Use `.ok_or_else()` with proper error propagation.

#### [Q-4] Output Channel Failures Silently Ignored (Medium)
**File**: `src/python_bindings/streaming.rs`, lines 400, 410, 421

```rust
let _ = output_sender.try_send(output);
```

Bounded channel (1000) drops messages silently when full. No logging, no metrics.

**Fix**: Log dropped messages at warn level. Consider exposing a dropped-message counter.

#### [Q-5] Protocol Panics on Malformed Data (Medium)
**File**: `src/streaming/proto.rs`, lines 1057-1379

14 test functions use `_ => panic!("Wrong message type")`. While currently in test code, the pattern of panic-on-unexpected-variant could propagate to production if copied.

**Fix**: Use proper error returns in all non-test code paths.

#### [Q-6] Unbounded Glyph Cache (Medium)
**File**: `src/screenshot/font_cache.rs`, line 96

HashMap glyph cache has no size limit. Long-running screenshot sessions could accumulate unbounded memory.

**Fix**: Add LRU eviction or max cache size.

#### [Q-7] Silent Scroll Region Failures (Medium)
**File**: `src/grid/scroll.rs`, lines 88-122

`scroll_region_up()`/`scroll_region_down()` return silently on invalid parameters. Could hide bugs in CSI handlers.

**Fix**: Return `bool` success indicator and add debug logging on invalid parameters.

#### [Q-8] Tab Stop Array Not Validated After Resize (Low)
**File**: `src/terminal/mod.rs`, lines 838-842

If `cols` is 0 after resize, tab stop array will be empty but later accesses may index out of bounds.

**Fix**: Validate `cols > 0` in resize.

#### [Q-9] Origin Mode Underflow Risk (Low)
**File**: `src/terminal/sequences/csi/cursor.rs`, lines 73-83

If `scroll_region_bottom < scroll_region_top` (invalid state), `region_height - 1` could underflow.

**Fix**: Add `region_height > 0` guard.

#### [Q-10] NaN Not Handled in Image Size (Low)
**File**: `src/graphics/mod.rs`, line 129

`self.value == 0.0` comparison for "auto" doesn't account for NaN.

**Fix**: Use `!self.value.is_finite() || self.value == 0.0`.

#### [Q-11] Unsafe UTF-8 Unwrap in Streaming (Low)
**File**: `src/python_bindings/streaming.rs`, line 408

```rust
let valid_str = std::str::from_utf8(&buffer[..valid_up_to]).unwrap();
```

While `valid_up_to` guarantees validity, a `.expect()` with explanation would be safer.

---

## 5. Test Coverage

### 5.1 Overview

| Category | Count | Quality |
|----------|------:|---------|
| Rust unit tests | 1,582 | Good - behavior-focused |
| Python integration tests | 366 | Strong - full workflow coverage |
| Terminal test modules | 23 files | Comprehensive for core features |
| Streaming protocol tests | 79 | Good serialization coverage |

### 5.2 Well-Covered Modules

- **Terminal core**: 210+ tests covering cursor, colors, modes, alt screen, mouse, tabs
- **CSI sequences**: 45 tests for cursor movement, erasing, scrolling, mode changes
- **OSC sequences**: 103 tests for title, colors, shell integration, clipboard, notifications
- **Grid operations**: 65 tests for cell ops, scrolling, damage tracking
- **Graphics**: 70 tests across Sixel/iTerm2/Kitty protocols
- **Python bindings (Rust-side)**: 142 tests for type conversions, enums, colors
- **Screenshot rendering**: 99 tests for SVG/image formats, config, fonts

### 5.3 Critical Coverage Gaps

#### [T-1] Streaming Server: 0 Unit Tests (Critical)
**File**: `src/streaming/server.rs` - 114 functions, 1,800+ lines

The most complex module in the project has **zero unit tests**. This covers:
- TLS configuration
- WebSocket lifecycle
- Client connection management
- Message broadcasting
- HTTP authentication (Basic Auth, API Key)
- Rate limiting

**Impact**: Authentication bugs, connection leaks, and protocol errors could ship undetected.

**Recommendation**: Add tests for authentication middleware, connection lifecycle, and message broadcasting at minimum.

#### [T-2] Python Streaming Bindings: 0 Unit Tests (High)
**File**: `src/python_bindings/streaming.rs` - 2,022 lines

Dict conversion logic, event type matching, and the callback system have no unit tests.

**Recommendation**: Add tests for dict conversion of all ServerMessage variants and event type matching.

#### [T-3] Terminal Perform: 0 Tests (High)
**File**: `src/terminal/perform.rs`

VTE parser callback dispatch layer has no direct tests. Tested indirectly via integration tests, but dispatch logic itself is unvalidated.

#### [T-4] Terminal Event System: 0 Tests (Medium)
**File**: `src/terminal/event.rs`

Event enum definitions and queuing logic only tested through Python integration tests.

#### [T-5] Recording/Replay: 0 Tests (Medium)
**File**: `src/terminal/recording.rs`

Session recording and event serialization (Asciicast, JSON, TTY) untested.

#### [T-6] HTML Export: 0 Tests (Medium)
**File**: `src/html_export.rs`

No tests for HTML export functionality.

#### [T-7] Coprocess: 0 Tests (Medium)
**File**: `src/coprocess.rs`

External process spawning untested.

#### [T-8] Search: Minimal Tests (Low)
**File**: `src/terminal/tests/search.rs` - 2 tests

Text search functionality has only 2 tests. Edge cases (regex, large buffers, Unicode) untested.

### 5.4 Missing Test Categories

- **Concurrency tests**: No tests for multi-client streaming scenarios or concurrent terminal access
- **Performance benchmarks**: No `benches/` directory or benchmark tests
- **Fuzz testing**: No fuzz tests for VT sequence parsing (high-value target)
- **Doctests**: No documentation examples with `#[test]` attributes

---

## 6. Documentation Review

### 6.1 Overview

**Score: 8.3/10** - Excellent for a project of this complexity.

17 documentation files totaling ~17,600 lines cover architecture, API reference, VT sequences, streaming, security, building, and more.

### 6.2 Strengths

- **Comprehensive API reference** (`docs/API_REFERENCE.md`, 2,191 lines) with organized sections
- **Architecture documentation** (`docs/ARCHITECTURE.md`, 1,155 lines) with Mermaid diagrams
- **VT sequence reference** (both quick reference and technical reference)
- **Excellent changelog** with semantic versioning and detailed categorization
- **Developer guidance** (`CLAUDE.md`) with build commands, workflow rules, and architecture overview
- **Cross-linking** between documents

### 6.3 Gaps

#### [DOC-1] No Instant Replay Guide (Medium)
v0.38.0 feature lacks dedicated documentation. SnapshotManager and ReplaySession API need examples.

#### [DOC-2] No C/C++ FFI Guide (Medium)
New `#[repr(C)]` types and C API deserve a dedicated guide for non-Rust/Python consumers.

#### [DOC-3] No Performance Tuning Guide (Medium)
jemalloc, TCP_NODELAY, compression thresholds, and output batching mentioned in changelog but not consolidated.

#### [DOC-4] Streaming Docs Lack Python Examples (Low)
`docs/STREAMING.md` focuses on Rust server; Python integration examples are minimal.

#### [DOC-5] Duplicate Content in README (Low)
"What's New" sections repeat CHANGELOG.md verbatim for recent versions.

#### [DOC-6] Stale Version in BUILDING.md Title (Low)
References version 0.18.0 in title despite covering 0.38.0 features.

#### [DOC-7] Observer Patterns Under-documented (Low)
Advanced observer patterns (subscription filtering, async observers) lack detailed examples.

---

## 7. Prioritized Remediation Plan

### Priority 1: Critical (Fix Immediately)

| ID | Finding | Effort |
|----|---------|--------|
| S-1 | Timing attack in API key auth - add `subtle` crate | Small |

### Priority 2: High (Fix Before Next Release)

| ID | Finding | Effort |
|----|---------|--------|
| S-2 | Non-constant-time password comparison | Small |
| S-3 | Disable API key in query params by default | Small |
| S-4 | Coprocess command validation | Medium |
| S-5 | Shell environment variable validation | Small |
| S-6 | Image size limits (iTerm2/Kitty) | Small |
| S-7 | Unbounded base64 image data | Small |
| S-8 | Unbounded sixel raster dimensions | Small |
| S-9 | Missing sixel color index validation | Small |
| Q-1 | Remove debug `eprintln!()` from grid reflow | Small |
| Q-2 | Fix mutex unwrap in streaming server | Small |
| Q-3 | Fix font cache unwrap (5 instances) | Small |
| T-1 | Add streaming server unit tests (auth, connections) | Large |

### Priority 3: Medium (Fix in Next 2 Releases)

| ID | Finding | Effort |
|----|---------|--------|
| S-10 | OSC string length limits | Small |
| S-11 | Clipboard size limits | Small |
| S-12 | TLS private key permission validation | Small |
| S-13 | Password file permission validation | Small |
| S-14 | Password memory zeroization | Small |
| A-1 | Begin Terminal struct decomposition | Large |
| D-1 | Fix observer dispatch race | Medium |
| Q-4 | Log output channel drops | Small |
| Q-5 | Replace protocol panics with errors | Medium |
| Q-6 | Add glyph cache LRU eviction | Medium |
| Q-7 | Return success from scroll operations | Small |
| T-2 | Python streaming binding tests | Medium |
| T-3 | Terminal perform dispatch tests | Medium |
| DOC-1 | Instant Replay guide | Medium |
| DOC-2 | C/C++ FFI guide | Medium |

### Priority 4: Low (Backlog)

| ID | Finding | Effort |
|----|---------|--------|
| A-2 | Rust edition upgrade to 2024 | Small |
| D-2 | Standardize mutex implementation | Small |
| S-15 | FFI safety contract documentation | Small |
| S-16 | CString double unwrap fix | Small |
| Q-8 | Tab stop resize validation | Small |
| Q-9 | Origin mode underflow guard | Small |
| Q-10 | NaN handling in image size | Small |
| Q-11 | UTF-8 unwrap -> expect | Small |
| T-4-T-8 | Various test coverage gaps | Medium-Large |
| DOC-3-7 | Documentation improvements | Small-Medium |

---

## Conclusion

par-term-emu-core-rust is a **mature, well-engineered project** with strong architecture and good test coverage for its core terminal emulation functionality. The primary areas requiring attention are:

1. **Security hardening** of the streaming server authentication (timing attacks, input bounds)
2. **Input validation** for graphics protocols (DoS via unbounded allocations)
3. **Test coverage** for the streaming server (0 tests for 1,800+ lines)
4. **Long-term maintainability** via Terminal struct decomposition

The ongoing refactoring work visible in the git status (splitting large files into focused modules) demonstrates healthy codebase evolution. Addressing the critical and high-priority items in this report will bring the project to production-ready security posture.
