//! HTML export functionality for terminal content

use crate::cell::Cell;
use crate::color::Color;
use crate::grid::Grid;

/// Generate HTML from terminal grid
pub fn export_html(grid: &Grid, include_styles: bool) -> String {
    let mut html = String::new();

    if include_styles {
        html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
        html.push_str("<meta charset=\"UTF-8\">\n");
        html.push_str("<style>\n");
        html.push_str("body { background-color: #000; color: #fff; margin: 0; padding: 20px; }\n");
        html.push_str(
            "pre { font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', 'Consolas', monospace; ",
        );
        html.push_str("font-size: 14px; line-height: 1.0; margin: 0; padding: 0; }\n");
        html.push_str(".term { display: inline; }\n");
        html.push_str("</style>\n");
        html.push_str("</head>\n<body>\n<pre>\n");
    }

    // Export scrollback
    for i in 0..grid.scrollback_len() {
        if let Some(line) = grid.scrollback_line(i) {
            export_line_to_html(line, &mut html);
            html.push('\n');
        }
    }

    // Export current screen
    for row in 0..grid.rows() {
        if let Some(line) = grid.row(row) {
            export_line_to_html(line, &mut html);
            html.push('\n');
        }
    }

    if include_styles {
        html.push_str("</pre>\n</body>\n</html>\n");
    }

    html
}

fn export_line_to_html(cells: &[Cell], html: &mut String) {
    let mut current_style: Option<String> = None;
    let mut span_open = false;

    for cell in cells {
        let cell_style = build_style_string(cell);

        // Close previous span if style changed
        if current_style.as_ref() != Some(&cell_style) {
            if span_open {
                html.push_str("</span>");
                span_open = false;
            }

            // Open new span if we have styles
            if !cell_style.is_empty() {
                html.push_str(&format!("<span class=\"term\" style=\"{}\">", cell_style));
                span_open = true;
            }

            current_style = Some(cell_style);
        }

        // Add the character
        let ch = cell.c;
        match ch {
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '&' => html.push_str("&amp;"),
            '"' => html.push_str("&quot;"),
            '\0' | ' ' => html.push(' '),
            _ => html.push(ch),
        }
    }

    // Close final span if open
    if span_open {
        html.push_str("</span>");
    }
}

fn build_style_string(cell: &Cell) -> String {
    let mut styles = Vec::new();

    // Foreground color
    if let Some((r, g, b)) = cell.fg.to_rgb_opt() {
        styles.push(format!("color: rgb({}, {}, {})", r, g, b));
    }

    // Background color
    if let Some((r, g, b)) = cell.bg.to_rgb_opt() {
        styles.push(format!("background-color: rgb({}, {}, {})", r, g, b));
    }

    // Text decoration
    let mut decorations = Vec::new();

    if cell.flags.bold() {
        styles.push("font-weight: bold".to_string());
    }

    if cell.flags.dim() {
        styles.push("opacity: 0.5".to_string());
    }

    if cell.flags.italic() {
        styles.push("font-style: italic".to_string());
    }

    if cell.flags.underline() {
        decorations.push("underline");
    }

    if cell.flags.strikethrough() {
        decorations.push("line-through");
    }

    if !decorations.is_empty() {
        styles.push(format!("text-decoration: {}", decorations.join(" ")));
    }

    if cell.flags.blink() {
        styles.push("animation: blink 1s step-start infinite".to_string());
    }

    if cell.flags.reverse() {
        // Swap fg and bg
        if let (Some((fg_r, fg_g, fg_b)), Some((bg_r, bg_g, bg_b))) =
            (cell.fg.to_rgb_opt(), cell.bg.to_rgb_opt())
        {
            styles.retain(|s| !s.starts_with("color:") && !s.starts_with("background-color:"));
            styles.push(format!("color: rgb({}, {}, {})", bg_r, bg_g, bg_b));
            styles.push(format!(
                "background-color: rgb({}, {}, {})",
                fg_r, fg_g, fg_b
            ));
        }
    }

    if cell.flags.hidden() {
        styles.push("visibility: hidden".to_string());
    }

    styles.join("; ")
}

impl Color {
    /// Convert color to RGB tuple, returning None for default colors
    #[allow(clippy::wrong_self_convention)]
    fn to_rgb_opt(&self) -> Option<(u8, u8, u8)> {
        match self {
            Color::Named(_) => Some(self.to_rgb()),
            Color::Indexed(_) => Some(self.to_rgb()),
            Color::Rgb(r, g, b) => Some((*r, *g, *b)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_html_basic() {
        let grid = Grid::new(10, 2, 0);
        let html = export_html(&grid, false);
        assert!(html.contains('\n'));
    }

    #[test]
    fn test_export_html_with_styles() {
        let grid = Grid::new(10, 2, 0);
        let html = export_html(&grid, true);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
    }
}
