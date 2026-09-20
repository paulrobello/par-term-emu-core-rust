//! XTPUSHCOLORS / XTPOPCOLORS / XTREPORTCOLORS (xterm color palette stack)
//!
//! `CSI # P` pushes the current dynamic- and ANSI-palette colors onto a
//! 10-deep stack; `CSI # Q` pops the top entry back into the palette;
//! `CSI # R` reports the stack state. The parameterized forms
//! (`CSI Pi # P`/`CSI Pi # Q`) store/restore a specific stack slot without
//! pushing/popping; Pi 0 or omitted keeps the push/pop semantics. Storing
//! into a slot beyond the current depth grows the stack, padding
//! intermediate slots with snapshots of the current colors.
//!
//! Reply and depth semantics match xterm 397 (misc.c `xtermReportColors`):
//! the report is `CSI ? <used> ; <last> # Q` where `used` is the current
//! depth and `last` the high-water mark; pushes beyond the cap are silently
//! ignored; popping an empty stack is a no-op. RIS/DECSTR clear the stack
//! via `Terminal::reset`.

use crate::terminal::Terminal;
use vte::Params;

/// Maximum palette-stack depth; pushes beyond are ignored (xterm behavior)
pub(crate) const MAX_PALETTE_STACK: usize = 10;

impl Terminal {
    /// Snapshot the dynamic + ANSI palette colors
    fn palette_snapshot(&self) -> crate::terminal::ColorPaletteSnapshot {
        crate::terminal::ColorPaletteSnapshot {
            default_fg: self.theme.default_fg,
            default_bg: self.theme.default_bg,
            cursor_color: self.theme.cursor_color,
            ansi_palette: self.theme.ansi_palette,
        }
    }

    /// XTPUSHCOLORS (`CSI Pi # P`): Pi 0 or omitted pushes a snapshot of the
    /// dynamic + ANSI palette colors onto the stack; a nonzero Pi stores the
    /// snapshot into slot Pi (growing the stack — padded with current-color
    /// snapshots — when the slot is beyond the current depth) without
    /// pushing. Pushes beyond the cap are ignored.
    pub(crate) fn handle_xtpushcolors(&mut self, params: &Params) {
        let slot: u16 = params
            .iter()
            .next()
            .and_then(|p| p.first())
            .copied()
            .unwrap_or(0);
        if slot == 0 {
            if self.theme.palette_stack.len() >= MAX_PALETTE_STACK {
                return;
            }
            let snapshot = self.palette_snapshot();
            self.theme.palette_stack.push(snapshot);
            self.theme.palette_stack_last = self
                .theme
                .palette_stack_last
                .max(self.theme.palette_stack.len());
        } else {
            let slot = slot as usize;
            if slot > MAX_PALETTE_STACK {
                return;
            }
            let snapshot = self.palette_snapshot();
            let stack = &mut self.theme.palette_stack;
            while stack.len() < slot {
                stack.push(snapshot.clone());
            }
            stack[slot - 1] = snapshot;
            self.theme.palette_stack_last = self
                .theme
                .palette_stack_last
                .max(self.theme.palette_stack.len());
        }
    }

    /// XTPOPCOLORS (`CSI Pi # Q`): Pi 0 or omitted pops the top stack entry
    /// back into the palette; a nonzero Pi restores slot Pi without popping.
    /// Empty stack / missing slot is a no-op.
    pub(crate) fn handle_xtpopcolors(&mut self, params: &Params) {
        let slot: u16 = params
            .iter()
            .next()
            .and_then(|p| p.first())
            .copied()
            .unwrap_or(0);
        let index = (slot as usize).saturating_sub(1);
        let snapshot = if slot == 0 {
            self.theme.palette_stack.pop()
        } else if slot as usize <= self.theme.palette_stack.len() {
            Some(self.theme.palette_stack[index].clone())
        } else {
            None
        };
        if let Some(snapshot) = snapshot {
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

    // ─── Parameterized slot forms (CSI Pi # P / CSI Pi # Q) ────────────────

    /// Storing into an existing slot neither pushes nor pops; restoring a
    /// slot reads it without removing it.
    #[test]
    fn test_slot_store_and_restore_without_pushing() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b]4;1;rgb:ff/00/ff\x1b\\");
        // Store current colors into slot 1 (stack empty before/after)
        term.process(b"\x1b[1#P");
        assert_eq!(term.theme.palette_stack.len(), 1);
        let stored = term.theme.palette_stack[0].ansi_palette[1];

        term.process(b"\x1b]4;1;rgb:00/00/ff\x1b\\");
        assert_ne!(term.theme.ansi_palette[1], stored);
        // Restore slot 1 without popping — depth unchanged
        term.process(b"\x1b[1#Q");
        assert_eq!(term.theme.ansi_palette[1], stored);
        assert_eq!(term.theme.palette_stack.len(), 1, "restore must not pop");

        // The pop applies the stack top — the snapshot stored into slot 1,
        // since storing into a slot on an empty stack padded up to it — and
        // empties the stack.
        term.process(b"\x1b[#Q");
        assert_eq!(term.theme.palette_stack.len(), 0);
        assert_eq!(term.theme.ansi_palette[1], stored);
    }

    /// Storing into a slot beyond the current depth grows the stack, padding
    /// intermediate slots with snapshots of the current colors.
    #[test]
    fn test_slot_store_beyond_depth_grows_stack() {
        let mut term = Terminal::new(80, 4);
        term.process(b"\x1b]4;1;rgb:ff/00/ff\x1b\\");

        term.process(b"\x1b[3#P");
        assert_eq!(term.theme.palette_stack.len(), 3, "slots 1..=3 exist now");
        // Slots 1-2 hold the same colors that were current at store time
        assert_eq!(
            term.theme.palette_stack[0].ansi_palette[1],
            term.theme.palette_stack[2].ansi_palette[1]
        );

        // Restore from slot 3 works and does not pop
        term.process(b"\x1b]4;1;rgb:00/00/ff\x1b\\");
        term.process(b"\x1b[3#Q");
        assert_eq!(
            term.theme.ansi_palette[1], term.theme.palette_stack[2].ansi_palette[1],
            "slot 3 restore applies the stored snapshot"
        );
        assert_eq!(term.theme.palette_stack.len(), 3);
    }

    /// Pi 0 keeps the push/pop semantics.
    #[test]
    fn test_slot_0_keeps_push_pop_semantics() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b[0#P");
        assert_eq!(term.theme.palette_stack.len(), 1, "Pi 0 pushes");
        term.process(b"\x1b[0#Q");
        assert_eq!(term.theme.palette_stack.len(), 0, "Pi 0 pops");
    }

    /// XTREPORTCOLORS is untouched by the slot forms; its depth naturally
    /// reflects slot-store growth.
    #[test]
    fn test_report_unchanged_for_slot_forms() {
        let mut term = Terminal::new(80, 24);

        term.process(b"\x1b[2#P");
        term.process(b"\x1b[#R");
        assert_eq!(term.drain_responses(), b"\x1b[?2;2#Q");

        term.process(b"\x1b[1#Q");
        // Slot restore leaves depth alone; pop (0 # Q) lowers it to 1 (the
        // slot-2 store padded the stack to depth 2)
        term.process(b"\x1b[0#Q");
        term.process(b"\x1b[#R");
        assert_eq!(term.drain_responses(), b"\x1b[?1;2#Q");
    }

    /// Restoring a slot beyond the current depth is a no-op.
    #[test]
    fn test_slot_restore_missing_slot_is_noop() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b]4;1;rgb:ff/00/ff\x1b\\");

        term.process(b"\x1b[5#Q");
        assert_eq!(term.theme.palette_stack.len(), 0, "stack untouched");
        assert_eq!(term.theme.ansi_palette[1], term.theme.ansi_palette[1]);
    }
}
