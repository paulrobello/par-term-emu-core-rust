//! Erase-related CSI sequence handling
//!
//! Capability boundary (ARC-002): the handlers take the active grid, the
//! erase background (the SGR pen's bg, for BCE), and — for ED 2/3 — the
//! graphics store and event broker, not `&mut Terminal`.

use crate::cell::PackedColor;
use crate::color::Color;
use crate::debug;
use crate::graphics::GraphicsStore;
use crate::grid::Grid;
use crate::terminal::{EventBroker, TerminalModes};
use vte::Params;

/// ED (`CSI J`), EL (`CSI K`), and ECH (`CSI X`); `cursor` is `(col, row)`.
pub(crate) fn handle_csi_erase(
    grid: &mut Grid,
    graphics_store: &mut GraphicsStore,
    events: &mut EventBroker,
    bg: Color,
    cursor: (usize, usize),
    action: char,
    params: &Params,
) {
    let (cursor_col, cursor_row) = cursor;
    match action {
        'J' => {
            // Erase in display (ED) — BCE: fill with current SGR background
            let n = params
                .iter()
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(0);
            match n {
                0 => {
                    grid.clear_screen_below(cursor_col, cursor_row, bg);
                }
                1 => {
                    grid.clear_screen_above(cursor_col, cursor_row, bg);
                }
                2 => {
                    grid.clear_with_bg(bg);
                    graphics_store.clear();
                    graphics_store.clear_scrollback_graphics();
                    events.push(crate::terminal::TerminalEvent::ScreenCleared {
                        include_scrollback: false,
                    });
                    debug::log(
                        debug::DebugLevel::Debug,
                        "CLEAR",
                        "Cleared screen and graphics (ED 2)",
                    );
                }
                3 => {
                    grid.clear_with_bg(bg);
                    grid.clear_scrollback();
                    graphics_store.clear();
                    graphics_store.clear_scrollback_graphics();
                    events.push(crate::terminal::TerminalEvent::ScreenCleared {
                        include_scrollback: true,
                    });
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
            // Erase in line (EL) — BCE: fill with current SGR background
            let n = params
                .iter()
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(0);
            match n {
                0 => {
                    grid.clear_line_right(cursor_col, cursor_row, bg);
                }
                1 => {
                    grid.clear_line_left(cursor_col, cursor_row, bg);
                }
                2 => {
                    grid.clear_row_with_bg(cursor_row, bg);
                }
                _ => {}
            }
        }
        'X' => {
            // Erase characters (ECH) — BCE: fill with current SGR background
            let n = params
                .iter()
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(1) as usize;
            let n = if n == 0 { 1 } else { n };
            grid.erase_characters(cursor_col, cursor_row, n, bg);
        }
        _ => {}
    }
}

/// DECSCA - Select Character Protection Attribute
/// CSI Ps " q
/// Ps = 0 or 2: disable protection, Ps = 1: enable protection
pub(crate) fn handle_decsca(modes: &mut TerminalModes, params: &Params) {
    let ps = params
        .iter()
        .next()
        .and_then(|p| p.first())
        .copied()
        .unwrap_or(0);
    match ps {
        1 => {
            modes.char_protected = true;
            debug::log(debug::DebugLevel::Debug, "DECSCA", "Protection enabled");
        }
        0 | 2 => {
            modes.char_protected = false;
            debug::log(debug::DebugLevel::Debug, "DECSCA", "Protection disabled");
        }
        _ => {}
    }
}

/// DECSERA - Selective Erase Rectangular Area
/// CSI Pt ; Pl ; Pb ; Pr $ {
/// Erases characters in the specified rectangle that are NOT protected (guarded)
pub(crate) fn handle_decsera(grid: &mut Grid, bg: Color, params: &Params) {
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
        .map(|v| if v == 0 { grid.rows() as u16 } else { v })
        .unwrap_or(grid.rows() as u16) as usize
        - 1;
    let right = params_vec
        .get(3)
        .copied()
        .map(|v| if v == 0 { grid.cols() as u16 } else { v })
        .unwrap_or(grid.cols() as u16) as usize
        - 1;

    let rows = grid.rows();
    let cols = grid.cols();
    let bottom = bottom.min(rows - 1);
    let right = right.min(cols - 1);

    // First pass: collect which cells to erase (unprotected only)
    let mut to_erase: Vec<(usize, usize)> = Vec::new();
    for row in top..=bottom {
        if let Some(cells) = grid.row(row) {
            for (col, cell) in cells.iter().enumerate().take(right + 1).skip(left) {
                if !cell.flags.guarded() {
                    to_erase.push((col, row));
                }
            }
        }
    }
    // Second pass: erase the collected cells with BCE
    for (col, row) in to_erase {
        if let Some(cells) = grid.row_mut(row) {
            cells[col].reset();
            cells[col].bg = PackedColor::pack(bg);
        }
    }

    crate::debug_log!(
        "DECSERA",
        "Selective erase rect ({},{}) to ({},{})",
        left,
        top,
        right,
        bottom
    );
}
