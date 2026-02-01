//! Unit tests for the Image types.
//!
//! These tests verify image content handling, MIME type detection,
//! base64 encoding, and serialization for the Claude Vision API.

use base64::{engine::general_purpose::STANDARD, Engine};
use patina::types::image::{ImageContent, ImageSource, MediaType};
use std::path::Path;

// ============================================================================
// Image Loading Tests - Base64 Encoding
// ============================================================================

#[test]
fn test_image_from_file_path_encodes_base64() {
    // Create a minimal valid PNG file for testing
    let png_bytes = create_minimal_png();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("test.png");
    std::fs::write(&file_path, &png_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(
        result.is_ok(),
        "Expected successful image load: {:?}",
        result
    );
    let image = result.unwrap();

    // Verify source is base64
    match &image.source {
        ImageSource::Base64 { data, .. } => {
            // Verify base64 decodes back to original bytes
            let decoded = STANDARD.decode(data).expect("Base64 should be valid");
            assert_eq!(decoded, png_bytes, "Decoded data should match original");
        }
        ImageSource::Url { .. } => panic!("Expected Base64 source, got URL"),
    }

    // Verify media type was detected correctly
    assert_eq!(image.media_type, MediaType::Png);
}

#[test]
fn test_image_from_bytes_encodes_base64() {
    let png_bytes = create_minimal_png();

    let result = ImageContent::from_bytes(&png_bytes, MediaType::Png);

    assert!(
        result.is_ok(),
        "Expected successful image creation: {:?}",
        result
    );
    let image = result.unwrap();

    match &image.source {
        ImageSource::Base64 { data, .. } => {
            // Verify base64 encoding is correct
            let decoded = STANDARD.decode(data).expect("Base64 should be valid");
            assert_eq!(decoded, png_bytes);
        }
        ImageSource::Url { .. } => panic!("Expected Base64 source, got URL"),
    }

    assert_eq!(image.media_type, MediaType::Png);
}

// ============================================================================
// MIME Type Detection Tests
// ============================================================================

#[test]
fn test_image_detects_mime_type_png() {
    let png_bytes = create_minimal_png();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("image.png");
    std::fs::write(&file_path, &png_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Png);
}

#[test]
fn test_image_detects_mime_type_jpeg() {
    let jpeg_bytes = create_minimal_jpeg();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("image.jpg");
    std::fs::write(&file_path, &jpeg_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Jpeg);
}

#[test]
fn test_image_detects_mime_type_gif() {
    let gif_bytes = create_minimal_gif();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("image.gif");
    std::fs::write(&file_path, &gif_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Gif);
}

#[test]
fn test_image_detects_mime_type_webp() {
    let webp_bytes = create_minimal_webp();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("image.webp");
    std::fs::write(&file_path, &webp_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Webp);
}

// ============================================================================
// URL Source Tests
// ============================================================================

#[test]
fn test_image_from_url_creates_url_source() {
    let url = "https://example.com/image.png";

    let result = ImageContent::from_url(url);

    assert!(result.is_ok(), "Expected successful URL image creation");
    let image = result.unwrap();

    match &image.source {
        ImageSource::Url { url: stored_url } => {
            assert_eq!(stored_url, url);
        }
        ImageSource::Base64 { .. } => panic!("Expected URL source, got Base64"),
    }

    // Media type should be inferred from URL extension
    assert_eq!(image.media_type, MediaType::Png);
}

#[test]
fn test_image_from_url_infers_jpeg_media_type() {
    let url = "https://example.com/photo.jpeg";

    let result = ImageContent::from_url(url);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Jpeg);
}

#[test]
fn test_image_from_url_infers_jpg_media_type() {
    let url = "https://example.com/photo.jpg";

    let result = ImageContent::from_url(url);

    assert!(result.is_ok());
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Jpeg);
}

// ============================================================================
// Validation Tests
// ============================================================================

#[test]
fn test_image_rejects_unsupported_format() {
    // BMP is not supported by Claude Vision API
    let bmp_bytes = create_minimal_bmp();
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("image.bmp");
    std::fs::write(&file_path, &bmp_bytes).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_err(), "Expected error for unsupported format");
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unsupported")
            || err.to_string().to_lowercase().contains("format"),
        "Error should mention unsupported format: {err}"
    );
}

#[test]
fn test_image_rejects_oversized_file() {
    // Claude Vision API limit is 20MB per image
    let large_bytes = vec![0u8; 21 * 1024 * 1024]; // 21MB
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = temp_dir.path().join("large.png");

    // Write with PNG header to pass format detection
    let mut data = create_minimal_png();
    data.extend_from_slice(&large_bytes);
    std::fs::write(&file_path, &data).expect("Failed to write test file");

    let result = ImageContent::from_file(&file_path);

    assert!(result.is_err(), "Expected error for oversized file");
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("size")
            || err.to_string().to_lowercase().contains("large")
            || err.to_string().to_lowercase().contains("exceed"),
        "Error should mention size limit: {err}"
    );
}

#[test]
fn test_image_from_bytes_rejects_oversized_data() {
    let large_bytes = vec![0u8; 21 * 1024 * 1024]; // 21MB

    let result = ImageContent::from_bytes(&large_bytes, MediaType::Png);

    assert!(result.is_err(), "Expected error for oversized data");
}

#[test]
fn test_image_rejects_nonexistent_file() {
    let nonexistent_path = Path::new("/nonexistent/path/image.png");

    let result = ImageContent::from_file(nonexistent_path);

    assert!(result.is_err(), "Expected error for nonexistent file");
}

// ============================================================================
// Serialization Tests
// ============================================================================

#[test]
fn test_image_content_block_serialization() {
    let png_bytes = create_minimal_png();
    let image = ImageContent::from_bytes(&png_bytes, MediaType::Png).unwrap();

    let json = serde_json::to_string(&image).expect("Serialization should succeed");

    // Verify the structure matches Claude API expectations
    assert!(
        json.contains("\"type\":\"base64\""),
        "Should have base64 type in source"
    );
    assert!(
        json.contains("\"media_type\":\"image/png\""),
        "Should have media_type"
    );
    assert!(json.contains("\"data\":"), "Should have data field");
}

#[test]
fn test_image_url_source_serialization() {
    let image = ImageContent::from_url("https://example.com/test.png").unwrap();

    let json = serde_json::to_string(&image).expect("Serialization should succeed");

    assert!(
        json.contains("\"type\":\"url\""),
        "Should have url type in source"
    );
    assert!(
        json.contains("\"url\":\"https://example.com/test.png\""),
        "Should have url field"
    );
}

#[test]
fn test_image_content_deserialization() {
    let json = r#"{
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "iVBORw0KGgo="
        },
        "media_type": "image/png"
    }"#;

    let result: Result<ImageContent, _> = serde_json::from_str(json);

    assert!(
        result.is_ok(),
        "Deserialization should succeed: {:?}",
        result
    );
    let image = result.unwrap();
    assert_eq!(image.media_type, MediaType::Png);
}

#[test]
fn test_image_url_source_deserialization() {
    let json = r#"{
        "source": {
            "type": "url",
            "url": "https://example.com/image.jpeg"
        },
        "media_type": "image/jpeg"
    }"#;

    let result: Result<ImageContent, _> = serde_json::from_str(json);

    assert!(result.is_ok(), "Deserialization should succeed");
    let image = result.unwrap();

    match &image.source {
        ImageSource::Url { url } => {
            assert_eq!(url, "https://example.com/image.jpeg");
        }
        _ => panic!("Expected URL source"),
    }
}

// ============================================================================
// MediaType Tests
// ============================================================================

#[test]
fn test_media_type_as_str() {
    assert_eq!(MediaType::Png.as_str(), "image/png");
    assert_eq!(MediaType::Jpeg.as_str(), "image/jpeg");
    assert_eq!(MediaType::Gif.as_str(), "image/gif");
    assert_eq!(MediaType::Webp.as_str(), "image/webp");
}

#[test]
fn test_media_type_from_extension() {
    assert_eq!(MediaType::from_extension("png"), Some(MediaType::Png));
    assert_eq!(MediaType::from_extension("PNG"), Some(MediaType::Png));
    assert_eq!(MediaType::from_extension("jpg"), Some(MediaType::Jpeg));
    assert_eq!(MediaType::from_extension("jpeg"), Some(MediaType::Jpeg));
    assert_eq!(MediaType::from_extension("gif"), Some(MediaType::Gif));
    assert_eq!(MediaType::from_extension("webp"), Some(MediaType::Webp));
    assert_eq!(MediaType::from_extension("bmp"), None);
    assert_eq!(MediaType::from_extension("unknown"), None);
}

#[test]
fn test_media_type_serialization() {
    let png = MediaType::Png;
    let json = serde_json::to_string(&png).expect("Serialization should succeed");
    assert_eq!(json, "\"image/png\"");

    let jpeg = MediaType::Jpeg;
    let json = serde_json::to_string(&jpeg).expect("Serialization should succeed");
    assert_eq!(json, "\"image/jpeg\"");
}

#[test]
fn test_media_type_deserialization() {
    let png: MediaType = serde_json::from_str("\"image/png\"").expect("Should deserialize");
    assert_eq!(png, MediaType::Png);

    let jpeg: MediaType = serde_json::from_str("\"image/jpeg\"").expect("Should deserialize");
    assert_eq!(jpeg, MediaType::Jpeg);
}

// ============================================================================
// Helper Functions - Minimal Valid Image Bytes
// ============================================================================

/// Creates a minimal valid PNG file (1x1 pixel, red).
fn create_minimal_png() -> Vec<u8> {
    // Minimal valid PNG: 1x1 red pixel
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1 dimensions
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // bit depth, color type, etc.
        0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, // IDAT chunk
        0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // compressed data
        0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x18, 0xDD, //
        0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // IEND chunk
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82, //
    ]
}

/// Creates minimal valid JPEG bytes.
fn create_minimal_jpeg() -> Vec<u8> {
    // Minimal valid JPEG: 1x1 pixel
    vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, // SOI, APP0
        0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, // JFIF header
        0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, // DQT
        0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, //
        0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, //
        0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, //
        0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, //
        0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, //
        0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, //
        0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, //
        0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, //
        0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, // SOF0
        0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, // DHT
        0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, //
        0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, //
        0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, //
        0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, //
        0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, //
        0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, //
        0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, //
        0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, //
        0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, //
        0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, //
        0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, //
        0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, //
        0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, //
        0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, //
        0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, //
        0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, //
        0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, //
        0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, //
        0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, //
        0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, //
        0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, //
        0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, //
        0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, //
        0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, //
        0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, //
        0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, // SOS
        0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, //
        0xFB, 0xD5, 0xDB, 0x20, 0xBA, 0x4A, 0x2B, 0x4F, //
        0xFF, 0xD9, // EOI
    ]
}

/// Creates minimal valid GIF bytes.
fn create_minimal_gif() -> Vec<u8> {
    // Minimal valid GIF: 1x1 pixel
    vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x01, 0x00, 0x01, 0x00, // 1x1 dimensions
        0x80, 0x00, 0x00, // global color table flag
        0xFF, 0xFF, 0xFF, // white
        0x00, 0x00, 0x00, // black
        0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, // graphic control extension
        0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, // image descriptor
        0x00, 0x01, 0x00, 0x01, 0x00, 0x00, //
        0x02, 0x02, 0x44, 0x01, 0x00, 0x3B, // image data + trailer
    ]
}

/// Creates minimal valid WebP bytes.
fn create_minimal_webp() -> Vec<u8> {
    // Minimal valid WebP (lossless 1x1)
    vec![
        0x52, 0x49, 0x46, 0x46, // RIFF
        0x1A, 0x00, 0x00, 0x00, // file size - 8
        0x57, 0x45, 0x42, 0x50, // WEBP
        0x56, 0x50, 0x38, 0x4C, // VP8L (lossless)
        0x0D, 0x00, 0x00, 0x00, // chunk size
        0x2F, 0x00, 0x00, 0x00, // signature
        0x00, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00, //
        0x00, //
    ]
}

/// Creates minimal BMP bytes (unsupported format for testing rejection).
fn create_minimal_bmp() -> Vec<u8> {
    // BMP header
    vec![
        0x42, 0x4D, // BM signature
        0x3E, 0x00, 0x00, 0x00, // file size
        0x00, 0x00, 0x00, 0x00, // reserved
        0x36, 0x00, 0x00, 0x00, // data offset
        0x28, 0x00, 0x00, 0x00, // header size
        0x01, 0x00, 0x00, 0x00, // width
        0x01, 0x00, 0x00, 0x00, // height
        0x01, 0x00, // planes
        0x18, 0x00, // bits per pixel
        0x00, 0x00, 0x00, 0x00, // compression
        0x00, 0x00, 0x00, 0x00, // image size
        0x00, 0x00, 0x00, 0x00, // x pixels per meter
        0x00, 0x00, 0x00, 0x00, // y pixels per meter
        0x00, 0x00, 0x00, 0x00, // colors used
        0x00, 0x00, 0x00, 0x00, // important colors
        0xFF, 0x00, 0x00, 0x00, // pixel data (blue)
    ]
}
