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

    /// Convert an RGB color to HSV (hue/saturation/value)
    ///
    /// Args:
    ///     r: Red channel (0-255)
    ///     g: Green channel (0-255)
    ///     b: Blue channel (0-255)
    ///
    /// Returns:
    ///     ColorHSV: hue in degrees (0.0-360.0), saturation and value in 0.0-1.0
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     hsv = term.rgb_to_hsv_color(255, 0, 0)
    ///     print(hsv.h, hsv.s, hsv.v)
    ///     ```
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

    /// Convert an HSV color to RGB
    ///
    /// Args:
    ///     h: Hue in degrees (0.0-360.0)
    ///     s: Saturation (0.0-1.0)
    ///     v: Value/brightness (0.0-1.0)
    ///
    /// Returns:
    ///     tuple[int, int, int]: (r, g, b) with each channel in 0-255
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     r, g, b = term.hsv_to_rgb_color(0.0, 1.0, 1.0)
    ///     ```
    fn hsv_to_rgb_color(&self, h: f32, s: f32, v: f32) -> PyResult<(u8, u8, u8)> {
        let hsv = crate::terminal::ColorHSV { h, s, v };
        Ok(self.inner.hsv_to_rgb_color(hsv))
    }

    /// Convert an RGB color to HSL (hue/saturation/lightness)
    ///
    /// Args:
    ///     r: Red channel (0-255)
    ///     g: Green channel (0-255)
    ///     b: Blue channel (0-255)
    ///
    /// Returns:
    ///     ColorHSL: hue in degrees (0.0-360.0), saturation and lightness in 0.0-1.0
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     hsl = term.rgb_to_hsl_color(255, 0, 0)
    ///     print(hsl.h, hsl.s, hsl.l)
    ///     ```
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

    /// Convert an HSL color to RGB
    ///
    /// Args:
    ///     h: Hue in degrees (0.0-360.0)
    ///     s: Saturation (0.0-1.0)
    ///     l: Lightness (0.0-1.0)
    ///
    /// Returns:
    ///     tuple[int, int, int]: (r, g, b) with each channel in 0-255
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     r, g, b = term.hsl_to_rgb_color(0.0, 1.0, 0.5)
    ///     ```
    fn hsl_to_rgb_color(&self, h: f32, s: f32, l: f32) -> PyResult<(u8, u8, u8)> {
        let hsl = crate::terminal::ColorHSL { h, s, l };
        Ok(self.inner.hsl_to_rgb_color(hsl))
    }

    /// Generate a themed color palette from a base RGB color
    ///
    /// Args:
    ///     r: Base color red channel (0-255)
    ///     g: Base color green channel (0-255)
    ///     b: Base color blue channel (0-255)
    ///     mode: Theme mode, one of "complementary", "analogous", "triadic",
    ///         "tetradic", "split_complementary", "monochromatic"
    ///
    /// Returns:
    ///     ColorPalette: object with `base` (r, g, b), `colors` (list of
    ///     (r, g, b) tuples generated from the theme), and `mode` (echoed back)
    ///
    /// Raises:
    ///     ValueError: If `mode` is not one of the supported theme modes
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     palette = term.generate_color_palette(255, 0, 0, "complementary")
    ///     print(palette.colors)
    ///     ```
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

    /// Calculate the Euclidean distance between two RGB colors
    ///
    /// Args:
    ///     r1: First color red channel (0-255)
    ///     g1: First color green channel (0-255)
    ///     b1: First color blue channel (0-255)
    ///     r2: Second color red channel (0-255)
    ///     g2: Second color green channel (0-255)
    ///     b2: Second color blue channel (0-255)
    ///
    /// Returns:
    ///     float: Euclidean distance in RGB space (0.0 = identical colors,
    ///     larger values = more different); the maximum possible value is
    ///     approximately 441.7 (black vs. white)
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     dist = term.color_distance(255, 0, 0, 0, 255, 0)
    ///     ```
    fn color_distance(&self, r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> PyResult<f64> {
        Ok(self.inner.color_distance(r1, g1, b1, r2, g2, b2) as f64)
    }

    // === Feature 19: Custom Rendering Hints ===

    /// Add a damage region marking a rectangular area of the grid as dirty
    ///
    /// Accumulated damage regions can be retrieved via `get_damage_regions()`
    /// and are intended for frontends that want to redraw only changed areas.
    ///
    /// Args:
    ///     left: Left column of the damaged rectangle (0-indexed, inclusive)
    ///     top: Top row of the damaged rectangle (0-indexed, inclusive)
    ///     right: Right column of the damaged rectangle (exclusive)
    ///     bottom: Bottom row of the damaged rectangle (exclusive)
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_damage_region(0, 0, 80, 1)
    ///     ```
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

    /// Get all accumulated damage regions without clearing them
    ///
    /// Returns:
    ///     list[DamageRegion]: Regions added since the last `clear_damage_regions()`
    ///     call, each with `left`, `top`, `right`, `bottom` attributes
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_damage_region(0, 0, 80, 1)
    ///     for region in term.get_damage_regions():
    ///         print(region.left, region.top, region.right, region.bottom)
    ///     ```
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

    /// Add a rendering hint describing how a damaged region should be redrawn
    ///
    /// Rendering hints let a frontend apply frame-level optimizations (e.g.
    /// z-ordering, animation, and update priority) instead of blindly
    /// redrawing every damaged cell. Hints accumulate until drained via
    /// `get_rendering_hints()` / `clear_rendering_hints()`.
    ///
    /// Args:
    ///     left: Left column of the damaged rectangle (0-indexed, inclusive)
    ///     top: Top row of the damaged rectangle (0-indexed, inclusive)
    ///     right: Right column of the damaged rectangle (exclusive)
    ///     bottom: Bottom row of the damaged rectangle (exclusive)
    ///     layer: Z-order layer, one of "background", "normal", "overlay", "cursor"
    ///         (case-insensitive)
    ///     animation: Animation hint, one of "none", "smoothscroll", "fade",
    ///         "cursorblink" (case-insensitive)
    ///     priority: Update priority, one of "low", "normal", "high", "critical"
    ///         (case-insensitive)
    ///
    /// Raises:
    ///     ValueError: If `layer`, `animation`, or `priority` is not one of the
    ///         supported values above
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_rendering_hint(0, 0, 80, 1, "overlay", "fade", "high")
    ///     ```
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

    /// Get all pending rendering hints without clearing them
    ///
    /// Args:
    ///     sort_by_priority: If True, sort hints highest priority first
    ///         (default: False, insertion order)
    ///
    /// Returns:
    ///     list[RenderingHint]: Each hint has `damage` (DamageRegion), `layer`
    ///     (str: "background"/"normal"/"overlay"/"cursor"), `animation` (str:
    ///     "none"/"smoothscroll"/"fade"/"cursorblink"), and `priority` (int:
    ///     0=low, 1=normal, 2=high, 3=critical)
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.add_rendering_hint(0, 0, 80, 1, "overlay", "fade", "high")
    ///     for hint in term.get_rendering_hints(sort_by_priority=True):
    ///         print(hint.layer, hint.priority)
    ///     ```
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
