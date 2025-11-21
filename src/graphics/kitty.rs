//! Kitty graphics protocol support
//!
//! Parses Kitty APC graphics sequences:
//! `APC G <key>=<value>,<key>=<value>;<base64-data> ST`
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use std::collections::HashMap;

use crate::graphics::{
    next_graphic_id, GraphicProtocol, GraphicsError, GraphicsStore, TerminalGraphic,
};

/// Kitty graphics transmission action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyAction {
    #[default]
    Transmit, // t - transmit image data
    TransmitDisplay,  // T - transmit and display
    Query,            // q - query terminal support
    Put,              // p - display previously transmitted image
    Delete,           // d - delete images
    Frame,            // f - animation frame
    AnimationControl, // a - animation control
}

impl KittyAction {
    /// Parse action character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            't' => Some(KittyAction::Transmit),
            'T' => Some(KittyAction::TransmitDisplay),
            'q' => Some(KittyAction::Query),
            'p' => Some(KittyAction::Put),
            'd' => Some(KittyAction::Delete),
            'f' => Some(KittyAction::Frame),
            'a' => Some(KittyAction::AnimationControl),
            _ => None,
        }
    }
}

/// Kitty transmission format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyFormat {
    #[default]
    Rgba, // 32 - 32-bit RGBA
    Rgb, // 24 - 24-bit RGB
    Png, // 100 - PNG compressed
}

impl KittyFormat {
    /// Parse format code
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            24 => Some(KittyFormat::Rgb),
            32 => Some(KittyFormat::Rgba),
            100 => Some(KittyFormat::Png),
            _ => None,
        }
    }
}

/// Kitty transmission medium
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyMedium {
    #[default]
    Direct, // d - direct in-band data
    File,      // f - read from file
    TempFile,  // t - read from temp file and delete
    SharedMem, // s - read from shared memory
}

impl KittyMedium {
    /// Parse medium character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'd' => Some(KittyMedium::Direct),
            'f' => Some(KittyMedium::File),
            't' => Some(KittyMedium::TempFile),
            's' => Some(KittyMedium::SharedMem),
            _ => None,
        }
    }
}

/// Kitty delete target
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyDeleteTarget {
    All,                           // a - all images
    ById(u32),                     // i - by image id
    ByPlacement(u32, Option<u32>), // (image_id, placement_id)
    AtCursor,                      // c - at cursor position
    InCell,                        // p - at specific cell
    OnScreen,                      // z - visible on screen
    ByColumn(u32),                 // x - in column
    ByRow(u32),                    // y - in row
}

/// Kitty graphics parser
#[derive(Debug, Default)]
pub struct KittyParser {
    /// Current action
    pub action: KittyAction,
    /// Image ID for reuse
    pub image_id: Option<u32>,
    /// Placement ID
    pub placement_id: Option<u32>,
    /// Transmission format
    pub format: KittyFormat,
    /// Transmission medium
    pub medium: KittyMedium,
    /// Image width
    pub width: Option<u32>,
    /// Image height
    pub height: Option<u32>,
    /// Columns to display (for scaling)
    pub columns: Option<u32>,
    /// Rows to display (for scaling)
    pub rows: Option<u32>,
    /// X offset within cell
    pub x_offset: Option<u32>,
    /// Y offset within cell
    pub y_offset: Option<u32>,
    /// More chunks expected
    pub more_chunks: bool,
    /// Accumulated data chunks
    data_chunks: Vec<Vec<u8>>,
    /// Delete target
    pub delete_target: Option<KittyDeleteTarget>,
    /// Raw parameters for debugging
    params: HashMap<String, String>,
}

impl KittyParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset parser state for new transmission
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Parse a Kitty graphics payload
    ///
    /// Format: `key=value,key=value,...;base64data`
    pub fn parse_chunk(&mut self, payload: &str) -> Result<bool, GraphicsError> {
        // Split into params and data
        let (params_str, data_str) = payload.split_once(';').unwrap_or((payload, ""));

        // Parse key=value pairs
        for pair in params_str.split(',') {
            if let Some((key, value)) = pair.split_once('=') {
                self.params.insert(key.to_string(), value.to_string());

                match key {
                    "a" => {
                        if let Some(c) = value.chars().next() {
                            self.action = KittyAction::from_char(c).unwrap_or_default();
                        }
                    }
                    "f" => {
                        if let Ok(code) = value.parse::<u32>() {
                            self.format = KittyFormat::from_code(code).unwrap_or_default();
                        }
                    }
                    "t" => {
                        if let Some(c) = value.chars().next() {
                            self.medium = KittyMedium::from_char(c).unwrap_or_default();
                        }
                    }
                    "i" => {
                        self.image_id = value.parse().ok();
                    }
                    "p" => {
                        self.placement_id = value.parse().ok();
                    }
                    "s" => {
                        self.width = value.parse().ok();
                    }
                    "v" => {
                        self.height = value.parse().ok();
                    }
                    "c" => {
                        self.columns = value.parse().ok();
                    }
                    "r" => {
                        self.rows = value.parse().ok();
                    }
                    "x" => {
                        self.x_offset = value.parse().ok();
                    }
                    "y" => {
                        self.y_offset = value.parse().ok();
                    }
                    "m" => {
                        self.more_chunks = value == "1";
                    }
                    "d" => {
                        // Delete specification
                        self.parse_delete_target(value);
                    }
                    _ => {}
                }
            }
        }

        // Decode and accumulate base64 data
        if !data_str.is_empty() {
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_str)
                    .map_err(|e| GraphicsError::Base64Error(e.to_string()))?;
            self.data_chunks.push(decoded);
        }

        // Return true if more chunks expected
        Ok(self.more_chunks)
    }

    /// Parse delete target specification
    fn parse_delete_target(&mut self, value: &str) {
        if let Some(c) = value.chars().next() {
            self.delete_target = match c {
                'a' | 'A' => Some(KittyDeleteTarget::All),
                'c' | 'C' => Some(KittyDeleteTarget::AtCursor),
                'z' | 'Z' => Some(KittyDeleteTarget::OnScreen),
                _ => None,
            };
        }
    }

    /// Get accumulated data
    pub fn get_data(&self) -> Vec<u8> {
        self.data_chunks.concat()
    }

    /// Build a TerminalGraphic from parsed data
    pub fn build_graphic(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<Option<TerminalGraphic>, GraphicsError> {
        match self.action {
            KittyAction::Delete => {
                // Handle delete
                if let Some(target) = &self.delete_target {
                    match target {
                        KittyDeleteTarget::All => store.clear(),
                        KittyDeleteTarget::ById(id) => {
                            store.delete_kitty_graphics(Some(*id), None);
                        }
                        KittyDeleteTarget::ByPlacement(iid, pid) => {
                            store.delete_kitty_graphics(Some(*iid), *pid);
                        }
                        _ => {} // TODO: implement other delete targets
                    }
                }
                Ok(None)
            }

            KittyAction::Query => {
                // Query doesn't create a graphic
                Ok(None)
            }

            KittyAction::Put => {
                // Display previously transmitted image
                if let Some(image_id) = self.image_id {
                    if let Some((width, height, pixels)) = store.get_kitty_image(image_id) {
                        let mut graphic = TerminalGraphic::with_shared_pixels(
                            next_graphic_id(),
                            GraphicProtocol::Kitty,
                            position,
                            width,
                            height,
                            pixels,
                        );
                        graphic.kitty_image_id = Some(image_id);
                        graphic.kitty_placement_id = self.placement_id;
                        return Ok(Some(graphic));
                    }
                }
                Err(GraphicsError::KittyError("Image not found".to_string()))
            }

            KittyAction::Transmit | KittyAction::TransmitDisplay => {
                let data = self.get_data();
                if data.is_empty() {
                    return Err(GraphicsError::KittyError("No image data".to_string()));
                }

                let (width, height, pixels) = self.decode_pixels(&data)?;

                // Store for reuse if image_id is specified
                if let Some(image_id) = self.image_id {
                    store.store_kitty_image(image_id, width, height, pixels.clone());
                }

                // Create graphic if TransmitDisplay or Put
                if self.action == KittyAction::TransmitDisplay {
                    let mut graphic = TerminalGraphic::new(
                        next_graphic_id(),
                        GraphicProtocol::Kitty,
                        position,
                        width,
                        height,
                        pixels,
                    );
                    graphic.kitty_image_id = self.image_id;
                    graphic.kitty_placement_id = self.placement_id;
                    Ok(Some(graphic))
                } else {
                    // Transmit only, no display
                    Ok(None)
                }
            }

            _ => Ok(None),
        }
    }

    /// Decode pixels based on format
    fn decode_pixels(&self, data: &[u8]) -> Result<(usize, usize, Vec<u8>), GraphicsError> {
        match self.format {
            KittyFormat::Png => {
                // Decode PNG
                let img = image::load_from_memory(data)
                    .map_err(|e| GraphicsError::ImageError(e.to_string()))?;
                let rgba = img.to_rgba8();
                let width = rgba.width() as usize;
                let height = rgba.height() as usize;
                Ok((width, height, rgba.into_raw()))
            }

            KittyFormat::Rgba => {
                // Raw RGBA data
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;

                if data.len() != width * height * 4 {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        width * height * 4
                    )));
                }
                Ok((width, height, data.to_vec()))
            }

            KittyFormat::Rgb => {
                // Raw RGB data - convert to RGBA
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;

                if data.len() != width * height * 3 {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        width * height * 3
                    )));
                }

                // Convert RGB to RGBA
                let mut rgba = Vec::with_capacity(width * height * 4);
                for chunk in data.chunks(3) {
                    rgba.push(chunk[0]);
                    rgba.push(chunk[1]);
                    rgba.push(chunk[2]);
                    rgba.push(255); // Alpha
                }
                Ok((width, height, rgba))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kitty_action_from_char() {
        assert_eq!(KittyAction::from_char('t'), Some(KittyAction::Transmit));
        assert_eq!(
            KittyAction::from_char('T'),
            Some(KittyAction::TransmitDisplay)
        );
        assert_eq!(KittyAction::from_char('q'), Some(KittyAction::Query));
        assert_eq!(KittyAction::from_char('p'), Some(KittyAction::Put));
        assert_eq!(KittyAction::from_char('d'), Some(KittyAction::Delete));
        assert_eq!(KittyAction::from_char('x'), None);
    }

    #[test]
    fn test_kitty_format_from_code() {
        assert_eq!(KittyFormat::from_code(24), Some(KittyFormat::Rgb));
        assert_eq!(KittyFormat::from_code(32), Some(KittyFormat::Rgba));
        assert_eq!(KittyFormat::from_code(100), Some(KittyFormat::Png));
        assert_eq!(KittyFormat::from_code(0), None);
    }

    #[test]
    fn test_kitty_parser_basic() {
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T,f=100,i=1;");
        assert!(result.is_ok());
        assert_eq!(parser.action, KittyAction::TransmitDisplay);
        assert_eq!(parser.format, KittyFormat::Png);
        assert_eq!(parser.image_id, Some(1));
    }

    #[test]
    fn test_kitty_parser_chunked() {
        let mut parser = KittyParser::new();

        // First chunk
        let result = parser.parse_chunk("a=T,f=100,m=1;AAAA");
        assert!(result.is_ok());
        assert!(result.unwrap()); // more_chunks = true

        // Final chunk
        let result = parser.parse_chunk("m=0;BBBB");
        assert!(result.is_ok());
        assert!(!result.unwrap()); // more_chunks = false
    }
}
