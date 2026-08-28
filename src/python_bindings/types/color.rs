//! Color-space (HSV / HSL / palette) types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// HSV color
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ColorHSV", from_py_object)]
#[derive(Clone)]
pub struct PyColorHSV {
    /// Hue in degrees (0.0-360.0)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Value/brightness (0.0-1.0)
    pub v: f32,
}

#[pymethods]
impl PyColorHSV {
    #[new]
    fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h, s, v }
    }

    fn __repr__(&self) -> String {
        format!(
            "ColorHSV(h={:.1}, s={:.2}, v={:.2})",
            self.h, self.s, self.v
        )
    }
}

/// HSL color
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ColorHSL", from_py_object)]
#[derive(Clone)]
pub struct PyColorHSL {
    /// Hue in degrees (0.0-360.0)
    pub h: f32,
    /// Saturation (0.0-1.0)
    pub s: f32,
    /// Lightness (0.0-1.0)
    pub l: f32,
}

#[pymethods]
impl PyColorHSL {
    #[new]
    fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    fn __repr__(&self) -> String {
        format!(
            "ColorHSL(h={:.1}, s={:.2}, l={:.2})",
            self.h, self.s, self.l
        )
    }
}

/// Color palette
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ColorPalette", from_py_object)]
#[derive(Clone)]
pub struct PyColorPalette {
    /// Base color the palette was generated from (r, g, b)
    pub base: (u8, u8, u8),
    /// Generated palette colors as (r, g, b) tuples
    pub colors: Vec<(u8, u8, u8)>,
    /// Palette generation mode
    pub mode: String,
}

#[pymethods]
impl PyColorPalette {
    fn __repr__(&self) -> String {
        format!(
            "ColorPalette(mode={}, colors={})",
            self.mode,
            self.colors.len()
        )
    }
}
