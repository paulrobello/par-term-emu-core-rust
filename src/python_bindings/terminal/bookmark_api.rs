//! Bookmark API methods for `PyTerminal` (ARC-002: split out of the monolithic
//! `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API or
//! behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Bookmark Methods ===

    /// Add a bookmark at the given scrollback row
    ///
    /// Args:
    ///     row: Row index (negative for scrollback, 0+ for visible screen)
    ///     label: Optional label for the bookmark
    ///
    /// Returns:
    ///     Bookmark ID
    #[pyo3(signature = (row, label=None))]
    fn add_bookmark(&mut self, row: isize, label: Option<String>) -> PyResult<usize> {
        Ok(self.inner.add_bookmark(row, label))
    }

    /// Get all bookmarks
    ///
    /// Returns:
    ///     List of Bookmark objects
    fn get_bookmarks(&self) -> PyResult<Vec<crate::python_bindings::types::PyBookmark>> {
        let bookmarks = self.inner.get_bookmarks();
        Ok(bookmarks
            .iter()
            .map(|b| crate::python_bindings::types::PyBookmark {
                id: b.id,
                row: b.row,
                label: b.label.clone(),
            })
            .collect())
    }

    /// Remove a bookmark by ID
    ///
    /// Args:
    ///     id: Bookmark ID
    ///
    /// Returns:
    ///     True if bookmark was removed, False if not found
    fn remove_bookmark(&mut self, id: usize) -> PyResult<bool> {
        Ok(self.inner.remove_bookmark(id))
    }

    /// Clear all bookmarks
    fn clear_bookmarks(&mut self) -> PyResult<()> {
        self.inner.clear_bookmarks();
        Ok(())
    }
}
