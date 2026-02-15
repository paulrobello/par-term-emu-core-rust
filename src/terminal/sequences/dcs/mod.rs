//! DCS (Device Control String) sequence handling dispatcher

mod sixel;

use crate::debug;
use crate::graphics::{next_graphic_id, GraphicProtocol, TerminalGraphic};
use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    /// VTE hook - start of DCS sequence
    pub(in crate::terminal) fn dcs_hook(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if action == 'q' && self.disable_insecure_sequences {
            debug::log(
                debug::DebugLevel::Debug,
                "SECURITY",
                "Blocked Sixel DCS (disable_insecure_sequences=true)",
            );
            return;
        }

        self.dcs_active = true;
        self.dcs_action = Some(action);
        self.dcs_buffer.clear();

        if action == 'q' {
            self.handle_sixel_hook(params);
        }
    }

    /// VTE put - data for DCS sequence
    pub(in crate::terminal) fn dcs_put(&mut self, byte: u8) {
        if !self.dcs_active {
            return;
        }

        if self.dcs_action == Some('q') {
            let is_sixel_data = (63..=126).contains(&byte);

            if is_sixel_data {
                let mut pending_repeat = None;
                let has_repeat = !self.dcs_buffer.is_empty() && self.dcs_buffer[0] == b'!';

                if has_repeat {
                    // Parse repeat count
                    let s = std::str::from_utf8(&self.dcs_buffer[1..]).unwrap_or("1");
                    let count = s.parse().unwrap_or(1);
                    pending_repeat = Some(count);
                    self.dcs_buffer.clear();
                } else if !self.dcs_buffer.is_empty() {
                    // Process any other pending commands (colors)
                    self.process_sixel_command();
                }

                // Feed to parser
                if let Some(parser) = &mut self.sixel_parser {
                    if let Some(count) = pending_repeat {
                        parser.parse_repeat(count, byte as char);
                    } else {
                        parser.parse_sixel(byte as char);
                    }
                }
            } else if byte == b'-' {
                if !self.dcs_buffer.is_empty() {
                    self.process_sixel_command();
                }
                if let Some(p) = &mut self.sixel_parser {
                    p.new_line();
                }
            } else if byte == b'$' {
                if !self.dcs_buffer.is_empty() {
                    self.process_sixel_command();
                }
                if let Some(p) = &mut self.sixel_parser {
                    p.carriage_return();
                }
            } else {
                // Control chars or parameters (#, ", !, digits)
                // If starting a new command, process previous one
                if (byte == b'#' || byte == b'"' || byte == b'!') && !self.dcs_buffer.is_empty() {
                    self.process_sixel_command();
                }
                self.dcs_buffer.push(byte);
            }
        } else {
            self.dcs_buffer.push(byte);
        }
    }

    /// VTE unhook - end of DCS sequence
    pub(in crate::terminal) fn dcs_unhook(&mut self) {
        if !self.dcs_active {
            return;
        }

        if self.dcs_action == Some('q') {
            self.process_sixel_command();
            if let Some(parser) = self.sixel_parser.take() {
                let position = (self.cursor.col, self.cursor.row);
                let sixel_graphic = parser.build_graphic(position);

                // Convert SixelGraphic to TerminalGraphic
                let mut pixels = Vec::with_capacity(sixel_graphic.width * sixel_graphic.height * 4);
                for y in 0..sixel_graphic.height {
                    for x in 0..sixel_graphic.width {
                        if let Some((r, g, b, a)) = sixel_graphic.get_pixel(x, y) {
                            pixels.push(r);
                            pixels.push(g);
                            pixels.push(b);
                            pixels.push(a);
                        } else {
                            pixels.extend_from_slice(&[0, 0, 0, 0]);
                        }
                    }
                }

                let mut graphic = TerminalGraphic::new(
                    next_graphic_id(),
                    GraphicProtocol::Sixel,
                    position,
                    sixel_graphic.width,
                    sixel_graphic.height,
                    pixels,
                );

                let (cell_w, cell_h) = self.cell_dimensions;
                graphic.set_cell_dimensions(cell_w, cell_h);

                let row = self.cursor.row;
                self.graphics_store.add_graphic(graphic);
                self.terminal_events
                    .push(crate::terminal::TerminalEvent::GraphicsAdded(row));

                // Advance cursor to next line(s) as per test expectation
                if cell_h > 0 {
                    let rows = (sixel_graphic.height as f32 / cell_h as f32).ceil() as usize;
                    self.cursor.col = 0;
                    let (_cols, screen_rows) = self.size();
                    self.cursor.move_down(rows, screen_rows.saturating_sub(1));
                }
            }
        }

        self.dcs_active = false;
        self.dcs_action = None;
        self.dcs_buffer.clear();
    }
}

#[cfg(test)]
mod tests;
