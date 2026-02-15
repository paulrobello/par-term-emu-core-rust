//! Screen and rendering types
//!
//! Provides types for selections, rendering hints, and line operations.

/// Selection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Character-by-character selection
    Character,
    /// Line-by-line selection
    Line,
    /// Rectangular block selection
    Block,
}

/// Selection state
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Start position (col, row)
    pub start: (usize, usize),
    /// End position (col, row)
    pub end: (usize, usize),
    /// Selection mode
    pub mode: SelectionMode,
}

/// Damage region for incremental rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRegion {
    /// Top-left column
    pub left: usize,
    /// Top-left row
    pub top: usize,
    /// Bottom-right column (exclusive)
    pub right: usize,
    /// Bottom-right row (exclusive)
    pub bottom: usize,
}

/// Z-order layer for rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZLayer {
    /// Background layer
    Background = 0,
    /// Normal content
    Normal = 1,
    /// Overlays (e.g., selections)
    Overlay = 2,
    /// Cursor and UI elements
    Cursor = 3,
}

/// Animation hint for smooth transitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationHint {
    /// No animation
    None,
    /// Smooth scroll
    SmoothScroll,
    /// Fade in/out
    Fade,
    /// Cursor blink
    CursorBlink,
}

/// Priority for partial updates
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UpdatePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Rendering hint for optimization
#[derive(Debug, Clone)]
pub struct RenderingHint {
    /// Damaged region that needs redrawing
    pub damage: DamageRegion,
    /// Z-order layer
    pub layer: ZLayer,
    /// Animation hint
    pub animation: AnimationHint,
    /// Update priority
    pub priority: UpdatePriority,
}

/// Line join result
#[derive(Debug, Clone)]
pub struct JoinedLines {
    /// The joined text
    pub text: String,
    /// Start row of joined section
    pub start_row: usize,
    /// End row of joined section (inclusive)
    pub end_row: usize,
    /// Number of lines joined
    pub lines_joined: usize,
}

/// Reflow statistics
#[derive(Debug, Clone)]
pub struct ReflowStats {
    /// Number of lines before reflow
    pub lines_before: usize,
    /// Number of lines after reflow
    pub lines_after: usize,
    /// Number of wrap points changed
    pub wraps_changed: usize,
}

/// HSV color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHSV {
    /// Hue (0-360 degrees)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Value/Brightness (0.0-1.0)
    pub v: f32,
}

/// HSL color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHSL {
    /// Hue (0-360 degrees)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Lightness (0.0-1.0)
    pub l: f32,
}

/// Color theme generation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Complementary color (opposite on color wheel)
    Complementary,
    /// Analogous colors (adjacent on color wheel)
    Analogous,
    /// Triadic colors (evenly spaced on color wheel)
    Triadic,
    /// Tetradic/square colors
    Tetradic,
    /// Split complementary
    SplitComplementary,
    /// Monochromatic (varying lightness)
    Monochromatic,
}

/// Generated color palette
#[derive(Debug, Clone)]
pub struct ColorPalette {
    /// Base color
    pub base: (u8, u8, u8),
    /// Generated colors based on theme mode
    pub colors: Vec<(u8, u8, u8)>,
    /// Theme mode used
    pub mode: ThemeMode,
}

/// Convert RGB to HSV
pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> ColorHSV {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    let v = max;

    ColorHSV { h, s, v }
}

/// Convert HSV to RGB
pub fn hsv_to_rgb(hsv: ColorHSV) -> (u8, u8, u8) {
    let c = hsv.v * hsv.s;
    let x = c * (1.0 - ((hsv.h / 60.0) % 2.0 - 1.0).abs());
    let m = hsv.v - c;

    let (r, g, b) = match hsv.h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

/// Convert RGB to HSL
pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> ColorHSL {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let l = (max + min) / 2.0;

    let s = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * l - 1.0).abs())
    };

    let h = if delta == 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };

    ColorHSL { h, s, l }
}

/// Convert HSL to RGB
pub fn hsl_to_rgb(hsl: ColorHSL) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * hsl.l - 1.0).abs()) * hsl.s;
    let x = c * (1.0 - ((hsl.h / 60.0) % 2.0 - 1.0).abs());
    let m = hsl.l - c / 2.0;

    let (r, g, b) = match hsl.h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 9: Line Wrapping Utilities ===

    /// Join wrapped lines starting from a given row
    ///
    /// Unwraps soft-wrapped lines into a single logical line.
    pub fn join_wrapped_lines(&self, start_row: usize) -> Option<JoinedLines> {
        let grid = self.active_grid();
        if start_row >= grid.rows() {
            return None;
        }

        let mut lines = Vec::new();
        let mut current_row = start_row;

        // Collect the first line
        if let Some(line) = grid.row(current_row) {
            lines.push(crate::terminal::cells_to_text(line));
        } else {
            return None;
        }

        // Follow wrapped lines
        while current_row < grid.rows() - 1 && grid.is_line_wrapped(current_row) {
            current_row += 1;
            if let Some(line) = grid.row(current_row) {
                lines.push(crate::terminal::cells_to_text(line));
            } else {
                break;
            }
        }

        Some(JoinedLines {
            text: lines.join(""),
            start_row,
            end_row: current_row,
            lines_joined: lines.len(),
        })
    }

    /// Get all logical lines (unwrapped) in the visible screen
    pub fn get_logical_lines(&self) -> Vec<String> {
        let grid = self.active_grid();
        let mut logical_lines = Vec::new();
        let mut row = 0;

        while row < grid.rows() {
            if let Some(joined) = self.join_wrapped_lines(row) {
                logical_lines.push(joined.text.trim_end().to_string());
                row = joined.end_row + 1;
            } else {
                row += 1;
            }
        }

        logical_lines
    }

    /// Check if a row starts a new logical line (not a continuation)
    pub fn is_line_start(&self, row: usize) -> bool {
        if row == 0 {
            return true;
        }
        let grid = self.active_grid();
        !grid.is_line_wrapped(row.saturating_sub(1))
    }

    /// Get a logical line by its row index
    ///
    /// Walks backwards to find the start of the logical line, then forward
    /// to collect the entire unwrapped content. This means calling with any
    /// physical row within a wrapped line returns the same full text.
    pub fn get_line_unwrapped(&self, row: usize) -> Option<String> {
        let grid = self.active_grid();
        // Walk backwards to find the start of the logical line
        let mut start = row;
        while start > 0 && grid.is_line_wrapped(start - 1) {
            start -= 1;
        }
        self.join_wrapped_lines(start).map(|j| j.text)
    }

    /// Get the word at the given position
    pub fn get_word_at(&self, col: usize, row: usize, word_chars: Option<&str>) -> Option<String> {
        let grid = self.active_grid();
        let line = grid.row(row)?;
        let line_text = crate::terminal::cells_to_text(line);

        if col >= line_text.len() {
            return None;
        }

        let is_word_char = |c: char| {
            if let Some(chars) = word_chars {
                chars.contains(c)
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };

        let chars: Vec<char> = line_text.chars().collect();
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }

        let mut end = col;
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }

        if start < end {
            Some(chars[start..end].iter().collect())
        } else {
            None
        }
    }

    // === Feature 19: Custom Rendering Hints ===

    /// Add a rendering hint for the frontend
    pub fn add_rendering_hint(&mut self, hint: RenderingHint) {
        self.rendering_hints.push(hint);
    }

    /// Get all pending rendering hints and clear the list
    pub fn poll_rendering_hints(&mut self) -> Vec<RenderingHint> {
        std::mem::take(&mut self.rendering_hints)
    }

    /// Add a damage region
    pub fn add_damage_region(&mut self, left: usize, top: usize, right: usize, bottom: usize) {
        self.damage_regions.push(DamageRegion {
            left,
            top,
            right,
            bottom,
        });
    }

    /// Get all accumulated damage regions and clear the list
    pub fn poll_damage_regions(&mut self) -> Vec<DamageRegion> {
        std::mem::take(&mut self.damage_regions)
    }

    /// Get all damage regions without clearing
    pub fn get_damage_regions(&self) -> &[DamageRegion] {
        &self.damage_regions
    }

    /// Merge all damage regions into a single bounding box
    pub fn merge_damage_regions(&mut self) {
        if self.damage_regions.is_empty() {
            return;
        }
        let mut min_left = usize::MAX;
        let mut min_top = usize::MAX;
        let mut max_right = 0;
        let mut max_bottom = 0;

        for region in &self.damage_regions {
            min_left = min_left.min(region.left);
            min_top = min_top.min(region.top);
            max_right = max_right.max(region.right);
            max_bottom = max_bottom.max(region.bottom);
        }

        self.damage_regions = vec![DamageRegion {
            left: min_left,
            top: min_top,
            right: max_right,
            bottom: max_bottom,
        }];
    }

    /// Clear all damage regions
    pub fn clear_damage_regions(&mut self) {
        self.damage_regions.clear();
    }

    /// Get all rendering hints without clearing
    pub fn get_rendering_hints(&self, sort_by_priority: bool) -> Vec<RenderingHint> {
        let mut hints = self.rendering_hints.clone();
        if sort_by_priority {
            hints.sort_by(|a, b| b.priority.cmp(&a.priority));
        }
        hints
    }

    /// Clear all rendering hints
    pub fn clear_rendering_hints(&mut self) {
        self.rendering_hints.clear();
    }

    // === Selection Management ===

    /// Set the current selection
    pub fn set_selection(
        &mut self,
        start: (usize, usize),
        end: (usize, usize),
        mode: SelectionMode,
    ) {
        self.selection = Some(Selection { start, end, mode });
    }

    /// Get the current selection
    pub fn get_selection(&self) -> Option<Selection> {
        self.selection.clone()
    }

    /// Get the text content of the current selection
    pub fn get_selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        let grid = self.active_grid();

        let (start_row, start_col) = (sel.start.1.min(sel.end.1), sel.start.0.min(sel.end.0));
        let (end_row, end_col) = (sel.start.1.max(sel.end.1), sel.start.0.max(sel.end.0));

        match sel.mode {
            SelectionMode::Character => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        let line_text = crate::terminal::cells_to_text(line);
                        let row_start = if row == start_row { start_col } else { 0 };
                        let row_end = if row == end_row {
                            end_col.min(line_text.len())
                        } else {
                            line_text.len()
                        };

                        if row_start < line_text.len() {
                            text.push_str(&line_text[row_start..row_end]);
                            if row < end_row {
                                text.push('\n');
                            }
                        }
                    }
                }
                Some(text)
            }
            SelectionMode::Line => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        text.push_str(&crate::terminal::cells_to_text(line));
                        if row < end_row {
                            text.push('\n');
                        }
                    }
                }
                Some(text)
            }
            SelectionMode::Block => {
                let mut text = String::new();
                for row in start_row..=end_row {
                    if let Some(line) = grid.row(row) {
                        let line_text = crate::terminal::cells_to_text(line);
                        let row_text = if start_col < line_text.len() {
                            &line_text[start_col..end_col.min(line_text.len())]
                        } else {
                            ""
                        };
                        text.push_str(row_text);
                        if row < end_row {
                            text.push('\n');
                        }
                    }
                }
                Some(text)
            }
        }
    }

    /// Select the word at the given position
    pub fn select_word_at(&mut self, col: usize, row: usize) {
        if let Some(word) = self.get_word_at(col, row, None) {
            // Find word boundaries
            let grid = self.active_grid();
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);
                if let Some(word_start) = line_text.find(word.as_str()) {
                    let word_end = word_start + word.len();
                    self.selection = Some(Selection {
                        start: (word_start, row),
                        end: (word_end, row),
                        mode: SelectionMode::Character,
                    });
                }
            }
        }
    }

    /// Select the word at the given position
    pub fn select_word(
        &mut self,
        col: usize,
        row: usize,
        word_chars: Option<&str>,
    ) -> Option<((usize, usize), (usize, usize))> {
        if let Some(word) = self.get_word_at(col, row, word_chars) {
            // Find word boundaries in the line
            let grid = self.active_grid();
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);
                if let Some(word_start) = line_text.find(word.as_str()) {
                    let word_end = word_start + word.len();
                    self.selection = Some(Selection {
                        start: (word_start, row),
                        end: (word_end, row),
                        mode: SelectionMode::Character,
                    });
                    return Some(((word_start, row), (word_end, row)));
                }
            }
        }
        None
    }

    /// Select the entire line at the given row
    pub fn select_line(&mut self, row: usize) {
        let grid = self.active_grid();
        let cols = grid.cols();
        self.selection = Some(Selection {
            start: (0, row),
            end: (cols, row),
            mode: SelectionMode::Line,
        });
    }

    /// Clear the current selection
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Select a semantic region based on delimiters
    pub fn select_semantic_region(
        &mut self,
        col: usize,
        row: usize,
        delimiters: Option<&str>,
    ) -> Option<String> {
        if let Some(delims) = delimiters {
            // Use the proper delimiter-aware implementation from text_utils
            let grid = self.active_grid();
            if let Some(content) = crate::text_utils::select_semantic_region(grid, col, row, delims)
            {
                return Some(content);
            }
        }
        // Fallback to word selection when no delimiters provided
        if let Some(word) = self.get_word_at(col, row, delimiters) {
            let grid = self.active_grid();
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);
                if let Some(word_start) = line_text.find(word.as_str()) {
                    let word_end = word_start + word.len();
                    self.selection = Some(Selection {
                        start: (word_start, row),
                        end: (word_end, row),
                        mode: SelectionMode::Character,
                    });
                    return Some(word);
                }
            }
        }
        None
    }

    // === Text Extraction ===

    /// Get text lines around a specific row (with context)
    ///
    /// # Arguments
    /// * `row` - The center row (0-based)
    /// * `context_before` - Number of lines before the row
    /// * `context_after` - Number of lines after the row
    ///
    /// Returns a vector of text lines.
    pub fn get_line_context(
        &self,
        row: usize,
        context_before: usize,
        context_after: usize,
    ) -> Vec<String> {
        let grid = self.active_grid();
        let mut lines = Vec::new();

        let start_row = row.saturating_sub(context_before);
        let end_row = (row + context_after).min(grid.rows() - 1);

        for r in start_row..=end_row {
            if let Some(line) = grid.row(r) {
                lines.push(crate::terminal::cells_to_text(line));
            }
        }

        lines
    }

    /// Get the paragraph at the given position
    ///
    /// A paragraph is defined as consecutive non-empty lines.
    pub fn get_paragraph_at(&self, row: usize) -> String {
        let grid = self.active_grid();
        let mut lines = Vec::new();

        // Find start of paragraph (search backwards)
        let mut start_row = row;
        while start_row > 0 {
            if let Some(line) = grid.row(start_row - 1) {
                let text = crate::terminal::cells_to_text(line).trim().to_string();
                if text.is_empty() {
                    break;
                }
                start_row -= 1;
            } else {
                break;
            }
        }

        // Find end of paragraph (search forwards)
        let mut end_row = row;
        while end_row < grid.rows() - 1 {
            if let Some(line) = grid.row(end_row + 1) {
                let text = crate::terminal::cells_to_text(line).trim().to_string();
                if text.is_empty() {
                    break;
                }
                end_row += 1;
            } else {
                break;
            }
        }

        // Collect paragraph lines
        for r in start_row..=end_row {
            if let Some(line) = grid.row(r) {
                lines.push(crate::terminal::cells_to_text(line));
            }
        }

        lines.join("\n")
    }
}
