# FFI Guide

This guide explains how to embed the terminal emulator in C, C++, Swift, Kotlin/JNI, or other languages that support C FFI.

## Overview

The library provides a C-compatible API for accessing terminal state via `#[repr(C)]` types and `extern "C"` functions. This enables embedding in applications written in languages other than Rust or Python.

### FFI Types

- **`SharedCell`** - A single terminal cell (character, colors, attributes)
- **`SharedState`** - A complete terminal snapshot (grid, cursor, metadata)
- **`TerminalObserverVtable`** - Function-pointer table for receiving terminal events

All types use `#[repr(C)]` layout for stable ABI across language boundaries.

## Building for C/C++

Use the `rust-only` feature flag to exclude Python bindings:

```bash
# Build static library (.a) + dynamic library (.so/.dylib/.dll)
cargo build --release --no-default-features --features rust-only

# Output: target/release/libpar_term_emu_core_rust.a
#         target/release/libpar_term_emu_core_rust.so (Linux)
#         target/release/libpar_term_emu_core_rust.dylib (macOS)
#         target/release/par_term_emu_core_rust.dll (Windows)
```

Link against the appropriate library in your C/C++ project and include the generated C header (or manually declare the FFI types).

## Memory Management Contract

### Ownership Rules

1. **`SharedState` ownership**: When you call `terminal_get_state()`, you receive a heap-allocated `SharedState` that you **own**. You must free it by calling `terminal_free_state()`.

2. **Raw pointer lifetimes**: The `title`, `cwd`, and `cells` pointers inside `SharedState` are valid **only** while the `SharedState` exists. After calling `terminal_free_state()`, these pointers become invalid.

3. **String encoding**: All strings (`title`, `cwd`) are NUL-terminated UTF-8 (`*mut c_char`). Do not free them directly; they are freed automatically when `SharedState` is dropped.

4. **Cell array**: The `cells` pointer is an array of `cell_count` elements. Do not free it directly; it is freed automatically when `SharedState` is dropped.

5. **Observer vtables**: The `user_data` pointer in `TerminalObserverVtable` must remain valid for the lifetime of the observer registration. The library does not take ownership of `user_data`; you are responsible for its lifetime.

### Safety Requirements

- **Thread safety**: Do not access a `Terminal` from multiple threads simultaneously without external synchronization. The FFI does not perform internal locking.

- **Snapshot consistency**: Only one `SharedState` should exist per `Terminal` at a time. Creating multiple snapshots concurrently may result in data races.

- **Pointer validity**: All function pointers in `TerminalObserverVtable` must be valid for the duration of the observer registration.

## API Reference

### Snapshot Functions

#### `terminal_get_state`

```c
SharedState* terminal_get_state(const Terminal* term);
```

Creates a snapshot of the terminal's current state.

**Parameters:**
- `term` - Pointer to a `Terminal` instance (must be non-null)

**Returns:**
- Pointer to a heap-allocated `SharedState`, or `NULL` if `term` is null

**Ownership:** Caller must free the returned pointer with `terminal_free_state()`.

#### `terminal_free_state`

```c
void terminal_free_state(SharedState* state);
```

Frees a `SharedState` previously returned by `terminal_get_state()`.

**Parameters:**
- `state` - Pointer to free (may be null; no-op if null)

**Ownership:** `state` must not be used after this call.

### Observer Functions

#### `terminal_add_observer`

```c
uint64_t terminal_add_observer(Terminal* term, TerminalObserverVtable vtable);
```

Registers an observer to receive terminal events.

**Parameters:**
- `term` - Pointer to a `Terminal` instance (must be non-null, mutable)
- `vtable` - Function-pointer table with event callbacks

**Returns:**
- Observer ID (use with `terminal_remove_observer`), or 0 on failure

**Safety:** The `vtable` (including `user_data`) must remain valid until the observer is removed.

#### `terminal_remove_observer`

```c
bool terminal_remove_observer(Terminal* term, uint64_t id);
```

Removes a previously registered observer.

**Parameters:**
- `term` - Pointer to a `Terminal` instance (must be non-null, mutable)
- `id` - Observer ID returned by `terminal_add_observer`

**Returns:**
- `true` if the observer was found and removed, `false` otherwise

## Example: C Code

```c
#include <stdio.h>
#include <stdint.h>

// Forward declarations (manually extracted from Rust FFI)
typedef struct Terminal Terminal;
typedef struct SharedState SharedState;

extern SharedState* terminal_get_state(const Terminal* term);
extern void terminal_free_state(SharedState* state);

void print_terminal_state(const Terminal* term) {
    SharedState* state = terminal_get_state(term);
    if (!state) {
        fprintf(stderr, "Failed to get terminal state\n");
        return;
    }

    printf("Terminal: %u cols x %u rows\n", state->cols, state->rows);
    printf("Cursor: (%u, %u) visible=%d\n",
           state->cursor_col, state->cursor_row, state->cursor_visible);
    printf("Title: %s\n", state->title);
    printf("CWD: %s\n", state->cwd ? state->cwd : "(none)");
    printf("Alt screen: %d\n", state->alt_screen_active);
    printf("Scrollback: %u lines\n", state->scrollback_lines);

    // Access first cell (top-left corner)
    if (state->cell_count > 0) {
        SharedCell* cell = &state->cells[0];
        printf("First cell: char='%.*s' fg=(%d,%d,%d) bg=(%d,%d,%d) attrs=0x%04x\n",
               (int)cell->text_len, (char*)cell->text,
               cell->fg_r, cell->fg_g, cell->fg_b,
               cell->bg_r, cell->bg_g, cell->bg_b,
               cell->attrs);
    }

    terminal_free_state(state);
}
```

## Example: Observer

```c
#include <stdio.h>

void on_event_callback(void* user_data, const char* event_json) {
    const char* prefix = (const char*)user_data;
    printf("%s: %s\n", prefix, event_json);
}

void register_observer(Terminal* term) {
    TerminalObserverVtable vtable = {
        .on_event = on_event_callback,
        .user_data = (void*)"EventLog"
    };
    uint64_t observer_id = terminal_add_observer(term, vtable);
    printf("Registered observer with ID: %llu\n", observer_id);
}
```

## Additional Notes

- **Character encoding**: All text fields use UTF-8. `SharedCell.text` holds up to 4 bytes (enough for any Unicode scalar).

- **Mouse mode mapping**: 0=Off, 1=X10, 2=Normal, 3=ButtonEvent, 4=AnyEvent

- **Cell attributes**: The `attrs` field is a bitfield of VT attributes (bold, italic, underline, etc.). Bit definitions are in `src/cell.rs`.

- **Event format**: Observer callbacks receive events as JSON-formatted debug strings (`format!("{:?}", event)`). Parse these strings to extract event data.
