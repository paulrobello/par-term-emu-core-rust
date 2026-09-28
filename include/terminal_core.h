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

/* ------------------------------------------------------------------ */
/* Embedding surface — lifecycle, feed, damage, pinned readback, keys  */
/* ------------------------------------------------------------------ */

/* Inclusive [start, end] range of dirty rows returned by
 * terminal_dirty_ranges — a renderer redraws only rows inside ranges. */
typedef struct TermRowRange {
    uint32_t start; /* first dirty row, 0-indexed, inclusive */
    uint32_t end;   /* last dirty row, 0-indexed, inclusive */
} TermRowRange;

_Static_assert(sizeof(TermRowRange) == 8, "TermRowRange must match Rust repr(C) layout");

typedef struct TermCursorState {
    uint32_t col;
    uint32_t row;
    bool visible;
    uint8_t style; /* 0 blinking block, 1 steady block, 2 blinking
                    * underline, 3 steady underline, 4 blinking bar,
                    * 5 steady bar */
} TermCursorState;

#ifdef __LP64__
_Static_assert(sizeof(TermCursorState) == 12, "TermCursorState must match Rust repr(C) layout (LP64)");
#endif

typedef struct TermModeState {
    bool alt_screen;
    bool bracketed_paste;
    bool application_cursor;
    bool origin_mode;
    bool insert_mode;
    bool auto_wrap;
    uint8_t mouse_mode;   /* TERM_MOUSE_MODE_* */
    uint16_t kitty_flags; /* kitty keyboard progressive-enhancement flags */
    uint32_t cols;
    uint32_t rows;
} TermModeState;

#ifdef __LP64__
_Static_assert(sizeof(TermModeState) == 20, "TermModeState must match Rust repr(C) layout (LP64)");
#endif

/* Key-event modifier bits (TermKeyEvent.modifiers) — kitty protocol order. */
#define TERM_MOD_SHIFT (1u << 0)
#define TERM_MOD_ALT   (1u << 1)
#define TERM_MOD_CTRL  (1u << 2)
#define TERM_MOD_SUPER (1u << 3)
#define TERM_MOD_HYPER (1u << 4)
#define TERM_MOD_META  (1u << 5)

/* TermKey codes. Functional-key values ARE the kitty protocol functional
 * codes; do not renumber. */
#define TERM_KEY_UNKNOWN    0
#define TERM_KEY_CHAR       1
#define TERM_KEY_TAB        9
#define TERM_KEY_ENTER      13
#define TERM_KEY_ESCAPE     27
#define TERM_KEY_BACKSPACE  127
#define TERM_KEY_INSERT     57426
#define TERM_KEY_DELETE     57427
#define TERM_KEY_LEFT       57428
#define TERM_KEY_RIGHT      57429
#define TERM_KEY_UP         57430
#define TERM_KEY_DOWN       57431
#define TERM_KEY_PAGE_UP    57432
#define TERM_KEY_PAGE_DOWN  57433
#define TERM_KEY_HOME       57434
#define TERM_KEY_END        57435
#define TERM_KEY_F1         57376
#define TERM_KEY_F2         57377
#define TERM_KEY_F3         57378
#define TERM_KEY_F4         57379
#define TERM_KEY_F5         57380
#define TERM_KEY_F6         57381
#define TERM_KEY_F7         57382
#define TERM_KEY_F8         57383
#define TERM_KEY_F9         57384
#define TERM_KEY_F10        57385
#define TERM_KEY_F11        57386
#define TERM_KEY_F12        57387

typedef struct TermKeyEvent {
    uint16_t key;       /* TERM_KEY_* */
    uint8_t modifiers;  /* TERM_MOD_* bitfield */
    uint8_t _pad;
    uint32_t codepoint; /* Unicode scalar for TERM_KEY_CHAR (typed form for
                         * plain text, base form for Ctrl/Alt), else 0 */
} TermKeyEvent;

_Static_assert(sizeof(TermKeyEvent) == 8, "TermKeyEvent must match Rust repr(C) layout");
_Static_assert(offsetof(TermKeyEvent, codepoint) == 4, "TermKeyEvent.codepoint offset must match Rust");

/* Lifecycle */
Terminal *terminal_create(uint32_t cols, uint32_t rows, uint32_t scrollback);
void terminal_free(Terminal *term);

/* Feed raw application/PTY output bytes (VT parsing). */
void terminal_feed(Terminal *term, const uint8_t *bytes, uint32_t len);
void terminal_resize(Terminal *term, uint32_t cols, uint32_t rows);

/* Damage: coalesced inclusive dirty-row ranges. Writes up to `cap` ranges
 * into `out` and returns the TOTAL range count — if the return exceeds
 * `cap`, call again with a larger buffer (out may be NULL when cap is 0). */
uint32_t terminal_dirty_ranges(Terminal *term, TermRowRange *out, uint32_t cap);
void terminal_mark_clean(Terminal *term);

/* Pinned readback into caller-owned buffers — no allocation, no full-grid
 * copy. Both return the number of cells written. Rows are 0-indexed;
 * scrollback `line` runs from oldest (0) to newest (count-1). */
uint32_t terminal_read_row(Terminal *term, uint32_t row, uint32_t col_start,
                           SharedCell *out, uint32_t cap);
uint32_t terminal_read_scrollback_row(Terminal *term, uint32_t line,
                                      uint32_t col_start, SharedCell *out,
                                      uint32_t cap);
uint32_t terminal_scrollback_count(Terminal *term);

/* Cursor and per-frame mode state. */
void terminal_get_cursor(Terminal *term, TermCursorState *out);
void terminal_get_modes(Terminal *term, TermModeState *out);

/* Encode a key event against the terminal's negotiated input state
 * (application cursor keys, kitty keyboard flags) — the bytes a frontend
 * writes to the PTY. Writes up to `cap` bytes into `out` and returns the
 * TOTAL encoded length — if the return exceeds `cap`, retry larger. */
uint32_t terminal_encode_key(Terminal *term, const TermKeyEvent *ev,
                             uint8_t *out, uint32_t cap);

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
