//! The divider-drag state machine: motion promotes a pending press to an
//! active drag, and the pump applies the pointer's cell delta as relative
//! resize-pane commands at the frame cadence.

use super::{DragState, WindowSession};
use crate::mux::attach::input::SgrMouse;

impl WindowSession {
    /// One motion or release while a drag is in flight: motion promotes a
    /// pending press to an active drag and records the pointer's signed
    /// delta from the press point (cells along the divider's axis);
    /// release ends the drag and clears the highlight — a release that
    /// never moved falls through as a click (focus the pane under the
    /// pointer, forward the release when the pane owns the mouse).
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
            if let Some(DragState::Pending { .. }) = state {
                if let Some(rect) = self.renderer.pane_at(x, y).cloned() {
                    let changed = self.renderer.focused() != Some(rect.pane);
                    self.renderer.focus(rect.pane);
                    if changed {
                        let _ = conn.send_forget(&format!("select-pane -t %{}", rect.pane));
                    }
                    let mouse = SgrMouse {
                        cb: 0,
                        col: x + 1,
                        row: y + 1,
                        release: true,
                    };
                    if self.pane_accepts_mouse(rect.pane, &mouse) {
                        self.forward_mouse(conn, &rect, &mouse);
                    }
                }
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
    /// `resize-pane` per unapplied cell of delta, aimed at the boundary's
    /// left/top pane (the daemon re-divides the neighbor). Best-effort —
    /// the %layout-change broadcast re-seeds the window through the
    /// pump's parked-layout path (PendingWork).
    pub(super) fn apply_drag(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Some(DragState::Active {
            divider,
            pending,
            applied,
            ..
        }) = &mut self.drag
        else {
            return;
        };
        let step = *pending - *applied;
        if step == 0 {
            return;
        }
        let flag = if divider.vertical {
            if step > 0 {
                "-R"
            } else {
                "-L"
            }
        } else if step > 0 {
            "-D"
        } else {
            "-U"
        };
        let command = format!(
            "resize-pane -t %{a} {flag} {n}",
            a = divider.a,
            n = step.abs()
        );
        *applied = *pending;
        let _ = conn.send_forget(&command);
    }
}
