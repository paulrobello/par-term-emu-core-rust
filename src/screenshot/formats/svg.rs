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
    .dim { opacity: 0.5; }
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

                // Output full grapheme cluster (base char + combining chars)
                run_text.push(current.c);
                for &combining in &current.combining {
                    run_text.push(combining);
                }
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

    #[test]
    fn test_escape_xml_all_special_chars() {
        let input = r#"<&"'>"#;
        let expected = "&lt;&amp;&quot;&apos;&gt;";
        assert_eq!(escape_xml(input), expected);
    }

    #[test]
    fn test_escape_xml_no_special_chars() {
        let input = "Hello World 123";
        assert_eq!(escape_xml(input), input);
    }

    #[test]
    fn test_escape_xml_mixed_content() {
        let input = "Hello <world> & \"friends\"!";
        let expected = "Hello &lt;world&gt; &amp; &quot;friends&quot;!";
        assert_eq!(escape_xml(input), expected);
    }

    #[test]
    fn test_is_default_color_white() {
        let color = Color::Named(NamedColor::White);
        assert!(is_default_color(&color));
    }

    #[test]
    fn test_is_default_color_black() {
        let color = Color::Named(NamedColor::Black);
        assert!(!is_default_color(&color));
    }

    #[test]
    fn test_is_default_color_rgb() {
        let color = Color::Rgb(255, 255, 255);
        assert!(!is_default_color(&color));
    }

    #[test]
    fn test_is_default_color_indexed() {
        let color = Color::Indexed(7); // White index
        assert!(!is_default_color(&color));
    }

    #[test]
    fn test_dimension_calculations() {
        let font_size = 14.0;
        let char_width = font_size * 0.6;
        let line_height = font_size * 1.2;

        assert_eq!(char_width, 8.4);
        assert_eq!(line_height, 16.8);
    }

    #[test]
    fn test_canvas_size_with_padding() {
        let rows = 24;
        let cols = 80;
        let font_size = 14.0;
        let padding = 10u32;

        let char_width = font_size * 0.6;
        let line_height = font_size * 1.2;

        let content_width = cols as f32 * char_width;
        let content_height = rows as f32 * line_height;
        let canvas_width = content_width + (padding as f32 * 2.0);
        let canvas_height = content_height + (padding as f32 * 2.0);

        // Allow small floating point differences
        assert!((content_width - 672.0).abs() < 0.01); // 80 * 8.4
        assert!((content_height - 403.2).abs() < 0.01); // 24 * 16.8
        assert!((canvas_width - 692.0).abs() < 0.01); // 672 + 20
        assert!((canvas_height - 423.2).abs() < 0.01); // 403.2 + 20
    }

    #[test]
    fn test_baseline_position() {
        let row = 5;
        let font_size = 14.0;
        let line_height = font_size * 1.2;

        let y = row as f32 * line_height + font_size;

        // Allow small floating point differences
        assert!((y - 98.0).abs() < 0.01); // 5 * 16.8 + 14 = 84 + 14
    }

    #[test]
    fn test_svg_contains_xml_declaration() {
        let grid = Grid::new(10, 5, 100);
        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    }

    #[test]
    fn test_svg_contains_xmlns() {
        let grid = Grid::new(10, 5, 100);
        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"xmlns="http://www.w3.org/2000/svg""#));
    }

    #[test]
    fn test_svg_contains_css_styles() {
        let grid = Grid::new(10, 5, 100);
        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains("<style>"));
        assert!(svg.contains(".bold { font-weight: bold; }"));
        assert!(svg.contains(".italic { font-style: italic; }"));
        assert!(svg.contains(".underline { text-decoration: underline; }"));
        assert!(svg.contains(".strikethrough { text-decoration: line-through; }"));
        assert!(svg.contains(".dim { opacity: 0.5; }"));
    }

    #[test]
    fn test_svg_has_background_rect() {
        let grid = Grid::new(10, 5, 100);
        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"<rect width="100%" height="100%""#));
    }

    #[test]
    fn test_svg_with_bold_text() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('B');
        cell.flags.set_bold(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"class="bold""#));
    }

    #[test]
    fn test_svg_with_italic_text() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('I');
        cell.flags.set_italic(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"class="italic""#));
    }

    #[test]
    fn test_svg_with_underline_text() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('U');
        cell.flags.set_underline(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"class="underline""#));
    }

    #[test]
    fn test_svg_with_strikethrough_text() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('S');
        cell.flags.set_strikethrough(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"class="strikethrough""#));
    }

    #[test]
    fn test_svg_with_dim_text() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('D');
        cell.flags.set_dim(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"class="dim""#));
    }

    #[test]
    fn test_svg_with_multiple_classes() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('M');
        cell.flags.set_bold(true);
        cell.flags.set_italic(true);
        cell.flags.set_underline(true);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains("bold"));
        assert!(svg.contains("italic"));
        assert!(svg.contains("underline"));
    }

    #[test]
    fn test_svg_with_custom_font_size() {
        let mut grid = Grid::new(10, 5, 100);
        // Add a character so there's actual text to render
        let cell = Cell::new('T');
        grid.set(0, 0, cell);

        let font_size = 20.0;
        let result = encode(&grid, font_size, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // Font size should be in the SVG output (in CSS or on text elements)
        assert!(svg.contains("20"));
    }

    #[test]
    fn test_svg_with_custom_padding() {
        let grid = Grid::new(10, 5, 100);
        let padding = 25u32;
        let result = encode(&grid, 14.0, padding);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // Transform should have padding offset
        assert!(svg.contains(r#"transform="translate(25, 25)""#));
    }

    #[test]
    fn test_svg_empty_grid() {
        let grid = Grid::new(10, 5, 100);
        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // Should still have valid SVG structure
        assert!(svg.contains("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn test_svg_single_character() {
        let mut grid = Grid::new(80, 24, 1000);
        let cell = Cell::new('X');
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // SVG output may vary based on how text is grouped
        // Just verify the character is present in some form
        assert!(svg.contains('X'));
    }

    #[test]
    fn test_svg_with_rgb_color() {
        let mut grid = Grid::new(10, 5, 100);
        let mut cell = Cell::new('C');
        cell.fg = Color::Rgb(255, 128, 64);
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains(r#"fill="rgb(255,128,64)""#));
    }

    #[test]
    fn test_svg_run_grouping() {
        let mut grid = Grid::new(10, 5, 100);

        // Add consecutive characters with same attributes
        for col in 0..5 {
            let cell = Cell::new('A');
            grid.set(col, 0, cell);
        }

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // Should contain the characters (may or may not be grouped)
        assert!(svg.contains('A'));
    }

    #[test]
    fn test_svg_special_chars_escaped() {
        let mut grid = Grid::new(10, 5, 100);
        let cell = Cell::new('<');
        grid.set(0, 0, cell);

        let result = encode(&grid, 14.0, 0);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();
        // XML special characters should be escaped
        assert!(svg.contains("&lt;"));
    }

    #[test]
    fn test_class_attribute_building() {
        let classes = ["bold", "italic"];

        let class_attr = if !classes.is_empty() {
            format!(r#" class="{}""#, classes.join(" "))
        } else {
            String::new()
        };

        assert_eq!(class_attr, r#" class="bold italic""#);
    }

    #[test]
    fn test_empty_class_attribute() {
        let classes: Vec<&str> = Vec::new();

        let class_attr = if !classes.is_empty() {
            format!(r#" class="{}""#, classes.join(" "))
        } else {
            String::new()
        };

        assert_eq!(class_attr, "");
    }

    #[test]
    fn test_svg_viewbox_matches_dimensions() {
        let grid = Grid::new(80, 24, 1000);
        let result = encode(&grid, 14.0, 10);
        assert!(result.is_ok());

        let svg = String::from_utf8(result.unwrap()).unwrap();

        // Extract width and height from svg tag
        // With 80 cols, 24 rows, font_size 14.0, padding 10:
        // char_width = 8.4, line_height = 16.8
        // canvas_width = 80 * 8.4 + 20 = 692
        // canvas_height = 24 * 16.8 + 20 = 423.2 -> 423
        assert!(svg.contains(r#"width="692""#));
        assert!(svg.contains(r#"height="423""#));
        assert!(svg.contains(r#"viewBox="0 0 692 423""#));
    }

    #[test]
    fn test_x_position_calculation() {
        let start_col = 10;
        let font_size = 14.0;
        let char_width = font_size * 0.6;

        let x = start_col as f32 * char_width;

        // Allow small floating point differences
        assert!((x - 84.0).abs() < 0.01); // 10 * 8.4
    }
}
