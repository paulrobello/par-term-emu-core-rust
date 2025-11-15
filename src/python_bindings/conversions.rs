//! Python conversion utilities
//!
//! This module contains parsing utilities for terminal configuration options.
//! Note: Type conversion implementations (e.g., PtyError to PyErr) are defined in lib.rs
//! to avoid conflicting trait implementations.

use pyo3::prelude::*;

/// Parse sixel rendering mode from string
pub fn parse_sixel_mode(mode: &str) -> PyResult<crate::screenshot::SixelRenderMode> {
    match mode.to_lowercase().as_str() {
        "disabled" | "none" | "false" => Ok(crate::screenshot::SixelRenderMode::Disabled),
        "pixels" | "pixel" | "full" => Ok(crate::screenshot::SixelRenderMode::Pixels),
        "halfblocks" | "half-blocks" | "half_blocks" | "blocks" | "true" => {
            Ok(crate::screenshot::SixelRenderMode::HalfBlocks)
        }
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Invalid sixel_mode: '{}'. Valid options: 'disabled', 'pixels', 'halfblocks'",
            mode
        ))),
    }
}
