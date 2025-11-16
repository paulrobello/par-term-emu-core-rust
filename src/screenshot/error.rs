use std::fmt;

/// Error types for screenshot operations
#[derive(Debug)]
pub enum ScreenshotError {
    /// Font loading failed
    FontLoadError(String),
    /// Rendering failed
    RenderError(String),
    /// Format encoding failed
    FormatError(String),
    /// I/O error
    IoError(std::io::Error),
    /// Invalid configuration
    InvalidConfig(String),
}

impl fmt::Display for ScreenshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FontLoadError(msg) => write!(f, "Font load error: {}", msg),
            Self::RenderError(msg) => write!(f, "Render error: {}", msg),
            Self::FormatError(msg) => write!(f, "Format error: {}", msg),
            Self::IoError(err) => write!(f, "I/O error: {}", err),
            Self::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
        }
    }
}

impl std::error::Error for ScreenshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ScreenshotError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

impl From<image::ImageError> for ScreenshotError {
    fn from(err: image::ImageError) -> Self {
        Self::FormatError(format!("Image encoding error: {}", err))
    }
}

/// Result type for screenshot operations
pub type ScreenshotResult<T> = Result<T, ScreenshotError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn test_font_load_error_display() {
        let err = ScreenshotError::FontLoadError("Failed to load font.ttf".to_string());
        assert_eq!(err.to_string(), "Font load error: Failed to load font.ttf");
    }

    #[test]
    fn test_render_error_display() {
        let err = ScreenshotError::RenderError("Invalid glyph".to_string());
        assert_eq!(err.to_string(), "Render error: Invalid glyph");
    }

    #[test]
    fn test_format_error_display() {
        let err = ScreenshotError::FormatError("PNG encoding failed".to_string());
        assert_eq!(err.to_string(), "Format error: PNG encoding failed");
    }

    #[test]
    fn test_io_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = ScreenshotError::IoError(io_err);
        assert!(err.to_string().contains("I/O error"));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_invalid_config_display() {
        let err = ScreenshotError::InvalidConfig("Quality must be 1-100".to_string());
        assert_eq!(err.to_string(), "Invalid config: Quality must be 1-100");
    }

    #[test]
    fn test_error_source_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = ScreenshotError::IoError(io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn test_error_source_other_errors() {
        let err = ScreenshotError::FontLoadError("test".to_string());
        assert!(err.source().is_none());

        let err = ScreenshotError::RenderError("test".to_string());
        assert!(err.source().is_none());

        let err = ScreenshotError::FormatError("test".to_string());
        assert!(err.source().is_none());

        let err = ScreenshotError::InvalidConfig("test".to_string());
        assert!(err.source().is_none());
    }

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "test");
        let err: ScreenshotError = io_err.into();
        match err {
            ScreenshotError::IoError(_) => {}
            _ => panic!("Expected IoError variant"),
        }
    }

    #[test]
    fn test_from_image_error() {
        use image::ImageError;
        let img_err =
            ImageError::Unsupported(image::error::UnsupportedError::from_format_and_kind(
                image::error::ImageFormatHint::Unknown,
                image::error::UnsupportedErrorKind::Format(image::error::ImageFormatHint::Unknown),
            ));
        let err: ScreenshotError = img_err.into();
        match err {
            ScreenshotError::FormatError(msg) => {
                assert!(msg.contains("Image encoding error"));
            }
            _ => panic!("Expected FormatError variant"),
        }
    }

    #[test]
    fn test_error_debug_format() {
        let err = ScreenshotError::RenderError("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("RenderError"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_all_error_variants_create() {
        let _ = ScreenshotError::FontLoadError("test".to_string());
        let _ = ScreenshotError::RenderError("test".to_string());
        let _ = ScreenshotError::FormatError("test".to_string());
        let _ = ScreenshotError::InvalidConfig("test".to_string());
        let io_err = std::io::Error::other("test");
        let _ = ScreenshotError::IoError(io_err);
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_screenshot_result_type() {
        let ok_result: ScreenshotResult<i32> = Ok(42);
        assert_eq!(ok_result.unwrap(), 42);

        let err_result: ScreenshotResult<i32> =
            Err(ScreenshotError::RenderError("failed".to_string()));
        assert!(err_result.is_err());
    }
}
