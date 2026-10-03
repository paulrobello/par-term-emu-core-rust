//! Export and string conversion methods for the terminal grid

use crate::cell::Cell;
use crate::color::{Color, NamedColor};
use crate::grid::Grid;

/// Append the ANSI SGR escape sequence for `fg`/`bg`/`flags` to `result`.
///
/// Emits `\x1b[0` followed by the foreground, background, and attribute codes,
/// terminating with `m`. Used by the styled export paths so the SGR-building
/// logic lives in exactly one place.
fn push_sgr_style(result: &mut String, fg: &Color, bg: &Color, flags: &crate::cell::CellFlags) {
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
}

/// The SGR attributes a styled export last emitted; starts (and resets) at
/// the default white-on-black, no flags.
struct SgrState {
    fg: Color,
    bg: Color,
    flags: crate::cell::CellFlags,
}

impl SgrState {
    fn reset() -> Self {
        Self {
            fg: Color::Named(NamedColor::White),
            bg: Color::Named(NamedColor::Black),
            flags: crate::cell::CellFlags::default(),
        }
    }
}

/// Append `row_cells` up to column `last_sig` (exclusive), skipping wide-char
/// spacers and emitting an SGR run whenever a cell's style differs from
/// `state` — the cell loop every styled export shares (QA-214). Row framing
/// (cursor addressing, resets, newlines) stays with the caller. Returns
/// whether any SGR run was emitted.
fn push_styled_cells(
    result: &mut String,
    row_cells: &[Cell],
    last_sig: usize,
    state: &mut SgrState,
) -> bool {
    let mut emitted_sgr = false;
    for (col, cell) in row_cells.iter().enumerate() {
        if cell.flags.wide_char_spacer() {
            continue;
        }
        if col >= last_sig {
            break;
        }
        let cell_fg = cell.fg();
        let cell_bg = cell.bg();
        if cell_fg != state.fg || cell_bg != state.bg || cell.flags != state.flags {
            push_sgr_style(result, &cell_fg, &cell_bg, &cell.flags);
            state.fg = cell_fg;
            state.bg = cell_bg;
            state.flags = cell.flags;
            emitted_sgr = true;
        }
        result.push(cell.c);
        for &combining in cell.combining() {
            result.push(combining);
        }
    }
    emitted_sgr
}

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
                        for &combining in cell.combining() {
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
                        for &combining in cell.combining() {
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
            let has_content = cell.c != ' ' || cell.has_combining_chars();
            let has_styling =
                cell.fg() != default_fg || cell.bg() != default_bg || cell.flags != default_flags;
            if has_content || has_styling {
                last_significant = col + 1;
            }
        }
        last_significant
    }

    /// Export the entire buffer with ANSI styling
    pub fn export_styled_buffer(&self) -> String {
        let mut result = String::new();
        let mut state = SgrState::reset();

        for i in 0..self.scrollback_lines {
            if let Some(line) = self.scrollback_line(i) {
                let last_sig = self.find_last_significant(line);
                push_styled_cells(&mut result, line, last_sig, &mut state);
                if !self.is_scrollback_wrapped(i) {
                    result.push_str("\x1b[0m\n");
                    state = SgrState::reset();
                }
            }
        }

        for row in 0..self.rows {
            if let Some(line) = self.row(row) {
                let last_sig = self.find_last_significant(line);
                push_styled_cells(&mut result, line, last_sig, &mut state);
                if row < self.rows - 1 {
                    if !self.is_line_wrapped(row) {
                        result.push_str("\x1b[0m\n");
                        state = SgrState::reset();
                    }
                } else if last_sig > 0 {
                    result.push_str("\x1b[0m\n");
                }
            }
        }

        result
    }

    /// One row's cells with ANSI styling: SGR-diffed text up to the last
    /// significant cell, then a reset. No cursor movement or line ending.
    pub(crate) fn export_row_styled(&self, row_cells: &[Cell]) -> String {
        let mut result = String::new();
        let last_sig = self.find_last_significant(row_cells);
        if last_sig == 0 {
            return result;
        }
        push_styled_cells(&mut result, row_cells, last_sig, &mut SgrState::reset());
        result.push_str("\x1b[0m");
        result
    }

    /// Export only the visible screen with ANSI styling
    pub fn export_visible_screen_styled(&self) -> String {
        let mut result = String::new();
        result.push_str("\x1b[H");
        let mut state = SgrState::reset();

        for row in 0..self.rows {
            if let Some(row_cells) = self.row(row) {
                let last_sig = self.find_last_significant(row_cells);
                if last_sig == 0 {
                    continue;
                }
                result.push_str(&format!("\x1b[{};1H", row + 1));
                push_styled_cells(&mut result, row_cells, last_sig, &mut state);
                result.push_str("\x1b[0m");
                state = SgrState::reset();
            }
        }
        result
    }

    /// Export the visible screen as newline-framed rows with inline SGR —
    /// the `capture-pane -e` shape (tmux's `-e` precedent: one line per
    /// grid row, escape bytes inline, no cursor addressing).
    ///
    /// Unlike [`Self::export_visible_screen_styled`] (cursor-addressed,
    /// for replay into an emulator), every row becomes exactly one
    /// `\n`-terminated line, empty rows included. A row that emitted no
    /// SGR run is plain text; a row that did is terminated with
    /// `\x1b[0m` before its newline.
    pub fn export_visible_screen_styled_lines(&self) -> String {
        let mut result = String::new();
        for row in 0..self.rows {
            let mut emitted_sgr = false;
            if let Some(row_cells) = self.row(row) {
                let last_sig = self.find_last_significant(row_cells);
                emitted_sgr =
                    push_styled_cells(&mut result, row_cells, last_sig, &mut SgrState::reset());
            }
            if emitted_sgr {
                result.push_str("\x1b[0m");
            }
            result.push('\n');
        }
        result
    }

    /// Export scrollback lines only, with ANSI styling
    ///
    /// The styled counterpart of the plain scrollback export: the same line
    /// selection and order (`max_lines` honored, same iteration as the Plain
    /// path), with per-run SGR so colors and attributes survive the export.
    pub fn export_scrollback_styled(&self, max_lines: Option<usize>) -> String {
        let lines_to_export = max_lines
            .unwrap_or(self.scrollback_lines)
            .min(self.scrollback_lines);
        let mut result = String::new();
        let mut state = SgrState::reset();

        for i in (0..lines_to_export).rev() {
            if let Some(line) = self.scrollback_line(i) {
                let last_sig = self.find_last_significant(line);
                push_styled_cells(&mut result, line, last_sig, &mut state);
                if !self.is_scrollback_wrapped(i) {
                    result.push_str("\x1b[0m\n");
                    state = SgrState::reset();
                }
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
        .expect("writing to a String cannot fail");
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
            writeln!(output, "{:3}: |{}|", row, line).expect("writing to a String cannot fail");
        }
        output
    }
}
