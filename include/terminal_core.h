/*
 * terminal_core.h — C API for embedding the par-term emulator core.
 *
 * Mirrors src/ffi.rs. The Rust types are #[repr(C)]; the _Static_asserts
 * below pin the shared layout so this header and the Rust side cannot
 * drift apart silently (a mismatch fails the header smoke-compile in
 * scripts/build-xcframework.sh and the layout test in src/ffi.rs).
 *
 * Ownership contract:
 * - terminal_get_state returns a heap-allocated SharedState owned by the
 *   caller; release it with terminal_free_state. Its `title`, `cwd`, and
 *   `cells` pointers are valid only until that call.
 * - The vtable (including its user_data) must stay valid for the lifetime
 *   of the observer registration.
 */

#ifndef PAR_TERM_EMU_CORE_TERMINAL_CORE_H
#define PAR_TERM_EMU_CORE_TERMINAL_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to the Rust `Terminal`. */
typedef struct Terminal Terminal;

/* Cell attribute bits — SharedCell.attrs (mirrors CellBitflags in cell.rs). */
#define TERM_CELL_BOLD              (1u << 0)
#define TERM_CELL_DIM               (1u << 1)
#define TERM_CELL_ITALIC            (1u << 2)
#define TERM_CELL_UNDERLINE         (1u << 3)
#define TERM_CELL_BLINK             (1u << 4)
#define TERM_CELL_REVERSE           (1u << 5)
#define TERM_CELL_HIDDEN            (1u << 6)
#define TERM_CELL_STRIKETHROUGH     (1u << 7)
#define TERM_CELL_OVERLINE          (1u << 8)
#define TERM_CELL_GUARDED           (1u << 9)
#define TERM_CELL_WIDE_CHAR         (1u << 10)
#define TERM_CELL_WIDE_CHAR_SPACER  (1u << 11)

/* Mouse tracking modes — SharedState.mouse_mode. */
#define TERM_MOUSE_MODE_OFF        0
#define TERM_MOUSE_MODE_X10        1
#define TERM_MOUSE_MODE_NORMAL     2
#define TERM_MOUSE_MODE_BUTTON     3
#define TERM_MOUSE_MODE_ANY        4

/* One grid cell in a C-compatible layout. `text` holds up to 4 UTF-8
 * bytes of the base character; `text_len` says how many are valid. */
typedef struct SharedCell {
    uint8_t text[4];   /* UTF-8 bytes of the character */
    uint8_t text_len;  /* valid bytes in text */
    uint8_t fg_r, fg_g, fg_b;
    uint8_t bg_r, bg_g, bg_b;
    uint16_t attrs;    /* TERM_CELL_* bitfield */
    uint8_t width;     /* display width (1 or 2) */
} SharedCell;

_Static_assert(sizeof(SharedCell) == 16, "SharedCell must match Rust repr(C) layout");
_Static_assert(offsetof(SharedCell, attrs) == 12, "SharedCell.attrs offset must match Rust");
_Static_assert(offsetof(SharedCell, width) == 14, "SharedCell.width offset must match Rust");

/* Complete terminal snapshot. `title`, `cwd`, and `cells` are owned by
 * this struct and freed by terminal_free_state (via the Rust Drop impl). */
typedef struct SharedState {
    uint32_t cols;
    uint32_t rows;
    uint32_t cursor_col;      /* 0-indexed */
    uint32_t cursor_row;      /* 0-indexed */
    bool cursor_visible;
    bool alt_screen_active;
    uint8_t mouse_mode;       /* TERM_MOUSE_MODE_* */
    char *title;              /* NUL-terminated, owned (title_len bytes) */
    uint32_t title_len;
    char *cwd;                /* NUL-terminated, owned, or NULL */
    uint32_t cwd_len;
    SharedCell *cells;        /* cell_count entries, owned */
    uint32_t cell_count;
    uint32_t scrollback_lines;
    uint32_t total_lines;     /* visible + scrollback */
} SharedState;

#ifdef __LP64__
_Static_assert(sizeof(SharedState) == 80, "SharedState must match Rust repr(C) layout (LP64)");
_Static_assert(offsetof(SharedState, title) == 24, "SharedState.title offset must match Rust (LP64)");
#endif

/* Event callback: receives user_data and a NUL-terminated, Debug-formatted
 * event description valid only for the duration of the call. Do not free. */
typedef void (*term_event_cb)(void *user_data, const char *event_text);

typedef struct TerminalObserverVtable {
    term_event_cb on_zone_event;     /* optional */
    term_event_cb on_command_event;  /* optional */
    term_event_cb on_environment_event; /* optional */
    term_event_cb on_screen_event;   /* optional */
    term_event_cb on_event;          /* catch-all, optional */
    void *user_data;                 /* passed to every callback */
} TerminalObserverVtable;

#ifdef __LP64__
_Static_assert(sizeof(TerminalObserverVtable) == 48, "vtable must match Rust repr(C) layout (LP64)");
#endif

/* Snapshot the terminal's current state. Caller owns the result and must
 * free it with terminal_free_state. Returns NULL if term is NULL. */
SharedState *terminal_get_state(const Terminal *term);

/* Free a SharedState returned by terminal_get_state. NULL is a no-op. */
void terminal_free_state(SharedState *state);

/* Register an observer; returns its id for terminal_remove_observer.
 * The vtable and its user_data must outlive the registration. */
uint64_t terminal_add_observer(Terminal *term, TerminalObserverVtable vtable);

/* Remove a previously registered observer; true if it was found. */
bool terminal_remove_observer(Terminal *term, uint64_t id);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PAR_TERM_EMU_CORE_TERMINAL_CORE_H */
