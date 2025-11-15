use image::{codecs::jpeg::JpegEncoder, ImageEncoder, RgbImage, RgbaImage};

use crate::screenshot::error::ScreenshotResult;

/// Encode image as JPEG bytes
///
/// Note: JPEG doesn't support transparency, so the alpha channel is discarded
pub fn encode(image: &RgbaImage, quality: u8) -> ScreenshotResult<Vec<u8>> {
    // Convert RGBA to RGB (JPEG doesn't support alpha)
    let rgb_image = RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        image::Rgb([pixel[0], pixel[1], pixel[2]])
    });

    let mut buf = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    encoder.write_image(
        rgb_image.as_raw(),
        rgb_image.width(),
        rgb_image.height(),
        image::ExtendedColorType::Rgb8,
    )?;

    Ok(buf)
}
