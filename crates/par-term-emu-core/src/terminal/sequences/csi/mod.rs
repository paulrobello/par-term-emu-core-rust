//! CSI (Control Sequence Introducer) sequence handling dispatcher

mod color_stack;
mod cursor;
mod edit;
mod erase;
mod keyboard;
pub(crate) mod mode;
mod report;
mod scroll;
mod style;
mod window;

use crate::debug;
use crate::terminal::Terminal;
use vte::Params;

/// First parameter as a count; 0 or missing means 1 (the VT default the
/// cursor-motion and insert/delete handlers share).
pub(super) fn count_param(params: &Params) -> usize {
    let n = params
        .iter()
        .next()
        .and_then(|p| p.first())
        .copied()
        .unwrap_or(1) as usize;
    if n == 0 {
        1
    } else {
        n
    }
}

impl Terminal {
    /// VTE CSI dispatch - handle CSI sequences
    pub(in crate::terminal) fn csi_dispatch_impl(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Extract params for debug logging only when a Debug-level message
        // will be written — this runs for every CSI sequence (QA-112), and
        // the Vec is not used by the dispatch itself (the 's' branch reads
        // params.is_empty() directly).
        if debug::is_enabled(debug::DebugLevel::Debug) {
            let params_vec: Vec<i64> = params
                .iter()
                .flat_map(|subparams| subparams.iter().copied().map(|p| p as i64))
                .collect();

            debug::log_csi_dispatch(&params_vec, intermediates, action);
        }

        match action {
            'A' | 'B' | 'C' | 'D' | 'H' | 'f' | 'E' | 'F' | 'G' | '`' | 'd' | 'I' | 'Z' | 'g' => {
                self.handle_csi_cursor(action, params, intermediates);
            }
            'J' | 'K' | 'X' => {
                let grid = if self.alt_screen_active {
                    &mut self.alt_grid
                } else {
                    &mut self.grid
                };
                erase::handle_csi_erase(
                    grid,
                    &mut self.graphics.graphics_store,
                    &mut self.events,
                    self.attrs.bg,
                    (self.cursor.col, self.cursor.row),
                    action,
                    params,
                );
            }
            'S' | 'T' => {
                self.handle_csi_scroll(action, params, intermediates);
            }
            'm' => {
                style::handle_csi_style(
                    &mut self.attrs,
                    &mut self.keyboard_state,
                    &self.theme,
                    &mut self.response_buffer,
                    action,
                    params,
                    intermediates,
                );
            }
            'h' | 'l' => {
                self.handle_csi_mode(action, params, intermediates);
            }
            'n' | 'c' => {
                self.handle_csi_report(action, params, intermediates);
            }
            'y' => {
                if intermediates.contains(&b'*') {
                    // DECRQCRA - Request Checksum of Rectangular Area
                    let grid = if self.alt_screen_active {
                        &self.alt_grid
                    } else {
                        &self.grid
                    };
                    report::handle_decrqcra(grid, &mut self.response_buffer, params);
                } else {
                    self.handle_csi_report(action, params, intermediates);
                }
            }
            'q' => {
                // q can be DECSCUSR (with space), DECSCA (with "), or XTVERSION (with >)
                if intermediates.contains(&b' ') {
                    self.handle_csi_cursor(action, params, intermediates);
                } else if intermediates.contains(&b'"') {
                    erase::handle_decsca(&mut self.modes, params);
                } else {
                    self.handle_csi_report(action, params, intermediates);
                }
            }
            't' | 'r' => {
                self.handle_csi_window(action, params, intermediates);
            }
            's' => {
                // s can be SCOSC (no params) or DECSLRM (with params, only if DECLRMM is set)
                // We check if there are any parameters to distinguish them
                if !params.is_empty() && self.margins.use_lr_margins {
                    self.handle_csi_window(action, params, intermediates);
                } else {
                    self.handle_csi_cursor(action, params, intermediates);
                }
            }
            'x' => {
                // x can be DECREQTPARM (no intermediates), rectangular area
                // operations (with $), or DECSACE (with *).
                if intermediates.contains(&b'$') {
                    self.handle_csi_window(action, params, intermediates);
                } else if intermediates.contains(&b'*') {
                    window::handle_decsace(&mut self.modes, params);
                } else {
                    self.handle_csi_report(action, params, intermediates);
                }
            }
            'v' | 'z' => {
                // Rectangular area operations (DECCRA, etc.)
                if intermediates.contains(&b'$') {
                    self.handle_csi_window(action, params, intermediates);
                }
            }
            '{' => {
                // { with $ is DECSERA (Selective Erase Rectangular Area)
                if intermediates.contains(&b'$') {
                    let bg = self.attrs.bg;
                    erase::handle_decsera(self.active_grid_mut(), bg, params);
                }
            }
            'L' | 'M' | '@' => {
                self.handle_csi_edit(action, params, intermediates);
            }
            'P' => {
                // P with # is XTPUSHCOLORS; bare P is DCH (delete chars)
                if intermediates.contains(&b'#') {
                    color_stack::handle_xtpushcolors(&mut self.theme, params);
                } else {
                    self.handle_csi_edit(action, params, intermediates);
                }
            }
            'Q' => {
                // Q with # is XTPOPCOLORS; bare Q is unused
                if intermediates.contains(&b'#') {
                    color_stack::handle_xtpopcolors(&mut self.theme, params);
                } else {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "CSI",
                        &format!("Unsupported CSI action: {}", action),
                    );
                }
            }
            'R' => {
                // R with # is XTREPORTCOLORS; bare R is the CPR reply an
                // application would echo, not something we act on
                if intermediates.contains(&b'#') {
                    color_stack::handle_xtreportcolors(&self.theme, &mut self.response_buffer);
                } else {
                    debug::log(
                        debug::DebugLevel::Debug,
                        "CSI",
                        &format!("Unsupported CSI action: {}", action),
                    );
                }
            }
            'p' => {
                // p can be DECSCL (with "), DECSTR (with !), or DECRQM (with $)
                if intermediates.contains(&b'"')
                    || intermediates.contains(&b'!')
                    || intermediates.contains(&b'$')
                {
                    self.handle_csi_report(action, params, intermediates);
                }
            }
            'u' => {
                // u can be SCORC (no params), DECSMBV (with params),
                // or Kitty keyboard protocol (with =, ?, >, or < intermediates)
                if intermediates.contains(&b'=')
                    || intermediates.contains(&b'?')
                    || intermediates.contains(&b'>')
                    || intermediates.contains(&b'<')
                {
                    keyboard::handle_csi_keyboard(
                        &mut self.keyboard_state,
                        &mut self.response_buffer,
                        action,
                        params,
                        intermediates,
                    );
                } else {
                    self.handle_csi_cursor(action, params, intermediates);
                }
            }
            _ => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "CSI",
                    &format!("Unsupported CSI action: {}", action),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod count_param_tests {
    use super::count_param;

    /// The first CSI's params, through a real vte parse.
    fn first_param_count(seq: &[u8]) -> usize {
        struct Capture(Option<usize>);
        impl vte::Perform for Capture {
            fn csi_dispatch(&mut self, params: &vte::Params, _: &[u8], _: bool, _: char) {
                self.0.get_or_insert(count_param(params));
            }
        }
        let mut capture = Capture(None);
        vte::Parser::new().advance(&mut capture, seq);
        capture.0.expect("a CSI dispatched")
    }

    #[test]
    fn zero_and_missing_mean_one() {
        assert_eq!(first_param_count(b"\x1b[0A"), 1, "param 0");
        assert_eq!(first_param_count(b"\x1b[A"), 1, "missing param");
        assert_eq!(first_param_count(b"\x1b[5A"), 5, "param 5");
    }
}
