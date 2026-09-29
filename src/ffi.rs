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
    /// Bitfield of cell attributes (bold, italic, etc.) — see `CellBitflags`
    pub attrs: u16,
    /// Display width of the character (typically 1 or 2)
    pub width: u8,
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

        // Mouse mode mapping
        let mouse_mode = match term.mouse_mode() {
            MouseMode::Off => 0u8,
            MouseMode::X10 => 1,
            MouseMode::Normal => 2,
            MouseMode::ButtonEvent => 3,
            MouseMode::AnyEvent => 4,
        };

        // Title
        let (title, title_len) = to_c_string(term.title());

        // CWD
        let (cwd, cwd_len) = match term.current_directory() {
            Some(s) => to_c_string(s),
            None => (std::ptr::null_mut(), 0u32),
        };

        // Cells
        let cell_count = (cols * rows) as u32;
        let mut cells_vec: Vec<SharedCell> = Vec::with_capacity(cols * rows);

        for row_idx in 0..rows {
            if let Some(row_cells) = grid.row(row_idx) {
                for col_idx in 0..cols {
                    let cell = row_cells
                        .get(col_idx)
                        .map(SharedCell::from_cell)
                        .unwrap_or_else(SharedCell::blank);
                    cells_vec.push(cell);
                }
            } else {
                // Row doesn't exist — fill with default cells
                for _ in 0..cols {
                    cells_vec.push(SharedCell::blank());
                }
            }
        }

        // Convert Vec to raw pointer — we now own the allocation
        let mut cells_boxed = cells_vec.into_boxed_slice();
        let cells = cells_boxed.as_mut_ptr();
        std::mem::forget(cells_boxed);

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
            unsafe {
                let _ = CString::from_raw(self.title);
            }
            self.title = std::ptr::null_mut();
        }

        // Free the cwd CString
        if !self.cwd.is_null() {
            unsafe {
                let _ = CString::from_raw(self.cwd);
            }
            self.cwd = std::ptr::null_mut();
        }

        // Free the cells array
        if !self.cells.is_null() && self.cell_count > 0 {
            unsafe {
                let slice = std::slice::from_raw_parts_mut(self.cells, self.cell_count as usize);
                let _ = Box::from_raw(slice as *mut [SharedCell]);
            }
            self.cells = std::ptr::null_mut();
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalObserverVtable — C function-pointer table for observers
// ---------------------------------------------------------------------------

/// A C-compatible vtable for terminal event observation.
///
/// Each function pointer receives the `user_data` pointer and a
/// Debug-formatted (`{:?}`) event description as a NUL-terminated C string.
/// The callee must NOT free the event string — it is owned by the caller and
/// valid only for the duration of the callback.
#[repr(C)]
pub struct TerminalObserverVtable {
    /// Called for zone lifecycle events
    pub on_zone_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>,
    /// Called for command/shell integration events
    pub on_command_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>,
    /// Called for environment change events
    pub on_environment_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>,
    /// Called for screen content events
    pub on_screen_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>,
    /// Called for ALL events (catch-all)
    pub on_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_text: *const c_char)>,
    /// Opaque pointer passed to every callback
    pub user_data: *mut std::ffi::c_void,
}

// SAFETY: The user_data pointer is opaque and the FFI contract requires the
// caller to ensure thread safety of the data it points to.
unsafe impl Send for TerminalObserverVtable {}
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
    /// an FFI callback with it.
    fn call_callback(
        &self,
        cb: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *const c_char)>,
        event: &TerminalEvent,
    ) {
        if let Some(f) = cb {
            let desc = format!("{:?}", event);
            if let Ok(cstr) = CString::new(desc) {
                unsafe {
                    f(self.vtable.user_data, cstr.as_ptr());
                }
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

/// Cursor position and style, C-compatible.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCursorState {
    pub col: u32,
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
    pub cols: u32,
    pub rows: u32,
}

impl SharedCell {
    /// Build a `SharedCell` from one grid cell.
    fn from_cell(cell: &crate::cell::Cell) -> Self {
        let mut text = [0u8; 4];
        let text_len = cell.c.encode_utf8(&mut text).len() as u8;
        let (fg_r, fg_g, fg_b) = cell.fg.to_rgb();
        let (bg_r, bg_g, bg_b) = cell.bg.to_rgb();
        SharedCell {
            text,
            text_len,
            fg_r,
            fg_g,
            fg_b,
            bg_r,
            bg_g,
            bg_b,
            attrs: cell.flags.to_bitflags(),
            width: cell.width,
        }
    }

    /// The default (space) cell used to pad short rows.
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

/// Create a terminal for C embedding.
///
/// # Safety
/// Caller owns the returned `Terminal` and must release it with
/// `terminal_free`. Returns null on allocation failure.
#[no_mangle]
pub unsafe extern "C" fn terminal_create(cols: u32, rows: u32, scrollback: u32) -> *mut Terminal {
    if cols == 0 || rows == 0 {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(Terminal::with_scrollback(
        cols as usize,
        rows as usize,
        scrollback as usize,
    )))
}

/// Free a `Terminal` created by `terminal_create`.
///
/// # Safety
/// `term` must have been returned by `terminal_create` and must not be
/// used after this call.
#[no_mangle]
pub unsafe extern "C" fn terminal_free(term: *mut Terminal) {
    if !term.is_null() {
        drop(unsafe { Box::from_raw(term) });
    }
}

/// Feed raw PTY/application output bytes into the terminal (VT parsing).
///
/// # Safety
/// `bytes` must be valid for reads of `len` bytes. `term` must be a valid
/// pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_feed(term: *mut Terminal, bytes: *const u8, len: u32) {
    if term.is_null() || bytes.is_null() {
        return;
    }
    let term_ref = unsafe { &mut *term };
    let data = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    term_ref.process(data);
}

/// Resize the terminal grid.
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_resize(term: *mut Terminal, cols: u32, rows: u32) {
    if term.is_null() || cols == 0 || rows == 0 {
        return;
    }
    let term_ref = unsafe { &mut *term };
    term_ref.resize(cols as usize, rows as usize);
}

/// Coalesce the dirty-row bitset into inclusive row ranges.
///
/// Writes up to `cap` ranges into `out` (caller-owned) and returns the
/// total range count — if the return exceeds `cap`, call again with a
/// larger buffer. A renderer redraws only rows inside the returned ranges.
///
/// # Safety
/// `out` must be valid for writes of `cap` `TermRowRange` values when the
/// total is being fetched it may be null with cap 0.
#[no_mangle]
pub unsafe extern "C" fn terminal_dirty_ranges(
    term: *const Terminal,
    out: *mut TermRowRange,
    cap: u32,
) -> u32 {
    if term.is_null() {
        return 0;
    }
    let term_ref = unsafe { &*term };
    let rows = term_ref.get_dirty_rows();
    let mut ranges: Vec<TermRowRange> = Vec::new();
    for &row in &rows {
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
    let fill = (ranges.len() as u32).min(cap);
    if fill > 0 && !out.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(ranges.as_ptr(), out, fill as usize);
        }
    }
    ranges.len() as u32
}

/// Mark the screen clean (all damage consumed).
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_mark_clean(term: *mut Terminal) {
    if term.is_null() {
        return;
    }
    unsafe { &mut *term }.mark_clean();
}

/// Copy a run of grid cells into a caller-owned buffer (pinned readback —
/// no allocation, no full-grid copy). Returns the number of cells written.
///
/// # Safety
/// `out` must be valid for writes of `cap` `SharedCell` values.
#[no_mangle]
pub unsafe extern "C" fn terminal_read_row(
    term: *const Terminal,
    row: u32,
    col_start: u32,
    out: *mut SharedCell,
    cap: u32,
) -> u32 {
    if term.is_null() || out.is_null() {
        return 0;
    }
    let term_ref = unsafe { &*term };
    let grid = term_ref.active_grid();
    let Some(row_cells) = grid.row(row as usize) else {
        return 0;
    };
    let cols = grid.cols() as u32;
    let mut written = 0u32;
    let mut col = col_start;
    while col < cols && written < cap {
        let cell = row_cells
            .get(col as usize)
            .map(SharedCell::from_cell)
            .unwrap_or_else(SharedCell::blank);
        unsafe { *out.add(written as usize) = cell };
        col += 1;
        written += 1;
    }
    written
}

/// Copy a run of scrollback cells into a caller-owned buffer.
/// `line` indexes scrollback from the oldest (0) to the newest
/// (`scrollback_count - 1`). Returns the number of cells written.
///
/// # Safety
/// `out` must be valid for writes of `cap` `SharedCell` values.
#[no_mangle]
pub unsafe extern "C" fn terminal_read_scrollback_row(
    term: *const Terminal,
    line: u32,
    col_start: u32,
    out: *mut SharedCell,
    cap: u32,
) -> u32 {
    if term.is_null() || out.is_null() {
        return 0;
    }
    let term_ref = unsafe { &*term };
    let grid = term_ref.active_grid();
    let Some(line_cells) = grid.scrollback_line(line as usize) else {
        return 0;
    };
    let cols = grid.cols() as u32;
    let mut written = 0u32;
    let mut col = col_start;
    while col < cols && written < cap {
        let cell = line_cells
            .get(col as usize)
            .map(SharedCell::from_cell)
            .unwrap_or_else(SharedCell::blank);
        unsafe { *out.add(written as usize) = cell };
        col += 1;
        written += 1;
    }
    written
}

/// Number of lines currently held in scrollback.
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_scrollback_count(term: *const Terminal) -> u32 {
    if term.is_null() {
        return 0;
    }
    unsafe { &*term }.active_grid().scrollback_len() as u32
}

/// Read cursor position/style.
///
/// # Safety
/// `out` must be valid for writes of one `TermCursorState`.
#[no_mangle]
pub unsafe extern "C" fn terminal_get_cursor(term: *const Terminal, out: *mut TermCursorState) {
    if term.is_null() || out.is_null() {
        return;
    }
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
pub unsafe extern "C" fn terminal_get_modes(term: *const Terminal, out: *mut TermModeState) {
    if term.is_null() || out.is_null() {
        return;
    }
    let term_ref = unsafe { &*term };
    let mouse_mode = match term_ref.mouse_mode() {
        MouseMode::Off => 0u8,
        MouseMode::X10 => 1,
        MouseMode::Normal => 2,
        MouseMode::ButtonEvent => 3,
        MouseMode::AnyEvent => 4,
    };
    let (cols, rows) = term_ref.size();
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
/// for writes of `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn terminal_encode_key(
    term: *const Terminal,
    ev: *const crate::keyboard::TermKeyEvent,
    out: *mut u8,
    cap: u32,
) -> u32 {
    if term.is_null() || ev.is_null() || out.is_null() {
        return 0;
    }
    let term_ref = unsafe { &*term };
    let ev_ref = unsafe { &*ev };
    let bytes = crate::keyboard::encode_key(ev_ref, term_ref);
    let fill = (bytes.len() as u32).min(cap);
    if fill > 0 {
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
/// `terminal_free_state`.
///
/// # Safety
/// `term` must be a valid pointer to a `Terminal`.
#[no_mangle]
pub unsafe extern "C" fn terminal_get_state(term: *const Terminal) -> *mut SharedState {
    if term.is_null() {
        return std::ptr::null_mut();
    }
    let term_ref = unsafe { &*term };
    let state = SharedState::from_terminal(term_ref);
    Box::into_raw(Box::new(state))
}

/// Free a `SharedState` previously returned by `terminal_get_state`.
///
/// # Safety
/// `state` must be a pointer previously returned by `terminal_get_state`,
/// and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn terminal_free_state(state: *mut SharedState) {
    if !state.is_null() {
        unsafe {
            let _ = Box::from_raw(state);
        }
    }
}

/// Register an FFI observer on the terminal.
///
/// Returns an observer ID that can be passed to `terminal_remove_observer`.
///
/// # Safety
/// `term` must be a valid, mutable pointer to a `Terminal`.
/// The `vtable` must remain valid (including its `user_data`) for as long as
/// the observer is registered.
#[no_mangle]
pub unsafe extern "C" fn terminal_add_observer(
    term: *mut Terminal,
    vtable: TerminalObserverVtable,
) -> u64 {
    if term.is_null() {
        return 0;
    }
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
pub unsafe extern "C" fn terminal_remove_observer(term: *mut Terminal, id: u64) -> bool {
    if term.is_null() {
        return false;
    }
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
        assert_eq!(size_of::<TerminalObserverVtable>(), 48);
        assert_eq!(size_of::<TermRowRange>(), 8);
        assert_eq!(size_of::<TermCursorState>(), 12);
        assert_eq!(size_of::<TermModeState>(), 20);
        assert_eq!(size_of::<crate::keyboard::TermKeyEvent>(), 8);
        assert_eq!(offset_of!(crate::keyboard::TermKeyEvent, codepoint), 4);
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
        let n = unsafe { terminal_read_row(term, row as u32, 0, buf.as_mut_ptr(), RT_COLS as u32) };
        assert_eq!(n as usize, RT_COLS);
        buf
    }

    /// The core's own view of a row, through the same SharedCell conversion.
    fn reference_row(term: &Terminal, row: usize) -> Vec<SharedCell> {
        let grid = term.active_grid();
        grid.row(row)
            .expect("row exists")
            .iter()
            .map(SharedCell::from_cell)
            .chain(std::iter::repeat(SharedCell::blank()))
            .take(RT_COLS)
            .collect()
    }

    #[test]
    fn ffi_round_trip_matches_core_state() {
        let term = unsafe { terminal_create(RT_COLS as u32, RT_ROWS as u32, 200) };
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
            unsafe { terminal_feed(term, frame.as_ptr(), frame.len() as u32) };

            // 1. Screen byte-for-byte: FFI readback == the core's own cells.
            for row in 0..RT_ROWS {
                let ffi = read_row_ffi(term, row);
                let core = reference_row(unsafe { &*term }, row);
                assert_eq!(ffi, core, "frame {i}: row {row} FFI != core");
            }

            // 2. Damage completeness: every row that differs from the
            //    previous frame must be inside a dirty range.
            let cap = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            let got = unsafe { terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
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

            unsafe { terminal_mark_clean(term) };
            let after = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            assert_eq!(after, 0, "frame {i}: mark_clean left dirty ranges");
        }

        // 3. Scrollback: FFI readback matches the core's scrollback cells,
        //    oldest first (line 0 = oldest).
        let sb = unsafe { terminal_scrollback_count(term) };
        assert!(sb >= 3, "expected scrollback after 8-line frame, got {sb}");
        let grid = unsafe { &*term }.active_grid();
        for line in 0..sb as usize {
            let mut buf = vec![SharedCell::blank(); RT_COLS];
            let n = unsafe {
                terminal_read_scrollback_row(term, line as u32, 0, buf.as_mut_ptr(), RT_COLS as u32)
            };
            assert_eq!(n as usize, RT_COLS);
            let core: Vec<SharedCell> = grid
                .scrollback_line(line)
                .expect("scrollback line exists")
                .iter()
                .map(SharedCell::from_cell)
                .collect();
            assert_eq!(buf, core, "scrollback line {line} FFI != core");
        }

        // 3b. RIS clears screen and scrollback, so it runs after the
        // scrollback readback: every row must land in a dirty range
        // (ARC-058's damage contract, pinned here at the FFI boundary).
        {
            unsafe { terminal_feed(term, b"\x1bc".as_ptr(), 2) };
            let cap = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
            let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
            unsafe { terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
            for row in 0..RT_ROWS {
                let inside = ranges
                    .iter()
                    .any(|r| row >= r.start as usize && row <= r.end as usize);
                assert!(inside, "RIS: row {row} not in any dirty range {ranges:?}");
            }
            let sb = unsafe { terminal_scrollback_count(term) };
            assert_eq!(sb, 0, "RIS clears scrollback");
        }

        // 4. Cursor + modes match the core's accessors.
        let mut cur = TermCursorState {
            col: 0,
            row: 0,
            visible: false,
            style: 0,
        };
        unsafe { terminal_get_cursor(term, &mut cur) };
        let c = unsafe { &*term }.cursor();
        assert_eq!(cur.col as usize, c.col);
        assert_eq!(cur.row as usize, c.row);
        assert_eq!(cur.visible, c.visible);

        let mut modes = TermModeState::default();
        unsafe { terminal_get_modes(term, &mut modes) };
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
            let n = unsafe { terminal_encode_key(term, &ev, buf.as_mut_ptr(), 32) };
            assert_eq!(
                &buf[..n as usize],
                crate::keyboard::encode_key(&ev, unsafe { &*term }).as_slice()
            );
        }

        unsafe { terminal_free(term) };
    }

    #[test]
    fn ffi_dirty_ranges_coalesce_and_resize_marks_damage() {
        let term = unsafe { terminal_create(10, 6, 50) };
        unsafe { terminal_feed(term, b"\x1b[Hrow0\x1b[3Hrow2\x1b[5Hrow4".as_ptr(), 26) };

        let cap = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
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
        unsafe { terminal_feed(term, b"\x1b[2Hxxxx\x1b[4Hxxxx".as_ptr(), 18) };
        let cap = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
        assert_eq!(ranges, vec![TermRowRange { start: 0, end: 4 }]);

        // Resize must mark the whole (new) screen dirty.
        unsafe { terminal_mark_clean(term) };
        unsafe { terminal_resize(term, 12, 4) };
        let cap = unsafe { terminal_dirty_ranges(term, std::ptr::null_mut(), 0) };
        let mut ranges = vec![TermRowRange { start: 0, end: 0 }; cap as usize];
        unsafe { terminal_dirty_ranges(term, ranges.as_mut_ptr(), cap) };
        assert_eq!(ranges, vec![TermRowRange { start: 0, end: 3 }]);

        unsafe { terminal_free(term) };
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

    /// QA-151: an arbitrary uint16_t in `TermKeyEvent.key` — a value C or
    /// Swift is free to write — encodes to nothing instead of being
    /// undefined behavior. key=2 is not a discriminant, 57388 sits between
    /// the F-keys and Insert, 0xFFFF is the extreme.
    #[test]
    fn encode_key_rejects_invalid_discriminants_without_ub() {
        let term = unsafe { terminal_create(20, 5, 100) };
        assert!(!term.is_null());
        let mut out = [0u8; 16];
        for raw in [2u16, 57388, 0xFFFF] {
            let ev = crate::keyboard::TermKeyEvent {
                key: raw,
                modifiers: 0,
                _pad: 0,
                codepoint: 0,
            };
            let n = unsafe { terminal_encode_key(term, &ev, out.as_mut_ptr(), out.len() as u32) };
            assert_eq!(n, 0, "raw key {raw} has no encoding");
        }
        // A real key still encodes through the same path.
        let ev = crate::keyboard::TermKeyEvent::functional(crate::keyboard::TermKey::Up, 0);
        let n = unsafe { terminal_encode_key(term, &ev, out.as_mut_ptr(), out.len() as u32) };
        assert_eq!(&out[..n as usize], b"\x1b[A");
        unsafe { terminal_free(term) };
    }
}
