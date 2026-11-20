//! Pointer-shape routing (config `change-cursor-shape`): which OSC 22
//! pointer the next frame writes, derived from the drags in flight and
//! the hover position.

use super::super::pointer::{RESIZE_COL, RESIZE_ROW, TAB_POINTER};
use super::super::sidebar::SidebarDrag;
use super::super::{DividerHit, DragState, Modal, WindowSession};
use crate::mux::attach::input::SgrMouse;

impl WindowSession {
    /// The pointer-shape routing (config `change-cursor-shape`): a live
    /// divider drag, a live sidebar drag, or a hover report ON a resize
    /// handle or a clickable tab wants that shape — the OSC 22 the next
    /// frame writes; everything else (option off, a modal up, the
    /// pointer elsewhere) restores the terminal's own pointer. Runs
    /// after the hit dispatch: the shape is the pointer's state, and a
    /// drag-ending release must compute its want from the cleared drag.
    pub(super) fn update_pointer(&mut self, mouse: &SgrMouse) {
        if !self.cursor.change_cursor_shape || !matches!(self.modal, Modal::None) {
            self.pointer.want(None);
            return;
        }
        // A live drag keeps its shape wherever the pointer wanders: a
        // pane divider drag its axis, a sidebar width drag left/right,
        // a sidebar split drag up/down.
        let dragged = self
            .drag
            .as_ref()
            .map(drag_pointer)
            .or_else(|| self.sidebar_drag.as_ref().map(sidebar_drag_pointer));
        let want = match dragged {
            Some(shape) => Some(shape),
            None if mouse.is_motion() && !mouse.release && mouse.motion_button().is_none() => {
                let (x, y) = (mouse.col.saturating_sub(1), mouse.row.saturating_sub(1));
                self.hover_resize(x, y).or_else(|| self.hover_tab(x, y))
            }
            None => None,
        };
        self.pointer.want(want);
    }

    /// The resize handle under a hovering pointer (host 0-based
    /// coordinates): the side panel's edge column (width, left/right)
    /// and rule row (split, up/down) — the same predicates the press
    /// hit-test applies — then the pane dividers below the strip row in
    /// content coordinates.
    fn hover_resize(&self, x: u16, y: u16) -> Option<&'static str> {
        let strip = self.renderer.sidebar_width();
        if strip > 0 && y > 0 && x < strip {
            use crate::mux::attach::panels::SidebarRowId;
            // The chevron (the panel's last row, beside the divider) is
            // the collapse handle, not a drag handle; the collapsed rail
            // has no divider column.
            if !self.sidebar.is_collapsed()
                && x + 1 == strip
                && self.renderer.sidebar_row_at(x, y) != Some(SidebarRowId::Collapse)
            {
                return Some(RESIZE_COL);
            }
            if self.renderer.sidebar_rule_at(x, y) {
                return Some(RESIZE_ROW);
            }
            return None;
        }
        if y < self.geometry.strip_rows {
            return None;
        }
        let (x, y) = self.geometry.host_to_content_clamped(x, y);
        self.renderer
            .divider_near(x, y, 0)
            .filter(|_| !self.renderer.label_cell_at(x, y))
            .map(|divider| divider_pointer(&divider))
    }

    /// The clickable tab under a hovering pointer (host 0-based
    /// coordinates): a no-button hover ON a tab block of the strip row
    /// wants the pointing POINTER — the I-beam is the text-area default
    /// and wrong for a clickable tab. The bar's other columns (the lead
    /// segment, the gaps, the ` + ` button) stay unshaped; leaving the
    /// tab restores.
    fn hover_tab(&self, x: u16, y: u16) -> Option<&'static str> {
        if y >= self.geometry.strip_rows {
            return None;
        }
        self.chrome.tab_strip.tab_cell_at(x).map(|_| TAB_POINTER)
    }
}

/// The pointer shape for a drag in flight — its divider's axis.
fn drag_pointer(drag: &DragState) -> &'static str {
    match drag {
        DragState::Pending { divider, .. } | DragState::Active { divider, .. } => {
            divider_pointer(divider)
        }
    }
}

/// The pointer shape for a sidebar drag in flight: the width edge drags
/// left/right, the rule row up/down.
fn sidebar_drag_pointer(drag: &SidebarDrag) -> &'static str {
    match drag {
        SidebarDrag::Width { .. } => RESIZE_COL,
        SidebarDrag::Split { .. } => RESIZE_ROW,
    }
}

/// A vertical divider column resizes left/right; a horizontal row
/// up/down.
fn divider_pointer(divider: &DividerHit) -> &'static str {
    if divider.vertical {
        RESIZE_COL
    } else {
        RESIZE_ROW
    }
}
