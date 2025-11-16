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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sixel_mode_disabled() {
        assert_eq!(
            parse_sixel_mode("disabled").unwrap(),
            crate::screenshot::SixelRenderMode::Disabled
        );
        assert_eq!(
            parse_sixel_mode("none").unwrap(),
            crate::screenshot::SixelRenderMode::Disabled
        );
        assert_eq!(
            parse_sixel_mode("false").unwrap(),
            crate::screenshot::SixelRenderMode::Disabled
        );
    }

    #[test]
    fn test_parse_sixel_mode_pixels() {
        assert_eq!(
            parse_sixel_mode("pixels").unwrap(),
            crate::screenshot::SixelRenderMode::Pixels
        );
        assert_eq!(
            parse_sixel_mode("pixel").unwrap(),
            crate::screenshot::SixelRenderMode::Pixels
        );
        assert_eq!(
            parse_sixel_mode("full").unwrap(),
            crate::screenshot::SixelRenderMode::Pixels
        );
    }

    #[test]
    fn test_parse_sixel_mode_halfblocks() {
        assert_eq!(
            parse_sixel_mode("halfblocks").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
        assert_eq!(
            parse_sixel_mode("half-blocks").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
        assert_eq!(
            parse_sixel_mode("half_blocks").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
        assert_eq!(
            parse_sixel_mode("blocks").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
        assert_eq!(
            parse_sixel_mode("true").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
    }

    #[test]
    fn test_parse_sixel_mode_case_insensitive() {
        assert_eq!(
            parse_sixel_mode("DISABLED").unwrap(),
            crate::screenshot::SixelRenderMode::Disabled
        );
        assert_eq!(
            parse_sixel_mode("Pixels").unwrap(),
            crate::screenshot::SixelRenderMode::Pixels
        );
        assert_eq!(
            parse_sixel_mode("HalfBlocks").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
        assert_eq!(
            parse_sixel_mode("HALF-BLOCKS").unwrap(),
            crate::screenshot::SixelRenderMode::HalfBlocks
        );
    }

    #[test]
    fn test_parse_sixel_mode_mixed_case() {
        assert_eq!(
            parse_sixel_mode("DiSaBlEd").unwrap(),
            crate::screenshot::SixelRenderMode::Disabled
        );
        assert_eq!(
            parse_sixel_mode("pIxElS").unwrap(),
            crate::screenshot::SixelRenderMode::Pixels
        );
    }

    #[test]
    fn test_parse_sixel_mode_invalid() {
        let result = parse_sixel_mode("invalid");
        assert!(result.is_err());

        let result = parse_sixel_mode("unknown");
        assert!(result.is_err());

        let result = parse_sixel_mode("");
        assert!(result.is_err());

        let result = parse_sixel_mode("half blocks");
        assert!(result.is_err()); // Space instead of hyphen/underscore
    }

    #[test]
    fn test_parse_sixel_mode_error_message() {
        let result = parse_sixel_mode("badvalue");
        assert!(result.is_err());
        if let Err(e) = result {
            let err_msg = e.to_string();
            assert!(err_msg.contains("Invalid sixel_mode"));
            assert!(err_msg.contains("badvalue"));
            assert!(err_msg.contains("disabled"));
            assert!(err_msg.contains("pixels"));
            assert!(err_msg.contains("halfblocks"));
        }
    }

    #[test]
    fn test_parse_sixel_mode_all_valid_variants() {
        // Test all documented variants
        let disabled_variants = vec!["disabled", "none", "false"];
        for variant in disabled_variants {
            assert!(
                parse_sixel_mode(variant).is_ok(),
                "Failed for disabled variant: {}",
                variant
            );
        }

        let pixels_variants = vec!["pixels", "pixel", "full"];
        for variant in pixels_variants {
            assert!(
                parse_sixel_mode(variant).is_ok(),
                "Failed for pixels variant: {}",
                variant
            );
        }

        let halfblocks_variants =
            vec!["halfblocks", "half-blocks", "half_blocks", "blocks", "true"];
        for variant in halfblocks_variants {
            assert!(
                parse_sixel_mode(variant).is_ok(),
                "Failed for halfblocks variant: {}",
                variant
            );
        }
    }
}
