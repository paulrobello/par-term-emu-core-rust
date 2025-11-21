//! iTerm2 OSC 1337 inline image support
//!
//! Parses iTerm2 inline image sequences:
//! `OSC 1337 ; File=name=<base64>;size=<bytes>;inline=1:<base64 data> ST`
//!
//! Reference: <https://iterm2.com/documentation-images.html>

use std::collections::HashMap;

use crate::graphics::{next_graphic_id, GraphicProtocol, GraphicsError, TerminalGraphic};

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

        let graphic = TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::ITermInline,
            position,
            width,
            height,
            pixels,
        );

        // Apply size parameters if provided
        // TODO: Handle width/height parameters for scaling

        Ok(graphic)
    }

    /// Get a parameter value
    pub fn get_param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
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
}
