//! Window-related CSI sequence handling (XTWINOPS, etc.)

use crate::terminal::Terminal;
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
            match action {
                'x' => {
                    // DECFRA - Fill Rectangular Area: CSI Pc ; Pt ; Pl ; Pb ; Pr $ x
                    let mut iter = params.iter();
                    let pc =
                        iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as u8 as char;
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

                    let mut fill_cell = crate::cell::Cell::new(pc);
                    fill_cell.fg = self.fg;
                    fill_cell.bg = self.bg;
                    fill_cell.flags = self.flags;

                    self.active_grid_mut()
                        .fill_rectangle(fill_cell, top, left, bottom, right);
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

                    self.active_grid_mut().copy_rectangle(
                        src_top, src_left, src_bottom, src_right, dst_top, dst_left,
                    );
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

                    self.active_grid_mut()
                        .erase_rectangle_unconditional(top, left, bottom, right);
                }
                '{' => {
                    // DECSERA - Selective Erase Rectangular Area: CSI Pt ; Pl ; Pb ; Pr $ {
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

                    self.active_grid_mut()
                        .erase_rectangle(top, left, bottom, right);
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

                    if action == 'r' {
                        self.active_grid_mut().change_attributes_in_rectangle(
                            top,
                            left,
                            bottom,
                            right,
                            &attributes,
                        );
                    } else {
                        self.active_grid_mut().reverse_attributes_in_rectangle(
                            top,
                            left,
                            bottom,
                            right,
                            &attributes,
                        );
                    }
                }
                _ => {}
            }
            return;
        }

        match action {
            't' => {
                // Window manipulation (XTWINOPS) or DECSWBV (Set Warning Bell Volume)
                let mut iter = params.iter();
                let n = iter.next().and_then(|p| p.first()).copied().unwrap_or(0);

                // DECSWBV - Set Warning Bell Volume: CSI Ps t or CSI Ps SP t
                if params.iter().count() == 1 && (n <= 8 || intermediates.contains(&b' ')) {
                    self.warning_bell_volume = n.min(8) as u8;
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
                        if self.window_iconified {
                            self.push_response(b"\x1b[2t");
                        } else {
                            self.push_response(b"\x1b[1t");
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
                        let x = self.window_position_x.max(0);
                        let y = self.window_position_y.max(0);
                        let response = format!("\x1b[3;{};{}t", x, y);
                        self.push_response(response.as_bytes());
                    }
                    14 => {
                        // Report text area size in pixels. Also covers the
                        // window-size-vs-text-area sub-form `CSI 14 ; 2 t` (n is
                        // still 14); no separate window frame exists here, so the
                        // reply is the same for both.
                        let response =
                            format!("\x1b[4;{};{}t", self.pixel_height, self.pixel_width);
                        self.push_response(response.as_bytes());
                    }
                    16 => {
                        // Report character cell size in pixels.
                        // Derive from text-area pixel size / grid size so the
                        // value matches what the renderer actually uses (set
                        // via `Terminal::set_pixel_size`, which the host
                        // updates from `cell_renderer.cell_width/_height` on
                        // every resize). Falls back to a 10x20 default if the
                        // pixel/grid dimensions have not been set yet.
                        let cpw = if cols > 0 && self.pixel_width > 0 {
                            (self.pixel_width / cols).max(1)
                        } else {
                            10
                        };
                        let cph = if rows > 0 && self.pixel_height > 0 {
                            (self.pixel_height / rows).max(1)
                        } else {
                            20
                        };
                        let response = format!("\x1b[6;{};{}t", cph, cpw);
                        self.push_response(response.as_bytes());
                    }
                    18 => {
                        // Report text area size in characters
                        let response = format!("\x1b[8;{};{}t", rows, cols);
                        self.push_response(response.as_bytes());
                    }
                    19 => {
                        // Report screen size in characters. No distinct "root
                        // window" exists in a library core, so report the
                        // terminal's own size.
                        let response = format!("\x1b[9;{};{}t", rows, cols);
                        self.push_response(response.as_bytes());
                    }
                    22 => {
                        // Push icon name and window title to stack
                        self.title_state.title_stack.push(self.title_state.title.clone());
                    }
                    23 => {
                        // Pop icon name and window title from stack
                        if let Some(title) = self.title_state.title_stack.pop() {
                            self.title_state.title = title;
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
                    self.margins.scroll_region_top = top;
                    self.margins.scroll_region_bottom = bottom;
                    // Reset cursor to (0,0) relative to region if origin mode
                    self.cursor.goto(0, if self.modes.origin_mode { top } else { 0 });
                }
            }
            's'
                // Set left and right margins (DECSLRM) - only if DECLRMM is set
                if self.margins.use_lr_margins => {
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
                        self.margins.left_margin = left;
                        self.margins.right_margin = right;
                    }
                }
            _ => {}
        }
    }
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
