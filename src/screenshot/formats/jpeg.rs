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

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_encode_empty_image() {
        let image = RgbaImage::new(1, 1);
        let result = encode(&image, 90);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        // JPEG SOI marker
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn test_encode_colored_image() {
        let mut image = RgbaImage::new(4, 4);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255])); // Red
        image.put_pixel(1, 0, Rgba([0, 255, 0, 255])); // Green
        image.put_pixel(0, 1, Rgba([0, 0, 255, 255])); // Blue

        let result = encode(&image, 85);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn test_encode_discards_alpha_channel() {
        let mut image = RgbaImage::new(2, 2);
        // Alpha values should be discarded
        image.put_pixel(0, 0, Rgba([255, 0, 0, 128])); // Semi-transparent
        image.put_pixel(1, 0, Rgba([0, 255, 0, 0])); // Fully transparent

        let result = encode(&image, 90);
        assert!(result.is_ok());
        // Should succeed even though alpha is discarded
    }

    #[test]
    fn test_encode_different_quality_levels() {
        let mut image = RgbaImage::new(10, 10);
        for x in 0..10 {
            for y in 0..10 {
                image.put_pixel(x, y, Rgba([x as u8 * 25, y as u8 * 25, 128, 255]));
            }
        }

        // Test various quality levels
        let qualities = vec![1, 25, 50, 75, 90, 100];
        let mut sizes = Vec::new();

        for quality in qualities {
            let result = encode(&image, quality);
            assert!(result.is_ok(), "Failed for quality {}", quality);
            sizes.push((quality, result.unwrap().len()));
        }

        // Higher quality should generally produce larger files
        // (though not guaranteed for all images)
        assert!(sizes[0].1 <= sizes[sizes.len() - 1].1 * 2);
    }

    #[test]
    fn test_encode_quality_clamping() {
        let image = RgbaImage::new(4, 4);

        // Quality values should be accepted
        let result = encode(&image, 1);
        assert!(result.is_ok());

        let result = encode(&image, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_various_sizes() {
        let sizes = vec![(1, 1), (10, 10), (100, 50)];

        for (width, height) in sizes {
            let image = RgbaImage::new(width, height);
            let result = encode(&image, 90);
            assert!(result.is_ok(), "Failed for size {}x{}", width, height);
        }
    }

    #[test]
    fn test_encode_output_can_be_decoded() {
        let mut image = RgbaImage::new(8, 8);
        for x in 0..8 {
            for y in 0..8 {
                image.put_pixel(x, y, Rgba([x as u8 * 32, y as u8 * 32, 128, 255]));
            }
        }

        let encoded = encode(&image, 95).unwrap();
        let decoded = image::load_from_memory(&encoded).unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 8);
    }
}
