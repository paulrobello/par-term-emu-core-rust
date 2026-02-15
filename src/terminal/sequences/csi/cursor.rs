//! Cursor-related CSI sequence handling

use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    pub(crate) fn handle_csi_cursor(
        &mut self,
        action: char,
        params: &Params,
        _intermediates: &[u8],
    ) {
        let (cols, rows) = self.size();

        match action {
            'A' => {
                // Cursor up (CUU)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_up(n);
                self.pending_wrap = false;
            }
            'B' => {
                // Cursor down (CUD)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_down(n, rows.saturating_sub(1));
                self.pending_wrap = false;
            }
            'C' => {
                // Cursor forward (CUF)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_right(n, cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            'D' => {
                // Cursor back (CUB)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_left(n);
                self.pending_wrap = false;
            }
            'H' | 'f' => {
                // Cursor position (CUP/HVP)
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;

                let col = col.saturating_sub(1);
                let row = row.saturating_sub(1);

                if self.origin_mode {
                    let region_height = self.scroll_region_bottom - self.scroll_region_top + 1;
                    let actual_row = self.scroll_region_top + row.min(region_height - 1);
                    let actual_col = col.min(cols.saturating_sub(1));
                    self.cursor.goto(actual_col, actual_row);
                } else {
                    self.cursor.goto(
                        col.min(cols.saturating_sub(1)),
                        row.min(rows.saturating_sub(1)),
                    );
                }
                self.pending_wrap = false;
            }
            'E' => {
                // Cursor next line (CNL)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_down(n, rows.saturating_sub(1));
                self.cursor.col = 0;
                self.pending_wrap = false;
            }
            'F' => {
                // Cursor preceding line (CPL)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_up(n);
                self.cursor.col = 0;
                self.pending_wrap = false;
            }
            'G' | '`' => {
                // Cursor horizontal absolute (CHA/HPA)
                let col = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                self.cursor.col = col.saturating_sub(1).min(cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            'd' => {
                // Line position absolute (VPA)
                let row = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                self.cursor.row = row.saturating_sub(1).min(rows.saturating_sub(1));
                self.pending_wrap = false;
            }
            'I' => {
                // Horizontal tab forward (CHT)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                for _ in 0..n {
                    self.write_char('\t');
                }
            }
            'Z' => {
                // Horizontal tab back (CBT)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                for _ in 0..n {
                    let mut col = self.cursor.col;
                    if col > 0 {
                        col -= 1;
                        while col > 0 && !self.tab_stops[col] {
                            col -= 1;
                        }
                        self.cursor.col = col;
                    }
                }
                self.pending_wrap = false;
            }
            'q' => {
                // DECSCUSR - Set Cursor Style OR DECSWBV - Set Warning Bell Volume
                if _intermediates.contains(&b' ') {
                    let mut iter = params.iter();
                    let n = iter.next().and_then(|p| p.first()).copied().unwrap_or(1);

                    // Handle DECSCUSR
                    use crate::cursor::CursorStyle;
                    self.cursor.style = match n {
                        0 | 1 => CursorStyle::BlinkingBlock,
                        2 => CursorStyle::SteadyBlock,
                        3 => CursorStyle::BlinkingUnderline,
                        4 => CursorStyle::SteadyUnderline,
                        5 => CursorStyle::BlinkingBar,
                        6 => CursorStyle::SteadyBar,
                        _ => CursorStyle::BlinkingBlock,
                    };

                    // Handle DECSWBV (VT520)
                    self.warning_bell_volume = n.min(8) as u8;
                }
            }
            's' => {
                // SCOSC - Save Cursor
                self.save_cursor();
            }
            'u' => {
                let mut iter = params.iter();
                let ps = iter.next().and_then(|p| p.first()).copied();
                // Treat None AND Some(0) as SCORC (Restore Cursor)
                // This prioritizes ANSI/SCO restore over DECSMBV volume 0
                if let Some(val) = ps {
                    if val == 0 {
                        self.restore_cursor();
                        self.margin_bell_volume = 0;
                    } else {
                        // DECSMBV - Set Margin Bell Volume: CSI Ps u
                        self.margin_bell_volume = val.min(8) as u8;
                    }
                } else {
                    // SCORC - Restore Cursor
                    self.restore_cursor();
                }
            }
            'g' => {
                // TBC - Tabulation Clear
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                match n {
                    0 => self.tab_stops[self.cursor.col] = false,
                    3 => self.tab_stops.fill(false),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
