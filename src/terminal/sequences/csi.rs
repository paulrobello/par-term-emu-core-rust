//! CSI (Control Sequence Introducer) sequence handling
//!
//! Handles CSI sequences for terminal control, including:
//! - Cursor movement and positioning
//! - Text editing (insert/delete lines/characters)
//! - Display attributes (SGR - colors, bold, italic, etc.)
//! - Scroll regions and margins
//! - Tab stops
//! - Mouse tracking modes
//! - Device status reports (DSR)
//! - Terminal modes (SM/RM)
//! - Keyboard protocol (Kitty)

use crate::cell::{Cell, CellFlags};
use crate::color::{Color, NamedColor};
use crate::debug;
use crate::mouse::{MouseEncoding, MouseMode};
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

        let (cols, rows) = self.size();

        // Check for private mode sequences (intermediates contains '?')
        let private = intermediates.contains(&b'?');

        match action {
            'A' => {
                // Cursor up (CUU)
                // VT spec: CSI n A moves cursor up n rows (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_up(n);
                self.pending_wrap = false;
            }
            'B' => {
                // Cursor down (CUD)
                // VT spec: CSI n B moves cursor down n rows (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_down(n, rows.saturating_sub(1));
                self.pending_wrap = false;
            }
            'C' => {
                // Cursor forward (CUF)
                // VT spec: CSI n C moves cursor right n columns (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_right(n, cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            'D' => {
                // Cursor back (CUB)
                // VT spec: CSI n D moves cursor left n columns (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.move_left(n);
                self.pending_wrap = false;
            }
            'H' | 'f' => {
                // Cursor position (CUP/HVP) - respects origin mode
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;

                // Convert to 0-indexed
                let col = col.saturating_sub(1);
                let row = row.saturating_sub(1);

                if self.origin_mode {
                    // Origin mode: coordinates relative to scrolling region
                    let region_height = self.scroll_region_bottom - self.scroll_region_top + 1;
                    let actual_row = self.scroll_region_top + row.min(region_height - 1);
                    let actual_col = col.min(cols.saturating_sub(1));
                    self.cursor.goto(actual_col, actual_row);
                    self.pending_wrap = false;
                } else {
                    // Normal mode: absolute coordinates
                    self.cursor.goto(
                        col.min(cols.saturating_sub(1)),
                        row.min(rows.saturating_sub(1)),
                    );
                    self.pending_wrap = false;
                }
            }
            'J' => {
                // Erase in display
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                match n {
                    0 => self
                        .active_grid_mut()
                        .clear_screen_below(cursor_col, cursor_row),
                    1 => self
                        .active_grid_mut()
                        .clear_screen_above(cursor_col, cursor_row),
                    2 | 3 => {
                        // Clear entire screen - also clear graphics
                        self.active_grid_mut().clear();
                        self.graphics.clear();
                        debug::log(
                            debug::DebugLevel::Debug,
                            "CLEAR",
                            "Cleared screen and graphics (ED 2/3)",
                        );
                    }
                    _ => {}
                }
            }
            'K' => {
                // Erase in line
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                match n {
                    0 => self
                        .active_grid_mut()
                        .clear_line_right(cursor_col, cursor_row),
                    1 => self
                        .active_grid_mut()
                        .clear_line_left(cursor_col, cursor_row),
                    2 => self.active_grid_mut().clear_row(cursor_row),
                    _ => {}
                }
            }
            'S' => {
                // Scroll up (SU)
                // VT spec: CSI n S scrolls up n lines (default 1) within the scroll region
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                // Scroll within the current scroll region, not the entire screen
                let top = self.scroll_region_top;
                let bottom = self.scroll_region_bottom;
                self.active_grid_mut().scroll_region_up(n, top, bottom);
                // Adjust graphics for scroll within region
                self.adjust_graphics_for_scroll_up(n, top, bottom);
            }
            'T' => {
                // Scroll down (SD)
                // VT spec: CSI n T scrolls down n lines (default 1) within the scroll region
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                // Scroll within the current scroll region, not the entire screen
                let top = self.scroll_region_top;
                let bottom = self.scroll_region_bottom;
                self.active_grid_mut().scroll_region_down(n, top, bottom);
                // Adjust graphics for scroll within region
                self.adjust_graphics_for_scroll_down(n, top, bottom);
            }
            'm' => {
                // SGR - Select Graphic Rendition
                if params.is_empty() {
                    self.flags = CellFlags::default();
                    // Reset to terminal defaults (OSC 10/11 configurable)
                    self.fg = self.default_fg;
                    self.bg = self.default_bg;
                    self.underline_color = None;
                } else {
                    let mut iter = params.iter();
                    while let Some(param_slice) = iter.next() {
                        let param = param_slice.first().copied().unwrap_or(0);
                        match param {
                            0 => {
                                self.flags = CellFlags::default();
                                // Reset to terminal defaults (OSC 10/11 configurable)
                                self.fg = self.default_fg;
                                self.bg = self.default_bg;
                                self.underline_color = None;
                            }
                            1 => self.flags.set_bold(true),
                            2 => self.flags.set_dim(true),
                            3 => self.flags.set_italic(true),
                            4 => {
                                // Underline - check for sub-parameter (SGR 4:x)
                                if let Some(&style_code) = param_slice.get(1) {
                                    // SGR 4:x - underline with style
                                    use crate::cell::UnderlineStyle;
                                    self.flags.set_underline(true);
                                    self.flags.underline_style = match style_code {
                                        0 => UnderlineStyle::None,
                                        1 => UnderlineStyle::Straight,
                                        2 => UnderlineStyle::Double,
                                        3 => UnderlineStyle::Curly,
                                        4 => UnderlineStyle::Dotted,
                                        5 => UnderlineStyle::Dashed,
                                        _ => UnderlineStyle::Straight, // Unknown style, default to straight
                                    };
                                    // If style is None, also disable underline flag
                                    if self.flags.underline_style == UnderlineStyle::None {
                                        self.flags.set_underline(false);
                                    }
                                } else {
                                    // Plain SGR 4 - straight underline (legacy)
                                    use crate::cell::UnderlineStyle;
                                    self.flags.set_underline(true);
                                    self.flags.underline_style = UnderlineStyle::Straight;
                                }
                            }
                            5 => self.flags.set_blink(true),
                            7 => self.flags.set_reverse(true),
                            8 => self.flags.set_hidden(true),
                            9 => self.flags.set_strikethrough(true),
                            22 => {
                                self.flags.set_bold(false);
                                self.flags.set_dim(false);
                            }
                            23 => self.flags.set_italic(false),
                            24 => {
                                // Disable underline
                                self.flags.set_underline(false);
                                self.flags.underline_style = crate::cell::UnderlineStyle::None;
                            }
                            25 => self.flags.set_blink(false),
                            27 => self.flags.set_reverse(false),
                            28 => self.flags.set_hidden(false),
                            29 => self.flags.set_strikethrough(false),
                            53 => self.flags.set_overline(true),
                            55 => self.flags.set_overline(false),
                            30..=37 => {
                                self.fg = Color::Named(NamedColor::from_u8((param - 30) as u8))
                            }
                            38 => {
                                // Extended foreground color
                                if let Some(next) = iter.next() {
                                    if let Some(&mode) = next.first() {
                                        match mode {
                                            2 => {
                                                // RGB (true color)
                                                let r = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let g = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let b = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                self.fg = Color::Rgb(r, g, b);
                                            }
                                            5 => {
                                                // 256 color
                                                if let Some(idx) =
                                                    iter.next().and_then(|p| p.first())
                                                {
                                                    self.fg = Color::from_ansi_code(*idx as u8);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            39 => self.fg = self.default_fg,
                            40..=47 => {
                                self.bg = Color::Named(NamedColor::from_u8((param - 40) as u8))
                            }
                            48 => {
                                // Extended background color
                                if let Some(next) = iter.next() {
                                    if let Some(&mode) = next.first() {
                                        match mode {
                                            2 => {
                                                // RGB (true color)
                                                let r = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let g = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let b = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                self.bg = Color::Rgb(r, g, b);
                                            }
                                            5 => {
                                                // 256 color
                                                if let Some(idx) =
                                                    iter.next().and_then(|p| p.first())
                                                {
                                                    self.bg = Color::from_ansi_code(*idx as u8);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            49 => self.bg = self.default_bg,
                            58 => {
                                // Extended underline color (SGR 58)
                                if let Some(next) = iter.next() {
                                    if let Some(&mode) = next.first() {
                                        match mode {
                                            2 => {
                                                // RGB (true color)
                                                let r = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let g = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                let b = iter
                                                    .next()
                                                    .and_then(|p| p.first())
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as u8;
                                                self.underline_color = Some(Color::Rgb(r, g, b));
                                            }
                                            5 => {
                                                // 256 color
                                                if let Some(idx) =
                                                    iter.next().and_then(|p| p.first())
                                                {
                                                    self.underline_color =
                                                        Some(Color::from_ansi_code(*idx as u8));
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            59 => self.underline_color = None, // Reset underline color (use foreground)
                            90..=97 => {
                                self.fg = Color::Named(NamedColor::from_u8((param - 90 + 8) as u8))
                            }
                            100..=107 => {
                                self.bg = Color::Named(NamedColor::from_u8((param - 100 + 8) as u8))
                            }
                            _ => {}
                        }
                    }
                }
            }
            'h' => {
                // Set mode
                if private {
                    // Private modes (DEC)
                    for param in params.iter() {
                        if let Some(&n) = param.first() {
                            match n {
                                1 => self.application_cursor = true, // Application cursor keys (DECCKM)
                                5 => self.reverse_video = true,      // Reverse video (DECSCNM)
                                6 => self.origin_mode = true,        // Origin mode (DECOM)
                                69 => {
                                    // DECLRMM - enable left/right margins
                                    self.use_lr_margins = true;
                                    // Ensure margins are sane
                                    let (cols, _rows) = self.size();
                                    if self.left_margin > self.right_margin {
                                        self.left_margin = 0;
                                        self.right_margin = cols.saturating_sub(1);
                                    }
                                }
                                7 => self.auto_wrap = true, // Auto wrap mode (DECAWM)
                                25 => self.cursor.show(),   // Show cursor (DECTCEM)
                                47 | 1047 => self.use_alt_screen(), // Alternate screen
                                1048 => {
                                    // Save cursor (often used with alt screen)
                                    self.saved_cursor = Some(self.cursor);
                                    self.saved_fg = self.fg;
                                    self.saved_bg = self.bg;
                                    self.saved_underline_color = self.underline_color;
                                    self.saved_flags = self.flags;
                                }
                                1049 => {
                                    // Save cursor and use alt screen
                                    self.saved_cursor = Some(self.cursor);
                                    self.saved_fg = self.fg;
                                    self.saved_bg = self.bg;
                                    self.saved_underline_color = self.underline_color;
                                    self.saved_flags = self.flags;
                                    self.use_alt_screen();
                                }
                                1000 => self.mouse_mode = MouseMode::Normal, // X11 mouse
                                1002 => self.mouse_mode = MouseMode::ButtonEvent, // Button event mode
                                1003 => self.mouse_mode = MouseMode::AnyEvent,    // Any event mode
                                1004 => self.focus_tracking = true,               // Focus tracking
                                1005 => self.mouse_encoding = MouseEncoding::Utf8, // UTF-8 mouse
                                1006 => self.mouse_encoding = MouseEncoding::Sgr, // SGR mouse
                                1015 => self.mouse_encoding = MouseEncoding::Urxvt, // URXVT mouse
                                2004 => self.bracketed_paste = true,              // Bracketed paste
                                2026 => self.synchronized_updates = true, // Synchronized updates
                                _ => {}
                            }
                        }
                    }
                } else {
                    // Standard modes
                    for param in params.iter() {
                        if let Some(&n) = param.first() {
                            match n {
                                4 => self.insert_mode = true,              // Insert mode (IRM)
                                20 => self.line_feed_new_line_mode = true, // Line feed/new line mode (LNM)
                                _ => {}
                            }
                        }
                    }
                }
            }
            'l' => {
                // Reset mode
                if private {
                    // Private modes (DEC)
                    for param in params.iter() {
                        if let Some(&n) = param.first() {
                            match n {
                                1 => self.application_cursor = false, // Normal cursor keys
                                5 => self.reverse_video = false,      // Normal video (DECSCNM)
                                6 => self.origin_mode = false,        // Normal addressing mode
                                69 => {
                                    // DECLRMM off
                                    self.use_lr_margins = false;
                                    let (cols, _rows) = self.size();
                                    self.left_margin = 0;
                                    self.right_margin = cols.saturating_sub(1);
                                }
                                7 => self.auto_wrap = false, // Disable auto wrap (DECAWM)
                                25 => self.cursor.hide(),    // Hide cursor (DECTCEM)
                                47 | 1047 => self.use_primary_screen(), // Primary screen
                                1048 => {
                                    // Restore cursor
                                    if let Some(saved) = self.saved_cursor {
                                        self.cursor = saved;
                                        self.fg = self.saved_fg;
                                        self.bg = self.saved_bg;
                                        self.underline_color = self.saved_underline_color;
                                        self.flags = self.saved_flags;
                                    }
                                }
                                1049 => {
                                    // Use primary screen and restore cursor
                                    self.use_primary_screen();
                                    if let Some(saved) = self.saved_cursor {
                                        self.cursor = saved;
                                        self.fg = self.saved_fg;
                                        self.bg = self.saved_bg;
                                        self.underline_color = self.saved_underline_color;
                                        self.flags = self.saved_flags;
                                    }
                                }
                                1000 | 1002 | 1003 => self.mouse_mode = MouseMode::Off, // Disable mouse
                                1004 => self.focus_tracking = false, // Focus tracking off
                                1005 | 1006 | 1015 => self.mouse_encoding = MouseEncoding::Default, // Default encoding
                                2004 => self.bracketed_paste = false, // Bracketed paste off
                                2026 => {
                                    // Synchronized updates off - flush buffer first
                                    self.flush_synchronized_updates();
                                    self.synchronized_updates = false;
                                }
                                _ => {}
                            }
                        }
                    }
                } else {
                    // Standard modes
                    for param in params.iter() {
                        if let Some(&n) = param.first() {
                            match n {
                                4 => self.insert_mode = false,              // Replace mode (IRM off)
                                20 => self.line_feed_new_line_mode = false, // Line feed mode (LNM off)
                                _ => {}
                            }
                        }
                    }
                }
            }
            's' => {
                // Ambiguous: Save Cursor (ANSI.SYS) or DECSLRM depending on DECLRMM
                if self.use_lr_margins {
                    // Set left/right margins (DECSLRM)
                    // CSI Pl ; Pr s
                    let mut iter = params.iter();
                    let left_param = iter.next().and_then(|p| p.first()).copied();
                    let right_param = iter.next().and_then(|p| p.first()).copied();
                    let (cols, _rows) = self.size();

                    let mut left = left_param.unwrap_or(1) as isize;
                    if left <= 0 {
                        left = 1;
                    }
                    let mut right = right_param.unwrap_or(cols as u16) as isize;
                    if right <= 0 {
                        right = cols as isize;
                    }

                    // Convert to 0-indexed and clamp
                    let mut left0 = (left - 1).max(0) as usize;
                    let mut right0 = (right - 1).max(0) as usize;
                    let max_col = cols.saturating_sub(1);
                    if right0 > max_col {
                        right0 = max_col;
                    }
                    if right0 <= left0 {
                        // Invalid or degenerate → full width
                        left0 = 0;
                        right0 = max_col;
                    }
                    self.left_margin = left0;
                    self.right_margin = right0;
                } else {
                    // Save cursor position (ANSI.SYS variant)
                    self.saved_cursor = Some(self.cursor);
                    self.saved_fg = self.fg;
                    self.saved_bg = self.bg;
                    self.saved_underline_color = self.underline_color;
                    self.saved_flags = self.flags;
                }
            }
            'u' => {
                // Kitty keyboard protocol uses CSI with intermediates
                // CSI = flags ; mode u → Set keyboard mode
                // CSI ? u → Query keyboard capabilities
                // CSI > flags u → Push flags to stack
                // CSI < number u → Pop from stack
                // CSI u (no intermediates) → Restore cursor position (ANSI.SYS variant)

                if intermediates.contains(&b'=') {
                    // CSI = flags ; mode u → Set keyboard mode
                    let flags = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);
                    let mode = params
                        .iter()
                        .nth(1)
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    // mode 1 = set, 0 = unset, 2 = lock, 3 = report
                    match mode {
                        0 => self.keyboard_flags = 0,     // Disable/unset
                        1 => self.keyboard_flags = flags, // Set flags
                        2 => self.keyboard_flags = flags, // Lock (same as set for now)
                        3 => {
                            // Report current flags
                            let response = format!("\x1b[?{}u", self.keyboard_flags);
                            self.push_response(response.as_bytes());
                        }
                        _ => {}
                    }
                } else if intermediates.contains(&b'?') {
                    // CSI ? u → Query keyboard capabilities
                    // Response: CSI ? flags u (current flags)
                    let response = format!("\x1b[?{}u", self.keyboard_flags);
                    self.push_response(response.as_bytes());
                } else if intermediates.contains(&b'>') {
                    // CSI > flags u → Push flags to stack
                    let flags = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    // Push to appropriate stack based on active screen
                    if self.alt_screen_active {
                        self.keyboard_stack_alt.push(self.keyboard_flags);
                    } else {
                        self.keyboard_stack.push(self.keyboard_flags);
                    }
                    self.keyboard_flags = flags;
                } else if intermediates.contains(&b'<') {
                    // CSI < number u → Pop from stack
                    let count = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1) as usize;

                    // Pop from appropriate stack based on active screen
                    let stack = if self.alt_screen_active {
                        &mut self.keyboard_stack_alt
                    } else {
                        &mut self.keyboard_stack
                    };

                    for _ in 0..count {
                        if let Some(flags) = stack.pop() {
                            self.keyboard_flags = flags;
                        }
                    }
                } else {
                    // No intermediates → Restore cursor position (ANSI.SYS variant)
                    if let Some(saved) = self.saved_cursor {
                        self.cursor = saved;
                        self.fg = self.saved_fg;
                        self.bg = self.saved_bg;
                        self.underline_color = self.saved_underline_color;
                        self.flags = self.saved_flags;
                    }
                }
            }
            'L' => {
                // Insert lines (IL) - VT220
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let cursor_row = self.cursor.row;
                let scroll_top = self.scroll_region_top;
                let scroll_bottom = self.scroll_region_bottom;
                self.active_grid_mut()
                    .insert_lines(cursor_row, n, scroll_top, scroll_bottom);
            }
            'M' => {
                // Delete lines (DL) - VT220
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let cursor_row = self.cursor.row;
                let scroll_top = self.scroll_region_top;
                let scroll_bottom = self.scroll_region_bottom;
                self.active_grid_mut()
                    .delete_lines(cursor_row, n, scroll_top, scroll_bottom);
            }
            '@' => {
                // Insert characters (ICH) - VT220
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                self.active_grid_mut()
                    .insert_chars(cursor_col, cursor_row, n);
            }
            'P' => {
                if intermediates.contains(&b'#') {
                    // XTPUSHCOLORS - Push current colors onto stack
                    // CSI # P or CSI # <params> P
                    // Push current fg, bg, and underline colors
                    self.color_stack
                        .push((self.fg, self.bg, self.underline_color));
                } else {
                    // Delete characters (DCH) - VT220
                    let n = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(1)
                        .max(1) as usize;
                    let cursor_col = self.cursor.col;
                    let cursor_row = self.cursor.row;
                    self.active_grid_mut()
                        .delete_chars(cursor_col, cursor_row, n);
                }
            }
            'Q' => {
                if intermediates.contains(&b'#') {
                    // XTPOPCOLORS - Pop colors from stack
                    // CSI # Q or CSI # <params> Q
                    // Pop fg, bg, and underline colors from stack
                    if let Some((fg, bg, underline_color)) = self.color_stack.pop() {
                        self.fg = fg;
                        self.bg = bg;
                        self.underline_color = underline_color;
                    }
                }
                // Note: 'Q' without '#' intermediate doesn't have a standard meaning
            }
            'X' => {
                // Erase characters (ECH) - VT220
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1)
                    .max(1) as usize;
                let cursor_col = self.cursor.col;
                let cursor_row = self.cursor.row;
                self.active_grid_mut()
                    .erase_chars(cursor_col, cursor_row, n);
            }
            'q' => {
                // Check for intermediate byte SP (0x20) - DECSCUSR (Set Cursor Style)
                if intermediates.contains(&b' ') {
                    // DECSCUSR - Set cursor style
                    // CSI SP q - Default (blinking block)
                    // CSI <n> SP q - Set specific style
                    use crate::cursor::CursorStyle;
                    let style_code = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    let style = match style_code {
                        0 | 1 => CursorStyle::BlinkingBlock,
                        2 => CursorStyle::SteadyBlock,
                        3 => CursorStyle::BlinkingUnderline,
                        4 => CursorStyle::SteadyUnderline,
                        5 => CursorStyle::BlinkingBar,
                        6 => CursorStyle::SteadyBar,
                        _ => CursorStyle::BlinkingBlock, // Unknown code, default to blinking block
                    };

                    self.cursor.set_style(style);
                } else if private && intermediates.contains(&b'"') {
                    // DECSCA - Select Character Protection Attribute
                    // CSI ? Ps " q
                    // Ps = 0 or 2: Characters are not protected (DECSED and DECSEL can erase)
                    // Ps = 1: Characters are protected (DECSED and DECSEL cannot erase)
                    let protection_mode = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    match protection_mode {
                        1 => {
                            // Enable character protection
                            self.char_protected = true;
                        }
                        0 | 2 => {
                            // Disable character protection
                            self.char_protected = false;
                        }
                        _ => {} // Unknown mode, ignore
                    }
                }
                // Note: CSI q without intermediates is not standard
            }
            'r' => {
                // Check for DECCARA first (VT420)
                if intermediates.contains(&b'$') {
                    // DECCARA - Change Attributes in Rectangular Area (VT420)
                    // CSI Pt ; Pl ; Pb ; Pr ; Ps $ r
                    // Set specified attributes in rectangle
                    let params_list: Vec<u16> =
                        params.iter().filter_map(|p| p.first().copied()).collect();

                    if params_list.len() >= 4 {
                        // Convert from 1-indexed VT coordinates to 0-indexed
                        let top = (params_list[0] as usize).saturating_sub(1);
                        let left = (params_list[1] as usize).saturating_sub(1);
                        let bottom = (params_list[2] as usize).saturating_sub(1);
                        let right = (params_list[3] as usize).saturating_sub(1);

                        // Remaining parameters are attributes to set
                        let attributes: Vec<u16> = if params_list.len() > 4 {
                            params_list[4..].to_vec()
                        } else {
                            vec![0] // Default: reset all attributes
                        };

                        self.active_grid_mut().change_attributes_in_rectangle(
                            top,
                            left,
                            bottom,
                            right,
                            &attributes,
                        );
                    }
                } else {
                    // Set scrolling region (DECSTBM)
                    let mut iter = params.iter();
                    let top_param = iter.next().and_then(|p| p.first()).copied();
                    let bottom_param = iter.next().and_then(|p| p.first()).copied();

                    // If no parameters, reset to full screen
                    if top_param.is_none() && bottom_param.is_none() {
                        let new_top = 0;
                        let new_bottom = rows.saturating_sub(1);
                        let changed = self.scroll_region_top != new_top
                            || self.scroll_region_bottom != new_bottom;
                        self.scroll_region_top = new_top;
                        self.scroll_region_bottom = new_bottom;
                        // Only move cursor if region actually changed
                        // This prevents disrupting tmux and other apps that repeatedly set the same region
                        if changed {
                            self.cursor.goto(0, 0);
                        }
                        return;
                    }

                    // DECSTBM defaults: missing or 0 → top=1, bottom=rows
                    // Many apps (tmux) rely on 0 being treated as default.
                    let mut top = top_param.unwrap_or(1) as usize;
                    if top == 0 {
                        top = 1;
                    }
                    let mut bottom = bottom_param.unwrap_or(rows as u16) as usize;
                    if bottom == 0 {
                        bottom = rows;
                    }

                    // Convert to 0-indexed
                    let top = top.saturating_sub(1);
                    let bottom = bottom.saturating_sub(1);

                    // Validate: top < bottom and both within screen bounds
                    if top < bottom && top < rows && bottom < rows {
                        // Check if region actually changed before moving cursor
                        let changed =
                            self.scroll_region_top != top || self.scroll_region_bottom != bottom;
                        self.scroll_region_top = top;
                        self.scroll_region_bottom = bottom;
                        // Move cursor to home position ONLY if region changed
                        // This prevents disrupting cursor positioning for apps like tmux
                        // that frequently re-set the same scroll region
                        if changed {
                            if self.origin_mode {
                                self.cursor.goto(0, self.scroll_region_top);
                            } else {
                                self.cursor.goto(0, 0);
                            }
                        }
                    }
                    // Invalid region - ignore the command per VT spec
                } // end DECSTBM else block
            }
            'n' => {
                // Device Status Report (DSR)
                let param = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                match param {
                    5 => {
                        // Operating status report
                        // Response: ESC [ 0 n (terminal ready, no malfunction)
                        self.push_response(b"\x1b[0n");
                    }
                    6 => {
                        // Cursor position report (CPR)
                        // Response: ESC [ row ; col R (1-indexed)
                        let row = self.cursor.row + 1;
                        let col = self.cursor.col + 1;
                        let response = format!("\x1b[{};{}R", row, col);
                        self.push_response(response.as_bytes());
                    }
                    _ => {}
                }
            }
            'c' => {
                // Device Attributes (DA)
                // Check for secondary DA (intermediates contain '>')
                if intermediates.contains(&b'>') {
                    // Secondary DA: ESC [ > 0 c or ESC [ > c
                    // Response: ESC [ > 82 ; 10000 ; 0 c
                    // 82 = 'P' for par-term-emu, version 10000 (1.0000)
                    self.push_response(b"\x1b[>82;10000;0c");
                } else {
                    // Primary DA: ESC [ c or ESC [ 0 c
                    // Response: ESC [ ? 1 ; 2 c (VT100 with Advanced Video Option)
                    // Or: ESC [ ? 62 ; 1 ; 4 ; 6 ; 9 ; 15 ; 22 c (VT220 with features)
                    // Using VT220 response for better compatibility:
                    // 62 = VT220
                    // 1 = 132 columns
                    // 4 = Sixel graphics
                    // 6 = Selective erase
                    // 9 = National replacement character sets
                    // 15 = Technical character set
                    // 22 = ANSI color
                    self.push_response(b"\x1b[?62;1;4;6;9;15;22c");
                }
            }
            'p' => {
                // Check for DECRQM (DEC Private Mode Status Request)
                // Sequence: CSI ? {mode} $ p
                if private && intermediates.contains(&b'$') {
                    // DEC Private Mode Status Request
                    let mode = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    // Response: CSI ? {mode} ; {state} $ y
                    // state: 0 = not recognized, 1 = set, 2 = reset, 3 = permanently set, 4 = permanently reset
                    let state = match mode {
                        1 => {
                            if self.application_cursor {
                                1
                            } else {
                                2
                            }
                        }
                        6 => {
                            if self.origin_mode {
                                1
                            } else {
                                2
                            }
                        }
                        7 => {
                            if self.auto_wrap {
                                1
                            } else {
                                2
                            }
                        }
                        25 => {
                            if self.cursor.visible {
                                1
                            } else {
                                2
                            }
                        }
                        47 | 1047 => {
                            if self.alt_screen_active {
                                1
                            } else {
                                2
                            }
                        }
                        1048 => {
                            if self.saved_cursor.is_some() {
                                1
                            } else {
                                2
                            }
                        }
                        1049 => {
                            if self.alt_screen_active {
                                1
                            } else {
                                2
                            }
                        }
                        1000 => {
                            if matches!(self.mouse_mode, MouseMode::Normal) {
                                1
                            } else {
                                2
                            }
                        }
                        1002 => {
                            if matches!(self.mouse_mode, MouseMode::ButtonEvent) {
                                1
                            } else {
                                2
                            }
                        }
                        1003 => {
                            if matches!(self.mouse_mode, MouseMode::AnyEvent) {
                                1
                            } else {
                                2
                            }
                        }
                        1004 => {
                            if self.focus_tracking {
                                1
                            } else {
                                2
                            }
                        }
                        1005 => {
                            if matches!(self.mouse_encoding, MouseEncoding::Utf8) {
                                1
                            } else {
                                2
                            }
                        }
                        1006 => {
                            if matches!(self.mouse_encoding, MouseEncoding::Sgr) {
                                1
                            } else {
                                2
                            }
                        }
                        1015 => {
                            if matches!(self.mouse_encoding, MouseEncoding::Urxvt) {
                                1
                            } else {
                                2
                            }
                        }
                        2004 => {
                            if self.bracketed_paste {
                                1
                            } else {
                                2
                            }
                        }
                        2026 => {
                            if self.synchronized_updates {
                                1
                            } else {
                                2
                            }
                        }
                        _ => 0, // Not recognized
                    };

                    let response = format!("\x1b[?{};{}$y", mode, state);
                    self.push_response(response.as_bytes());
                }
            }
            'x' => {
                if intermediates.contains(&b'*') {
                    // DECSACE - Select Attribute Change Extent (VT420)
                    // CSI Ps * x
                    // Ps = 0 or 1: stream mode, Ps = 2: rectangle mode (default)
                    let mode = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(2);

                    self.attribute_change_extent = if mode <= 1 { 1 } else { 2 };
                } else if intermediates.contains(&b'$') {
                    // DECFRA - Fill Rectangular Area (VT420)
                    // CSI Pc ; Pt ; Pl ; Pb ; Pr $ x
                    // Fill rectangle with character Pc
                    let params_list: Vec<usize> = params
                        .iter()
                        .filter_map(|p| p.first().copied().map(|v| v as usize))
                        .collect();

                    if params_list.len() >= 5 {
                        // First param is the fill character (ASCII value)
                        let fill_char = if params_list[0] > 0 && params_list[0] <= 127 {
                            params_list[0] as u8 as char
                        } else {
                            ' ' // Default to space
                        };

                        let fill_char_width =
                            unicode_width::UnicodeWidthChar::width(fill_char).unwrap_or(1) as u8;

                        // Create fill cell with current attributes
                        let fill_cell = Cell {
                            c: fill_char,
                            fg: self.fg,
                            bg: self.bg,
                            underline_color: self.underline_color,
                            flags: self.flags,
                            width: fill_char_width,
                        };

                        // Convert from 1-indexed VT coordinates to 0-indexed
                        let top = params_list[1].saturating_sub(1);
                        let left = params_list[2].saturating_sub(1);
                        let bottom = params_list[3].saturating_sub(1);
                        let right = params_list[4].saturating_sub(1);

                        self.active_grid_mut()
                            .fill_rectangle(fill_cell, top, left, bottom, right);
                    }
                } else {
                    // Terminal Parameters (DECREQTPARM)
                    // Sequence: CSI {x} x where x is 0 or 1
                    let param = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    if param == 0 || param == 1 {
                        // Response: CSI {sol} ; 1 ; 1 ; 120 ; 120 ; 1 ; 0 x
                        // sol = 2 (solicited) if param=0, 3 (unsolicited) if param=1
                        // 1 = no parity
                        // 1 = 8 bits per char
                        // 120 = 9600 bps TX speed
                        // 120 = 9600 bps RX speed
                        // 1 = bit rate multiplier
                        // 0 = flags
                        let sol = if param == 0 { 2 } else { 3 };
                        let response = format!("\x1b[{};1;1;120;120;1;0x", sol);
                        self.push_response(response.as_bytes());
                    }
                }
            }
            'g' => {
                // Tab clear (TBC)
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                match n {
                    0 => {
                        // Clear tab stop at current column
                        if self.cursor.col < self.tab_stops.len() {
                            self.tab_stops[self.cursor.col] = false;
                        }
                    }
                    3 => {
                        // Clear all tab stops
                        for stop in &mut self.tab_stops {
                            *stop = false;
                        }
                    }
                    _ => {}
                }
            }
            'G' => {
                // Cursor horizontal absolute (CHA)
                let col = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                // Treat parameter 0 as 1 per VT spec
                let col = if col == 0 { 1 } else { col };
                self.cursor.col = col.saturating_sub(1).min(cols.saturating_sub(1));
            }
            'd' => {
                // Line position absolute (VPA)
                let row = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                // Treat parameter 0 as 1 per VT spec
                let row = if row == 0 { 1 } else { row };
                self.cursor.row = row.saturating_sub(1).min(rows.saturating_sub(1));
            }
            'E' => {
                // Cursor next line (CNL)
                // VT spec: CSI n E moves cursor down n lines and to column 1 (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.row = (self.cursor.row + n).min(rows.saturating_sub(1));
                self.cursor.col = 0;
            }
            'F' => {
                // Cursor previous line (CPL)
                // VT spec: CSI n F moves cursor up n lines and to column 1 (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                self.cursor.row = self.cursor.row.saturating_sub(n);
                self.cursor.col = 0;
            }
            'I' => {
                // Cursor forward tabulation (CHT)
                // VT spec: CSI n I moves forward n tab stops (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                for _ in 0..n {
                    let mut next_col = self.cursor.col + 1;
                    while next_col < cols {
                        if self.tab_stops.get(next_col).copied().unwrap_or(false) {
                            break;
                        }
                        next_col += 1;
                    }
                    self.cursor.col = next_col.min(cols - 1);
                }
            }
            'Z' => {
                // Cursor backward tabulation (CBT)
                // VT spec: CSI n Z moves backward n tab stops (default 1)
                // Note: param 0 should be treated as 1
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(1) as usize;
                let n = if n == 0 { 1 } else { n };
                for _ in 0..n {
                    if self.cursor.col > 0 {
                        let mut prev_col = self.cursor.col - 1;
                        loop {
                            if self.tab_stops.get(prev_col).copied().unwrap_or(false) {
                                break;
                            }
                            if prev_col == 0 {
                                break;
                            }
                            prev_col -= 1;
                        }
                        self.cursor.col = prev_col;
                    }
                }
            }
            'v' => {
                // DECCRA - Copy Rectangular Area (VT420)
                // CSI Pts ; Pls ; Pbs ; Prs ; Pps ; Ptd ; Pld ; Ppd $ v
                // Copy rectangle from (src_top, src_left) to (src_bottom, src_right)
                // to destination (dst_top, dst_left)
                if intermediates.contains(&b'$') {
                    let params_list: Vec<usize> = params
                        .iter()
                        .filter_map(|p| p.first().copied().map(|v| v as usize))
                        .collect();

                    // Need at least 7 parameters (8th is optional page number)
                    if params_list.len() >= 7 {
                        // Convert from 1-indexed VT coordinates to 0-indexed
                        // VT spec: 0 or missing defaults to 1
                        let src_top = params_list[0].saturating_sub(1);
                        let src_left = params_list[1].saturating_sub(1);
                        let src_bottom = params_list[2].saturating_sub(1);
                        let src_right = params_list[3].saturating_sub(1);
                        // params_list[4] is source page (ignored for now)
                        let dst_top = params_list[5].saturating_sub(1);
                        let dst_left = params_list[6].saturating_sub(1);
                        // params_list[7] is destination page (ignored for now)

                        self.active_grid_mut().copy_rectangle(
                            src_top, src_left, src_bottom, src_right, dst_top, dst_left,
                        );
                    }
                }
            }
            '{' => {
                // DECSERA - Selective Erase Rectangular Area (VT420)
                // CSI Pt ; Pl ; Pb ; Pr $ {
                // Erase rectangle (selective erase, but we implement as simple erase)
                if intermediates.contains(&b'$') {
                    let params_list: Vec<usize> = params
                        .iter()
                        .filter_map(|p| p.first().copied().map(|v| v as usize))
                        .collect();

                    if params_list.len() >= 4 {
                        // Convert from 1-indexed VT coordinates to 0-indexed
                        let top = params_list[0].saturating_sub(1);
                        let left = params_list[1].saturating_sub(1);
                        let bottom = params_list[2].saturating_sub(1);
                        let right = params_list[3].saturating_sub(1);

                        self.active_grid_mut()
                            .erase_rectangle(top, left, bottom, right);
                    }
                }
            }
            'z' => {
                // DECERA - Erase Rectangular Area (VT420)
                // CSI Pt ; Pl ; Pb ; Pr $ z
                // Erase rectangle unconditionally (doesn't respect protection/guarded flag)
                if intermediates.contains(&b'$') {
                    let params_list: Vec<usize> = params
                        .iter()
                        .filter_map(|p| p.first().copied().map(|v| v as usize))
                        .collect();

                    if params_list.len() >= 4 {
                        // Convert from 1-indexed VT coordinates to 0-indexed
                        let top = params_list[0].saturating_sub(1);
                        let left = params_list[1].saturating_sub(1);
                        let bottom = params_list[2].saturating_sub(1);
                        let right = params_list[3].saturating_sub(1);

                        self.active_grid_mut()
                            .erase_rectangle_unconditional(top, left, bottom, right);
                    }
                }
            }
            't' => {
                // Check for DECRARA first (VT420)
                if intermediates.contains(&b'$') {
                    // DECRARA - Reverse Attributes in Rectangular Area (VT420)
                    // CSI Pt ; Pl ; Pb ; Pr ; Ps $ t
                    // Reverse specified attributes in rectangle
                    let params_list: Vec<u16> =
                        params.iter().filter_map(|p| p.first().copied()).collect();

                    if params_list.len() >= 4 {
                        // Convert from 1-indexed VT coordinates to 0-indexed
                        let top = (params_list[0] as usize).saturating_sub(1);
                        let left = (params_list[1] as usize).saturating_sub(1);
                        let bottom = (params_list[2] as usize).saturating_sub(1);
                        let right = (params_list[3] as usize).saturating_sub(1);

                        // Remaining parameters are attributes to reverse
                        let attributes: Vec<u16> = if params_list.len() > 4 {
                            params_list[4..].to_vec()
                        } else {
                            vec![0] // Default: reverse all standard attributes
                        };

                        self.active_grid_mut().reverse_attributes_in_rectangle(
                            top,
                            left,
                            bottom,
                            right,
                            &attributes,
                        );
                    }
                } else {
                    // XTWINOPS - Window manipulation and reporting
                    let param = params
                        .iter()
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0);

                    match param {
                        18 => {
                            // Report text area size in characters
                            // Response: CSI 8 ; rows ; cols t
                            let response = format!("\x1b[8;{};{}t", rows, cols);
                            self.push_response(response.as_bytes());
                            debug::log(
                                debug::DebugLevel::Debug,
                                "XTWINOPS",
                                &format!("Reporting text area size: {} rows x {} cols", rows, cols),
                            );
                        }
                        14 => {
                            // Report text area size in pixels
                            // Response: CSI 4 ; height ; width t
                            let (w, h) = (self.pixel_width, self.pixel_height);
                            let (rep_w, rep_h) = if w > 0 && h > 0 { (w, h) } else { (cols, rows) };
                            // Fallback: approximate pixels as cols/rows if no pixel info set
                            // (some apps treat 0 as invalid)
                            let response = format!("\x1b[4;{};{}t", rep_h, rep_w);
                            self.push_response(response.as_bytes());
                            debug::log(
                                debug::DebugLevel::Debug,
                                "XTWINOPS",
                                &format!("Reporting pixel size: {}x{} (px)", rep_w, rep_h),
                            );
                        }
                        22 => {
                            // Save window title on stack (XTWINOPS push title)
                            self.title_stack.push(self.title.clone());
                            debug::log(
                                debug::DebugLevel::Debug,
                                "XTWINOPS",
                                &format!("Saved window title to stack: '{}'", self.title),
                            );
                        }
                        23 => {
                            // Restore window title from stack (XTWINOPS pop title)
                            if let Some(title) = self.title_stack.pop() {
                                self.title = title.clone();
                                debug::log(
                                    debug::DebugLevel::Debug,
                                    "XTWINOPS",
                                    &format!("Restored window title from stack: '{}'", title),
                                );
                            } else {
                                debug::log(
                                    debug::DebugLevel::Debug,
                                    "XTWINOPS",
                                    "Restore window title: stack is empty",
                                );
                            }
                        }
                        _ => {
                            // Other XTWINOPS we don't implement
                            debug::log(
                                debug::DebugLevel::Debug,
                                "XTWINOPS",
                                &format!("Unimplemented XTWINOPS param: {}", param),
                            );
                        }
                    }
                } // end XTWINOPS else block
            }
            'y' => {
                // DECRQCRA - Request Checksum of Rectangular Area (VT420)
                // CSI Pi ; Pg ; Pt ; Pl ; Pb ; Pr * y
                // Response: DCS Pi ! ~ xxxx ST
                if intermediates.contains(&b'*') {
                    let params_list: Vec<usize> = params
                        .iter()
                        .filter_map(|p| p.first().copied().map(|v| v as usize))
                        .collect();

                    if params_list.len() >= 6 {
                        let request_id = params_list[0];
                        // params_list[1] is page number (ignored)
                        let top = params_list[2].saturating_sub(1);
                        let left = params_list[3].saturating_sub(1);
                        let bottom = params_list[4].saturating_sub(1);
                        let right = params_list[5].saturating_sub(1);

                        // Calculate checksum of the rectangular area
                        let checksum = self.calculate_rectangle_checksum(top, left, bottom, right);

                        // Response: DCS Pi ! ~ xxxx ST
                        // where xxxx is a 4-digit hex checksum
                        let response = format!("\x1bP{}!~{:04X}\x1b\\", request_id, checksum);
                        self.push_response(response.as_bytes());
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::color::{Color, NamedColor};
    use crate::cursor::CursorStyle;
    use crate::mouse::{MouseEncoding, MouseMode};
    use crate::terminal::Terminal;

    // ========== Cursor Movement Tests ==========

    #[test]
    fn test_cursor_up() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;10H"); // Move to (10,10)

        term.process(b"\x1b[5A"); // Move up 5
        assert_eq!(term.cursor.row, 4); // 10-1-5 = 4 (0-indexed)

        term.process(b"\x1b[A"); // Default (1)
        assert_eq!(term.cursor.row, 3);

        term.process(b"\x1b[0A"); // 0 treated as 1
        assert_eq!(term.cursor.row, 2);
    }

    #[test]
    fn test_cursor_down() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[5;10H"); // Move to (10,5)

        term.process(b"\x1b[3B"); // Move down 3
        assert_eq!(term.cursor.row, 7); // 5-1+3 = 7 (0-indexed)

        // Test bounds
        term.process(b"\x1b[100B");
        assert_eq!(term.cursor.row, 23); // Last row (0-indexed)
    }

    #[test]
    fn test_cursor_forward() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;10H");

        term.process(b"\x1b[5C"); // Move right 5
        assert_eq!(term.cursor.col, 14); // 10-1+5 = 14 (0-indexed)

        // Test bounds
        term.process(b"\x1b[100C");
        assert_eq!(term.cursor.col, 79); // Last column
    }

    #[test]
    fn test_cursor_back() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;20H");

        term.process(b"\x1b[10D"); // Move left 10
        assert_eq!(term.cursor.col, 9); // 20-1-10 = 9 (0-indexed)

        // Test bounds
        term.process(b"\x1b[100D");
        assert_eq!(term.cursor.col, 0); // First column
    }

    #[test]
    fn test_cursor_position() {
        let mut term = Terminal::new(80, 24);

        // CUP - Cursor Position (1-indexed)
        term.process(b"\x1b[10;20H");
        assert_eq!(term.cursor.row, 9); // 0-indexed
        assert_eq!(term.cursor.col, 19);

        // Default position (1,1)
        term.process(b"\x1b[H");
        assert_eq!(term.cursor.row, 0);
        assert_eq!(term.cursor.col, 0);

        // HVP (same as CUP)
        term.process(b"\x1b[5;10f");
        assert_eq!(term.cursor.row, 4);
        assert_eq!(term.cursor.col, 9);
    }

    #[test]
    fn test_cursor_horizontal_absolute() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;10H");

        // CHA - Move to column 30 (1-indexed)
        term.process(b"\x1b[30G");
        assert_eq!(term.cursor.col, 29); // 0-indexed
        assert_eq!(term.cursor.row, 9); // Row unchanged
    }

    #[test]
    fn test_cursor_vertical_absolute() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;10H");

        // VPA - Move to row 15 (1-indexed)
        term.process(b"\x1b[15d");
        assert_eq!(term.cursor.row, 14); // 0-indexed
        assert_eq!(term.cursor.col, 9); // Column unchanged
    }

    #[test]
    fn test_cursor_next_prev_line() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b[10;20H");

        // CNL - Cursor next line (move down and to column 0)
        term.process(b"\x1b[3E");
        assert_eq!(term.cursor.row, 12); // 10-1+3 = 12
        assert_eq!(term.cursor.col, 0);

        // CPL - Cursor previous line (move up and to column 0)
        term.process(b"\x1b[5F");
        assert_eq!(term.cursor.row, 7); // 12-5 = 7
        assert_eq!(term.cursor.col, 0);
    }

    // ========== SGR Attribute Tests ==========

    #[test]
    fn test_sgr_reset() {
        let mut term = Terminal::new(80, 24);

        // Set some attributes
        term.process(b"\x1b[1;31;42m"); // Bold, red fg, green bg

        // Reset all
        term.process(b"\x1b[0m");
        assert!(!term.flags.bold());
    }

    #[test]
    fn test_sgr_bold_dim_italic() {
        let mut term = Terminal::new(80, 24);

        // Bold
        term.process(b"\x1b[1m");
        assert!(term.flags.bold());

        // Dim
        term.process(b"\x1b[2m");
        assert!(term.flags.dim());

        // Italic
        term.process(b"\x1b[3m");
        assert!(term.flags.italic());

        // Reset bold/dim
        term.process(b"\x1b[22m");
        assert!(!term.flags.bold());
        assert!(!term.flags.dim());

        // Reset italic
        term.process(b"\x1b[23m");
        assert!(!term.flags.italic());
    }

    #[test]
    fn test_sgr_underline() {
        let mut term = Terminal::new(80, 24);

        // Underline
        term.process(b"\x1b[4m");
        assert!(term.flags.underline());

        // No underline
        term.process(b"\x1b[24m");
        assert!(!term.flags.underline());
    }

    #[test]
    fn test_sgr_other_attributes() {
        let mut term = Terminal::new(80, 24);

        // Blink
        term.process(b"\x1b[5m");
        assert!(term.flags.blink());
        term.process(b"\x1b[25m");
        assert!(!term.flags.blink());

        // Reverse
        term.process(b"\x1b[7m");
        assert!(term.flags.reverse());
        term.process(b"\x1b[27m");
        assert!(!term.flags.reverse());

        // Hidden
        term.process(b"\x1b[8m");
        assert!(term.flags.hidden());
        term.process(b"\x1b[28m");
        assert!(!term.flags.hidden());

        // Strikethrough
        term.process(b"\x1b[9m");
        assert!(term.flags.strikethrough());
        term.process(b"\x1b[29m");
        assert!(!term.flags.strikethrough());
    }

    #[test]
    fn test_sgr_basic_colors() {
        let mut term = Terminal::new(80, 24);

        // Foreground colors (30-37)
        term.process(b"\x1b[31m"); // Red
        assert_eq!(term.fg, Color::Named(NamedColor::Red));

        term.process(b"\x1b[34m"); // Blue
        assert_eq!(term.fg, Color::Named(NamedColor::Blue));

        // Background colors (40-47)
        term.process(b"\x1b[42m"); // Green
        assert_eq!(term.bg, Color::Named(NamedColor::Green));

        // Bright colors (90-97)
        term.process(b"\x1b[91m"); // Bright red
        assert_eq!(term.fg, Color::Named(NamedColor::BrightRed));

        // Reset to defaults
        term.process(b"\x1b[39m");
        assert_eq!(term.fg, term.default_fg);
        term.process(b"\x1b[49m");
        assert_eq!(term.bg, term.default_bg);
    }

    #[test]
    fn test_sgr_rgb_colors() {
        let mut term = Terminal::new(80, 24);

        // Foreground RGB (38;2;r;g;b)
        term.process(b"\x1b[38;2;255;128;64m");
        assert_eq!(term.fg, Color::Rgb(255, 128, 64));

        // Background RGB (48;2;r;g;b)
        term.process(b"\x1b[48;2;10;20;30m");
        assert_eq!(term.bg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn test_sgr_256_colors() {
        let mut term = Terminal::new(80, 24);

        // Foreground 256 color (38;5;idx)
        term.process(b"\x1b[38;5;123m");
        assert_eq!(term.fg, Color::from_ansi_code(123));

        // Background 256 color (48;5;idx)
        term.process(b"\x1b[48;5;200m");
        assert_eq!(term.bg, Color::from_ansi_code(200));
    }

    // ========== Mode Tests ==========

    #[test]
    fn test_private_mode_cursor_visibility() {
        let mut term = Terminal::new(80, 24);

        // Show cursor
        term.process(b"\x1b[?25h");
        assert!(term.cursor.visible);

        // Hide cursor
        term.process(b"\x1b[?25l");
        assert!(!term.cursor.visible);
    }

    #[test]
    fn test_private_mode_application_cursor() {
        let mut term = Terminal::new(80, 24);

        // Enable application cursor
        term.process(b"\x1b[?1h");
        assert!(term.application_cursor);

        // Disable
        term.process(b"\x1b[?1l");
        assert!(!term.application_cursor);
    }

    #[test]
    fn test_private_mode_autowrap() {
        let mut term = Terminal::new(80, 24);

        // Disable autowrap
        term.process(b"\x1b[?7l");
        assert!(!term.auto_wrap);

        // Enable autowrap
        term.process(b"\x1b[?7h");
        assert!(term.auto_wrap);
    }

    #[test]
    fn test_private_mode_alt_screen() {
        let mut term = Terminal::new(80, 24);

        // Switch to alternate screen
        term.process(b"\x1b[?1049h");
        assert!(term.alt_screen_active);

        // Switch back to primary
        term.process(b"\x1b[?1049l");
        assert!(!term.alt_screen_active);
    }

    #[test]
    fn test_private_mode_mouse() {
        let mut term = Terminal::new(80, 24);

        // Normal mouse tracking
        term.process(b"\x1b[?1000h");
        assert!(matches!(term.mouse_mode, MouseMode::Normal));

        // Button event mode
        term.process(b"\x1b[?1002h");
        assert!(matches!(term.mouse_mode, MouseMode::ButtonEvent));

        // Any event mode
        term.process(b"\x1b[?1003h");
        assert!(matches!(term.mouse_mode, MouseMode::AnyEvent));

        // Disable
        term.process(b"\x1b[?1000l");
        assert!(matches!(term.mouse_mode, MouseMode::Off));
    }

    #[test]
    fn test_private_mode_mouse_encoding() {
        let mut term = Terminal::new(80, 24);

        // SGR mouse
        term.process(b"\x1b[?1006h");
        assert!(matches!(term.mouse_encoding, MouseEncoding::Sgr));

        // UTF-8 mouse
        term.process(b"\x1b[?1005h");
        assert!(matches!(term.mouse_encoding, MouseEncoding::Utf8));

        // URXVT mouse
        term.process(b"\x1b[?1015h");
        assert!(matches!(term.mouse_encoding, MouseEncoding::Urxvt));

        // Reset to default
        term.process(b"\x1b[?1006l");
        assert!(matches!(term.mouse_encoding, MouseEncoding::Default));
    }

    #[test]
    fn test_private_mode_bracketed_paste() {
        let mut term = Terminal::new(80, 24);

        // Enable bracketed paste
        term.process(b"\x1b[?2004h");
        assert!(term.bracketed_paste);

        // Disable
        term.process(b"\x1b[?2004l");
        assert!(!term.bracketed_paste);
    }

    // ========== Device Response Tests ==========

    #[test]
    fn test_device_status_report() {
        let mut term = Terminal::new(80, 24);

        // DSR 5 - Operating status
        term.process(b"\x1b[5n");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[0n");

        // DSR 6 - Cursor position report
        term.process(b"\x1b[10;20H");
        term.process(b"\x1b[6n");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[10;20R"); // 1-indexed
    }

    #[test]
    fn test_device_attributes() {
        let mut term = Terminal::new(80, 24);

        // Primary DA
        term.process(b"\x1b[c");
        let response = term.drain_responses();
        assert!(response.starts_with(b"\x1b[?"));

        // Secondary DA
        term.process(b"\x1b[>c");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[>82;10000;0c");
    }

    // ========== Scroll Region and Tab Tests ==========

    #[test]
    fn test_scroll_region() {
        let mut term = Terminal::new(80, 24);

        // Set scroll region rows 6-16 (1-indexed)
        term.process(b"\x1b[6;16r");
        assert_eq!(term.scroll_region_top, 5); // 0-indexed
        assert_eq!(term.scroll_region_bottom, 15);

        // Reset to full screen
        term.process(b"\x1b[r");
        assert_eq!(term.scroll_region_top, 0);
        assert_eq!(term.scroll_region_bottom, 23);
    }

    #[test]
    fn test_tab_stops() {
        let mut term = Terminal::new(80, 24);

        // Set a tab stop
        term.process(b"\x1b[1;20H");
        term.process(b"\x1bH"); // HTS (ESC H)
        assert!(term.tab_stops[19]); // 0-indexed

        // Clear tab at current position
        term.process(b"\x1b[g"); // or \x1b[0g
        assert!(!term.tab_stops[19]);

        // Clear all tabs
        term.process(b"\x1b[3g");
        assert!(!term.tab_stops.iter().any(|&x| x));
    }

    // ========== Cursor Style Tests ==========

    #[test]
    fn test_cursor_style() {
        let mut term = Terminal::new(80, 24);

        // Blinking block
        term.process(b"\x1b[1 q");
        assert_eq!(term.cursor.style, CursorStyle::BlinkingBlock);

        // Steady block
        term.process(b"\x1b[2 q");
        assert_eq!(term.cursor.style, CursorStyle::SteadyBlock);

        // Blinking underline
        term.process(b"\x1b[3 q");
        assert_eq!(term.cursor.style, CursorStyle::BlinkingUnderline);

        // Steady underline
        term.process(b"\x1b[4 q");
        assert_eq!(term.cursor.style, CursorStyle::SteadyUnderline);

        // Blinking bar
        term.process(b"\x1b[5 q");
        assert_eq!(term.cursor.style, CursorStyle::BlinkingBar);

        // Steady bar
        term.process(b"\x1b[6 q");
        assert_eq!(term.cursor.style, CursorStyle::SteadyBar);
    }

    // ========== Save/Restore Cursor Tests ==========

    #[test]
    fn test_save_restore_cursor_ansi() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b[10;15H");
        term.process(b"\x1b[31m"); // Red fg

        // Save cursor (ANSI.SYS style)
        term.process(b"\x1b[s");

        // Move and change
        term.process(b"\x1b[20;5H");
        term.process(b"\x1b[32m");

        // Restore cursor
        term.process(b"\x1b[u");
        assert_eq!(term.cursor.col, 14); // 0-indexed
        assert_eq!(term.cursor.row, 9);
    }

    // ========== XTWINOPS Tests ==========

    #[test]
    fn test_xtwinops_report_size() {
        let mut term = Terminal::new(80, 24);

        // Report text area size (CSI 18 t)
        term.process(b"\x1b[18t");
        let response = term.drain_responses();
        assert_eq!(response, b"\x1b[8;24;80t");
    }

    #[test]
    fn test_xtwinops_title_stack() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]0;Original\x1b\\");

        // Save title (CSI 22 t)
        term.process(b"\x1b[22t");

        term.process(b"\x1b]0;New\x1b\\");

        // Restore title (CSI 23 t)
        term.process(b"\x1b[23t");
        assert_eq!(term.title(), "Original");
    }

    // ========== Insert Mode Tests ==========

    #[test]
    fn test_insert_mode() {
        let mut term = Terminal::new(80, 24);

        // Enable insert mode (IRM)
        term.process(b"\x1b[4h");
        assert!(term.insert_mode);

        // Disable insert mode
        term.process(b"\x1b[4l");
        assert!(!term.insert_mode);
    }
}
