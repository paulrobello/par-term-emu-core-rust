//! DCS (Device Control String) sequence handling
//!
//! Handles DCS sequences, primarily for Sixel graphics support.

use crate::debug;
use crate::sixel;
use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    /// Process accumulated Sixel command from DCS buffer
    pub(in crate::terminal) fn process_sixel_command(&mut self) {
        if self.dcs_buffer.is_empty() {
            return;
        }

        let Some(parser) = &mut self.sixel_parser else {
            return;
        };

        let buffer_str = String::from_utf8_lossy(&self.dcs_buffer);
        let command = buffer_str.chars().next().unwrap_or('\0');

        match command {
            '#' => {
                // Color command: #Pc or #Pc;Pu;Px;Py;Pz
                let params: Vec<&str> = buffer_str[1..].split(';').collect();
                if let Ok(color_idx) = params[0].parse::<usize>() {
                    if params.len() == 1 {
                        // Select color
                        parser.select_color(color_idx);
                    } else if params.len() == 5 {
                        // Define color
                        if let (Ok(color_system), Ok(x), Ok(y), Ok(z)) = (
                            params[1].parse::<u8>(),
                            params[2].parse::<u16>(),
                            params[3].parse::<u16>(),
                            params[4].parse::<u16>(),
                        ) {
                            parser.define_color(color_idx, color_system, x, y, z);
                        }
                    }
                }
            }
            '"' => {
                // Raster attributes: "Pan;Pad;Ph;Pv
                let params: Vec<&str> = buffer_str[1..].split(';').collect();
                if params.len() >= 4 {
                    if let (Ok(pan), Ok(pad), Ok(width), Ok(height)) = (
                        params[0].parse::<u16>(),
                        params[1].parse::<u16>(),
                        params[2].parse::<usize>(),
                        params[3].parse::<usize>(),
                    ) {
                        parser.set_raster_attributes(pan, pad, width, height);
                    }
                }
            }
            '!' => {
                // Repeat sequence: !Pn character
                if buffer_str.len() >= 2 {
                    let count_str = &buffer_str[1..buffer_str.len() - 1];
                    let repeat_char = buffer_str.chars().last().unwrap_or('?');
                    if let Ok(count) = count_str.parse::<usize>() {
                        parser.parse_repeat(count, repeat_char);
                    }
                }
            }
            _ => {}
        }

        self.dcs_buffer.clear();
    }

    /// VTE hook - start of DCS sequence
    pub(in crate::terminal) fn dcs_hook(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Block Sixel graphics if insecure sequences are disabled
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
            // Sixel graphics
            let mut parser = sixel::SixelParser::new();

            // Extract parameters
            let params_vec: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();

            parser.set_params(&params_vec);
            self.sixel_parser = Some(parser);

            debug::log(
                debug::DebugLevel::Debug,
                "SIXEL",
                "Started Sixel DCS sequence",
            );
        }
    }

    /// VTE put - DCS data byte
    pub(in crate::terminal) fn dcs_put(&mut self, byte: u8) {
        if !self.dcs_active {
            return;
        }

        if let Some(action) = self.dcs_action {
            if action == 'q' {
                // Sixel data
                let ch = byte as char;

                // If we are currently accumulating a command, decide whether to flush it
                if !self.dcs_buffer.is_empty() {
                    let first = self.dcs_buffer[0] as char;

                    match first {
                        // Repeat command needs the trailing data character included
                        '!' => {
                            match ch {
                                // Still reading the repeat count
                                '0'..='9' | ';' => {
                                    self.dcs_buffer.push(byte);
                                    return;
                                }
                                // Next non-digit is the character to repeat; include it then flush
                                '?'..='~' => {
                                    self.dcs_buffer.push(byte);
                                    self.process_sixel_command();
                                    return;
                                }
                                // Any other token ends the command; flush then re-handle this char
                                _ => {
                                    self.process_sixel_command();
                                    // Fall through to handle current char anew
                                }
                            }
                        }
                        // Parameterized commands (" and #) should flush when a non-digit/';' appears
                        '"' | '#' => {
                            match ch {
                                '0'..='9' | ';' => {
                                    self.dcs_buffer.push(byte);
                                    return;
                                }
                                // New command/data token -> flush and then handle it normally
                                _ => {
                                    self.process_sixel_command();
                                    // Fall through
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Handle current character (no pending parameter buffer or it was just flushed)
                if let Some(parser) = &mut self.sixel_parser {
                    match ch {
                        '$' => parser.carriage_return(),
                        '-' => parser.new_line(),
                        '#' | '"' | '!' => {
                            // Start accumulating a new command
                            self.dcs_buffer.push(byte);
                        }
                        // Sixel data character
                        '?'..='~' => parser.parse_sixel(ch),
                        // Parameter digits encountered without an active command - ignore
                        _ => {}
                    }
                }
            }
        }
    }

    /// VTE unhook - end of DCS sequence
    pub(in crate::terminal) fn dcs_unhook(&mut self) {
        // Process any remaining buffered command
        if !self.dcs_buffer.is_empty() {
            self.process_sixel_command();
        }

        if let Some(action) = self.dcs_action {
            if action == 'q' {
                // Finalize Sixel graphic
                if let Some(parser) = self.sixel_parser.take() {
                    let position = (self.cursor.col, self.cursor.row);
                    let mut graphic = parser.build_graphic(position);

                    // Store graphic dimensions before moving it
                    let graphic_width = graphic.width;
                    let graphic_height = graphic.height;

                    // Set cell dimensions for Sixel graphics based on how they're displayed
                    // Each character cell displays 2 vertical Sixel pixels using half-blocks
                    // For horizontal, we'll use a 1:1 mapping (1 pixel per column)
                    // This ensures consistent rendering across different screenshot font sizes
                    graphic.set_cell_dimensions(1, 2);

                    debug::log(
                        debug::DebugLevel::Debug,
                        "SIXEL",
                        &format!(
                            "Graphic added at ({},{}) size {}x{}",
                            position.0, position.1, graphic_width, graphic_height
                        ),
                    );

                    // Calculate how many terminal rows the graphic occupies
                    // Each terminal row displays 2 pixel rows using Unicode half-blocks
                    let graphic_height_in_rows = graphic_height.div_ceil(2);

                    self.graphics.push(graphic);

                    // After Sixel graphic, cursor should move to left margin of line below graphic
                    // per VT340 specification - this makes graphics "occupy space"
                    self.cursor.col = 0;
                    self.cursor.row = self.cursor.row.saturating_add(graphic_height_in_rows);

                    // Clamp cursor to valid range
                    let (_, rows) = self.size();
                    if self.cursor.row >= rows {
                        // If graphic pushed cursor past bottom, scroll up and place cursor at bottom
                        let scroll_amount = self.cursor.row - rows + 1;
                        let scroll_top = self.scroll_region_top;
                        let scroll_bottom = self.scroll_region_bottom;
                        self.active_grid_mut().scroll_region_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );
                        // Adjust graphics to scroll with content
                        self.adjust_graphics_for_scroll_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );
                        self.cursor.row = rows - 1;
                    }

                    debug::log(
                        debug::DebugLevel::Debug,
                        "SIXEL",
                        &format!(
                            "Cursor advanced to ({},{}) after graphic (height {} pixels = {} rows)",
                            self.cursor.col,
                            self.cursor.row,
                            graphic_height,
                            graphic_height_in_rows
                        ),
                    );
                }
            }
        }

        self.dcs_active = false;
        self.dcs_action = None;
        self.dcs_buffer.clear();
    }
}
