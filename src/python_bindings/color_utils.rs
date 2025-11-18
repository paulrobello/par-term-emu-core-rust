//! Python bindings for color utility functions
use crate::color::Color;
use pyo3::prelude::*;

/// Calculate perceived brightness of an RGB color using NTSC formula.
///
/// The NTSC formula weights color components based on human perception:
/// - Red: 30%
/// - Green: 59%
/// - Blue: 11%
///
/// Args:
///     r (int): Red component (0-255)
///     g (int): Green component (0-255)
///     b (int): Blue component (0-255)
///
/// Returns:
///     float: Perceived brightness value (0.0-1.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import perceived_brightness_rgb
///     >>> brightness = perceived_brightness_rgb(128, 128, 128)
///     >>> print(f"Gray brightness: {brightness:.2f}")
///     Gray brightness: 0.50
#[pyfunction]
#[pyo3(name = "perceived_brightness_rgb")]
pub fn py_perceived_brightness_rgb(r: u8, g: u8, b: u8) -> f64 {
    crate::color_utils::perceived_brightness_rgb(r, g, b)
}

/// Adjust foreground color to maintain minimum contrast against background.
///
/// Implements iTerm2's minimum contrast algorithm using NTSC perceived brightness.
/// The algorithm preserves the color's hue while adjusting brightness to ensure
/// readability.
///
/// Args:
///     fg (tuple): Foreground color (r, g, b) where each component is 0-255
///     bg (tuple): Background color (r, g, b) where each component is 0-255
///     minimum_contrast (float): Minimum required brightness difference (0.0-1.0)
///
/// Returns:
///     tuple: Adjusted foreground color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_contrast_rgb
///     >>> # Dark gray text on black background - will be lightened
///     >>> fg = (64, 64, 64)
///     >>> bg = (0, 0, 0)
///     >>> adjusted = adjust_contrast_rgb(fg, bg, 0.5)
///     >>> print(f"Adjusted color: {adjusted}")
///     Adjusted color: (128, 128, 128)
#[pyfunction]
#[pyo3(name = "adjust_contrast_rgb")]
pub fn py_adjust_contrast_rgb(
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    minimum_contrast: f64,
) -> (u8, u8, u8) {
    crate::color_utils::adjust_contrast_rgb(fg, bg, minimum_contrast)
}

/// Lighten an RGB color by a given amount.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///     amount (float): Amount to lighten (0.0 to 1.0)
///
/// Returns:
///     tuple: Lightened color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import lighten_rgb
///     >>> lightened = lighten_rgb((128, 64, 32), 0.5)
///     >>> print(f"Lightened: {lightened}")
#[pyfunction]
#[pyo3(name = "lighten_rgb")]
pub fn py_lighten_rgb(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.lighten(amount).to_rgb()
}

/// Darken an RGB color by a given amount.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///     amount (float): Amount to darken (0.0 to 1.0)
///
/// Returns:
///     tuple: Darkened color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import darken_rgb
///     >>> darkened = darken_rgb((200, 150, 100), 0.3)
///     >>> print(f"Darkened: {darkened}")
#[pyfunction]
#[pyo3(name = "darken_rgb")]
pub fn py_darken_rgb(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.darken(amount).to_rgb()
}

/// Calculate WCAG relative luminance of an RGB color.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     float: Relative luminance (0.0-1.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import color_luminance
///     >>> lum = color_luminance((255, 255, 255))
///     >>> print(f"White luminance: {lum:.2f}")
#[pyfunction]
#[pyo3(name = "color_luminance")]
pub fn py_color_luminance(rgb: (u8, u8, u8)) -> f32 {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.luminance()
}

/// Check if a color is dark (luminance < 0.5).
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     bool: True if color is dark
///
/// Example:
///     >>> from par_term_emu_core_rust import is_dark_color
///     >>> print(is_dark_color((50, 50, 50)))
///     True
#[pyfunction]
#[pyo3(name = "is_dark_color")]
pub fn py_is_dark_color(rgb: (u8, u8, u8)) -> bool {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.is_dark()
}

/// Calculate WCAG contrast ratio between two colors.
///
/// Args:
///     rgb1 (tuple): First color (r, g, b)
///     rgb2 (tuple): Second color (r, g, b)
///
/// Returns:
///     float: Contrast ratio (1.0 to 21.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import contrast_ratio
///     >>> ratio = contrast_ratio((0, 0, 0), (255, 255, 255))
///     >>> print(f"Black/White ratio: {ratio:.1f}:1")
///     Black/White ratio: 21.0:1
#[pyfunction]
#[pyo3(name = "contrast_ratio")]
pub fn py_contrast_ratio(rgb1: (u8, u8, u8), rgb2: (u8, u8, u8)) -> f32 {
    let color1 = Color::Rgb(rgb1.0, rgb1.1, rgb1.2);
    let color2 = Color::Rgb(rgb2.0, rgb2.1, rgb2.2);
    color1.contrast_ratio(&color2)
}

/// Check if two colors meet WCAG AA standard (4.5:1 for normal text).
///
/// Args:
///     fg (tuple): Foreground color (r, g, b)
///     bg (tuple): Background color (r, g, b)
///
/// Returns:
///     bool: True if colors meet WCAG AA standard
///
/// Example:
///     >>> from par_term_emu_core_rust import meets_wcag_aa
///     >>> print(meets_wcag_aa((0, 0, 0), (255, 255, 255)))
///     True
#[pyfunction]
#[pyo3(name = "meets_wcag_aa")]
pub fn py_meets_wcag_aa(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> bool {
    let fg_color = Color::Rgb(fg.0, fg.1, fg.2);
    let bg_color = Color::Rgb(bg.0, bg.1, bg.2);
    fg_color.meets_wcag_aa(&bg_color)
}

/// Check if two colors meet WCAG AAA standard (7:1 for normal text).
///
/// Args:
///     fg (tuple): Foreground color (r, g, b)
///     bg (tuple): Background color (r, g, b)
///
/// Returns:
///     bool: True if colors meet WCAG AAA standard
///
/// Example:
///     >>> from par_term_emu_core_rust import meets_wcag_aaa
///     >>> print(meets_wcag_aaa((0, 0, 0), (255, 255, 255)))
///     True
#[pyfunction]
#[pyo3(name = "meets_wcag_aaa")]
pub fn py_meets_wcag_aaa(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> bool {
    let fg_color = Color::Rgb(fg.0, fg.1, fg.2);
    let bg_color = Color::Rgb(bg.0, bg.1, bg.2);
    fg_color.meets_wcag_aaa(&bg_color)
}

/// Mix two colors with a given ratio.
///
/// Args:
///     rgb1 (tuple): First color (r, g, b)
///     rgb2 (tuple): Second color (r, g, b)
///     ratio (float): Mix ratio (0.0 = all rgb1, 1.0 = all rgb2)
///
/// Returns:
///     tuple: Mixed color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import mix_colors
///     >>> mixed = mix_colors((255, 0, 0), (0, 0, 255), 0.5)
///     >>> print(f"Purple: {mixed}")
#[pyfunction]
#[pyo3(name = "mix_colors")]
pub fn py_mix_colors(rgb1: (u8, u8, u8), rgb2: (u8, u8, u8), ratio: f32) -> (u8, u8, u8) {
    let color1 = Color::Rgb(rgb1.0, rgb1.1, rgb1.2);
    let color2 = Color::Rgb(rgb2.0, rgb2.1, rgb2.2);
    color1.mix(&color2, ratio).to_rgb()
}

/// Convert RGB to HSL color space.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     tuple: (hue, saturation, lightness) where:
///         - hue is in degrees (0-360)
///         - saturation is percentage (0-100)
///         - lightness is percentage (0-100)
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_hsl
///     >>> h, s, l = rgb_to_hsl((255, 0, 0))
///     >>> print(f"Red in HSL: H={h:.0f}° S={s:.0f}% L={l:.0f}%")
///     Red in HSL: H=0° S=100% L=50%
#[pyfunction]
#[pyo3(name = "rgb_to_hsl")]
pub fn py_rgb_to_hsl(rgb: (u8, u8, u8)) -> (f32, f32, f32) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_hsl()
}

/// Convert HSL to RGB color space.
///
/// Args:
///     h (float): Hue in degrees (0-360)
///     s (float): Saturation percentage (0-100)
///     l (float): Lightness percentage (0-100)
///
/// Returns:
///     tuple: RGB color (r, g, b) where each component is 0-255
///
/// Example:
///     >>> from par_term_emu_core_rust import hsl_to_rgb
///     >>> rgb = hsl_to_rgb(120, 100, 50)  # Pure green
///     >>> print(f"Green RGB: {rgb}")
///     Green RGB: (0, 255, 0)
#[pyfunction]
#[pyo3(name = "hsl_to_rgb")]
pub fn py_hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    Color::from_hsl(h, s, l).to_rgb()
}

/// Adjust color saturation.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///     amount (float): Amount to adjust saturation (-100 to 100)
///
/// Returns:
///     tuple: Adjusted color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_saturation
///     >>> saturated = adjust_saturation((200, 100, 100), 50)
///     >>> desaturated = adjust_saturation((200, 100, 100), -50)
#[pyfunction]
#[pyo3(name = "adjust_saturation")]
pub fn py_adjust_saturation(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.adjust_saturation(amount).to_rgb()
}

/// Adjust color hue.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///     degrees (float): Degrees to rotate hue (wraps around 360)
///
/// Returns:
///     tuple: Adjusted color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_hue
///     >>> shifted = adjust_hue((255, 0, 0), 120)  # Red -> Green
#[pyfunction]
#[pyo3(name = "adjust_hue")]
pub fn py_adjust_hue(rgb: (u8, u8, u8), degrees: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.adjust_hue(degrees).to_rgb()
}

/// Get complementary color (opposite on color wheel).
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     tuple: Complementary color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import complementary_color
///     >>> comp = complementary_color((255, 0, 0))  # Red -> Cyan
///     >>> print(f"Complement of red: {comp}")
#[pyfunction]
#[pyo3(name = "complementary_color")]
pub fn py_complementary_color(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.complementary().to_rgb()
}

/// Convert RGB to hex string.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     str: Hex color string (e.g., "#FF0000")
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_hex
///     >>> hex_str = rgb_to_hex((255, 128, 64))
///     >>> print(hex_str)
///     #FF8040
#[pyfunction]
#[pyo3(name = "rgb_to_hex")]
pub fn py_rgb_to_hex(rgb: (u8, u8, u8)) -> String {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_hex()
}

/// Convert hex string to RGB.
///
/// Args:
///     hex_str (str): Hex color string (e.g., "#FF0000" or "FF0000")
///
/// Returns:
///     tuple or None: RGB color (r, g, b) or None if invalid
///
/// Example:
///     >>> from par_term_emu_core_rust import hex_to_rgb
///     >>> rgb = hex_to_rgb("#FF8040")
///     >>> print(f"RGB: {rgb}")
///     RGB: (255, 128, 64)
#[pyfunction]
#[pyo3(name = "hex_to_rgb")]
pub fn py_hex_to_rgb(hex_str: &str) -> Option<(u8, u8, u8)> {
    Color::from_hex(hex_str).map(|c| c.to_rgb())
}

/// Convert RGB to nearest 256-color ANSI palette index.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     int: ANSI 256-color index (16-255)
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_ansi_256
///     >>> idx = rgb_to_ansi_256((255, 0, 0))
///     >>> print(f"Red is closest to ANSI color {idx}")
#[pyfunction]
#[pyo3(name = "rgb_to_ansi_256")]
pub fn py_rgb_to_ansi_256(rgb: (u8, u8, u8)) -> u8 {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_ansi_256()
}
