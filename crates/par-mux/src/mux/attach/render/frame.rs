//! The frame model (ARC-004): the double buffers, the staleness flag,
//! and the resize/invalidate/diff cycle — the owner of "the current
//! composite is stale", separate from the glyph rendering that fills
//! the buffer. [`super::PaneRenderer`] composes this; every refit path
//! goes through [`FrameBuffers::resize`] or [`FrameBuffers::invalidate`],
//! so the ENH-044 baseline invariant (both buffers re-fitted together)
//! holds by construction instead of by caller discipline.

use ratatui::buffer::{Buffer, Cell as RtCell};
use ratatui::layout::Rect as RtRect;
use ratatui::style::{Color as RtColor, Style as RtStyle};

pub(super) struct FrameBuffers {
    pub(super) width: u16,
    pub(super) height: u16,
    /// The frame being painted.
    pub(super) buffer: Buffer,
    /// The last frame handed out; the diff reads it as the baseline.
    pub(super) prev_buffer: Buffer,
    /// The composite is stale — the next frame repaints it.
    pub(super) dirty: bool,
}

impl FrameBuffers {
    pub(super) fn new(width: u16, height: u16) -> Self {
        let area = RtRect::new(0, 0, width, height);
        Self {
            width,
            height,
            buffer: Buffer::empty(area),
            prev_buffer: Buffer::empty(area),
            dirty: true,
        }
    }

    /// Re-fit the buffers to `cols` x `rows`: both rebuild at the new
    /// extent (ENH-044 — a half-refitted baseline diffs garbage), and
    /// the frame marks stale. An unchanged size is a no-op.
    pub(super) fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == (self.width, self.height) {
            return;
        }
        let area = RtRect::new(0, 0, cols, rows);
        self.width = cols;
        self.height = rows;
        self.buffer = Buffer::empty(area);
        self.prev_buffer = Buffer::empty(area);
        self.dirty = true;
    }

    /// Forget the last painted frame so the next frame's diff emits
    /// every cell: the host screen no longer holds that frame (a
    /// `repaint_all` cleared it, or the view switched windows).
    /// `begin_frame` swaps the buffers first, so the baseline the next
    /// diff reads is the current `buffer`; both are cleared.
    pub(super) fn invalidate(&mut self) {
        let area = RtRect::new(0, 0, self.width, self.height);
        self.buffer = Buffer::empty(area);
        self.prev_buffer = Buffer::empty(area);
        self.dirty = true;
    }

    /// The frame's full extent — the host grid the seed and `resize`
    /// derived.
    pub(super) fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Open the next frame: `false` when the composite is clean (the
    /// caller hands back an empty diff). Otherwise consume the stale
    /// mark and make `buffer` the painting surface: the last frame
    /// becomes the diff baseline by a swap, not a clone (ARC-132 — both
    /// buffers share the area, and the recycled buffer is cleared next),
    /// then the buffer resets to the host background fill. A failed
    /// probe (`None`) paints NO color, so bg-fill cells stay at the
    /// terminal default and can never mismatch the host theme.
    pub(super) fn begin_frame(&mut self, bg: Option<RtColor>) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        std::mem::swap(&mut self.buffer, &mut self.prev_buffer);
        self.buffer.reset();
        if let Some(bg) = bg {
            let bg_style = RtStyle::default().bg(bg);
            self.buffer
                .set_style(RtRect::new(0, 0, self.width, self.height), bg_style);
        }
        true
    }

    /// The painted frame's cell diff against the baseline.
    pub(super) fn diff(&self) -> Vec<(u16, u16, RtCell)> {
        self.prev_buffer
            .diff(&self.buffer)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone()))
            .collect()
    }
}
