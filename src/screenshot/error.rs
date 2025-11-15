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
