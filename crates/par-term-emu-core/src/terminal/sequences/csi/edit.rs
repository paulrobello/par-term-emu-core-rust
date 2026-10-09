//! Edit-related CSI sequence handling (insertion/deletion)
//!
//! Capability boundary (ARC-002): ICH/DCH are free functions over the
//! active grid. IL/DL stay on `Terminal` because they go through
//! `insert_lines_tracked`/`delete_lines_tracked`, which run the trigger
//! engine (`TriggerEngine::scan_rows` takes `&mut Terminal`) on departing
//! rows before the grid moves them.

use super::count_param;
use crate::grid::Grid;
use crate::terminal::Terminal;
use vte::Params;

/// ICH (`CSI @`): insert `n` blank characters at the cursor.
pub(crate) fn handle_ich(grid: &mut Grid, cursor_col: usize, cursor_row: usize, params: &Params) {
    grid.insert_characters(cursor_col, cursor_row, count_param(params));
}

/// DCH (`CSI P`): delete `n` characters at the cursor.
pub(crate) fn handle_dch(grid: &mut Grid, cursor_col: usize, cursor_row: usize, params: &Params) {
    grid.delete_characters(cursor_col, cursor_row, count_param(params));
}

impl Terminal {
    pub(crate) fn handle_csi_edit(&mut self, action: char, params: &Params, _intermediates: &[u8]) {
        let cursor_row = self.cursor.row;
        let scroll_top = self.margins.scroll_region_top;
        let scroll_bottom = self.margins.scroll_region_bottom;

        match action {
            'L' => {
                // Insert line (IL)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                // Insert lines within current scroll region if cursor is inside it
                if cursor_row >= scroll_top && cursor_row <= scroll_bottom {
                    self.insert_lines_tracked(n, cursor_row, scroll_bottom);
                }
            }
            'M' => {
                // Delete line (DL)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                // Delete lines within current scroll region if cursor is inside it
                if cursor_row >= scroll_top && cursor_row <= scroll_bottom {
                    self.delete_lines_tracked(n, cursor_row, scroll_bottom);
                }
            }
            '@' => {
                // Insert characters (ICH)
                let cursor_col = self.cursor.col;
                handle_ich(self.active_grid_mut(), cursor_col, cursor_row, params);
            }
            'P' => {
                // Delete characters (DCH)
                let cursor_col = self.cursor.col;
                handle_dch(self.active_grid_mut(), cursor_col, cursor_row, params);
            }
            _ => {}
        }
    }
}
