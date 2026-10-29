//! Lock-light text export (ARC-004).
//!
//! A full-buffer export walks every scrollback row. Under a shared
//! `Arc<RwLock<Terminal>>` (`PtySession`) doing that walk while holding the
//! read lock stalls the PTY reader's write lock for the whole export. A
//! [`TextExportSnapshot`] copies only the grids the export reads, so the
//! caller holds the lock for a clone and builds the text after releasing it.
//!
//! `Terminal::{export_text, export_styled, scrollback}` are defined in terms
//! of the same grid functions, so the live and snapshot paths cannot drift.

use super::Terminal;
use crate::grid::Grid;

/// The grid state a text export reads, detached from the live terminal.
#[derive(Debug, Clone)]
pub struct TextExportSnapshot {
    /// Primary grid: the scrollback source for every export.
    primary: Grid,
    /// The alternate grid when it is the active screen; `None` when the
    /// primary is active (the visible screen is then `primary`'s).
    active_alt: Option<Grid>,
}

impl TextExportSnapshot {
    /// The grid whose visible rows are on screen.
    fn active(&self) -> &Grid {
        self.active_alt.as_ref().unwrap_or(&self.primary)
    }

    /// Same as [`Terminal::export_text`], built from the copy.
    pub fn export_text(&self) -> String {
        export_text(&self.primary, self.active())
    }

    /// Same as [`Terminal::export_styled`], built from the copy.
    pub fn export_styled(&self) -> String {
        self.export_text()
    }

    /// Same as [`Terminal::scrollback`], built from the copy.
    pub fn scrollback(&self) -> Vec<String> {
        scrollback_lines(&self.primary)
    }
}

impl Terminal {
    /// Copy the grids a text export reads. Cost is one clone of the primary
    /// grid (plus the alternate grid when it is active); the export itself
    /// then runs on the copy with no access to `self`.
    pub fn text_export_snapshot(&self) -> TextExportSnapshot {
        TextExportSnapshot {
            primary: self.grid.clone(),
            active_alt: self.alt_screen_active.then(|| self.alt_grid.clone()),
        }
    }
}

/// Scrollback rows of `grid` as text, oldest first.
pub(super) fn scrollback_lines(grid: &Grid) -> Vec<String> {
    (0..grid.scrollback_len())
        .filter_map(|i| grid.scrollback_line(i))
        .map(crate::text_utils::cells_to_text)
        .collect()
}

/// Visible rows of `grid` as logical lines: soft-wrapped rows joined, each
/// line's trailing whitespace trimmed.
pub(super) fn logical_lines(grid: &Grid) -> Vec<String> {
    let mut lines = Vec::new();
    let mut row = 0;
    while row < grid.rows() {
        let Some(first) = grid.row(row) else {
            row += 1;
            continue;
        };
        let mut text = crate::text_utils::cells_to_text(first);
        while row < grid.rows() - 1 && grid.is_line_wrapped(row) {
            row += 1;
            match grid.row(row) {
                Some(next) => text.push_str(&crate::text_utils::cells_to_text(next)),
                None => break,
            }
        }
        lines.push(text.trim_end().to_string());
        row += 1;
    }
    lines
}

/// Scrollback of `primary` newest-first, then the visible logical lines of
/// `active` — the historical `Terminal::export_text` shape.
pub(super) fn export_text(primary: &Grid, active: &Grid) -> String {
    let mut output = String::new();
    for i in (0..primary.scrollback_len()).rev() {
        if let Some(line) = primary.scrollback_line(i) {
            output.push_str(&crate::text_utils::cells_to_text(line));
            output.push('\n');
        }
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&logical_lines(active).join("\n"));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal exercising every input the export reads: multi-screen
    /// scrollback, a soft-wrapped line, wide and combining characters.
    fn busy_terminal() -> Terminal {
        let mut term = Terminal::with_scrollback(12, 4, 100);
        for i in 0..10 {
            term.process(format!("line {i}\r\n").as_bytes());
        }
        term.process("wide 日本 e\u{301}\r\n".as_bytes());
        term.process(b"a soft wrapped line that spans rows");
        term
    }

    #[test]
    fn snapshot_export_matches_live_export_on_the_primary_screen() {
        let term = busy_terminal();
        let snap = term.text_export_snapshot();
        assert!(term.grid().scrollback_len() > 0);
        assert_eq!(snap.export_text(), term.export_text());
        assert_eq!(snap.export_styled(), term.export_styled());
        assert_eq!(snap.scrollback(), term.scrollback());
    }

    #[test]
    fn snapshot_export_matches_live_export_on_the_alt_screen() {
        let mut term = busy_terminal();
        term.process(b"\x1b[?1049h\x1b[Halt screen text that also wraps");
        assert!(term.is_alt_screen_active());
        let snap = term.text_export_snapshot();
        let live = term.export_text();
        assert!(live.contains("alt screen text"));
        assert_eq!(snap.export_text(), live);
        assert_eq!(snap.scrollback(), term.scrollback());
    }

    #[test]
    fn snapshot_is_detached_from_later_output() {
        let mut term = busy_terminal();
        let snap = term.text_export_snapshot();
        let before = snap.export_text();
        term.process(b"\r\nlater output\r\n");
        assert_eq!(snap.export_text(), before);
        assert!(!before.contains("later output"));
    }

    /// The grid-level `logical_lines` must equal the Terminal-level
    /// `get_logical_lines` it replaced inside `content()`.
    #[test]
    fn logical_lines_match_get_logical_lines() {
        let mut term = busy_terminal();
        assert_eq!(logical_lines(term.active_grid()), term.get_logical_lines());
        term.process(b"\x1b[?1049hx");
        assert_eq!(logical_lines(term.active_grid()), term.get_logical_lines());
    }
}
