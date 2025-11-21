/// Sixel graphics support for DEC VT340 compatible terminals
///
/// Sixel (Six Pixels) is a bitmap graphics format that encodes images as
/// vertical strips of 6 pixels. It was introduced by DEC for the VT240/VT340
/// terminals and is supported by many modern terminal emulators.
///
/// # Format
///
/// Sixel data is sent via DCS (Device Control String):
/// `DCS P1 ; P2 ; P3 ; q s...s ST`
///
/// Where:
/// - P1: Pixel aspect ratio (0-9, default 2:1)
/// - P2: Background mode (0=pixel, 1=transparent, 2=pixel)
/// - P3: Horizontal grid size (usually omitted)
/// - q: Sixel command indicator
/// - s...s: Sixel data
/// - ST: String terminator (ESC \ or 0x9C)
use std::collections::HashMap;

use crate::graphics::{GraphicProtocol, TerminalGraphic};

/// Hard upper bounds for Sixel resources. These are deliberately high but
/// finite, and are used to clamp user-configurable limits.
pub const SIXEL_HARD_MAX_WIDTH: usize = 4096;
pub const SIXEL_HARD_MAX_HEIGHT: usize = 4096;
pub const SIXEL_HARD_MAX_REPEAT: usize = 10_000;
pub const SIXEL_HARD_MAX_GRAPHICS: usize = 1024;

/// Default per-terminal Sixel limits. These can be overridden via API but
/// are themselves clamped to the hard maxima.
pub const SIXEL_DEFAULT_MAX_WIDTH: usize = 1024;
pub const SIXEL_DEFAULT_MAX_HEIGHT: usize = 1024;
pub const SIXEL_DEFAULT_MAX_REPEAT: usize = 10_000;
pub const SIXEL_DEFAULT_MAX_GRAPHICS: usize = 256;

/// Per-terminal Sixel resource limits
#[derive(Debug, Clone, Copy)]
pub struct SixelLimits {
    pub max_width: usize,
    pub max_height: usize,
    pub max_repeat: usize,
}

impl SixelLimits {
    pub fn new(max_width: usize, max_height: usize, max_repeat: usize) -> Self {
        Self {
            max_width: max_width.clamp(1, SIXEL_HARD_MAX_WIDTH),
            max_height: max_height.clamp(1, SIXEL_HARD_MAX_HEIGHT),
            max_repeat: max_repeat.clamp(1, SIXEL_HARD_MAX_REPEAT),
        }
    }
}

impl Default for SixelLimits {
    fn default() -> Self {
        Self::new(
            SIXEL_DEFAULT_MAX_WIDTH,
            SIXEL_DEFAULT_MAX_HEIGHT,
            SIXEL_DEFAULT_MAX_REPEAT,
        )
    }
}

/// RGB color value (0-255 for each component)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl SixelColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_hls(h: u16, l: u8, s: u8) -> Self {
        // Convert HLS to RGB
        // H: 0-360 degrees
        // L: 0-100 percent
        // S: 0-100 percent

        let l = l as f32 / 100.0;
        let s = s as f32 / 100.0;
        let h = (h % 360) as f32;

        if s == 0.0 {
            // Achromatic (gray)
            let gray = (l * 255.0) as u8;
            return Self::new(gray, gray, gray);
        }

        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };

        let p = 2.0 * l - q;

        let h = h / 360.0;

        let hue_to_rgb = |p: f32, q: f32, mut t: f32| -> u8 {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            let value = if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 1.0 / 2.0 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            };
            (value * 255.0) as u8
        };

        let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
        let g = hue_to_rgb(p, q, h);
        let b = hue_to_rgb(p, q, h - 1.0 / 3.0);

        Self::new(r, g, b)
    }

    pub fn from_rgb_percent(r: u8, g: u8, b: u8) -> Self {
        // Convert 0-100 percent to 0-255
        let r = ((r.min(100) as f32 / 100.0) * 255.0) as u8;
        let g = ((g.min(100) as f32 / 100.0) * 255.0) as u8;
        let b = ((b.min(100) as f32 / 100.0) * 255.0) as u8;
        Self::new(r, g, b)
    }
}

/// Global counter for unique sixel graphic IDs
static SIXEL_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A complete Sixel graphic
#[derive(Debug, Clone)]
pub struct SixelGraphic {
    /// Unique identifier for this graphic (stable across position changes)
    pub id: u64,
    /// Position where graphic was placed (column, row)
    pub position: (usize, usize),
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
    /// Pixel data (row-major order, RGBA format)
    pub pixels: Vec<u8>,
    /// Color palette used
    pub palette: HashMap<usize, SixelColor>,
    /// Cell dimensions when this graphic was created (width, height) in pixels
    /// Used to properly scale the graphic when rendering in screenshots
    pub cell_dimensions: Option<(u32, u32)>,
    /// Number of terminal rows that have scrolled off the top
    /// Used to render the correct portion when graphic is partially scrolled off-screen
    pub scroll_offset_rows: usize,
}

impl SixelGraphic {
    pub fn new(position: (usize, usize), width: usize, height: usize) -> Self {
        let id = SIXEL_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pixels = vec![0u8; width * height * 4]; // RGBA
        Self {
            id,
            position,
            width,
            height,
            pixels,
            palette: HashMap::new(),
            cell_dimensions: None,
            scroll_offset_rows: 0,
        }
    }

    /// Set the cell dimensions that were used when creating this graphic
    pub fn set_cell_dimensions(&mut self, cell_width: u32, cell_height: u32) {
        self.cell_dimensions = Some((cell_width, cell_height));
    }

    /// Calculate how many terminal cells this graphic spans
    /// Returns (columns, rows) based on either stored cell dimensions or provided ones
    pub fn cell_span(&self, fallback_cell_width: u32, fallback_cell_height: u32) -> (usize, usize) {
        let (cell_w, cell_h) = self
            .cell_dimensions
            .unwrap_or((fallback_cell_width, fallback_cell_height));
        let cols = (self.width as u32).div_ceil(cell_w) as usize;
        let rows = (self.height as u32).div_ceil(cell_h) as usize;
        (cols, rows)
    }

    /// Set a pixel at (x, y) with given color
    pub fn set_pixel(&mut self, x: usize, y: usize, color: SixelColor) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 4;
            self.pixels[idx] = color.r;
            self.pixels[idx + 1] = color.g;
            self.pixels[idx + 2] = color.b;
            self.pixels[idx + 3] = 255; // Alpha
        }
    }

    /// Get pixel color at (x, y) as (r, g, b, a)
    pub fn get_pixel(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 4;
            Some((
                self.pixels[idx],
                self.pixels[idx + 1],
                self.pixels[idx + 2],
                self.pixels[idx + 3],
            ))
        } else {
            None
        }
    }

    /// Convert to unified TerminalGraphic representation
    pub fn to_terminal_graphic(&self) -> TerminalGraphic {
        let mut graphic = TerminalGraphic::new(
            self.id,
            GraphicProtocol::Sixel,
            self.position,
            self.width,
            self.height,
            self.pixels.clone(),
        );
        if let Some((w, h)) = self.cell_dimensions {
            graphic.set_cell_dimensions(w, h);
        }
        graphic.scroll_offset_rows = self.scroll_offset_rows;
        graphic
    }
}

/// Sixel parser state machine
#[derive(Debug)]
pub struct SixelParser {
    /// Parameters from DCS (P1, P2, P3)
    params: Vec<u16>,
    /// Current color palette
    palette: HashMap<usize, SixelColor>,
    /// Current selected color index
    current_color: usize,
    /// Current cursor position (x, y) in pixels
    cursor: (usize, usize),
    /// Image data being built
    image_data: Vec<Vec<usize>>, // [row][col] = color_index
    /// Maximum width seen
    max_width: usize,
    /// Background mode (0=pixel, 1=transparent, 2=pixel)
    background_mode: u8,
    /// Raster attributes (width, height)
    raster_size: Option<(usize, usize)>,
    /// Resource limits for this parser
    limits: SixelLimits,
}

impl SixelParser {
    pub fn new() -> Self {
        Self::new_with_limits(SixelLimits::default())
    }

    /// Create a new parser with explicit limits
    pub fn new_with_limits(limits: SixelLimits) -> Self {
        let mut palette = HashMap::new();
        // Default VGA palette (first 16 colors)
        palette.insert(0, SixelColor::new(0, 0, 0)); // Black
        palette.insert(1, SixelColor::new(51, 102, 179)); // Blue
        palette.insert(2, SixelColor::new(204, 51, 51)); // Red
        palette.insert(3, SixelColor::new(51, 179, 51)); // Green
        palette.insert(4, SixelColor::new(179, 51, 179)); // Magenta
        palette.insert(5, SixelColor::new(51, 179, 179)); // Cyan
        palette.insert(6, SixelColor::new(179, 179, 51)); // Yellow
        palette.insert(7, SixelColor::new(179, 179, 179)); // Gray
        palette.insert(8, SixelColor::new(77, 77, 77)); // Dark Gray
        palette.insert(9, SixelColor::new(102, 153, 230)); // Light Blue
        palette.insert(10, SixelColor::new(230, 102, 102)); // Light Red
        palette.insert(11, SixelColor::new(102, 230, 102)); // Light Green
        palette.insert(12, SixelColor::new(230, 102, 230)); // Light Magenta
        palette.insert(13, SixelColor::new(102, 230, 230)); // Light Cyan
        palette.insert(14, SixelColor::new(230, 230, 102)); // Light Yellow
        palette.insert(15, SixelColor::new(255, 255, 255)); // White

        Self {
            params: Vec::new(),
            palette,
            current_color: 0,
            cursor: (0, 0),
            image_data: vec![vec![]],
            max_width: 0,
            background_mode: 0,
            raster_size: None,
            limits,
        }
    }

    pub fn set_params(&mut self, params: &[u16]) {
        self.params = params.to_vec();
        // P2 is background mode
        if params.len() >= 2 {
            self.background_mode = params[1] as u8;
        }
    }

    /// Parse a sixel data character (? to ~, hex 3F-7E)
    pub fn parse_sixel(&mut self, ch: char) {
        let code = ch as u8;
        if !(0x3F..=0x7E).contains(&code) {
            return;
        }

        // Sixel value is character code minus 0x3F
        let sixel_value = code - 0x3F;

        // Each bit represents a pixel (bit 0 = top, bit 5 = bottom)
        for bit in 0..6 {
            if sixel_value & (1 << bit) != 0 {
                let y = self.cursor.1 + bit;
                let x = self.cursor.0;

                // Enforce conservative bounds on image dimensions to prevent
                // unbounded growth from malicious or malformed input.
                if x >= self.limits.max_width || y >= self.limits.max_height {
                    continue;
                }

                // Expand image_data if needed
                while self.image_data.len() <= y {
                    self.image_data.push(vec![]);
                }

                while self.image_data[y].len() <= x {
                    self.image_data[y].push(0); // Background color
                }

                self.image_data[y][x] = self.current_color;
            }
        }

        // Move cursor right
        self.cursor.0 += 1;
        if self.cursor.0 < self.limits.max_width {
            self.max_width = self.max_width.max(self.cursor.0);
        }
    }

    /// Handle repeat sequence: !Pn character
    pub fn parse_repeat(&mut self, count: usize, ch: char) {
        let repeat = count.min(self.limits.max_repeat);
        for _ in 0..repeat {
            self.parse_sixel(ch);
        }
    }

    /// Handle graphics carriage return ($)
    pub fn carriage_return(&mut self) {
        self.cursor.0 = 0;
    }

    /// Handle graphics new line (-)
    pub fn new_line(&mut self) {
        self.cursor.0 = 0;
        self.cursor.1 += 6; // Move down by 6 pixels (one sixel height)
    }

    /// Select color (#Pc)
    pub fn select_color(&mut self, color_index: usize) {
        self.current_color = color_index;
    }

    /// Define color (#Pc;Pu;Px;Py;Pz)
    pub fn define_color(&mut self, index: usize, color_system: u8, x: u16, y: u16, z: u16) {
        let color = if color_system == 1 {
            // HLS
            SixelColor::from_hls(x, y as u8, z as u8)
        } else if color_system == 2 {
            // RGB (0-100 percent)
            SixelColor::from_rgb_percent(x as u8, y as u8, z as u8)
        } else {
            return;
        };

        self.palette.insert(index, color);
    }

    /// Set raster attributes ("Pan;Pad;Ph;Pv)
    pub fn set_raster_attributes(&mut self, _pan: u16, _pad: u16, width: usize, height: usize) {
        // Clamp raster size to the same conservative bounds used for image data.
        self.raster_size = Some((
            width.min(self.limits.max_width),
            height.min(self.limits.max_height),
        ));
    }

    /// Build final graphic from parsed data
    pub fn build_graphic(&self, position: (usize, usize)) -> SixelGraphic {
        let height = self.image_data.len();
        let width = self.max_width.max(1);

        // Use raster size if provided, otherwise use calculated size
        let (mut final_width, mut final_height) = self.raster_size.unwrap_or((width, height));

        // Enforce limits on final graphic size as a last line of defense.
        final_width = final_width.min(self.limits.max_width);
        final_height = final_height.min(self.limits.max_height);

        let mut graphic = SixelGraphic::new(position, final_width, final_height);
        graphic.palette = self.palette.clone();

        // Fill in pixels from image_data
        for (y, row) in self.image_data.iter().enumerate() {
            if y >= final_height {
                break;
            }
            for (x, &color_idx) in row.iter().enumerate() {
                if x >= final_width {
                    break;
                }
                if let Some(&color) = self.palette.get(&color_idx) {
                    graphic.set_pixel(x, y, color);
                }
            }
        }

        graphic
    }
}

impl Default for SixelParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sixel_color_from_rgb() {
        let color = SixelColor::from_rgb_percent(100, 50, 0);
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 127);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_sixel_parser_basic() {
        let mut parser = SixelParser::new();
        parser.set_params(&[0, 0, 0]);

        // Parse a simple sixel character
        parser.parse_sixel('?'); // 000000 (no pixels)
        parser.parse_sixel('~'); // 111111 (all pixels)

        assert_eq!(parser.cursor.0, 2);
        assert_eq!(parser.max_width, 2);
    }

    #[test]
    fn test_sixel_parser_color() {
        let mut parser = SixelParser::new();

        // Define a color (red in RGB)
        parser.define_color(1, 2, 100, 0, 0);

        let color = parser.palette.get(&1).unwrap();
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 0);
        assert_eq!(color.b, 0);
    }

    #[test]
    fn test_sixel_parser_carriage_return() {
        let mut parser = SixelParser::new();
        parser.parse_sixel('~');
        parser.parse_sixel('~');
        assert_eq!(parser.cursor.0, 2);

        parser.carriage_return();
        assert_eq!(parser.cursor.0, 0);
    }

    #[test]
    fn test_sixel_parser_new_line() {
        let mut parser = SixelParser::new();
        parser.parse_sixel('~');
        assert_eq!(parser.cursor.1, 0);

        parser.new_line();
        assert_eq!(parser.cursor.0, 0);
        assert_eq!(parser.cursor.1, 6);
    }

    #[test]
    fn test_sixel_graphic_set_get_pixel() {
        let mut graphic = SixelGraphic::new((0, 0), 10, 10);
        let color = SixelColor::new(255, 128, 64);

        graphic.set_pixel(5, 5, color);

        let pixel = graphic.get_pixel(5, 5).unwrap();
        assert_eq!(pixel, (255, 128, 64, 255));
    }

    #[test]
    fn test_sixel_parser_repeat() {
        let mut parser = SixelParser::new();
        parser.parse_repeat(5, '~');

        assert_eq!(parser.cursor.0, 5);
    }

    #[test]
    fn test_sixel_parser_repeat_clamped() {
        let limits = SixelLimits::new(128, 64, 1000);
        let mut parser = SixelParser::new_with_limits(limits);
        // Use a very large repeat count to ensure clamping kicks in
        parser.parse_repeat(limits.max_repeat * 10, '~');

        // Cursor advance should be limited by max_repeat
        assert_eq!(parser.cursor.0, limits.max_repeat);
        // And the internal image width should not exceed the configured limit
        assert!(parser
            .image_data
            .iter()
            .all(|row| row.len() <= limits.max_width));
    }

    #[test]
    fn test_sixel_parser_dimension_limits() {
        let limits = SixelLimits::new(64, 32, 1000);
        let mut parser = SixelParser::new_with_limits(limits);

        // Force cursor far beyond limits
        parser.cursor.0 = limits.max_width * 2;
        parser.cursor.1 = limits.max_height * 2;

        // Parsing should not panic or allocate unbounded memory
        parser.parse_sixel('~');

        // image_data should be bounded by configured limits
        assert!(parser.image_data.len() <= limits.max_height);
        assert!(parser
            .image_data
            .iter()
            .all(|row| row.len() <= limits.max_width));
        // max_width should remain 0 since cursor was beyond limits
        assert_eq!(parser.max_width, 0);
    }
}
