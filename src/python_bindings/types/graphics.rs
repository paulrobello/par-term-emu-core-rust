//! Image / graphics protocol types (Sixel, iTerm2, Kitty).
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Type alias for half-block rendering colors
/// Tuple contains: ((top_r, top_g, top_b, top_a), (bottom_r, bottom_g, bottom_b, bottom_a))
type HalfBlockColors = ((u8, u8, u8, u8), (u8, u8, u8, u8));

/// Image dimension with unit for sizing
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ImageDimension", from_py_object)]
#[derive(Clone)]
pub struct PyImageDimension {
    /// Numeric value (0 means auto)
    pub value: f64,
    /// Unit: "auto", "cells", "pixels", or "percent"
    pub unit: String,
}

#[pymethods]
impl PyImageDimension {
    /// Check if this is an auto dimension
    fn is_auto(&self) -> bool {
        self.unit == "auto" || self.value == 0.0
    }

    fn __repr__(&self) -> PyResult<String> {
        if self.is_auto() {
            Ok("ImageDimension(auto)".to_string())
        } else {
            Ok(format!("ImageDimension({} {})", self.value, self.unit))
        }
    }
}

impl From<&crate::graphics::ImageDimension> for PyImageDimension {
    fn from(dim: &crate::graphics::ImageDimension) -> Self {
        Self {
            value: dim.value,
            unit: dim.unit.as_str().to_string(),
        }
    }
}

/// Image placement metadata for rendering
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ImagePlacement", from_py_object)]
#[derive(Clone)]
pub struct PyImagePlacement {
    /// Display mode: "inline" or "download"
    pub display_mode: String,
    /// Requested width dimension
    pub requested_width: PyImageDimension,
    /// Requested height dimension
    pub requested_height: PyImageDimension,
    /// Whether to preserve aspect ratio when scaling
    pub preserve_aspect_ratio: bool,
    /// Number of columns to display (Kitty)
    pub columns: Option<u32>,
    /// Number of rows to display (Kitty)
    pub rows: Option<u32>,
    /// Z-index for layering
    pub z_index: i32,
    /// X offset within the cell in pixels
    pub x_offset: u32,
    /// Y offset within the cell in pixels
    pub y_offset: u32,
}

#[pymethods]
impl PyImagePlacement {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ImagePlacement(mode='{}', preserve_aspect_ratio={}, z_index={})",
            self.display_mode, self.preserve_aspect_ratio, self.z_index
        ))
    }
}

impl From<&crate::graphics::ImagePlacement> for PyImagePlacement {
    fn from(placement: &crate::graphics::ImagePlacement) -> Self {
        Self {
            display_mode: placement.display_mode.as_str().to_string(),
            requested_width: PyImageDimension::from(&placement.requested_width),
            requested_height: PyImageDimension::from(&placement.requested_height),
            preserve_aspect_ratio: placement.preserve_aspect_ratio,
            columns: placement.columns,
            rows: placement.rows,
            z_index: placement.z_index,
            x_offset: placement.x_offset,
            y_offset: placement.y_offset,
        }
    }
}

/// Graphics representation (Sixel, iTerm2, or Kitty)
#[pyclass(name = "Graphic", from_py_object)]
#[derive(Clone)]
pub struct PyGraphic {
    #[pyo3(get)]
    pub id: u64,
    #[pyo3(get)]
    pub protocol: String,
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub width: usize,
    #[pyo3(get)]
    pub height: usize,
    #[pyo3(get)]
    pub original_width: usize,
    #[pyo3(get)]
    pub original_height: usize,
    #[pyo3(get)]
    pub scroll_offset_rows: usize,
    #[pyo3(get)]
    pub cell_dimensions: Option<(u32, u32)>,
    #[pyo3(get)]
    pub was_compressed: bool,
    #[pyo3(get)]
    pub placement: PyImagePlacement,
    pixels: Vec<u8>,
}

#[pymethods]
impl PyGraphic {
    /// Get pixel color at (x, y) coordinates
    ///
    /// Args:
    ///     x: X coordinate (0-based)
    ///     y: Y coordinate (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b, a) values, or None if out of bounds
    fn get_pixel(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        crate::graphics::pixel_at_in(&self.pixels, self.width, self.height, x, y)
    }

    /// Get raw pixel data as bytes (RGBA format)
    ///
    /// Returns:
    ///     Bytes containing RGBA pixel data in row-major order
    fn pixels(&self) -> Vec<u8> {
        self.pixels.clone()
    }

    /// Get size in terminal cells
    fn cell_size(&self, cell_width: u32, cell_height: u32) -> (usize, usize) {
        crate::graphics::cell_size_for(self.width, self.height, cell_width, cell_height)
    }

    /// Sample for half-block rendering at cell (col, row)
    /// Returns ((top_r, top_g, top_b, top_a), (bottom_r, bottom_g, bottom_b, bottom_a))
    fn sample_half_block(
        &self,
        cell_col: usize,
        cell_row: usize,
        cell_width: u32,
        cell_height: u32,
    ) -> Option<HalfBlockColors> {
        crate::graphics::sample_half_block_in(
            &self.pixels,
            self.width,
            self.height,
            self.position,
            cell_col,
            cell_row,
            cell_width,
            cell_height,
        )
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Graphic(id={}, protocol='{}', position=({},{}), size={}x{}, original_size={}x{})",
            self.id,
            self.protocol,
            self.position.0,
            self.position.1,
            self.width,
            self.height,
            self.original_width,
            self.original_height
        ))
    }
}

impl From<&crate::sixel::SixelGraphic> for PyGraphic {
    fn from(graphic: &crate::sixel::SixelGraphic) -> Self {
        Self {
            id: graphic.id,
            protocol: "sixel".to_string(),
            position: graphic.position,
            width: graphic.width,
            height: graphic.height,
            original_width: graphic.width,
            original_height: graphic.height,
            scroll_offset_rows: graphic.scroll_offset_rows,
            cell_dimensions: graphic.cell_dimensions,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: graphic.pixels.clone(),
        }
    }
}

impl From<&crate::graphics::TerminalGraphic> for PyGraphic {
    fn from(graphic: &crate::graphics::TerminalGraphic) -> Self {
        Self {
            id: graphic.id,
            protocol: graphic.protocol.as_str().to_string(),
            position: graphic.position,
            width: graphic.width,
            height: graphic.height,
            original_width: graphic.original_width,
            original_height: graphic.original_height,
            scroll_offset_rows: graphic.scroll_offset_rows,
            cell_dimensions: graphic.cell_dimensions,
            was_compressed: graphic.was_compressed,
            placement: PyImagePlacement::from(&graphic.placement),
            pixels: (*graphic.pixels).clone(),
        }
    }
}

/// Image protocol
#[pyclass(name = "ImageProtocol", from_py_object)]
#[derive(Clone)]
pub enum PyImageProtocol {
    Sixel,
    ITerm2,
    Kitty,
}

/// Image format
#[pyclass(name = "ImageFormat", from_py_object)]
#[derive(Clone)]
pub enum PyImageFormat {
    PNG,
    JPEG,
    GIF,
    BMP,
    RGBA,
    RGB,
}

/// Inline image
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "InlineImage", from_py_object)]
#[derive(Clone)]
pub struct PyInlineImage {
    pub id: Option<String>,
    pub protocol: String,
    pub format: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub position: (usize, usize),
    pub display_cols: usize,
    pub display_rows: usize,
}

#[pymethods]
impl PyInlineImage {
    fn __repr__(&self) -> String {
        format!(
            "InlineImage(protocol={}, format={}, size={}x{}, pos={:?})",
            self.protocol, self.format, self.width, self.height, self.position
        )
    }
}

impl From<&crate::terminal::InlineImage> for PyInlineImage {
    fn from(img: &crate::terminal::InlineImage) -> Self {
        use crate::terminal::{ImageFormat, ImageProtocol};

        let protocol = match img.protocol {
            ImageProtocol::Sixel => "sixel",
            ImageProtocol::ITerm2 => "iterm2",
            ImageProtocol::Kitty => "kitty",
        }
        .to_string();

        let format = match img.format {
            ImageFormat::PNG => "png",
            ImageFormat::JPEG => "jpeg",
            ImageFormat::GIF => "gif",
            ImageFormat::BMP => "bmp",
            ImageFormat::RGBA => "rgba",
            ImageFormat::RGB => "rgb",
        }
        .to_string();

        PyInlineImage {
            id: img.id.clone(),
            protocol,
            format,
            data: img.data.clone(),
            width: img.width,
            height: img.height,
            position: img.position,
            display_cols: img.display_cols,
            display_rows: img.display_rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pygraphic_get_pixel_valid() {
        // Create a 2x2 pixel graphic with RGBA data
        let pixels = vec![
            255, 0, 0, 255, // Red pixel at (0, 0)
            0, 255, 0, 255, // Green pixel at (1, 0)
            0, 0, 255, 255, // Blue pixel at (0, 1)
            255, 255, 0, 255, // Yellow pixel at (1, 1)
        ];

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            original_width: 2,
            original_height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 255))); // Red
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 255))); // Green
        assert_eq!(graphic.get_pixel(0, 1), Some((0, 0, 255, 255))); // Blue
        assert_eq!(graphic.get_pixel(1, 1), Some((255, 255, 0, 255))); // Yellow
    }

    #[test]
    fn test_pygraphic_get_pixel_out_of_bounds() {
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            original_width: 2,
            original_height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: vec![0; 16], // 2x2 RGBA
        };

        assert_eq!(graphic.get_pixel(2, 0), None); // X out of bounds
        assert_eq!(graphic.get_pixel(0, 2), None); // Y out of bounds
        assert_eq!(graphic.get_pixel(2, 2), None); // Both out of bounds
    }

    #[test]
    fn test_pygraphic_get_pixel_edge_cases() {
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (5, 10),
            width: 3,
            height: 3,
            original_width: 3,
            original_height: 3,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: vec![128; 36], // 3x3 RGBA with all values at 128
        };

        // Test valid edge pixels
        assert_eq!(graphic.get_pixel(0, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(0, 2), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 2), Some((128, 128, 128, 128)));

        // Test just outside bounds
        assert_eq!(graphic.get_pixel(3, 0), None);
        assert_eq!(graphic.get_pixel(0, 3), None);
    }

    #[test]
    fn test_pygraphic_pixels_returns_copy() {
        let original_pixels = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 1,
            original_width: 2,
            original_height: 1,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: original_pixels.clone(),
        };

        let retrieved = graphic.pixels();
        assert_eq!(retrieved, original_pixels);
        assert_eq!(retrieved.len(), 8); // 2 pixels * 4 channels
    }

    #[test]
    fn test_pygraphic_repr() {
        let graphic = PyGraphic {
            id: 42,
            protocol: "sixel".to_string(),
            position: (10, 20),
            width: 100,
            height: 50,
            original_width: 100,
            original_height: 50,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: vec![],
        };

        let repr = graphic.__repr__().unwrap();
        assert!(repr.contains("id=42"));
        assert!(repr.contains("protocol='sixel'"));
        assert!(repr.contains("position=(10,20)"));
        assert!(repr.contains("size=100x50"));
    }

    #[test]
    fn test_pygraphic_clone() {
        let graphic1 = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (5, 10),
            width: 20,
            height: 30,
            original_width: 20,
            original_height: 30,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels: vec![1, 2, 3, 4],
        };

        let graphic2 = graphic1.clone();

        assert_eq!(graphic1.id, graphic2.id);
        assert_eq!(graphic1.protocol, graphic2.protocol);
        assert_eq!(graphic1.position, graphic2.position);
        assert_eq!(graphic1.width, graphic2.width);
        assert_eq!(graphic1.height, graphic2.height);
        assert_eq!(graphic1.pixels(), graphic2.pixels());
    }

    #[test]
    fn test_pygraphic_pixel_index_calculation() {
        // Test that pixel indexing is calculated correctly
        let mut pixels = vec![0u8; 16]; // 2x2 grid, RGBA

        // Manually set pixel at (1, 1) to red
        let x = 1usize;
        let y = 1usize;
        let width = 2usize;
        let idx = (y * width + x) * 4;

        pixels[idx] = 255; // R
        pixels[idx + 1] = 0; // G
        pixels[idx + 2] = 0; // B
        pixels[idx + 3] = 255; // A

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 2,
            height: 2,
            original_width: 2,
            original_height: 2,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels,
        };

        assert_eq!(graphic.get_pixel(1, 1), Some((255, 0, 0, 255)));
    }

    #[test]
    fn test_pygraphic_alpha_channel() {
        // Test graphics with various alpha values
        let pixels = vec![
            255, 0, 0, 0, // Red, fully transparent
            0, 255, 0, 128, // Green, semi-transparent
            0, 0, 255, 255, // Blue, fully opaque
            128, 128, 128, 64, // Gray, mostly transparent
        ];

        let graphic = PyGraphic {
            id: 1,
            protocol: "sixel".to_string(),
            position: (0, 0),
            width: 4,
            height: 1,
            original_width: 4,
            original_height: 1,
            scroll_offset_rows: 0,
            cell_dimensions: None,
            was_compressed: false,
            placement: PyImagePlacement::from(&crate::graphics::ImagePlacement::inline()),
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 0)));
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((0, 0, 255, 255)));
        assert_eq!(graphic.get_pixel(3, 0), Some((128, 128, 128, 64)));
    }
}
