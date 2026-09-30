//! Report-related CSI sequence handling (DSR, DA, etc.)

use crate::terminal::Terminal;
use vte::Params;

/// First parameter of a CSI sequence, or `default` when absent.
fn first_param(params: &Params, default: u16) -> u16 {
    params
        .iter()
        .next()
        .and_then(|p| p.first())
        .copied()
        .unwrap_or(default)
}

/// DECRQM status for a mode that is either set (1) or reset (2).
fn mode_status(set: bool) -> u8 {
    if set {
        1
    } else {
        2
    }
}

impl Terminal {
    pub(crate) fn handle_csi_report(
        &mut self,
        action: char,
        params: &Params,
        intermediates: &[u8],
    ) {
        let private = intermediates.contains(&b'?');

        match action {
            'n' => self.report_dsr(params, private),
            'c' => self.report_device_attributes(intermediates),
            'q' => self.report_xtversion(intermediates, private),
            'p' => self.handle_csi_p(params, intermediates),
            'x' => self.report_decreqtparm(params),
            _ => {}
        }
    }

    /// DSR - Device Status Report (`CSI Ps n`, `CSI ? Ps n`).
    fn report_dsr(&mut self, params: &Params, private: bool) {
        match first_param(params, 0) {
            5 => {
                // Status report - response: CSI 0 n (OK)
                self.push_response(b"\x1b[0n");
            }
            6 => {
                // Cursor position report (CPR)
                // Response: CSI r ; c R
                let (col, row) = if private {
                    // Private mode CPR (some terminals use this)
                    (self.cursor.col, self.cursor.row)
                } else if self.modes.origin_mode {
                    // Respect origin mode: report relative to scroll region
                    (
                        self.cursor.col,
                        self.cursor
                            .row
                            .saturating_sub(self.margins.scroll_region_top),
                    )
                } else {
                    (self.cursor.col, self.cursor.row)
                };
                let response = format!("\x1b[{};{}R", row + 1, col + 1);
                self.push_response(response.as_bytes());
            }
            _ => {}
        }
    }

    /// DA - Device Attributes (`CSI c` primary, `CSI > c` secondary).
    fn report_device_attributes(&mut self, intermediates: &[u8]) {
        if intermediates.contains(&b'>') {
            // Secondary DA - response: CSI > 82 ; 10000 ; 0 c
            // (par-term version 82, 10000 = scrollback, 0 = ROM)
            self.push_response(b"\x1b[>82;10000;0c");
        } else {
            // Primary DA - response: CSI ? <id> ; 1 ; 4 ; 6 ; 9 ; 15 ; 22 ; 52 c
            // <id> based on conformance level
            let id = self.conformance_level.da_identifier();
            let response = format!("\x1b[?{};1;4;6;9;15;22;52c", id);
            self.push_response(response.as_bytes());
        }
    }

    /// XTVERSION - `CSI > q`.
    fn report_xtversion(&mut self, intermediates: &[u8], private: bool) {
        if intermediates.contains(&b'>') {
            let version = env!("CARGO_PKG_VERSION");
            let response = format!("\x1bP>|par-term({})\x1b\\", version);
            self.push_response(response.as_bytes());
        } else if private && intermediates.is_empty() {
            // XTVERSION can also be CSI > 0 q
            let version = env!("CARGO_PKG_VERSION");
            let response = format!("\x1bP>|par-term({})\x1b\\", version);
            self.push_response(response.as_bytes());
        }
    }

    /// `CSI … p` family: DECSCL (`"`), DECSTR (`!`), DECRQM (`$`).
    fn handle_csi_p(&mut self, params: &Params, intermediates: &[u8]) {
        if intermediates.contains(&b'"') {
            // DECSCL - Set Conformance Level: CSI Pl ; Pc " p
            let pl = first_param(params, 65);
            if let Some(level) = crate::conformance_level::ConformanceLevel::from_decscl_param(pl) {
                self.conformance_level = level;
            }
        } else if intermediates.contains(&b'!') {
            // DECSTR - Soft Terminal Reset: CSI ! p
            self.soft_reset();
        } else if intermediates.contains(&b'$') {
            // DECRQM - Request Mode (ANSI or DEC): CSI ? Pa $ p
            let private = intermediates.contains(&b'?');
            let mode = first_param(params, 0);
            let (status, mode_type) = self.decrqm_status(mode, private);
            let response = format!("\x1b[{}{};{}$y", mode_type, mode, status);
            self.push_response(response.as_bytes());
        }
    }

    /// DECRQM reply for `mode`: status (0 not recognized, 1 set, 2 reset)
    /// and the mode-type prefix (`?` for DEC private modes, empty for ANSI).
    fn decrqm_status(&self, mode: u16, private: bool) -> (u8, &'static str) {
        if !private {
            let status = match mode {
                4 => mode_status(self.modes.insert_mode),
                20 => mode_status(self.modes.line_feed_new_line_mode),
                _ => 0, // Not recognized
            };
            return (status, "");
        }
        let status = match mode {
            1 => mode_status(self.modes.application_cursor),
            6 => mode_status(self.modes.origin_mode),
            7 => mode_status(self.modes.auto_wrap),
            25 => mode_status(self.cursor.visible),
            69 => mode_status(self.margins.use_lr_margins),
            1004 => mode_status(self.modes.focus_tracking),
            1005 | 1006 | 1015 => {
                mode_status(self.modes.mouse_encoding != crate::mouse::MouseEncoding::Default)
            }
            9 | 1000 | 1002 | 1003 => {
                mode_status(self.modes.mouse_mode != crate::mouse::MouseMode::Off)
            }
            47 | 1047 | 1049 => mode_status(self.alt_screen_active),
            // 1048 is a save/restore action, not a persistent mode;
            // report whether a saved cursor is pending restore
            1048 => mode_status(self.saved_state.saved_cursor.is_some()),
            2004 => mode_status(self.modes.bracketed_paste),
            2026 => mode_status(self.sync_state.synchronized_updates),
            80 => mode_status(self.modes.sixel_display_mode),
            _ => 0, // Not recognized
        };
        (status, "?")
    }

    /// DECREQTPARM - Request Terminal Parameters (`CSI Ps x`).
    fn report_decreqtparm(&mut self, params: &Params) {
        let ps = first_param(params, 0);

        // Response: CSI <sol>; <par>; <nb>; <nw>; <tw>; <ti>; <cl> x
        // sol: 2=solicited, 3=unsolicited
        // par: 1=no parity
        // nb: 1=8 bits
        // nw: 120=speed (9600)
        // tw: 120=speed
        // ti: 1=bit multiplier
        // cl: 0=no flags

        // Test expectations: ps=0 -> sol=2, ps=1 -> sol=3
        let sol = if ps == 0 { 2 } else { 3 };
        let response = format!("\x1b[{};1;1;120;120;1;0x", sol);
        self.push_response(response.as_bytes());
    }

    /// DECRQCRA - Request Checksum of Rectangular Area
    /// CSI Pi ; Pg ; Pt ; Pl ; Pb ; Pr * y
    /// Response: DCS Pi ! ~ xxxx ST (4 hex-digit checksum)
    pub(crate) fn handle_decrqcra(&mut self, params: &Params) {
        let params_vec: Vec<u16> = params
            .iter()
            .flat_map(|subparams| subparams.iter().copied())
            .collect();

        let pi = params_vec.first().copied().unwrap_or(0); // Request ID
        let _pg = params_vec.get(1).copied().unwrap_or(1); // Page number (ignored, single page)

        // Rectangle coordinates (1-indexed, default to full screen)
        let top = params_vec.get(2).copied().unwrap_or(1).max(1) as usize - 1;
        let left = params_vec.get(3).copied().unwrap_or(1).max(1) as usize - 1;
        let bottom = params_vec
            .get(4)
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
            .get(5)
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

        // Compute checksum: sum of character values in the rectangle
        let mut checksum: u16 = 0;
        for row in top..=bottom {
            if let Some(cells) = self.active_grid().row(row) {
                let cell_end = right.min(cells.len().saturating_sub(1));
                for cell in cells.iter().take(cell_end + 1).skip(left) {
                    let ch = cell.c;
                    if ch == '\0' || ch == ' ' {
                        checksum = checksum.wrapping_add(b' ' as u16);
                    } else {
                        checksum = checksum.wrapping_add(ch as u16);
                    }
                }
                // Fill remaining columns with spaces if right > cells.len()
                for _ in cells.len()..=right {
                    checksum = checksum.wrapping_add(b' ' as u16);
                }
            } else {
                // Empty row - add spaces for all columns
                for _ in left..=right {
                    checksum = checksum.wrapping_add(b' ' as u16);
                }
            }
        }

        // Response: DCS Pi ! ~ XXXX ST
        let response = format!("\x1bP{}!~{:04X}\x1b\\", pi, checksum);
        self.push_response(response.as_bytes());
    }
}
