//! VTE Perform trait implementation for Terminal
//!
//! This module implements the `vte::Perform` trait, which is the primary
//! interface between the terminal parser and the terminal state.
//! Most methods here delegate to specialized handlers in other modules.

use crate::debug;
use crate::terminal::{BellEvent, Terminal, TerminalEvent};
use vte::{Params, Perform};

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        debug::log_print(c, self.cursor.col, self.cursor.row);

        // Apply Unicode normalization if configured
        if !self.normalization_form.is_none() {
            let normalized = self.normalization_form.normalize_char(c);
            let mut chars = normalized.chars();
            if let Some(first) = chars.next() {
                self.write_char(first);
                for ch in chars {
                    self.write_char(ch);
                }
            }
        } else {
            self.write_char(c);
        }
    }

    fn execute(&mut self, byte: u8) {
        debug::log_execute(byte);
        match byte {
            b'\n' => self.write_char('\n'),
            b'\r' => self.write_char('\r'),
            b'\t' => self.write_char('\t'),
            b'\x08' => self.write_char('\x08'),
            b'\x05' => {
                // ENQ (Enquiry) - send answerback string if configured
                if let Some(ref answerback) = self.answerback_string {
                    self.response_buffer
                        .extend_from_slice(answerback.as_bytes());
                }
            }
            b'\x07' => {
                // Bell - increment counter for visual bell support
                self.bell_count = self.bell_count.wrapping_add(1);
                // Also increment in session variables for badge evaluation
                self.session_variables.increment_bell_count();
                // Add bell event based on volume settings
                let event = if self.warning_bell_volume > 0 {
                    BellEvent::WarningBell(self.warning_bell_volume)
                } else {
                    BellEvent::VisualBell
                };
                self.bell_events.push(event.clone());
                self.terminal_events.push(TerminalEvent::BellRang(event));
            }
            _ => {}
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.dcs_hook(params, intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        self.dcs_put(byte);
    }

    fn unhook(&mut self) {
        self.dcs_unhook();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.osc_dispatch_impl(params, bell_terminated);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.csi_dispatch_impl(params, intermediates, ignore, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.esc_dispatch_impl(intermediates, ignore, byte);
    }
}
