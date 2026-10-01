/*
 * terminal_core_layout.h — hand-written companion to the cbindgen-generated
 * terminal_core.h (ENH-027). cbindgen emits the structs and prototypes; this
 * file carries what it cannot: the TERM_* constants and the _Static_asserts
 * that pin the generated layouts to the Rust #[repr(C)] side. It is included
 * from the trailer of terminal_core.h — do not include it directly.
 *
 * Every #define below is pinned to its Rust source of truth by
 * layout_header_defines_match_rust in src/ffi.rs:
 * - TERM_CELL_* == crate::cell::CellBitflags bits
 * - TERM_ATTR_* == crate::ffi::attr_bits
 * - TERM_MOUSE_MODE_* == crate::mouse::MouseMode discriminants
 * - TERM_MOD_* == crate::keyboard::modifiers
 * - TERM_KEY_* == crate::keyboard::TermKey discriminants (kitty codes)
 * - TERM_EVENT_* == the term_event_kinds! table in src/ffi.rs
 * - TERM_CORE_ABI_VERSION == TERM_CORE_ABI_VERSION in src/ffi.rs
 * - TERM_SCROLL_FULL_REDRAW == crate::ffi::TERM_SCROLL_FULL_REDRAW
 * Renumber either side and that test fails.
 */

#ifndef PAR_TERM_EMU_CORE_TERMINAL_CORE_LAYOUT_H
#define PAR_TERM_EMU_CORE_TERMINAL_CORE_LAYOUT_H

/* Contract version of terminal_core.h (ARC-063). Compare against
 * ptec_terminal_abi_version() at runtime to detect a layout mismatch; bump on
 * any layout or contract change to the C surface. Version 4 (breaking): the
 * ptec_ symbol prefix, palette-resolved SharedCell colors + TERM_ATTR_* bits,
 * ptec_terminal_read_cell_grapheme, the on_event_v2 slot + TermEvent, and
 * ptec_terminal_scrollback_total_scrolled. Version 5 (additive, ENH-038):
 * scroll-aware damage — TermScrollDelta, ptec_terminal_scroll_delta_since,
 * ptec_terminal_content_dirty_ranges_since, TERM_SCROLL_FULL_REDRAW. */
#define TERM_CORE_ABI_VERSION 5

/* Cell attribute bits — SharedCell.attrs (mirrors CellBitflags in cell.rs). */
#define TERM_CELL_BOLD 1u             /* bit 0 */
#define TERM_CELL_DIM 2u              /* bit 1 */
#define TERM_CELL_ITALIC 4u           /* bit 2 */
#define TERM_CELL_UNDERLINE 8u        /* bit 3 */
#define TERM_CELL_BLINK 16u           /* bit 4 */
#define TERM_CELL_REVERSE 32u         /* bit 5 */
#define TERM_CELL_HIDDEN 64u          /* bit 6 */
#define TERM_CELL_STRIKETHROUGH 128u  /* bit 7 */
#define TERM_CELL_OVERLINE 256u       /* bit 8 */
#define TERM_CELL_GUARDED 512u        /* bit 9 */
#define TERM_CELL_WIDE_CHAR 1024u     /* bit 10 */
#define TERM_CELL_WIDE_CHAR_SPACER 2048u /* bit 11 */

/* Readback bits in SharedCell.attrs above the TERM_CELL_* bits (ARC-101;
 * crate::ffi::attr_bits). */
#define TERM_ATTR_DEFAULT_FG 4096u     /* bit 12: fg is the OSC 10 default */
#define TERM_ATTR_DEFAULT_BG 8192u     /* bit 13: bg is the OSC 11 default */
#define TERM_ATTR_HAS_COMBINING 16384u /* bit 14: read the cluster with ptec_terminal_read_cell_grapheme */

/* Mouse tracking modes — SharedState.mouse_mode / TermModeState.mouse_mode
 * (MouseMode discriminants in mouse.rs). */
#define TERM_MOUSE_MODE_OFF 0
#define TERM_MOUSE_MODE_X10 1
#define TERM_MOUSE_MODE_NORMAL 2
#define TERM_MOUSE_MODE_BUTTON 3
#define TERM_MOUSE_MODE_ANY 4

/* Key-event modifier bits (TermKeyEvent.modifiers) — kitty protocol order. */
#define TERM_MOD_SHIFT 1u /* bit 0 */
#define TERM_MOD_ALT 2u   /* bit 1 */
#define TERM_MOD_CTRL 4u  /* bit 2 */
#define TERM_MOD_SUPER 8u /* bit 3 */
#define TERM_MOD_HYPER 16u /* bit 4 */
#define TERM_MOD_META 32u  /* bit 5 */
/* Side info, not a modifier: the held Alt key is the right one (ENH-028).
 * Selects TermKeyOptions.right_option over left_option; ignored by every
 * xterm/kitty modifier parameter field. */
#define TERM_MOD_ALT_RIGHT 64u /* bit 6 */

/* macOS Option-key modes for TermKeyOptions (ENH-028) — left_option /
 * right_option fields of ptec_terminal_encode_key_ex. */
#define TERM_OPTION_MODE_NORMAL 0u /* pass the composed character through */
#define TERM_OPTION_MODE_META 1u   /* 8th bit on ASCII bases, ESC otherwise */
#define TERM_OPTION_MODE_ESC 2u    /* ESC-prefix the base character */

/* TermKey codes. Functional-key values ARE the kitty protocol functional
 * codes; do not renumber. */
#define TERM_KEY_UNKNOWN 0
#define TERM_KEY_CHAR 1
#define TERM_KEY_TAB 9
#define TERM_KEY_ENTER 13
#define TERM_KEY_ESCAPE 27
#define TERM_KEY_BACKSPACE 127
#define TERM_KEY_INSERT 57426
#define TERM_KEY_DELETE 57427
#define TERM_KEY_LEFT 57428
#define TERM_KEY_RIGHT 57429
#define TERM_KEY_UP 57430
#define TERM_KEY_DOWN 57431
#define TERM_KEY_PAGE_UP 57432
#define TERM_KEY_PAGE_DOWN 57433
#define TERM_KEY_HOME 57434
#define TERM_KEY_END 57435
#define TERM_KEY_F1 57376
#define TERM_KEY_F2 57377
#define TERM_KEY_F3 57378
#define TERM_KEY_F4 57379
#define TERM_KEY_F5 57380
#define TERM_KEY_F6 57381
#define TERM_KEY_F7 57382
#define TERM_KEY_F8 57383
#define TERM_KEY_F9 57384
#define TERM_KEY_F10 57385
#define TERM_KEY_F11 57386
#define TERM_KEY_F12 57387

/* Structured event kinds — TermEvent.kind, delivered to on_event_v2
 * (ARC-114). Stable codes: never renumbered, new kinds are appended. */
#define TERM_EVENT_BELL 1
#define TERM_EVENT_TITLE_CHANGED 2
#define TERM_EVENT_SIZE_CHANGED 3
#define TERM_EVENT_MODE_CHANGED 4
#define TERM_EVENT_GRAPHICS_ADDED 5
#define TERM_EVENT_HYPERLINK_ADDED 6
#define TERM_EVENT_DIRTY_REGION 7
#define TERM_EVENT_CWD_CHANGED 8
#define TERM_EVENT_TRIGGER_MATCHED 9
#define TERM_EVENT_USER_VAR_CHANGED 10
#define TERM_EVENT_PROGRESS_BAR_CHANGED 11
#define TERM_EVENT_BADGE_CHANGED 12
#define TERM_EVENT_SHELL_INTEGRATION 13
#define TERM_EVENT_ZONE_OPENED 14
#define TERM_EVENT_ZONE_CLOSED 15
#define TERM_EVENT_ZONE_SCROLLED_OUT 16
#define TERM_EVENT_ENVIRONMENT_CHANGED 17
#define TERM_EVENT_REMOTE_HOST_TRANSITION 18
#define TERM_EVENT_SUB_SHELL_DETECTED 19
#define TERM_EVENT_FILE_TRANSFER_STARTED 20
#define TERM_EVENT_FILE_TRANSFER_PROGRESS 21
#define TERM_EVENT_FILE_TRANSFER_COMPLETED 22
#define TERM_EVENT_FILE_TRANSFER_FAILED 23
#define TERM_EVENT_UPLOAD_REQUESTED 24
#define TERM_EVENT_SCREEN_CLEARED 25
#define TERM_EVENT_INLINE_IMAGE_DROPPED 26

/* Scroll-aware damage (ENH-038) — TermScrollDelta.flags. Bit 0: the scroll
 * movement since the generation cannot be expressed (screen switch, resize,
 * RIS, snapshot restore, scrollback clear, log overflow, mixed regions);
 * treat every row as dirty and skip the blit. */
#define TERM_SCROLL_FULL_REDRAW 1u

/*
 * Layout pins. The structs come from terminal_core.h (included before this
 * file via its trailer); these asserts fail the compile of any consumer
 * when the generated layout no longer matches the values the Rust
 * offset_of! tests in src/ffi.rs enforce on the other side.
 */
_Static_assert(sizeof(SharedCell) == 16, "SharedCell must match Rust repr(C) layout");
_Static_assert(offsetof(SharedCell, attrs) == 12, "SharedCell.attrs offset must match Rust");
_Static_assert(offsetof(SharedCell, width) == 14, "SharedCell.width offset must match Rust");

#ifdef __LP64__
_Static_assert(sizeof(SharedState) == 80, "SharedState must match Rust repr(C) layout (LP64)");
_Static_assert(offsetof(SharedState, title) == 24, "SharedState.title offset must match Rust (LP64)");
#endif

#ifdef __LP64__
_Static_assert(sizeof(TerminalObserverVtable) == 56, "vtable must match Rust repr(C) layout (LP64)");
_Static_assert(offsetof(TerminalObserverVtable, on_event_v2) == 40, "vtable.on_event_v2 offset must match Rust (LP64)");
_Static_assert(sizeof(TermEvent) == 16, "TermEvent must match Rust repr(C) layout (LP64)");
_Static_assert(offsetof(TermEvent, payload) == 8, "TermEvent.payload offset must match Rust (LP64)");
#endif
_Static_assert(offsetof(TermEvent, payload_len) == 4, "TermEvent.payload_len offset must match Rust");

_Static_assert(sizeof(TermRowRange) == 8, "TermRowRange must match Rust repr(C) layout");
_Static_assert(sizeof(TermScrollDelta) == 16, "TermScrollDelta must match Rust repr(C) layout");

#ifdef __LP64__
_Static_assert(sizeof(TermCursorState) == 12, "TermCursorState must match Rust repr(C) layout (LP64)");
#endif

#ifdef __LP64__
_Static_assert(sizeof(TermModeState) == 20, "TermModeState must match Rust repr(C) layout (LP64)");
#endif

_Static_assert(sizeof(TermKeyEvent) == 8, "TermKeyEvent must match Rust repr(C) layout");
_Static_assert(offsetof(TermKeyEvent, codepoint) == 4, "TermKeyEvent.codepoint offset must match Rust");
_Static_assert(sizeof(TermKeyOptions) == 2, "TermKeyOptions must match Rust repr(C) layout");

#endif /* PAR_TERM_EMU_CORE_TERMINAL_CORE_LAYOUT_H */
