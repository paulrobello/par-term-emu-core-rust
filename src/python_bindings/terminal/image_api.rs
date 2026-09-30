//! Image API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 21: Image Protocol Support ===

    /// Add an inline image
    ///
    /// Args:
    ///     image: PyInlineImage to add
    ///
    /// Raises:
    ///     ValueError: If the image's protocol or format name is not recognized
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     # InlineImage has no Python constructor, so images come from
    ///     # another terminal's store (get_all_images/get_images_at/get_image_by_id).
    ///     source = Terminal(80, 24)
    ///     term = Terminal(80, 24)
    ///     for image in source.get_all_images():
    ///         term.add_inline_image(image)
    ///     len(term.get_all_images()) == len(source.get_all_images())   # True
    ///     ```
    fn add_inline_image(
        &mut self,
        image: &crate::python_bindings::types::PyInlineImage,
    ) -> PyResult<()> {
        use crate::terminal::{ImageFormat, ImageProtocol, InlineImage};

        let protocol = match image.protocol.as_str() {
            "sixel" => ImageProtocol::Sixel,
            "iterm2" => ImageProtocol::ITerm2,
            "kitty" => ImageProtocol::Kitty,
            _ => return Err(PyValueError::new_err("Invalid image protocol")),
        };

        let format = match image.format.as_str() {
            "png" => ImageFormat::PNG,
            "jpeg" => ImageFormat::JPEG,
            "gif" => ImageFormat::GIF,
            "bmp" => ImageFormat::BMP,
            "rgba" => ImageFormat::RGBA,
            "rgb" => ImageFormat::RGB,
            _ => return Err(PyValueError::new_err("Invalid image format")),
        };

        let rust_image = InlineImage {
            id: image.id.clone(),
            protocol,
            format,
            data: image.data.clone(),
            width: image.width,
            height: image.height,
            position: image.position,
            display_cols: image.display_cols,
            display_rows: image.display_rows,
        };

        self.inner.add_inline_image(rust_image);
        Ok(())
    }

    /// Get inline images at a specific position
    ///
    /// Args:
    ///     col: Column index
    ///     row: Row index
    ///
    /// Returns:
    ///     List of PyInlineImage at the position
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_images_at(0, 0)   # []
    ///     ```
    fn get_images_at(
        &self,
        col: usize,
        row: usize,
    ) -> PyResult<Vec<crate::python_bindings::types::PyInlineImage>> {
        let images = self.inner.get_images_at(col, row);
        Ok(images
            .iter()
            .map(crate::python_bindings::types::PyInlineImage::from)
            .collect())
    }

    /// Get all inline images
    ///
    /// Returns:
    ///     List of all PyInlineImage
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_all_images()   # []
    ///     ```
    fn get_all_images(&self) -> PyResult<Vec<crate::python_bindings::types::PyInlineImage>> {
        let images = self.inner.get_all_images();
        Ok(images
            .iter()
            .map(crate::python_bindings::types::PyInlineImage::from)
            .collect())
    }

    /// Delete image by ID
    ///
    /// Args:
    ///     id: Image ID to delete
    ///
    /// Returns:
    ///     True if image was found and deleted
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.delete_image("logo")   # False (no image with that ID)
    ///     ```
    fn delete_image(&mut self, id: &str) -> PyResult<bool> {
        Ok(self.inner.delete_image(id))
    }

    /// Clear all inline images
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.clear_images()
    ///     term.get_all_images()   # []
    ///     ```
    fn clear_images(&mut self) -> PyResult<()> {
        self.inner.clear_images();
        Ok(())
    }

    /// Get image by ID
    ///
    /// Args:
    ///     id: Image ID to find
    ///
    /// Returns:
    ///     PyInlineImage if found, None otherwise
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_image_by_id("logo")   # None
    ///     ```
    fn get_image_by_id(
        &self,
        id: &str,
    ) -> PyResult<Option<crate::python_bindings::types::PyInlineImage>> {
        Ok(self
            .inner
            .get_image_by_id(id)
            .map(|img| crate::python_bindings::types::PyInlineImage::from(&img)))
    }

    /// Set maximum inline images
    ///
    /// Args:
    ///     max: Maximum number of images to keep; the oldest images are
    ///         dropped when the store exceeds it
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.set_max_inline_images(10)
    ///     ```
    fn set_max_inline_images(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_inline_images(max);
        Ok(())
    }
}
