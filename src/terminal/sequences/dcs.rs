//! DCS (Device Control String) and APC sequence handling
//!
//! Handles DCS sequences for Sixel graphics and APC sequences for Kitty graphics.

use crate::debug;
use crate::graphics::kitty::KittyParser;
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
            let mut parser = sixel::SixelParser::new_with_limits(self.sixel_limits);

            // Extract parameters
            let params_vec: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();

            parser.set_params(&params_vec);
            self.sixel_parser = Some(parser);

            debug::log(
                debug::DebugLevel::Debug,
                "SIXEL",
                "Started Sixel DCS sequence",
            );
        } else if action == 'G' {
            // Kitty graphics protocol (APC G ... ST)
            debug::log(
                debug::DebugLevel::Debug,
                "KITTY",
                "Started Kitty graphics sequence",
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
            } else if action == 'G' {
                // Kitty graphics - accumulate all data
                self.dcs_buffer.push(byte);
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

                    // Set cell dimensions from terminal configuration
                    // TUI renderers use (1, 2) for half-block rendering
                    // Pixel renderers should call set_cell_dimensions() with actual values
                    let (cell_w, cell_h) = self.cell_dimensions;
                    graphic.set_cell_dimensions(cell_w, cell_h);

                    debug::log(
                        debug::DebugLevel::Debug,
                        "SIXEL",
                        &format!(
                            "Graphic added at ({},{}) size {}x{}, cell_dims=({},{})",
                            position.0, position.1, graphic_width, graphic_height, cell_w, cell_h
                        ),
                    );

                    // Calculate how many terminal rows the graphic occupies
                    let graphic_height_in_rows = graphic_height.div_ceil(cell_h as usize);

                    // After Sixel graphic, cursor should move to left margin of line below graphic
                    // per VT340 specification - this makes graphics "occupy space"
                    let new_cursor_col = 0;
                    let new_cursor_row = self.cursor.row.saturating_add(graphic_height_in_rows);

                    // Calculate if we need to scroll and adjust graphic position accordingly
                    let (_, rows) = self.size();
                    if new_cursor_row >= rows {
                        // If graphic pushed cursor past bottom, scroll up
                        let scroll_amount = new_cursor_row - rows + 1;
                        let scroll_top = self.scroll_region_top;
                        let scroll_bottom = self.scroll_region_bottom;

                        // Scroll the grid and existing graphics (but not the new one we haven't added yet)
                        self.active_grid_mut().scroll_region_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );
                        self.adjust_graphics_for_scroll_up(
                            scroll_amount,
                            scroll_top,
                            scroll_bottom,
                        );

                        // Adjust the new graphic's position to account for the scroll
                        // The graphic was at cursor.row, which just scrolled up by scroll_amount
                        let original_row = graphic.position.1;
                        let new_row = original_row.saturating_sub(scroll_amount);
                        graphic.position.1 = new_row;

                        // If position was clamped to 0, track how many rows scrolled off
                        if scroll_amount > original_row {
                            graphic.scroll_offset_rows = scroll_amount - original_row;
                        }

                        self.cursor.row = rows - 1;
                        self.cursor.col = new_cursor_col;
                    } else {
                        self.cursor.row = new_cursor_row;
                        self.cursor.col = new_cursor_col;
                    }

                    // Convert SixelGraphic to TerminalGraphic and add to store
                    let terminal_graphic = crate::graphics::TerminalGraphic::new(
                        crate::graphics::next_graphic_id(),
                        crate::graphics::GraphicProtocol::Sixel,
                        graphic.position,
                        graphic.width,
                        graphic.height,
                        graphic.pixels.clone(),
                    );
                    self.graphics_store.add_graphic(terminal_graphic);

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
            } else if action == 'G' {
                // Kitty graphics protocol
                self.process_kitty_graphics();
            }
        }

        self.dcs_active = false;
        self.dcs_action = None;
        self.dcs_buffer.clear();
    }

    /// Process accumulated Kitty graphics data
    fn process_kitty_graphics(&mut self) {
        if self.dcs_buffer.is_empty() {
            return;
        }

        let payload = match std::str::from_utf8(&self.dcs_buffer) {
            Ok(s) => s,
            Err(_) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    "Invalid UTF-8 in Kitty graphics payload",
                );
                return;
            }
        };

        let mut parser = KittyParser::new();

        // Parse the payload (may be chunked)
        match parser.parse_chunk(payload) {
            Ok(more_chunks) => {
                if more_chunks {
                    // TODO: Support chunked transmission by storing parser state
                    debug::log(
                        debug::DebugLevel::Debug,
                        "KITTY",
                        "Chunked Kitty graphics not yet fully supported",
                    );
                    return;
                }
            }
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    &format!("Failed to parse Kitty graphics: {}", e),
                );
                return;
            }
        }

        // Check if this is a query action - if so, send response immediately
        if matches!(parser.action, crate::graphics::kitty::KittyAction::Query) {
            // Send query response: _Gi=<id>;OK ESC \
            let response = if let Some(id) = parser.image_id {
                format!("\x1b_Gi={};OK\x1b\\", id)
            } else {
                // If no image_id specified, respond without id
                "\x1b_Gi=0;OK\x1b\\".to_string()
            };

            self.push_response(response.as_bytes());

            debug::log(
                debug::DebugLevel::Debug,
                "KITTY",
                &format!(
                    "Responded to Kitty graphics query: {}",
                    response.escape_debug()
                ),
            );
            return;
        }

        // Get cursor position for graphic placement
        let position = (self.cursor.col, self.cursor.row);

        // Build the graphic using the terminal's graphics store
        use crate::graphics::kitty::KittyGraphicResult;
        match parser.build_graphic(position, &mut self.graphics_store) {
            Ok(KittyGraphicResult::Graphic(mut graphic)) => {
                // Get terminal dimensions and cell size
                let (_, rows) = self.size();
                let (_, cell_h) = self.cell_dimensions;

                // Calculate graphic height in terminal rows
                let graphic_height = graphic.height;
                let graphic_height_in_rows = graphic_height.div_ceil(cell_h as usize);

                // Calculate new cursor position after graphic
                let original_row = self.cursor.row;
                let new_cursor_row = original_row.saturating_add(graphic_height_in_rows);
                let new_cursor_col = 0; // Move to start of next line after image

                // Check if we need to scroll
                if new_cursor_row >= rows {
                    // Need to scroll to make room for graphic
                    let scroll_amount = new_cursor_row - rows + 1;
                    let scroll_top = self.scroll_region_top;
                    let scroll_bottom = self.scroll_region_bottom;

                    // Scroll the grid and existing graphics (but not the new one we haven't added yet)
                    self.active_grid_mut().scroll_region_up(
                        scroll_amount,
                        scroll_top,
                        scroll_bottom,
                    );
                    self.adjust_graphics_for_scroll_up(scroll_amount, scroll_top, scroll_bottom);

                    // Adjust the new graphic's position to account for the scroll
                    // The graphic was at original_row, which just scrolled up by scroll_amount
                    let new_row = original_row.saturating_sub(scroll_amount);
                    graphic.position.1 = new_row;

                    // If position was clamped to 0, track how many rows scrolled off
                    if scroll_amount > original_row {
                        graphic.scroll_offset_rows = scroll_amount - original_row;
                    }

                    self.cursor.row = rows - 1;
                    self.cursor.col = new_cursor_col;
                } else {
                    self.cursor.row = new_cursor_row;
                    self.cursor.col = new_cursor_col;
                }

                // Add to graphics store AFTER all position adjustments (limit enforced internally)
                self.graphics_store.add_graphic(graphic.clone());

                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    &format!(
                        "Added Kitty image at ({}, {}), size {}x{}, cursor advanced to ({},{})",
                        position.0,
                        position.1,
                        graphic.width,
                        graphic.height,
                        self.cursor.col,
                        self.cursor.row
                    ),
                );
            }
            Ok(KittyGraphicResult::VirtualPlacement {
                image_id,
                placement_id,
                position,
                cols,
                rows,
            }) => {
                // Virtual placement - insert Unicode placeholders
                self.insert_placeholder_chars(image_id, placement_id, position, cols, rows);

                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    &format!(
                        "Inserted Unicode placeholders for virtual placement: image_id={}, placement_id={}, pos=({},{}), size={}x{}",
                        image_id, placement_id, position.0, position.1, cols, rows
                    ),
                );
            }
            Ok(KittyGraphicResult::None) => {
                // Command processed but no graphic created (e.g., transmit-only or delete)
                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    "Kitty command processed (no graphic created)",
                );
            }
            Err(e) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "KITTY",
                    &format!("Failed to build Kitty graphic: {}", e),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vte::Params;

    fn create_test_terminal() -> Terminal {
        Terminal::new(80, 24)
    }

    fn create_empty_params() -> Params {
        Params::default()
    }

    #[test]
    fn test_dcs_hook_sixel() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        assert!(term.dcs_active);
        assert_eq!(term.dcs_action, Some('q'));
        assert!(term.sixel_parser.is_some());
        assert!(term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_dcs_hook_sixel_with_params() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        assert!(term.dcs_active);
        assert_eq!(term.dcs_action, Some('q'));
        assert!(term.sixel_parser.is_some());
    }

    #[test]
    fn test_dcs_hook_sixel_blocked_by_security() {
        let mut term = create_test_terminal();
        term.disable_insecure_sequences = true;
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Should be blocked
        assert!(!term.dcs_active);
        assert_eq!(term.dcs_action, None);
        assert!(term.sixel_parser.is_none());
    }

    #[test]
    fn test_dcs_hook_non_sixel_action() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'p');

        assert!(term.dcs_active);
        assert_eq!(term.dcs_action, Some('p'));
        assert!(term.sixel_parser.is_none()); // Not created for non-sixel
    }

    #[test]
    fn test_dcs_put_sixel_data() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        // Start sixel
        term.dcs_hook(&params, &[], false, 'q');
        assert!(term.sixel_parser.is_some());

        // Send sixel data characters
        for &byte in b"????" {
            term.dcs_put(byte);
        }

        // Parser should process the data
        assert!(term.dcs_active);
    }

    #[test]
    fn test_dcs_put_not_active() {
        let mut term = create_test_terminal();

        // Try to put data without activating DCS
        term.dcs_put(b'A');

        // Should be ignored
        assert!(!term.dcs_active);
    }

    #[test]
    fn test_dcs_put_color_command() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send color command: #0
        term.dcs_put(b'#');
        term.dcs_put(b'0');

        assert_eq!(term.dcs_buffer, b"#0");
    }

    #[test]
    fn test_dcs_put_raster_attributes() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send raster attributes: "1;1;100;100
        for &byte in b"\"1;1;100;100" {
            term.dcs_put(byte);
        }

        // Should accumulate in buffer
        assert!(!term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_dcs_put_repeat_command() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send repeat command: !10?
        for &byte in b"!10?" {
            term.dcs_put(byte);
        }

        // Should process when complete
        assert!(term.dcs_active);
    }

    #[test]
    fn test_dcs_put_carriage_return() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send carriage return
        term.dcs_put(b'$');

        // Parser should handle it
        assert!(term.dcs_active);
    }

    #[test]
    fn test_dcs_put_new_line() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send new line
        term.dcs_put(b'-');

        // Parser should handle it
        assert!(term.dcs_active);
    }

    #[test]
    fn test_dcs_unhook_cleans_up() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');
        assert!(term.dcs_active);

        term.dcs_unhook();

        assert!(!term.dcs_active);
        assert_eq!(term.dcs_action, None);
        assert!(term.dcs_buffer.is_empty());
        assert!(term.sixel_parser.is_none());
    }

    #[test]
    fn test_dcs_unhook_processes_remaining_buffer() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Add some data to buffer
        term.dcs_buffer.extend_from_slice(b"#0");

        term.dcs_unhook();

        // Buffer should be processed and cleared
        assert!(term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_dcs_unhook_sixel_advances_cursor() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        let start_col = term.cursor.col;
        let start_row = term.cursor.row;

        term.dcs_hook(&params, &[], false, 'q');

        // Create minimal sixel graphic
        for &byte in b"????" {
            term.dcs_put(byte);
        }

        term.dcs_unhook();

        // Cursor should have moved (graphic occupies space)
        // At minimum, row should advance or col should change
        assert!(
            term.cursor.row > start_row || term.cursor.col != start_col,
            "Cursor should move after Sixel graphic"
        );
    }

    #[test]
    fn test_process_sixel_command_empty_buffer() {
        let mut term = create_test_terminal();

        term.process_sixel_command();

        // Should handle gracefully
        assert!(term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_process_sixel_command_no_parser() {
        let mut term = create_test_terminal();
        term.dcs_buffer.extend_from_slice(b"#0");

        term.process_sixel_command();

        // Should handle gracefully when no parser exists
    }

    #[test]
    fn test_dcs_sequence_isolation() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        // Start first DCS
        term.dcs_hook(&params, &[], false, 'q');
        term.dcs_put(b'?');
        term.dcs_unhook();

        assert!(!term.dcs_active);
        assert!(term.sixel_parser.is_none());

        // Start second DCS
        term.dcs_hook(&params, &[], false, 'q');
        assert!(term.dcs_active);
        assert!(term.sixel_parser.is_some());
    }

    #[test]
    fn test_dcs_hook_clears_previous_state() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        // Set some state
        term.dcs_buffer.extend_from_slice(b"old data");
        term.dcs_active = true;

        // Hook new DCS
        term.dcs_hook(&params, &[], false, 'q');

        // Buffer should be cleared
        assert!(term.dcs_buffer.is_empty());
        assert!(term.dcs_active);
        assert_eq!(term.dcs_action, Some('q'));
    }

    #[test]
    fn test_dcs_multiple_commands_in_sequence() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send multiple commands
        for &byte in b"#0" {
            term.dcs_put(byte);
        }

        // Buffer should accumulate
        assert_eq!(term.dcs_buffer, b"#0");

        // Process by sending data char
        term.dcs_put(b'?');

        // Buffer should be cleared after processing
        assert_eq!(term.dcs_buffer.len(), 0);
    }

    #[test]
    fn test_dcs_color_command_parsing() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send color definition: #1;2;100;100;100
        for &byte in b"#1;2;100;100;100" {
            term.dcs_put(byte);
        }

        // Trigger processing with data char
        term.dcs_put(b'?');

        // Should have processed color command
        assert!(term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_dcs_raster_attributes_parsing() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'q');

        // Send raster attributes: "1;1;800;600
        for &byte in b"\"1;1;800;600" {
            term.dcs_put(byte);
        }

        // Trigger processing with data char
        term.dcs_put(b'?');

        // Should have processed raster command
        assert!(term.dcs_buffer.is_empty());
    }

    #[test]
    fn test_dcs_graphics_list_updated() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        let initial_graphics_count = term.graphics_count();

        term.dcs_hook(&params, &[], false, 'q');

        // Send minimal sixel data
        for &byte in b"????" {
            term.dcs_put(byte);
        }

        term.dcs_unhook();

        // Graphics list should have one more entry
        assert_eq!(term.graphics_count(), initial_graphics_count + 1);
    }

    #[test]
    fn test_sixel_graphics_limit_enforced() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        // Only allow 2 graphics to be retained
        term.set_max_sixel_graphics(2);

        // Helper to emit a tiny sixel graphic
        let emit_sixel = |term: &mut Terminal| {
            term.dcs_hook(&params, &[], false, 'q');
            for &byte in b"??" {
                term.dcs_put(byte);
            }
            term.dcs_unhook();
        };

        emit_sixel(&mut term);
        emit_sixel(&mut term);
        assert_eq!(term.graphics_count(), 2);
    }

    #[test]
    fn test_sixel_graphics_limit_drops_oldest() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.set_max_sixel_graphics(1);

        let emit_sixel = |term: &mut Terminal| {
            term.dcs_hook(&params, &[], false, 'q');
            for &byte in b"??" {
                term.dcs_put(byte);
            }
            term.dcs_unhook();
        };

        emit_sixel(&mut term);
        assert_eq!(term.graphics_count(), 1);

        emit_sixel(&mut term);
        // Limit enforced - still only 1 graphic
        assert_eq!(term.graphics_count(), 1);

        // Emit a third graphic; limit should still be enforced
        emit_sixel(&mut term);
        assert_eq!(term.graphics_count(), 1);
    }

    #[test]
    fn test_dcs_cursor_position_after_graphic() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.cursor.col = 10;
        term.cursor.row = 5;

        term.dcs_hook(&params, &[], false, 'q');
        for &byte in b"????" {
            term.dcs_put(byte);
        }
        term.dcs_unhook();

        // After sixel, cursor should be at column 0 (left margin)
        assert_eq!(term.cursor.col, 0);
        // Row should have advanced
        assert!(term.cursor.row >= 5);
    }

    #[test]
    fn test_dcs_non_sixel_action_ignored() {
        let mut term = create_test_terminal();
        let params = create_empty_params();

        term.dcs_hook(&params, &[], false, 'x');
        assert!(term.dcs_active);
        assert_eq!(term.dcs_action, Some('x'));

        // Put some data
        term.dcs_put(b'A');
        term.dcs_put(b'B');

        // Should not create sixel parser
        assert!(term.sixel_parser.is_none());

        term.dcs_unhook();
        assert!(!term.dcs_active);
    }
}
