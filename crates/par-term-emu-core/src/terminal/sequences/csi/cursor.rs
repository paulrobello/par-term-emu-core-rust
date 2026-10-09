//! Cursor-related CSI sequence handling
//!
//! Capability boundary (ARC-002): cursor motion (CUU/CUD/CUF/CUB, CUP/HVP,
//! CNL/CPL, CHA/HPA, VPA, CBT) is a free function over the cursor, the
//! delayed-wrap flag, the margins, and the tab stops. The router stays on
//! `Terminal`: CHT writes tab characters through `write_char` (the full
//! print path), and the remaining arms (DECSCUSR + DECSWBV, SCOSC/SCORC +
//! DECSMBV, TBC) each touch one or two loose `Terminal` fields.

use super::count_param;
use crate::cursor::Cursor;
use crate::terminal::{MarginState, Terminal};
use vte::Params;

/// Cursor motion sequences. Returns `false` (and changes nothing) for an
/// action this function does not handle. `size` is the active screen's
/// `(cols, rows)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_cursor_motion(
    cursor: &mut Cursor,
    pending_wrap: &mut bool,
    origin_mode: bool,
    margins: &MarginState,
    tab_stops: &[bool],
    size: (usize, usize),
    action: char,
    params: &Params,
) -> bool {
    let (cols, rows) = size;
    match action {
        'A' => {
            // Cursor up (CUU)
            cursor.move_up(count_param(params));
        }
        'B' => {
            // Cursor down (CUD)
            cursor.move_down(count_param(params), rows.saturating_sub(1));
        }
        'C' => {
            // Cursor forward (CUF)
            cursor.move_right(count_param(params), cols.saturating_sub(1));
        }
        'D' => {
            // Cursor back (CUB)
            cursor.move_left(count_param(params));
        }
        'H' | 'f' => {
            // Cursor position (CUP/HVP)
            let mut iter = params.iter();
            let row = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let col = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;

            let col = col.saturating_sub(1);
            let row = row.saturating_sub(1);

            if origin_mode {
                let region_height = margins
                    .scroll_region_bottom
                    .saturating_sub(margins.scroll_region_top)
                    + 1;
                let actual_row =
                    margins.scroll_region_top + row.min(region_height.saturating_sub(1));
                let actual_col = col.min(cols.saturating_sub(1));
                cursor.goto(actual_col, actual_row);
            } else {
                cursor.goto(
                    col.min(cols.saturating_sub(1)),
                    row.min(rows.saturating_sub(1)),
                );
            }
        }
        'E' => {
            // Cursor next line (CNL)
            cursor.move_down(count_param(params), rows.saturating_sub(1));
            cursor.col = 0;
        }
        'F' => {
            // Cursor preceding line (CPL)
            cursor.move_up(count_param(params));
            cursor.col = 0;
        }
        'G' | '`' => {
            // Cursor horizontal absolute (CHA/HPA)
            let col = params
                .iter()
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(1) as usize;
            cursor.col = col.saturating_sub(1).min(cols.saturating_sub(1));
        }
        'd' => {
            // Line position absolute (VPA)
            let row = params
                .iter()
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(1) as usize;
            cursor.row = row.saturating_sub(1).min(rows.saturating_sub(1));
        }
        'Z' => {
            // Horizontal tab back (CBT)
            for _ in 0..count_param(params) {
                let mut col = cursor.col;
                if col > 0 {
                    col -= 1;
                    while col > 0 && !tab_stops[col] {
                        col -= 1;
                    }
                    cursor.col = col;
                }
            }
        }
        _ => return false,
    }
    *pending_wrap = false;
    true
}

impl Terminal {
    pub(crate) fn handle_csi_cursor(
        &mut self,
        action: char,
        params: &Params,
        _intermediates: &[u8],
    ) {
        let size = self.size();
        if handle_cursor_motion(
            &mut self.cursor,
            &mut self.pending_wrap,
            self.modes.origin_mode,
            &self.margins,
            &self.tab_stops,
            size,
            action,
            params,
        ) {
            return;
        }

        match action {
            'I' => {
                // Horizontal tab forward (CHT)
                for _ in 0..count_param(params) {
                    self.write_char('\t');
                }
            }
            'q'
                // DECSCUSR - Set Cursor Style OR DECSWBV - Set Warning Bell Volume
                if _intermediates.contains(&b' ') => {
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
