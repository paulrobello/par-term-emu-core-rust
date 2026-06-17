//! Kitty graphics protocol support
//!
//! Parses Kitty APC graphics sequences:
//! `APC G <key>=<value>,<key>=<value>;<base64-data> ST`
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;

use crate::graphics::{
    next_graphic_id, AnimationControl, AnimationFrame, CompositionMode, GraphicProtocol,
    GraphicsError, GraphicsStore, ImageDimension, ImagePlacement, TerminalGraphic,
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

/// Kitty compression format (o= parameter)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyCompression {
    #[default]
    None, // No compression (default)
    Zlib, // zlib/deflate compression (o=z)
}

impl KittyCompression {
    /// Parse compression character
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'z' => Some(KittyCompression::Zlib),
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
    /// Compression format (o= parameter)
    pub compression: KittyCompression,
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
    /// Z-index for layering (z= for placement commands)
    pub z_index: Option<i32>,
    /// Quietness level (q= parameter)
    /// 0 = default (reply with OK and errors)
    /// 1 = suppress OK reply only
    /// 2 = suppress all replies
    pub quietness: u8,
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
                    "V"
                        // Relative Y offset in pixels (note: different from v=height)
                        // Only parse as relative offset if we have parent placement
                        if self.parent_image_id.is_some() => {
                            self.relative_y_offset = value.parse().ok();
                        }
                    "o" => {
                        // Compression format
                        if let Some(c) = value.chars().next() {
                            if let Some(comp) = KittyCompression::from_char(c) {
                                self.compression = comp;
                            }
                        }
                    }
                    "z" => {
                        // z= is overloaded: frame delay for animations, z-index for placements
                        if self.action == KittyAction::Frame {
                            self.frame_delay_ms = value.parse().ok();
                        } else {
                            self.z_index = value.parse().ok();
                        }
                    }
                    "q" => {
                        // Quietness level (0 = reply, 1 = suppress OK, 2 = suppress all)
                        if let Ok(level) = value.parse::<u8>() {
                            self.quietness = level;
                        }
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

    /// Get accumulated data, decompressing if necessary
    pub fn get_data(&self) -> Vec<u8> {
        let raw = self.data_chunks.concat();
        if self.compression == KittyCompression::Zlib {
            match Self::decompress_zlib(&raw) {
                Ok(decompressed) => decompressed,
                Err(_) => raw, // Fall back to raw data on decompression failure
            }
        } else {
            raw
        }
    }

    /// Decompress zlib-compressed data
    fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>, GraphicsError> {
        let mut decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| GraphicsError::KittyError(format!("Zlib decompression failed: {}", e)))?;
        Ok(decompressed)
    }

    /// Check if data was compressed
    pub fn is_compressed(&self) -> bool {
        self.compression != KittyCompression::None
    }

    /// Build an ImagePlacement from the parsed Kitty parameters
    pub fn build_placement(&self) -> ImagePlacement {
        let mut placement = ImagePlacement::inline();

        if let Some(cols) = self.columns {
            placement.columns = Some(cols);
            placement.requested_width = ImageDimension::cells(cols as f64);
        }

        if let Some(rows) = self.rows {
            placement.rows = Some(rows);
            placement.requested_height = ImageDimension::cells(rows as f64);
        }

        if let Some(z) = self.z_index {
            placement.z_index = z;
        }

        if let Some(x) = self.x_offset {
            placement.x_offset = x;
        }

        if let Some(y) = self.y_offset {
            placement.y_offset = y;
        }

        placement
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
                        KittyDeleteTarget::AtCursor => {
                            let (cursor_col, cursor_row) = position;
                            store
                                .placements
                                .retain(|g| g.position != (cursor_col, cursor_row));
                        }
                        KittyDeleteTarget::InCell => {
                            // Same as AtCursor in our context since we use the cursor position
                            let (cursor_col, cursor_row) = position;
                            store
                                .placements
                                .retain(|g| g.position != (cursor_col, cursor_row));
                        }
                        KittyDeleteTarget::OnScreen => {
                            // Remove all visible placements but preserve shared images
                            store.placements.clear();
                        }
                        KittyDeleteTarget::ByColumn(col) => {
                            let target_col = *col as usize;
                            store.placements.retain(|g| {
                                let start_col = g.position.0;
                                let cell_width =
                                    g.cell_dimensions.map(|(w, _)| w as usize).unwrap_or(1);
                                let end_col = start_col + g.width.div_ceil(cell_width);
                                target_col < start_col || target_col >= end_col
                            });
                        }
                        KittyDeleteTarget::ByRow(row) => {
                            let target_row = *row as usize;
                            store.placements.retain(|g| {
                                let start_row = g.position.1;
                                let cell_height =
                                    g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
                                let end_row = start_row + g.height.div_ceil(cell_height);
                                target_row < start_row || target_row >= end_row
                            });
                        }
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
                    graphic.placement = self.build_placement();

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

                let compressed = self.is_compressed();

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
                        graphic.was_compressed = compressed;
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
                        graphic.was_compressed = compressed;
                        graphic.placement = self.build_placement();

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

                let compressed = self.is_compressed();

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
                    graphic.was_compressed = compressed;
                    graphic.placement = self.build_placement();

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

    #[test]
    fn test_kitty_compression_from_char() {
        assert_eq!(
            KittyCompression::from_char('z'),
            Some(KittyCompression::Zlib)
        );
        assert_eq!(KittyCompression::from_char('x'), None);
    }

    #[test]
    fn test_kitty_compression_default() {
        let parser = KittyParser::new();
        assert_eq!(parser.compression, KittyCompression::None);
        assert!(!parser.is_compressed());
    }

    #[test]
    fn test_kitty_parse_compression_param() {
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T,f=32,o=z,s=2,v=2;");
        assert!(result.is_ok());
        assert_eq!(parser.compression, KittyCompression::Zlib);
        assert!(parser.is_compressed());
    }

    #[test]
    fn test_kitty_zlib_decompression() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        // Create a 2x2 RGBA image (16 bytes)
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, // Red
            0, 255, 0, 255, // Green
            0, 0, 255, 255, // Blue
            255, 255, 0, 255, // Yellow
        ];

        // Compress with zlib
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&pixel_data).unwrap();
        let compressed = encoder.finish().unwrap();

        // Base64 encode the compressed data
        let b64_compressed =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

        // Parse with o=z compression flag
        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=32,o=z,s=2,v=2;{}", b64_compressed);
        let result = parser.parse_chunk(&payload);
        assert!(result.is_ok());
        assert_eq!(parser.compression, KittyCompression::Zlib);

        // get_data() should return decompressed data
        let data = parser.get_data();
        assert_eq!(data, pixel_data);
    }

    #[test]
    fn test_kitty_zlib_build_graphic() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        // Create a 2x2 RGBA image (16 bytes)
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, // Red
            0, 255, 0, 255, // Green
            0, 0, 255, 255, // Blue
            255, 255, 0, 255, // Yellow
        ];

        // Compress with zlib
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&pixel_data).unwrap();
        let compressed = encoder.finish().unwrap();

        // Base64 encode
        let b64_compressed =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

        // Parse and build graphic
        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=32,o=z,s=2,v=2,i=42;{}", b64_compressed);
        parser.parse_chunk(&payload).unwrap();

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_ok());

        // Transmit-only, no display - should store the image
        let stored = store.get_kitty_image(42);
        assert!(stored.is_some());
        let (w, h, pixels) = stored.unwrap();
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(*pixels, pixel_data);
    }

    #[test]
    fn test_kitty_zlib_transmit_display_sets_compressed_flag() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        // Create a 2x2 RGBA image (16 bytes)
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];

        // Compress with zlib
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&pixel_data).unwrap();
        let compressed = encoder.finish().unwrap();

        let b64_compressed =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

        // TransmitDisplay with compression
        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=32,o=z,s=2,v=2;{}", b64_compressed);
        parser.parse_chunk(&payload).unwrap();

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((5, 10), &mut store).unwrap();

        match result {
            KittyGraphicResult::Graphic(graphic) => {
                assert!(graphic.was_compressed, "was_compressed should be true");
                assert_eq!(graphic.width, 2);
                assert_eq!(graphic.height, 2);
                assert_eq!(*graphic.pixels, pixel_data);
            }
            _ => panic!("Expected Graphic result"),
        }
    }

    #[test]
    fn test_kitty_no_compression_flag_unset() {
        // Uncompressed RGBA data for a 2x2 image
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];

        let b64_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=32,s=2,v=2;{}", b64_data);
        parser.parse_chunk(&payload).unwrap();

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();

        match result {
            KittyGraphicResult::Graphic(graphic) => {
                assert!(!graphic.was_compressed, "was_compressed should be false");
            }
            _ => panic!("Expected Graphic result"),
        }
    }

    #[test]
    fn test_kitty_zlib_chunked_transfer() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        // Create a 2x2 RGBA image (16 bytes)
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];

        // Compress with zlib
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&pixel_data).unwrap();
        let compressed = encoder.finish().unwrap();

        let b64_compressed =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

        // Split base64 into two chunks at a 4-byte boundary (base64 block size)
        let mid = (b64_compressed.len() / 2) & !3; // Round down to nearest multiple of 4
        let chunk1 = &b64_compressed[..mid];
        let chunk2 = &b64_compressed[mid..];

        // First chunk
        let mut parser = KittyParser::new();
        let payload1 = format!("a=T,f=32,o=z,s=2,v=2,m=1;{}", chunk1);
        let more = parser.parse_chunk(&payload1).unwrap();
        assert!(more);
        assert_eq!(parser.compression, KittyCompression::Zlib);

        // Second chunk
        let payload2 = format!("m=0;{}", chunk2);
        let more = parser.parse_chunk(&payload2).unwrap();
        assert!(!more);

        // Data should be decompressed correctly
        let data = parser.get_data();
        assert_eq!(data, pixel_data);
    }

    #[test]
    fn test_kitty_decompress_zlib_invalid_data() {
        // Test decompression with invalid zlib data falls back gracefully
        let invalid_data = vec![0x00, 0x01, 0x02, 0x03];
        let result = KittyParser::decompress_zlib(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_kitty_build_placement_defaults() {
        let parser = KittyParser::new();
        let placement = parser.build_placement();
        assert_eq!(
            placement.display_mode,
            crate::graphics::ImageDisplayMode::Inline
        );
        assert!(placement.preserve_aspect_ratio);
        assert!(placement.columns.is_none());
        assert!(placement.rows.is_none());
        assert_eq!(placement.z_index, 0);
        assert_eq!(placement.x_offset, 0);
        assert_eq!(placement.y_offset, 0);
    }

    #[test]
    fn test_kitty_build_placement_with_columns_rows() {
        let mut parser = KittyParser::new();
        parser.parse_chunk("a=T,f=100,c=10,r=5;").unwrap();
        let placement = parser.build_placement();
        assert_eq!(placement.columns, Some(10));
        assert_eq!(placement.rows, Some(5));
        assert_eq!(placement.requested_width.value, 10.0);
        assert_eq!(
            placement.requested_width.unit,
            crate::graphics::ImageSizeUnit::Cells
        );
        assert_eq!(placement.requested_height.value, 5.0);
        assert_eq!(
            placement.requested_height.unit,
            crate::graphics::ImageSizeUnit::Cells
        );
    }

    #[test]
    fn test_kitty_build_placement_with_offsets() {
        let mut parser = KittyParser::new();
        parser.parse_chunk("a=T,f=100,x=5,y=3;").unwrap();
        let placement = parser.build_placement();
        assert_eq!(placement.x_offset, 5);
        assert_eq!(placement.y_offset, 3);
    }

    #[test]
    fn test_kitty_z_index_for_placement() {
        let mut parser = KittyParser::new();
        parser.parse_chunk("a=p,i=1,z=-1;").unwrap();
        let placement = parser.build_placement();
        assert_eq!(placement.z_index, -1);
    }

    #[test]
    fn test_kitty_z_as_frame_delay_for_frames() {
        let mut parser = KittyParser::new();
        parser.parse_chunk("a=f,i=1,z=100;").unwrap();
        // For frames, z is frame_delay, not z_index
        assert_eq!(parser.frame_delay_ms, Some(100));
        assert!(parser.z_index.is_none());
    }

    #[test]
    fn test_kitty_transmit_display_has_placement() {
        let pixel_data: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let b64_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

        let mut parser = KittyParser::new();
        let payload = format!("a=T,f=32,s=2,v=2,c=10,r=5,x=2,y=3;{}", b64_data);
        parser.parse_chunk(&payload).unwrap();

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();

        match result {
            KittyGraphicResult::Graphic(graphic) => {
                assert_eq!(graphic.placement.columns, Some(10));
                assert_eq!(graphic.placement.rows, Some(5));
                assert_eq!(graphic.placement.x_offset, 2);
                assert_eq!(graphic.placement.y_offset, 3);
                assert_eq!(
                    graphic.placement.display_mode,
                    crate::graphics::ImageDisplayMode::Inline
                );
            }
            _ => panic!("Expected Graphic result"),
        }
    }

    #[test]
    fn test_kitty_put_placement_with_z_index() {
        // First store an image
        let pixel_data: Vec<u8> = vec![255, 0, 0, 255];
        let b64_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

        let mut parser = KittyParser::new();
        let payload = format!("a=t,f=32,s=1,v=1,i=42;{}", b64_data);
        parser.parse_chunk(&payload).unwrap();
        let mut store = GraphicsStore::new();
        parser.build_graphic((0, 0), &mut store).unwrap();

        // Now put with z-index
        let mut parser2 = KittyParser::new();
        parser2.parse_chunk("a=p,i=42,z=5;").unwrap();
        let result = parser2.build_graphic((0, 0), &mut store).unwrap();

        match result {
            KittyGraphicResult::Graphic(graphic) => {
                assert_eq!(graphic.placement.z_index, 5);
            }
            _ => panic!("Expected Graphic result"),
        }
    }

    // =========================================================================
    // Additional coverage: malformed/edge-case parser input, decompression
    // fallback, placement math, delete/query actions, file security, raw
    // RGB/RGBA decoding, frame/animation-control error paths.
    // =========================================================================

    // --- parse_chunk edge cases ---

    #[test]
    fn test_parse_chunk_empty_payload() {
        // Empty string: no pairs, no data — should succeed, no more_chunks.
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("");
        assert!(result.is_ok());
        assert!(!result.unwrap());
        // Default action preserved
        assert_eq!(parser.action, KittyAction::Transmit);
    }

    #[test]
    fn test_parse_chunk_no_semicolon_uses_entire_payload_as_params() {
        // Without ';' data_str is "" so nothing is decoded.
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=q,i=7");
        assert!(result.is_ok());
        assert_eq!(parser.action, KittyAction::Query);
        assert_eq!(parser.image_id, Some(7));
    }

    #[test]
    fn test_parse_chunk_pair_without_equals_is_ignored() {
        // A pair with no '=' should be skipped silently (no panic).
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("garbage,a=q");
        assert!(result.is_ok());
        assert_eq!(parser.action, KittyAction::Query);
    }

    #[test]
    fn test_parse_chunk_empty_value_leaves_action_unchanged() {
        // Empty value -> value.chars().next() is None -> the action match arm's
        // `if let Some(c)` body never runs, so action is UNCHANGED (not reset).
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Query; // pre-set something non-default
        let _ = parser.parse_chunk("a=");
        assert_eq!(parser.action, KittyAction::Query); // unchanged
    }

    #[test]
    fn test_parse_chunk_invalid_format_code_falls_back_to_default() {
        // Format code 99 is invalid -> falls back to default (Rgba).
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,f=99;");
        assert_eq!(parser.format, KittyFormat::Rgba);
    }

    #[test]
    fn test_parse_chunk_non_numeric_format_is_ignored() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Png;
        let _ = parser.parse_chunk("a=T,f=abc;");
        assert_eq!(parser.format, KittyFormat::Png); // unchanged
    }

    #[test]
    fn test_parse_chunk_invalid_medium_char_falls_back_to_default() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,t=x;"); // 'x' is not a valid medium
        assert_eq!(parser.medium, KittyMedium::Direct); // default
    }

    #[test]
    fn test_parse_chunk_invalid_action_char_falls_back_to_default() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=Z;"); // 'Z' is not a valid action
        assert_eq!(parser.action, KittyAction::Transmit); // default
    }

    #[test]
    fn test_parse_chunk_non_numeric_id_is_ignored() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,i=notanumber,p=alsonot;");
        assert_eq!(parser.image_id, None);
        assert_eq!(parser.placement_id, None);
    }

    #[test]
    fn test_parse_chunk_unknown_key_is_ignored() {
        // Unknown key should hit the `_ => {}` arm cleanly.
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("zzz=123,a=q");
        assert!(result.is_ok());
        assert_eq!(parser.action, KittyAction::Query);
    }

    #[test]
    fn test_parse_chunk_more_chunks_explicit_zero() {
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T,m=0;");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_parse_chunk_more_chunks_non_one_is_false() {
        // m= only true when value == "1"; "2", "true", etc. are false.
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,m=2;");
        assert!(!parser.more_chunks);
    }

    #[test]
    fn test_parse_chunk_width_and_height_for_transmit() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,s=10,v=20;");
        assert_eq!(parser.width, Some(10));
        assert_eq!(parser.height, Some(20));
    }

    #[test]
    fn test_parse_chunk_columns_rows_for_non_frame() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,c=8,r=4;");
        assert_eq!(parser.columns, Some(8));
        assert_eq!(parser.rows, Some(4));
    }

    #[test]
    fn test_parse_chunk_frame_overloads_c_and_r() {
        // For Frame: c= is composition, r= is frame number
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=f,c=1,r=5;");
        assert_eq!(parser.frame_composition, Some(CompositionMode::Overwrite));
        assert_eq!(parser.frame_number, Some(5));
        // columns/rows should NOT be set for frame action
        assert_eq!(parser.columns, None);
        assert_eq!(parser.rows, None);
    }

    #[test]
    fn test_parse_chunk_frame_composition_invalid_char() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=f,c=Z;");
        assert_eq!(parser.frame_composition, None);
    }

    #[test]
    fn test_parse_chunk_animation_control_overloads_s_and_v() {
        // For AnimationControl: s= is control, v= is num_plays
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=a,s=3,v=5,i=1;");
        assert_eq!(parser.action, KittyAction::AnimationControl);
        assert_eq!(
            parser.animation_control,
            Some(AnimationControl::EnableLooping)
        );
        assert_eq!(parser.num_plays, Some(5));
        // width/height should NOT be set for animation-control action
        assert_eq!(parser.width, None);
        assert_eq!(parser.height, None);
    }

    #[test]
    fn test_parse_chunk_animation_control_invalid_s_value() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=a,s=99;");
        assert_eq!(parser.animation_control, None);
    }

    #[test]
    fn test_parse_chunk_quietness_levels() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,q=2;");
        assert_eq!(parser.quietness, 2);

        let mut parser2 = KittyParser::new();
        let _ = parser2.parse_chunk("a=T,q=notanumber;");
        assert_eq!(parser2.quietness, 0); // default, parse failed
    }

    #[test]
    fn test_parse_chunk_offsets_and_parents() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=p,i=1,P=2,Q=3,H=10;");
        assert_eq!(parser.x_offset, None); // x= not parsed here
        assert_eq!(parser.y_offset, None); // y= not parsed here
        assert_eq!(parser.parent_image_id, Some(2));
        assert_eq!(parser.parent_placement_id, Some(3));
        assert_eq!(parser.relative_x_offset, Some(10));
    }

    #[test]
    fn test_parse_chunk_virtual_placement_flag() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=p,i=1,U=1;");
        assert!(parser.is_virtual);

        let mut parser2 = KittyParser::new();
        let _ = parser2.parse_chunk("a=p,i=1,U=0;");
        assert!(!parser2.is_virtual);
    }

    #[test]
    fn test_parse_chunk_x_y_offsets_parsed() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,x=15,y=25;");
        assert_eq!(parser.x_offset, Some(15));
        assert_eq!(parser.y_offset, Some(25));
    }

    #[test]
    fn test_parse_chunk_compression_invalid_char_ignored() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,o=Q;"); // 'Q' not a valid compression
        assert_eq!(parser.compression, KittyCompression::None);
    }

    #[test]
    fn test_parse_chunk_base64_invalid_returns_error() {
        // Characters outside the base64 alphabet must produce Base64Error.
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T;!!!!notbase64!!!!");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("base64") || err.contains("Invalid base64"));
    }

    #[test]
    fn test_parse_chunk_base64_unpadded_decodes() {
        // STANDARD_NO_PAD fallback path: "QUFB" is "AAA" with no padding.
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T;QUFB");
        assert!(result.is_ok());
        let data = parser.get_data();
        assert_eq!(data, b"AAA");
    }

    #[test]
    fn test_parse_chunk_base64_padded_decodes() {
        // Standard padded path: 5 bytes (QkFBQPI= is "AAAABBBB" tail) -- use a
        // length that requires padding. 1 byte -> "QQ==" decodes to "A".
        let mut parser = KittyParser::new();
        let result = parser.parse_chunk("a=T;QQ==");
        assert!(result.is_ok());
        assert_eq!(parser.get_data(), b"A");
    }

    // --- reset() ---

    #[test]
    fn test_reset_clears_all_state() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,f=100,i=42,m=1;o=z,s=2,v=2;AAAA");
        assert_eq!(parser.image_id, Some(42));
        assert!(parser.more_chunks);

        parser.reset();
        assert_eq!(parser.action, KittyAction::Transmit);
        assert_eq!(parser.image_id, None);
        assert_eq!(parser.format, KittyFormat::Rgba);
        assert!(!parser.more_chunks);
        assert_eq!(parser.compression, KittyCompression::None);
        // After reset, accumulated data chunks are gone.
        assert!(parser.get_data().is_empty());
    }

    // --- get_data decompression fallback ---

    #[test]
    fn test_get_data_zlib_failure_falls_back_to_raw() {
        // If o=z is set but the data is not actually zlib, get_data() must
        // return the raw bytes (not propagate an error / panic).
        let mut parser = KittyParser::new();
        // Mark compressed with valid zlib marker but corrupt the body.
        let bad = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"not zlib");
        let _ = parser.parse_chunk(&format!("a=T,o=z;{}", bad));
        assert!(parser.is_compressed());
        // decompress fails -> should return raw concatenated data
        let data = parser.get_data();
        assert_eq!(data, b"not zlib");
    }

    #[test]
    fn test_get_data_uncompressed_returns_raw_concat() {
        let mut parser = KittyParser::new();
        let _ = parser.parse_chunk("a=T,m=1;QUFB"); // "AAA"
        let _ = parser.parse_chunk("m=0;QkJD"); // "BBC"
        assert_eq!(parser.get_data(), b"AAABBC");
    }

    // --- parse_delete_target variants ---

    #[test]
    fn test_parse_delete_target_uppercase_letters() {
        // A, C, Z uppercase are valid per parse_delete_target match arms.
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;

        let _ = parser.parse_chunk("a=d,d=A;");
        assert_eq!(parser.delete_target, Some(KittyDeleteTarget::All));

        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=C;");
        assert_eq!(parser.delete_target, Some(KittyDeleteTarget::AtCursor));

        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=Z;");
        assert_eq!(parser.delete_target, Some(KittyDeleteTarget::OnScreen));
    }

    #[test]
    fn test_parse_delete_target_lowercase_c_and_z() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=c;");
        assert_eq!(parser.delete_target, Some(KittyDeleteTarget::AtCursor));

        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=z;");
        assert_eq!(parser.delete_target, Some(KittyDeleteTarget::OnScreen));
    }

    #[test]
    fn test_parse_delete_target_unknown_char_is_none() {
        // 'x', 'y', 'i', 'p' are NOT handled by parse_delete_target,
        // so the target stays None even though build_graphic has branches
        // for ByColumn/ByRow/ById/ByPlacement/InCell that are never reached.
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=x;");
        assert_eq!(parser.delete_target, None);

        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=p;");
        assert_eq!(parser.delete_target, None);
    }

    #[test]
    fn test_parse_delete_target_empty_value_is_none() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let _ = parser.parse_chunk("a=d,d=;");
        assert_eq!(parser.delete_target, None);
    }

    // --- build_graphic: Delete action coverage ---

    #[test]
    fn test_build_graphic_delete_no_target_returns_none() {
        // Delete with no parsed target should be a no-op -> None.
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Delete;
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
    }

    /// Helper: transmit a 1x1 RGBA image with the given id and placement_id at
    /// the given position, and add the resulting placement to `store.placements`.
    ///
    /// `build_graphic()` calls `store.store_kitty_image()` internally but does
    /// NOT push the returned graphic into `store.placements` — that is the
    /// terminal-integration layer's job (see GraphicsStore::add_graphic).
    fn transmit_and_add(
        store: &mut GraphicsStore,
        image_id: u32,
        placement_id: Option<u32>,
        position: (usize, usize),
    ) {
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut tx = KittyParser::new();
        let payload = match placement_id {
            Some(pid) => format!("a=T,f=32,s=1,v=1,i={},p={};{}", image_id, pid, b64),
            None => format!("a=T,f=32,s=1,v=1,i={};{}", image_id, b64),
        };
        tx.parse_chunk(&payload).unwrap();
        match tx.build_graphic(position, store).unwrap() {
            KittyGraphicResult::Graphic(g) => store.add_graphic(g),
            other => panic!("transmit_and_add expected Graphic, got {:?}", other),
        }
    }

    #[test]
    fn test_build_graphic_delete_all_clears_store() {
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, None, (0, 0));
        assert_eq!(store.placements.len(), 1);

        let mut del = KittyParser::new();
        del.parse_chunk("a=d,d=a;").unwrap();
        let result = del.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
        assert!(store.placements.is_empty());
    }

    #[test]
    fn test_build_graphic_delete_at_cursor() {
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, None, (3, 4));
        assert_eq!(store.placements.len(), 1);

        // Delete at cursor (3,4) — should remove it.
        let mut del = KittyParser::new();
        del.parse_chunk("a=d,d=c;").unwrap();
        let _ = del.build_graphic((3, 4), &mut store).unwrap();
        assert!(store.placements.is_empty());

        // A different cursor position should leave other placements alone.
        transmit_and_add(&mut store, 2, None, (5, 6));
        assert_eq!(store.placements.len(), 1);

        let mut del2 = KittyParser::new();
        del2.parse_chunk("a=d,d=c;").unwrap();
        let _ = del2.build_graphic((0, 0), &mut store).unwrap(); // cursor elsewhere
        assert_eq!(store.placements.len(), 1); // still there
    }

    #[test]
    fn test_build_graphic_delete_in_cell_uses_cursor() {
        // InCell behaves the same as AtCursor in this implementation.
        // parse_delete_target never produces InCell, so we set it directly
        // to cover the InCell branch.
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, None, (7, 8));
        assert_eq!(store.placements.len(), 1);

        let mut del = KittyParser::new();
        del.action = KittyAction::Delete;
        del.delete_target = Some(KittyDeleteTarget::InCell);
        let _ = del.build_graphic((7, 8), &mut store).unwrap();
        assert!(store.placements.is_empty());
    }

    #[test]
    fn test_build_graphic_delete_on_screen() {
        let mut store = GraphicsStore::new();
        for i in 1..=3 {
            transmit_and_add(&mut store, i, None, (i as usize, 0));
        }
        assert_eq!(store.placements.len(), 3);

        let mut del = KittyParser::new();
        del.parse_chunk("a=d,d=z;").unwrap();
        let _ = del.build_graphic((0, 0), &mut store).unwrap();
        assert!(store.placements.is_empty());
    }

    #[test]
    fn test_build_graphic_delete_by_id() {
        // parse_delete_target cannot produce ById, so we set it directly
        // to cover the ById branch.
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, None, (0, 0));
        transmit_and_add(&mut store, 2, None, (1, 0));
        assert_eq!(store.placements.len(), 2);

        // Delete only image id=1
        let mut del = KittyParser::new();
        del.action = KittyAction::Delete;
        del.delete_target = Some(KittyDeleteTarget::ById(1));
        let _ = del.build_graphic((0, 0), &mut store).unwrap();
        // One placement should remain (image id=2).
        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.placements[0].kitty_image_id, Some(2));
    }

    #[test]
    fn test_build_graphic_delete_by_placement() {
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, Some(100), (0, 0));
        transmit_and_add(&mut store, 1, Some(200), (1, 0));
        assert_eq!(store.placements.len(), 2);

        // Delete only (image_id=1, placement_id=100)
        let mut del = KittyParser::new();
        del.action = KittyAction::Delete;
        del.delete_target = Some(KittyDeleteTarget::ByPlacement(1, Some(100)));
        let _ = del.build_graphic((0, 0), &mut store).unwrap();
        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.placements[0].kitty_placement_id, Some(200));
    }

    #[test]
    fn test_build_graphic_delete_by_column_and_row() {
        // These branches need cell_dimensions set; the retain math uses
        // div_ceil over cell width/height. We construct targets directly
        // (parse_delete_target cannot produce ByColumn/ByRow).
        let mut store = GraphicsStore::new();
        transmit_and_add(&mut store, 1, None, (0, 0));
        transmit_and_add(&mut store, 2, None, (5, 5));
        for g in store.placements.iter_mut() {
            g.set_cell_dimensions(1, 1); // 1x1 pixel cell, simplest case
        }
        assert_eq!(store.placements.len(), 2);

        // ByColumn(0): placement at col=0 spans col 0 -> removed.
        // Placement at col=5 stays.
        let mut del_col = KittyParser::new();
        del_col.action = KittyAction::Delete;
        del_col.delete_target = Some(KittyDeleteTarget::ByColumn(0));
        let _ = del_col.build_graphic((0, 0), &mut store).unwrap();
        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.placements[0].position.0, 5);

        // ByRow(5): remaining placement is at row=5 -> removed.
        let mut del_row = KittyParser::new();
        del_row.action = KittyAction::Delete;
        del_row.delete_target = Some(KittyDeleteTarget::ByRow(5));
        let _ = del_row.build_graphic((0, 0), &mut store).unwrap();
        assert!(store.placements.is_empty());
    }

    // --- build_graphic: Query ---

    #[test]
    fn test_build_graphic_query_returns_none() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Query;
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
    }

    // --- build_graphic: Put ---

    #[test]
    fn test_build_graphic_put_missing_image_returns_error() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Put;
        parser.image_id = Some(999); // never transmitted
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Image not found"));
    }

    #[test]
    fn test_build_graphic_put_virtual_placement() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Put;
        parser.image_id = Some(7);
        parser.placement_id = Some(11);
        parser.is_virtual = true;
        parser.columns = Some(3);
        parser.rows = Some(2);

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((4, 5), &mut store).unwrap();
        match result {
            KittyGraphicResult::VirtualPlacement {
                image_id,
                placement_id,
                position,
                cols,
                rows,
            } => {
                assert_eq!(image_id, 7);
                assert_eq!(placement_id, 11);
                assert_eq!(position, (4, 5));
                assert_eq!(cols, 3);
                assert_eq!(rows, 2);
            }
            other => panic!("Expected VirtualPlacement, got {:?}", other),
        }
        // Should also register a virtual placement in the store.
        assert!(store.get_virtual_placement(7, 11).is_some());
    }

    #[test]
    fn test_build_graphic_put_regular_uses_stored_pixels() {
        // Transmit then Put.
        let pixels: Vec<u8> = vec![10, 20, 30, 40];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();

        let mut tx = KittyParser::new();
        tx.parse_chunk(&format!("a=t,f=32,s=1,v=1,i=5;{}", b64))
            .unwrap();
        let _ = tx.build_graphic((0, 0), &mut store).unwrap();

        let mut put = KittyParser::new();
        put.parse_chunk("a=p,i=5,z=7,x=1,y=2;").unwrap();
        let result = put.build_graphic((9, 9), &mut store).unwrap();
        match result {
            KittyGraphicResult::Graphic(g) => {
                assert_eq!(g.kitty_image_id, Some(5));
                assert_eq!(g.position, (9, 9));
                assert_eq!(g.placement.z_index, 7);
                assert_eq!(g.placement.x_offset, 1);
                assert_eq!(g.placement.y_offset, 2);
            }
            other => panic!("Expected Graphic, got {:?}", other),
        }
    }

    #[test]
    fn test_build_graphic_put_with_relative_positioning() {
        let pixels: Vec<u8> = vec![1, 2, 3, 4];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();

        let mut tx = KittyParser::new();
        tx.parse_chunk(&format!("a=t,f=32,s=1,v=1,i=8;{}", b64))
            .unwrap();
        let _ = tx.build_graphic((0, 0), &mut store).unwrap();

        let mut put = KittyParser::new();
        // P= parent_image_id enables the relative-positioning branch.
        put.parse_chunk("a=p,i=8,P=99,Q=88,H=5;").unwrap();
        let result = put.build_graphic((1, 1), &mut store).unwrap();
        match result {
            KittyGraphicResult::Graphic(g) => {
                assert_eq!(g.parent_image_id, Some(99));
                assert_eq!(g.parent_placement_id, Some(88));
                assert_eq!(g.relative_x_offset, 5);
                assert_eq!(g.relative_y_offset, 0); // V= not parseable without parent (chicken/egg), defaults to 0
            }
            other => panic!("Expected Graphic, got {:?}", other),
        }
    }

    // --- build_graphic: Transmit / TransmitDisplay errors ---

    #[test]
    fn test_build_graphic_transmit_no_data_returns_error() {
        let mut parser = KittyParser::new();
        // TransmitDisplay with no data section.
        parser.action = KittyAction::TransmitDisplay;
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No image data"));
    }

    #[test]
    fn test_build_graphic_transmit_shared_memory_unsupported() {
        let mut parser = KittyParser::new();
        // Provide some data so we get past the empty check, then fail at SharedMem.
        parser.action = KittyAction::TransmitDisplay;
        parser.medium = KittyMedium::SharedMem;
        // Manually push a data chunk by parsing one (compression stays None).
        let _ = parser.parse_chunk("a=T,t=s;QUFB"); // sets medium=SharedMem
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Shared memory transmission not supported"));
    }

    #[test]
    fn test_build_graphic_transmit_only_stores_but_no_placement() {
        // Action 't' (Transmit, not TransmitDisplay) with image_id stores
        // the image but returns KittyGraphicResult::None.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut parser = KittyParser::new();
        parser
            .parse_chunk(&format!("a=t,f=32,s=1,v=1,i=33;{}", b64))
            .unwrap();
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
        assert!(store.get_kitty_image(33).is_some());
        // No placement should have been created.
        assert!(store.placements.is_empty());
    }

    #[test]
    fn test_build_graphic_transmit_display_virtual() {
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut parser = KittyParser::new();
        // U=1 makes TransmitDisplay produce a virtual placement.
        parser
            .parse_chunk(&format!("a=T,f=32,s=1,v=1,i=44,U=1,c=2,r=3;{}", b64))
            .unwrap();
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((1, 2), &mut store).unwrap();
        match result {
            KittyGraphicResult::VirtualPlacement {
                image_id,
                cols,
                rows,
                position,
                ..
            } => {
                assert_eq!(image_id, 44);
                assert_eq!(cols, 2);
                assert_eq!(rows, 3);
                assert_eq!(position, (1, 2));
            }
            other => panic!("Expected VirtualPlacement, got {:?}", other),
        }
    }

    #[test]
    fn test_build_graphic_transmit_display_with_relative_positioning() {
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut parser = KittyParser::new();
        parser
            .parse_chunk(&format!("a=T,f=32,s=1,v=1,i=55,P=1,Q=2,H=3;{}", b64))
            .unwrap();
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store).unwrap();
        match result {
            KittyGraphicResult::Graphic(g) => {
                assert_eq!(g.parent_image_id, Some(1));
                assert_eq!(g.parent_placement_id, Some(2));
                assert_eq!(g.relative_x_offset, 3);
                assert_eq!(g.relative_y_offset, 0);
            }
            other => panic!("Expected Graphic, got {:?}", other),
        }
    }

    // --- decode_pixels error paths ---

    #[test]
    fn test_decode_pixels_rgba_missing_width() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgba;
        parser.height = Some(2); // no width
        let result = parser.decode_pixels(&[0u8; 8]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Width required"));
    }

    #[test]
    fn test_decode_pixels_rgba_missing_height() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgba;
        parser.width = Some(2); // no height
        let result = parser.decode_pixels(&[0u8; 8]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Height required"));
    }

    #[test]
    fn test_decode_pixels_rgba_size_mismatch() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgba;
        parser.width = Some(2);
        parser.height = Some(2);
        // Expected 2*2*4 = 16 bytes, but we provide 8.
        let result = parser.decode_pixels(&[0u8; 8]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Data size mismatch"));
    }

    #[test]
    fn test_decode_pixels_rgba_exact_match() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgba;
        parser.width = Some(1);
        parser.height = Some(1);
        let data = vec![1, 2, 3, 4];
        let (w, h, px) = parser.decode_pixels(&data).unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        assert_eq!(px, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_decode_pixels_rgb_missing_width() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgb;
        parser.height = Some(2);
        let result = parser.decode_pixels(&[0u8; 6]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Width required"));
    }

    #[test]
    fn test_decode_pixels_rgb_missing_height() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgb;
        parser.width = Some(2);
        let result = parser.decode_pixels(&[0u8; 6]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Height required"));
    }

    #[test]
    fn test_decode_pixels_rgb_size_mismatch() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgb;
        parser.width = Some(2);
        parser.height = Some(2);
        // Expected 2*2*3 = 12 bytes, provide 6.
        let result = parser.decode_pixels(&[0u8; 6]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Data size mismatch"));
    }

    #[test]
    fn test_decode_pixels_rgb_converts_to_rgba() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Rgb;
        parser.width = Some(1);
        parser.height = Some(2);
        // 2 pixels RGB = 6 bytes
        let data = vec![10, 20, 30, 40, 50, 60];
        let (w, h, px) = parser.decode_pixels(&data).unwrap();
        assert_eq!(w, 1);
        assert_eq!(h, 2);
        // Each RGB triple becomes RGBA with alpha=255.
        assert_eq!(px, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn test_decode_pixels_png_invalid() {
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Png;
        let result = parser.decode_pixels(b"not a png");
        assert!(result.is_err());
        // ImageError carries "Image decode failed" in its Display.
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Image decode failed") || msg.contains("decode"));
    }

    #[test]
    fn test_decode_pixels_png_valid() {
        // Encode a real PNG and round-trip through decode_pixels.
        let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 4]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut parser = KittyParser::new();
        parser.format = KittyFormat::Png;
        let (w, h, px) = parser.decode_pixels(&png).unwrap();
        assert_eq!(w, 2);
        assert_eq!(h, 1);
        assert_eq!(px, vec![1, 2, 3, 4, 1, 2, 3, 4]);
    }

    // --- build_graphic: Frame action ---

    #[test]
    fn test_build_graphic_frame_missing_image_id_returns_error() {
        let mut parser = KittyParser::new();
        // Provide data so we get past the empty check.
        let _ = parser.parse_chunk("a=f,f=32,s=1,v=1;QUFB"); // no i=
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Frame requires image ID"));
    }

    #[test]
    fn test_build_graphic_frame_no_data_returns_error() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Frame;
        parser.image_id = Some(1);
        // No data parsed -> get_data() is empty.
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No frame data"));
    }

    #[test]
    fn test_build_graphic_frame_shared_memory_unsupported() {
        let mut parser = KittyParser::new();
        // f=action, with data, medium=SharedMem.
        let _ = parser.parse_chunk("a=f,f=32,s=1,v=1,t=s,i=1;QUFB");
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Shared memory not supported for frames"));
    }

    #[test]
    fn test_build_graphic_frame_num_one_creates_placement_and_animation() {
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut parser = KittyParser::new();
        // Frame 1: should create both an animation frame AND a placement.
        parser
            .parse_chunk(&format!(
                "a=f,f=32,s=2,v=2,i=77,r=1,z=50,c=1,x=1,y=2;{}",
                b64
            ))
            .unwrap();
        assert_eq!(parser.frame_number, Some(1));
        assert_eq!(parser.frame_delay_ms, Some(50));
        assert_eq!(parser.frame_composition, Some(CompositionMode::Overwrite));

        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((3, 4), &mut store).unwrap();
        match result {
            KittyGraphicResult::Graphic(g) => {
                assert_eq!(g.kitty_image_id, Some(77));
                assert_eq!(g.position, (3, 4));
                assert_eq!(g.placement.x_offset, 1);
                assert_eq!(g.placement.y_offset, 2);
                // x= was also used for frame offset
            }
            other => panic!("Expected Graphic for frame 1, got {:?}", other),
        }
        // Animation frame should be present.
        let anim = store.get_animation(77);
        assert!(anim.is_some(), "animation should exist for image_id=77");
        let frame = anim.unwrap().get_frame(1);
        assert!(frame.is_some());
        assert_eq!(frame.unwrap().delay_ms, 50);
        // Image should also be stored for Put reuse.
        assert!(store.get_kitty_image(77).is_some());
    }

    #[test]
    fn test_build_graphic_frame_subsequent_no_placement() {
        // Pre-seed an animation by transmitting frame 1 first.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();

        let mut f1 = KittyParser::new();
        f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=9,r=1;{}", b64))
            .unwrap();
        let _ = f1.build_graphic((0, 0), &mut store).unwrap();

        // Now frame 2: should NOT create a new placement.
        let mut f2 = KittyParser::new();
        f2.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=9,r=2;{}", b64))
            .unwrap();
        let result = f2.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
        // Animation should now have 2 frames.
        let anim = store.get_animation(9).unwrap();
        assert_eq!(anim.frame_count(), 2);
    }

    // --- build_graphic: AnimationControl ---

    #[test]
    fn test_build_graphic_animation_control_missing_image_id() {
        let mut parser = KittyParser::new();
        parser.action = KittyAction::AnimationControl;
        // No i=
        let mut store = GraphicsStore::new();
        let result = parser.build_graphic((0, 0), &mut store);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Animation control requires image ID"));
    }

    #[test]
    fn test_build_graphic_animation_control_with_state() {
        // Seed an animation so control_animation has something to act on.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();
        let mut f1 = KittyParser::new();
        f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=5,r=1;{}", b64))
            .unwrap();
        let _ = f1.build_graphic((0, 0), &mut store).unwrap();

        // Now send animation control: s=1 (stop), v=2 (num_plays=2).
        let mut ctrl = KittyParser::new();
        ctrl.parse_chunk("a=a,i=5,s=1,v=2;").unwrap();
        let result = ctrl.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));

        // num_plays=2 -> loop_count = N-1 = 1.
        let anim = store.get_animation(5).unwrap();
        assert_eq!(anim.loop_count, 1);
    }

    #[test]
    fn test_build_graphic_animation_control_num_plays_zero_ignored() {
        // Per spec, v=0 is ignored.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();
        let mut f1 = KittyParser::new();
        f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=6,r=1;{}", b64))
            .unwrap();
        let _ = f1.build_graphic((0, 0), &mut store).unwrap();

        let anim_before = store.get_animation(6).unwrap().loop_count;
        let mut ctrl = KittyParser::new();
        ctrl.parse_chunk("a=a,i=6,v=0;").unwrap();
        let _ = ctrl.build_graphic((0, 0), &mut store).unwrap();
        let anim_after = store.get_animation(6).unwrap().loop_count;
        assert_eq!(anim_before, anim_after, "v=0 must be ignored");
    }

    #[test]
    fn test_build_graphic_animation_control_num_plays_one_is_infinite() {
        // v=1 means infinite -> loop_count = 0.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();
        let mut f1 = KittyParser::new();
        f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=7,r=1;{}", b64))
            .unwrap();
        let _ = f1.build_graphic((0, 0), &mut store).unwrap();

        let mut ctrl = KittyParser::new();
        ctrl.parse_chunk("a=a,i=7,v=1;").unwrap();
        let _ = ctrl.build_graphic((0, 0), &mut store).unwrap();
        let anim = store.get_animation(7).unwrap();
        assert_eq!(anim.loop_count, 0, "v=1 must mean infinite (loop_count=0)");
    }

    #[test]
    fn test_build_graphic_animation_control_no_state_only_loops() {
        // Animation control with only v= (no s=) should still set loop_count.
        let pixels: Vec<u8> = vec![255, 0, 0, 255];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
        let mut store = GraphicsStore::new();
        let mut f1 = KittyParser::new();
        f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=8,r=1;{}", b64))
            .unwrap();
        let _ = f1.build_graphic((0, 0), &mut store).unwrap();

        let mut ctrl = KittyParser::new();
        ctrl.parse_chunk("a=a,i=8,v=3;").unwrap(); // no s=
        let result = ctrl.build_graphic((0, 0), &mut store).unwrap();
        assert!(matches!(result, KittyGraphicResult::None));
        // v=3 -> loop_count = 2
        let anim = store.get_animation(8).unwrap();
        assert_eq!(anim.loop_count, 2);
    }

    // --- load_file_data additional security/edge paths ---

    #[test]
    fn test_load_file_data_invalid_utf8_returns_error() {
        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File;
        // 0xFF is invalid as leading byte in UTF-8.
        let result = parser.load_file_data(&[0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid UTF-8") || msg.contains("file path"));
    }

    #[test]
    fn test_load_file_data_path_is_directory_returns_error() {
        // A directory exists and is not a regular file.
        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File;
        let dir = std::env::temp_dir(); // guaranteed to exist and be a dir
        let result = parser.load_file_data(dir.to_string_lossy().as_bytes());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a file"));
    }

    #[test]
    fn test_load_file_data_directory_traversal_in_middle_of_path() {
        // ".." anywhere in the path must be rejected.
        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File;
        let result = parser.load_file_data(b"/tmp/foo/../bar.png");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Directory traversal"));
    }

    #[test]
    fn test_load_file_data_temp_file_is_deleted_after_read() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Write a real PNG to a NamedTempFile, then close the handle (keep path).
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 8, 7, 6]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let mut tf = NamedTempFile::new().unwrap();
        tf.write_all(&png).unwrap();
        let (file, path) = tf.keep().expect("keep temp file");
        drop(file); // close OS handle so removal can succeed

        let path_str = path.to_string_lossy().into_owned();
        assert!(path.exists());

        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::TempFile;
        let data = parser.load_file_data(path_str.as_bytes()).unwrap();
        assert_eq!(data, png);
        // TempFile medium: file must have been removed during load.
        assert!(!path.exists(), "temp file should be deleted after read");

        // Clean up defensively in case the assertion above failed.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_file_data_valid_file_round_trip() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 1, 1, 1]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let mut tf = NamedTempFile::new().unwrap();
        tf.write_all(&png).unwrap();
        let path = tf.path().to_string_lossy().into_owned();
        // Keep tf alive so the file persists through the read.

        let mut parser = KittyParser::new();
        parser.medium = KittyMedium::File; // NOT temp file -> should remain
        let data = parser.load_file_data(path.as_bytes()).unwrap();
        assert_eq!(data, png);
        assert!(tf.path().exists(), "non-temp file should NOT be deleted");
    }

    // --- KittyGraphicResult Debug round-trip (cheap enum coverage) ---

    #[test]
    fn test_kitty_graphic_result_debug_repr() {
        // Each variant's Debug impl should not panic; format! exercises it.
        let none = KittyGraphicResult::None;
        let s = format!("{:?}", none);
        assert!(s.contains("None"));

        let virt = KittyGraphicResult::VirtualPlacement {
            image_id: 1,
            placement_id: 2,
            position: (0, 0),
            cols: 1,
            rows: 1,
        };
        let s = format!("{:?}", virt);
        assert!(s.contains("VirtualPlacement"));
        assert!(s.contains("image_id"));
    }

    // --- decompress_zlib: empty input ---

    #[test]
    fn test_decompress_zlib_empty_input_succeeds_with_empty_output() {
        // ZlibDecoder treats empty input as a valid empty stream.
        let result = KittyParser::decompress_zlib(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_decompress_zlib_empty_valid_stream() {
        // A valid zlib stream that decompresses to zero bytes.
        // RFC 1950 wrapper around deflate of empty stored block.
        let empty_zlib: [u8; 8] = [0x78, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
        let result = KittyParser::decompress_zlib(&empty_zlib);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // --- is_compressed() reflects compression flag ---

    #[test]
    fn test_is_compressed_reflects_compression_field() {
        let mut parser = KittyParser::new();
        assert!(!parser.is_compressed());
        parser.compression = KittyCompression::Zlib;
        assert!(parser.is_compressed());
    }

    // --- build_placement: z_index populated when set ---

    #[test]
    fn test_build_placement_negative_z_index() {
        let mut parser = KittyParser::new();
        parser.z_index = Some(-100);
        let placement = parser.build_placement();
        assert_eq!(placement.z_index, -100);
    }

    #[test]
    fn test_build_placement_all_fields_set() {
        let mut parser = KittyParser::new();
        parser.columns = Some(6);
        parser.rows = Some(4);
        parser.z_index = Some(2);
        parser.x_offset = Some(7);
        parser.y_offset = Some(9);
        let placement = parser.build_placement();
        assert_eq!(placement.columns, Some(6));
        assert_eq!(placement.rows, Some(4));
        assert_eq!(placement.z_index, 2);
        assert_eq!(placement.x_offset, 7);
        assert_eq!(placement.y_offset, 9);
    }

    // --- KittyAction/KittyFormat/KittyMedium/KittyCompression defaults & Debug ---

    #[test]
    fn test_kitty_action_default_is_transmit() {
        assert_eq!(KittyAction::default(), KittyAction::Transmit);
    }

    #[test]
    fn test_kitty_format_default_is_rgba() {
        assert_eq!(KittyFormat::default(), KittyFormat::Rgba);
    }

    #[test]
    fn test_kitty_medium_default_is_direct() {
        assert_eq!(KittyMedium::default(), KittyMedium::Direct);
    }

    #[test]
    fn test_kitty_compression_default_is_none() {
        assert_eq!(KittyCompression::default(), KittyCompression::None);
    }

    #[test]
    fn test_kitty_action_all_chars_covered() {
        // Complete coverage of from_char for every documented action.
        assert_eq!(KittyAction::from_char('f'), Some(KittyAction::Frame));
        assert_eq!(
            KittyAction::from_char('a'),
            Some(KittyAction::AnimationControl)
        );
        // Empty string -> next() is None -> action is left unchanged.
        let mut parser = KittyParser::new();
        parser.action = KittyAction::Query;
        let _ = parser.parse_chunk("a=;");
        assert_eq!(parser.action, KittyAction::Query);
    }

    #[test]
    fn test_kitty_format_from_code_uncovered_codes() {
        // Confirm a couple of additional edge cases.
        assert_eq!(KittyFormat::from_code(1), None);
        assert_eq!(KittyFormat::from_code(u32::MAX), None);
    }

    #[test]
    fn test_kitty_compression_from_char_only_z() {
        // Every non-'z' char returns None.
        for c in ['a', 'Z', '0', ' ', '\0'] {
            assert_eq!(KittyCompression::from_char(c), None);
        }
    }

    #[test]
    fn test_kitty_medium_from_char_invalid_chars() {
        assert_eq!(KittyMedium::from_char('D'), None); // uppercase not valid
        assert_eq!(KittyMedium::from_char('F'), None);
        assert_eq!(KittyMedium::from_char(' '), None);
    }
}
