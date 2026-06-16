//! Advanced color operations and rendering-hint API methods for `PyTerminal`
//! (ARC-002: split out of the monolithic `#[pymethods]` block in `mod.rs`). Pure
//! relocation — no Python API or behavior change; these methods remain on the same
//! `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 8: Advanced Color Operations ===

    /// Convert RGB to HSV
    fn rgb_to_hsv_color(
        &self,
        r: u8,
        g: u8,
        b: u8,
    ) -> PyResult<crate::python_bindings::types::PyColorHSV> {
        let hsv = self.inner.rgb_to_hsv_color(r, g, b);
        Ok(crate::python_bindings::types::PyColorHSV {
            h: hsv.h,
            s: hsv.s,
            v: hsv.v,
        })
    }

    /// Convert HSV to RGB
    fn hsv_to_rgb_color(&self, h: f32, s: f32, v: f32) -> PyResult<(u8, u8, u8)> {
        let hsv = crate::terminal::ColorHSV { h, s, v };
        Ok(self.inner.hsv_to_rgb_color(hsv))
    }

    /// Convert RGB to HSL
    fn rgb_to_hsl_color(
        &self,
        r: u8,
        g: u8,
        b: u8,
    ) -> PyResult<crate::python_bindings::types::PyColorHSL> {
        let hsl = self.inner.rgb_to_hsl_color(r, g, b);
        Ok(crate::python_bindings::types::PyColorHSL {
            h: hsl.h,
            s: hsl.s,
            l: hsl.l,
        })
    }

    /// Convert HSL to RGB
    fn hsl_to_rgb_color(&self, h: f32, s: f32, l: f32) -> PyResult<(u8, u8, u8)> {
        let hsl = crate::terminal::ColorHSL { h, s, l };
        Ok(self.inner.hsl_to_rgb_color(hsl))
    }

    /// Generate a color palette
    ///
    /// Args:
    ///     r, g, b: Base color RGB values
    ///     mode: Theme mode (complementary, analogous, triadic, tetradic, split_complementary, monochromatic)
    fn generate_color_palette(
        &self,
        r: u8,
        g: u8,
        b: u8,
        mode: &str,
    ) -> PyResult<crate::python_bindings::types::PyColorPalette> {
        use crate::terminal::ThemeMode;
        let theme_mode = match mode {
            "complementary" => ThemeMode::Complementary,
            "analogous" => ThemeMode::Analogous,
            "triadic" => ThemeMode::Triadic,
            "tetradic" => ThemeMode::Tetradic,
            "split_complementary" => ThemeMode::SplitComplementary,
            "monochromatic" => ThemeMode::Monochromatic,
            _ => return Err(PyValueError::new_err("Invalid theme mode")),
        };

        let palette = self.inner.generate_color_palette(r, g, b, theme_mode);
        Ok(crate::python_bindings::types::PyColorPalette {
            base: palette.base,
            colors: palette.colors,
            mode: mode.to_string(),
        })
    }

    /// Calculate color distance
    fn color_distance(&self, r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> PyResult<f64> {
        Ok(self.inner.color_distance(r1, g1, b1, r2, g2, b2) as f64)
    }

    // === Feature 19: Custom Rendering Hints ===

    /// Add a damage region
    fn add_damage_region(
        &mut self,
        left: usize,
        top: usize,
        right: usize,
        bottom: usize,
    ) -> PyResult<()> {
        self.inner.add_damage_region(left, top, right, bottom);
        Ok(())
    }

    /// Get damage regions
    fn get_damage_regions(&self) -> PyResult<Vec<crate::python_bindings::types::PyDamageRegion>> {
        let regions = self.inner.get_damage_regions();
        Ok(regions
            .iter()
            .map(crate::python_bindings::types::PyDamageRegion::from)
            .collect())
    }

    /// Merge overlapping damage regions
    fn merge_damage_regions(&mut self) -> PyResult<()> {
        self.inner.merge_damage_regions();
        Ok(())
    }

    /// Clear damage regions
    fn clear_damage_regions(&mut self) -> PyResult<()> {
        self.inner.clear_damage_regions();
        Ok(())
    }

    /// Add a rendering hint
    #[allow(clippy::too_many_arguments)]
    fn add_rendering_hint(
        &mut self,
        left: usize,
        top: usize,
        right: usize,
        bottom: usize,
        layer: &str,
        animation: &str,
        priority: &str,
    ) -> PyResult<()> {
        use crate::terminal::{AnimationHint, DamageRegion, UpdatePriority, ZLayer};

        let damage = DamageRegion {
            left,
            top,
            right,
            bottom,
        };

        let layer = match layer.to_lowercase().as_str() {
            "background" => ZLayer::Background,
            "normal" => ZLayer::Normal,
            "overlay" => ZLayer::Overlay,
            "cursor" => ZLayer::Cursor,
            _ => return Err(PyValueError::new_err("Invalid layer")),
        };

        let animation = match animation.to_lowercase().as_str() {
            "none" => AnimationHint::None,
            "smoothscroll" => AnimationHint::SmoothScroll,
            "fade" => AnimationHint::Fade,
            "cursorblink" => AnimationHint::CursorBlink,
            _ => return Err(PyValueError::new_err("Invalid animation hint")),
        };

        let priority = match priority.to_lowercase().as_str() {
            "low" => UpdatePriority::Low,
            "normal" => UpdatePriority::Normal,
            "high" => UpdatePriority::High,
            "critical" => UpdatePriority::Critical,
            _ => return Err(PyValueError::new_err("Invalid priority")),
        };

        use crate::terminal::RenderingHint;
        self.inner.add_rendering_hint(RenderingHint {
            damage,
            layer,
            animation,
            priority,
        });
        Ok(())
    }

    /// Get rendering hints
    #[pyo3(signature = (sort_by_priority=false))]
    fn get_rendering_hints(
        &self,
        sort_by_priority: bool,
    ) -> PyResult<Vec<crate::python_bindings::types::PyRenderingHint>> {
        let hints = self.inner.get_rendering_hints(sort_by_priority);
        Ok(hints
            .iter()
            .map(crate::python_bindings::types::PyRenderingHint::from)
            .collect())
    }

    /// Clear rendering hints
    fn clear_rendering_hints(&mut self) -> PyResult<()> {
        self.inner.clear_rendering_hints();
        Ok(())
    }
}
