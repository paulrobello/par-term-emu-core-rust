use super::*;

#[test]
fn test_kitty_action_from_char() {
    assert_eq!(KittyAction::from_char('t'), Some(KittyAction::Transmit));
    assert_eq!(
        KittyAction::from_char('T'),
        Some(KittyAction::TransmitDisplay)
    );
    assert_eq!(KittyAction::from_char('q'), Some(KittyAction::Query));
    assert_eq!(KittyAction::from_char('p'), Some(KittyAction::Put));
    assert_eq!(KittyAction::from_char('d'), Some(KittyAction::Delete));
    assert_eq!(KittyAction::from_char('x'), None);
}

#[test]
fn test_kitty_format_from_code() {
    assert_eq!(KittyFormat::from_code(24), Some(KittyFormat::Rgb));
    assert_eq!(KittyFormat::from_code(32), Some(KittyFormat::Rgba));
    assert_eq!(KittyFormat::from_code(100), Some(KittyFormat::Png));
    assert_eq!(KittyFormat::from_code(0), None);
}

#[test]
fn test_kitty_parser_basic() {
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T,f=100,i=1;");
    assert!(result.is_ok());
    assert_eq!(parser.action, KittyAction::TransmitDisplay);
    assert_eq!(parser.format, KittyFormat::Png);
    assert_eq!(parser.image_id, Some(1));
}

#[test]
fn test_kitty_parser_c_key_controls_cursor_movement_flag() {
    let mut parser = KittyParser::new();
    parser.parse_chunk("a=p,i=1").unwrap();
    assert!(
        !parser.suppress_cursor_move,
        "omitted C= keeps the default (cursor moves)"
    );

    parser.reset();
    parser.parse_chunk("a=p,i=1,C=1").unwrap();
    assert!(
        parser.suppress_cursor_move,
        "C=1 suppresses the cursor move"
    );

    parser.reset();
    parser.parse_chunk("a=p,i=1,C=0").unwrap();
    assert!(
        !parser.suppress_cursor_move,
        "C=0 keeps the default (cursor moves)"
    );
}

#[test]
fn test_kitty_parser_chunked() {
    let mut parser = KittyParser::new();

    // First chunk
    let result = parser.parse_chunk("a=T,f=100,m=1;AAAA");
    assert!(result.is_ok());
    assert!(result.unwrap()); // more_chunks = true

    // Final chunk
    let result = parser.parse_chunk("m=0;BBBB");
    assert!(result.is_ok());
    assert!(!result.unwrap()); // more_chunks = false
}

#[test]
fn test_kitty_medium_from_char() {
    assert_eq!(KittyMedium::from_char('d'), Some(KittyMedium::Direct));
    assert_eq!(KittyMedium::from_char('f'), Some(KittyMedium::File));
    assert_eq!(KittyMedium::from_char('t'), Some(KittyMedium::TempFile));
    assert_eq!(KittyMedium::from_char('s'), Some(KittyMedium::SharedMem));
    assert_eq!(KittyMedium::from_char('x'), None);
}

/// `retain_temp_files` is terminal configuration, not per-escape parse
/// state: a mux daemon terminal keeps it across every escape's reset,
/// so the second `t=t` image in a pane retains its file exactly like
/// the first. Pins both that and the default (rendering client) delete.
#[test]
fn retain_temp_files_survives_reset_and_guards_the_delete() {
    use std::io::Write;

    // The t=t gate (SEC-101) requires the spec's marker in the filename.
    let mk_temp_png = || {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 4]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut f = tempfile::Builder::new()
            .prefix("tty-graphics-protocol-")
            .tempfile()
            .unwrap();
        f.write_all(&png).unwrap();
        f
    };

    // Default: a decoded payload deletes its file (the rendering
    // client's contract). Deletion rides decode_payload, not the bare
    // read, so a valid PNG is the fixture.
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let f = mk_temp_png();
    parser
        .decode_payload(f.path().to_str().unwrap().as_bytes().to_vec(), "shm")
        .unwrap();
    assert!(!f.path().exists(), "decoded read deletes the t=t file");

    // Retained: the read keeps the file, across resets.
    let mut parser = KittyParser::new();
    parser.retain_temp_files = true;
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let f = mk_temp_png();
    parser
        .decode_payload(f.path().to_str().unwrap().as_bytes().to_vec(), "shm")
        .unwrap();
    assert!(f.path().exists(), "retained read keeps the t=t file");
    parser.reset();
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let f2 = mk_temp_png();
    parser
        .decode_payload(f2.path().to_str().unwrap().as_bytes().to_vec(), "shm")
        .unwrap();
    assert!(
        f2.path().exists(),
        "reset preserves retain_temp_files — the second escape retains too"
    );
}

#[test]
fn test_kitty_file_transmission() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Create a valid 1x1 red PNG using the image crate
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    let mut png_data = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_data),
        image::ImageFormat::Png,
    )
    .expect("Failed to encode PNG");

    // Write to temp file
    let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
    temp_file
        .write_all(&png_data)
        .expect("Failed to write PNG data");
    let file_path = temp_file.path().to_str().unwrap();

    // Create parser and parse file transmission command
    // Note: file path must be base64-encoded in the protocol (without padding to match Kitty)
    let file_path_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD_NO_PAD, file_path);
    let mut parser = KittyParser::new();
    // t=f is unrestricted only under `All` (SEC-101 gate).
    parser.allow_file_media = FileMediaMode::All;
    let payload = format!("a=T,f=100,t=f;{}", file_path_b64);
    let result = parser.parse_chunk(&payload);

    assert!(result.is_ok());
    assert_eq!(parser.action, KittyAction::TransmitDisplay);
    assert_eq!(parser.format, KittyFormat::Png);
    assert_eq!(parser.medium, KittyMedium::File);

    // Test file loading
    let data = parser.get_data();
    let data = data.unwrap();
    assert!(!data.is_empty());
    assert_eq!(data, file_path.as_bytes());

    // Load file data
    let file_data = parser.load_file_data(&data);
    assert!(file_data.is_ok());
    let (file_data, pending_delete) = file_data.unwrap();
    assert_eq!(file_data.len(), png_data.len());
    assert!(pending_delete.is_none(), "t=f never deletes");

    // Decode pixels
    let decode_result = parser.decode_pixels(&file_data);
    assert!(
        decode_result.is_ok(),
        "Failed to decode: {:?}",
        decode_result.err()
    );
    let (width, height, pixels) = decode_result.unwrap();
    assert_eq!(width, 1);
    assert_eq!(height, 1);
    assert_eq!(pixels.len(), 4); // RGBA
                                 // Verify it's red
    assert_eq!(pixels[0], 255); // R
    assert_eq!(pixels[1], 0); // G
    assert_eq!(pixels[2], 0); // B
    assert_eq!(pixels[3], 255); // A
}

#[test]
fn test_kitty_file_security_directory_traversal() {
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;

    // Test directory traversal attempt
    let malicious_path = b"../../../etc/passwd";
    let result = parser.load_file_data(malicious_path);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Directory traversal"));
}

#[test]
fn test_kitty_file_security_nonexistent() {
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    parser.allow_file_media = FileMediaMode::All;

    // Test nonexistent file
    let nonexistent_path = b"/this/file/does/not/exist.png";
    let result = parser.load_file_data(nonexistent_path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("File not found"));
}

#[test]
fn test_kitty_compression_from_char() {
    assert_eq!(
        KittyCompression::from_char('z'),
        Some(KittyCompression::Zlib)
    );
    assert_eq!(KittyCompression::from_char('x'), None);
}

#[test]
fn test_kitty_compression_default() {
    let parser = KittyParser::new();
    assert_eq!(parser.compression, KittyCompression::None);
    assert!(!parser.is_compressed());
}

#[test]
fn test_kitty_parse_compression_param() {
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T,f=32,o=z,s=2,v=2;");
    assert!(result.is_ok());
    assert_eq!(parser.compression, KittyCompression::Zlib);
    assert!(parser.is_compressed());
}

#[test]
fn test_kitty_zlib_decompression() {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // Create a 2x2 RGBA image (16 bytes)
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, // Red
        0, 255, 0, 255, // Green
        0, 0, 255, 255, // Blue
        255, 255, 0, 255, // Yellow
    ];

    // Compress with zlib
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&pixel_data).unwrap();
    let compressed = encoder.finish().unwrap();

    // Base64 encode the compressed data
    let b64_compressed =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

    // Parse with o=z compression flag
    let mut parser = KittyParser::new();
    let payload = format!("a=T,f=32,o=z,s=2,v=2;{}", b64_compressed);
    let result = parser.parse_chunk(&payload);
    assert!(result.is_ok());
    assert_eq!(parser.compression, KittyCompression::Zlib);

    // get_data() should return decompressed data
    let data = parser.get_data().unwrap();
    assert_eq!(data, pixel_data);
}

#[test]
fn test_kitty_zlib_build_graphic() {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // Create a 2x2 RGBA image (16 bytes)
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, // Red
        0, 255, 0, 255, // Green
        0, 0, 255, 255, // Blue
        255, 255, 0, 255, // Yellow
    ];

    // Compress with zlib
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&pixel_data).unwrap();
    let compressed = encoder.finish().unwrap();

    // Base64 encode
    let b64_compressed =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

    // Parse and build graphic
    let mut parser = KittyParser::new();
    let payload = format!("a=T,f=32,o=z,s=2,v=2,i=42;{}", b64_compressed);
    parser.parse_chunk(&payload).unwrap();

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_ok());

    // Transmit-only, no display - should store the image
    let stored = store.get_kitty_image(42);
    assert!(stored.is_some());
    let (w, h, pixels) = stored.unwrap();
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    assert_eq!(*pixels, pixel_data);
}

#[test]
fn test_kitty_zlib_transmit_display_sets_compressed_flag() {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // Create a 2x2 RGBA image (16 bytes)
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];

    // Compress with zlib
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&pixel_data).unwrap();
    let compressed = encoder.finish().unwrap();

    let b64_compressed =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

    // TransmitDisplay with compression
    let mut parser = KittyParser::new();
    let payload = format!("a=T,f=32,o=z,s=2,v=2;{}", b64_compressed);
    parser.parse_chunk(&payload).unwrap();

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((5, 10), &mut store).unwrap();

    match result {
        KittyGraphicResult::Graphic(graphic) => {
            assert!(graphic.was_compressed, "was_compressed should be true");
            assert_eq!(graphic.width, 2);
            assert_eq!(graphic.height, 2);
            assert_eq!(*graphic.pixels, pixel_data);
        }
        _ => panic!("Expected Graphic result"),
    }
}

#[test]
fn test_kitty_no_compression_flag_unset() {
    // Uncompressed RGBA data for a 2x2 image
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];

    let b64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

    let mut parser = KittyParser::new();
    let payload = format!("a=T,f=32,s=2,v=2;{}", b64_data);
    parser.parse_chunk(&payload).unwrap();

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();

    match result {
        KittyGraphicResult::Graphic(graphic) => {
            assert!(!graphic.was_compressed, "was_compressed should be false");
        }
        _ => panic!("Expected Graphic result"),
    }
}

#[test]
fn test_kitty_zlib_chunked_transfer() {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // Create a 2x2 RGBA image (16 bytes)
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];

    // Compress with zlib
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&pixel_data).unwrap();
    let compressed = encoder.finish().unwrap();

    let b64_compressed =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

    // Split base64 into two chunks at a 4-byte boundary (base64 block size)
    let mid = (b64_compressed.len() / 2) & !3; // Round down to nearest multiple of 4
    let chunk1 = &b64_compressed[..mid];
    let chunk2 = &b64_compressed[mid..];

    // First chunk
    let mut parser = KittyParser::new();
    let payload1 = format!("a=T,f=32,o=z,s=2,v=2,m=1;{}", chunk1);
    let more = parser.parse_chunk(&payload1).unwrap();
    assert!(more);
    assert_eq!(parser.compression, KittyCompression::Zlib);

    // Second chunk
    let payload2 = format!("m=0;{}", chunk2);
    let more = parser.parse_chunk(&payload2).unwrap();
    assert!(!more);

    // Data should be decompressed correctly
    let data = parser.get_data().unwrap();
    assert_eq!(data, pixel_data);
}

#[test]
fn test_kitty_decompress_zlib_invalid_data() {
    // Test decompression with invalid zlib data falls back gracefully
    let invalid_data = vec![0x00, 0x01, 0x02, 0x03];
    let result = KittyParser::decompress_zlib(&invalid_data, MAX_KITTY_DECOMPRESSED_BYTES);
    assert!(result.is_err());
}

/// SEC-116: the streaming output bound rejects during inflation.
#[test]
fn test_decompress_zlib_limit_rejects_oversized_output() {
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut enc = ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    enc.write_all(&[0u8; 1024]).unwrap();
    let compressed = enc.finish().unwrap();

    let result = KittyParser::decompress_zlib(&compressed, 16);
    assert!(
        result.is_err(),
        "1 KiB inflating past a 16-byte limit errors"
    );
    let ok = KittyParser::decompress_zlib(&compressed, 1024).unwrap();
    assert_eq!(ok.len(), 1024, "a fitting limit still succeeds");
}

/// SEC-116: a small wire payload that inflates past the transmission's
/// geometry-derived limit errors quickly, never allocating the bomb's
/// full size (`f=32,s=1,v=1` bounds decompression at 4 bytes).
#[test]
fn test_kitty_zlib_bomb_errors_quickly() {
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut enc = ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
    enc.write_all(&vec![0u8; 4 * 1024 * 1024]).unwrap();
    let compressed = enc.finish().unwrap();
    assert!(
        compressed.len() < 64 * 1024,
        "the bomb stays small on the wire"
    );
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);
    let mut parser = KittyParser::new();
    parser
        .parse_chunk(&format!("a=T,f=32,s=1,v=1,o=z;{}", b64))
        .unwrap();
    assert!(
        parser.get_data().is_err(),
        "the bomb is rejected at the streaming bound"
    );
}

/// SEC-116: decoded chunks past [`MAX_KITTY_PAYLOAD_BYTES`] error and
/// reset the parser, so the next transmission starts from zero.
#[test]
fn test_chunk_accumulation_cap_resets_parser() {
    let chunk = vec![b'A'; 16 * 1024 * 1024]; // 16 MiB decoded
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk);
    let mut parser = KittyParser::new();
    // Four chunks land exactly at the 64 MiB cap (allowed), the fifth
    // crosses it.
    for i in 0..4 {
        parser
            .parse_chunk(&format!("a=T,m=1;{}", b64))
            .unwrap_or_else(|e| panic!("chunk {i} within cap: {e}"));
    }
    let err = parser
        .parse_chunk(&format!("a=T,m=0;{}", b64))
        .expect_err("the fifth 16 MiB chunk crosses the cap");
    assert!(err.to_string().contains("payload exceeds"));
    assert!(parser.data_chunks.is_empty(), "parser reset on overflow");
    assert_eq!(parser.data_bytes, 0, "byte counter reset with it");
    // A fresh small transmission parses cleanly afterward.
    parser.parse_chunk("a=T;QUFB").unwrap();
    assert_eq!(parser.get_data().unwrap(), b"AAA");
}

#[test]
fn test_kitty_build_placement_defaults() {
    let parser = KittyParser::new();
    let placement = parser.build_placement();
    assert_eq!(
        placement.display_mode,
        crate::graphics::ImageDisplayMode::Inline
    );
    assert!(placement.preserve_aspect_ratio);
    assert!(placement.columns.is_none());
    assert!(placement.rows.is_none());
    assert_eq!(placement.z_index, 0);
    assert_eq!(placement.x_offset, 0);
    assert_eq!(placement.y_offset, 0);
}

#[test]
fn test_kitty_build_placement_with_columns_rows() {
    let mut parser = KittyParser::new();
    parser.parse_chunk("a=T,f=100,c=10,r=5;").unwrap();
    let placement = parser.build_placement();
    assert_eq!(placement.columns, Some(10));
    assert_eq!(placement.rows, Some(5));
    assert_eq!(placement.requested_width.value, 10.0);
    assert_eq!(
        placement.requested_width.unit,
        crate::graphics::ImageSizeUnit::Cells
    );
    assert_eq!(placement.requested_height.value, 5.0);
    assert_eq!(
        placement.requested_height.unit,
        crate::graphics::ImageSizeUnit::Cells
    );
}

#[test]
fn test_kitty_build_placement_with_offsets() {
    let mut parser = KittyParser::new();
    parser
        .parse_chunk("a=T,f=100,x=5,y=3,w=40,h=20,X=7,Y=9;")
        .unwrap();
    let placement = parser.build_placement();
    assert_eq!(placement.source_x, 5);
    assert_eq!(placement.source_y, 3);
    assert_eq!(placement.source_width, 40);
    assert_eq!(placement.source_height, 20);
    assert_eq!(placement.x_offset, 7);
    assert_eq!(placement.y_offset, 9);
}

#[test]
fn test_kitty_z_index_for_placement() {
    let mut parser = KittyParser::new();
    parser.parse_chunk("a=p,i=1,z=-1;").unwrap();
    let placement = parser.build_placement();
    assert_eq!(placement.z_index, -1);
}

#[test]
fn test_kitty_z_as_frame_delay_for_frames() {
    let mut parser = KittyParser::new();
    parser.parse_chunk("a=f,i=1,z=100;").unwrap();
    // For frames, z is frame_delay, not z_index
    assert_eq!(parser.frame_delay_ms, Some(100));
    assert!(parser.z_index.is_none());
}

#[test]
fn test_kitty_transmit_display_has_placement() {
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let b64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

    let mut parser = KittyParser::new();
    let payload = format!("a=T,f=32,s=2,v=2,c=10,r=5,X=2,Y=3;{}", b64_data);
    parser.parse_chunk(&payload).unwrap();

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();

    match result {
        KittyGraphicResult::Graphic(graphic) => {
            assert_eq!(graphic.placement.columns, Some(10));
            assert_eq!(graphic.placement.rows, Some(5));
            assert_eq!(graphic.placement.x_offset, 2);
            assert_eq!(graphic.placement.y_offset, 3);
            assert_eq!(
                graphic.placement.display_mode,
                crate::graphics::ImageDisplayMode::Inline
            );
        }
        _ => panic!("Expected Graphic result"),
    }
}

#[test]
fn test_kitty_put_placement_with_z_index() {
    // First store an image
    let pixel_data: Vec<u8> = vec![255, 0, 0, 255];
    let b64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);

    let mut parser = KittyParser::new();
    let payload = format!("a=t,f=32,s=1,v=1,i=42;{}", b64_data);
    parser.parse_chunk(&payload).unwrap();
    let mut store = GraphicsStore::new();
    parser.build_graphic((0, 0), &mut store).unwrap();

    // Now put with z-index
    let mut parser2 = KittyParser::new();
    parser2.parse_chunk("a=p,i=42,z=5;").unwrap();
    let result = parser2.build_graphic((0, 0), &mut store).unwrap();

    match result {
        KittyGraphicResult::Graphic(graphic) => {
            assert_eq!(graphic.placement.z_index, 5);
        }
        _ => panic!("Expected Graphic result"),
    }
}

// =========================================================================
// Additional coverage: malformed/edge-case parser input, decompression
// fallback, placement math, delete/query actions, file security, raw
// RGB/RGBA decoding, frame/animation-control error paths.
// =========================================================================

// --- parse_chunk edge cases ---

#[test]
fn test_parse_chunk_empty_payload() {
    // Empty string: no pairs, no data — should succeed, no more_chunks.
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("");
    assert!(result.is_ok());
    assert!(!result.unwrap());
    // Default action preserved
    assert_eq!(parser.action, KittyAction::Transmit);
}

#[test]
fn test_parse_chunk_no_semicolon_uses_entire_payload_as_params() {
    // Without ';' data_str is "" so nothing is decoded.
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=q,i=7");
    assert!(result.is_ok());
    assert_eq!(parser.action, KittyAction::Query);
    assert_eq!(parser.image_id, Some(7));
}

#[test]
fn test_parse_chunk_pair_without_equals_is_ignored() {
    // A pair with no '=' should be skipped silently (no panic).
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("garbage,a=q");
    assert!(result.is_ok());
    assert_eq!(parser.action, KittyAction::Query);
}

#[test]
fn test_parse_chunk_empty_value_leaves_action_unchanged() {
    // Empty value -> value.chars().next() is None -> the action match arm's
    // `if let Some(c)` body never runs, so action is UNCHANGED (not reset).
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Query; // pre-set something non-default
    let _ = parser.parse_chunk("a=");
    assert_eq!(parser.action, KittyAction::Query); // unchanged
}

#[test]
fn test_parse_chunk_invalid_format_code_falls_back_to_default() {
    // Format code 99 is invalid -> falls back to default (Rgba).
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,f=99;");
    assert_eq!(parser.format, KittyFormat::Rgba);
}

#[test]
fn test_parse_chunk_non_numeric_format_is_ignored() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Png;
    let _ = parser.parse_chunk("a=T,f=abc;");
    assert_eq!(parser.format, KittyFormat::Png); // unchanged
}

#[test]
fn test_parse_chunk_invalid_medium_char_falls_back_to_default() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,t=x;"); // 'x' is not a valid medium
    assert_eq!(parser.medium, KittyMedium::Direct); // default
}

#[test]
fn test_parse_chunk_invalid_action_char_falls_back_to_default() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=Z;"); // 'Z' is not a valid action
    assert_eq!(parser.action, KittyAction::Transmit); // default
}

#[test]
fn test_parse_chunk_non_numeric_id_is_ignored() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,i=notanumber,p=alsonot;");
    assert_eq!(parser.image_id, None);
    assert_eq!(parser.placement_id, None);
}

#[test]
fn test_parse_chunk_unknown_key_is_ignored() {
    // Unknown key should hit the `_ => {}` arm cleanly.
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("zzz=123,a=q");
    assert!(result.is_ok());
    assert_eq!(parser.action, KittyAction::Query);
}

#[test]
fn test_parse_chunk_more_chunks_explicit_zero() {
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T,m=0;");
    assert!(result.is_ok());
    assert!(!result.unwrap());
}

#[test]
fn test_parse_chunk_more_chunks_non_one_is_false() {
    // m= only true when value == "1"; "2", "true", etc. are false.
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,m=2;");
    assert!(!parser.more_chunks);
}

#[test]
fn test_parse_chunk_width_and_height_for_transmit() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,s=10,v=20;");
    assert_eq!(parser.width, Some(10));
    assert_eq!(parser.height, Some(20));
}

#[test]
fn test_parse_chunk_columns_rows_for_non_frame() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,c=8,r=4;");
    assert_eq!(parser.columns, Some(8));
    assert_eq!(parser.rows, Some(4));
}

#[test]
fn test_parse_chunk_frame_overloads_c_and_r() {
    // For Frame: c= is composition, r= is frame number
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=f,c=1,r=5;");
    assert_eq!(parser.frame_composition, Some(CompositionMode::Overwrite));
    assert_eq!(parser.frame_number, Some(5));
    // columns/rows should NOT be set for frame action
    assert_eq!(parser.columns, None);
    assert_eq!(parser.rows, None);
}

#[test]
fn test_parse_chunk_frame_composition_invalid_char() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=f,c=Z;");
    assert_eq!(parser.frame_composition, None);
}

#[test]
fn test_parse_chunk_animation_control_overloads_s_and_v() {
    // For AnimationControl: s= is control, v= is num_plays
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=a,s=3,v=5,i=1;");
    assert_eq!(parser.action, KittyAction::AnimationControl);
    assert_eq!(
        parser.animation_control,
        Some(AnimationControl::EnableLooping)
    );
    assert_eq!(parser.num_plays, Some(5));
    // width/height should NOT be set for animation-control action
    assert_eq!(parser.width, None);
    assert_eq!(parser.height, None);
}

#[test]
fn test_parse_chunk_animation_control_invalid_s_value() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=a,s=99;");
    assert_eq!(parser.animation_control, None);
}

#[test]
fn test_parse_chunk_quietness_levels() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,q=2;");
    assert_eq!(parser.quietness, 2);

    let mut parser2 = KittyParser::new();
    let _ = parser2.parse_chunk("a=T,q=notanumber;");
    assert_eq!(parser2.quietness, 0); // default, parse failed
}

#[test]
fn test_parse_chunk_offsets_and_parents() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=p,i=1,P=2,Q=3,H=10;");
    assert_eq!(parser.x_offset, None); // x= not parsed here
    assert_eq!(parser.y_offset, None); // y= not parsed here
    assert_eq!(parser.parent_image_id, Some(2));
    assert_eq!(parser.parent_placement_id, Some(3));
    assert_eq!(parser.relative_x_offset, Some(10));
}

#[test]
fn test_parse_chunk_virtual_placement_flag() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=p,i=1,U=1;");
    assert!(parser.is_virtual);

    let mut parser2 = KittyParser::new();
    let _ = parser2.parse_chunk("a=p,i=1,U=0;");
    assert!(!parser2.is_virtual);
}

#[test]
fn test_parse_chunk_x_y_offsets_parsed() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,x=15,y=25;");
    assert_eq!(parser.source_x, Some(15));
    assert_eq!(parser.source_y, Some(25));
    let _ = parser.parse_chunk("X=35,Y=45;");
    assert_eq!(parser.x_offset, Some(35));
    assert_eq!(parser.y_offset, Some(45));
}

#[test]
fn test_parse_chunk_herdr_scrambled_field_order() {
    // Emitters disagree on key order (Herdr interleaves identifying and
    // display params). Every field must parse regardless of position.
    let mut parser = KittyParser::new();
    let _ =
        parser.parse_chunk("a=T,Y=9,X=7,h=20,w=40,y=3,x=5,r=5,c=10,s=11,v=13,i=9,p=2,z=3,q=1,U=0;");
    assert_eq!(parser.image_id, Some(9));
    assert_eq!(parser.placement_id, Some(2));
    assert_eq!(parser.width, Some(11));
    assert_eq!(parser.height, Some(13));
    assert_eq!(parser.columns, Some(10));
    assert_eq!(parser.rows, Some(5));
    assert_eq!(parser.source_x, Some(5));
    assert_eq!(parser.source_y, Some(3));
    assert_eq!(parser.source_width, Some(40));
    assert_eq!(parser.source_height, Some(20));
    assert_eq!(parser.x_offset, Some(7));
    assert_eq!(parser.y_offset, Some(9));
    assert_eq!(parser.z_index, Some(3));
    assert_eq!(parser.quietness, 1);
    assert!(!parser.is_virtual);
}

#[test]
fn test_parse_chunk_compression_invalid_char_ignored() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,o=Q;"); // 'Q' not a valid compression
    assert_eq!(parser.compression, KittyCompression::None);
}

#[test]
fn test_parse_chunk_base64_invalid_returns_error() {
    // Characters outside the base64 alphabet must produce Base64Error.
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T;!!!!notbase64!!!!");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("base64") || err.contains("Invalid base64"));
}

#[test]
fn test_parse_chunk_base64_unpadded_decodes() {
    // STANDARD_NO_PAD fallback path: "QUFB" is "AAA" with no padding.
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T;QUFB");
    assert!(result.is_ok());
    let data = parser.get_data().unwrap();
    assert_eq!(data, b"AAA");
}

#[test]
fn test_parse_chunk_base64_padded_decodes() {
    // Standard padded path: 5 bytes (QkFBQPI= is "AAAABBBB" tail) -- use a
    // length that requires padding. 1 byte -> "QQ==" decodes to "A".
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=T;QQ==");
    assert!(result.is_ok());
    assert_eq!(parser.get_data().unwrap(), b"A");
}

// --- reset() ---

#[test]
fn test_reset_clears_all_state() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,f=100,i=42,m=1;o=z,s=2,v=2;AAAA");
    assert_eq!(parser.image_id, Some(42));
    assert!(parser.more_chunks);

    parser.reset();
    assert_eq!(parser.action, KittyAction::Transmit);
    assert_eq!(parser.image_id, None);
    assert_eq!(parser.format, KittyFormat::Rgba);
    assert!(!parser.more_chunks);
    assert_eq!(parser.compression, KittyCompression::None);
    // After reset, accumulated data chunks are gone.
    assert!(parser.get_data().unwrap().is_empty());
}

// --- get_data decompression failure (SEC-116) ---

#[test]
fn test_get_data_zlib_failure_propagates() {
    // If o=z is set but the data is not actually zlib, get_data()
    // propagates the error (SEC-116): the old raw-bytes fallback fed
    // compressed bytes to the pixel decoder.
    let mut parser = KittyParser::new();
    // Mark compressed with valid zlib marker but corrupt the body.
    let bad = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"not zlib");
    let _ = parser.parse_chunk(&format!("a=T,o=z;{}", bad));
    assert!(parser.is_compressed());
    assert!(parser.get_data().is_err());
}

#[test]
fn test_get_data_uncompressed_returns_raw_concat() {
    let mut parser = KittyParser::new();
    let _ = parser.parse_chunk("a=T,m=1;QUFB"); // "AAA"
    let _ = parser.parse_chunk("m=0;QkJD"); // "BBC"
    assert_eq!(parser.get_data().unwrap(), b"AAABBC");
}

// --- parse_delete_target variants ---

#[test]
fn test_parse_delete_target_uppercase_letters() {
    // A, C, Z uppercase are valid per parse_delete_target match arms.
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;

    let _ = parser.parse_chunk("a=d,d=A;");
    assert_eq!(parser.delete_target, Some(KittyDeleteTarget::All));

    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=C;");
    assert_eq!(parser.delete_target, Some(KittyDeleteTarget::AtCursor));

    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=Z;");
    assert_eq!(parser.delete_target, Some(KittyDeleteTarget::OnScreen));
}

#[test]
fn test_parse_delete_target_lowercase_c_and_z() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=c;");
    assert_eq!(parser.delete_target, Some(KittyDeleteTarget::AtCursor));

    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=z;");
    assert_eq!(parser.delete_target, Some(KittyDeleteTarget::OnScreen));
}

#[test]
fn test_parse_delete_target_unknown_char_is_none() {
    // 'x', 'y', 'i', 'p' are NOT handled by parse_delete_target,
    // so the target stays None even though build_graphic has branches
    // for ByColumn/ByRow/ById/ByPlacement/InCell that are never reached.
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=x;");
    assert_eq!(parser.delete_target, None);

    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=p;");
    assert_eq!(parser.delete_target, None);
}

#[test]
fn test_parse_delete_target_empty_value_is_none() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let _ = parser.parse_chunk("a=d,d=;");
    assert_eq!(parser.delete_target, None);
}

// --- parse_delete_target: by-id / by-placement / by-column / by-row ---

#[test]
fn parse_delete_target_by_id_column_row_placement() {
    // d=i -> ById(image_id)
    let mut p = KittyParser::new();
    p.image_id = Some(7);
    let _ = p.parse_chunk("d=i;");
    assert_eq!(p.delete_target, Some(KittyDeleteTarget::ById(7)));

    // d=x -> ByColumn(source_x)
    let mut p = KittyParser::new();
    p.source_x = Some(3);
    let _ = p.parse_chunk("d=x;");
    assert_eq!(p.delete_target, Some(KittyDeleteTarget::ByColumn(3)));

    // d=y -> ByRow(source_y)
    let mut p = KittyParser::new();
    p.source_y = Some(9);
    let _ = p.parse_chunk("d=y;");
    assert_eq!(p.delete_target, Some(KittyDeleteTarget::ByRow(9)));

    // d=p -> ByPlacement(image_id, placement_id)
    let mut p = KittyParser::new();
    p.image_id = Some(5);
    p.placement_id = Some(2);
    let _ = p.parse_chunk("d=p;");
    assert_eq!(
        p.delete_target,
        Some(KittyDeleteTarget::ByPlacement(5, Some(2)))
    );
}

#[test]
fn parse_delete_target_by_id_without_identifying_param_is_none() {
    // d=i with no image_id parsed yet -> no-op (None), not a panic.
    let mut p = KittyParser::new();
    let _ = p.parse_chunk("d=i;");
    assert_eq!(p.delete_target, None);
}

#[test]
fn herdr_delete_order_resolves_id_and_placement_targets() {
    let mut by_id = KittyParser::new();
    by_id.parse_chunk("a=d,d=I,i=42;").unwrap();
    assert_eq!(by_id.delete_target, Some(KittyDeleteTarget::ById(42)));

    let mut by_placement = KittyParser::new();
    by_placement.parse_chunk("a=d,d=i,i=42,p=7;").unwrap();
    assert_eq!(
        by_placement.delete_target,
        Some(KittyDeleteTarget::ByPlacement(42, Some(7)))
    );
}

#[test]
fn herdr_by_placement_delete_preserves_sibling_placement() {
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 42, Some(1), (0, 0));
    place_only(&mut store, 42, 2, (5, 0));
    assert_eq!(store.placements.len(), 2);

    let mut delete = KittyParser::new();
    delete.parse_chunk("a=d,d=i,i=42,p=1;").unwrap();
    delete.build_graphic((0, 0), &mut store).unwrap();

    assert_eq!(store.placements.len(), 1);
    assert_eq!(store.placements[0].kitty_image_id, Some(42));
    assert_eq!(store.placements[0].kitty_placement_id, Some(2));
}

#[test]
fn herdr_by_id_delete_removes_all_image_placements() {
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 42, Some(1), (0, 0));
    place_only(&mut store, 42, 2, (5, 0));
    assert_eq!(store.placements.len(), 2);

    let mut delete = KittyParser::new();
    delete.parse_chunk("a=d,d=I,i=42;").unwrap();
    delete.build_graphic((0, 0), &mut store).unwrap();

    assert!(store.placements.is_empty());
}

// --- V= relative offset regression (was dropped unless P= preceded it) ---

#[test]
fn parse_chunk_v_offset_without_parent_is_parsed() {
    // Regression: V= was silently dropped unless P= had been parsed first
    // (unlike H=). It must be stored unconditionally; the offset is only
    // applied when a parent exists (see build_graphic).
    let mut parser = KittyParser::new();
    let result = parser.parse_chunk("a=t,V=15;");
    assert!(result.is_ok());
    assert_eq!(parser.relative_y_offset, Some(15));
}

// --- decode_pixels dimension-overflow regression ---

#[test]
fn decode_pixels_rejects_rgba_dimension_overflow() {
    // Regression: width*height*4 must not wrap usize. Attacker-controlled
    // u32 dimensions that overflow must report "overflow" (previously the
    // wrapping arithmetic bypassed the size check -> huge dims over a tiny
    // buffer -> OOB/panic downstream).
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgba;
    parser.width = Some(u32::MAX);
    parser.height = Some(u32::MAX);
    let result = parser.decode_pixels(&[0u8; 8]);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("overflow"),
        "overflowing dimensions must be reported as an overflow error"
    );
}

#[test]
fn decode_pixels_rejects_rgb_dimension_overflow() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgb;
    parser.width = Some(u32::MAX);
    parser.height = Some(u32::MAX);
    let result = parser.decode_pixels(&[0u8; 8]);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("overflow"),
        "overflowing RGB dimensions must be reported as an overflow error"
    );
}

// --- build_graphic: Delete action coverage ---

#[test]
fn test_build_graphic_delete_no_target_returns_none() {
    // Delete with no parsed target should be a no-op -> None.
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Delete;
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
}

/// Helper: transmit a 1x1 RGBA image with the given id and placement_id at
/// the given position, and add the resulting placement to `store.placements`.
///
/// `build_graphic()` calls `store.store_kitty_image()` internally but does
/// NOT push the returned graphic into `store.placements` — that is the
/// terminal-integration layer's job (see GraphicsStore::add_graphic).
fn transmit_and_add(
    store: &mut GraphicsStore,
    image_id: u32,
    placement_id: Option<u32>,
    position: (usize, usize),
) {
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut tx = KittyParser::new();
    let payload = match placement_id {
        Some(pid) => format!("a=T,f=32,s=1,v=1,i={},p={};{}", image_id, pid, b64),
        None => format!("a=T,f=32,s=1,v=1,i={};{}", image_id, b64),
    };
    tx.parse_chunk(&payload).unwrap();
    match tx.build_graphic(position, store).unwrap() {
        KittyGraphicResult::Graphic(g) => store.add_graphic(g),
        other => panic!("transmit_and_add expected Graphic, got {:?}", other),
    }
}
/// Place an already-transmitted image (a=p) without retransmitting.
fn place_only(
    store: &mut GraphicsStore,
    image_id: u32,
    placement_id: u32,
    position: (usize, usize),
) {
    let mut p = KittyParser::new();
    let payload = format!("a=p,i={},p={};", image_id, placement_id);
    p.parse_chunk(&payload).unwrap();
    match p.build_graphic(position, store).unwrap() {
        KittyGraphicResult::Graphic(g) => store.add_graphic(g),
        other => panic!("place_only expected Graphic, got {:?}", other),
    }
}

#[test]
fn test_repeated_nonzero_placement_pair_replaces() {
    // A repeated nonzero (image_id, placement_id) pair is an upsert:
    // the second a=p replaces the first in place without retransmitting.
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 5, Some(3), (0, 0));
    place_only(&mut store, 5, 3, (4, 2));
    assert_eq!(store.placements.len(), 1);
    assert_eq!(store.placements[0].position, (4, 2));
    assert_eq!(store.placements[0].kitty_image_id, Some(5));
    assert_eq!(store.placements[0].kitty_placement_id, Some(3));
}

#[test]
fn test_zero_and_omitted_placement_ids_coexist_full_pipeline() {
    let mut store = GraphicsStore::new();
    // Transmit once; subsequent coexisting placements use a=p (no retransmit).
    transmit_and_add(&mut store, 5, None, (0, 0));
    assert_eq!(store.placements.len(), 1);
    place_only(&mut store, 5, 0, (1, 0));
    assert_eq!(store.placements.len(), 2);
    place_only(&mut store, 5, 3, (3, 0));
    place_only(&mut store, 5, 4, (4, 0));
    assert_eq!(store.placements.len(), 4);
    // Re-sending (5, 4) replaces only that placement.
    place_only(&mut store, 5, 4, (9, 9));
    assert_eq!(store.placements.len(), 4);
    assert!(store
        .placements
        .iter()
        .any(|g| g.position == (9, 9) && g.kitty_placement_id == Some(4)));
}

#[test]
fn test_build_graphic_delete_all_clears_store() {
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, None, (0, 0));
    assert_eq!(store.placements.len(), 1);

    let mut del = KittyParser::new();
    del.parse_chunk("a=d,d=a;").unwrap();
    let result = del.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
    assert!(store.placements.is_empty());
}

#[test]
fn test_build_graphic_delete_at_cursor() {
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, None, (3, 4));
    assert_eq!(store.placements.len(), 1);

    // Delete at cursor (3,4) — should remove it.
    let mut del = KittyParser::new();
    del.parse_chunk("a=d,d=c;").unwrap();
    let _ = del.build_graphic((3, 4), &mut store).unwrap();
    assert!(store.placements.is_empty());

    // A different cursor position should leave other placements alone.
    transmit_and_add(&mut store, 2, None, (5, 6));
    assert_eq!(store.placements.len(), 1);

    let mut del2 = KittyParser::new();
    del2.parse_chunk("a=d,d=c;").unwrap();
    let _ = del2.build_graphic((0, 0), &mut store).unwrap(); // cursor elsewhere
    assert_eq!(store.placements.len(), 1); // still there
}

#[test]
fn test_build_graphic_delete_in_cell_uses_cursor() {
    // InCell behaves the same as AtCursor in this implementation.
    // parse_delete_target never produces InCell, so we set it directly
    // to cover the InCell branch.
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, None, (7, 8));
    assert_eq!(store.placements.len(), 1);

    let mut del = KittyParser::new();
    del.action = KittyAction::Delete;
    del.delete_target = Some(KittyDeleteTarget::InCell);
    let _ = del.build_graphic((7, 8), &mut store).unwrap();
    assert!(store.placements.is_empty());
}

#[test]
fn test_build_graphic_delete_on_screen() {
    let mut store = GraphicsStore::new();
    for i in 1..=3 {
        transmit_and_add(&mut store, i, None, (i as usize, 0));
    }
    assert_eq!(store.placements.len(), 3);

    let mut del = KittyParser::new();
    del.parse_chunk("a=d,d=z;").unwrap();
    let _ = del.build_graphic((0, 0), &mut store).unwrap();
    assert!(store.placements.is_empty());
}

#[test]
fn test_build_graphic_delete_by_id() {
    // parse_delete_target cannot produce ById, so we set it directly
    // to cover the ById branch.
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, None, (0, 0));
    transmit_and_add(&mut store, 2, None, (1, 0));
    assert_eq!(store.placements.len(), 2);

    // Delete only image id=1
    let mut del = KittyParser::new();
    del.action = KittyAction::Delete;
    del.delete_target = Some(KittyDeleteTarget::ById(1));
    let _ = del.build_graphic((0, 0), &mut store).unwrap();
    // One placement should remain (image id=2).
    assert_eq!(store.placements.len(), 1);
    assert_eq!(store.placements[0].kitty_image_id, Some(2));
}

#[test]
fn test_build_graphic_delete_by_placement() {
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, Some(100), (0, 0));
    place_only(&mut store, 1, 200, (1, 0));
    assert_eq!(store.placements.len(), 2);

    // Delete only (image_id=1, placement_id=100)
    let mut del = KittyParser::new();
    del.action = KittyAction::Delete;
    del.delete_target = Some(KittyDeleteTarget::ByPlacement(1, Some(100)));
    let _ = del.build_graphic((0, 0), &mut store).unwrap();
    assert_eq!(store.placements.len(), 1);
    assert_eq!(store.placements[0].kitty_placement_id, Some(200));
}

#[test]
fn test_build_graphic_delete_by_column_and_row() {
    // These branches need cell_dimensions set; the retain math uses
    // div_ceil over cell width/height. We construct targets directly
    // (parse_delete_target cannot produce ByColumn/ByRow).
    let mut store = GraphicsStore::new();
    transmit_and_add(&mut store, 1, None, (0, 0));
    transmit_and_add(&mut store, 2, None, (5, 5));
    for g in store.placements.iter_mut() {
        g.set_cell_dimensions(1, 1); // 1x1 pixel cell, simplest case
    }
    assert_eq!(store.placements.len(), 2);

    // ByColumn(0): placement at col=0 spans col 0 -> removed.
    // Placement at col=5 stays.
    let mut del_col = KittyParser::new();
    del_col.action = KittyAction::Delete;
    del_col.delete_target = Some(KittyDeleteTarget::ByColumn(0));
    let _ = del_col.build_graphic((0, 0), &mut store).unwrap();
    assert_eq!(store.placements.len(), 1);
    assert_eq!(store.placements[0].position.0, 5);

    // ByRow(5): remaining placement is at row=5 -> removed.
    let mut del_row = KittyParser::new();
    del_row.action = KittyAction::Delete;
    del_row.delete_target = Some(KittyDeleteTarget::ByRow(5));
    let _ = del_row.build_graphic((0, 0), &mut store).unwrap();
    assert!(store.placements.is_empty());
}

// --- build_graphic: Query ---

#[test]
fn test_build_graphic_query_returns_none() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Query;
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
}

// --- build_graphic: Put ---

#[test]
fn test_build_graphic_put_missing_image_returns_error() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Put;
    parser.image_id = Some(999); // never transmitted
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Image not found"));
}

#[test]
fn test_build_graphic_put_virtual_placement() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Put;
    parser.image_id = Some(7);
    parser.placement_id = Some(11);
    parser.is_virtual = true;
    parser.columns = Some(3);
    parser.rows = Some(2);

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((4, 5), &mut store).unwrap();
    match result {
        KittyGraphicResult::VirtualPlacement {
            image_id,
            placement_id,
            position,
            cols,
            rows,
        } => {
            assert_eq!(image_id, 7);
            assert_eq!(placement_id, 11);
            assert_eq!(position, (4, 5));
            assert_eq!(cols, 3);
            assert_eq!(rows, 2);
        }
        other => panic!("Expected VirtualPlacement, got {:?}", other),
    }
    // Should also register a virtual placement in the store.
    assert!(store.get_virtual_placement(7, 11).is_some());
}

#[test]
fn test_build_graphic_put_regular_uses_stored_pixels() {
    // Transmit then Put.
    let pixels: Vec<u8> = vec![10, 20, 30, 40];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();

    let mut tx = KittyParser::new();
    tx.parse_chunk(&format!("a=t,f=32,s=1,v=1,i=5;{}", b64))
        .unwrap();
    let _ = tx.build_graphic((0, 0), &mut store).unwrap();

    let mut put = KittyParser::new();
    put.parse_chunk("a=p,i=5,z=7,X=1,Y=2;").unwrap();
    let result = put.build_graphic((9, 9), &mut store).unwrap();
    match result {
        KittyGraphicResult::Graphic(g) => {
            assert_eq!(g.kitty_image_id, Some(5));
            assert_eq!(g.position, (9, 9));
            assert_eq!(g.placement.z_index, 7);
            assert_eq!(g.placement.x_offset, 1);
            assert_eq!(g.placement.y_offset, 2);
        }
        other => panic!("Expected Graphic, got {:?}", other),
    }
}

#[test]
fn test_build_graphic_put_with_relative_positioning() {
    let pixels: Vec<u8> = vec![1, 2, 3, 4];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();

    let mut tx = KittyParser::new();
    tx.parse_chunk(&format!("a=t,f=32,s=1,v=1,i=8;{}", b64))
        .unwrap();
    let _ = tx.build_graphic((0, 0), &mut store).unwrap();

    let mut put = KittyParser::new();
    // P= parent_image_id enables the relative-positioning branch.
    put.parse_chunk("a=p,i=8,P=99,Q=88,H=5;").unwrap();
    let result = put.build_graphic((1, 1), &mut store).unwrap();
    match result {
        KittyGraphicResult::Graphic(g) => {
            assert_eq!(g.parent_image_id, Some(99));
            assert_eq!(g.parent_placement_id, Some(88));
            assert_eq!(g.relative_x_offset, 5);
            assert_eq!(g.relative_y_offset, 0); // V= not parseable without parent (chicken/egg), defaults to 0
        }
        other => panic!("Expected Graphic, got {:?}", other),
    }
}

// --- build_graphic: Transmit / TransmitDisplay errors ---

#[test]
fn test_build_graphic_transmit_no_data_returns_error() {
    let mut parser = KittyParser::new();
    // TransmitDisplay with no data section.
    parser.action = KittyAction::TransmitDisplay;
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No image data"));
}

#[test]
fn test_build_graphic_transmit_shared_memory_unsupported() {
    let mut parser = KittyParser::new();
    // Provide some data so we get past the empty check, then fail at SharedMem.
    parser.action = KittyAction::TransmitDisplay;
    parser.medium = KittyMedium::SharedMem;
    // Manually push a data chunk by parsing one (compression stays None).
    let _ = parser.parse_chunk("a=T,t=s;QUFB"); // sets medium=SharedMem
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Shared memory transmission not supported"));
}

#[test]
fn test_build_graphic_transmit_only_stores_but_no_placement() {
    // Action 't' (Transmit, not TransmitDisplay) with image_id stores
    // the image but returns KittyGraphicResult::None.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut parser = KittyParser::new();
    parser
        .parse_chunk(&format!("a=t,f=32,s=1,v=1,i=33;{}", b64))
        .unwrap();
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
    assert!(store.get_kitty_image(33).is_some());
    // No placement should have been created.
    assert!(store.placements.is_empty());
}

#[test]
fn test_build_graphic_transmit_display_virtual() {
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut parser = KittyParser::new();
    // U=1 makes TransmitDisplay produce a virtual placement.
    parser
        .parse_chunk(&format!("a=T,f=32,s=1,v=1,i=44,U=1,c=2,r=3;{}", b64))
        .unwrap();
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((1, 2), &mut store).unwrap();
    match result {
        KittyGraphicResult::VirtualPlacement {
            image_id,
            cols,
            rows,
            position,
            ..
        } => {
            assert_eq!(image_id, 44);
            assert_eq!(cols, 2);
            assert_eq!(rows, 3);
            assert_eq!(position, (1, 2));
        }
        other => panic!("Expected VirtualPlacement, got {:?}", other),
    }
}

#[test]
fn test_build_graphic_transmit_display_with_relative_positioning() {
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut parser = KittyParser::new();
    parser
        .parse_chunk(&format!("a=T,f=32,s=1,v=1,i=55,P=1,Q=2,H=3;{}", b64))
        .unwrap();
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store).unwrap();
    match result {
        KittyGraphicResult::Graphic(g) => {
            assert_eq!(g.parent_image_id, Some(1));
            assert_eq!(g.parent_placement_id, Some(2));
            assert_eq!(g.relative_x_offset, 3);
            assert_eq!(g.relative_y_offset, 0);
        }
        other => panic!("Expected Graphic, got {:?}", other),
    }
}

// --- decode_pixels error paths ---

#[test]
fn test_decode_pixels_rgba_missing_width() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgba;
    parser.height = Some(2); // no width
    let result = parser.decode_pixels(&[0u8; 8]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Width required"));
}

#[test]
fn test_decode_pixels_rgba_missing_height() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgba;
    parser.width = Some(2); // no height
    let result = parser.decode_pixels(&[0u8; 8]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Height required"));
}

#[test]
fn test_decode_pixels_rgba_size_mismatch() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgba;
    parser.width = Some(2);
    parser.height = Some(2);
    // Expected 2*2*4 = 16 bytes, but we provide 8.
    let result = parser.decode_pixels(&[0u8; 8]);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Data size mismatch"));
}

#[test]
fn test_decode_pixels_rgba_exact_match() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgba;
    parser.width = Some(1);
    parser.height = Some(1);
    let data = vec![1, 2, 3, 4];
    let (w, h, px) = parser.decode_pixels(&data).unwrap();
    assert_eq!(w, 1);
    assert_eq!(h, 1);
    assert_eq!(px, vec![1, 2, 3, 4]);
}

#[test]
fn test_decode_pixels_rgb_missing_width() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgb;
    parser.height = Some(2);
    let result = parser.decode_pixels(&[0u8; 6]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Width required"));
}

#[test]
fn test_decode_pixels_rgb_missing_height() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgb;
    parser.width = Some(2);
    let result = parser.decode_pixels(&[0u8; 6]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Height required"));
}

#[test]
fn test_decode_pixels_rgb_size_mismatch() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgb;
    parser.width = Some(2);
    parser.height = Some(2);
    // Expected 2*2*3 = 12 bytes, provide 6.
    let result = parser.decode_pixels(&[0u8; 6]);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Data size mismatch"));
}

#[test]
fn test_decode_pixels_rgb_converts_to_rgba() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Rgb;
    parser.width = Some(1);
    parser.height = Some(2);
    // 2 pixels RGB = 6 bytes
    let data = vec![10, 20, 30, 40, 50, 60];
    let (w, h, px) = parser.decode_pixels(&data).unwrap();
    assert_eq!(w, 1);
    assert_eq!(h, 2);
    // Each RGB triple becomes RGBA with alpha=255.
    assert_eq!(px, vec![10, 20, 30, 255, 40, 50, 60, 255]);
}

#[test]
fn test_decode_pixels_png_invalid() {
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Png;
    let result = parser.decode_pixels(b"not a png");
    assert!(result.is_err());
    // ImageError carries "Image decode failed" in its Display.
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Image decode failed") || msg.contains("decode"));
}

#[test]
fn test_decode_pixels_png_valid() {
    // Encode a real PNG and round-trip through decode_pixels.
    let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 4]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Png;
    let (w, h, px) = parser.decode_pixels(&png).unwrap();
    assert_eq!(w, 2);
    assert_eq!(h, 1);
    assert_eq!(px, vec![1, 2, 3, 4, 1, 2, 3, 4]);
}

#[test]
fn test_decode_pixels_png_rejects_oversized_dimensions() {
    // Encode a tiny valid PNG, then patch its IHDR width/height so the
    // implied RGBA buffer would exceed MAX_IMAGE_DIMENSION and the image
    // crate's default max_alloc (512 MiB). This exercises the
    // `ImageReader::limits()` guard (SEC-001) without needing to
    // actually construct a real oversized image.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    // PNG layout: 8-byte signature, 4-byte length, 4-byte "IHDR" type,
    // 13-byte IHDR data (width, height, ...), 4-byte CRC32 of type+data.
    let oversized: u32 = 100_000; // exceeds MAX_IMAGE_DIMENSION (16384)
    png[16..20].copy_from_slice(&oversized.to_be_bytes()); // width
    png[20..24].copy_from_slice(&oversized.to_be_bytes()); // height

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());

    let mut parser = KittyParser::new();
    parser.format = KittyFormat::Png;
    let result = parser.decode_pixels(&png);
    assert!(result.is_err(), "oversized PNG dimensions must be rejected");
}

// --- build_graphic: Frame action ---

#[test]
fn test_build_graphic_frame_missing_image_id_returns_error() {
    let mut parser = KittyParser::new();
    // Provide data so we get past the empty check.
    let _ = parser.parse_chunk("a=f,f=32,s=1,v=1;QUFB"); // no i=
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Frame requires image ID"));
}

#[test]
fn test_build_graphic_frame_no_data_returns_error() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Frame;
    parser.image_id = Some(1);
    // No data parsed -> get_data() is empty.
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No frame data"));
}

#[test]
fn test_build_graphic_frame_shared_memory_unsupported() {
    let mut parser = KittyParser::new();
    // f=action, with data, medium=SharedMem.
    let _ = parser.parse_chunk("a=f,f=32,s=1,v=1,t=s,i=1;QUFB");
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Shared memory not supported for frames"));
}

#[test]
fn test_build_graphic_frame_num_one_creates_placement_and_animation() {
    let pixels: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut parser = KittyParser::new();
    // Frame 1: should create both an animation frame AND a placement.
    parser
        .parse_chunk(&format!(
            "a=f,f=32,s=2,v=2,i=77,r=1,z=50,c=1,x=1,y=2;{}",
            b64
        ))
        .unwrap();
    assert_eq!(parser.frame_number, Some(1));
    assert_eq!(parser.frame_delay_ms, Some(50));
    assert_eq!(parser.frame_composition, Some(CompositionMode::Overwrite));

    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((3, 4), &mut store).unwrap();
    match result {
        KittyGraphicResult::Graphic(g) => {
            assert_eq!(g.kitty_image_id, Some(77));
            assert_eq!(g.position, (3, 4));
            assert_eq!(g.placement.x_offset, 0);
            assert_eq!(g.placement.y_offset, 0);
            // x= was also used for frame offset
        }
        other => panic!("Expected Graphic for frame 1, got {:?}", other),
    }
    // Animation frame should be present.
    let anim = store.get_animation(77);
    assert!(anim.is_some(), "animation should exist for image_id=77");
    let frame = anim.unwrap().get_frame(1);
    assert!(frame.is_some());
    assert_eq!(frame.unwrap().delay_ms, 50);
    // Image should also be stored for Put reuse.
    assert!(store.get_kitty_image(77).is_some());
}

#[test]
fn test_build_graphic_frame_subsequent_no_placement() {
    // Pre-seed an animation by transmitting frame 1 first.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();

    let mut f1 = KittyParser::new();
    f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=9,r=1;{}", b64))
        .unwrap();
    let _ = f1.build_graphic((0, 0), &mut store).unwrap();

    // Now frame 2: should NOT create a new placement.
    let mut f2 = KittyParser::new();
    f2.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=9,r=2;{}", b64))
        .unwrap();
    let result = f2.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
    // Animation should now have 2 frames.
    let anim = store.get_animation(9).unwrap();
    assert_eq!(anim.frame_count(), 2);
}

// --- build_graphic: AnimationControl ---

#[test]
fn test_build_graphic_animation_control_missing_image_id() {
    let mut parser = KittyParser::new();
    parser.action = KittyAction::AnimationControl;
    // No i=
    let mut store = GraphicsStore::new();
    let result = parser.build_graphic((0, 0), &mut store);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Animation control requires image ID"));
}

#[test]
fn test_build_graphic_animation_control_with_state() {
    // Seed an animation so control_animation has something to act on.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();
    let mut f1 = KittyParser::new();
    f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=5,r=1;{}", b64))
        .unwrap();
    let _ = f1.build_graphic((0, 0), &mut store).unwrap();

    // Now send animation control: s=1 (stop), v=2 (num_plays=2).
    let mut ctrl = KittyParser::new();
    ctrl.parse_chunk("a=a,i=5,s=1,v=2;").unwrap();
    let result = ctrl.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));

    // num_plays=2 -> loop_count = N-1 = 1.
    let anim = store.get_animation(5).unwrap();
    assert_eq!(anim.loop_count, 1);
}

#[test]
fn test_build_graphic_animation_control_num_plays_zero_ignored() {
    // Per spec, v=0 is ignored.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();
    let mut f1 = KittyParser::new();
    f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=6,r=1;{}", b64))
        .unwrap();
    let _ = f1.build_graphic((0, 0), &mut store).unwrap();

    let anim_before = store.get_animation(6).unwrap().loop_count;
    let mut ctrl = KittyParser::new();
    ctrl.parse_chunk("a=a,i=6,v=0;").unwrap();
    let _ = ctrl.build_graphic((0, 0), &mut store).unwrap();
    let anim_after = store.get_animation(6).unwrap().loop_count;
    assert_eq!(anim_before, anim_after, "v=0 must be ignored");
}

#[test]
fn test_build_graphic_animation_control_num_plays_one_is_infinite() {
    // v=1 means infinite -> loop_count = 0.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();
    let mut f1 = KittyParser::new();
    f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=7,r=1;{}", b64))
        .unwrap();
    let _ = f1.build_graphic((0, 0), &mut store).unwrap();

    let mut ctrl = KittyParser::new();
    ctrl.parse_chunk("a=a,i=7,v=1;").unwrap();
    let _ = ctrl.build_graphic((0, 0), &mut store).unwrap();
    let anim = store.get_animation(7).unwrap();
    assert_eq!(anim.loop_count, 0, "v=1 must mean infinite (loop_count=0)");
}

#[test]
fn test_build_graphic_animation_control_no_state_only_loops() {
    // Animation control with only v= (no s=) should still set loop_count.
    let pixels: Vec<u8> = vec![255, 0, 0, 255];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixels);
    let mut store = GraphicsStore::new();
    let mut f1 = KittyParser::new();
    f1.parse_chunk(&format!("a=f,f=32,s=1,v=1,i=8,r=1;{}", b64))
        .unwrap();
    let _ = f1.build_graphic((0, 0), &mut store).unwrap();

    let mut ctrl = KittyParser::new();
    ctrl.parse_chunk("a=a,i=8,v=3;").unwrap(); // no s=
    let result = ctrl.build_graphic((0, 0), &mut store).unwrap();
    assert!(matches!(result, KittyGraphicResult::None));
    // v=3 -> loop_count = 2
    let anim = store.get_animation(8).unwrap();
    assert_eq!(anim.loop_count, 2);
}

// --- load_file_data additional security/edge paths ---

#[test]
fn test_load_file_data_invalid_utf8_returns_error() {
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    // 0xFF is invalid as leading byte in UTF-8.
    let result = parser.load_file_data(&[0xFF, 0xFE, 0xFD]);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Invalid UTF-8") || msg.contains("file path"));
}

#[test]
fn test_load_file_data_path_is_directory_returns_error() {
    // A directory exists and is not a regular file.
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    parser.allow_file_media = FileMediaMode::All;
    let dir = std::env::temp_dir(); // guaranteed to exist and be a dir
    let result = parser.load_file_data(dir.to_string_lossy().as_bytes());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not a file"));
}

#[test]
fn test_load_file_data_directory_traversal_in_middle_of_path() {
    // ".." anywhere in the path must be rejected.
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    let result = parser.load_file_data(b"/tmp/foo/../bar.png");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Directory traversal"));
}

#[test]
fn test_load_file_data_parent_dir_component_is_rejected() {
    // A real `..` component is rejected even when the path also
    // contains a benign ".."-inside-filename segment.
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    let result = parser.load_file_data(b"a/../b.png");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Directory traversal"));
}

#[test]
fn test_load_file_data_double_dot_inside_filename_is_accepted() {
    // "my..notes.png" contains ".." as a substring but has no
    // parent-dir component — it must remain loadable.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 4]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("my..notes.png");
    std::fs::write(&path, &png).unwrap();

    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File;
    parser.allow_file_media = FileMediaMode::All;
    let (data, _) = parser
        .load_file_data(path.to_string_lossy().as_bytes())
        .expect("a '..' substring inside a filename must be readable");
    assert_eq!(data, png);
}

#[test]
fn test_load_file_data_temp_file_is_deleted_after_decode() {
    use std::io::Write;

    // Write a real PNG to a spec-named temp file, then close the handle.
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 8, 7, 6]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let mut tf = tempfile::Builder::new()
        .prefix("tty-graphics-protocol-")
        .tempfile()
        .unwrap();
    tf.write_all(&png).unwrap();
    let (file, path) = tf.keep().expect("keep temp file");
    drop(file); // close OS handle so removal can succeed

    let path_str = path.to_string_lossy().into_owned();
    assert!(path.exists());

    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let (data, pending) = parser.load_file_data(path_str.as_bytes()).unwrap();
    assert_eq!(data, png);
    // Deletion is deferred to decode: the bare read keeps the file.
    assert!(path.exists(), "bare read must not delete the t=t file");
    let pending = pending.expect("t=t read returns a pending delete path");
    parser
        .decode_payload(pending.path.to_string_lossy().as_bytes().to_vec(), "shm")
        .expect("valid PNG decodes");
    assert!(!path.exists(), "temp file should be deleted after decode");
}

#[test]
fn test_load_file_data_valid_file_round_trip() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 1, 1, 1]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let mut tf = NamedTempFile::new().unwrap();
    tf.write_all(&png).unwrap();
    let path = tf.path().to_string_lossy().into_owned();
    // Keep tf alive so the file persists through the read.

    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::File; // NOT temp file -> should remain
    parser.allow_file_media = FileMediaMode::All;
    let (data, pending) = parser.load_file_data(path.as_bytes()).unwrap();
    assert_eq!(data, png);
    assert!(pending.is_none(), "t=f never returns a delete path");
    assert!(tf.path().exists(), "non-temp file should NOT be deleted");
}

// --- SEC-101 regression suite: the t=t file-media gate ---

/// Helper: write bytes to a spec-named file inside the real temp dir.
fn write_gated_temp_file(bytes: &[u8]) -> std::path::PathBuf {
    use std::io::Write;
    let mut f = tempfile::Builder::new()
        .prefix("tty-graphics-protocol-")
        .tempfile()
        .expect("create gated temp file");
    f.write_all(bytes).expect("write gated temp file");
    let (_, path) = f.keep().expect("keep gated temp file");
    path
}

fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([7, 7, 7, 7]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

/// SEC-101 criterion 1: a `t=t` payload naming an absolute path
/// outside the allowed temp roots is refused AND the file survives.
#[test]
fn tt_absolute_path_outside_temp_roots_refused_and_kept() {
    // A directory outside every temp root: the crate's target/ dir
    // (cwd for cargo test is the crate root).
    let dir = std::env::current_dir()
        .expect("cwd")
        .join("target")
        .join("sec101-fixtures");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join("tty-graphics-protocol-innocent.png");
    std::fs::write(&path, tiny_png()).expect("write fixture");
    // The fixture must genuinely live outside every allowed root for
    // this test to mean anything; if the whole checkout is inside a
    // temp dir, say so loudly instead of passing vacuously.
    let canonical = path.canonicalize().unwrap();
    assert!(
        !super::is_under_allowed_temp_root(&canonical),
        "test setup broken: fixture {:?} is inside a temp root",
        canonical
    );

    let mut parser = KittyParser::new(); // default TempOnly
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let result = parser.decode_payload(path.to_string_lossy().as_bytes().to_vec(), "shm");
    let msg = result
        .expect_err("path outside temp roots must be refused")
        .to_string();
    assert!(
        msg.contains("outside the allowed temp"),
        "unexpected error: {}",
        msg
    );
    assert!(path.exists(), "refused payload must NOT delete the file");

    let _ = std::fs::remove_file(&path); // clean up
    let _ = std::fs::remove_dir(&dir);
}

/// SEC-101 criterion 2: a gated temp file whose bytes are not a
/// decodable image is refused and never deleted — deletion happens
/// only after a successful decode.
#[test]
fn tt_non_image_gated_file_survives_decode() {
    let path = write_gated_temp_file(b"definitely not an image");

    let mut parser = KittyParser::new(); // default TempOnly
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let result = parser.decode_payload(path.to_string_lossy().as_bytes().to_vec(), "shm");
    assert!(result.is_err(), "garbage bytes must not decode");
    assert!(path.exists(), "undecodable t=t payload must NOT be deleted");

    let _ = std::fs::remove_file(&path);
}

/// The gated happy path still works: a valid PNG in a spec-named temp
/// file loads under the default mode and deletes after decoding.
#[test]
fn tt_valid_gated_png_loads_and_deletes_under_default_mode() {
    let path = write_gated_temp_file(&tiny_png());

    let mut parser = KittyParser::new(); // default TempOnly
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let (w, h, px) = parser
        .decode_payload(path.to_string_lossy().as_bytes().to_vec(), "shm")
        .expect("gated temp PNG must load under TempOnly");
    assert_eq!((w, h), (1, 1));
    assert_eq!(px.len(), 4);
    assert!(!path.exists(), "decoded gated file is deleted");
}

/// `Off` refuses both file media outright; `All` opens `t=f` back up.
#[test]
fn file_media_mode_off_refuses_both_media() {
    let path = write_gated_temp_file(&tiny_png());

    let mut parser = KittyParser::new();
    parser.allow_file_media = FileMediaMode::Off;
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let err = parser
        .decode_payload(path.to_string_lossy().as_bytes().to_vec(), "shm")
        .expect_err("Off must refuse t=t");
    assert!(err.to_string().contains("disabled"));
    assert!(path.exists(), "Off must not delete");

    let mut parser = KittyParser::new();
    parser.allow_file_media = FileMediaMode::Off;
    parser.medium = KittyMedium::File;
    let err = parser
        .load_file_data(path.to_string_lossy().as_bytes())
        .expect_err("Off must refuse t=f");
    assert!(err.to_string().contains("t=f"));

    let _ = std::fs::remove_file(&path);
}

/// The gate is reachable from processed PTY bytes through
/// `Terminal::process` — the path the audit reproduced the
/// vulnerability on. A gated temp file loads and deletes by default;
/// with the mode switched off on the terminal the same APC is refused
/// and the file survives.
#[test]
fn terminal_applies_the_file_media_gate_over_apc() {
    use crate::terminal::Terminal;
    use crate::terminal::TerminalEvent;
    use base64::Engine;

    let path = write_gated_temp_file(&tiny_png());
    let path_b64 =
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(path.to_string_lossy().as_bytes());
    let apc = format!("\x1b_Ga=T,f=100,t=t;{}\x1b\\", path_b64);

    // Default (TempOnly): the graphic lands and the file is deleted.
    let mut term = Terminal::new(10, 5);
    term.process(apc.as_bytes());
    let events = term.poll_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TerminalEvent::GraphicsAdded(_))),
        "gated t=t must produce a graphic by default"
    );
    assert!(!path.exists(), "rendering terminal deletes the temp file");

    // Off: refused, no event, file survives.
    let path = write_gated_temp_file(&tiny_png());
    let path_b64 =
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(path.to_string_lossy().as_bytes());
    let apc = format!("\x1b_Ga=T,f=100,t=t;{}\x1b\\", path_b64);
    let mut term = Terminal::new(10, 5);
    term.set_allow_file_media(crate::graphics::kitty::FileMediaMode::Off);
    term.process(apc.as_bytes());
    let events = term.poll_events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, TerminalEvent::GraphicsAdded(_))),
        "Off must refuse the t=t APC"
    );
    assert!(path.exists(), "refused APC must leave the file alone");

    let _ = std::fs::remove_file(&path);
}

// --- SEC-130 regression suite: t=t check-to-use identity ---

/// Read a gated `t=t` file named `name` inside a fresh temp-root dir
/// and return the dir guard, the file's path and its pending delete.
#[cfg(unix)]
fn read_gated_file_in_dir(name: &str) -> (tempfile::TempDir, std::path::PathBuf, PendingDelete) {
    let dir = tempfile::Builder::new()
        .prefix("sec130-")
        .tempdir()
        .expect("create temp dir");
    let path = dir.path().join(name);
    std::fs::write(&path, tiny_png()).expect("write gated file");
    let mut parser = KittyParser::new();
    parser.medium = KittyMedium::TempFile;
    parser.format = KittyFormat::Png;
    let (_, pending) = parser
        .load_file_data(path.to_string_lossy().as_bytes())
        .expect("gated temp file loads");
    let pending = pending.expect("non-retained t=t read returns a pending delete");
    (dir, path, pending)
}

/// A file swapped in at the same path after the read is not the file
/// that was read, so the delete leaves it alone.
#[cfg(unix)]
#[test]
fn temp_file_delete_skips_a_swapped_file() {
    let (dir, path, pending) = read_gated_file_in_dir("tty-graphics-protocol-swap.png");
    std::fs::rename(&path, dir.path().join("moved-away.png")).expect("move original");
    std::fs::write(&path, b"a different file").expect("write replacement");

    assert!(
        !delete_if_same_file(&pending),
        "swapped file must not be deleted"
    );
    assert!(path.exists(), "the replacement survives");
    assert_eq!(std::fs::read(&path).unwrap(), b"a different file");
}

/// The pending delete removes the file that was read, addressed by
/// its canonical path (macOS `/var` → `/private/var` included).
#[cfg(unix)]
#[test]
fn temp_file_delete_removes_the_file_that_was_read() {
    let (_dir, path, pending) = read_gated_file_in_dir("tty-graphics-protocol-same.png");
    assert_eq!(
        pending.path,
        path.canonicalize().unwrap(),
        "delete targets the canonical path"
    );

    assert!(
        delete_if_same_file(&pending),
        "the file that was read is deleted"
    );
    assert!(!pending.path.exists());
    assert!(!path.exists());
}

/// A symlinked parent directory escapes `O_NOFOLLOW`, which guards
/// only the final component. The handle's kernel path exposes where
/// the open really landed, and the gate refuses it: this is the check
/// that catches a parent swapped between canonicalize and open.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn opened_path_exposes_a_symlinked_parent_outside_temp_roots() {
    let outside = std::env::current_dir()
        .expect("cwd")
        .join("target")
        .join("sec130-fixtures");
    std::fs::create_dir_all(&outside).expect("create fixture dir");
    let target = outside.join("tty-graphics-protocol-outside.png");
    std::fs::write(&target, tiny_png()).expect("write fixture");
    let canonical_target = target.canonicalize().unwrap();
    assert!(
        !super::is_under_allowed_temp_root(&canonical_target),
        "test setup broken: fixture {:?} is inside a temp root",
        canonical_target
    );

    let dir = tempfile::Builder::new()
        .prefix("sec130-link-")
        .tempdir()
        .expect("create temp dir");
    let link = dir.path().join("swapped-parent");
    std::os::unix::fs::symlink(&outside, &link).expect("symlink parent dir");
    let via_link = link.join("tty-graphics-protocol-outside.png");

    let file = open_no_follow(&via_link).expect("O_NOFOLLOW follows a parent symlink");
    let resolved = opened_path(&file)
        .expect("platform has an fd path lookup")
        .expect("fd path lookup succeeds");
    assert_eq!(resolved, canonical_target, "kernel reports the real file");
    let err = check_temp_file_gate(&resolved).expect_err("gate refuses the real path");
    assert!(
        err.to_string().contains("outside the allowed temp"),
        "{}",
        err
    );

    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir(&outside);
}

// --- KittyGraphicResult Debug round-trip (cheap enum coverage) ---

#[test]
fn test_kitty_graphic_result_debug_repr() {
    // Each variant's Debug impl should not panic; format! exercises it.
    let none = KittyGraphicResult::None;
    let s = format!("{:?}", none);
    assert!(s.contains("None"));

    let virt = KittyGraphicResult::VirtualPlacement {
        image_id: 1,
        placement_id: 2,
        position: (0, 0),
        cols: 1,
        rows: 1,
    };
    let s = format!("{:?}", virt);
    assert!(s.contains("VirtualPlacement"));
    assert!(s.contains("image_id"));
}

// --- decompress_zlib: empty input ---

#[test]
fn test_decompress_zlib_empty_input_succeeds_with_empty_output() {
    // ZlibDecoder treats empty input as a valid empty stream.
    let result = KittyParser::decompress_zlib(&[], MAX_KITTY_DECOMPRESSED_BYTES);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_decompress_zlib_empty_valid_stream() {
    // A valid zlib stream that decompresses to zero bytes.
    // RFC 1950 wrapper around deflate of empty stored block.
    let empty_zlib: [u8; 8] = [0x78, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
    let result = KittyParser::decompress_zlib(&empty_zlib, MAX_KITTY_DECOMPRESSED_BYTES);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

// --- is_compressed() reflects compression flag ---

#[test]
fn test_is_compressed_reflects_compression_field() {
    let mut parser = KittyParser::new();
    assert!(!parser.is_compressed());
    parser.compression = KittyCompression::Zlib;
    assert!(parser.is_compressed());
}

// --- build_placement: z_index populated when set ---

#[test]
fn test_build_placement_negative_z_index() {
    let mut parser = KittyParser::new();
    parser.z_index = Some(-100);
    let placement = parser.build_placement();
    assert_eq!(placement.z_index, -100);
}

#[test]
fn test_build_placement_all_fields_set() {
    let mut parser = KittyParser::new();
    parser.columns = Some(6);
    parser.rows = Some(4);
    parser.z_index = Some(2);
    parser.x_offset = Some(7);
    parser.y_offset = Some(9);
    let placement = parser.build_placement();
    assert_eq!(placement.columns, Some(6));
    assert_eq!(placement.rows, Some(4));
    assert_eq!(placement.z_index, 2);
    assert_eq!(placement.x_offset, 7);
    assert_eq!(placement.y_offset, 9);
}

// --- KittyAction/KittyFormat/KittyMedium/KittyCompression defaults & Debug ---

#[test]
fn test_kitty_action_default_is_transmit() {
    assert_eq!(KittyAction::default(), KittyAction::Transmit);
}

#[test]
fn test_kitty_format_default_is_rgba() {
    assert_eq!(KittyFormat::default(), KittyFormat::Rgba);
}

#[test]
fn test_kitty_medium_default_is_direct() {
    assert_eq!(KittyMedium::default(), KittyMedium::Direct);
}

#[test]
fn test_kitty_compression_default_is_none() {
    assert_eq!(KittyCompression::default(), KittyCompression::None);
}

#[test]
fn test_kitty_action_all_chars_covered() {
    // Complete coverage of from_char for every documented action.
    assert_eq!(KittyAction::from_char('f'), Some(KittyAction::Frame));
    assert_eq!(
        KittyAction::from_char('a'),
        Some(KittyAction::AnimationControl)
    );
    // Empty string -> next() is None -> action is left unchanged.
    let mut parser = KittyParser::new();
    parser.action = KittyAction::Query;
    let _ = parser.parse_chunk("a=;");
    assert_eq!(parser.action, KittyAction::Query);
}

#[test]
fn test_kitty_format_from_code_uncovered_codes() {
    // Confirm a couple of additional edge cases.
    assert_eq!(KittyFormat::from_code(1), None);
    assert_eq!(KittyFormat::from_code(u32::MAX), None);
}

#[test]
fn test_kitty_compression_from_char_only_z() {
    // Every non-'z' char returns None.
    for c in ['a', 'Z', '0', ' ', '\0'] {
        assert_eq!(KittyCompression::from_char(c), None);
    }
}

#[test]
fn test_kitty_medium_from_char_invalid_chars() {
    assert_eq!(KittyMedium::from_char('D'), None); // uppercase not valid
    assert_eq!(KittyMedium::from_char('F'), None);
    assert_eq!(KittyMedium::from_char(' '), None);
}
