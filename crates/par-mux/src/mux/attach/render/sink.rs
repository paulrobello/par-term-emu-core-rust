//! The stdout flush sink: per-cell CUP + SGR writes and the host cursor's DECSCUSR placement.

use super::FlushSink;
use par_term_emu_core::cursor::CursorStyle;
use ratatui::buffer::Cell as RtCell;
use ratatui::style::{Color as RtColor, Modifier as RtModifier};
use std::fmt::Write as _;
use std::io::Write as _;

/// The stdout sink: absolute CUP + SGR per diff cell — ratatui's diff is
/// exactly the minimal cell set, and each cell carries its full style, so
/// per-cell reset+SGR+CUP+glyph is correct if not maximal-minimal. Frame
/// cadence (16 ms) keeps the volume at TUI-ordinary levels.
pub(super) struct StdoutSink;

impl FlushSink for StdoutSink {
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
        let mut out = String::with_capacity(diff.len() * 16);
        for (x, y, cell) in diff {
            // ratatui coordinates are 0-based; terminals are 1-based.
            let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
            write_styled(&mut out, cell);
            out.push_str(cell.symbol());
        }
        super::trace_input(format_args!(
            "frame flush {} cells {} bytes",
            diff.len(),
            out.len()
        ));
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }

    fn repaint_all(&mut self) {
        super::trace_input(format_args!("repaint_all (clear screen)"));
        let mut stdout = std::io::stdout().lock();
        // Clear, home, hide the cursor — the renderer owns the screen.
        let _ = stdout.write_all(b"\x1b[2J\x1b[H\x1b[?25l");
        let _ = stdout.flush();
    }

    fn place_cursor(&mut self, cursor: Option<(u16, u16, CursorStyle)>) {
        super::trace_cursor(format_args!("place_cursor {cursor:?}"));
        let mut out = String::with_capacity(24);
        match cursor {
            Some((x, y, style)) => {
                // 0-based window cell -> 1-based CUP, then the DECSCUSR
                // shape, then show.
                let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
                push_cursor_shape(&mut out, style);
                out.push_str("\x1b[?25h");
            }
            None => out.push_str("\x1b[?25l"),
        }
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }
}

/// The DECSCUSR spelling for one core cursor shape (`CSI Ps SP q`, xterm
/// ctlseqs "Set cursor style"), mapped from the pane's tracked DECSCUSR
/// state. Blink variants keep blinking: the blink rate is the host's.
pub(super) fn push_cursor_shape(out: &mut String, style: CursorStyle) {
    let ps = match style {
        CursorStyle::BlinkingBlock => 0,
        CursorStyle::SteadyBlock => 2,
        CursorStyle::BlinkingUnderline => 3,
        CursorStyle::SteadyUnderline => 4,
        CursorStyle::BlinkingBar => 5,
        CursorStyle::SteadyBar => 6,
    };
    let _ = write!(out, "\x1b[{ps} q");
}

/// One cell's SGR run: reset, then emit only what differs from default.
pub(super) fn write_styled(out: &mut String, cell: &RtCell) {
    out.push_str("\x1b[0m");
    if cell.fg != RtColor::Reset {
        write_color(out, cell.fg, true);
    }
    if cell.bg != RtColor::Reset {
        write_color(out, cell.bg, false);
    }
    if cell.modifier.contains(RtModifier::BOLD) {
        out.push_str("\x1b[1m");
    }
    if cell.modifier.contains(RtModifier::DIM) {
        out.push_str("\x1b[2m");
    }
    if cell.modifier.contains(RtModifier::ITALIC) {
        out.push_str("\x1b[3m");
    }
    if cell.modifier.contains(RtModifier::UNDERLINED) {
        out.push_str("\x1b[4m");
    }
    if cell.modifier.contains(RtModifier::REVERSED) {
        out.push_str("\x1b[7m");
    }
}

pub(super) fn write_color(out: &mut String, color: RtColor, fg: bool) {
    let base = if fg { 38 } else { 48 };
    match color {
        RtColor::Reset => {}
        // The named variants only arise if a future mapping introduces
        // them; emit their closest indexed spelling so the match stays
        // exhaustive and the output stays 256-color-correct.
        RtColor::Black => {
            let _ = write!(out, "\x1b[{base};5;0m");
        }
        RtColor::Red => {
            let _ = write!(out, "\x1b[{base};5;1m");
        }
        RtColor::Green => {
            let _ = write!(out, "\x1b[{base};5;2m");
        }
        RtColor::Yellow => {
            let _ = write!(out, "\x1b[{base};5;3m");
        }
        RtColor::Blue => {
            let _ = write!(out, "\x1b[{base};5;4m");
        }
        RtColor::Magenta => {
            let _ = write!(out, "\x1b[{base};5;5m");
        }
        RtColor::Cyan => {
            let _ = write!(out, "\x1b[{base};5;6m");
        }
        RtColor::Gray => {
            let _ = write!(out, "\x1b[{base};5;7m");
        }
        RtColor::DarkGray => {
            let _ = write!(out, "\x1b[{base};5;8m");
        }
        RtColor::LightRed => {
            let _ = write!(out, "\x1b[{base};5;9m");
        }
        RtColor::LightGreen => {
            let _ = write!(out, "\x1b[{base};5;10m");
        }
        RtColor::LightYellow => {
            let _ = write!(out, "\x1b[{base};5;11m");
        }
        RtColor::LightBlue => {
            let _ = write!(out, "\x1b[{base};5;12m");
        }
        RtColor::LightMagenta => {
            let _ = write!(out, "\x1b[{base};5;13m");
        }
        RtColor::LightCyan => {
            let _ = write!(out, "\x1b[{base};5;14m");
        }
        RtColor::White => {
            let _ = write!(out, "\x1b[{base};5;15m");
        }
        RtColor::Indexed(i) => {
            let _ = write!(out, "\x1b[{base};5;{i}m");
        }
        RtColor::Rgb(r, g, b) => {
            let _ = write!(out, "\x1b[{base};2;{r};{g};{b}m");
        }
    }
}
