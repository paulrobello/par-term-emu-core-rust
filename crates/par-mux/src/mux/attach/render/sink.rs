//! The stdout flush sink: run-length frame encoding and the host cursor's DECSCUSR placement.

use super::FlushSink;
use par_term_emu_core::cursor::CursorStyle;
use ratatui::buffer::Cell as RtCell;
use ratatui::style::{Color as RtColor, Modifier as RtModifier};
use std::fmt::Write as _;
use std::io::Write as _;
use unicode_width::UnicodeWidthChar;

/// The stdout sink: [`encode_frame`] keeps a frame's byte volume near the
/// cell count instead of several times it. Frame cadence (16 ms) does the
/// rest.
pub(super) struct StdoutSink;

impl FlushSink for StdoutSink {
    fn flush(&mut self, diff: &[(u16, u16, RtCell)]) {
        let out = encode_frame(diff);
        super::trace_input(format_args!(
            "frame flush {} cells {} bytes",
            diff.len(),
            out.len()
        ));
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(bytes);
        let _ = stdout.flush();
    }

    fn repaint_all(&mut self) {
        super::trace_input(format_args!("repaint_all (clear screen)"));
        let mut stdout = std::io::stdout().lock();
        // Reset whatever a pane app's passthrough bytes (or a previous
        // session) left borrowed on the host — a DECSTBM sized for a
        // pre-restart screen displaces every later scroll — then clear,
        // home, hide the cursor: the renderer owns the screen.
        let _ = stdout.write_all(crate::mux::attach::HOST_TERMINAL_RESET);
        let _ = stdout.write_all(b"\x1b[2J\x1b[H\x1b[?25l");
        let _ = stdout.flush();
    }

    fn anchor_hidden_cursor(&mut self, x: u16, y: u16) {
        super::trace_cursor(format_args!("anchor_hidden_cursor ({x}, {y})"));
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b[{};{}H", y + 1, x + 1);
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

/// Encode one frame diff for the terminal. The diff arrives row-major, so
/// a cell adjacent to the last written one rides the cursor's
/// auto-advance — a CUP goes out only on a jump, and a wide glyph's
/// two-column advance never lets a following cell misplace. SGR rides
/// runs: reset+delta only when the style changes from the previously
/// emitted cell. The per-cell CUP+SGR spelling made a full htop dialog
/// redraw span ~35 bytes/cell (120 KB — more than the TTY pipe delivers
/// in one piece, so the terminal painted the stages and flashed).
pub(super) fn encode_frame(diff: &[(u16, u16, RtCell)]) -> String {
    let mut out = String::with_capacity(diff.len() * 8);
    // Where the terminal cursor sits after the last emitted cell, in
    // 0-based ratatui coordinates.
    let mut cursor: Option<(u16, u16)> = None;
    let mut last_style: Option<(RtColor, RtColor, RtModifier)> = None;
    for &(x, y, ref cell) in diff {
        // A wide glyph's trailing cell: it writes nothing (no glyph, no
        // style landing), so skipping it entirely also skips its CUP —
        // the cursor stays past the wide glyph either way.
        if cell.symbol().is_empty() {
            continue;
        }
        if cursor != Some((x, y)) {
            // ratatui coordinates are 0-based; terminals are 1-based.
            let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
        }
        let style = (cell.fg, cell.bg, cell.modifier);
        if last_style != Some(style) {
            write_styled(&mut out, cell);
            last_style = Some(style);
        }
        out.push_str(cell.symbol());
        cursor = Some((x + symbol_width(cell.symbol()), y));
    }
    out
}

/// The columns `symbol` advances the cursor after writing it. A wide
/// glyph moves two, so the bookkeeping lands past the next cell's own
/// position and that cell re-addresses explicitly.
fn symbol_width(symbol: &str) -> u16 {
    symbol
        .chars()
        .map(|c| c.width().unwrap_or(0).min(2) as u16)
        .sum()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(symbol: &str, fg: RtColor) -> RtCell {
        let mut c = RtCell::new(" ");
        c.set_symbol(symbol);
        c.fg = fg;
        c
    }

    /// Adjacent cells share one CUP and one SGR; a jump or a style change
    /// emits exactly its own escape, nothing per cell beyond the glyph.
    #[test]
    fn frame_encoding_rides_runs_and_jumps() {
        let diff = vec![
            (5, 3, cell("a", RtColor::Reset)),
            (6, 3, cell("b", RtColor::Reset)),
            (7, 3, cell("c", RtColor::Reset)),
            (9, 3, cell("d", RtColor::Reset)), // jump within the row
            (0, 4, cell("e", RtColor::Indexed(2))), // row change + style
            (1, 4, cell("f", RtColor::Indexed(2))),
        ];
        let out = encode_frame(&diff);
        assert_eq!(
            out,
            concat!(
                "\x1b[4;6H",  // CUP to (5,3) 0-based -> 1-based
                "\x1b[0m",    // Reset style: the reset alone
                "abc",        // the run: no per-cell CUP or SGR
                "\x1b[4;10H", // the jump to (9,3)
                "d",
                "\x1b[5;1H",           // row change -> CUP
                "\x1b[0m\x1b[38;5;2m", // style change: reset + fg
                "ef"                   // the run
            )
        );
    }

    /// A wide glyph advances the cursor two columns, so the next cell
    /// always re-addresses explicitly; its trailing cell (empty symbol)
    /// writes nothing and leaves the address bookkeeping aligned.
    #[test]
    fn wide_glyphs_readdress_the_next_cell() {
        let mut trailing = RtCell::new(" ");
        trailing.set_symbol("");
        let diff = vec![
            (2, 7, cell("x", RtColor::Reset)),
            (3, 7, cell("漢", RtColor::Reset)),
            (4, 7, trailing),
            (5, 7, cell("y", RtColor::Reset)),
        ];
        let out = encode_frame(&diff);
        assert_eq!(
            out,
            concat!(
                "\x1b[8;3H\x1b[0m", // CUP + style, then…
                "x漢",              // the wide glyph rides the run
                "y"                 // (5,7): the trailing cell was skipped,
                                    // and after 漢 the cursor sits exactly
                                    // here — no CUP needed
            )
        );
    }

    /// The run-length encoding must replay to the byte-equivalent SCREEN
    /// as the old per-cell CUP+SGR spelling — a differential over
    /// deterministic pseudo-random buffer pairs (ASCII, wide CJK, box
    /// drawing, style runs, and diff gaps).
    #[test]
    fn run_length_encoding_replays_the_per_cell_screen() {
        use par_term_emu_core::terminal::Terminal;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let old_encode = |diff: &[(u16, u16, RtCell)]| -> String {
            let mut out = String::new();
            for (x, y, cell) in diff {
                out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
                write_styled(&mut out, cell);
                out.push_str(cell.symbol());
            }
            out
        };
        let mut state = 0xC0FFEEu64;
        let mut lcg = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state >> 33
        };
        for case in 0..100u32 {
            let mut prev = Buffer::empty(Rect::new(0, 0, 80, 24));
            let mut next = Buffer::empty(Rect::new(0, 0, 80, 24));
            for buf in [&mut prev, &mut next] {
                for y in 0..24u16 {
                    for x in 0..80u16 {
                        let r = lcg();
                        let cell = buf.cell_mut((x, y)).unwrap();
                        let pick = r % 12;
                        cell.set_symbol(match pick {
                            0 => "漢",
                            1 => "❯",
                            2 => "│",
                            3 => "x",
                            4..=6 => "a",
                            _ => " ",
                        });
                        if r % 3 == 0 {
                            cell.fg = RtColor::Indexed(((r >> 8) % 8) as u8);
                        }
                        if r % 5 == 0 {
                            cell.bg = RtColor::Indexed(((r >> 12) % 8) as u8);
                        }
                    }
                }
            }
            let diff: Vec<(u16, u16, RtCell)> = prev
                .diff(&next)
                .into_iter()
                .map(|(x, y, c)| (x, y, c.clone()))
                .collect();
            let screen = |bytes: &str| {
                let mut t = Terminal::new(80, 24);
                t.process(bytes.as_bytes());
                t.content()
            };
            let old = screen(&old_encode(&diff));
            let new = screen(&encode_frame(&diff));
            assert_eq!(old, new, "case {case} diverged; first differing row above");
        }
    }
}
