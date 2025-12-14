//! Kitty graphics protocol support
//!
//! Parses Kitty APC graphics sequences:
//! `APC G <key>=<value>,<key>=<value>;<base64-data> ST`
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::graphics::{
    next_graphic_id, AnimationControl, AnimationFrame, CompositionMode, GraphicProtocol,
    GraphicsError, GraphicsStore, TerminalGraphic,
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

/// Result of building a Kitty graphic
#[derive(Debug, Clone)]
pub enum KittyGraphicResult {
    /// A regular graphic that should be displayed
    Graphic(TerminalGraphic),
    /// A virtual placement - insert Unicode placeholders into grid
    VirtualPlacement {
        image_id: u32,
        placement_id: u32,
        position: (usize, usize),
        cols: usize,
        rows: usize,
    },
    /// Command processed but no output (delete, query, transmit-only, etc.)
    None,
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
    /// Virtual placement (U=1)
    pub is_virtual: bool,
    /// Parent image ID for relative positioning (P= key)
    pub parent_image_id: Option<u32>,
    /// Parent placement ID for relative positioning (Q= key)
    pub parent_placement_id: Option<u32>,
    /// Relative X offset (H= key) in pixels
    pub relative_x_offset: Option<i32>,
    /// Relative Y offset (V= key) in pixels
    pub relative_y_offset: Option<i32>,
    /// Frame number for animation
    pub frame_number: Option<u32>,
    /// Frame delay in milliseconds
    pub frame_delay_ms: Option<u32>,
    /// Frame composition mode
    pub frame_composition: Option<CompositionMode>,
    /// Animation control
    pub animation_control: Option<AnimationControl>,
    /// Number of times to play animation (v= parameter)
    /// Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
    pub num_plays: Option<u32>,
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
                        // Animation control state (for AnimationControl action) takes priority
                        if self.action == KittyAction::AnimationControl {
                            self.animation_control = AnimationControl::from_value(value);
                            debug_log!(
                                "KITTY",
                                "Parsed animation control: s={} -> {:?}",
                                value,
                                self.animation_control
                            );
                        } else {
                            // Otherwise it's width
                            self.width = value.parse().ok();
                        }
                    }
                    "v" => {
                        // v= is overloaded: height for images, num_plays for animation control
                        if self.action == KittyAction::AnimationControl {
                            // Number of times to play animation (v= for animation control)
                            // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
                            self.num_plays = value.parse().ok();
                        } else {
                            // Height for image transmission/display
                            self.height = value.parse().ok();
                        }
                    }
                    "c" => {
                        // Frame composition mode (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            if let Some(first_char) = value.chars().next() {
                                self.frame_composition = CompositionMode::from_char(first_char);
                            }
                        } else {
                            // Otherwise it's columns
                            self.columns = value.parse().ok();
                        }
                    }
                    "r" => {
                        // Frame number (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            self.frame_number = value.parse().ok();
                        } else {
                            // Otherwise it's rows
                            self.rows = value.parse().ok();
                        }
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
                    "U" => {
                        // Virtual placement
                        self.is_virtual = value == "1";
                    }
                    "P" => {
                        // Parent image ID for relative positioning
                        self.parent_image_id = value.parse().ok();
                    }
                    "Q" => {
                        // Parent placement ID for relative positioning
                        self.parent_placement_id = value.parse().ok();
                    }
                    "H" => {
                        // Relative X offset in pixels
                        self.relative_x_offset = value.parse().ok();
                    }
                    "V" => {
                        // Relative Y offset in pixels (note: different from v=height)
                        // Only parse as relative offset if we have parent placement
                        if self.parent_image_id.is_some() {
                            self.relative_y_offset = value.parse().ok();
                        }
                    }
                    "z" => {
                        // Frame delay in milliseconds (for animations)
                        self.frame_delay_ms = value.parse().ok();
                    }
                    _ => {}
                }
            }
        }

        // Decode and accumulate base64 data
        if !data_str.is_empty() {
            // Try STANDARD first (with padding), then NO_PAD if that fails
            // This handles both padded and unpadded base64 (Kitty allows both)
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_str)
                    .or_else(|_| {
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD_NO_PAD,
                            data_str,
                        )
                    })
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
    ) -> Result<KittyGraphicResult, GraphicsError> {
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
                Ok(KittyGraphicResult::None)
            }

            KittyAction::Query => {
                // Query doesn't create a graphic
                Ok(KittyGraphicResult::None)
            }

            KittyAction::Put => {
                // Display previously transmitted image or create virtual placement
                let image_id = self.image_id.unwrap_or(0);

                // If U=1, create a virtual placement
                if self.is_virtual {
                    let cols = self.columns.unwrap_or(1) as usize;
                    let rows = self.rows.unwrap_or(1) as usize;
                    let placement_id = self.placement_id.unwrap_or(0);

                    // Create virtual placement without image data
                    let mut graphic = TerminalGraphic::new(
                        next_graphic_id(),
                        GraphicProtocol::Kitty,
                        position,
                        cols,
                        rows,
                        vec![], // Virtual placements don't need pixel data
                    );
                    graphic.kitty_image_id = Some(image_id);
                    graphic.kitty_placement_id = Some(placement_id);
                    graphic.is_virtual = true;
                    store.add_virtual_placement(graphic);

                    // Return virtual placement info for placeholder insertion
                    return Ok(KittyGraphicResult::VirtualPlacement {
                        image_id,
                        placement_id,
                        position,
                        cols,
                        rows,
                    });
                }

                // Regular placement
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

                    // Handle relative positioning
                    if let Some(parent_img_id) = self.parent_image_id {
                        graphic.parent_image_id = Some(parent_img_id);
                        graphic.parent_placement_id = self.parent_placement_id;
                        graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                        graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
                    }

                    return Ok(KittyGraphicResult::Graphic(graphic));
                }
                Err(GraphicsError::KittyError("Image not found".to_string()))
            }

            KittyAction::Transmit | KittyAction::TransmitDisplay => {
                let raw_data = self.get_data();
                if raw_data.is_empty() {
                    return Err(GraphicsError::KittyError("No image data".to_string()));
                }

                // Load image data based on transmission medium
                let image_data = match self.medium {
                    KittyMedium::File | KittyMedium::TempFile => {
                        // For file transmission, raw_data is a file path (not base64-encoded)
                        self.load_file_data(&raw_data)?
                    }
                    KittyMedium::Direct => {
                        // For direct transmission, use data as-is
                        raw_data
                    }
                    KittyMedium::SharedMem => {
                        return Err(GraphicsError::KittyError(
                            "Shared memory transmission not supported".to_string(),
                        ));
                    }
                };

                let (width, height, pixels) = self.decode_pixels(&image_data)?;

                // Store for reuse if image_id is specified
                if let Some(image_id) = self.image_id {
                    store.store_kitty_image(image_id, width, height, pixels.clone());
                }

                // Create graphic if TransmitDisplay, or virtual placement if U=1
                if self.action == KittyAction::TransmitDisplay {
                    if self.is_virtual {
                        let cols = self.columns.unwrap_or(1) as usize;
                        let rows = self.rows.unwrap_or(1) as usize;
                        let image_id = self.image_id.unwrap_or(0);
                        let placement_id = self.placement_id.unwrap_or(0);

                        // Create virtual placement
                        let mut graphic = TerminalGraphic::new(
                            next_graphic_id(),
                            GraphicProtocol::Kitty,
                            position,
                            cols,
                            rows,
                            vec![], // Virtual placements don't need pixel data
                        );
                        graphic.kitty_image_id = Some(image_id);
                        graphic.kitty_placement_id = Some(placement_id);
                        graphic.is_virtual = true;
                        store.add_virtual_placement(graphic);

                        // Return virtual placement info for placeholder insertion
                        Ok(KittyGraphicResult::VirtualPlacement {
                            image_id,
                            placement_id,
                            position,
                            cols,
                            rows,
                        })
                    } else {
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

                        // Handle relative positioning
                        if let Some(parent_img_id) = self.parent_image_id {
                            graphic.parent_image_id = Some(parent_img_id);
                            graphic.parent_placement_id = self.parent_placement_id;
                            graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                            graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
                        }

                        Ok(KittyGraphicResult::Graphic(graphic))
                    }
                } else {
                    // Transmit only, no display
                    Ok(KittyGraphicResult::None)
                }
            }

            KittyAction::Frame => {
                // Add animation frame
                let raw_data = self.get_data();
                if raw_data.is_empty() {
                    return Err(GraphicsError::KittyError("No frame data".to_string()));
                }

                let image_id = self.image_id.ok_or_else(|| {
                    GraphicsError::KittyError("Frame requires image ID".to_string())
                })?;

                // Decode frame data
                let image_data = match self.medium {
                    KittyMedium::File | KittyMedium::TempFile => self.load_file_data(&raw_data)?,
                    KittyMedium::Direct => raw_data,
                    KittyMedium::SharedMem => {
                        return Err(GraphicsError::KittyError(
                            "Shared memory not supported for frames".to_string(),
                        ));
                    }
                };

                let (width, height, pixels) = self.decode_pixels(&image_data)?;

                // Create frame
                let frame_num = self.frame_number.unwrap_or(1);
                let mut frame = AnimationFrame::new(frame_num, pixels.clone(), width, height);

                if let Some(delay) = self.frame_delay_ms {
                    frame = frame.with_delay(delay);
                }

                if let Some(x) = self.x_offset {
                    if let Some(y) = self.y_offset {
                        frame = frame.with_offset(x, y);
                    }
                }

                if let Some(comp) = self.frame_composition {
                    frame = frame.with_composition(comp);
                }

                // Add frame to animation
                store.add_animation_frame(image_id, frame);

                // Frame 1 creates both animation entry AND a placement for display
                if frame_num == 1 {
                    // Store as shared image so it can be referenced by Put commands
                    store.store_kitty_image(image_id, width, height, pixels.clone());

                    // Create placement to display the animation
                    let mut graphic = TerminalGraphic::new(
                        next_graphic_id(),
                        GraphicProtocol::Kitty,
                        position,
                        width,
                        height,
                        pixels,
                    );
                    graphic.kitty_image_id = Some(image_id);
                    graphic.kitty_placement_id = self.placement_id;

                    // Handle relative positioning
                    if let Some(parent_img_id) = self.parent_image_id {
                        graphic.parent_image_id = Some(parent_img_id);
                        graphic.parent_placement_id = self.parent_placement_id;
                        graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                        graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
                    }

                    return Ok(KittyGraphicResult::Graphic(graphic));
                }

                // Subsequent frames only add to animation, don't create new placements
                Ok(KittyGraphicResult::None)
            }

            KittyAction::AnimationControl => {
                // Control animation playback
                let image_id = self.image_id.ok_or_else(|| {
                    GraphicsError::KittyError("Animation control requires image ID".to_string())
                })?;

                // Handle num_plays (v= parameter) for setting loop count
                // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
                // We store loop_count as (N-1) so animation stops after (N-1) additional loops
                if let Some(num_plays) = self.num_plays {
                    if num_plays > 0 {
                        let loop_count = if num_plays == 1 {
                            0 // v=1 means infinite looping
                        } else {
                            num_plays - 1 // Store N-1 to get N total plays
                        };
                        debug_info!(
                            "KITTY",
                            "Setting loop count for image_id={}: num_plays={}, loop_count={}",
                            image_id,
                            num_plays,
                            loop_count
                        );
                        store.set_animation_loops(image_id, loop_count);
                    }
                }

                // Handle state control (s= parameter)
                if let Some(control) = self.animation_control {
                    debug_info!(
                        "KITTY",
                        "Applying animation control: image_id={}, control={:?}",
                        image_id,
                        control
                    );
                    store.control_animation(image_id, control);
                } else {
                    debug_log!(
                        "KITTY",
                        "Animation control command received but no control parsed (image_id={})",
                        image_id
                    );
                }

                Ok(KittyGraphicResult::None)
            }
        }
    }

    /// Load image data from file path with security validation
    fn load_file_data(&self, path_data: &[u8]) -> Result<Vec<u8>, GraphicsError> {
        // Decode path from UTF-8 bytes (NOT base64-encoded for file transmission)
        let path_str = String::from_utf8(path_data.to_vec())
            .map_err(|e| GraphicsError::KittyError(format!("Invalid UTF-8 in file path: {}", e)))?;

        let path = Path::new(&path_str);

        // Security validations

        // 1. Check for directory traversal attacks
        if path_str.contains("..") {
            return Err(GraphicsError::KittyError(
                "Directory traversal not allowed".to_string(),
            ));
        }

        // 2. Validate file exists and is readable
        if !path.exists() {
            return Err(GraphicsError::KittyError(format!(
                "File not found: {}",
                path_str
            )));
        }

        if !path.is_file() {
            return Err(GraphicsError::KittyError(format!(
                "Path is not a file: {}",
                path_str
            )));
        }

        // 3. Check file size (limit to 100MB for safety)
        const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB
        let metadata = fs::metadata(path)
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file metadata: {}", e)))?;

        if metadata.len() > MAX_FILE_SIZE {
            return Err(GraphicsError::KittyError(format!(
                "File too large: {} bytes (max {})",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        // 4. Read file
        let file_data = fs::read(path)
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file: {}", e)))?;

        // Delete temp file if requested
        if self.medium == KittyMedium::TempFile {
            let _ = fs::remove_file(path); // Ignore errors on cleanup
        }

        Ok(file_data)
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

    #[test]
    fn test_kitty_medium_from_char() {
        assert_eq!(KittyMedium::from_char('d'), Some(KittyMedium::Direct));
        assert_eq!(KittyMedium::from_char('f'), Some(KittyMedium::File));
        assert_eq!(KittyMedium::from_char('t'), Some(KittyMedium::TempFile));
        assert_eq!(KittyMedium::from_char('s'), Some(KittyMedium::SharedMem));
        assert_eq!(KittyMedium::from_char('x'), None);
    }

    #[test]
    fn test_kitty_file_transmission() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a valid 1x1 red PNG using the image crate
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut png_data = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_data),
            image::ImageFormat::Png,
        )
        .expect("Failed to encode PNG");

        // Write to temp file
        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        temp_file
            .write_all(&png_data)
            .expect("Failed to write PNG data");
        let file_path = temp_file.path().to_str().unwrap();

        // Create parser and parse file transmission command
        // Note: file path must be base64-encoded in the protocol (without padding to match Kitty)
        let file_path_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD_NO_PAD, file_path);
        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=100,t=f;{}", file_path_b64);
        let result = parser.parse_chunk(&payload);

        assert!(result.is_ok());
        assert_eq!(parser.action, KittyAction::TransmitDisplay);
        assert_eq!(parser.format, KittyFormat::Png);
        assert_eq!(parser.medium, KittyMedium::File);

        // Test file loading
        let data = parser.get_data();
        assert!(!data.is_empty());
        assert_eq!(data, file_path.as_bytes());

        // Load file data
        let file_data = parser.load_file_data(&data);
        assert!(file_data.is_ok());
        let file_data = file_data.unwrap();
        assert_eq!(file_data.len(), png_data.len());

        // Decode pixels
        let decode_result = parser.decode_pixels(&file_data);
        assert!(
            decode_result.is_ok(),
            "Failed to decode: {:?}",
            decode_result.err()
        );
        let (width, height, pixels) = decode_result.unwrap();
        assert_eq!(width, 1);
        assert_eq!(height, 1);
        assert_eq!(pixels.len(), 4); // RGBA
                                     // Verify it's red
        assert_eq!(pixels[0], 255); // R
        assert_eq!(pixels[1], 0); // G
        assert_eq!(pixels[2], 0); // B
        assert_eq!(pixels[3], 255); // A
    }

    #[test]
    fn test_kitty_file_security_directory_traversal() {
        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File;

        // Test directory traversal attempt
        let malicious_path = b"../../../etc/passwd";
        let result = parser.load_file_data(malicious_path);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Directory traversal"));
    }

    #[test]
    fn test_kitty_file_security_nonexistent() {
        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File;

        // Test nonexistent file
        let nonexistent_path = b"/this/file/does/not/exist.png";
        let result = parser.load_file_data(nonexistent_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("File not found"));
    }
}
