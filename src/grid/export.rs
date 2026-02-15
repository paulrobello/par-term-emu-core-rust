//! Export and string conversion methods for the terminal grid

use crate::cell::Cell;
use crate::color::{Color, NamedColor};
use crate::grid::Grid;

impl Grid {
    /// Export the entire buffer (scrollback + visible) as plain text
    pub fn export_text_buffer(&self) -> String {
        let mut result = String::new();

        // Export scrollback
        for i in 0..self.scrollback_lines {
            if let Some(line) = self.scrollback_line(i) {
                let mut line_text = String::new();
                for cell in line {
                    if !cell.flags.wide_char_spacer() {
                        line_text.push(cell.c);
                        for &combining in &cell.combining {
                            line_text.push(combining);
                        }
                    }
                }
                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                if !self.is_scrollback_wrapped(i) {
                    result.push('\n');
                }
            }
        }

        // Export current screen
        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                let mut line_text = String::new();
                for cell in row_cells {
                    if !cell.flags.wide_char_spacer() {
                        line_text.push(cell.c);
                        for &combining in &cell.combining {
                            line_text.push(combining);
                        }
                    }
                }
                let trimmed = line_text.trim_end();
                result.push_str(trimmed);

                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push('\n');
                    }
                } else if !trimmed.is_empty() {
                    result.push('\n');
                }
            }
        }

        result
    }

    /// Get current screen content as string
    pub fn content_as_string(&self) -> String {
        let mut result = String::new();
        for row in 0..self.rows {
            let line = self.row_text(row);
            result.push_str(line.trim_end());
            result.push('\n');
        }
        result
    }

    /// Helper to find the last significant column in a row (non-space or styled)
    fn find_last_significant(&self, row_cells: &[Cell]) -> usize {
        let default_fg = Color::Named(NamedColor::White);
        let default_bg = Color::Named(NamedColor::Black);
        let default_flags = crate::cell::CellFlags::default();

        let mut last_significant = 0;
        for (col, cell) in row_cells.iter().enumerate() {
            if cell.flags.wide_char_spacer() {
                continue;
            }
            let has_content = cell.c != ' ' || !cell.combining.is_empty();
            let has_styling =
                cell.fg != default_fg || cell.bg != default_bg || cell.flags != default_flags;
            if has_content || has_styling {
                last_significant = col + 1;
            }
        }
        last_significant
    }

    /// Export the entire buffer with ANSI styling
    pub fn export_styled_buffer(&self) -> String {
        let mut result = String::new();
        let mut current_fg = Color::Named(NamedColor::White);
        let mut current_bg = Color::Named(NamedColor::Black);
        let mut current_flags = crate::cell::CellFlags::default();

        let emit_style =
            |result: &mut String, fg: &Color, bg: &Color, flags: &crate::cell::CellFlags| {
                result.push_str("\x1b[0");
                match fg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 30,
                            NamedColor::Red => 31,
                            NamedColor::Green => 32,
                            NamedColor::Yellow => 33,
                            NamedColor::Blue => 34,
                            NamedColor::Magenta => 35,
                            NamedColor::Cyan => 36,
                            NamedColor::White => 37,
                            NamedColor::BrightBlack => 90,
                            NamedColor::BrightRed => 91,
                            NamedColor::BrightGreen => 92,
                            NamedColor::BrightYellow => 93,
                            NamedColor::BrightBlue => 94,
                            NamedColor::BrightMagenta => 95,
                            NamedColor::BrightCyan => 96,
                            NamedColor::BrightWhite => 97,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => result.push_str(&format!(";38;5;{}", i)),
                    Color::Rgb(r, g, b) => result.push_str(&format!(";38;2;{};{};{}", r, g, b)),
                }
                match bg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 40,
                            NamedColor::Red => 41,
                            NamedColor::Green => 42,
                            NamedColor::Yellow => 43,
                            NamedColor::Blue => 44,
                            NamedColor::Magenta => 45,
                            NamedColor::Cyan => 46,
                            NamedColor::White => 47,
                            NamedColor::BrightBlack => 100,
                            NamedColor::BrightRed => 101,
                            NamedColor::BrightGreen => 102,
                            NamedColor::BrightYellow => 103,
                            NamedColor::BrightBlue => 104,
                            NamedColor::BrightMagenta => 105,
                            NamedColor::BrightCyan => 106,
                            NamedColor::BrightWhite => 107,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => result.push_str(&format!(";48;5;{}", i)),
                    Color::Rgb(r, g, b) => result.push_str(&format!(";48;2;{};{};{}", r, g, b)),
                }
                if flags.bold() {
                    result.push_str(";1");
                }
                if flags.dim() {
                    result.push_str(";2");
                }
                if flags.italic() {
                    result.push_str(";3");
                }
                if flags.underline() {
                    result.push_str(";4");
                }
                if flags.blink() {
                    result.push_str(";5");
                }
                if flags.reverse() {
                    result.push_str(";7");
                }
                if flags.hidden() {
                    result.push_str(";8");
                }
                if flags.strikethrough() {
                    result.push_str(";9");
                }
                result.push('m');
            };

        for i in 0..self.scrollback_lines {
            if let Some(line) = self.scrollback_line(i) {
                let last_sig = self.find_last_significant(line);
                for (col, cell) in line.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }
                    if col >= last_sig {
                        break;
                    }
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut result, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }
                    result.push(cell.c);
                    for &combining in &cell.combining {
                        result.push(combining);
                    }
                }
                if !self.is_scrollback_wrapped(i) {
                    result.push_str("\x1b[0m\n");
                    current_fg = Color::Named(NamedColor::White);
                    current_bg = Color::Named(NamedColor::Black);
                    current_flags = crate::cell::CellFlags::default();
                }
            }
        }

        for row in 0..self.rows {
            if let Some(line) = self.row(row) {
                let last_sig = self.find_last_significant(line);
                for (col, cell) in line.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }
                    if col >= last_sig {
                        break;
                    }
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut result, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }
                    result.push(cell.c);
                    for &combining in &cell.combining {
                        result.push(combining);
                    }
                }
                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push_str("\x1b[0m\n");
                        current_fg = Color::Named(NamedColor::White);
                        current_bg = Color::Named(NamedColor::Black);
                        current_flags = crate::cell::CellFlags::default();
                    }
                } else if last_sig > 0 {
                    result.push_str("\x1b[0m\n");
                }
            }
        }

        result
    }

    /// Export only the visible screen with ANSI styling
    pub fn export_visible_screen_styled(&self) -> String {
        let mut result = String::new();
        result.push_str("\x1b[H");
        let mut current_fg = Color::Named(NamedColor::White);
        let mut current_bg = Color::Named(NamedColor::Black);
        let mut current_flags = crate::cell::CellFlags::default();

        let emit_style =
            |result: &mut String, fg: &Color, bg: &Color, flags: &crate::cell::CellFlags| {
                result.push_str("\x1b[0");
                match fg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 30,
                            NamedColor::Red => 31,
                            NamedColor::Green => 32,
                            NamedColor::Yellow => 33,
                            NamedColor::Blue => 34,
                            NamedColor::Magenta => 35,
                            NamedColor::Cyan => 36,
                            NamedColor::White => 37,
                            NamedColor::BrightBlack => 90,
                            NamedColor::BrightRed => 91,
                            NamedColor::BrightGreen => 92,
                            NamedColor::BrightYellow => 93,
                            NamedColor::BrightBlue => 94,
                            NamedColor::BrightMagenta => 95,
                            NamedColor::BrightCyan => 96,
                            NamedColor::BrightWhite => 97,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => result.push_str(&format!(";38;5;{}", i)),
                    Color::Rgb(r, g, b) => result.push_str(&format!(";38;2;{};{};{}", r, g, b)),
                }
                match bg {
                    Color::Named(nc) => {
                        let code = match nc {
                            NamedColor::Black => 40,
                            NamedColor::Red => 41,
                            NamedColor::Green => 42,
                            NamedColor::Yellow => 43,
                            NamedColor::Blue => 44,
                            NamedColor::Magenta => 45,
                            NamedColor::Cyan => 46,
                            NamedColor::White => 47,
                            NamedColor::BrightBlack => 100,
                            NamedColor::BrightRed => 101,
                            NamedColor::BrightGreen => 102,
                            NamedColor::BrightYellow => 103,
                            NamedColor::BrightBlue => 104,
                            NamedColor::BrightMagenta => 105,
                            NamedColor::BrightCyan => 106,
                            NamedColor::BrightWhite => 107,
                        };
                        result.push_str(&format!(";{}", code));
                    }
                    Color::Indexed(i) => result.push_str(&format!(";48;5;{}", i)),
                    Color::Rgb(r, g, b) => result.push_str(&format!(";48;2;{};{};{}", r, g, b)),
                }
                if flags.bold() {
                    result.push_str(";1");
                }
                if flags.dim() {
                    result.push_str(";2");
                }
                if flags.italic() {
                    result.push_str(";3");
                }
                if flags.underline() {
                    result.push_str(";4");
                }
                if flags.blink() {
                    result.push_str(";5");
                }
                if flags.reverse() {
                    result.push_str(";7");
                }
                if flags.hidden() {
                    result.push_str(";8");
                }
                if flags.strikethrough() {
                    result.push_str(";9");
                }
                result.push('m');
            };

        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                let last_sig = self.find_last_significant(row_cells);
                if last_sig == 0 {
                    continue;
                }
                result.push_str(&format!("\x1b[{};1H", row + 1));
                for (col, cell) in row_cells.iter().enumerate() {
                    if cell.flags.wide_char_spacer() {
                        continue;
                    }
                    if col >= last_sig {
                        break;
                    }
                    if cell.fg != current_fg || cell.bg != current_bg || cell.flags != current_flags
                    {
                        emit_style(&mut result, &cell.fg, &cell.bg, &cell.flags);
                        current_fg = cell.fg;
                        current_bg = cell.bg;
                        current_flags = cell.flags;
                    }
                    result.push(cell.c);
                    for &combining in &cell.combining {
                        result.push(combining);
                    }
                }
                result.push_str("\x1b[0m");
                current_fg = Color::Named(NamedColor::White);
                current_bg = Color::Named(NamedColor::Black);
                current_flags = crate::cell::CellFlags::default();
            }
        }
        result
    }

    /// Generate a debug snapshot of the grid
    pub fn debug_snapshot(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();
        writeln!(
            output,
            "Grid: {}x{} (scrollback: {}/{})",
            self.cols, self.rows, self.scrollback_lines, self.max_scrollback
        )
        .unwrap();
        for row in 0..self.rows {
            let line: String = (0..self.cols)
                .map(|col| {
                    if let Some(cell) = self.get(col, row) {
                        if cell.c == '\0' || cell.c == ' ' {
                            ' '
                        } else {
                            cell.c
                        }
                    } else {
                        '?'
                    }
                })
                .collect();
            writeln!(output, "{:3}: |{}|", row, line).unwrap();
        }
        output
    }
}
