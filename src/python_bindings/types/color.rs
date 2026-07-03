//! Color-space (HSV / HSL / palette) types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// HSV color
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ColorHSV", from_py_object)]
#[derive(Clone)]
pub struct PyColorHSV {
    pub h: f32,
    pub s: f32,
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
    pub h: f32,
    pub s: f32,
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
    pub base: (u8, u8, u8),
    pub colors: Vec<(u8, u8, u8)>,
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
