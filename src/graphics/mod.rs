//! Unified graphics protocol support
//!
//! Multi-protocol graphics support for Sixel, iTerm2 inline images, and Kitty graphics protocol.
//!
//! # Supported Protocols
//! - **Sixel**: DEC VT340 compatible bitmap graphics
//! - **iTerm2**: OSC 1337 inline images (PNG, JPEG, GIF)
//! - **Kitty**: APC-based graphics protocol with image reuse
//!
//! # Architecture
//! All protocols are normalized to a unified `TerminalGraphic` representation with RGBA pixel data.
//! The `GraphicsStore` handles storage, scrolling, and Kitty image ID reuse.

pub mod animation;
pub mod iterm;
pub mod kitty;
pub mod placeholder;

use std::collections::HashMap;
use std::sync::Arc;

// Re-export for convenience
pub use animation::{Animation, AnimationControl, AnimationFrame, AnimationState, CompositionMode};
pub use placeholder::{
    create_placeholder_with_diacritics, number_to_diacritic, PlaceholderInfo, PLACEHOLDER_CHAR,
};

/// Graphics protocol identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicProtocol {
    Sixel,
    ITermInline, // OSC 1337
    Kitty,       // APC graphics protocol
}

impl GraphicProtocol {
    /// Get protocol name as string
    pub fn as_str(&self) -> &'static str {
        match self {
            GraphicProtocol::Sixel => "sixel",
            GraphicProtocol::ITermInline => "iterm",
            GraphicProtocol::Kitty => "kitty",
        }
    }
}

/// Limits for graphics to prevent resource exhaustion
#[derive(Debug, Clone, Copy)]
pub struct GraphicsLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: usize,
    pub max_total_memory: usize,
    pub max_graphics_count: usize,
    pub max_scrollback_graphics: usize,
}

impl Default for GraphicsLimits {
    fn default() -> Self {
        Self {
            max_width: 10000,
            max_height: 10000,
            max_pixels: 25_000_000,              // 25MP
            max_total_memory: 256 * 1024 * 1024, // 256MB
            max_graphics_count: 1000,
            max_scrollback_graphics: 500,
        }
    }
}

/// Protocol-agnostic graphic representation
#[derive(Debug, Clone)]
pub struct TerminalGraphic {
    /// Unique placement ID
    pub id: u64,
    /// Graphics protocol used
    pub protocol: GraphicProtocol,
    /// Position in terminal (col, row)
    pub position: (usize, usize),
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
    /// RGBA pixel data (Arc for Kitty sharing)
    pub pixels: Arc<Vec<u8>>,
    /// Cell dimensions (cell_width, cell_height) for rendering
    pub cell_dimensions: Option<(u32, u32)>,
    /// Rows scrolled off visible area (for partial rendering)
    pub scroll_offset_rows: usize,
    /// Row in scrollback buffer (only set when in scrollback)
    pub scrollback_row: Option<usize>,

    // Kitty-specific (None for other protocols)
    /// Kitty image ID for image reuse
    pub kitty_image_id: Option<u32>,
    /// Kitty placement ID
    pub kitty_placement_id: Option<u32>,
    /// Virtual placement (U=1) - used as prototype for Unicode placeholders
    pub is_virtual: bool,
    /// Parent placement for relative positioning (P= key)
    pub parent_image_id: Option<u32>,
    /// Parent placement ID for relative positioning (Q= key)
    pub parent_placement_id: Option<u32>,
    /// X offset relative to parent placement (in pixels)
    pub relative_x_offset: i32,
    /// Y offset relative to parent placement (in pixels)
    pub relative_y_offset: i32,
}

impl TerminalGraphic {
    /// Create a new terminal graphic
    pub fn new(
        id: u64,
        protocol: GraphicProtocol,
        position: (usize, usize),
        width: usize,
        height: usize,
        pixels: Vec<u8>,
    ) -> Self {
        Self {
            id,
            protocol,
            position,
            width,
            height,
            pixels: Arc::new(pixels),
            cell_dimensions: None,
            scroll_offset_rows: 0,
            scrollback_row: None,
            kitty_image_id: None,
            kitty_placement_id: None,
            is_virtual: false,
            parent_image_id: None,
            parent_placement_id: None,
            relative_x_offset: 0,
            relative_y_offset: 0,
        }
    }

    /// Create with shared pixel data (for Kitty image reuse)
    pub fn with_shared_pixels(
        id: u64,
        protocol: GraphicProtocol,
        position: (usize, usize),
        width: usize,
        height: usize,
        pixels: Arc<Vec<u8>>,
    ) -> Self {
        Self {
            id,
            protocol,
            position,
            width,
            height,
            pixels,
            cell_dimensions: None,
            scroll_offset_rows: 0,
            scrollback_row: None,
            kitty_image_id: None,
            kitty_placement_id: None,
            is_virtual: false,
            parent_image_id: None,
            parent_placement_id: None,
            relative_x_offset: 0,
            relative_y_offset: 0,
        }
    }

    /// Set cell dimensions used when creating this graphic
    pub fn set_cell_dimensions(&mut self, cell_width: u32, cell_height: u32) {
        self.cell_dimensions = Some((cell_width, cell_height));
    }

    /// Calculate how many terminal cells this graphic spans
    pub fn cell_span(&self, fallback_cell_width: u32, fallback_cell_height: u32) -> (usize, usize) {
        let (cell_w, cell_h) = self
            .cell_dimensions
            .unwrap_or((fallback_cell_width, fallback_cell_height));
        let cols = (self.width as u32).div_ceil(cell_w) as usize;
        let rows = (self.height as u32).div_ceil(cell_h) as usize;
        (cols, rows)
    }

    /// Get RGBA color at pixel coordinates
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y * self.width + x) * 4;
        if offset + 3 >= self.pixels.len() {
            return None;
        }
        Some((
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ))
    }

    /// Alias for pixel_at (compatibility with SixelGraphic API)
    pub fn get_pixel(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        self.pixel_at(x, y)
    }

    /// Sample color for half-block cell rendering
    /// Returns (top_half_rgba, bottom_half_rgba) for the cell at (col, row)
    #[allow(clippy::type_complexity)]
    pub fn sample_half_block(
        &self,
        cell_col: usize,
        cell_row: usize,
        cell_width: u32,
        cell_height: u32,
    ) -> Option<((u8, u8, u8, u8), (u8, u8, u8, u8))> {
        // Calculate pixel coordinates relative to graphic position
        let rel_col = cell_col.checked_sub(self.position.0)?;
        let rel_row = cell_row.checked_sub(self.position.1)?;

        let px_x = rel_col * cell_width as usize;
        let px_y = rel_row * cell_height as usize;

        // Sample center of top and bottom halves
        let top_y = px_y + cell_height as usize / 4;
        let bottom_y = px_y + (cell_height as usize * 3) / 4;
        let center_x = px_x + cell_width as usize / 2;

        let top = self.pixel_at(center_x, top_y)?;
        let bottom = self.pixel_at(center_x, bottom_y)?;

        Some((top, bottom))
    }

    /// Get dimensions in terminal cells
    pub fn cell_size(&self, cell_width: u32, cell_height: u32) -> (usize, usize) {
        let cols = self.width.div_ceil(cell_width as usize);
        let rows = self.height.div_ceil(cell_height as usize);
        (cols, rows)
    }

    /// Calculate height in terminal rows
    pub fn height_in_rows(&self, cell_height: u32) -> usize {
        let cell_h = self.cell_dimensions.map(|(_, h)| h).unwrap_or(cell_height);
        (self.height as u32).div_ceil(cell_h) as usize
    }
}

/// Global counter for unique graphic IDs
static GRAPHIC_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Generate a unique graphic placement ID
pub fn next_graphic_id() -> u64 {
    GRAPHIC_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Centralized graphics storage supporting image reuse
#[derive(Debug, Default)]
pub struct GraphicsStore {
    /// Kitty shared images: image_id -> (width, height, pixel_data)
    shared_images: HashMap<u32, (usize, usize, Arc<Vec<u8>>)>,

    /// All active placements (visible area)
    placements: Vec<TerminalGraphic>,

    /// Virtual placements - prototypes for Unicode placeholder images
    /// Key is (image_id, placement_id)
    virtual_placements: HashMap<(u32, u32), TerminalGraphic>,

    /// Animations indexed by image ID
    animations: HashMap<u32, Animation>,

    /// Graphics in scrollback (keyed by scrollback row)
    scrollback: Vec<TerminalGraphic>,

    /// Current scrollback row counter (incremented when lines scroll off)
    scrollback_position: usize,

    /// Resource limits
    limits: GraphicsLimits,

    /// Count of graphics dropped due to limits
    dropped_count: usize,
}

impl GraphicsStore {
    /// Create a new graphics store with default limits
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with custom limits
    pub fn with_limits(limits: GraphicsLimits) -> Self {
        Self {
            limits,
            ..Default::default()
        }
    }

    /// Add a graphic placement
    pub fn add_graphic(&mut self, graphic: TerminalGraphic) {
        // Enforce placement limit
        if self.placements.len() >= self.limits.max_graphics_count {
            // Remove oldest placement
            self.placements.remove(0);
            self.dropped_count += 1;
        }
        self.placements.push(graphic);
    }

    /// Remove a graphic by ID
    pub fn remove_graphic(&mut self, id: u64) {
        self.placements.retain(|g| g.id != id);
    }

    /// Get graphics at a specific row
    pub fn graphics_at_row(&self, row: usize) -> Vec<&TerminalGraphic> {
        self.placements
            .iter()
            .filter(|g| {
                let start_row = g.position.1;
                // Default cell height of 2 for half-block rendering
                let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
                let end_row = start_row + g.height.div_ceil(cell_height);
                row >= start_row && row < end_row
            })
            .collect()
    }

    /// Get all active graphics
    pub fn all_graphics(&self) -> &[TerminalGraphic] {
        &self.placements
    }

    /// Get mutable access to all graphics
    pub fn all_graphics_mut(&mut self) -> &mut Vec<TerminalGraphic> {
        &mut self.placements
    }

    /// Get total graphics count
    pub fn graphics_count(&self) -> usize {
        self.placements.len()
    }

    /// Get count of graphics dropped due to limits
    pub fn dropped_count(&self) -> usize {
        self.dropped_count
    }

    /// Get current limits
    pub fn limits(&self) -> &GraphicsLimits {
        &self.limits
    }

    /// Set maximum graphics count
    pub fn set_max_graphics(&mut self, max: usize) {
        self.limits.max_graphics_count = max;
        // Enforce new limit
        while self.placements.len() > max {
            self.placements.remove(0);
        }
    }

    /// Clear all graphics
    pub fn clear(&mut self) {
        self.placements.clear();
    }

    // --- Kitty image management ---

    /// Store a Kitty image for later reuse
    pub fn store_kitty_image(
        &mut self,
        image_id: u32,
        width: usize,
        height: usize,
        pixels: Vec<u8>,
    ) {
        self.shared_images
            .insert(image_id, (width, height, Arc::new(pixels)));
    }

    /// Get a stored Kitty image
    pub fn get_kitty_image(&self, image_id: u32) -> Option<(usize, usize, Arc<Vec<u8>>)> {
        self.shared_images.get(&image_id).cloned()
    }

    /// Remove a Kitty image
    pub fn remove_kitty_image(&mut self, image_id: u32) {
        self.shared_images.remove(&image_id);
    }

    /// Delete graphics by Kitty criteria
    pub fn delete_kitty_graphics(&mut self, image_id: Option<u32>, placement_id: Option<u32>) {
        self.placements.retain(|g| {
            if g.protocol != GraphicProtocol::Kitty {
                return true;
            }
            if let Some(iid) = image_id {
                if g.kitty_image_id != Some(iid) {
                    return true;
                }
            }
            if let Some(pid) = placement_id {
                if g.kitty_placement_id != Some(pid) {
                    return true;
                }
            }
            // Matches criteria, remove it
            false
        });

        // Also delete from virtual placements if criteria match
        if let (Some(iid), Some(pid)) = (image_id, placement_id) {
            self.virtual_placements.remove(&(iid, pid));
        } else if let Some(iid) = image_id {
            // Remove all virtual placements with this image_id
            self.virtual_placements
                .retain(|(img_id, _), _| *img_id != iid);
        }
    }

    // --- Virtual placements ---

    /// Add or update a virtual placement
    pub fn add_virtual_placement(&mut self, mut graphic: TerminalGraphic) {
        graphic.is_virtual = true;
        let image_id = graphic.kitty_image_id.unwrap_or(0);
        let placement_id = graphic.kitty_placement_id.unwrap_or(0);
        self.virtual_placements
            .insert((image_id, placement_id), graphic);
    }

    /// Get a virtual placement
    pub fn get_virtual_placement(
        &self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&TerminalGraphic> {
        self.virtual_placements.get(&(image_id, placement_id))
    }

    /// Remove a virtual placement
    pub fn remove_virtual_placement(
        &mut self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<TerminalGraphic> {
        self.virtual_placements.remove(&(image_id, placement_id))
    }

    /// Get all virtual placements
    pub fn all_virtual_placements(&self) -> &HashMap<(u32, u32), TerminalGraphic> {
        &self.virtual_placements
    }

    /// Get a virtual placement for rendering a Unicode placeholder
    ///
    /// This looks up the virtual placement using the image_id and placement_id
    /// from the placeholder info, and returns it for rendering.
    pub fn get_placeholder_graphic(
        &self,
        placeholder_info: &PlaceholderInfo,
    ) -> Option<&TerminalGraphic> {
        let image_id = placeholder_info.full_image_id();
        let placement_id = placeholder_info.placement_id;

        // Try exact match first
        if let Some(graphic) = self.virtual_placements.get(&(image_id, placement_id)) {
            return Some(graphic);
        }

        // If placement_id is 0, try to find any virtual placement for this image
        if placement_id == 0 {
            for ((img_id, _pid), graphic) in &self.virtual_placements {
                if *img_id == image_id {
                    return Some(graphic);
                }
            }
        }

        None
    }

    // --- Animation management ---

    /// Create or get animation for an image
    pub fn get_or_create_animation(
        &mut self,
        image_id: u32,
        default_delay_ms: u32,
    ) -> &mut Animation {
        self.animations
            .entry(image_id)
            .or_insert_with(|| Animation::new(image_id, default_delay_ms))
    }

    /// Get animation for an image
    pub fn get_animation(&self, image_id: u32) -> Option<&Animation> {
        self.animations.get(&image_id)
    }

    /// Get mutable animation for an image
    pub fn get_animation_mut(&mut self, image_id: u32) -> Option<&mut Animation> {
        self.animations.get_mut(&image_id)
    }

    /// Add a frame to an animation
    pub fn add_animation_frame(&mut self, image_id: u32, frame: AnimationFrame) {
        let frame_num = frame.frame_number;
        let default_delay = frame.delay_ms.max(100); // Default to 100ms if not specified
        let anim = self.get_or_create_animation(image_id, default_delay);
        anim.add_frame(frame);
        debug_info!(
            "GRAPHICS",
            "Added animation frame {} to image_id={} (total frames: {})",
            frame_num,
            image_id,
            anim.frame_count()
        );
    }

    /// Apply animation control to an image
    pub fn control_animation(&mut self, image_id: u32, control: AnimationControl) {
        if let Some(anim) = self.get_animation_mut(image_id) {
            anim.apply_control(control);
        }
    }

    /// Set loop count for an animation
    pub fn set_animation_loops(&mut self, image_id: u32, loop_count: u32) {
        if let Some(anim) = self.get_animation_mut(image_id) {
            anim.set_loops(loop_count);
        }
    }

    /// Update all animations and return list of image IDs that changed frames
    ///
    /// This method advances animation frames based on timing and updates the pixel data
    /// in all placements associated with animated images.
    pub fn update_animations(&mut self) -> Vec<u32> {
        let mut changed = Vec::new();
        for (image_id, anim) in &mut self.animations {
            if anim.update() {
                changed.push(*image_id);

                // Update pixel data in all placements for this animated image
                if let Some(current_frame) = anim.current_frame() {
                    // Clone the pixels arc for sharing with placements
                    let frame_pixels = current_frame.pixels.clone();

                    // Update all placements that reference this image
                    for placement in &mut self.placements {
                        if placement.kitty_image_id == Some(*image_id) {
                            placement.pixels = frame_pixels.clone();
                            placement.width = current_frame.width;
                            placement.height = current_frame.height;
                        }
                    }
                }
            }
        }
        changed
    }

    /// Remove animation for an image
    pub fn remove_animation(&mut self, image_id: u32) {
        self.animations.remove(&image_id);
    }

    /// Get all animations
    pub fn all_animations(&self) -> &HashMap<u32, Animation> {
        &self.animations
    }

    // --- Scrolling ---

    /// Notify that lines have been added to text scrollback
    /// This should be called when text scrolls off the screen
    pub fn notify_scrollback_advance(&mut self, lines: usize) {
        self.scrollback_position += lines;
    }

    /// Get current scrollback position
    pub fn scrollback_position(&self) -> usize {
        self.scrollback_position
    }

    /// Adjust graphics positions when scrolling up
    pub fn adjust_for_scroll_up(&mut self, lines: usize, top: usize, bottom: usize) {
        self.adjust_for_scroll_up_with_scrollback(lines, top, bottom, 0);
    }

    /// Adjust graphics positions when scrolling up, with scrollback tracking
    ///
    /// # Arguments
    /// * `lines` - Number of lines to scroll
    /// * `top` - Top of scroll region
    /// * `bottom` - Bottom of scroll region
    /// * `grid_scrollback_len` - Current length of text scrollback buffer
    pub fn adjust_for_scroll_up_with_scrollback(
        &mut self,
        lines: usize,
        top: usize,
        bottom: usize,
        grid_scrollback_len: usize,
    ) {
        let mut to_scrollback = Vec::new();

        self.placements.retain_mut(|g| {
            let graphic_row = g.position.1;
            let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
            let graphic_height_in_rows = g.height.div_ceil(cell_height);
            let graphic_bottom = graphic_row + graphic_height_in_rows;

            // Check if graphic is within the scroll region
            if graphic_bottom > top && graphic_row <= bottom && graphic_row >= top {
                // Adjust position
                let new_position = graphic_row.saturating_sub(lines);
                let additional_scroll = lines.saturating_sub(graphic_row);
                g.scroll_offset_rows = g.scroll_offset_rows.saturating_add(additional_scroll);
                g.position.1 = new_position;

                // Check if completely scrolled off
                if g.scroll_offset_rows >= graphic_height_in_rows {
                    // Move to scrollback - set scrollback_row to match text scrollback position
                    // The graphic was originally at graphic_row, which is now at scrollback position
                    let mut scrollback_graphic = g.clone();
                    scrollback_graphic.scrollback_row = Some(grid_scrollback_len);

                    to_scrollback.push(scrollback_graphic);
                    return false;
                }
            }
            true
        });

        // Add to scrollback (with limit)
        for g in to_scrollback {
            if self.scrollback.len() >= self.limits.max_scrollback_graphics {
                self.scrollback.remove(0);
            }
            self.scrollback.push(g);
        }
    }

    /// Adjust graphics positions when scrolling down
    pub fn adjust_for_scroll_down(&mut self, lines: usize, top: usize, bottom: usize) {
        for g in &mut self.placements {
            let graphic_row = g.position.1;
            let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
            let graphic_height_in_rows = g.height.div_ceil(cell_height);
            let graphic_bottom = graphic_row + graphic_height_in_rows;

            // Graphic starts within scroll region
            if graphic_bottom > top && graphic_row >= top && graphic_row <= bottom {
                let new_row = graphic_row + lines;
                if new_row <= bottom {
                    g.position.1 = new_row;
                }
            }
        }
    }

    // --- Scrollback ---

    /// Get graphics in scrollback for a range of scrollback rows
    pub fn graphics_in_scrollback(
        &self,
        start_row: usize,
        end_row: usize,
    ) -> Vec<&TerminalGraphic> {
        self.scrollback
            .iter()
            .filter(|g| {
                if let Some(sb_row) = g.scrollback_row {
                    sb_row >= start_row && sb_row < end_row
                } else {
                    false
                }
            })
            .collect()
    }

    /// Get all scrollback graphics
    pub fn all_scrollback_graphics(&self) -> &[TerminalGraphic] {
        &self.scrollback
    }

    /// Clear scrollback graphics
    pub fn clear_scrollback_graphics(&mut self) {
        self.scrollback.clear();
    }

    /// Get scrollback graphics count
    pub fn scrollback_count(&self) -> usize {
        self.scrollback.len()
    }
}

/// Graphics error types
#[derive(Debug, Clone)]
pub enum GraphicsError {
    InvalidDimensions(u32, u32),
    ImageTooLarge(usize, usize),
    UnsupportedFormat(String),
    DecodeError(String),
    Base64Error(String),
    ImageError(String),
    KittyError(String),
    ITermError(String),
}

impl std::fmt::Display for GraphicsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphicsError::InvalidDimensions(w, h) => {
                write!(f, "Invalid image dimensions: {}x{}", w, h)
            }
            GraphicsError::ImageTooLarge(size, max) => {
                write!(f, "Image too large: {} bytes (max {})", size, max)
            }
            GraphicsError::UnsupportedFormat(fmt) => write!(f, "Unsupported format: {}", fmt),
            GraphicsError::DecodeError(msg) => write!(f, "Decode error: {}", msg),
            GraphicsError::Base64Error(msg) => write!(f, "Invalid base64: {}", msg),
            GraphicsError::ImageError(msg) => write!(f, "Image decode failed: {}", msg),
            GraphicsError::KittyError(msg) => write!(f, "Kitty protocol error: {}", msg),
            GraphicsError::ITermError(msg) => write!(f, "iTerm protocol error: {}", msg),
        }
    }
}

impl std::error::Error for GraphicsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graphic_protocol_as_str() {
        assert_eq!(GraphicProtocol::Sixel.as_str(), "sixel");
        assert_eq!(GraphicProtocol::ITermInline.as_str(), "iterm");
        assert_eq!(GraphicProtocol::Kitty.as_str(), "kitty");
    }

    #[test]
    fn test_terminal_graphic_new() {
        let pixels = vec![255u8; 40]; // 10 RGBA pixels
        let graphic = TerminalGraphic::new(1, GraphicProtocol::Sixel, (5, 10), 10, 1, pixels);
        assert_eq!(graphic.id, 1);
        assert_eq!(graphic.position, (5, 10));
        assert_eq!(graphic.width, 10);
        assert_eq!(graphic.height, 1);
    }

    #[test]
    fn test_terminal_graphic_pixel_at() {
        // 2x2 image, RGBA
        let pixels = vec![
            255, 0, 0, 255, // (0,0) red
            0, 255, 0, 255, // (1,0) green
            0, 0, 255, 255, // (0,1) blue
            255, 255, 0, 255, // (1,1) yellow
        ];
        let graphic = TerminalGraphic::new(1, GraphicProtocol::Sixel, (0, 0), 2, 2, pixels);

        assert_eq!(graphic.pixel_at(0, 0), Some((255, 0, 0, 255)));
        assert_eq!(graphic.pixel_at(1, 0), Some((0, 255, 0, 255)));
        assert_eq!(graphic.pixel_at(0, 1), Some((0, 0, 255, 255)));
        assert_eq!(graphic.pixel_at(1, 1), Some((255, 255, 0, 255)));
        assert_eq!(graphic.pixel_at(2, 0), None);
    }

    #[test]
    fn test_graphics_store_add_remove() {
        let mut store = GraphicsStore::new();
        let graphic = TerminalGraphic::new(1, GraphicProtocol::Sixel, (0, 0), 10, 10, vec![]);

        store.add_graphic(graphic);
        assert_eq!(store.graphics_count(), 1);

        store.remove_graphic(1);
        assert_eq!(store.graphics_count(), 0);
    }

    #[test]
    fn test_graphics_store_kitty_image() {
        let mut store = GraphicsStore::new();
        let pixels = vec![255u8; 16];

        store.store_kitty_image(42, 2, 2, pixels);

        let result = store.get_kitty_image(42);
        assert!(result.is_some());
        let (w, h, data) = result.unwrap();
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(data.len(), 16);

        store.remove_kitty_image(42);
        assert!(store.get_kitty_image(42).is_none());
    }
}
