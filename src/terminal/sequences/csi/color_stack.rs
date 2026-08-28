//! XTPUSHCOLORS / XTPOPCOLORS / XTREPORTCOLORS (xterm color palette stack)
//!
//! `CSI # P` pushes the current dynamic- and ANSI-palette colors onto a
//! 10-deep stack; `CSI # Q` pops the top entry back into the palette;
//! `CSI # R` reports the stack state. Parameterized forms (`CSI Pi # P/Q`,
//! which store/restore a specific stack slot without pushing/popping) are
//! not implemented — the no-parameter form is.
//!
//! Reply and depth semantics match xterm 397 (misc.c `xtermReportColors`):
//! the report is `CSI ? <used> ; <last> # Q` where `used` is the current
//! depth and `last` the high-water mark; pushes beyond the cap are silently
//! ignored; popping an empty stack is a no-op. RIS/DECSTR clear the stack
//! via `Terminal::reset`.

use crate::terminal::Terminal;

/// Maximum palette-stack depth; pushes beyond are ignored (xterm behavior)
pub(crate) const MAX_PALETTE_STACK: usize = 10;

impl Terminal {
    /// XTPUSHCOLORS (`CSI # P`): snapshot dynamic + ANSI palette colors onto the stack
    pub(crate) fn handle_xtpushcolors(&mut self) {
        if self.theme.palette_stack.len() >= MAX_PALETTE_STACK {
            return;
        }
        let snapshot = crate::terminal::ColorPaletteSnapshot {
            default_fg: self.theme.default_fg,
            default_bg: self.theme.default_bg,
            cursor_color: self.theme.cursor_color,
            ansi_palette: self.theme.ansi_palette,
        };
        self.theme.palette_stack.push(snapshot);
        self.theme.palette_stack_last = self
            .theme
            .palette_stack_last
            .max(self.theme.palette_stack.len());
    }

    /// XTPOPCOLORS (`CSI # Q`): restore the top stack entry; empty stack is a no-op
    pub(crate) fn handle_xtpopcolors(&mut self) {
        if let Some(snapshot) = self.theme.palette_stack.pop() {
            self.theme.default_fg = snapshot.default_fg;
            self.theme.default_bg = snapshot.default_bg;
            self.theme.cursor_color = snapshot.cursor_color;
            self.theme.ansi_palette = snapshot.ansi_palette;
        }
    }

    /// XTREPORTCOLORS (`CSI # R`): reply `CSI ? used ; last # Q` (xterm form)
    pub(crate) fn handle_xtreportcolors(&mut self) {
        let used = self.theme.palette_stack.len();
        let last = self.theme.palette_stack_last;
        let response = format!("\x1b[?{};{}#Q", used, last);
        self.push_response(response.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal::Terminal;

    fn push() -> &'static [u8] {
        b"\x1b[#P"
    }
    fn pop() -> &'static [u8] {
        b"\x1b[#Q"
    }
    fn report() -> &'static [u8] {
        b"\x1b[#R"
    }

    #[test]
    fn test_push_pop_restores_palette() {
        let mut term = Terminal::new(80, 24);
        let original = term.theme.ansi_palette[1];

        term.process(push());
        // Change palette entry 1 and the default fg via OSC 4 / OSC 10
        term.process(b"\x1b]4;1;rgb:ff/00/ff\x1b\\");
        term.process(b"\x1b]10;rgb:00/ff/00\x1b\\");
        assert_ne!(term.theme.ansi_palette[1], original);

        term.process(pop());
        assert_eq!(term.theme.ansi_palette[1], original);
        // fg must be back to the pre-push default (Named White), not the OSC 10 value
        assert_eq!(
            term.theme.default_fg,
            crate::color::Color::Named(crate::color::NamedColor::White)
        );
    }

    #[test]
    fn test_push_pop_restores_dynamic_colors() {
        let mut term = Terminal::new(80, 24);
        let (fg, bg, cursor) = (
            term.theme.default_fg,
            term.theme.default_bg,
            term.theme.cursor_color,
        );

        term.process(push());
        term.process(b"\x1b]10;rgb:11/11/11\x1b\\");
        term.process(b"\x1b]11;rgb:22/22/22\x1b\\");
        term.process(b"\x1b]12;rgb:33/33/33\x1b\\");

        term.process(pop());
        assert_eq!(term.theme.default_fg, fg);
        assert_eq!(term.theme.default_bg, bg);
        assert_eq!(term.theme.cursor_color, cursor);
    }

    #[test]
    fn test_pop_empty_stack_is_noop() {
        let mut term = Terminal::new(80, 24);
        let palette_before = term.theme.ansi_palette;
        let fg_before = term.theme.default_fg;

        term.process(pop());

        assert_eq!(term.theme.ansi_palette, palette_before);
        assert_eq!(term.theme.default_fg, fg_before);
    }

    #[test]
    fn test_stack_cap_at_ten() {
        let mut term = Terminal::new(80, 24);
        for _ in 0..15 {
            term.process(push());
        }
        assert_eq!(term.theme.palette_stack.len(), 10);

        // The 11th push was ignored, so 10 pops return to the same palette
        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?10;10#Q");
    }

    #[test]
    fn test_report_reply_matches_xterm() {
        let mut term = Terminal::new(80, 24);

        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?0;0#Q");

        term.process(push());
        term.process(push());
        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?2;2#Q");

        // One pop of two entries: depth 1, high-water mark stays 2 (xterm s->last)
        term.process(pop());
        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?1;2#Q");
    }

    #[test]
    fn test_bare_csi_p_is_still_dch() {
        let mut term = Terminal::new(80, 24);
        term.process(b"ABCDE");
        term.process(b"\x1b[H"); // home so DCH deletes from col 0
        term.process(b"\x1b[2P"); // DCH: delete 2 chars at cursor
        let line: String = term
            .grid
            .row(0)
            .unwrap()
            .iter()
            .take(5)
            .map(|c| c.c)
            .collect();
        assert_eq!(line, "CDE  ");
    }

    #[test]
    fn test_pushcolors_does_not_delete_chars() {
        // Regression: CSI # P previously misrouted to DCH
        let mut term = Terminal::new(80, 24);
        term.process(b"ABCDE");
        term.process(b"\x1b[#P");
        let line: String = term
            .grid
            .row(0)
            .unwrap()
            .iter()
            .take(5)
            .map(|c| c.c)
            .collect();
        assert_eq!(line, "ABCDE");
    }

    #[test]
    fn test_ris_clears_stack() {
        let mut term = Terminal::new(80, 24);
        term.process(push());
        term.process(push());
        term.process(b"\x1bc"); // RIS
        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?0;0#Q");
    }

    #[test]
    fn test_decstr_clears_stack() {
        let mut term = Terminal::new(80, 24);
        term.process(push());
        term.process(b"\x1b[!p"); // DECSTR
        term.process(report());
        assert_eq!(term.drain_responses(), b"\x1b[?0;0#Q");
    }
}
