//! FFI-safe types and C API for embedding the terminal emulator
//!
//! This module provides `#[repr(C)]` types that can be safely shared across
//! FFI boundaries (Swift, Kotlin/JNI, C/C++, etc.) and extern "C" functions
//! for creating, querying, and observing terminal state.

use std::collections::HashSet;
use std::ffi::{c_char, CString};
use std::sync::Arc;

/// A C string plus its byte length for the FFI snapshot fields (SEC-117).
///
/// The length must always name the bytes of the string the pointer names —
/// `strlen` of the result, never the source's length. An interior NUL
/// cannot ride in a C string, so it is replaced with U+FFFD rather than
/// truncating (the old `unwrap_or_default` shape reported the source
/// length beside an empty string, and a consumer honoring the length
/// over-read the heap).
fn to_c_string(s: &str) -> (*mut c_char, u32) {
    // Infallible: the only failure mode of `CString::new` is an interior
    // NUL, replaced above.
    let cs = if s.contains('\0') {
        CString::new(s.replace('\0', "\u{FFFD}")).expect("interior NUL replaced")
    } else {
        CString::new(s).expect("no interior NUL")
    };
    let len = cs.as_bytes().len() as u32;
    (cs.into_raw(), len)
}

use crate::mouse::MouseMode;
use crate::observer::TerminalObserver;
use crate::terminal::{Terminal, TerminalEvent, TerminalEventKind};

// ---------------------------------------------------------------------------
// SharedCell — one cell in the grid, repr(C)-safe
// ---------------------------------------------------------------------------

/// A single terminal cell in a C-compatible layout.
///
/// The `text` field holds the UTF-8 bytes of the base character (up to 4 bytes
/// for any Unicode scalar value). `text_len` indicates how many bytes are valid.
/// When the cell also carries combining marks, `attrs` has
/// `TERM_ATTR_HAS_COMBINING` set and `ptec_terminal_read_cell_grapheme` returns the
/// full UTF-8 cluster (ARC-101).
///
/// Colors are resolved for display (ARC-101): the live ANSI palette (OSC 4),
/// the terminal default colors (OSC 10/11) for default cells, flagged with
/// `TERM_ATTR_DEFAULT_FG` / `TERM_ATTR_DEFAULT_BG`, and bold brightening. A
/// color counts as default when it equals the unstyled color or the current
/// OSC 10/11 value ([`Terminal::resolve_cell_colors`]).
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedCell {
    /// UTF-8 encoded character bytes (up to 4 bytes for any Unicode scalar)
    pub text: [u8; 4],
    /// Number of valid bytes in `text`
    pub text_len: u8,
    /// Foreground color — red component
    pub fg_r: u8,
    /// Foreground color — green component
    pub fg_g: u8,
    /// Foreground color — blue component
    pub fg_b: u8,
    /// Background color — red component
    pub bg_r: u8,
    /// Background color — green component
    pub bg_g: u8,
    /// Background color — blue component
    pub bg_b: u8,
    /// Bitfield: `TERM_CELL_*` cell attributes (bits 0-11) plus the
    /// `TERM_ATTR_*` readback bits (12-14)
    pub attrs: u16,
    /// Display width of the character (typically 1 or 2)
    pub width: u8,
}

/// `SharedCell.attrs` readback bits above the `TERM_CELL_*` attribute bits
/// (ARC-101). The values are the `TERM_ATTR_*` defines in
/// terminal_core_layout.h, pinned by `layout_header_defines_match_rust`.
pub mod attr_bits {
    /// The foreground is the terminal default (OSC 10), not an SGR color.
    pub const DEFAULT_FG: u16 = 1 << 12;
    /// The background is the terminal default (OSC 11), not an SGR color.
    pub const DEFAULT_BG: u16 = 1 << 13;
    /// The cell carries combining marks after its base character; read the
    /// full cluster with `ptec_terminal_read_cell_grapheme`.
    pub const HAS_COMBINING: u16 = 1 << 14;
}

// ---------------------------------------------------------------------------
// SharedState — full terminal snapshot, repr(C)-safe
// ---------------------------------------------------------------------------

/// A complete, C-compatible snapshot of the terminal state.
///
/// All heap-allocated fields (`title`, `cwd`, `cells`) are owned by this struct
/// and freed on `Drop`.
#[repr(C)]
pub struct SharedState {
    /// Number of columns in the terminal grid
    pub cols: u32,
    /// Number of rows in the terminal grid
    pub rows: u32,
    /// Current cursor column (0-indexed)
    pub cursor_col: u32,
    /// Current cursor row (0-indexed)
    pub cursor_row: u32,
    /// Whether the cursor is visible
    pub cursor_visible: bool,
    /// Whether the alternate screen buffer is active
    pub alt_screen_active: bool,
    /// Mouse tracking mode (0=Off, 1=X10, 2=Normal, 3=ButtonEvent, 4=AnyEvent)
    pub mouse_mode: u8,
    /// Terminal title as a NUL-terminated C string (owned)
    pub title: *mut c_char,
    /// Length of the title string in bytes (not counting NUL)
    pub title_len: u32,
    /// Current working directory as a NUL-terminated C string (owned), or null
    pub cwd: *mut c_char,
    /// Length of the cwd string in bytes (not counting NUL), 0 if cwd is null
    pub cwd_len: u32,
    /// Pointer to an array of `cell_count` SharedCell values (owned)
    pub cells: *mut SharedCell,
    /// Total number of cells (cols * rows)
    pub cell_count: u32,
    /// Number of lines currently in the scrollback buffer
    pub scrollback_lines: u32,
    /// Total lines (visible + scrollback)
    pub total_lines: u32,
}

impl SharedState {
    /// Build a `SharedState` snapshot from the current terminal state.
    ///
    /// The returned value owns all heap memory and will free it on drop.
    /// Raw pointers (`title`, `cwd`, `cells`) are valid only for the lifetime
    /// of this `SharedState` — accessing them after `Drop` is undefined behavior.
    ///
    /// # Safety Contract
    ///
    /// Callers must ensure:
    /// - The `Terminal` reference is not accessed concurrently from other threads
    ///   while this snapshot is being built
    /// - The returned `SharedState` must not outlive any external references to its
    ///   raw pointer fields (`title`, `cwd`, `cells`)
    /// - The `cells` pointer is valid for `cell_count` elements only
    /// - The `title` pointer (if non-null) is a NUL-terminated C string of `title_len` bytes
    /// - The `cwd` pointer (if non-null) is a NUL-terminated C string of `cwd_len` bytes
    /// - Only one `SharedState` should be built from a `Terminal` at a time to prevent
    ///   data races on the underlying grid state
    pub fn from_terminal(term: &Terminal) -> Self {
        let grid = term.active_grid();
        let cols = grid.cols();
        let rows = grid.rows();
        let cursor = term.cursor();

        let mouse_mode = mouse_mode_code(term.mouse_mode());

        // Title
        let (title, title_len) = to_c_string(term.title());

        // CWD
        let (cwd, cwd_len) = match term.current_directory() {
            Some(s) => to_c_string(s),
            None => (std::ptr::null_mut(), 0u32),
        };

        // Cells
        let mut cells_vec: Vec<SharedCell> = Vec::with_capacity(cols * rows);
        let pad = SharedCell::padding(term);

        for row_idx in 0..rows {
            if let Some(row_cells) = grid.row(row_idx) {
                for col_idx in 0..cols {
                    let cell = row_cells
                        .get(col_idx)
                        .map(|c| SharedCell::from_cell(term, c))
                        .unwrap_or_else(|| pad.clone());
                    cells_vec.push(cell);
                }
            } else {
                // Row doesn't exist — fill with default cells
                for _ in 0..cols {
                    cells_vec.push(pad.clone());
                }
            }
        }

        // The count comes from the allocation itself, so `Drop` rebuilds
        // exactly the slice `Box::into_raw` released (QA-215).
        let cells: Box<[SharedCell]> = cells_vec.into_boxed_slice();
        let cell_count = u32::try_from(cells.len()).expect("grid cell count fits in u32");
        let cells = Box::into_raw(cells).cast::<SharedCell>();

        // Scrollback stats
        let sb = term.scrollback_stats();

        SharedState {
            cols: cols as u32,
            rows: rows as u32,
            cursor_col: cursor.col as u32,
            cursor_row: cursor.row as u32,
            cursor_visible: cursor.visible,
            alt_screen_active: term.is_alt_screen_active(),
            mouse_mode,
            title,
            title_len,
            cwd,
            cwd_len,
            cells,
            cell_count,
            scrollback_lines: sb.total_lines as u32,
            total_lines: (sb.total_lines + rows) as u32,
        }
    }
}

impl Drop for SharedState {
    fn drop(&mut self) {
        // Free the title CString
        if !self.title.is_null() {
            // SAFETY: `title` came from `CString::into_raw` in `to_c_string`;
            // nulling it below makes this the only reconstruction.
            unsafe {
                let _ = CString::from_raw(self.title);
            }
            self.title = std::ptr::null_mut();
        }

        // Free the cwd CString
        if !self.cwd.is_null() {
            // SAFETY: as for `title` — a non-null `cwd` is always a
            // `to_c_string` allocation, freed exactly once.
            unsafe {
                let _ = CString::from_raw(self.cwd);
            }
            self.cwd = std::ptr::null_mut();
        }

        // Free the cells array. A zero-length box round-trips through
        // into_raw/from_raw as a dangling non-null pointer, so no length
        // guard is needed.
        if !self.cells.is_null() {
            let slice = std::ptr::slice_from_raw_parts_mut(self.cells, self.cell_count as usize);
            // SAFETY: `cells` and `cell_count` came from one
            // `Box::<[SharedCell]>::into_raw` in `from_terminal`; nulling the
            // pointer below makes this the only reconstruction.
            drop(unsafe { Box::from_raw(slice) });
            self.cells = std::ptr::null_mut();
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalObserverVtable — C function-pointer table for observers
// ---------------------------------------------------------------------------

/// The callback shape shared by the text slots of [`TerminalObserverVtable`]:
/// receives the vtable's `user_data` and a NUL-terminated, Debug-formatted
/// event description valid only for the duration of the call. A named alias
/// (not an inline type) so cbindgen emits the `term_event_cb` typedef the
/// C header promises (ENH-027).
// snake_case on purpose: the name is the C typedef the header exports.
#[allow(nonstandard_style)]
pub type term_event_cb =
    Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>;

/// The structured-event callback of [`TerminalObserverVtable::on_event_v2`]
/// (ARC-114): receives the vtable's `user_data` and one [`TermEvent`], both
/// valid only for the duration of the call.
// snake_case on purpose: the name is the C typedef the header exports.
#[allow(nonstandard_style)]
pub type term_event_v2_cb =
    Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event: *const TermEvent)>;

/// One terminal event in structured form, delivered to `on_event_v2`
/// (ARC-114).
///
/// `kind` is a `TERM_EVENT_*` code (terminal_core_layout.h). `payload` is a
/// UTF-8 JSON object of `payload_len` bytes, **not** NUL-terminated: text
/// fields are JSON strings, so an interior NUL arrives escaped (`\u0000`)
/// instead of truncating the payload. Its keys are the event's named fields
/// (the same keys as the Python event dicts, including `"type"`); an unset
/// optional field is present as `null`. The key set is part of the
/// `TERM_CORE_ABI_VERSION` contract. The struct and the bytes it points to
/// are owned by the library and valid only during the callback — copy what
/// you need.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TermEvent {
    /// `TERM_EVENT_*` kind code
    pub kind: u16,
    /// Reserved, always 0
    pub _pad: u16,
    /// Length of `payload` in bytes
    pub payload_len: u32,
    /// UTF-8 JSON object, `payload_len` bytes, not NUL-terminated
    pub payload: *const u8,
}

/// A C-compatible vtable for terminal event observation.
///
/// The five text slots receive the `user_data` pointer and a Debug-formatted
/// (`{:?}`) event description as a NUL-terminated C string. That text is
/// DIAGNOSTIC, not a stable format: it changes whenever the Rust event enum
/// changes — parse it only for logging. An interior NUL in the text is
/// replaced with U+FFFD, never dropped. The callee must NOT free the event
/// string — it is owned by the caller and valid only for the duration of the
/// callback.
///
/// `on_event_v2` is the structured channel (ARC-114): one [`TermEvent`] per
/// event, carrying a `TERM_EVENT_*` kind and a length-delimited JSON payload.
/// Any slot may be NULL; a NULL slot costs nothing (its text or JSON is never
/// built).
///
/// Callbacks fire inline while the terminal is processing input or
/// resizing (a resize can scroll zones out). A callback must NOT re-enter
/// the FFI on the same `Terminal` handle (any `ptec_terminal_*` function):
/// the terminal is mutably borrowed for the duration of the dispatch, so
/// re-entry aliases `&`/`&mut` — undefined behavior. Queue what you need
/// and call back after `ptec_terminal_feed` / `ptec_terminal_resize`
/// returns.
#[repr(C)]
pub struct TerminalObserverVtable {
    /// Called for zone lifecycle events (diagnostic text)
    pub on_zone_event: term_event_cb,
    /// Called for command/shell integration events (diagnostic text)
    pub on_command_event: term_event_cb,
    /// Called for environment change events (diagnostic text)
    pub on_environment_event: term_event_cb,
    /// Called for screen content events (diagnostic text)
    pub on_screen_event: term_event_cb,
    /// Called for ALL events (catch-all, diagnostic text)
    pub on_event: term_event_cb,
    /// Called for ALL events with the structured [`TermEvent`] (ARC-114)
    pub on_event_v2: term_event_v2_cb,
    /// Opaque pointer passed to every callback
    pub user_data: *mut std::ffi::c_void,
}

/// Declares the `TERM_EVENT_*` code of every [`TerminalEventKind`] once: the
/// exhaustive match forces a code for a new kind, and the same invocation
/// feeds the header-define test, so the C constant cannot be forgotten.
/// Codes are stable ABI — never renumber; append new kinds.
macro_rules! term_event_kinds {
    ($($variant:ident = $code:literal => $cname:literal),* $(,)?) => {
        /// The `TERM_EVENT_*` code for an event kind (ARC-114).
        fn event_kind_code(kind: &TerminalEventKind) -> u16 {
            match kind {
                $(TerminalEventKind::$variant => $code,)*
            }
        }

        /// Every (`TERM_EVENT_*` name, code) pair, for the header pin test.
        #[cfg(test)]
        const TERM_EVENT_CODES: &[(&str, u16)] = &[$(($cname, $code)),*];
    };
}

term_event_kinds! {
    BellRang = 1 => "TERM_EVENT_BELL",
    TitleChanged = 2 => "TERM_EVENT_TITLE_CHANGED",
    SizeChanged = 3 => "TERM_EVENT_SIZE_CHANGED",
    ModeChanged = 4 => "TERM_EVENT_MODE_CHANGED",
    GraphicsAdded = 5 => "TERM_EVENT_GRAPHICS_ADDED",
    HyperlinkAdded = 6 => "TERM_EVENT_HYPERLINK_ADDED",
    DirtyRegion = 7 => "TERM_EVENT_DIRTY_REGION",
    CwdChanged = 8 => "TERM_EVENT_CWD_CHANGED",
    TriggerMatched = 9 => "TERM_EVENT_TRIGGER_MATCHED",
    UserVarChanged = 10 => "TERM_EVENT_USER_VAR_CHANGED",
    ProgressBarChanged = 11 => "TERM_EVENT_PROGRESS_BAR_CHANGED",
    BadgeChanged = 12 => "TERM_EVENT_BADGE_CHANGED",
    ShellIntegrationEvent = 13 => "TERM_EVENT_SHELL_INTEGRATION",
    ZoneOpened = 14 => "TERM_EVENT_ZONE_OPENED",
    ZoneClosed = 15 => "TERM_EVENT_ZONE_CLOSED",
    ZoneScrolledOut = 16 => "TERM_EVENT_ZONE_SCROLLED_OUT",
    EnvironmentChanged = 17 => "TERM_EVENT_ENVIRONMENT_CHANGED",
    RemoteHostTransition = 18 => "TERM_EVENT_REMOTE_HOST_TRANSITION",
    SubShellDetected = 19 => "TERM_EVENT_SUB_SHELL_DETECTED",
    FileTransferStarted = 20 => "TERM_EVENT_FILE_TRANSFER_STARTED",
    FileTransferProgress = 21 => "TERM_EVENT_FILE_TRANSFER_PROGRESS",
    FileTransferCompleted = 22 => "TERM_EVENT_FILE_TRANSFER_COMPLETED",
    FileTransferFailed = 23 => "TERM_EVENT_FILE_TRANSFER_FAILED",
    UploadRequested = 24 => "TERM_EVENT_UPLOAD_REQUESTED",
    ScreenCleared = 25 => "TERM_EVENT_SCREEN_CLEARED",
    InlineImageDropped = 26 => "TERM_EVENT_INLINE_IMAGE_DROPPED",
}

/// The `on_event_v2` JSON payload for an event: an object of the event's
/// named fields (`event_fields`, shared with the Python dicts), `null` for an
/// unset optional.
fn event_payload_json(event: &TerminalEvent) -> Vec<u8> {
    use crate::terminal::event_fields::{event_fields, EventField};
    use serde_json::Value;
    let mut map = serde_json::Map::new();
    for (key, field) in event_fields(event) {
        let value = match field {
            EventField::Str(s) => Value::String(s),
            EventField::Int(i) => Value::from(i),
            EventField::Bool(b) => Value::Bool(b),
            EventField::None => Value::Null,
        };
        map.insert(key, value);
    }
    serde_json::to_vec(&Value::Object(map)).expect("a JSON object of scalars always serializes")
}

// SAFETY: The user_data pointer is opaque and the FFI contract requires the
// caller to ensure thread safety of the data it points to.
unsafe impl Send for TerminalObserverVtable {}
// SAFETY: same contract as `Send`: the vtable is never mutated after
// registration, and the caller owns any synchronization `user_data` needs.
unsafe impl Sync for TerminalObserverVtable {}

// ---------------------------------------------------------------------------
// FfiObserver — bridges TerminalObserverVtable to the Rust trait
// ---------------------------------------------------------------------------

/// An observer implementation that delegates to C function pointers.
pub struct FfiObserver {
    vtable: TerminalObserverVtable,
}

impl FfiObserver {
    /// Create a new `FfiObserver` from a vtable.
    pub fn new(vtable: TerminalObserverVtable) -> Self {
        Self { vtable }
    }

    /// Format a terminal event as a Debug-formatted (`{:?}`) string and call
    /// an FFI text callback with it. An interior NUL is replaced (U+FFFD)
    /// rather than dropping the event.
    fn call_callback(&self, cb: term_event_cb, event: &TerminalEvent) {
        if let Some(f) = cb {
            let (ptr, _len) = to_c_string(&format!("{:?}", event));
            // SAFETY: `ptr` came from `CString::into_raw` just above and is
            // reclaimed exactly once, after the call.
            let cstr = unsafe { CString::from_raw(ptr) };
            // SAFETY: `f` and `user_data` come from the caller's vtable,
            // which `ptec_terminal_add_observer`'s contract keeps valid while
            // registered; `cstr` outlives the call.
            unsafe {
                f(self.vtable.user_data, cstr.as_ptr());
            }
        }
    }

    /// Deliver the structured [`TermEvent`] to `on_event_v2`, if set.
    fn call_v2(&self, event: &TerminalEvent) {
        if let Some(f) = self.vtable.on_event_v2 {
            let payload = event_payload_json(event);
            let ev = TermEvent {
                kind: event_kind_code(&event.kind()),
                _pad: 0,
                payload_len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
                payload: payload.as_ptr(),
            };
            // SAFETY: `f` and `user_data` come from the caller's vtable,
            // which `ptec_terminal_add_observer`'s contract keeps valid while
            // registered; `ev` and `payload` outlive the call.
            unsafe {
                f(self.vtable.user_data, &ev);
            }
        }
    }
}

impl TerminalObserver for FfiObserver {
    fn on_zone_event(&self, event: &TerminalEvent) {
        self.call_callback(self.vtable.on_zone_event, event);
    }

    fn on_command_event(&self, event: &TerminalEvent) {
        self.call_callback(self.vtable.on_command_event, event);
    }

    fn on_environment_event(&self, event: &TerminalEvent) {
        self.call_callback(self.vtable.on_environment_event, event);
    }

    fn on_screen_event(&self, event: &TerminalEvent) {
        self.call_callback(self.vtable.on_screen_event, event);
    }

    fn on_event(&self, event: &TerminalEvent) {
        self.call_callback(self.vtable.on_event, event);
        self.call_v2(event);
    }

    fn subscriptions(&self) -> Option<&HashSet<TerminalEventKind>> {
        // FFI observers receive all events — no filtering
        None
    }
}

// ---------------------------------------------------------------------------
// Embedding surface — lifecycle, feed, damage, pinned readback, key encoding
// ---------------------------------------------------------------------------

/// An inclusive [start, end] range of dirty screen rows, in coalesced form.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermRowRange {
    /// First dirty row (0-indexed, inclusive)
    pub start: u32,
    /// Last dirty row (0-indexed, inclusive)
    pub end: u32,
}

/// `TermScrollDelta.flags` bit 0 (ENH-038): the scroll movement since the
/// generation cannot be expressed — a screen switch, resize or reflow, RIS,
/// snapshot restore, scrollback clear, scroll-log overflow, or mixed scroll
/// regions happened. The renderer must treat every row as dirty and skip
/// the blit (today's full-redraw behavior).
pub const TERM_SCROLL_FULL_REDRAW: u32 = 1;

/// Scroll-aware damage report (ENH-038, ABI v5 additive): how the visible
/// content moved since a generation. When `flags` has no
/// `TERM_SCROLL_FULL_REDRAW` bit, a renderer can blit its previous frame's
/// `[top, bottom]` region by `delta` rows and then redraw only the rows
/// `ptec_terminal_content_dirty_ranges_since` reports — rows vacated by the
/// blit always carry a fresh content generation and are in that set.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermScrollDelta {
    /// Net rows the region content moved; positive means content moved up.
    pub delta: i32,
    /// First row of the scrolled region (0-indexed, inclusive)
    pub top: u32,
    /// Last row of the scrolled region (0-indexed, inclusive)
    pub bottom: u32,
    /// Bit flags; bit 0 is `TERM_SCROLL_FULL_REDRAW`.
    pub flags: u32,
}

/// Cursor position and style, C-compatible.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCursorState {
    /// Cursor column (0-indexed).
    pub col: u32,
    /// Cursor row (0-indexed).
    pub row: u32,
    /// Whether the cursor is visible (DECTCEM)
    pub visible: bool,
    /// Cursor style code: 0 blinking block, 1 steady block, 2 blinking
    /// underline, 3 steady underline, 4 blinking bar, 5 steady bar
    pub style: u8,
}

/// Terminal mode state a renderer needs per frame, C-compatible.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TermModeState {
    /// Alternate screen buffer active
    pub alt_screen: bool,
    /// Bracketed paste mode (paste input should be wrapped in ESC[200~…201~)
    pub bracketed_paste: bool,
    /// Application cursor keys (arrows encode as SS3, not CSI)
    pub application_cursor: bool,
    /// Origin mode (DECOM) — cursor addresses are scroll-region-relative
    pub origin_mode: bool,
    /// Insert mode (IRM) — typed characters shift the row right
    pub insert_mode: bool,
    /// Autowrap (DECAWM)
    pub auto_wrap: bool,
    /// Mouse tracking mode (0=Off, 1=X10, 2=Normal, 3=ButtonEvent, 4=AnyEvent)
    pub mouse_mode: u8,
    /// Kitty keyboard protocol progressive-enhancement flags
    pub kitty_flags: u16,
    /// Grid width in columns.
    pub cols: u32,
    /// Grid height in rows.
    pub rows: u32,
}

impl SharedCell {
    /// Build a `SharedCell` from one grid cell, with colors resolved through
    /// `term`'s palette and defaults ([`Terminal::resolve_cell_colors`]).
    fn from_cell(term: &Terminal, cell: &crate::cell::Cell) -> Self {
        let mut text = [0u8; 4];
        let text_len = cell.c.encode_utf8(&mut text).len() as u8;
        let colors = term.resolve_cell_colors(cell);
        let (fg_r, fg_g, fg_b) = colors.fg;
        let (bg_r, bg_g, bg_b) = colors.bg;
        let mut attrs = cell.flags.to_bitflags();
        if colors.default_fg {
            attrs |= attr_bits::DEFAULT_FG;
        }
        if colors.default_bg {
            attrs |= attr_bits::DEFAULT_BG;
        }
        if cell.has_combining_chars() {
            attrs |= attr_bits::HAS_COMBINING;
        }
        SharedCell {
            text,
            text_len,
            fg_r,
            fg_g,
            fg_b,
            bg_r,
            bg_g,
            bg_b,
            attrs,
            width: cell.width,
        }
    }

    /// The cell that pads a line shorter than the grid: an unstyled space,
    /// resolved like any real blank cell (default colors and bits).
    fn padding(term: &Terminal) -> Self {
        Self::from_cell(term, &crate::cell::Cell::default())
    }

    /// A placeholder value for initializing caller-side buffers before a
    /// read. It is not what the readback writes for a blank cell: that is
    /// resolved through the terminal's default colors and carries the
    /// `TERM_ATTR_DEFAULT_*` bits.
    pub fn blank() -> Self {
        SharedCell {
            text: [b' ', 0, 0, 0],
            text_len: 1,
            fg_r: 255,
            fg_g: 255,
            fg_b: 255,
            bg_r: 0,
            bg_g: 0,
            bg_b: 0,
            attrs: 0,
            width: 1,
        }
    }
}

/// The `TERM_MOUSE_MODE_*` code for a mouse mode (0 Off, 1 X10, 2 Normal,
/// 3 ButtonEvent, 4 AnyEvent). The discriminants are the codes;
/// `layout_header_defines_match_rust` pins them to the header.
fn mouse_mode_code(mode: MouseMode) -> u8 {
    mode as u8
}

/// Copy one grid line into a caller buffer — the shared body of
/// `ptec_terminal_read_row` and `ptec_terminal_read_scrollback_row`. With `out` NULL
/// returns the cells available from `col_start` (the sizing answer);
/// otherwise copies `cells[col_start..cols]` into `out` (up to `cap`),
/// padding a line shorter than `cols` with resolved blank cells, and returns
/// the number written.
///
/// # Safety
/// `out` must be NULL or valid for writes of `cap` `SharedCell` values.
unsafe fn copy_row(
    term: &Terminal,
    cells: &[crate::cell::Cell],
    cols: u32,
    col_start: u32,
    out: *mut SharedCell,
    cap: u32,
) -> u32 {
    if out.is_null() {
        return cols.saturating_sub(col_start);
    }
    let mut written = 0u32;
    let mut col = col_start;
    while col < cols && written < cap {
        let cell = cells
            .get(col as usize)
            .map(|c| SharedCell::from_cell(term, c))
            .unwrap_or_else(|| SharedCell::padding(term));
        // SAFETY: `written < cap`, and the caller guarantees `out` is valid
        // for `cap` writes.
        unsafe { out.add(written as usize).write(cell) };
        col += 1;
        written += 1;
    }
    written
}

/// Write the ranges `each` visits into `out` (up to `cap`) and return the
/// total visited — the shared body of the two dirty-range calls. `out` may
/// be NULL for a sizing call. Allocation-free: the render loop calls it
/// twice per frame (ENH-026, pinned by tests/ffi_dirty_ranges_alloc.rs).
///
/// # Safety
/// `out` must be NULL or valid for writes of `cap` `TermRowRange` values.
unsafe fn write_ranges(
    out: *mut TermRowRange,
    cap: u32,
    each: impl FnOnce(&mut dyn FnMut(u32, u32)),
) -> u32 {
    let mut total: u32 = 0;
    each(&mut |start, end| {
        if total < cap && !out.is_null() {
            // SAFETY: `total < cap` and `out` is non-null, and the caller
            // guarantees `out` is valid for `cap` writes.
            unsafe { out.add(total as usize).write(TermRowRange { start, end }) };
        }
        total += 1;
    });
    total
}

/// ABI version of the C surface (ARC-063). Must equal
/// `TERM_CORE_ABI_VERSION` in include/terminal_core_layout.h (included from
/// the cbindgen-generated include/terminal_core.h); bump both on any layout
/// or contract change. Version 4 (breaking, D4): the `ptec_` symbol prefix
/// (ARC-112), palette-resolved `SharedCell` colors with the `TERM_ATTR_*`
/// bits and `ptec_terminal_read_cell_grapheme` (ARC-101), the `on_event_v2`
/// vtable slot and `TermEvent` (ARC-114), and
/// `ptec_terminal_scrollback_total_scrolled`, which shipped while the
/// constant still read 3. Version 5 (additive, ENH-038):
/// scroll-aware damage — `TermScrollDelta`,
/// `ptec_terminal_scroll_delta_since`, `ptec_terminal_content_dirty_ranges_since`,
/// and `TERM_SCROLL_FULL_REDRAW`.
pub const TERM_CORE_ABI_VERSION: u32 = 5;

/// The ABI version this library implements. A binary detects a mismatch
/// by comparing this call's return against its compiled-in header macro.
/// Safe to call at any time — it touches no terminal state.
#[no_mangle]
pub extern "C" fn ptec_terminal_abi_version() -> u32 {
    TERM_CORE_ABI_VERSION
}

/// Create a terminal for C embedding.
///
/// Returns NULL when `cols` or `rows` is 0. Allocation failure aborts the
/// process — this surface has no error channel, so OOM is fatal by design.
///
/// # Safety
/// Caller owns the returned `Terminal` and must release it with
/// `ptec_terminal_free`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_create(
    cols: u32,
    rows: u32,
    scrollback: u32,
) -> *mut Terminal {
    if cols == 0 || rows == 0 {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(Terminal::with_scrollback(
        cols as usize,
        rows as usize,
        scrollback as usize,
    )))
}

/// Free a `Terminal` created by `ptec_terminal_create`.
///
/// # Safety
/// `term` must have been returned by `ptec_terminal_create` and must not be
/// used after this call.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_free(term: *mut Terminal) {
    if !term.is_null() {
        // SAFETY: non-null, and per this fn's contract it came from
        // `ptec_terminal_create`'s `Box::into_raw` and is not used afterwards.
        drop(unsafe { Box::from_raw(term) });
    }
}

/// Feed raw PTY/application output bytes into the terminal (VT parsing).
///
/// # Safety
/// `bytes` must be valid for reads of `len` bytes. `term` must be a valid
/// pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_feed(term: *mut Terminal, bytes: *const u8, len: u32) {
    if term.is_null() || bytes.is_null() {
        return;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` with no other live borrow.
    let term_ref = unsafe { &mut *term };
    // SAFETY: `bytes` is non-null (checked above) and the caller guarantees
    // it is valid for reads of `len` bytes.
    let data = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    term_ref.process(data);
}

/// Resize the terminal grid. A zero `cols` or `rows` is a no-op.
///
/// Observers receive any `ZoneScrolledOut` events the resize causes
/// before this returns (same no-re-entry rule as `ptec_terminal_feed`).
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_resize(term: *mut Terminal, cols: u32, rows: u32) {
    if term.is_null() || cols == 0 || rows == 0 {
        return;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` with no other live borrow.
    let term_ref = unsafe { &mut *term };
    term_ref.resize(cols as usize, rows as usize);
}

/// Coalesce ascending row numbers into inclusive ranges.
///
/// Reference algorithm for the `for_each_dirty_range` equivalence tests —
/// the FFI entry points coalesce in place since ENH-026.
#[cfg(test)]
fn coalesce_row_ranges(rows: impl Iterator<Item = usize>) -> Vec<TermRowRange> {
    let mut ranges: Vec<TermRowRange> = Vec::new();
    for row in rows {
        let row = row as u32;
        match ranges.last_mut() {
            // Consecutive rows coalesce; anything else starts a new range.
            Some(last) if last.end + 1 == row => last.end = row,
            _ => ranges.push(TermRowRange {
                start: row,
                end: row,
            }),
        }
    }
    ranges
}

/// Coalesce the dirty-row generations into inclusive row ranges.
///
/// Writes up to `cap` ranges into `out` (caller-owned) and returns the
/// total range count — if the return exceeds `cap`, call again with a
/// larger buffer. A renderer redraws only rows inside the returned ranges.
/// Serves the built-in default consumer; `ptec_terminal_mark_clean` advances it.
///
/// # Safety
/// `out` must be valid for writes of `cap` `TermRowRange` values. It may
/// be NULL only as a sizing call, with `cap` 0 — the call then just
/// returns the total range count.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_dirty_ranges(
    term: *const Terminal,
    out: *mut TermRowRange,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    // SAFETY: the caller guarantees `out` is NULL or valid for `cap` writes,
    // which is `write_ranges`' contract.
    unsafe { write_ranges(out, cap, |f| term_ref.for_each_dirty_range(f)) }
}

/// Current damage generation. A renderer remembers this value between
/// frames and passes it to `ptec_terminal_dirty_ranges_since` to observe only
/// what changed since — independent of `ptec_terminal_mark_clean`, which
/// advances the default consumer's generation and cannot hide damage from
/// generation consumers (ENH-025).
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_damage_generation(term: *const Terminal) -> u64 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    unsafe { &*term }.damage_generation()
}

/// Coalesced dirty-row ranges since generation `gen`, from an earlier
/// `ptec_terminal_damage_generation` call. Same buffer contract as
/// `ptec_terminal_dirty_ranges`: fills up to `cap` ranges into `out` and
/// returns the total count; `out` may be NULL with `cap` 0 as a sizing
/// call. A screen switch dirties every row of the newly visible grid.
///
/// # Safety
/// `out` must be valid for writes of `cap` `TermRowRange` values, or NULL
/// with `cap` 0 for a sizing call. `term` must be a valid pointer to a
/// `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_dirty_ranges_since(
    term: *const Terminal,
    gen: u64,
    out: *mut TermRowRange,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    // SAFETY: the caller guarantees `out` is NULL or valid for `cap` writes,
    // which is `write_ranges`' contract.
    unsafe { write_ranges(out, cap, |f| term_ref.for_each_dirty_range_since(gen, f)) }
}

/// Scroll-aware damage since generation `gen` (ENH-038): writes how the
/// visible content moved into `*out` and returns true. `flags` bit 0
/// (`TERM_SCROLL_FULL_REDRAW`) marks the fallback contract: blitting is
/// unsafe, treat every row as dirty. Returns false only when `term` or
/// `out` is null.
///
/// # Safety
/// `out` must be a valid pointer to a `TermScrollDelta`. `term` must be a
/// valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_scroll_delta_since(
    term: *const Terminal,
    gen: u64,
    out: *mut TermScrollDelta,
) -> bool {
    if term.is_null() || out.is_null() {
        return false;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let report = term_ref.scroll_damage_since(gen);
    // SAFETY: `out` is non-null (checked above) and the caller guarantees
    // it points to a writable `TermScrollDelta`.
    unsafe {
        out.write(TermScrollDelta {
            delta: report.delta,
            top: report.top,
            bottom: report.bottom,
            flags: if report.full_redraw {
                TERM_SCROLL_FULL_REDRAW
            } else {
                0
            },
        });
    }
    true
}

/// Coalesced *content*-dirty row ranges since generation `gen` (ENH-038):
/// only the rows whose content changed — the blit-complement of
/// `ptec_terminal_scroll_delta_since`. After blitting the reported region
/// by the reported delta, redraw exactly these rows. Same buffer contract
/// as `ptec_terminal_dirty_ranges_since` (NULL/0 sizing call, total-count
/// return). A screen switch dirties the content of every row of the newly
/// visible grid for any older generation.
///
/// # Safety
/// `out` must be valid for writes of `cap` `TermRowRange` values, or NULL
/// with `cap` 0 for a sizing call. `term` must be a valid pointer to a
/// `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_content_dirty_ranges_since(
    term: *const Terminal,
    gen: u64,
    out: *mut TermRowRange,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    // SAFETY: the caller guarantees `out` is NULL or valid for `cap` writes,
    // which is `write_ranges`' contract.
    unsafe {
        write_ranges(out, cap, |f| {
            term_ref.for_each_content_dirty_range_since(gen, f)
        })
    }
}

/// Mark the screen clean (all damage consumed).
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_mark_clean(term: *mut Terminal) {
    if term.is_null() {
        return;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` with no other live borrow.
    unsafe { &mut *term }.mark_clean();
}

/// Copy a run of grid cells into a caller-owned buffer (pinned readback —
/// no allocation, no full-grid copy). Returns the number of cells written;
/// with `out` NULL it returns the number of cells available from
/// `col_start` (the sizing answer) instead. Reads target the active grid,
/// so while the alternate screen is active there is no scrollback.
///
/// # Safety
/// `out` must be valid for writes of `cap` `SharedCell` values, or NULL
/// with `cap` 0 for a sizing call.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_read_row(
    term: *const Terminal,
    row: u32,
    col_start: u32,
    out: *mut SharedCell,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let grid = term_ref.active_grid();
    let Some(row_cells) = grid.row(row as usize) else {
        return 0;
    };
    // SAFETY: the caller guarantees `out` is NULL or valid for `cap` writes,
    // which is `copy_row`'s contract.
    unsafe { copy_row(term_ref, row_cells, grid.cols() as u32, col_start, out, cap) }
}

/// Copy a run of scrollback cells into a caller-owned buffer.
/// `line` indexes scrollback from the oldest (0) to the newest
/// (`scrollback_count - 1`). Returns the number of cells written; with
/// `out` NULL it returns the number of cells available from `col_start`
/// (the sizing answer) instead.
///
/// # Safety
/// `out` must be valid for writes of `cap` `SharedCell` values, or NULL
/// with `cap` 0 for a sizing call.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_read_scrollback_row(
    term: *const Terminal,
    line: u32,
    col_start: u32,
    out: *mut SharedCell,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let grid = term_ref.active_grid();
    let Some(line_cells) = grid.scrollback_line(line as usize) else {
        return 0;
    };
    // SAFETY: the caller guarantees `out` is NULL or valid for `cap` writes,
    // which is `copy_row`'s contract.
    unsafe {
        copy_row(
            term_ref,
            line_cells,
            grid.cols() as u32,
            col_start,
            out,
            cap,
        )
    }
}

/// Copy the full grapheme cluster of one screen cell — the base character
/// plus every combining mark — as UTF-8 into a caller-owned buffer
/// (ARC-101). `SharedCell.text` holds only the base character; a cell whose
/// `attrs` has `TERM_ATTR_HAS_COMBINING` needs this call for the rest.
///
/// Writes up to `cap` bytes (no NUL terminator) and returns the cluster's
/// total byte length — if the return exceeds `cap`, call again with a larger
/// buffer; `out` NULL with `cap` 0 is the sizing call. Returns 0 for a
/// position outside the active grid. Same active-grid addressing as
/// `ptec_terminal_read_row`.
///
/// # Safety
/// `out` must be valid for writes of `cap` bytes, or NULL with `cap` 0 for a
/// sizing call. `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_read_cell_grapheme(
    term: *const Terminal,
    row: u32,
    col: u32,
    out: *mut u8,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let Some(cell) = term_ref.active_grid().get(col as usize, row as usize) else {
        return 0;
    };
    let mut buf = [0u8; 4];
    let mut total = cell.c.encode_utf8(&mut buf).len();
    for ch in cell.combining() {
        total += ch.len_utf8();
    }
    if !out.is_null() && cap > 0 {
        let mut written = 0usize;
        let cap = cap as usize;
        for ch in std::iter::once(cell.c).chain(cell.combining().iter().copied()) {
            let bytes = ch.encode_utf8(&mut buf).as_bytes();
            let take = bytes.len().min(cap - written);
            // SAFETY: `written + take <= cap`, `out` is non-null, and the
            // caller guarantees `out` is valid for `cap` byte writes; `buf`
            // is a stack array, so the ranges cannot overlap.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.add(written), take) };
            written += take;
            if written == cap {
                break;
            }
        }
    }
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// Number of lines currently held in scrollback.
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_scrollback_count(term: *const Terminal) -> u32 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    unsafe { &*term }.active_grid().scrollback_len() as u32
}

/// Total lines ever pushed into scrollback.
///
/// Monotone within a buffer's lifetime; `clear_scrollback` resets it to 0
/// together with the count. An embedder mirroring the scrollback window
/// pairs this with `ptec_terminal_scrollback_count`: the window holds lines
/// `[total - count, total)`, so head evictions and tail appends are both
/// derivable per frame, including when the ring is full and the count
/// alone stops moving. This counts buffer entries only — alt-screen and
/// in-region scrolls that never reach scrollback do not move it, which is
/// why it is a window-sync cursor and not a scroll-damage source (ENH-038
/// rejects that use).
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_scrollback_total_scrolled(term: *const Terminal) -> u64 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    unsafe { &*term }.active_grid().total_lines_scrolled() as u64
}

/// Read cursor position/style.
///
/// # Safety
/// `out` must be valid for writes of one `TermCursorState`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_get_cursor(
    term: *const Terminal,
    out: *mut TermCursorState,
) {
    if term.is_null() || out.is_null() {
        return;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let cursor = term_ref.cursor();
    let style = match cursor.style {
        crate::cursor::CursorStyle::BlinkingBlock => 0u8,
        crate::cursor::CursorStyle::SteadyBlock => 1,
        crate::cursor::CursorStyle::BlinkingUnderline => 2,
        crate::cursor::CursorStyle::SteadyUnderline => 3,
        crate::cursor::CursorStyle::BlinkingBar => 4,
        crate::cursor::CursorStyle::SteadyBar => 5,
    };
    // SAFETY: `out` is non-null (checked above) and the caller guarantees it
    // is valid for one `TermCursorState` write.
    unsafe {
        *out = TermCursorState {
            col: cursor.col as u32,
            row: cursor.row as u32,
            visible: cursor.visible,
            style,
        };
    }
}

/// Read per-frame mode state.
///
/// # Safety
/// `out` must be valid for writes of one `TermModeState`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_get_modes(term: *const Terminal, out: *mut TermModeState) {
    if term.is_null() || out.is_null() {
        return;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let mouse_mode = mouse_mode_code(term_ref.mouse_mode());
    let (cols, rows) = term_ref.size();
    // SAFETY: `out` is non-null (checked above) and the caller guarantees it
    // is valid for one `TermModeState` write.
    unsafe {
        *out = TermModeState {
            alt_screen: term_ref.is_alt_screen_active(),
            bracketed_paste: term_ref.bracketed_paste(),
            application_cursor: term_ref.application_cursor(),
            origin_mode: term_ref.origin_mode(),
            insert_mode: term_ref.insert_mode(),
            auto_wrap: term_ref.auto_wrap_mode(),
            mouse_mode,
            kitty_flags: term_ref.keyboard_flags(),
            cols: cols as u32,
            rows: rows as u32,
        };
    }
}

/// Encode a key event against the terminal's negotiated input state
/// (application cursor keys, kitty keyboard flags). Writes up to `cap`
/// bytes into `out` (caller-owned) and returns the total encoded length —
/// if the return exceeds `cap`, call again with a larger buffer. The
/// returned bytes are what a frontend would write to the PTY.
///
/// # Safety
/// `ev` must be valid for reads of one `TermKeyEvent`; `out` must be valid
/// for writes of `cap` bytes, or NULL with `cap` 0 to fetch the total
/// encoded length.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_encode_key(
    term: *const Terminal,
    ev: *const crate::keyboard::TermKeyEvent,
    out: *mut u8,
    cap: u32,
) -> u32 {
    // SAFETY: forwards this fn's own contract; a NULL `opts` is allowed by
    // `ptec_terminal_encode_key_ex`.
    unsafe { ptec_terminal_encode_key_ex(term, ev, std::ptr::null(), out, cap) }
}

/// [`ptec_terminal_encode_key`] with explicit macOS Option-key modes
/// (`TermKeyOptions`, ENH-028). Pass NULL `opts` for the defaults (ESC
/// prefix on both sides — the classic xterm Alt behavior, identical to
/// `ptec_terminal_encode_key`). A zeroed struct means Normal passthrough on
/// both sides; see `terminal_core_layout.h` for the mode values.
///
/// # Safety
/// `ev` must be valid for reads of one `TermKeyEvent`; `opts`, when not
/// NULL, must be valid for reads of one `TermKeyOptions`; `out` must be
/// valid for writes of `cap` bytes, or NULL with `cap` 0 to fetch the
/// total encoded length.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_encode_key_ex(
    term: *const Terminal,
    ev: *const crate::keyboard::TermKeyEvent,
    opts: *const crate::keyboard::KeyEncodeOptions,
    out: *mut u8,
    cap: u32,
) -> u32 {
    if term.is_null() || ev.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    // SAFETY: `ev` is non-null (checked above) and the caller guarantees it
    // is valid for reads of one `TermKeyEvent`.
    let ev_ref = unsafe { &*ev };
    // SAFETY: `as_ref` maps NULL to `None`; otherwise the caller guarantees
    // `opts` is valid for reads of one `TermKeyOptions`.
    let opts_ref = match unsafe { opts.as_ref() } {
        Some(o) => o,
        None => &crate::keyboard::KeyEncodeOptions::default(),
    };
    let bytes = crate::keyboard::encode_key_with(ev_ref, term_ref, opts_ref);
    let fill = (bytes.len() as u32).min(cap);
    if fill > 0 && !out.is_null() {
        // SAFETY: `fill <= cap`, `out` is non-null, and the caller guarantees
        // `out` is valid for `cap` byte writes; `bytes` is a separate
        // allocation, so the ranges cannot overlap.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, fill as usize) };
    }
    bytes.len() as u32
}

// ---------------------------------------------------------------------------
// C API extern functions
// ---------------------------------------------------------------------------

/// Create a snapshot of the terminal's current state.
///
/// The caller owns the returned `SharedState` and must free it by calling
/// `ptec_terminal_free_state`.
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_get_state(term: *const Terminal) -> *mut SharedState {
    if term.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` not mutably borrowed elsewhere.
    let term_ref = unsafe { &*term };
    let state = SharedState::from_terminal(term_ref);
    Box::into_raw(Box::new(state))
}

/// Free a `SharedState` previously returned by `ptec_terminal_get_state`.
///
/// # Safety
/// `state` must be a pointer previously returned by `ptec_terminal_get_state`,
/// and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_free_state(state: *mut SharedState) {
    if !state.is_null() {
        // SAFETY: non-null, and per this fn's contract it came from
        // `ptec_terminal_get_state`'s `Box::into_raw` and is not used afterwards.
        unsafe {
            let _ = Box::from_raw(state);
        }
    }
}

/// Register an FFI observer on the terminal.
///
/// Returns an observer ID that can be passed to `ptec_terminal_remove_observer`.
///
/// # Safety
/// `term` must be a valid, mutable pointer to a `Terminal`.
/// The `vtable` must remain valid (including its `user_data`) for as long as
/// the observer is registered.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_add_observer(
    term: *mut Terminal,
    vtable: TerminalObserverVtable,
) -> u64 {
    if term.is_null() {
        return 0;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` with no other live borrow.
    let term_ref = unsafe { &mut *term };
    let observer = FfiObserver::new(vtable);
    term_ref.add_observer(Arc::new(observer))
}

/// Remove a previously registered observer.
///
/// Returns `true` if the observer was found and removed.
///
/// # Safety
/// `term` must be a valid, mutable pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn ptec_terminal_remove_observer(term: *mut Terminal, id: u64) -> bool {
    if term.is_null() {
        return false;
    }
    // SAFETY: `term` is non-null (checked above) and, per this fn's
    // contract, points to a live `Terminal` with no other live borrow.
    let term_ref = unsafe { &mut *term };
    term_ref.remove_observer(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::mem::{align_of, offset_of, size_of};

    // Pin the #[repr(C)] layout to the values asserted by the C header
    // (include/terminal_core.h). If a test here fails, the header's
    // _Static_asserts are stale too — update both, never just one.
    #[test]
    fn shared_cell_layout_matches_header() {
        assert_eq!(size_of::<SharedCell>(), 16);
        assert_eq!(align_of::<SharedCell>(), 2);
        assert_eq!(offset_of!(SharedCell, text), 0);
        assert_eq!(offset_of!(SharedCell, text_len), 4);
        assert_eq!(offset_of!(SharedCell, attrs), 12);
        assert_eq!(offset_of!(SharedCell, width), 14);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn shared_state_layout_matches_header() {
        assert_eq!(size_of::<SharedState>(), 80);
        assert_eq!(align_of::<SharedState>(), 8);
        assert_eq!(offset_of!(SharedState, title), 24);
        assert_eq!(offset_of!(SharedState, cells), 56);
        assert_eq!(size_of::<TerminalObserverVtable>(), 56);
        assert_eq!(offset_of!(TerminalObserverVtable, on_event_v2), 40);
        assert_eq!(offset_of!(TerminalObserverVtable, user_data), 48);
        assert_eq!(size_of::<TermEvent>(), 16);
        assert_eq!(offset_of!(TermEvent, payload_len), 4);
        assert_eq!(offset_of!(TermEvent, payload), 8);
        assert_eq!(size_of::<TermRowRange>(), 8);
        assert_eq!(size_of::<TermScrollDelta>(), 16);
        assert_eq!(align_of::<TermScrollDelta>(), 4);
        assert_eq!(offset_of!(TermScrollDelta, delta), 0);
        assert_eq!(offset_of!(TermScrollDelta, top), 4);
        assert_eq!(offset_of!(TermScrollDelta, bottom), 8);
        assert_eq!(offset_of!(TermScrollDelta, flags), 12);
        assert_eq!(size_of::<TermCursorState>(), 12);
        assert_eq!(size_of::<TermModeState>(), 20);
        assert_eq!(size_of::<crate::keyboard::TermKeyEvent>(), 8);
        assert_eq!(offset_of!(crate::keyboard::TermKeyEvent, codepoint), 4);
        assert_eq!(size_of::<crate::keyboard::KeyEncodeOptions>(), 2);
    }

    // ------------------------------------------------------------------
    // Embedding-surface round trip: a seeded VT replay through the FFI
    // must match the core's own screen state byte-for-byte, and the
    // dirty ranges must cover every row that actually changed.
    // ------------------------------------------------------------------

    const RT_COLS: usize = 20;
    const RT_ROWS: usize = 5;

    /// FFI-read one full row through a pinned caller buffer.
    fn read_row_ffi(term: *const Terminal, row: usize) -> Vec<SharedCell> {
        let mut buf = vec![SharedCell::blank(); RT_COLS];
        let n = unsafe {
            ptec_terminal_read_row(term, row as u32, 0, buf.as_mut_ptr(), RT_COLS as u32)
        };
        assert_eq!(n as usize, RT_COLS);
        buf
    }

    /// The core's own view of a row, through the same SharedCell conversion.
    fn reference_row(term: &Terminal, row: usize) -> Vec<SharedCell> {
        let grid = term.active_grid();
        grid.row(row)
            .expect("row exists")
            .iter()
            .map(|c| SharedCell::from_cell(term, c))
            .chain(std::iter::repeat(SharedCell::padding(term)))
            .take(RT_COLS)
            .collect()
    }

    #[test]
    fn ffi_round_trip_matches_core_state() {
        let term = unsafe { ptec_terminal_create(RT_COLS as u32, RT_ROWS as u32, 200) };
        assert!(!term.is_null());

        // Frame chunks: text, colors, cursor moves, wide chars, full erase,
        // and enough lines to push rows into scrollback. The QA-150 frames
        // (ICH, DCH, the rectangle ops, RIS) pin the damage contract for
        // every mutator that used to skip marking.
        let frames: [&[u8]; 13] = [
            b"hello world",
            b"\x1b[31mred\x1b[0m plain \x1b[1;32mbold-green\x1b[0m",
            b"\x1b[3;2HX at 3;2",
            "\u{6F22}\u{5B57} wide".as_bytes(), // CJK wide chars
            b"\x1b[2J\x1b[Hcleared",
            b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight", // forces scroll
            b"\x1b[104;200H tail write",
            b"\x1b[3@",            // ICH
            b"\x1b[2P",            // DCH
            b"\x1b[65;1;1;3;5$x",  // DECFRA
            b"\x1b[1;1;3;5;4;1$v", // DECCRA
            b"\x1b[1;1;3;5$z",     // DECERA
            b"\x1b[1;1;2;5;7$r",   // DECCARA
        ];

        let mut prev_screen: Vec<Vec<SharedCell>> =
            (0..RT_ROWS).map(|r| read_row_ffi(term, r)).collect();

        for (i, frame) in frames.iter().enumerate() {
            unsafe { ptec_terminal_feed(term, frame.as_ptr(), frame.len() as u32) };

            // 1. Screen byte-for-byte: FFI readback == the core's own cells.
            for row in 0..RT_ROWS {
                let ffi = read_row_ffi(term, row);
                let core = reference_row(unsafe { &*term }, row);
                assert_eq!(ffi, core, "frame {i}: row {row} FFI != core");
            }

            // 2. Damage completeness: every row that differs from the
            //    previous frame must be inside a dirty range.
            let cap = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            let got = unsafe { ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
            assert_eq!(got, cap);

            for (row, prev) in prev_screen.iter_mut().enumerate() {
                let now = read_row_ffi(term, row);
                if &now != prev {
                    let inside = ranges
                        .iter()
                        .any(|r| row >= r.start as usize && row <= r.end as usize);
                    assert!(
                        inside,
                        "frame {i}: row {row} changed but is not in any dirty range {ranges:?}"
                    );
                }
                *prev = now;
            }

            unsafe { ptec_terminal_mark_clean(term) };
            let after = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            assert_eq!(after, 0, "frame {i}: mark_clean left dirty ranges");
        }

        // 3. Scrollback: FFI readback matches the core's scrollback cells,
        //    oldest first (line 0 = oldest).
        let sb = unsafe { ptec_terminal_scrollback_count(term) };
        assert!(sb >= 3, "expected scrollback after 8-line frame, got {sb}");
        let term_ref = unsafe { &*term };
        let grid = term_ref.active_grid();
        for line in 0..sb as usize {
            let mut buf = vec![SharedCell::blank(); RT_COLS];
            let n = unsafe {
                ptec_terminal_read_scrollback_row(
                    term,
                    line as u32,
                    0,
                    buf.as_mut_ptr(),
                    RT_COLS as u32,
                )
            };
            assert_eq!(n as usize, RT_COLS);
            let core: Vec<SharedCell> = grid
                .scrollback_line(line)
                .expect("scrollback line exists")
                .iter()
                .map(|c| SharedCell::from_cell(term_ref, c))
                .collect();
            assert_eq!(buf, core, "scrollback line {line} FFI != core");
        }

        // 3b. RIS clears screen and scrollback, so it runs after the
        // scrollback readback: every row must land in a dirty range
        // (ARC-058's damage contract, pinned here at the FFI boundary).
        {
            unsafe { ptec_terminal_feed(term, b"\x1bc".as_ptr(), 2) };
            let cap = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            unsafe { ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
            for row in 0..RT_ROWS {
                let inside = ranges
                    .iter()
                    .any(|r| row >= r.start as usize && row <= r.end as usize);
                assert!(inside, "RIS: row {row} not in any dirty range {ranges:?}");
            }
            let sb = unsafe { ptec_terminal_scrollback_count(term) };
            assert_eq!(sb, 0, "RIS clears scrollback");
        }

        // 4. Cursor + modes match the core's accessors.
        let mut cur = TermCursorState {
            col: 0,
            row: 0,
            visible: false,
            style: 0,
        };
        unsafe { ptec_terminal_get_cursor(term, &mut cur) };
        let c = unsafe { &*term }.cursor();
        assert_eq!(cur.col as usize, c.col);
        assert_eq!(cur.row as usize, c.row);
        assert_eq!(cur.visible, c.visible);

        let mut modes = TermModeState::default();
        unsafe { ptec_terminal_get_modes(term, &mut modes) };
        let t = unsafe { &*term };
        assert_eq!(modes.cols as usize, RT_COLS);
        assert_eq!(modes.rows as usize, RT_ROWS);
        assert_eq!(modes.alt_screen, t.is_alt_screen_active());
        assert_eq!(modes.bracketed_paste, t.bracketed_paste());
        assert_eq!(modes.application_cursor, t.application_cursor());
        assert_eq!(modes.auto_wrap, t.auto_wrap_mode());

        // 5. Key encoding through the FFI matches the direct encoder.
        for ev in [
            crate::keyboard::TermKeyEvent::char_('a', crate::keyboard::modifiers::CTRL),
            crate::keyboard::TermKeyEvent::functional(crate::keyboard::TermKey::Up, 0),
            crate::keyboard::TermKeyEvent::functional(crate::keyboard::TermKey::Enter, 0),
        ] {
            let mut buf = [0u8; 32];
            let n = unsafe { ptec_terminal_encode_key(term, &ev, buf.as_mut_ptr(), 32) };
            assert_eq!(
                &buf[..n as usize],
                crate::keyboard::encode_key(&ev, unsafe { &*term }).as_slice()
            );
        }

        unsafe { ptec_terminal_free(term) };
    }

    /// ENH-028: `ptec_terminal_encode_key_ex` honors `TermKeyOptions` (NULL opts
    /// = the ESC defaults, a zeroed struct = Normal passthrough, and the
    /// modifyOtherKeys mode read from the terminal's own negotiated state).
    #[test]
    fn ffi_encode_key_ex_options_and_null_default() {
        use crate::keyboard::{modifiers, option_modes, KeyEncodeOptions, TermKeyEvent};

        let term = unsafe { ptec_terminal_create(10, 6, 50) };
        let seq = b"\x1b[>4;2m";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };
        let ev = TermKeyEvent::char_('f', modifiers::ALT);

        let mut buf = [0u8; 32];
        // modifyOtherKeys 2 outranks the option modes: CSI 27-form.
        let n = unsafe {
            ptec_terminal_encode_key_ex(
                term,
                &ev,
                &KeyEncodeOptions::default(),
                buf.as_mut_ptr(),
                32,
            )
        };
        assert_eq!(&buf[..n as usize], b"\x1b[27;3;102~");

        // Reset modifyOtherKeys; now the option modes decide.
        let seq = b"\x1b[>4m";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };
        let n = unsafe {
            ptec_terminal_encode_key_ex(
                term,
                &ev,
                &KeyEncodeOptions {
                    left_option: option_modes::META,
                    right_option: option_modes::NORMAL,
                },
                buf.as_mut_ptr(),
                32,
            )
        };
        assert_eq!(&buf[..n as usize], &[0xE6], "left Alt → Meta mode");

        // NULL opts = ESC defaults, byte-identical to ptec_terminal_encode_key.
        let n = unsafe {
            ptec_terminal_encode_key_ex(term, &ev, std::ptr::null(), buf.as_mut_ptr(), 32)
        };
        let m = unsafe { ptec_terminal_encode_key(term, &ev, buf.as_mut_ptr(), 32) };
        assert_eq!(&buf[..n as usize], &[0x1b, b'f']);
        assert_eq!(n, m);

        // A zeroed struct is Normal on both sides.
        let zeroed = KeyEncodeOptions {
            left_option: 0,
            right_option: 0,
        };
        let n = unsafe { ptec_terminal_encode_key_ex(term, &ev, &zeroed, buf.as_mut_ptr(), 32) };
        assert_eq!(&buf[..n as usize], b"f");

        unsafe { ptec_terminal_free(term) };
    }

    #[test]
    fn ffi_dirty_ranges_coalesce_and_resize_marks_damage() {
        let term = unsafe { ptec_terminal_create(10, 6, 50) };
        let seq = b"\x1b[Hrow0\x1b[3Hrow2\x1b[5Hrow4";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };

        let cap = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
        // Rows 0, 2, 4 were written: three one-row ranges, none adjacent.
        assert_eq!(
            ranges,
            vec![
                TermRowRange { start: 0, end: 0 },
                TermRowRange { start: 2, end: 2 },
                TermRowRange { start: 4, end: 4 },
            ]
        );

        // A full-row write makes the ranges coalesce.
        let seq = b"\x1b[2Hxxxx\x1b[4Hxxxx";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };
        let cap = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
        assert_eq!(ranges, vec![TermRowRange { start: 0, end: 4 }]);

        // Resize must mark the whole (new) screen dirty.
        unsafe { ptec_terminal_mark_clean(term) };
        unsafe { ptec_terminal_resize(term, 12, 4) };
        let cap = unsafe { ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
        assert_eq!(ranges, vec![TermRowRange { start: 0, end: 3 }]);

        unsafe { ptec_terminal_free(term) };
    }

    /// Damage localization gate for the readback benchmark: an in-place
    /// status rewrite must dirty only a small fraction of the screen. If
    /// this fails, damage tracking has regressed into full repaints and the
    /// ffi_readback dirty benchmark would be benchmarking a full copy.
    #[test]
    fn status_frames_localize_damage() {
        let (cols, rows) = (120usize, 40usize);
        let mut term = Terminal::with_scrollback(cols, rows, 500);
        // Fill the screen with a scrolling stream first.
        for f in 0..8 {
            let mut frame = format!("\x1b[1;1H\x1b[2K== frame {f:05} ==");
            for line in 0..3 {
                frame.push_str(&format!(
                    "\x1b[{row};1H\x1b[2Kstatus[{line}]: {val:x>16}",
                    row = line + 2,
                    val = f * (line + 1)
                ));
            }
            frame.push_str(&format!(
                "\x1b[{rows};1Hlog line {f} padding-padding-padding\r\n"
            ));
            term.process(frame.as_bytes());
            term.mark_clean();
        }
        // A status-only frame (no scroll append) must stay localized.
        term.process(b"\x1b[1;1H\x1b[2K== session frame 00100 ==\x1b[3;1H\x1b[2Kstatus[0]: hello");
        let dirty = term.get_dirty_rows();
        assert!(
            dirty.len() <= 3,
            "status frame dirtied {dirty:?} of {rows} rows — damage tracking degraded"
        );
    }

    /// DOC-071: every buffer-returning embedding call accepts
    /// out == NULL / cap == 0 as a sizing call and returns the total,
    /// the way ptec_terminal_dirty_ranges always has.
    #[test]
    fn ffi_sizing_calls_return_totals_without_buffers() {
        let term = unsafe { ptec_terminal_create(20, 5, 100) };
        assert!(!term.is_null());
        unsafe { ptec_terminal_feed(term, b"hello\nworld".as_ptr(), 11) };

        // read_row sizes to the columns available from col_start.
        let n = unsafe { ptec_terminal_read_row(term, 0, 0, std::ptr::null_mut(), 0) };
        assert_eq!(n, 20);
        let n = unsafe { ptec_terminal_read_row(term, 0, 17, std::ptr::null_mut(), 0) };
        assert_eq!(n, 3);
        let n = unsafe { ptec_terminal_read_row(term, 0, 20, std::ptr::null_mut(), 0) };
        assert_eq!(n, 0);
        let n = unsafe { ptec_terminal_read_row(term, 99, 0, std::ptr::null_mut(), 0) };
        assert_eq!(n, 0, "row past the screen sizes to 0");

        // read_scrollback_row sizes the same way, once scrollback exists.
        unsafe { ptec_terminal_feed(term, b"a\nb\nc\nd\ne\nf\ng\nh".as_ptr(), 15) };
        let sb = unsafe { ptec_terminal_scrollback_count(term) };
        assert!(sb >= 1, "expected scrollback after 8 lines on 5 rows");
        let n = unsafe { ptec_terminal_read_scrollback_row(term, 0, 0, std::ptr::null_mut(), 0) };
        assert_eq!(n, 20);
        let n =
            unsafe { ptec_terminal_read_scrollback_row(term, sb - 1, 18, std::ptr::null_mut(), 0) };
        assert_eq!(n, 2);

        // encode_key sizing total equals the written length for the event.
        let ev = crate::keyboard::TermKeyEvent::functional(crate::keyboard::TermKey::Up, 0);
        let total = unsafe { ptec_terminal_encode_key(term, &ev, std::ptr::null_mut(), 0) };
        let mut buf = [0u8; 16];
        let written = unsafe { ptec_terminal_encode_key(term, &ev, buf.as_mut_ptr(), 16) };
        assert_eq!(total, written);
        assert_eq!(&buf[..written as usize], b"\x1b[A");

        unsafe { ptec_terminal_free(term) };
    }

    /// The scrollback window cursor: `ptec_terminal_scrollback_total_scrolled`
    /// paired with `ptec_terminal_scrollback_count` gives an embedder the window
    /// `[total - count, total)`, so head evictions and tail appends are
    /// derivable even when the ring is full and the count freezes at the
    /// cap. Pinned here: the `count == min(total, cap)` invariant (a
    /// violation is the consumer's full-replace signal), the cap freeze,
    /// the alt-screen active-grid reads, and the ED 3J reset.
    #[test]
    fn ffi_scrollback_total_scrolled_tracks_the_window() {
        let cols = 20;
        let rows = 5;
        let cap = 4;
        let term = unsafe { ptec_terminal_create(cols, rows, cap) };
        assert!(!term.is_null());

        let total = |t: *const Terminal| unsafe { ptec_terminal_scrollback_total_scrolled(t) };
        let count = |t: *const Terminal| unsafe { ptec_terminal_scrollback_count(t) };

        assert_eq!(total(term), 0);
        assert_eq!(count(term), 0);

        // Fill past the screen and past the ring: every scrolled-off line
        // raises the total; the count tracks it only until the cap.
        for i in 0..(rows + cap + 3) {
            let line = format!("line{i}\r\n");
            unsafe { ptec_terminal_feed(term, line.as_ptr(), line.len() as u32) };
            let (t, c) = (total(term), count(term));
            assert_eq!(
                c as u64,
                t.min(cap as u64),
                "window invariant count == min(total, cap) broke after line {i}"
            );
        }
        assert_eq!(count(term), cap, "count freezes at the cap");
        assert_eq!(total(term), (rows + cap + 3 - (rows - 1)) as u64);
        assert!(total(term) > cap as u64, "total keeps counting evictions");

        // The alternate screen is a separate scrollback-less grid: reads
        // target it while active, and the primary window survives the
        // round trip (a switch is a replace, not a slide, for consumers).
        unsafe { ptec_terminal_feed(term, b"\x1b[?1049h".as_ptr(), 8) };
        assert_eq!(count(term), 0);
        assert_eq!(total(term), 0, "the alt grid has its own (empty) counters");
        unsafe { ptec_terminal_feed(term, b"\x1b[?1049l".as_ptr(), 8) };
        assert_eq!(count(term), cap);
        assert_eq!(total(term), (rows + cap + 3 - (rows - 1)) as u64);

        // ED 3J clears the buffer: both counters reset together, so a
        // delta consumer sees an empty window, never a negative one.
        unsafe { ptec_terminal_feed(term, b"\x1b[3J".as_ptr(), 4) };
        assert_eq!(count(term), 0);
        assert_eq!(total(term), 0);

        assert_eq!(
            unsafe { ptec_terminal_scrollback_total_scrolled(std::ptr::null()) },
            0
        );
        unsafe { ptec_terminal_free(term) };
    }

    /// ARC-063: the exported ABI version must equal the header's
    /// TERM_CORE_ABI_VERSION, so a binary can detect a layout mismatch by
    /// comparing the two. Bumping one side without the other fails here.
    #[test]
    fn abi_version_matches_header_macro() {
        assert_eq!(ptec_terminal_abi_version(), 5);
        let header = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/include/terminal_core_layout.h"
        ))
        .expect("header readable");
        assert!(
            header.contains("#define TERM_CORE_ABI_VERSION 5"),
            "header TERM_CORE_ABI_VERSION drifted from ptec_terminal_abi_version()"
        );
    }

    /// ENH-038: the scroll-aware damage surface. One linefeed at the bottom
    /// of a full screen reports delta 1 over the whole screen with only the
    /// cleared row content-dirty, while the positional surface still
    /// reports every row (pre-v5 behavior unchanged). The same contract
    /// holds on the alternate screen — where
    /// `ptec_terminal_scrollback_total_scrolled` stays 0: it is a
    /// window-sync counter, never a scroll-damage source — and under a
    /// DECSTBM region.
    #[test]
    fn scroll_delta_reports_blittable_damage() {
        unsafe {
            let term = ptec_terminal_create(80, 24, 1000);
            let mut fill = Vec::new();
            for i in 0..24 {
                if i > 0 {
                    fill.extend_from_slice(b"\r\n");
                }
                fill.extend_from_slice(format!("row {i:02}").as_bytes());
            }
            ptec_terminal_feed(term, fill.as_ptr(), fill.len() as u32);
            let gen = ptec_terminal_damage_generation(term);
            ptec_terminal_feed(term, b"\n".as_ptr(), 1);

            let mut delta = TermScrollDelta {
                delta: 0,
                top: 0,
                bottom: 0,
                flags: 0,
            };
            assert!(ptec_terminal_scroll_delta_since(term, gen, &mut delta));
            assert_eq!(delta.delta, 1);
            assert_eq!(delta.top, 0);
            assert_eq!(delta.bottom, 23);
            assert_eq!(delta.flags & TERM_SCROLL_FULL_REDRAW, 0);

            let mut ranges = vec![TermRowRange { start: 0, end: 0 }; 8];
            let n = ptec_terminal_content_dirty_ranges_since(term, gen, ranges.as_mut_ptr(), 8);
            assert_eq!(n, 1);
            assert_eq!(ranges[0], TermRowRange { start: 23, end: 23 });

            // Positional damage unchanged: every row of the screen is dirty.
            let n = ptec_terminal_dirty_ranges_since(term, gen, ranges.as_mut_ptr(), 8);
            assert_eq!(n, 1);
            assert_eq!(ranges[0], TermRowRange { start: 0, end: 23 });

            // Alternate screen: the delta is reported although the
            // window-sync counter stays 0.
            ptec_terminal_feed(term, b"\x1b[?1049h\x1b[H".as_ptr(), 11);
            ptec_terminal_feed(term, fill.as_ptr(), fill.len() as u32);
            let gen = ptec_terminal_damage_generation(term);
            ptec_terminal_feed(term, b"\n".as_ptr(), 1);
            assert!(ptec_terminal_scroll_delta_since(term, gen, &mut delta));
            assert_eq!(delta.delta, 1);
            assert_eq!(delta.top, 0);
            assert_eq!(delta.bottom, 23);
            assert_eq!(delta.flags & TERM_SCROLL_FULL_REDRAW, 0);
            assert_eq!(ptec_terminal_scrollback_total_scrolled(term), 0);

            // DECSTBM 2;23: the report carries the region, not the screen.
            ptec_terminal_feed(term, b"\x1b[?1049l\x1b[2;23r\x1b[H".as_ptr(), 17);
            for _ in 0..22 {
                ptec_terminal_feed(term, b"\n".as_ptr(), 1);
            }
            let gen = ptec_terminal_damage_generation(term);
            ptec_terminal_feed(term, b"\n".as_ptr(), 1);
            assert!(ptec_terminal_scroll_delta_since(term, gen, &mut delta));
            assert_eq!(delta.delta, 1);
            assert_eq!(delta.top, 1);
            assert_eq!(delta.bottom, 22);
            assert_eq!(delta.flags & TERM_SCROLL_FULL_REDRAW, 0);
            let n = ptec_terminal_content_dirty_ranges_since(term, gen, ranges.as_mut_ptr(), 8);
            assert_eq!(n, 1);
            assert_eq!(ranges[0], TermRowRange { start: 22, end: 22 });

            ptec_terminal_free(term);
        }
    }

    /// ENH-027: every `#define` in terminal_core_layout.h — the hand-written
    /// companion to the cbindgen-generated terminal_core.h — must equal its
    /// Rust source of truth. cbindgen cannot emit these constants, so this
    /// test is what keeps the C side from renumbering silently: change the
    /// Rust value and this fails until the companion is updated (and vice
    /// versa). It also fails on a define that no longer has a Rust
    /// counterpart, so stale macros cannot accumulate.
    #[test]
    fn layout_header_defines_match_rust() {
        use crate::cell::CellBitflags;
        use crate::keyboard::modifiers;
        use crate::keyboard::TermKey;
        use crate::mouse::MouseMode;

        let cb = |f: CellBitflags| f.bits() as u32;
        let mm = |m: MouseMode| m as u32;
        let tk = |k: TermKey| k as u16 as u32;
        let expected: Vec<(&str, u32)> = vec![
            ("TERM_CORE_ABI_VERSION", TERM_CORE_ABI_VERSION),
            ("TERM_SCROLL_FULL_REDRAW", TERM_SCROLL_FULL_REDRAW),
            ("TERM_CELL_BOLD", cb(CellBitflags::BOLD)),
            ("TERM_CELL_DIM", cb(CellBitflags::DIM)),
            ("TERM_CELL_ITALIC", cb(CellBitflags::ITALIC)),
            ("TERM_CELL_UNDERLINE", cb(CellBitflags::UNDERLINE)),
            ("TERM_CELL_BLINK", cb(CellBitflags::BLINK)),
            ("TERM_CELL_REVERSE", cb(CellBitflags::REVERSE)),
            ("TERM_CELL_HIDDEN", cb(CellBitflags::HIDDEN)),
            ("TERM_CELL_STRIKETHROUGH", cb(CellBitflags::STRIKETHROUGH)),
            ("TERM_CELL_OVERLINE", cb(CellBitflags::OVERLINE)),
            ("TERM_CELL_GUARDED", cb(CellBitflags::GUARDED)),
            ("TERM_CELL_WIDE_CHAR", cb(CellBitflags::WIDE_CHAR)),
            (
                "TERM_CELL_WIDE_CHAR_SPACER",
                cb(CellBitflags::WIDE_CHAR_SPACER),
            ),
            ("TERM_MOUSE_MODE_OFF", mm(MouseMode::Off)),
            ("TERM_MOUSE_MODE_X10", mm(MouseMode::X10)),
            ("TERM_MOUSE_MODE_NORMAL", mm(MouseMode::Normal)),
            ("TERM_MOUSE_MODE_BUTTON", mm(MouseMode::ButtonEvent)),
            ("TERM_MOUSE_MODE_ANY", mm(MouseMode::AnyEvent)),
            ("TERM_MOD_SHIFT", modifiers::SHIFT as u32),
            ("TERM_MOD_ALT", modifiers::ALT as u32),
            ("TERM_MOD_CTRL", modifiers::CTRL as u32),
            ("TERM_MOD_SUPER", modifiers::SUPER as u32),
            ("TERM_MOD_HYPER", modifiers::HYPER as u32),
            ("TERM_MOD_META", modifiers::META as u32),
            ("TERM_MOD_ALT_RIGHT", modifiers::ALT_RIGHT as u32),
            (
                "TERM_OPTION_MODE_NORMAL",
                crate::keyboard::option_modes::NORMAL as u32,
            ),
            (
                "TERM_OPTION_MODE_META",
                crate::keyboard::option_modes::META as u32,
            ),
            (
                "TERM_OPTION_MODE_ESC",
                crate::keyboard::option_modes::ESC as u32,
            ),
            ("TERM_KEY_UNKNOWN", tk(TermKey::Unknown)),
            ("TERM_KEY_CHAR", tk(TermKey::Char)),
            ("TERM_KEY_TAB", tk(TermKey::Tab)),
            ("TERM_KEY_ENTER", tk(TermKey::Enter)),
            ("TERM_KEY_ESCAPE", tk(TermKey::Escape)),
            ("TERM_KEY_BACKSPACE", tk(TermKey::Backspace)),
            ("TERM_KEY_INSERT", tk(TermKey::Insert)),
            ("TERM_KEY_DELETE", tk(TermKey::Delete)),
            ("TERM_KEY_LEFT", tk(TermKey::Left)),
            ("TERM_KEY_RIGHT", tk(TermKey::Right)),
            ("TERM_KEY_UP", tk(TermKey::Up)),
            ("TERM_KEY_DOWN", tk(TermKey::Down)),
            ("TERM_KEY_PAGE_UP", tk(TermKey::PageUp)),
            ("TERM_KEY_PAGE_DOWN", tk(TermKey::PageDown)),
            ("TERM_KEY_HOME", tk(TermKey::Home)),
            ("TERM_KEY_END", tk(TermKey::End)),
            ("TERM_KEY_F1", tk(TermKey::F1)),
            ("TERM_KEY_F2", tk(TermKey::F2)),
            ("TERM_KEY_F3", tk(TermKey::F3)),
            ("TERM_KEY_F4", tk(TermKey::F4)),
            ("TERM_KEY_F5", tk(TermKey::F5)),
            ("TERM_KEY_F6", tk(TermKey::F6)),
            ("TERM_KEY_F7", tk(TermKey::F7)),
            ("TERM_KEY_F8", tk(TermKey::F8)),
            ("TERM_KEY_F9", tk(TermKey::F9)),
            ("TERM_KEY_F10", tk(TermKey::F10)),
            ("TERM_KEY_F11", tk(TermKey::F11)),
            ("TERM_KEY_F12", tk(TermKey::F12)),
        ];
        let mut expected = expected;
        expected.extend(TERM_EVENT_CODES.iter().map(|&(n, c)| (n, c as u32)));
        expected.extend([
            ("TERM_ATTR_DEFAULT_FG", attr_bits::DEFAULT_FG as u32),
            ("TERM_ATTR_DEFAULT_BG", attr_bits::DEFAULT_BG as u32),
            ("TERM_ATTR_HAS_COMBINING", attr_bits::HAS_COMBINING as u32),
        ]);
        // The readback bits must sit above every CellBitflags bit.
        let readback = attr_bits::DEFAULT_FG | attr_bits::DEFAULT_BG | attr_bits::HAS_COMBINING;
        assert_eq!(CellBitflags::all().bits() & readback, 0);

        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/include/terminal_core_layout.h"
        ))
        .expect("terminal_core_layout.h readable");

        // #define NAME VALUE — values are `0`, `1u`, `1024u`, …
        let mut actual: Vec<(String, u32)> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("#define ") {
                let mut parts = rest.split_whitespace();
                if let (Some(name), Some(raw)) = (parts.next(), parts.next()) {
                    if let Ok(v) = raw.trim_end_matches('u').parse::<u32>() {
                        actual.push((name.to_string(), v));
                    }
                }
            }
        }
        assert!(
            !actual.is_empty(),
            "no #defines parsed — terminal_core_layout.h shape changed?"
        );

        for (name, want) in &expected {
            match actual.iter().find(|(n, _)| n == name) {
                Some((_, got)) => assert_eq!(
                    got, want,
                    "{name} in terminal_core_layout.h drifted from Rust"
                ),
                None => panic!("{name} missing from terminal_core_layout.h"),
            }
        }
        for (name, _) in &actual {
            assert!(
                expected.iter().any(|(n, _)| n == name),
                "terminal_core_layout.h defines {name} with no Rust counterpart — stale macro?"
            );
        }
    }

    /// ENH-025: the FFI generation consumer and the built-in default
    /// consumer (`ptec_terminal_dirty_ranges`/`ptec_terminal_mark_clean`) each
    /// observe the same edit, and the default consumer's mark_clean does
    /// not hide damage from the generation consumer.
    #[test]
    fn two_damage_consumers_are_isolated() {
        let term = unsafe { ptec_terminal_create(20, 4, 0) };
        assert!(!term.is_null());
        unsafe {
            ptec_terminal_feed(term, b"hello\r\nworld".as_ptr(), 12);

            // Both consumers observe the edit.
            let gen0 = ptec_terminal_damage_generation(term);
            assert!(gen0 > 0, "feeding must advance the generation");
            assert_eq!(
                ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0),
                1,
                "default consumer sees rows 0-1 coalesced"
            );
            assert_eq!(
                ptec_terminal_dirty_ranges_since(term, gen0, std::ptr::null_mut(), 0),
                0,
                "nothing changed since gen0 was captured"
            );

            ptec_terminal_feed(term, b"!".as_ptr(), 1);

            let cap = ptec_terminal_dirty_ranges_since(term, gen0, std::ptr::null_mut(), 0);
            assert_eq!(cap, 1, "generation consumer sees only row 1");
            let mut since = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            assert_eq!(
                ptec_terminal_dirty_ranges_since(term, gen0, since.as_mut_ptr(), cap),
                cap
            );
            assert_eq!(since[0], TermRowRange { start: 1, end: 1 });
            assert_eq!(
                ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0),
                1,
                "default consumer also sees row 1"
            );

            // The default consumer repaints; the generation consumer's
            // window is untouched.
            ptec_terminal_mark_clean(term);
            assert_eq!(
                ptec_terminal_dirty_ranges(term, std::ptr::null_mut(), 0),
                0,
                "mark_clean advances the default consumer"
            );
            assert_eq!(
                ptec_terminal_dirty_ranges_since(term, gen0, std::ptr::null_mut(), 0),
                1,
                "mark_clean must not hide damage from the generation consumer"
            );

            // A fresh generation window starts empty.
            let gen1 = ptec_terminal_damage_generation(term);
            assert!(gen1 > gen0);
            assert_eq!(
                ptec_terminal_dirty_ranges_since(term, gen1, std::ptr::null_mut(), 0),
                0
            );

            // A screen switch dirties every row of the new grid for any
            // consumer holding an older generation.
            let bytes = b"\x1b[?1049h";
            ptec_terminal_feed(term, bytes.as_ptr(), bytes.len() as u32);
            let cap = ptec_terminal_dirty_ranges_since(term, gen1, std::ptr::null_mut(), 0);
            let mut switched = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            assert_eq!(
                ptec_terminal_dirty_ranges_since(term, gen1, switched.as_mut_ptr(), cap),
                cap
            );
            assert_eq!(switched[0], TermRowRange { start: 0, end: 3 });

            ptec_terminal_free(term);
        }
    }

    /// ENH-026: `for_each_dirty_range` coalesces runs in place (no Vec)
    /// and must match the reference algorithm over `get_dirty_rows()`
    /// for every shape a renderer can meet.
    #[test]
    fn for_each_dirty_range_matches_reference() {
        // (rows, marked ranges): empty, a single row, a run crossing the
        // 64-row boundary, two separate runs, and every row at once.
        let cases: [(usize, &[(usize, usize)]); 5] = [
            (24, &[]),
            (24, &[(7, 7)]),
            (200, &[(62, 66)]),
            (24, &[(3, 5), (10, 12)]),
            (200, &[(0, 199)]),
        ];
        for (rows, marks) in cases {
            let mut term = Terminal::with_scrollback(10, rows, 0);
            for &(top, bottom) in marks {
                term.grid.mark_rows_damage(top, bottom);
            }
            let mut runs = Vec::new();
            term.for_each_dirty_range(|start, end| runs.push(TermRowRange { start, end }));
            let reference = coalesce_row_ranges(term.get_dirty_rows().into_iter());
            assert_eq!(runs, reference, "rows={rows} marks={marks:?}");
        }
    }

    /// SEC-117: the string length fields must always equal `strlen` of
    /// the NUL-terminated C string they accompany — an interior NUL is
    /// replaced (U+FFFD), never silently truncated while the length
    /// still names the source bytes.
    #[test]
    fn string_lengths_equal_strlen() {
        let (ptr, len) = to_c_string("a\0b");
        let bytes = unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec();
        drop(unsafe { CString::from_raw(ptr) });
        assert_eq!(bytes.len() as u32, len, "len == strlen");
        assert_eq!(
            bytes,
            b"a\xEF\xBF\xBDb".to_vec(),
            "NUL replaced, not dropped"
        );

        // End to end: OSC 7 carrying %00 is rejected at the source, so
        // the snapshot's cwd is absent — never a truncated string with a
        // stale length (the audit's cwd_len 4003 / strlen 0 shape).
        let mut term = Terminal::with_scrollback(10, 5, 100);
        term.process(b"\x1b]2;hello\x07");
        term.process(b"\x1b]7;file:///tmp/a%00b\x07");
        let state = SharedState::from_terminal(&term);
        assert!(state.cwd.is_null(), "decoded-NUL cwd is rejected upstream");
        assert_eq!(state.cwd_len, 0);
        let title = unsafe { CStr::from_ptr(state.title) };
        assert_eq!(title.to_bytes().len() as u32, state.title_len);
        assert_eq!(title.to_bytes(), b"hello");
        drop(state); // Drop frees title/cwd/cells
    }

    /// QA-215: `cell_count` is the length of the allocation `cells` points
    /// to, so the last cell is readable and `ptec_terminal_free_state` rebuilds
    /// exactly that slice.
    #[test]
    fn shared_state_cell_count_matches_grid() {
        let term = unsafe { ptec_terminal_create(7, 3, 10) };
        assert!(!term.is_null());
        unsafe { ptec_terminal_feed(term, b"\x1b[3;7HZ".as_ptr(), 7) };
        let state = unsafe { ptec_terminal_get_state(term) };
        assert!(!state.is_null());
        let s = unsafe { &*state };
        assert_eq!(s.cell_count, 21);
        let cells = unsafe { std::slice::from_raw_parts(s.cells, s.cell_count as usize) };
        let last = &cells[20];
        assert_eq!(&last.text[..last.text_len as usize], b"Z");
        unsafe { ptec_terminal_free_state(state) };
        unsafe { ptec_terminal_free(term) };
    }

    /// QA-151: an arbitrary uint16_t in `TermKeyEvent.key` — a value C or
    /// Swift is free to write — encodes to nothing instead of being
    /// undefined behavior. key=2 is not a discriminant, 57388 sits between
    /// the F-keys and Insert, 0xFFFF is the extreme.
    #[test]
    fn encode_key_rejects_invalid_discriminants_without_ub() {
        let term = unsafe { ptec_terminal_create(20, 5, 100) };
        assert!(!term.is_null());
        let mut out = [0u8; 16];
        for raw in [2u16, 57388, 0xFFFF] {
            let ev = crate::keyboard::TermKeyEvent {
                key: raw,
                modifiers: 0,
                _pad: 0,
                codepoint: 0,
            };
            let n =
                unsafe { ptec_terminal_encode_key(term, &ev, out.as_mut_ptr(), out.len() as u32) };
            assert_eq!(n, 0, "raw key {raw} has no encoding");
        }
        // A real key still encodes through the same path.
        let ev = crate::keyboard::TermKeyEvent::functional(crate::keyboard::TermKey::Up, 0);
        let n = unsafe { ptec_terminal_encode_key(term, &ev, out.as_mut_ptr(), out.len() as u32) };
        assert_eq!(&out[..n as usize], b"\x1b[A");
        unsafe { ptec_terminal_free(term) };
    }

    /// What an observer test callback recorded: (kind, payload bytes) from
    /// `on_event_v2`, and the text from `on_event`.
    #[derive(Default)]
    struct Recorded {
        v2: Vec<(u16, Vec<u8>)>,
        text: Vec<Vec<u8>>,
    }

    unsafe extern "C" fn record_v2(user_data: *mut std::ffi::c_void, ev: *const TermEvent) {
        let rec = unsafe { &mut *(user_data as *mut Recorded) };
        let ev = unsafe { &*ev };
        let bytes = unsafe { std::slice::from_raw_parts(ev.payload, ev.payload_len as usize) };
        assert_eq!(ev._pad, 0);
        rec.v2.push((ev.kind, bytes.to_vec()));
    }

    unsafe extern "C" fn record_text(user_data: *mut std::ffi::c_void, text: *const c_char) {
        let rec = unsafe { &mut *(user_data as *mut Recorded) };
        rec.text
            .push(unsafe { CStr::from_ptr(text) }.to_bytes().to_vec());
    }

    fn vtable_for(rec: &mut Recorded, text: bool, v2: bool) -> TerminalObserverVtable {
        TerminalObserverVtable {
            on_zone_event: None,
            on_command_event: None,
            on_environment_event: None,
            on_screen_event: None,
            on_event: if text { Some(record_text) } else { None },
            on_event_v2: if v2 { Some(record_v2) } else { None },
            user_data: rec as *mut Recorded as *mut std::ffi::c_void,
        }
    }

    /// ARC-114: `on_event_v2` delivers a `TERM_EVENT_*` kind and a JSON
    /// payload whose keys match the Python event dicts, once per event.
    #[test]
    fn event_v2_carries_kind_and_json_payload() {
        let mut rec = Recorded::default();
        let term = unsafe { ptec_terminal_create(20, 4, 10) };
        let id = unsafe { ptec_terminal_add_observer(term, vtable_for(&mut rec, false, true)) };
        assert_ne!(id, 0);

        let seq = b"\x1b]0;hello\x07\x07";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };

        let title = rec
            .v2
            .iter()
            .find(|(k, _)| *k == event_kind_code(&TerminalEventKind::TitleChanged))
            .expect("TitleChanged delivered");
        let json: serde_json::Value = serde_json::from_slice(&title.1).expect("payload is JSON");
        assert_eq!(json["type"], "title_changed");
        assert_eq!(json["title"], "hello");

        let bell = rec
            .v2
            .iter()
            .find(|(k, _)| *k == event_kind_code(&TerminalEventKind::BellRang))
            .expect("BellRang delivered");
        let json: serde_json::Value = serde_json::from_slice(&bell.1).expect("payload is JSON");
        assert_eq!(json["type"], "bell");
        assert!(json["bell_type"].is_string(), "bell_type rides the payload");

        // Exactly one v2 delivery per dispatched event (not one per slot).
        let before = rec.v2.len();
        unsafe { ptec_terminal_feed(term, b"\x07".as_ptr(), 1) };
        assert_eq!(rec.v2.len(), before + 1);

        assert!(unsafe { ptec_terminal_remove_observer(term, id) });
        unsafe { ptec_terminal_free(term) };
    }

    /// ARC-114: text that contains a NUL reaches both channels. The old
    /// `CString::new` path silently dropped such an event from every text
    /// slot; v2 carries it length-delimited (JSON-escaped), and the text
    /// slots carry it with U+FFFD.
    #[test]
    fn event_with_nul_is_delivered_on_both_channels() {
        let mut rec = Recorded::default();
        let observer = FfiObserver::new(vtable_for(&mut rec, true, true));
        let event = TerminalEvent::TitleChanged("a\0b".to_string());
        crate::observer::TerminalObserver::on_event(&observer, &event);

        assert_eq!(rec.text.len(), 1, "text slot no longer drops NUL events");
        assert!(!rec.text[0].contains(&0), "C string has no interior NUL");
        assert_eq!(rec.v2.len(), 1);
        let json: serde_json::Value = serde_json::from_slice(&rec.v2[0].1).expect("JSON");
        assert_eq!(json["title"], "a\u{0}b", "v2 carries the NUL intact");
        assert_eq!(
            rec.v2[0].0,
            event_kind_code(&TerminalEventKind::TitleChanged)
        );
    }

    /// ARC-114: unset optional fields are present as JSON null, and integer
    /// fields are JSON numbers (the Python dict value types).
    #[test]
    fn event_v2_payload_keeps_nulls_and_numbers() {
        let payload = event_payload_json(&TerminalEvent::HyperlinkAdded {
            url: "https://example.com".to_string(),
            row: 1,
            col: 2,
            id: None,
        });
        let json: serde_json::Value = serde_json::from_slice(&payload).expect("JSON");
        assert!(json["id"].is_null());
        assert_eq!(json["row"], 1);
        assert_eq!(json["col"], 2);
    }

    fn read_cell(term: *const Terminal, row: u32, col: u32) -> SharedCell {
        let mut buf = vec![SharedCell::blank(); 1];
        let n = unsafe { ptec_terminal_read_row(term, row, col, buf.as_mut_ptr(), 1) };
        assert_eq!(n, 1);
        buf.remove(0)
    }

    /// ARC-101 audit probe, replayed through the C entry points: after
    /// `OSC 4;1;rgb:00/00/ff`, SGR 31 reads back as the new palette color
    /// (it read (128,0,0) on ABI 3), and default cells carry the default
    /// bits with the OSC 10/11 colors.
    #[test]
    fn read_row_follows_palette_and_defaults() {
        let term = unsafe { ptec_terminal_create(20, 4, 10) };
        let seq = b"\x1b]4;1;rgb:00/00/ff\x07\x1b[31mR\x1b[0mD\x1b[1;31mB";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };

        let red = read_cell(term, 0, 0);
        assert_eq!((red.fg_r, red.fg_g, red.fg_b), (0, 0, 255));
        assert_eq!(red.attrs & attr_bits::DEFAULT_FG, 0);
        assert_ne!(red.attrs & attr_bits::DEFAULT_BG, 0);

        let plain = read_cell(term, 0, 1);
        assert_ne!(plain.attrs & attr_bits::DEFAULT_FG, 0);
        // The default fg is Named(White) out of the box: the live palette's
        // slot 7, not Color::to_rgb's fixed (192,192,192).
        let t = unsafe { &*term };
        assert_eq!(
            (plain.fg_r, plain.fg_g, plain.fg_b),
            t.resolve_color(&t.default_fg())
        );
        assert_eq!(
            (plain.fg_r, plain.fg_g, plain.fg_b),
            t.get_ansi_palette()[7].to_rgb()
        );

        // Bold brightening: bold SGR 31 → palette slot 9.
        let bold = read_cell(term, 0, 2);
        let slot9 = unsafe { &*term }.get_ansi_palette()[9].to_rgb();
        assert_eq!((bold.fg_r, bold.fg_g, bold.fg_b), slot9);

        let seq = b"\x1b]10;rgb:11/22/33\x07\x1b]11;rgb:44/55/66\x07";
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };
        let blank = read_cell(term, 3, 5);
        assert_eq!((blank.fg_r, blank.fg_g, blank.fg_b), (0x11, 0x22, 0x33));
        assert_eq!((blank.bg_r, blank.bg_g, blank.bg_b), (0x44, 0x55, 0x66));
        assert_ne!(blank.attrs & attr_bits::DEFAULT_FG, 0);
        assert_ne!(blank.attrs & attr_bits::DEFAULT_BG, 0);

        // The snapshot resolves the same way as the pinned readback.
        let state = unsafe { ptec_terminal_get_state(term) };
        let s = unsafe { &*state };
        let cells = unsafe { std::slice::from_raw_parts(s.cells, s.cell_count as usize) };
        assert_eq!(cells[0], read_cell(term, 0, 0));
        assert_eq!(cells[3 * 20 + 5], blank);
        unsafe { ptec_terminal_free_state(state) };
        unsafe { ptec_terminal_free(term) };
    }

    /// ARC-101 audit probe: `x` + U+0301 kept only `x`. The base char stays
    /// in `text`, `TERM_ATTR_HAS_COMBINING` flags the cell, and
    /// `ptec_terminal_read_cell_grapheme` returns the whole cluster with the
    /// cap/return-total protocol.
    #[test]
    fn read_cell_grapheme_returns_full_cluster() {
        let term = unsafe { ptec_terminal_create(10, 3, 0) };
        let seq = "x\u{301}y".as_bytes();
        unsafe { ptec_terminal_feed(term, seq.as_ptr(), seq.len() as u32) };

        let x = read_cell(term, 0, 0);
        assert_eq!(&x.text[..x.text_len as usize], b"x");
        assert_ne!(x.attrs & attr_bits::HAS_COMBINING, 0);
        let y = read_cell(term, 0, 1);
        assert_eq!(y.attrs & attr_bits::HAS_COMBINING, 0);

        let want = "x\u{301}".as_bytes();
        let total =
            unsafe { ptec_terminal_read_cell_grapheme(term, 0, 0, std::ptr::null_mut(), 0) };
        assert_eq!(total as usize, want.len());
        let mut buf = [0u8; 8];
        let n = unsafe { ptec_terminal_read_cell_grapheme(term, 0, 0, buf.as_mut_ptr(), 8) };
        assert_eq!(&buf[..n as usize], want);

        // A short buffer gets a prefix and the full total back.
        let mut short = [0u8; 2];
        let n = unsafe { ptec_terminal_read_cell_grapheme(term, 0, 0, short.as_mut_ptr(), 2) };
        assert_eq!(n as usize, want.len());
        assert_eq!(&short, &want[..2]);

        // A plain cell's cluster is its base char; off-grid is 0.
        let n = unsafe { ptec_terminal_read_cell_grapheme(term, 0, 1, buf.as_mut_ptr(), 8) };
        assert_eq!(&buf[..n as usize], b"y");
        assert_eq!(
            unsafe { ptec_terminal_read_cell_grapheme(term, 9, 0, buf.as_mut_ptr(), 8) },
            0
        );
        assert_eq!(
            unsafe {
                ptec_terminal_read_cell_grapheme(std::ptr::null(), 0, 0, buf.as_mut_ptr(), 8)
            },
            0
        );
        unsafe { ptec_terminal_free(term) };
    }

    /// The `TERM_EVENT_*` codes are exactly 1..=26 with no gaps or
    /// duplicates, so a new `TerminalEventKind` must take the next code.
    #[test]
    fn term_event_codes_are_dense_and_unique() {
        let mut codes: Vec<u16> = TERM_EVENT_CODES.iter().map(|&(_, c)| c).collect();
        codes.sort_unstable();
        let want: Vec<u16> = (1..=TERM_EVENT_CODES.len() as u16).collect();
        assert_eq!(codes, want);
    }
}
