use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

use crate::screenshot::error::ScreenshotResult;

/// Encode image as PNG bytes
pub fn encode(image: &RgbaImage) -> ScreenshotResult<Vec<u8>> {
    let mut buf = Vec::new();
    image.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)?;
    Ok(buf)
}
