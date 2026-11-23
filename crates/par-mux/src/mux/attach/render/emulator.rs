//! One pane's local emulator: the core terminal fed the pane's wire bytes, plus
//! the client-side scroll view.

use par_term_emu_core::mouse::MouseMode;
use par_term_emu_core::terminal::Terminal;

/// One pane's local emulator: the core [`Terminal`] fed the pane's wire
/// bytes, at the pane's layout geometry.
pub struct PaneEmulator {
    pub(super) term: Terminal,
    /// Client-side scroll offset into the pane's scrollback (0 = live).
    /// Driven by the wheel when the pane does not own mouse mode; every
    /// keystroke forwarded to the pane snaps it back to live.
    pub(super) scroll: usize,
    /// Scroll hold: pane output does not snap the view back to live
    /// while held (the copy-mode viewport, or a wheel-scrolled
    /// view — the wheel arms the hold so a pane that keeps writing,
    /// like omp's streaming chat, cannot instantly swallow the scroll).
    /// The offset grows by the lines the pane pushes into scrollback so
    /// the view stays on its content, clamped to the extent.
    pub(super) hold_scroll: bool,
}

impl PaneEmulator {
    /// A fresh emulator at `cols` x `rows`.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            term: Terminal::new(cols as usize, rows as usize),
            scroll: 0,
            hold_scroll: false,
        }
    }

    /// A fresh emulator whose image math uses `cell_px` pixels per cell.
    pub(crate) fn with_cell_px(cols: u16, rows: u16, cell_px: (u32, u32)) -> Self {
        let mut emulator = Self::new(cols, rows);
        emulator.term.set_cell_dimensions(cell_px.0, cell_px.1);
        emulator
    }

    /// Re-derive image cell spans from `cell_px` pixels per cell.
    pub(crate) fn set_cell_px(&mut self, cell_px: (u32, u32)) {
        self.term.set_cell_dimensions(cell_px.0, cell_px.1);
    }

    /// Drop every graphic the emulator holds.
    pub(crate) fn clear_graphics(&mut self) {
        self.term.clear_graphics();
    }

    /// The emulator's grid state.
    pub fn terminal(&self) -> &Terminal {
        &self.term
    }

    /// The client-side scroll offset (0 = live bottom).
    pub fn scroll_offset(&self) -> usize {
        self.scroll
    }

    /// Whether the pane owns mouse tracking (its emulator has a mouse mode
    /// other than off — tracked from the pane's own DECSET 1000/1002/1003
    /// and replay bytes).
    pub fn owns_mouse(&self) -> bool {
        self.term.mouse_mode() != MouseMode::Off
    }

    /// The pane's mouse encoding (its negotiated 1005/1006/1015), for the
    /// router's forward decision.
    #[cfg(test)]
    pub fn mouse_encoding(&self) -> par_term_emu_core::mouse::MouseEncoding {
        self.term.mouse_encoding()
    }

    /// The pane's DECCKM application-cursor mode, for the key re-encoder.
    #[cfg(test)]
    pub fn application_cursor(&self) -> bool {
        self.term.application_cursor()
    }

    /// Feed the pane's byte stream — a `refresh-client` replay body or a
    /// `%output` chunk — through the emulator. Any fed byte resets the
    /// pane's client scroll to live: the pane wrote, so the view snaps
    /// to the bottom. While scroll hold is set ([`Self::set_scroll_hold`],
    /// the prefix-[ viewport or a wheel-scrolled view), the offset
    /// survives instead and tracks the lines the pane pushes into
    /// scrollback, so the view stays put relative to its CONTENT (tmux's
    /// copy-mode behavior), clamped to the scrollback extent. Returns
    /// the lines pushed into scrollback while held (0 otherwise) — the
    /// copy-mode state's rebase delta.
    pub fn feed(&mut self, bytes: &[u8]) -> usize {
        let sb_before = if self.hold_scroll {
            Some(self.term.active_grid().scrollback_len())
        } else {
            None
        };
        self.term.process(bytes);
        // The daemon's PTY session already answers the pane's queries and
        // owns its bells; a mirror must never answer on its behalf, and
        // undrained queues would grow with the pane's output (SEC-207).
        let _ = self.term.drain_responses();
        let _ = self.term.drain_bell_events();
        let pushed = if self.hold_scroll {
            let max = self.term.active_grid().scrollback_len();
            let pushed = max.saturating_sub(sb_before.unwrap_or(max));
            self.scroll = (self.scroll + pushed).min(max);
            pushed
        } else {
            self.scroll = 0;
            0
        };
        pushed
    }

    /// Scroll this pane's client-side view by `delta` lines (positive =
    /// up into history). Clamped to the scrollback extent; returns the
    /// resulting offset. Pane output ([`Self::feed`]) resets to live.
    pub fn scroll_by(&mut self, delta: isize) -> usize {
        let max = self.term.active_grid().scrollback_len();
        let current = self.scroll as isize;
        self.scroll = (current + delta).clamp(0, max as isize) as usize;
        self.scroll
    }

    /// Set the scroll-mode hold. Entering the prefix-[ viewport holds;
    /// leaving clears it (clearing does NOT snap — the caller snaps
    /// explicitly with [`Self::scroll_to_live`]).
    pub fn set_scroll_hold(&mut self, hold: bool) {
        self.hold_scroll = hold;
    }

    /// Snap the client view back to the live bottom.
    pub fn scroll_to_live(&mut self) {
        self.scroll = 0;
    }

    /// Re-fit to a new geometry (`refresh-client -C` re-division).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols as usize, rows as usize);
    }
}
