//! Shell-integration accessors and semantic-zone queries (ARC-003).

use super::Terminal;
use crate::shell_integration::ShellIntegration;

impl Terminal {
    /// Get shell integration state
    pub fn shell_integration(&self) -> &ShellIntegration {
        &self.shell_state.shell_integration
    }

    /// Get shell integration state mutably
    pub fn shell_integration_mut(&mut self) -> &mut ShellIntegration {
        &mut self.shell_state.shell_integration
    }

    // ========== Semantic Zone Methods ==========

    /// Get all semantic zones from the primary grid
    pub fn get_zones(&self) -> &[crate::zone::Zone] {
        self.grid.zones()
    }

    /// Get the zone containing the given absolute row
    pub fn get_zone_at(&self, abs_row: usize) -> Option<&crate::zone::Zone> {
        self.grid.zone_at(abs_row)
    }

    /// Extract the text content of the zone containing the given absolute row.
    /// Returns None if no zone contains this row.
    pub fn get_zone_text(&self, abs_row: usize) -> Option<String> {
        let zone = self.grid.zone_at(abs_row)?;
        self.extract_text_from_row_range(zone.abs_row_start, zone.abs_row_end)
    }

    /// Extract text from an absolute row range (inclusive).
    /// Returns None if no rows could be found (e.g., evicted from scrollback).
    /// Handles wrapped lines by omitting newlines between them.
    pub(super) fn extract_text_from_row_range(
        &self,
        abs_start: usize,
        abs_end: usize,
    ) -> Option<String> {
        let scrollback_len = self.grid.scrollback_len();

        // Check if the range is entirely evicted from the scrollback buffer
        let total_scrolled = self.grid.total_lines_scrolled();
        let max_sb = self.grid.max_scrollback();
        if total_scrolled > max_sb {
            let floor = total_scrolled - max_sb;
            if abs_end < floor {
                return None;
            }
        }

        let mut text = String::new();
        let mut found_any = false;

        for row in abs_start..=abs_end {
            if row < scrollback_len {
                // Row is in scrollback
                if let Some(line) = self.grid.scrollback_line(row) {
                    found_any = true;
                    let line_text: String = line
                        .iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in c.combining() {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() {
                        // Check if previous line was wrapped
                        if row > abs_start && self.grid.is_scrollback_wrapped(row - 1) {
                            // Wrapped line - no newline
                        } else {
                            text.push('\n');
                        }
                    }
                    text.push_str(trimmed);
                }
            } else {
                // Row is in main grid
                let grid_row = row - scrollback_len;
                if let Some(line) = self.grid.row(grid_row) {
                    found_any = true;
                    let line_text: String = line
                        .iter()
                        .filter(|c| !c.flags.wide_char_spacer())
                        .map(|c| {
                            let mut s = String::new();
                            s.push(c.c);
                            for &combining in c.combining() {
                                s.push(combining);
                            }
                            s
                        })
                        .collect();
                    let trimmed = line_text.trim_end();
                    if !text.is_empty() && row > abs_start {
                        let prev_row = row - 1;
                        if prev_row < scrollback_len {
                            if !self.grid.is_scrollback_wrapped(prev_row) {
                                text.push('\n');
                            }
                        } else {
                            let prev_grid_row = prev_row - scrollback_len;
                            if !self.grid.is_line_wrapped(prev_grid_row) {
                                text.push('\n');
                            }
                        }
                    }
                    text.push_str(trimmed);
                }
            }
        }

        if found_any {
            Some(text)
        } else {
            None
        }
    }
}
