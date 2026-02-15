//! CSI (Control Sequence Introducer) sequence handling dispatcher

mod cursor;
mod edit;
mod erase;
mod keyboard;
mod mode;
mod report;
mod scroll;
mod style;
mod window;

use crate::debug;
use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    /// VTE CSI dispatch - handle CSI sequences
    pub(in crate::terminal) fn csi_dispatch_impl(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Extract params for debug logging
        let params_vec: Vec<i64> = params
            .iter()
            .flat_map(|subparams| subparams.iter().copied().map(|p| p as i64))
            .collect();

        debug::log_csi_dispatch(&params_vec, intermediates, action);

        match action {
            'A' | 'B' | 'C' | 'D' | 'H' | 'f' | 'E' | 'F' | 'G' | '`' | 'd' | 'I' | 'Z' | 'g' => {
                self.handle_csi_cursor(action, params, intermediates);
            }
            'J' | 'K' | 'X' => {
                self.handle_csi_erase(action, params, intermediates);
            }
            'S' | 'T' => {
                self.handle_csi_scroll(action, params, intermediates);
            }
            'm' => {
                self.handle_csi_style(action, params, intermediates);
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
                    self.handle_decrqcra(params);
                } else {
                    self.handle_csi_report(action, params, intermediates);
                }
            }
            'q' => {
                // q can be DECSCUSR (with space), DECSCA (with "), or XTVERSION (with >)
                if intermediates.contains(&b' ') {
                    self.handle_csi_cursor(action, params, intermediates);
                } else if intermediates.contains(&b'"') {
                    self.handle_decsca(params);
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
                if !params_vec.is_empty() && self.use_lr_margins {
                    self.handle_csi_window(action, params, intermediates);
                } else {
                    self.handle_csi_cursor(action, params, intermediates);
                }
            }
            'x' => {
                // x can be DECREQTPARM (no intermediates) or rectangular area operations (with $)
                if intermediates.contains(&b'$') {
                    self.handle_csi_window(action, params, intermediates);
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
                    self.handle_decsera(params);
                }
            }
            'L' | 'M' | '@' | 'P' => {
                self.handle_csi_edit(action, params, intermediates);
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
                    self.handle_csi_keyboard(action, params, intermediates);
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
