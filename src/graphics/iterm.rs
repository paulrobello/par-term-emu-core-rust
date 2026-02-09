//! iTerm2 OSC 1337 inline image support
//!
//! Parses iTerm2 inline image sequences:
//! `OSC 1337 ; File=name=<base64>;size=<bytes>;inline=1:<base64 data> ST`
//!
//! Reference: <https://iterm2.com/documentation-images.html>

use std::collections::HashMap;

use crate::graphics::{
    next_graphic_id, GraphicProtocol, GraphicsError, ImageDimension, ImageDisplayMode,
    ImagePlacement, TerminalGraphic,
};

/// iTerm2 inline image parser
#[derive(Debug, Default)]
pub struct ITermParser {
    /// Parsed parameters
    params: HashMap<String, String>,
    /// Base64-encoded image data
    data: Vec<u8>,
}

impl ITermParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse parameters from OSC 1337 File= sequence
    ///
    /// Format: `name=<base64>;size=<bytes>;width=<n>;height=<n>;inline=1`
    pub fn parse_params(&mut self, params_str: &str) -> Result<(), GraphicsError> {
        self.params.clear();

        for part in params_str.split(';') {
            if let Some((key, value)) = part.split_once('=') {
                self.params.insert(key.to_string(), value.to_string());
            }
        }

        // inline=1 is required for display
        match self.params.get("inline") {
            Some(v) if v == "1" => Ok(()),
            _ => Err(GraphicsError::ITermError(
                "inline=1 required for display".to_string(),
            )),
        }
    }

    /// Set the base64-encoded image data
    pub fn set_data(&mut self, data: &[u8]) {
        self.data = data.to_vec();
    }

    /// Decode the image and create a TerminalGraphic
    pub fn decode_image(&self, position: (usize, usize)) -> Result<TerminalGraphic, GraphicsError> {
        // Decode base64
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &self.data)
                .map_err(|e| GraphicsError::Base64Error(e.to_string()))?;

        // Decode image using image crate
        let img = image::load_from_memory(&decoded)
            .map_err(|e| GraphicsError::ImageError(e.to_string()))?;

        let rgba = img.to_rgba8();
        let width = rgba.width() as usize;
        let height = rgba.height() as usize;
        let pixels = rgba.into_raw();

        let mut graphic = TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::ITermInline,
            position,
            width,
            height,
            pixels,
        );

        // Build placement metadata from parsed parameters
        graphic.placement = self.build_placement();

        Ok(graphic)
    }

    /// Get a parameter value
    pub fn get_param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Build an ImagePlacement from the parsed parameters
    pub fn build_placement(&self) -> ImagePlacement {
        let display_mode = match self.params.get("inline") {
            Some(v) if v == "1" => ImageDisplayMode::Inline,
            _ => ImageDisplayMode::Download,
        };

        let requested_width = self
            .params
            .get("width")
            .map(|s| Self::parse_dimension(s))
            .unwrap_or_default();

        let requested_height = self
            .params
            .get("height")
            .map(|s| Self::parse_dimension(s))
            .unwrap_or_default();

        let preserve_aspect_ratio = self
            .params
            .get("preserveAspectRatio")
            .map(|v| v != "0")
            .unwrap_or(true); // Default is true per iTerm2 spec

        ImagePlacement {
            display_mode,
            requested_width,
            requested_height,
            preserve_aspect_ratio,
            ..Default::default()
        }
    }

    /// Parse an iTerm2 dimension string (e.g., "100", "50px", "80%", "auto", "10")
    ///
    /// iTerm2 dimension format:
    /// - N or Npx: N pixels
    /// - N%: N percent of terminal
    /// - "auto": automatic sizing
    /// - Plain number without suffix: cells
    fn parse_dimension(s: &str) -> ImageDimension {
        let s = s.trim();

        if s.eq_ignore_ascii_case("auto") || s == "0" {
            return ImageDimension::auto();
        }

        if let Some(stripped) = s.strip_suffix('%') {
            if let Ok(val) = stripped.parse::<f64>() {
                return ImageDimension::percent(val);
            }
        }

        if let Some(stripped) = s.strip_suffix("px") {
            if let Ok(val) = stripped.parse::<f64>() {
                return ImageDimension::pixels(val);
            }
        }

        // Plain number = cells per iTerm2 spec
        if let Ok(val) = s.parse::<f64>() {
            return ImageDimension::cells(val);
        }

        ImageDimension::auto()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_params_basic() {
        let mut parser = ITermParser::new();
        let result = parser.parse_params("inline=1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_params_missing_inline() {
        let mut parser = ITermParser::new();
        let result = parser.parse_params("name=test");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_params_full() {
        let mut parser = ITermParser::new();
        let result = parser.parse_params("name=dGVzdA==;size=1234;width=100;height=50;inline=1");
        assert!(result.is_ok());
        assert_eq!(parser.get_param("name"), Some("dGVzdA=="));
        assert_eq!(parser.get_param("size"), Some("1234"));
        assert_eq!(parser.get_param("width"), Some("100"));
        assert_eq!(parser.get_param("height"), Some("50"));
    }

    #[test]
    fn test_parse_dimension_auto() {
        let dim = ITermParser::parse_dimension("auto");
        assert!(dim.is_auto());
        assert_eq!(dim.unit, crate::graphics::ImageSizeUnit::Auto);
    }

    #[test]
    fn test_parse_dimension_zero() {
        let dim = ITermParser::parse_dimension("0");
        assert!(dim.is_auto());
    }

    #[test]
    fn test_parse_dimension_cells() {
        let dim = ITermParser::parse_dimension("10");
        assert_eq!(dim.value, 10.0);
        assert_eq!(dim.unit, crate::graphics::ImageSizeUnit::Cells);
    }

    #[test]
    fn test_parse_dimension_pixels() {
        let dim = ITermParser::parse_dimension("200px");
        assert_eq!(dim.value, 200.0);
        assert_eq!(dim.unit, crate::graphics::ImageSizeUnit::Pixels);
    }

    #[test]
    fn test_parse_dimension_percent() {
        let dim = ITermParser::parse_dimension("50%");
        assert_eq!(dim.value, 50.0);
        assert_eq!(dim.unit, crate::graphics::ImageSizeUnit::Percent);
    }

    #[test]
    fn test_parse_dimension_invalid() {
        let dim = ITermParser::parse_dimension("invalid");
        assert!(dim.is_auto());
    }

    #[test]
    fn test_build_placement_basic() {
        let mut parser = ITermParser::new();
        parser.parse_params("inline=1").unwrap();
        let placement = parser.build_placement();
        assert_eq!(
            placement.display_mode,
            crate::graphics::ImageDisplayMode::Inline
        );
        assert!(placement.preserve_aspect_ratio);
        assert!(placement.requested_width.is_auto());
        assert!(placement.requested_height.is_auto());
    }

    #[test]
    fn test_build_placement_with_dimensions() {
        let mut parser = ITermParser::new();
        parser
            .parse_params("inline=1;width=100px;height=50%")
            .unwrap();
        let placement = parser.build_placement();

        assert_eq!(placement.requested_width.value, 100.0);
        assert_eq!(
            placement.requested_width.unit,
            crate::graphics::ImageSizeUnit::Pixels
        );
        assert_eq!(placement.requested_height.value, 50.0);
        assert_eq!(
            placement.requested_height.unit,
            crate::graphics::ImageSizeUnit::Percent
        );
    }

    #[test]
    fn test_build_placement_preserve_aspect_ratio() {
        let mut parser = ITermParser::new();
        parser
            .parse_params("inline=1;preserveAspectRatio=0")
            .unwrap();
        let placement = parser.build_placement();
        assert!(!placement.preserve_aspect_ratio);

        let mut parser2 = ITermParser::new();
        parser2
            .parse_params("inline=1;preserveAspectRatio=1")
            .unwrap();
        let placement2 = parser2.build_placement();
        assert!(placement2.preserve_aspect_ratio);
    }

    #[test]
    fn test_build_placement_download_mode() {
        let mut parser = ITermParser::new();
        // parse_params fails without inline=1, but build_placement works on whatever was parsed
        parser.params.insert("inline".to_string(), "0".to_string());
        let placement = parser.build_placement();
        assert_eq!(
            placement.display_mode,
            crate::graphics::ImageDisplayMode::Download
        );
    }
}
