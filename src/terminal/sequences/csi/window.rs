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
                    0..=8 => {
                        // Already handled above, but kept for match exhaustiveness/structure
                    }
                    14 => {
                        // Report text area size in pixels
                        let response =
                            format!("\x1b[4;{};{}t", self.pixel_height, self.pixel_width);
                        self.push_response(response.as_bytes());
                    }
                    16 => {
                        // Report character cell size in pixels
                        let (cpw, cph) = (10, 20); // Default cell size
                        let response = format!("\x1b[6;{};{}t", cph, cpw);
                        self.push_response(response.as_bytes());
                    }
                    18 => {
                        // Report text area size in characters
                        let response = format!("\x1b[8;{};{}t", rows, cols);
                        self.push_response(response.as_bytes());
                    }
                    22 => {
                        // Push icon name and window title to stack
                        self.title_stack.push(self.title.clone());
                    }
                    23 => {
                        // Pop icon name and window title from stack
                        if let Some(title) = self.title_stack.pop() {
                            self.title = title;
                        }
                    }
                    _ => {}
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
                    self.scroll_region_top = top;
                    self.scroll_region_bottom = bottom;
                    // Reset cursor to (0,0) relative to region if origin mode
                    self.cursor.goto(0, if self.origin_mode { top } else { 0 });
                }
            }
            's' => {
                // Set left and right margins (DECSLRM) - only if DECLRMM is set
                if self.use_lr_margins {
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
                        self.left_margin = left;
                        self.right_margin = right;
                    }
                }
            }
            _ => {}
        }
    }
}
