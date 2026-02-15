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

        // Add the base character (with HTML escaping)
        let ch = cell.c;
        match ch {
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '&' => html.push_str("&amp;"),
            '"' => html.push_str("&quot;"),
            '\0' | ' ' => html.push(' '),
            _ => html.push(ch),
        }

        // Add combining characters (variation selectors, ZWJ, skin tone modifiers, etc.)
        for &combining in &cell.combining {
            match combining {
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                '&' => html.push_str("&amp;"),
                '"' => html.push_str("&quot;"),
                _ => html.push(combining),
            }
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
    use crate::cell::Cell;
    use crate::color::{Color, NamedColor};

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
        assert!(html.contains("<head>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("</body>"));
    }

    #[test]
    fn test_basic_text_renders_correctly() {
        let mut grid = Grid::new(15, 1, 0);
        let text = "Hello, World!";
        for (i, ch) in text.chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }

        let html = export_html(&grid, false);
        assert!(html.contains("Hello, World!"));
    }

    #[test]
    fn test_html_special_characters_escaped() {
        let mut grid = Grid::new(10, 1, 0);
        let chars = ['<', '>', '&', '"'];
        for (i, &ch) in chars.iter().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }

        let html = export_html(&grid, false);
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        // Ensure special characters in content are escaped (not checking for <span which is valid HTML structure)
        let lines: Vec<&str> = html.lines().collect();
        let content_line = lines[0]; // First line should have our characters
                                     // Content between tags should be escaped
        assert!(
            content_line.contains("&lt;")
                || content_line.contains("&gt;")
                || content_line.contains("&amp;")
                || content_line.contains("&quot;")
        );
    }

    #[test]
    fn test_bold_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('B');
        cell.flags.set_bold(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("font-weight: bold"));
    }

    #[test]
    fn test_italic_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('I');
        cell.flags.set_italic(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("font-style: italic"));
    }

    #[test]
    fn test_underline_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('U');
        cell.flags.set_underline(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("text-decoration:"));
        assert!(html.contains("underline"));
    }

    #[test]
    fn test_strikethrough_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('S');
        cell.flags.set_strikethrough(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("text-decoration:"));
        assert!(html.contains("line-through"));
    }

    #[test]
    fn test_ansi_colors_render() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('R');
        cell.fg = Color::Named(NamedColor::Red);
        cell.bg = Color::Named(NamedColor::Blue);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        // Named colors should be converted to RGB
        assert!(html.contains("color: rgb("));
        assert!(html.contains("background-color: rgb("));
    }

    #[test]
    fn test_rgb_colors_render() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('C');
        cell.fg = Color::Rgb(255, 0, 0);
        cell.bg = Color::Rgb(0, 0, 255);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("color: rgb(255, 0, 0)"));
        assert!(html.contains("background-color: rgb(0, 0, 255)"));
    }

    #[test]
    fn test_include_styles_true_produces_full_document() {
        let grid = Grid::new(5, 2, 0);
        let html = export_html(&grid, true);

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html>"));
        assert!(html.contains("<head>"));
        assert!(html.contains("<meta charset=\"UTF-8\">"));
        assert!(html.contains("<style>"));
        assert!(html.contains("</style>"));
        assert!(html.contains("</head>"));
        assert!(html.contains("<body>"));
        assert!(html.contains("<pre>"));
        assert!(html.contains("</pre>"));
        assert!(html.contains("</body>"));
        assert!(html.ends_with("</html>\n"));
    }

    #[test]
    fn test_include_styles_false_produces_content_only() {
        let grid = Grid::new(5, 2, 0);
        let html = export_html(&grid, false);

        assert!(!html.contains("<!DOCTYPE"));
        assert!(!html.contains("<html>"));
        assert!(!html.contains("<head>"));
        assert!(!html.contains("<body>"));
        // Content should still be present (newlines for rows)
        assert!(html.contains('\n'));
    }

    #[test]
    fn test_empty_grid_produces_valid_output() {
        let grid = Grid::new(0, 0, 0);
        let html = export_html(&grid, true);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        // Should have structure but no content
        assert!(html.contains("<pre>"));
        assert!(html.contains("</pre>"));
    }

    #[test]
    fn test_wide_characters_cjk() {
        let mut grid = Grid::new(10, 1, 0);
        // CJK characters are typically wide (2 columns)
        grid.set(0, 0, Cell::new('中'));
        grid.set(1, 0, Cell::new('文'));
        grid.set(2, 0, Cell::new('字'));

        let html = export_html(&grid, false);
        assert!(html.contains('中'));
        assert!(html.contains('文'));
        assert!(html.contains('字'));
    }

    #[test]
    fn test_scrollback_included_in_export() {
        let mut grid = Grid::new(10, 2, 10);

        // Add some text
        for (i, ch) in "Line1".chars().enumerate() {
            grid.set(i, 0, Cell::new(ch));
        }
        for (i, ch) in "Line2".chars().enumerate() {
            grid.set(i, 1, Cell::new(ch));
        }

        // Scroll one line into scrollback
        grid.scroll_up(1);

        // Add new line
        for (i, ch) in "Line3".chars().enumerate() {
            grid.set(i, 1, Cell::new(ch));
        }

        let html = export_html(&grid, false);
        // Should contain both scrollback and current screen
        assert!(html.contains("Line1"));
        assert!(html.contains("Line3"));
    }

    #[test]
    fn test_multiple_attributes_combined() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('X');
        cell.flags.set_bold(true);
        cell.flags.set_italic(true);
        cell.flags.set_underline(true);
        cell.fg = Color::Rgb(255, 128, 0);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("font-weight: bold"));
        assert!(html.contains("font-style: italic"));
        assert!(html.contains("text-decoration:"));
        assert!(html.contains("underline"));
        assert!(html.contains("color: rgb(255, 128, 0)"));
    }

    #[test]
    fn test_dim_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('D');
        cell.flags.set_dim(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("opacity: 0.5"));
    }

    #[test]
    fn test_reverse_video_swaps_colors() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('R');
        cell.fg = Color::Rgb(255, 0, 0);
        cell.bg = Color::Rgb(0, 255, 0);
        cell.flags.set_reverse(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        // Colors should be swapped
        assert!(html.contains("color: rgb(0, 255, 0)"));
        assert!(html.contains("background-color: rgb(255, 0, 0)"));
    }

    #[test]
    fn test_hidden_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('H');
        cell.flags.set_hidden(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("visibility: hidden"));
    }

    #[test]
    fn test_blink_attribute_renders() {
        let mut grid = Grid::new(10, 1, 0);
        let mut cell = Cell::new('B');
        cell.flags.set_blink(true);
        grid.set(0, 0, cell);

        let html = export_html(&grid, false);
        assert!(html.contains("animation: blink"));
    }
}
