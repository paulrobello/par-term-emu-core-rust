//! FFI-safe types and C API for embedding the terminal emulator
//!
//! This module provides `#[repr(C)]` types that can be safely shared across
//! FFI boundaries (Swift, Kotlin/JNI, C/C++, etc.) and extern "C" functions
//! for creating, querying, and observing terminal state.

use std::collections::HashSet;
use std::ffi::{c_char, CString};
use std::sync::Arc;

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
#[derive(Debug, Clone)]
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
        let title_str = term.title();
        let title_len = title_str.len() as u32;
        let title_cstring = CString::new(title_str).unwrap_or_else(|_| CString::new("").unwrap());
        let title = title_cstring.into_raw();

        // CWD
        let cwd_opt = term.current_directory();
        let (cwd, cwd_len) = match cwd_opt {
            Some(s) => {
                let len = s.len() as u32;
                let cs = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
                (cs.into_raw(), len)
            }
            None => (std::ptr::null_mut(), 0u32),
        };

        // Cells
        let cell_count = (cols * rows) as u32;
        let mut cells_vec: Vec<SharedCell> = Vec::with_capacity(cols * rows);

        for row_idx in 0..rows {
            if let Some(row_cells) = grid.row(row_idx) {
                for col_idx in 0..cols {
                    if col_idx < row_cells.len() {
                        let cell = &row_cells[col_idx];
                        let mut text = [0u8; 4];
                        let encoded = cell.c.encode_utf8(&mut text);
                        let text_len = encoded.len() as u8;

                        let (fg_r, fg_g, fg_b) = cell.fg.to_rgb();
                        let (bg_r, bg_g, bg_b) = cell.bg.to_rgb();
                        let attrs = cell.flags.to_bitflags();

                        cells_vec.push(SharedCell {
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
                        });
                    } else {
                        // Pad with default (space) cells
                        cells_vec.push(SharedCell {
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
                        });
                    }
                }
            } else {
                // Row doesn't exist — fill with default cells
                for _ in 0..cols {
                    cells_vec.push(SharedCell {
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
                    });
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
/// Each function pointer receives the `user_data` pointer and a JSON-encoded
/// event description as a NUL-terminated C string. The callee must NOT free
/// the event string — it is owned by the caller and valid only for the
/// duration of the callback.
#[repr(C)]
pub struct TerminalObserverVtable {
    /// Called for zone lifecycle events
    pub on_zone_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_json: *const c_char)>,
    /// Called for command/shell integration events
    pub on_command_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_json: *const c_char)>,
    /// Called for environment change events
    pub on_environment_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_json: *const c_char)>,
    /// Called for screen content events
    pub on_screen_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_json: *const c_char)>,
    /// Called for ALL events (catch-all)
    pub on_event:
        Option<unsafe extern "C" fn(user_data: *mut std::ffi::c_void, event_json: *const c_char)>,
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

    /// Format a terminal event as a simple JSON-ish debug string and call an
    /// FFI callback with it.
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
