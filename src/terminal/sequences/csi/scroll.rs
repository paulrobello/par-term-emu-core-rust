//! Scroll-related CSI sequence handling

use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    pub(crate) fn handle_csi_scroll(
        &mut self,
        action: char,
        params: &Params,
        _intermediates: &[u8],
    ) {
        match action {
            'S' => {
                // Scroll up (SU)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                let top = self.scroll_region_top;
                let bottom = self.scroll_region_bottom;
                self.active_grid_mut().scroll_region_up(n, top, bottom);
                self.adjust_graphics_for_scroll_up(n, top, bottom);
            }
            'T' => {
                // Scroll down (SD)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                let top = self.scroll_region_top;
                let bottom = self.scroll_region_bottom;
                self.active_grid_mut().scroll_region_down(n, top, bottom);
                self.adjust_graphics_for_scroll_down(n, top, bottom);
            }
            _ => {}
        }
    }
}
