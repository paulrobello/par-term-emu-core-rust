//! Host-cursor placement bookkeeping for [`super::WindowSession::frame`].

use super::{trace_cursor, PaneRenderer};
use crate::mux::attach::CURSOR_HEARTBEAT;
use par_term_emu_core::cursor::CursorStyle;
use std::time::Instant;

/// A placed cursor: host column, host row, shape.
pub(super) type PlacedCursor = Option<(u16, u16, CursorStyle)>;

pub(super) struct CursorState {
    /// The cursor state the last `place_cursor` reported, `Some(None)`
    /// initially (repaint_all hides the cursor): the guard that keeps a
    /// quiet pump from re-emitting identical cursor escapes every frame
    /// tick, while a flushed frame (whose per-cell CUP left the terminal
    /// cursor wherever the last diff cell sits) always repositions.
    placed: Option<PlacedCursor>,
    /// The instant the last `place_cursor` wrote, heartbeat-gated: a
    /// quiet pump emits nothing, and a host terminal that lost the
    /// cursor state (an output race outside the guarded writers) would
    /// keep it lost forever, so every frame re-asserts the placement
    /// after [`CURSOR_HEARTBEAT`].
    last_emit: Instant,
    /// The `[client] change-cursor-shape` setting in force: shapes the
    /// host POINTER (OSC 22) over resize handles, and while a divider
    /// drag is live the host text cursor (best-effort DECSCUSR steady
    /// block; restored on drag end).
    pub(super) change_cursor_shape: bool,
    /// The hidden-cursor IME anchor the last frame parked at (host
    /// cell), `None` when none was parked.
    anchor: Option<(u16, u16)>,
}

impl CursorState {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            placed: Some(None),
            last_emit: now,
            change_cursor_shape: false,
            anchor: None,
        }
    }

    /// Whether the frame must park the hidden cursor at `anchor`: after
    /// every cursor emit (a flushed diff left the host cursor at its
    /// last cell) and whenever the anchor moved. No heartbeat: a hidden
    /// cursor has no visible state to drift, and every flush re-parks.
    pub(super) fn decide_anchor(&mut self, anchor: Option<(u16, u16)>, emitted: bool) -> bool {
        let moved = self.anchor != anchor;
        self.anchor = anchor;
        anchor.is_some() && (emitted || moved)
    }

    /// The frame's emit decision: true when the sink must be written with
    /// `cursor`. Updates the guard and the heartbeat clock when it does.
    pub(super) fn decide(&mut self, cursor: PlacedCursor, flushed: bool, now: Instant) -> bool {
        let belief_changed = self.placed.as_ref() != Some(&cursor);
        // Heartbeat: every guard is belief-based, so a host terminal
        // that lost the cursor state outside the guarded writers
        // never re-syncs on its own: re-assert the placement (idempotent
        // on a healthy host) once per heartbeat so any desync
        // self-heals within a second.
        let heartbeat_due =
            cursor.is_some() && now.saturating_duration_since(self.last_emit) >= CURSOR_HEARTBEAT;
        if flushed || belief_changed || heartbeat_due {
            trace_cursor(format_args!(
                "frame emit (flushed={flushed}, belief-changed={belief_changed}, heartbeat={heartbeat_due})"
            ));
            self.placed = Some(cursor);
            self.last_emit = now;
            true
        } else {
            false
        }
    }

    /// Forget the recorded placement so the next frame re-emits (a window
    /// re-seed, a refit, a repaint-all).
    pub(super) fn invalidate(&mut self) {
        self.placed = Some(None);
        self.anchor = None;
    }

    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: std::time::Duration) {
        self.last_emit = Instant::now() - by;
    }
}

impl PaneRenderer {
    /// The terminal cursor position for the focused pane, mapped through
    /// its rect origin and the client scroll offset: `Some((x, y, style))`
    /// where `(x, y)` is the window-relative cell to place the host
    /// cursor at. `None` — the cursor hides — when nothing is focused,
    /// the pane's emulator tracks a hidden cursor (DECTCEM), the client
    /// view is scrolled off live (the wheel/prefix-[ viewport: the live
    /// cell is not on screen), or the tracked cell is somehow outside
    /// the pane's rect.
    ///
    /// The style is the pane's tracked DECSCUSR shape, for the sink to
    /// re-emit (`CSI Ps SP q`).
    pub fn focused_cursor(&self) -> Option<(u16, u16, CursorStyle)> {
        self.focused_cell(true)
    }

    /// Where the host cursor PARKS while the focused pane hides its
    /// cursor (DECTCEM off): the same tracked cell as
    /// [`Self::focused_cursor`], minus the visibility test. The host
    /// terminal anchors IME preedit and the candidate window at its
    /// cursor even when hidden, so a TUI that paints its own cursor
    /// (agent CLIs) keeps the composition at the input line instead of
    /// at whatever cell the last diff wrote. `None` for every other
    /// hide reason (scrolled view, nothing focused, cell outside view).
    pub fn focused_ime_anchor(&self) -> Option<(u16, u16)> {
        self.focused_cell(false).map(|(x, y, _)| (x, y))
    }

    fn focused_cell(&self, require_visible: bool) -> Option<(u16, u16, CursorStyle)> {
        let Some(focus) = self.focused else {
            return super::trace_none(format_args!("no focused pane"));
        };
        let Some(rect) = self.layout.iter().find(|r| r.pane == focus) else {
            return super::trace_none(format_args!("focused pane has no layout rect"));
        };
        let Some(emulator) = self.emulators.get(&focus) else {
            return super::trace_none(format_args!("no emulator for focused pane"));
        };
        if emulator.scroll_offset() > 0 {
            return super::trace_none(format_args!(
                "view scrolled: offset {}",
                emulator.scroll_offset()
            ));
        }
        let cursor = emulator.terminal().cursor();
        if require_visible && !cursor.visible {
            return super::trace_none(format_args!("pane tracked cursor hidden by DECTCEM"));
        }
        let (col, row) = (cursor.col, cursor.row);
        // The placement path shares the paint inset: with the per-pane
        // border option the cursor maps inside the border ring, hidden
        // when the tracked cell falls in the cropped perimeter band.
        let (inset_x, inset_y, view_w, view_h) = self.content_view(rect);
        if col >= usize::from(view_w) || row >= usize::from(view_h) {
            return super::trace_none(format_args!(
                "tracked cell ({col},{row}) outside content view {view_w}x{view_h}"
            ));
        }
        let (x, y) = self
            .geometry
            .content_to_frame(rect.x + inset_x + col as u16, rect.y + inset_y + row as u16);
        Some((x, y, cursor.style))
    }
}
