use crate::color::{Color, NamedColor};
use crate::grid::Grid;
use crate::screenshot::ScreenshotResult;

/// Encode grid as SVG
///
/// SVG encoder for terminal screenshots that generates clean, scalable vector graphics
/// that preserve text as actual SVG text elements (not rasterized).
pub fn encode(grid: &Grid, font_size: f32, padding: u32) -> ScreenshotResult<Vec<u8>> {
    let rows = grid.rows();
    let cols = grid.cols();

    // Calculate dimensions
    // SVG uses font size directly for spacing
    let char_width = font_size * 0.6; // Monospace approximation
    let line_height = font_size * 1.2;

    let content_width = cols as f32 * char_width;
    let content_height = rows as f32 * line_height;
    let canvas_width = content_width + (padding as f32 * 2.0);
    let canvas_height = content_height + (padding as f32 * 2.0);

    let mut svg = String::new();

    // SVG header
    svg.push_str(&format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">
"#,
        canvas_width as u32, canvas_height as u32, canvas_width as u32, canvas_height as u32
    ));

    // Add CSS styles for text
    svg.push_str(
        r#"<style>
    text {
        font-family: 'Courier New', 'Monaco', 'Menlo', 'Consolas', monospace;
        white-space: pre;
    }
    .bold { font-weight: bold; }
    .italic { font-style: italic; }
    .underline { text-decoration: underline; }
    .strikethrough { text-decoration: line-through; }
    .dim { opacity: 0.6; }
</style>
"#,
    );

    // Background - use default black
    let bg_color = grid
        .get(0, 0)
        .map(|cell| cell.bg.to_rgb())
        .unwrap_or((0, 0, 0));

    svg.push_str(&format!(
        r#"<rect width="100%" height="100%" fill="rgb({},{},{})" />
"#,
        bg_color.0, bg_color.1, bg_color.2
    ));

    // Group for content with padding offset
    svg.push_str(&format!(
        r#"<g transform="translate({}, {})">
"#,
        padding, padding
    ));

    // Render each row
    for row in 0..rows {
        let y = row as f32 * line_height + font_size; // Baseline position

        // Process cells in runs of same attributes
        let mut col = 0;
        while col < cols {
            let cell = match grid.get(col, row) {
                Some(c) => c,
                None => {
                    col += 1;
                    continue;
                }
            };

            // Skip empty cells (spaces with default colors)
            if cell.c == ' ' && is_default_color(&cell.fg) {
                col += 1;
                continue;
            }

            // Collect run of cells with same attributes
            let mut run_text = String::new();
            let fg = cell.fg.to_rgb();
            let start_col = col;
            let bold = cell.flags.bold();
            let italic = cell.flags.italic();
            let underline = cell.flags.underline();
            let strikethrough = cell.flags.strikethrough();
            let dim = cell.flags.dim();

            while col < cols {
                let current = match grid.get(col, row) {
                    Some(c) => c,
                    None => break,
                };

                // Check if attributes match
                let current_fg = current.fg.to_rgb();
                if current_fg != fg
                    || current.flags.bold() != bold
                    || current.flags.italic() != italic
                    || current.flags.underline() != underline
                    || current.flags.strikethrough() != strikethrough
                    || current.flags.dim() != dim
                {
                    break;
                }

                run_text.push(current.c);
                col += 1;
            }

            // Skip if all spaces
            if run_text.trim().is_empty() {
                continue;
            }

            // Build SVG text element
            let x = start_col as f32 * char_width;

            let mut classes = Vec::new();
            if bold {
                classes.push("bold");
            }
            if italic {
                classes.push("italic");
            }
            if underline {
                classes.push("underline");
            }
            if strikethrough {
                classes.push("strikethrough");
            }
            if dim {
                classes.push("dim");
            }

            let class_attr = if !classes.is_empty() {
                format!(r#" class="{}""#, classes.join(" "))
            } else {
                String::new()
            };

            // Escape XML special characters
            let escaped_text = escape_xml(&run_text);

            svg.push_str(&format!(
                r#"<text x="{}" y="{}" font-size="{}" fill="rgb({},{},{})"{}>{}</text>
"#,
                x, y, font_size, fg.0, fg.1, fg.2, class_attr, escaped_text
            ));
        }
    }

    svg.push_str("</g>\n");
    svg.push_str("</svg>\n");

    Ok(svg.into_bytes())
}

/// Check if color is default foreground (white)
fn is_default_color(color: &Color) -> bool {
    matches!(color, Color::Named(NamedColor::White))
}

/// Escape XML special characters
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;
    use crate::grid::Grid;

    #[test]
    fn test_svg_encode_basic() {
        let mut grid = Grid::new(80, 24, 1000);

        // Add some test content
        for col in 0..5 {
            let cell = Cell::new('H');
            grid.set(col, 0, cell);
        }

        let result = encode(&grid, 14.0, 10);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("HHHHH"));
    }

    #[test]
    fn test_svg_xml_escaping() {
        assert_eq!(escape_xml("Hello"), "Hello");
        assert_eq!(escape_xml("A & B"), "A &amp; B");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_xml("\"quoted\""), "&quot;quoted&quot;");
    }
}
