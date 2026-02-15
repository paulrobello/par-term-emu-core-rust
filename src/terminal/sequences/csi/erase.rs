//! Erase-related CSI sequence handling

use crate::debug;
use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    pub(crate) fn handle_csi_erase(
        &mut self,
        action: char,
        params: &Params,
        _intermediates: &[u8],
    ) {
        match action {
            'J' => {
                // Erase in display (ED)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                match n {
                    0 => self
                        .active_grid_mut()
                        .clear_screen_below(cursor_col, cursor_row),
                    1 => self
                        .active_grid_mut()
                        .clear_screen_above(cursor_col, cursor_row),
                    2 => {
                        self.active_grid_mut().clear();
                        self.graphics_store.clear();
                        debug::log(
                            debug::DebugLevel::Debug,
                            "CLEAR",
                            "Cleared screen and graphics (ED 2)",
                        );
                    }
                    3 => {
                        self.active_grid_mut().clear();
                        self.active_grid_mut().clear_scrollback();
                        self.graphics_store.clear();
                        debug::log(
                            debug::DebugLevel::Debug,
                            "CLEAR",
                            "Cleared screen, scrollback, and graphics (ED 3)",
                        );
                    }
                    _ => {}
                }
            }
            'K' => {
                // Erase in line (EL)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                match n {
                    0 => self
                        .active_grid_mut()
                        .clear_line_right(cursor_col, cursor_row),
                    1 => self
                        .active_grid_mut()
                        .clear_line_left(cursor_col, cursor_row),
                    2 => self.active_grid_mut().clear_row(cursor_row),
                    _ => {}
                }
            }
            'X' => {
                // Erase characters (ECH)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                self.active_grid_mut()
                    .erase_characters(cursor_col, cursor_row, n);
            }
            _ => {}
        }
    }

    /// DECSCA - Select Character Protection Attribute
    /// CSI Ps " q
    /// Ps = 0 or 2: disable protection, Ps = 1: enable protection
    pub(crate) fn handle_decsca(&mut self, params: &Params) {
        let ps = params
            .iter()
            .next()
            .and_then(|p| p.first())
            .copied()
            .unwrap_or(0);
        match ps {
            1 => {
                self.char_protected = true;
                debug::log(debug::DebugLevel::Debug, "DECSCA", "Protection enabled");
            }
            0 | 2 => {
                self.char_protected = false;
                debug::log(debug::DebugLevel::Debug, "DECSCA", "Protection disabled");
            }
            _ => {}
        }
    }

    /// DECSERA - Selective Erase Rectangular Area
    /// CSI Pt ; Pl ; Pb ; Pr $ {
    /// Erases characters in the specified rectangle that are NOT protected (guarded)
    pub(crate) fn handle_decsera(&mut self, params: &Params) {
        let params_vec: Vec<u16> = params
            .iter()
            .flat_map(|subparams| subparams.iter().copied())
            .collect();

        // Parameters: top, left, bottom, right (1-indexed, default to full screen)
        let top = params_vec.first().copied().unwrap_or(1).max(1) as usize - 1;
        let left = params_vec.get(1).copied().unwrap_or(1).max(1) as usize - 1;
        let bottom = params_vec
            .get(2)
            .copied()
            .map(|v| {
                if v == 0 {
                    self.active_grid().rows() as u16
                } else {
                    v
                }
            })
            .unwrap_or(self.active_grid().rows() as u16) as usize
            - 1;
        let right = params_vec
            .get(3)
            .copied()
            .map(|v| {
                if v == 0 {
                    self.active_grid().cols() as u16
                } else {
                    v
                }
            })
            .unwrap_or(self.active_grid().cols() as u16) as usize
            - 1;

        let rows = self.active_grid().rows();
        let cols = self.active_grid().cols();
        let bottom = bottom.min(rows - 1);
        let right = right.min(cols - 1);

        // First pass: collect which cells to erase (unprotected only)
        let mut to_erase: Vec<(usize, usize)> = Vec::new();
        for row in top..=bottom {
            if let Some(cells) = self.active_grid().row(row) {
                for (col, cell) in cells.iter().enumerate().take(right + 1).skip(left) {
                    if !cell.flags.guarded() {
                        to_erase.push((col, row));
                    }
                }
            }
        }
        // Second pass: erase the collected cells
        for (col, row) in to_erase {
            if let Some(cells) = self.active_grid_mut().row_mut(row) {
                cells[col] = crate::cell::Cell::default();
            }
        }

        debug::log(
            debug::DebugLevel::Debug,
            "DECSERA",
            &format!(
                "Selective erase rect ({},{}) to ({},{})",
                left, top, right, bottom
            ),
        );
    }
}
