//! The client's selection and the drags that build it: the selection
//! shapes the paint path highlights, and the two drag state machines
//! (divider grab, text sweep) the pointer routes through.

use super::DividerHit;

/// A divider drag in flight. `Pending` is a press near a divider that has
/// not moved yet — it must not focus or forward (a divider press is not a
/// click-through); the first motion promotes it to `Active`, which
/// accumulates the pointer's signed cell delta from the press point for
/// the pump's frame-cadence application.
pub(super) enum DragState {
    Pending {
        divider: DividerHit,
        x: u16,
        y: u16,
    },
    Active {
        divider: DividerHit,
        x: u16,
        y: u16,
        applied: i32,
        pending: i32,
        /// When the last resize step was sent — the drag's send
        /// throttle. Every step is a daemon layout change that repaints
        /// the whole window, and pointer motion arrives far faster than
        /// that should happen.
        last_sent: Option<std::time::Instant>,
    },
}

/// A text-selection drag in flight — the mouse's sweep over a pane that
/// is not capturing the pointer. `Pending` is a press that has not moved
/// yet (a plain click: it must neither highlight nor copy); the first
/// motion promotes it to `Active` with the anchor the press stored, and
/// release copies the swept text. The pane id and anchor live in the
/// renderer's [`Selection::Chars`] once promoted.
pub(super) enum TextDrag {
    Pending { pane: u32, anchor: (usize, usize) },
    Active { pane: u32 },
}

/// What the client has selected on screen. Lines are the pane's ABSOLUTE
/// line coordinate — scrollback lines first (oldest at 0), then the live
/// grid's rows — the coordinate both painting and text extraction share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selection {
    /// Scroll mode's whole-line selection: absolute lines `a..=b`, set
    /// by `v` and extended by the scrolling viewport.
    Lines { pane: u32, a: usize, b: usize },
    /// The mouse drag's character selection: anchor and head are
    /// `(absolute line, column)`. Reading order between the two cells
    /// reverses whichever way the drag ran.
    Chars {
        pane: u32,
        anchor: (usize, usize),
        head: (usize, usize),
    },
    /// Copy mode's block selection: rows `top..=bottom`, columns
    /// `left..=right` (the v Ctrl+V span's corners normalized).
    Block {
        pane: u32,
        top: usize,
        bottom: usize,
        left: usize,
        right: usize,
    },
}

impl Selection {
    /// Whether the selection covers cell `(line, col)` of `pane` — the
    /// paint path's highlight test.
    pub(crate) fn covers(&self, pane: u32, line: usize, col: usize) -> bool {
        match *self {
            Selection::Lines { pane: p, a, b } => p == pane && line >= a && line <= b,
            Selection::Chars {
                pane: p,
                anchor,
                head,
            } => {
                if p != pane {
                    return false;
                }
                let ((first_line, first_col), (last_line, last_col)) = normalize_span(anchor, head);
                if line < first_line || line > last_line {
                    return false;
                }
                match (line == first_line, line == last_line) {
                    (true, true) => col >= first_col && col <= last_col,
                    (true, false) => col >= first_col,
                    (false, true) => col <= last_col,
                    // A middle line is covered whole.
                    (false, false) => true,
                }
            }
            Selection::Block {
                pane: p,
                top,
                bottom,
                left,
                right,
            } => p == pane && line >= top && line <= bottom && col >= left && col <= right,
        }
    }
}

/// The span's cells in reading order: the earlier cell first.
pub(crate) fn normalize_span(
    anchor: (usize, usize),
    head: (usize, usize),
) -> ((usize, usize), (usize, usize)) {
    if (anchor.0, anchor.1) <= (head.0, head.1) {
        (anchor, head)
    } else {
        (head, anchor)
    }
}
