use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

use crate::screenshot::error::ScreenshotResult;

/// Encode image as BMP bytes
pub fn encode(image: &RgbaImage) -> ScreenshotResult<Vec<u8>> {
    let mut buf = Vec::new();
    image.write_to(&mut Cursor::new(&mut buf), ImageFormat::Bmp)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_encode_empty_image() {
        let image = RgbaImage::new(1, 1);
        let result = encode(&image);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        // BMP signature "BM"
        assert_eq!(&bytes[0..2], b"BM");
    }

    #[test]
    fn test_encode_colored_image() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255])); // Red
        image.put_pixel(1, 0, Rgba([0, 255, 0, 255])); // Green
        image.put_pixel(0, 1, Rgba([0, 0, 255, 255])); // Blue
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255])); // White

        let result = encode(&image);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[0..2], b"BM");
    }

    #[test]
    fn test_encode_with_transparency() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 128])); // Semi-transparent red
        image.put_pixel(1, 0, Rgba([0, 255, 0, 0])); // Fully transparent

        let result = encode(&image);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_encode_various_sizes() {
        let sizes = vec![(1, 1), (10, 10), (100, 50), (256, 256)];

        for (width, height) in sizes {
            let image = RgbaImage::new(width, height);
            let result = encode(&image);
            assert!(result.is_ok(), "Failed for size {}x{}", width, height);
            let bytes = result.unwrap();
            assert_eq!(&bytes[0..2], b"BM");
        }
    }

    #[test]
    fn test_encode_output_can_be_decoded() {
        let mut image = RgbaImage::new(4, 4);
        for x in 0..4 {
            for y in 0..4 {
                image.put_pixel(x, y, Rgba([x as u8 * 64, y as u8 * 64, 128, 255]));
            }
        }

        let encoded = encode(&image).unwrap();
        let decoded = image::load_from_memory(&encoded).unwrap();
        assert_eq!(decoded.width(), 4);
        assert_eq!(decoded.height(), 4);
    }

    #[test]
    fn test_bmp_is_larger_than_compressed() {
        // BMP is uncompressed, so should be larger than PNG for same image
        let mut image = RgbaImage::new(10, 10);
        for x in 0..10 {
            for y in 0..10 {
                image.put_pixel(x, y, Rgba([128, 128, 128, 255]));
            }
        }

        let bmp_bytes = encode(&image).unwrap();
        let png_bytes = crate::screenshot::formats::png::encode(&image).unwrap();

        // BMP should be larger (uncompressed)
        assert!(bmp_bytes.len() > png_bytes.len());
    }
}
