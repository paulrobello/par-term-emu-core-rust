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

pub mod iterm;
pub mod kitty;

use std::collections::HashMap;
use std::sync::Arc;

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
            max_pixels: 25_000_000, // 25MP
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

    // Kitty-specific (None for other protocols)
    /// Kitty image ID for image reuse
    pub kitty_image_id: Option<u32>,
    /// Kitty placement ID
    pub kitty_placement_id: Option<u32>,
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
            kitty_image_id: None,
            kitty_placement_id: None,
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
            kitty_image_id: None,
            kitty_placement_id: None,
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

    /// Sample color for half-block cell rendering
    /// Returns (top_half_rgba, bottom_half_rgba) for the cell at (col, row)
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
        let cols = (self.width + cell_width as usize - 1) / cell_width as usize;
        let rows = (self.height + cell_height as usize - 1) / cell_height as usize;
        (cols, rows)
    }

    /// Calculate height in terminal rows
    pub fn height_in_rows(&self, cell_height: u32) -> usize {
        let cell_h = self
            .cell_dimensions
            .map(|(_, h)| h)
            .unwrap_or(cell_height);
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

    /// Graphics in scrollback (keyed by scrollback row)
    scrollback: Vec<TerminalGraphic>,

    /// Resource limits
    limits: GraphicsLimits,
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

    /// Clear all graphics
    pub fn clear(&mut self) {
        self.placements.clear();
    }

    // --- Kitty image management ---

    /// Store a Kitty image for later reuse
    pub fn store_kitty_image(&mut self, image_id: u32, width: usize, height: usize, pixels: Vec<u8>) {
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
    }

    // --- Scrolling ---

    /// Adjust graphics positions when scrolling up
    pub fn adjust_for_scroll_up(&mut self, lines: usize, top: usize, bottom: usize) {
        let mut to_scrollback = Vec::new();

        self.placements.retain_mut(|g| {
            let graphic_row = g.position.1;
            let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
            let graphic_height_in_rows = g.height.div_ceil(cell_height);
            let graphic_bottom = graphic_row + graphic_height_in_rows;

            // Check if graphic is within or overlaps the scroll region
            if graphic_bottom > top && graphic_row <= bottom {
                if graphic_row >= top {
                    // Adjust position
                    let new_position = graphic_row.saturating_sub(lines);
                    let additional_scroll = lines.saturating_sub(graphic_row);
                    g.scroll_offset_rows = g.scroll_offset_rows.saturating_add(additional_scroll);
                    g.position.1 = new_position;

                    // Check if completely scrolled off
                    if g.scroll_offset_rows >= graphic_height_in_rows {
                        // Move to scrollback instead of deleting
                        to_scrollback.push(g.clone());
                        return false;
                    }
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

            if graphic_bottom > top && graphic_row <= bottom {
                if graphic_row >= top && graphic_row <= bottom {
                    let new_row = graphic_row + lines;
                    if new_row <= bottom {
                        g.position.1 = new_row;
                    }
                }
            }
        }
    }

    // --- Scrollback ---

    /// Get graphics in scrollback for a range of rows
    pub fn graphics_in_scrollback(&self, start_row: usize, end_row: usize) -> Vec<&TerminalGraphic> {
        self.scrollback
            .iter()
            .filter(|g| {
                let row = g.position.1;
                row >= start_row && row < end_row
            })
            .collect()
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
        let graphic = TerminalGraphic::new(
            1,
            GraphicProtocol::Sixel,
            (5, 10),
            10,
            1,
            pixels,
        );
        assert_eq!(graphic.id, 1);
        assert_eq!(graphic.position, (5, 10));
        assert_eq!(graphic.width, 10);
        assert_eq!(graphic.height, 1);
    }

    #[test]
    fn test_terminal_graphic_pixel_at() {
        // 2x2 image, RGBA
        let pixels = vec![
            255, 0, 0, 255,   // (0,0) red
            0, 255, 0, 255,   // (1,0) green
            0, 0, 255, 255,   // (0,1) blue
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
