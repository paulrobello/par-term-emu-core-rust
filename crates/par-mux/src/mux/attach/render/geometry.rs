//! The render client's frame geometry: the one owner of how the host
//! grid divides into the tab strip (top), the status bar (bottom), the
//! side panel (left), and the pane content area between them.
//!
//! Three coordinate spaces meet here, all 0-based:
//! - **host**: the terminal's grid, as `terminal_grid()` reports it and
//!   as SGR mouse reports address it (less their 1-based origin);
//! - **frame**: the renderer's buffer — the host rows between the strip
//!   and the status bar, full host width (the side panel paints inside
//!   it);
//! - **content**: the daemon's window/layout coordinates — the frame
//!   less the side panel's columns.

use ratatui::layout::Rect;

/// Rows the tab strip occupies at the top of the host grid.
pub(crate) const STRIP_ROWS: u16 = 1;

/// One frame's chrome layout over a host grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameGeometry {
    /// Host grid width.
    pub host_cols: u16,
    /// Host grid height.
    pub host_rows: u16,
    /// Rows the tab strip takes at the top.
    pub strip_rows: u16,
    /// Rows the status bar takes at the bottom (0 while hidden).
    pub status_rows: u16,
    /// Columns the side panel takes at the left (the EFFECTIVE width:
    /// 0 while the panel is hidden).
    pub sidebar_w: u16,
}

impl FrameGeometry {
    /// The geometry of a `host_cols` x `host_rows` grid with the strip,
    /// the status bar when `status_bar_on`, and a `sidebar_w`-column side
    /// panel.
    pub fn new(host_cols: u16, host_rows: u16, status_bar_on: bool, sidebar_w: u16) -> Self {
        Self {
            host_cols,
            host_rows,
            strip_rows: STRIP_ROWS,
            status_rows: u16::from(status_bar_on),
            sidebar_w,
        }
    }

    /// The renderer's frame extent: full host width, the rows between
    /// the strip and the status bar.
    pub fn frame_size(&self) -> (u16, u16) {
        (
            self.host_cols,
            self.host_rows
                .saturating_sub(self.strip_rows)
                .saturating_sub(self.status_rows),
        )
    }

    /// The pane content area in HOST coordinates: the host less the
    /// strip (top), the status bar (bottom), and the side panel (left).
    /// Zero-sized when the chrome eats the whole host.
    pub fn content_rect(&self) -> Rect {
        let (frame_w, frame_h) = self.frame_size();
        let x = self.sidebar_w.min(frame_w);
        Rect::new(x, self.strip_rows.min(self.host_rows), frame_w - x, frame_h)
    }

    /// The host row the status bar paints on, `None` while hidden or
    /// when the host has no rows.
    pub fn status_row(&self) -> Option<u16> {
        (self.status_rows > 0 && self.host_rows > 0).then(|| self.host_rows - 1)
    }

    /// A frame (renderer buffer) cell rebased to the host grid: frame
    /// rows sit below the strip.
    pub fn frame_to_host(&self, x: u16, y: u16) -> (u16, u16) {
        (x, y + self.strip_rows)
    }

    #[cfg(test)]
    /// A 0-based host cell mapped into content coordinates, `None` when
    /// it falls on the strip, the status bar, the side panel, or outside
    /// the host.
    pub fn host_to_content(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        let rect = self.content_rect();
        let inside = col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height;
        inside.then(|| (col - rect.x, row - rect.y))
    }

    /// A 0-based host cell mapped into content coordinates, saturating
    /// at the content origin (a point on the strip or the side panel
    /// clamps to content row/column 0) — the mapping a drag or a
    /// forwarded pane click uses once the hit-test already chose the
    /// pane.
    pub fn host_to_content_clamped(&self, col: u16, row: u16) -> (u16, u16) {
        let rect = self.content_rect();
        (col.saturating_sub(rect.x), row.saturating_sub(rect.y))
    }

    /// A content cell mapped back to the 0-based host grid.
    pub fn content_to_host(&self, col: u16, row: u16) -> (u16, u16) {
        let rect = self.content_rect();
        (col + rect.x, row + rect.y)
    }

    /// A content cell mapped into the renderer's frame buffer: right of
    /// the side panel, rows unchanged (the frame already starts below the
    /// strip).
    pub fn content_to_frame(&self, col: u16, row: u16) -> (u16, u16) {
        let (x, y) = self.content_to_host(col, row);
        (x, y - self.strip_rows.min(y))
    }

    /// A 0-based host cell mapped into the frame buffer — the inverse of
    /// [`Self::frame_to_host`] — `None` on the strip rows.
    pub fn host_to_frame(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        row.checked_sub(self.strip_rows).map(|y| (col, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_on_sidebar_off_carves_strip_and_status() {
        let g = FrameGeometry::new(80, 24, true, 0);
        assert_eq!(g.frame_size(), (80, 22));
        assert_eq!(g.content_rect(), Rect::new(0, 1, 80, 22));
        assert_eq!(g.status_row(), Some(23));
        assert_eq!(g.frame_to_host(5, 0), (5, 1));
        assert_eq!(g.host_to_content(0, 0), None, "the strip row");
        assert_eq!(g.host_to_content(0, 1), Some((0, 0)));
        assert_eq!(g.host_to_content(79, 22), Some((79, 21)));
        assert_eq!(g.host_to_content(0, 23), None, "the status row");
        assert_eq!(g.host_to_content(80, 5), None, "past the right edge");
    }

    #[test]
    fn status_off_hands_the_bottom_row_to_the_content() {
        let g = FrameGeometry::new(80, 24, false, 0);
        assert_eq!(g.frame_size(), (80, 23));
        assert_eq!(g.content_rect(), Rect::new(0, 1, 80, 23));
        assert_eq!(g.status_row(), None);
        assert_eq!(g.host_to_content(3, 23), Some((3, 22)));
    }

    #[test]
    fn sidebar_on_shifts_content_right() {
        let g = FrameGeometry::new(80, 24, true, 20);
        assert_eq!(g.frame_size(), (80, 22), "the frame keeps the full width");
        assert_eq!(g.content_rect(), Rect::new(20, 1, 60, 22));
        assert_eq!(g.host_to_content(19, 5), None, "the side panel");
        assert_eq!(g.host_to_content(20, 5), Some((0, 4)));
        assert_eq!(g.content_to_host(0, 4), (20, 5));
        assert_eq!(g.host_to_content_clamped(3, 0), (0, 0));
        assert_eq!(g.host_to_content_clamped(25, 3), (5, 2));
    }

    #[test]
    fn content_round_trips_through_the_host() {
        for (status, sidebar) in [(true, 0), (false, 0), (true, 20), (false, 20)] {
            let g = FrameGeometry::new(100, 40, status, sidebar);
            let rect = g.content_rect();
            for (cx, cy) in [(0, 0), (rect.width - 1, rect.height - 1), (7, 3)] {
                let (hx, hy) = g.content_to_host(cx, cy);
                assert_eq!(g.host_to_content(hx, hy), Some((cx, cy)));
            }
        }
    }

    #[test]
    fn frame_mappings_offset_by_the_panel_and_the_strip() {
        let g = FrameGeometry::new(80, 24, true, 0);
        assert_eq!(g.content_to_frame(5, 3), (5, 3));
        assert_eq!(g.host_to_frame(5, 0), None, "the strip row");
        assert_eq!(g.host_to_frame(5, 1), Some((5, 0)));
        let g = FrameGeometry::new(80, 24, true, 20);
        assert_eq!(g.content_to_frame(5, 3), (25, 3));
        assert_eq!(g.host_to_frame(25, 4), Some((25, 3)));
        for (cx, cy) in [(0, 0), (7, 3), (59, 21)] {
            let (fx, fy) = g.content_to_frame(cx, cy);
            assert_eq!(g.frame_to_host(fx, fy), g.content_to_host(cx, cy));
            assert_eq!(g.host_to_frame(fx, fy + 1), Some((fx, fy)));
        }
    }

    #[test]
    fn zero_and_tiny_hosts_never_underflow() {
        let g = FrameGeometry::new(0, 0, true, 20);
        assert_eq!(g.frame_size(), (0, 0));
        assert_eq!(g.content_rect(), Rect::new(0, 0, 0, 0));
        assert_eq!(g.status_row(), None);
        assert_eq!(g.host_to_content(0, 0), None);
        let g = FrameGeometry::new(10, 1, true, 20);
        assert_eq!(g.frame_size(), (10, 0));
        assert_eq!(g.content_rect().width, 0, "the panel eats a narrow host");
        assert_eq!(g.host_to_content(5, 0), None);
    }
}
