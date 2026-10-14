//! The divider-drag state machine: motion promotes a pending press to an
//! active drag, and the pump applies the pointer's cell delta as relative
//! resize-pane commands at the frame cadence.

use super::{DividerHit, DragState, WindowSession};

/// The minimum gap between a drag's `resize-pane` steps: each step is a
/// daemon layout change that re-fits and repaints every pane, and
/// pointer motion arrives at tens of Hertz — the manual-pass trace shows
/// sixty whole-window repaints inside one held click. The unapplied
/// remainder is held until the interval passes and flushed on release,
/// so the drag still lands exactly where it is let go.
const DRAG_SEND_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);

/// The relative resize command for one drag step, aimed at the
/// boundary's left/top pane (the daemon re-divides the neighbor).
fn drag_resize_command(divider: &DividerHit, step: i32) -> String {
    let flag = match (divider.vertical, step > 0) {
        (true, true) => "-R",
        (true, false) => "-L",
        (false, true) => "-D",
        (false, false) => "-U",
    };
    format!(
        "resize-pane -t %{a} {flag} {n}",
        a = divider.a,
        n = step.abs()
    )
}

impl WindowSession {
    /// One motion or release while a drag is in flight: motion promotes a
    /// pending press to an active drag and records the pointer's signed
    /// delta from the press point (cells along the divider's axis);
    /// release ends the drag and clears the highlight — an ACTIVE drag
    /// flushes its throttled remainder first (the drag lands exactly
    /// where it is let go), and a release that never moved falls through
    /// as a focus-only click: the press was consumed as a divider grab,
    /// so forwarding the lone release would hand the app a phantom click
    /// at the release point (the manual-pass report — the htop click
    /// that "opens the menu again").
    pub(super) fn drag_event(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        x: u16,
        y: u16,
        release: bool,
    ) {
        if release {
            let state = self.drag.take();
            self.renderer.set_drag_divider(None);
            match state {
                Some(DragState::Pending { .. }) => {
                    if let Some(rect) = self.renderer.pane_at(x, y).cloned() {
                        let changed = self.renderer.focused() != Some(rect.pane);
                        self.renderer.focus(rect.pane);
                        if changed {
                            let _ = conn.send_forget(&format!("select-pane -t %{}", rect.pane));
                        }
                    }
                }
                Some(DragState::Active {
                    divider,
                    pending,
                    applied,
                    ..
                }) => {
                    let step = pending - applied;
                    if step != 0 {
                        let _ = conn.send_forget(&drag_resize_command(&divider, step));
                    }
                }
                None => {}
            }
            return;
        }
        // Motion.
        // Promote only a PENDING press: an already-active drag must stay
        // in place — `take()` here on an Active drag would drop the whole
        // drag state, starving every resize after the first motion (the
        // round-3 "highlight engages, drag does not resize" defect).
        if matches!(&self.drag, Some(DragState::Pending { .. })) {
            if let Some(DragState::Pending { divider, x, y }) = self.drag.take() {
                self.drag = Some(DragState::Active {
                    divider,
                    x,
                    y,
                    applied: 0,
                    pending: 0,
                    last_sent: None,
                });
                self.renderer
                    .set_drag_divider(Some((divider.vertical, divider.a, divider.b)));
            }
        }
        if let Some(DragState::Active {
            divider,
            x: px,
            y: py,
            pending,
            ..
        }) = &mut self.drag
        {
            *pending = if divider.vertical {
                i32::from(x) - i32::from(*px)
            } else {
                i32::from(y) - i32::from(*py)
            };
        }
    }

    /// The pump's frame-cadence drag application: one relative
    /// `resize-pane` for the unapplied cell delta, aimed at the
    /// boundary's left/top pane (the daemon re-divides the neighbor), at
    /// most once per [`DRAG_SEND_MIN_INTERVAL`]. Best-effort — the
    /// %layout-change broadcast re-seeds the window through the pump's
    /// parked-layout path (PendingWork), and that whole-window repaint
    /// is exactly what the interval keeps from tracking pointer motion.
    pub(super) fn apply_drag(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(DragState::Active {
            divider,
            pending,
            applied,
            last_sent,
            ..
        }) = &mut self.drag
        else {
            return;
        };
        let step = *pending - *applied;
        if step == 0 {
            return;
        }
        if last_sent.is_some_and(|sent| sent.elapsed() < DRAG_SEND_MIN_INTERVAL) {
            return;
        }
        let _ = conn.send_forget(&drag_resize_command(divider, step));
        *applied = *pending;
        *last_sent = Some(std::time::Instant::now());
    }
}
