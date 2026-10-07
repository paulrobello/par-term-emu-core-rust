//! Window-related CSI sequence handling (XTWINOPS, etc.)
//!
//! Capability boundary (ARC-002): the rectangle operations (`CSI … $ x/v/z/r/t`),
//! XTWINOPS/DECSWBV (`CSI … t`), and the margin setters (DECSTBM/DECSLRM)
//! are free functions over the grid, pen, host config, title state, and
//! margins they touch. The router stays a `Terminal` method so the arm
//! order (the `$` forms first, then the bare forms) is unchanged.

use crate::cursor::Cursor;
use crate::grid::Grid;
use crate::terminal::{
    AttributeChangeExtent, HostConfig, MarginState, Terminal, TerminalModes, TextAttributes,
    TitleState,
};
use vte::Params;

impl Terminal {
    pub(crate) fn handle_csi_window(
        &mut self,
        action: char,
        params: &Params,
        intermediates: &[u8],
    ) {
        let (cols, rows) = self.size();

        if intermediates.contains(&b'$') {
            let extent = self.modes.attribute_change_extent;
            let grid = if self.alt_screen_active {
                &mut self.alt_grid
            } else {
                &mut self.grid
            };
            handle_rect_ops(grid, &self.attrs, extent, (cols, rows), action, params);
            return;
        }

        match action {
            't' => handle_xtwinops(
                &mut self.response_buffer,
                &self.host,
                &mut self.title_state,
                &mut self.warning_bell_volume,
                (self.pixel_width, self.pixel_height),
                (cols, rows),
                params,
                intermediates,
            ),
            'r' | 's' => handle_margins(
                &mut self.margins,
                &mut self.cursor,
                self.modes.origin_mode,
                (cols, rows),
                action,
                params,
            ),
            _ => {}
        }
    }
}

/// DEC rectangular-area operations (`CSI … $ x/v/z/r/t`): DECFRA, DECCRA,
/// DECERA, DECCARA, DECRARA. `size` is the active screen's `(cols, rows)`.
fn handle_rect_ops(
    grid: &mut Grid,
    attrs: &TextAttributes,
    extent: AttributeChangeExtent,
    size: (usize, usize),
    action: char,
    params: &Params,
) {
    let (cols, rows) = size;
    match action {
        'x' => {
            // DECFRA - Fill Rectangular Area: CSI Pc ; Pt ; Pl ; Pb ; Pr $ x
            let mut iter = params.iter();
            let pc = iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as u8 as char;
            let pt = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pl = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pb = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(rows as u16) as usize;
            let pr = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(cols as u16) as usize;

            let top = pt.saturating_sub(1);
            let left = pl.saturating_sub(1);
            let bottom = pb.saturating_sub(1);
            let right = pr.saturating_sub(1);

            let mut fill_cell = crate::cell::Cell::with_colors(pc, attrs.fg, attrs.bg);
            fill_cell.flags = attrs.flags;

            grid.fill_rectangle(fill_cell, top, left, bottom, right);
        }
        'v' => {
            // DECCRA - Copy Rectangular Area: CSI Pt ; Pl ; Pb ; Pr ; Pp ; Dt ; Dl ; Dp $ v
            let mut iter = params.iter();
            let pt = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pl = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pb = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(rows as u16) as usize;
            let pr = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(cols as u16) as usize;
            let _pp = iter.next(); // Source page
            let dt = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let dl = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;

            let src_top = pt.saturating_sub(1);
            let src_left = pl.saturating_sub(1);
            let src_bottom = pb.saturating_sub(1);
            let src_right = pr.saturating_sub(1);
            let dst_top = dt.saturating_sub(1);
            let dst_left = dl.saturating_sub(1);

            grid.copy_rectangle(src_top, src_left, src_bottom, src_right, dst_top, dst_left);
            // The destination rows change; the untouched source
            // rows must not be marked (false dirty).
        }
        'z' => {
            // DECERA - Erase Rectangular Area: CSI Pt ; Pl ; Pb ; Pr $ z
            let mut iter = params.iter();
            let pt = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pl = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pb = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(rows as u16) as usize;
            let pr = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(cols as u16) as usize;

            let top = pt.saturating_sub(1);
            let left = pl.saturating_sub(1);
            let bottom = pb.saturating_sub(1);
            let right = pr.saturating_sub(1);

            grid.erase_rectangle_unconditional(top, left, bottom, right);
        }
        'r' | 't' => {
            // DECCARA - Change Attributes in Rectangular Area: CSI Pt ; Pl ; Pb ; Pr ; Ps1 ; Ps2 ... $ r
            // DECRARA - Reverse Attributes in Rectangular Area: CSI Pt ; Pl ; Pb ; Pr ; Ps1 ; Ps2 ... $ t
            let mut iter = params.iter();
            let pt = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pl = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let pb = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(rows as u16) as usize;
            let pr = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(cols as u16) as usize;

            let top = pt.saturating_sub(1);
            let left = pl.saturating_sub(1);
            let bottom = pb.saturating_sub(1);
            let right = pr.saturating_sub(1);

            let mut attributes = Vec::new();
            for param_slice in iter {
                if let Some(&p) = param_slice.first() {
                    attributes.push(p);
                }
            }

            // DECSACE: stream extent covers everything in reading
            // order from (top,left) to (bottom,right); rectangle
            // extent is the addressed area only.
            let segments: Vec<(usize, usize, usize, usize)> = match extent {
                AttributeChangeExtent::Rectangle => {
                    vec![(top, left, bottom, right)]
                }
                AttributeChangeExtent::Stream if top == bottom => {
                    vec![(top, left, bottom, right)]
                }
                AttributeChangeExtent::Stream => {
                    let mut segs = vec![(top, left, top, cols - 1)];
                    if bottom > top + 1 {
                        segs.push((top + 1, 0, bottom - 1, cols - 1));
                    }
                    segs.push((bottom, 0, bottom, right));
                    segs
                }
            };

            for (t, l, b, r) in segments {
                if action == 'r' {
                    grid.change_attributes_in_rectangle(t, l, b, r, &attributes);
                } else {
                    grid.reverse_attributes_in_rectangle(t, l, b, r, &attributes);
                }
            }
            // Every segment falls inside top..=bottom (stream
            // extent included), so one range covers them all.
        }
        _ => {}
    }
}

/// XTWINOPS window reports/title stack (`CSI Ps t`) and DECSWBV.
/// `pixel_size` is `(width, height)`; `size` is `(cols, rows)`.
#[allow(clippy::too_many_arguments)]
fn handle_xtwinops(
    response: &mut Vec<u8>,
    host: &HostConfig,
    title_state: &mut TitleState,
    warning_bell_volume: &mut u8,
    pixel_size: (usize, usize),
    size: (usize, usize),
    params: &Params,
    intermediates: &[u8],
) {
    let (pixel_width, pixel_height) = pixel_size;
    let (cols, rows) = size;
    // Window manipulation (XTWINOPS) or DECSWBV (Set Warning Bell Volume)
    let mut iter = params.iter();
    let n = iter.next().and_then(|p| p.first()).copied().unwrap_or(0);

    // DECSWBV - Set Warning Bell Volume: CSI Ps t or CSI Ps SP t
    if params.iter().count() == 1 && (n <= 8 || intermediates.contains(&b' ')) {
        *warning_bell_volume = n.min(8) as u8;
        // If it was just a bell volume sequence, we can return early
        // unless it's a value that overlaps with XTWINOPS (unlikely for n > 8)
        if n > 8 {
            return;
        }
    }

    match n {
        1 | 2 | 3 | 4 | 5 | 6 | 9 | 10 => {
            // Window manipulation: deiconify(1)/iconify(2)/move(3)/
            // resize-pixels(4)/raise(5)/lower(6)/maximize-restore(9)/
            // fullscreen(10). No-op for a headless terminal core (no
            // window to act on).
        }
        11 => {
            // Report window state: iconified/non-iconified. The
            // core is headless, so this reflects whatever the
            // host last supplied via `Terminal::set_window_iconified`
            // (defaults to non-iconified when never set).
            if host.window_iconified {
                response.extend_from_slice(b"\x1b[2t");
            } else {
                response.extend_from_slice(b"\x1b[1t");
            }
        }
        13 => {
            // Report window position in pixels. Also covers the
            // text-area-position sub-form `CSI 13 ; 2 t` (n is still
            // 13, the second param is ignored). The core is
            // headless, so this reflects whatever the host last
            // supplied via `Terminal::set_window_position`
            // (defaults to the origin when never set). CSI
            // parameters are unsigned, so a negative host-supplied
            // coordinate (possible on multi-monitor setups where
            // the window sits left of/above the primary display)
            // is clamped to 0 for the reply -- xterm's own reply
            // grammar has no way to encode a negative parameter
            // either.
            let x = host.window_position.0.max(0);
            let y = host.window_position.1.max(0);
            let reply = format!("\x1b[3;{};{}t", x, y);
            response.extend_from_slice(reply.as_bytes());
        }
        14 => {
            // Report text area size in pixels. Also covers the
            // window-size-vs-text-area sub-form `CSI 14 ; 2 t` (n is
            // still 14); no separate window frame exists here, so the
            // reply is the same for both.
            let reply = format!("\x1b[4;{};{}t", pixel_height, pixel_width);
            response.extend_from_slice(reply.as_bytes());
        }
        16 => {
            // Report character cell size in pixels.
            // Derive from text-area pixel size / grid size so the
            // value matches what the renderer actually uses (set
            // via `Terminal::set_pixel_size`, which the host
            // updates from `cell_renderer.cell_width/_height` on
            // every resize). Falls back to a 10x20 default if the
            // pixel/grid dimensions have not been set yet.
            let cpw = if cols > 0 && pixel_width > 0 {
                (pixel_width / cols).max(1)
            } else {
                10
            };
            let cph = if rows > 0 && pixel_height > 0 {
                (pixel_height / rows).max(1)
            } else {
                20
            };
            let reply = format!("\x1b[6;{};{}t", cph, cpw);
            response.extend_from_slice(reply.as_bytes());
        }
        18 => {
            // Report text area size in characters
            let reply = format!("\x1b[8;{};{}t", rows, cols);
            response.extend_from_slice(reply.as_bytes());
        }
        19 => {
            // Report screen size in characters. No distinct "root
            // window" exists in a library core, so report the
            // terminal's own size.
            let reply = format!("\x1b[9;{};{}t", rows, cols);
            response.extend_from_slice(reply.as_bytes());
        }
        22 => {
            // Push icon name and window title to stack
            title_state.title_stack.push(title_state.title.clone());
        }
        23 => {
            // Pop icon name and window title from stack
            if let Some(title) = title_state.title_stack.pop() {
                title_state.title = title;
            }
        }
        0..=8 => {
            // Remaining values (0, 7, 8) already handled above, but
            // kept for match exhaustiveness/structure
        }
        _ => {
            // Ps >= 24: "resize to Ps lines" (DECSLPP) - no-op; a
            // library core does not self-resize.
        }
    }
}

/// DECSTBM (`CSI Pt ; Pb r`) and DECSLRM (`CSI Pl ; Pr s`, only while
/// DECLRMM is set). `size` is `(cols, rows)`.
fn handle_margins(
    margins: &mut MarginState,
    cursor: &mut Cursor,
    origin_mode: bool,
    size: (usize, usize),
    action: char,
    params: &Params,
) {
    let (cols, rows) = size;
    match action {
    'r' => {
        // Set scrolling region (DECSTBM)
        let mut iter = params.iter();
        let top = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
        let bottom = iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as usize;

        let top = if top == 0 { 1 } else { top };
        let bottom = if bottom == 0 { rows } else { bottom };

        let top = top.saturating_sub(1);
        let bottom = bottom.saturating_sub(1).min(rows.saturating_sub(1));

        if top < bottom {
            margins.scroll_region_top = top;
            margins.scroll_region_bottom = bottom;
            // Reset cursor to (0,0) relative to region if origin mode
            cursor.goto(0, if origin_mode { top } else { 0 });
        }
    }
    's'
        // Set left and right margins (DECSLRM) - only if DECLRMM is set
        if margins.use_lr_margins => {
            let mut iter = params.iter();
            let left = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            let right = iter
                .next()
                .and_then(|p| p.first())
                .copied()
                .unwrap_or(cols as u16) as usize;

            let left = left.saturating_sub(1);
            let right = right.saturating_sub(1).min(cols.saturating_sub(1));

            if left < right {
                margins.left_margin = left;
                margins.right_margin = right;
            }
        }
        _ => {}
    }
}

/// DECSACE - Select Attribute Change Extent: CSI Ps * x
///
/// Ps = 0 or 1 selects stream extent, 2 selects rectangle; any other
/// value is ignored. Missing parameter defaults to 1.
pub(crate) fn handle_decsace(modes: &mut TerminalModes, params: &Params) {
    let ps = params
        .iter()
        .next()
        .and_then(|p| p.first())
        .copied()
        .unwrap_or(1);

    modes.attribute_change_extent = match ps {
        0 | 1 => AttributeChangeExtent::Stream,
        2 => AttributeChangeExtent::Rectangle,
        _ => return,
    };
}

#[cfg(test)]
mod tests {
    use crate::terminal::Terminal;

    // ========== XTWINOPS Report Tests (window.rs-local) ==========

    #[test]
    fn test_xtwinops_report_window_state() {
        let mut term = Terminal::new(80, 24);

        // Report window state (CSI 11 t) -> non-iconified
        term.process(b"\x1b[11t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[1t");
    }

    #[test]
    fn test_xtwinops_report_window_position() {
        let mut term = Terminal::new(80, 24);

        // Report window position in pixels (CSI 13 t)
        term.process(b"\x1b[13t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[3;0;0t");
    }

    #[test]
    fn test_xtwinops_report_screen_size_chars() {
        let mut term = Terminal::new(80, 24);

        // Report screen size in characters (CSI 19 t)
        term.process(b"\x1b[19t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[9;24;80t");
    }

    #[test]
    fn test_xtwinops_report_text_area_size_chars_still_works() {
        let mut term = Terminal::new(80, 24);

        // Report text area size in characters (CSI 18 t) - pre-existing behavior
        term.process(b"\x1b[18t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[8;24;80t");
    }

    #[test]
    fn test_xtwinops_report_window_state_iconified() {
        let mut term = Terminal::new(80, 24);

        term.set_window_iconified(true);
        term.process(b"\x1b[11t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[2t");
    }

    #[test]
    fn test_xtwinops_report_window_state_toggle_back_to_non_iconified() {
        let mut term = Terminal::new(80, 24);

        term.set_window_iconified(true);
        term.set_window_iconified(false);
        term.process(b"\x1b[11t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[1t");
    }

    #[test]
    fn test_xtwinops_report_window_position_host_supplied() {
        let mut term = Terminal::new(80, 24);

        term.set_window_position(100, 50);
        term.process(b"\x1b[13t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[3;100;50t");
    }

    #[test]
    fn test_xtwinops_report_window_position_text_area_subform_uses_host_value() {
        let mut term = Terminal::new(80, 24);

        term.set_window_position(200, 75);
        // CSI 13 ; 2 t - text-area-position sub-form; the second param is
        // ignored and the reply is identical to the plain CSI 13 t form.
        term.process(b"\x1b[13;2t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[3;200;75t");
    }

    #[test]
    fn test_xtwinops_report_window_position_negative_clamped_to_zero() {
        let mut term = Terminal::new(80, 24);

        term.set_window_position(-10, -20);
        term.process(b"\x1b[13t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[3;0;0t");
    }

    #[test]
    fn test_xtwinops_window_position_and_iconified_getters_default() {
        let term = Terminal::new(80, 24);

        assert_eq!(term.window_position(), (0, 0));
        assert!(!term.window_iconified());
    }

    #[test]
    fn test_xtwinops_window_position_and_iconified_getters_reflect_host_values() {
        let mut term = Terminal::new(80, 24);

        term.set_window_position(30, 40);
        term.set_window_iconified(true);

        assert_eq!(term.window_position(), (30, 40));
        assert!(term.window_iconified());
    }

    #[test]
    fn test_xtwinops_manipulation_ops_are_noop() {
        let mut term = Terminal::new(80, 24);

        // Raise window to front (CSI 5 t) - no window to act on, no response
        term.process(b"\x1b[5t");
        assert!(!term.has_pending_responses());
        let response = term.drain_responses();
        assert!(response.is_empty());
    }
}
