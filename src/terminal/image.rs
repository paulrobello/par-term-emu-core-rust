//! Image protocol support
//!
//! Provides types for Sixel, iTerm2, and Kitty graphics protocols.

use std::collections::HashMap;

/// Image protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    /// Sixel graphics
    Sixel,
    /// iTerm2 inline images
    ITerm2,
    /// Kitty graphics protocol
    Kitty,
}

/// Image format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    PNG,
    JPEG,
    GIF,
    BMP,
    RGBA,
    RGB,
}

/// Inline image data
#[derive(Debug, Clone)]
pub struct InlineImage {
    /// Image identifier
    pub id: Option<String>,
    /// Protocol used
    pub protocol: ImageProtocol,
    /// Image format
    pub format: ImageFormat,
    /// Image data (encoded)
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Position in terminal (col, row)
    pub position: (usize, usize),
    /// Display width in cells
    pub display_cols: usize,
    /// Display height in cells
    pub display_rows: usize,
}

/// Image placement action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePlacement {
    /// Display image
    Display,
    /// Delete image
    Delete,
    /// Query image
    Query,
}

/// State for multi-part iTerm2 image transfers (MultipartFile/FilePart protocol)
#[derive(Debug, Default)]
pub(crate) struct ITermMultipartState {
    /// Parameters from MultipartFile command (inline, size, name, etc.)
    pub params: HashMap<String, String>,
    /// Accumulated base64 chunks from FilePart commands
    pub chunks: Vec<String>,
    /// Expected total size in bytes (from size= parameter)
    pub total_size: Option<usize>,
    /// Current accumulated size (sum of decoded chunks)
    pub accumulated_size: usize,
    /// Whether this multipart transfer is a file transfer (not inline image)
    pub is_file_transfer: bool,
    /// Transfer ID if this is a file transfer (from FileTransferManager)
    pub transfer_id: Option<u64>,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 21: Image Protocol Support ===

    /// Add an inline image
    pub fn add_inline_image(&mut self, image: InlineImage) {
        self.inline_image_state.inline_images.push(image);

        // Limit number of stored images
        if self.inline_image_state.inline_images.len() > self.inline_image_state.max_inline_images {
            self.inline_image_state.inline_images.drain(
                0..self.inline_image_state.inline_images.len()
                    - self.inline_image_state.max_inline_images,
            );
        }
    }

    /// Get inline images at a specific position
    pub fn get_images_at(&self, col: usize, row: usize) -> Vec<InlineImage> {
        self.inline_image_state
            .inline_images
            .iter()
            .filter(|img| img.position == (col, row))
            .cloned()
            .collect()
    }

    /// Get all inline images
    pub fn get_all_images(&self) -> Vec<InlineImage> {
        self.inline_image_state.inline_images.clone()
    }

    /// Delete image by ID
    pub fn delete_image(&mut self, id: &str) -> bool {
        let before_len = self.inline_image_state.inline_images.len();
        self.inline_image_state
            .inline_images
            .retain(|img| img.id.as_ref().is_none_or(|img_id| img_id != id));
        self.inline_image_state.inline_images.len() < before_len
    }

    /// Clear all inline images
    pub fn clear_images(&mut self) {
        self.inline_image_state.inline_images.clear();
    }

    /// Get image by ID
    pub fn get_image_by_id(&self, id: &str) -> Option<InlineImage> {
        self.inline_image_state
            .inline_images
            .iter()
            .find(|img| img.id.as_ref().is_some_and(|img_id| img_id == id))
            .cloned()
    }

    /// Set maximum inline images
    pub fn set_max_inline_images(&mut self, max: usize) {
        self.inline_image_state.max_inline_images = max;
        if self.inline_image_state.inline_images.len() > max {
            self.inline_image_state
                .inline_images
                .drain(0..self.inline_image_state.inline_images.len() - max);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    fn make_image(id: &str, col: usize, row: usize) -> InlineImage {
        InlineImage {
            id: Some(id.to_string()),
            protocol: ImageProtocol::Sixel,
            format: ImageFormat::PNG,
            data: vec![1, 2, 3],
            width: 10,
            height: 10,
            position: (col, row),
            display_cols: 1,
            display_rows: 1,
        }
    }

    #[test]
    fn add_and_query_inline_images() {
        let mut term = Terminal::new(80, 24);
        assert!(term.get_all_images().is_empty());

        term.add_inline_image(make_image("a", 0, 0));
        assert_eq!(term.get_all_images().len(), 1);
        assert_eq!(term.get_images_at(0, 0).len(), 1);
        assert!(term.get_images_at(1, 1).is_empty());
    }

    #[test]
    fn get_image_by_id_and_delete() {
        let mut term = Terminal::new(80, 24);
        term.add_inline_image(make_image("a", 0, 0));
        term.add_inline_image(make_image("b", 1, 0));

        assert!(term.get_image_by_id("a").is_some());
        assert!(term.get_image_by_id("missing").is_none());

        assert!(term.delete_image("a"));
        assert!(term.get_image_by_id("a").is_none());
        assert_eq!(term.get_all_images().len(), 1);
        // Deleting a non-existent id returns false (nothing removed).
        assert!(!term.delete_image("a"));
    }

    #[test]
    fn clear_images_removes_all() {
        let mut term = Terminal::new(80, 24);
        term.add_inline_image(make_image("a", 0, 0));
        term.add_inline_image(make_image("b", 1, 0));
        term.clear_images();
        assert!(term.get_all_images().is_empty());
    }

    #[test]
    fn set_max_inline_images_evicts_oldest_after_the_fact() {
        let mut term = Terminal::new(80, 24);
        term.add_inline_image(make_image("a", 0, 0));
        term.add_inline_image(make_image("b", 1, 0));
        term.add_inline_image(make_image("c", 2, 0));

        // Lowering the cap evicts the oldest entries.
        term.set_max_inline_images(1);
        assert_eq!(term.get_all_images().len(), 1);
        assert!(term.get_image_by_id("a").is_none());
        assert!(term.get_image_by_id("c").is_some());
    }

    #[test]
    fn add_inline_image_enforces_cap_at_add_time() {
        let mut term = Terminal::new(80, 24);
        term.set_max_inline_images(2);
        term.add_inline_image(make_image("a", 0, 0));
        term.add_inline_image(make_image("b", 1, 0));
        term.add_inline_image(make_image("c", 2, 0));

        // Adding beyond the cap evicts the oldest entry.
        assert_eq!(term.get_all_images().len(), 2);
        assert!(term.get_image_by_id("a").is_none());
        assert!(term.get_image_by_id("c").is_some());
    }

    #[test]
    fn image_without_id_is_not_matched_by_id_queries() {
        let mut term = Terminal::new(80, 24);
        let mut img = make_image("x", 0, 0);
        img.id = None;
        term.add_inline_image(img);

        // get_image_by_id / delete_image ignore id-less images.
        assert!(term.get_image_by_id("x").is_none());
        assert!(!term.delete_image("x"));
        assert_eq!(term.get_all_images().len(), 1);
    }
}
