//! The passthrough `Session`'s status row: composition, the DECSTBM
//! reservation draw, and the tracked-cell cursor placement.

use super::*;
use std::io::Write as _;

impl Session {
    /// Whether the host grid changed since the last status draw. The
    /// first call sees `drawn_size == None` and reports true; the draw
    /// then records the size.
    pub(super) fn size_changed(&mut self) -> bool {
        let size = conn::terminal_grid();
        let changed = self.drawn_size != Some(size);
        self.drawn_size = Some(size);
        changed
    }

    /// The composed status line, sans padding/positioning: session name,
    /// pane title, agent count, and the held-dead cue with its respawn
    /// hint.
    pub(super) fn status_line(&self) -> String {
        let session = if self.session_name.is_empty() {
            "-"
        } else {
            &self.session_name
        };
        let title = if self.pane_title.is_empty() {
            &self.pane
        } else {
            &self.pane_title
        };
        // The workspaces segment leads the line: every workspace's name
        // in id order, the daemon's active one bracketed (passthrough has
        // no styling surface inside the inverse-video row).
        let mut line = String::new();
        if !self.workspaces.is_empty() {
            line.push(' ');
            for (index, (id, name)) in self.workspaces.iter().enumerate() {
                if index > 0 {
                    line.push(' ');
                }
                if Some(id) == self.active_workspace.as_ref() {
                    line.push('[');
                    line.push_str(name);
                    line.push(']');
                } else {
                    line.push_str(name);
                }
            }
            line.push_str(" |");
        }
        line.push_str(&format!(" {session} | {title}"));
        if self.agents > 0 {
            line.push_str(&format!(" | {} agent(s)", self.agents));
        }
        if let Some(code) = self.exited {
            match code {
                Some(code) => line.push_str(&format!(" | (exited {code} — C-b r respawns)")),
                None => line.push_str(" | (exited ? — C-b r respawns)"),
            }
        }
        // A reload (or its failure) leads the line: it is the freshest
        // fact and the one the user is waiting to see.
        if let Some(flash) = self.flash.as_deref() {
            line = format!(" {flash} |{line}");
        }
        line
    }

    /// The status line: reserve the bottom row with DECSTBM, draw
    /// `session | title [| N agents] [| (exited N — C-b r respawns)]`
    /// inverse-video, and re-place the cursor ABSOLUTELY at the pane's
    /// tracked cell.
    ///
    /// The old emission ended with ESC8 (restore saved cursor), which races
    /// pane output: a scroll landing between ESC7 and ESC8 leaves the saved
    /// position one line off and every later output paints over the wrong
    /// row. The draw keeps the ESC7/ESC8 wrap (protects against output
    /// interleaved WITHIN the draw itself), but the final position is a
    /// fresh absolute CUP computed from the shadow emulator's tracked cell
    /// — a scroll landing between the draw and the placement cannot make a
    /// fresh absolute position wrong.
    pub(super) fn draw_status(&mut self) {
        let (cols, rows) = conn::terminal_grid();
        if rows < 2 || cols < 2 {
            return; // nowhere to put a status row
        }
        let bytes = self.status_draw_bytes(rows, cols);
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }

    /// The status draw's byte emission, `(rows, cols)` parameterized so the
    /// headless suite can assert the cursor contract. The tracked-cell CUP
    /// closes the byte run.
    pub(super) fn status_draw_bytes(&mut self, rows: u16, cols: u16) -> Vec<u8> {
        let bottom = rows; // DECSTBM + CUP rows are 1-indexed inclusive
        let line = self.status_line();
        let width = (cols as usize).saturating_sub(1);
        let mut text: String = line.chars().take(width).collect();
        let used = text.chars().count();
        if used < width {
            text.push_str(&" ".repeat(width - used));
        }
        let scroll_region_bottom = rows - 1;
        let (tracked_col, tracked_row) = self.tracked_cell(rows, cols);
        let mut out = Vec::with_capacity(text.len() + 48);
        out.extend_from_slice(
            format!(
                "\x1b7\x1b[1;{scroll_region_bottom}r\x1b[{bottom};1H\x1b[7m{text}\x1b[0m\x1b8\x1b[{};{}H",
                tracked_row + 1,
                tracked_col + 1
            )
            .as_bytes(),
        );
        out
    }

    /// The pane's tracked cursor cell, `(col, row)`, 0-based, clamped into
    /// the host grid. The shadow emulator is re-fit to the host grid when
    /// the host size changed; its tracked cursor is the pane's truth —
    /// including over a held-dead pane, where the frozen screen's cell is
    /// the right place to put the cursor.
    pub(super) fn tracked_cell(&mut self, rows: u16, cols: u16) -> (u16, u16) {
        let (ecols, erows) = self.emulator.terminal().size();
        if (ecols as u16, erows as u16) != (cols, rows) {
            // Re-fit to the host grid; the tracked cursor survives the
            // re-fit (the core Terminal clamps it into bounds).
            self.emulator.resize(cols, rows);
        }
        let cursor = self.emulator.terminal().cursor();
        let (col, row) = (cursor.col as u16, cursor.row as u16);
        (
            col.min(cols.saturating_sub(1)),
            row.min(rows.saturating_sub(1)),
        )
    }
}
