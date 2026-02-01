//! Integration tests for vision (image input) support.
//!
//! These tests verify:
//! - Image content block serialization matches Claude API format
//! - Vision model routing logic works correctly
//! - Token estimation for images is accurate

use patina::api::multi_model::{contains_images, select_model_for_content};
use patina::api::tokens::{estimate_image_tokens, DEFAULT_IMAGE_TOKENS};
use patina::types::image::{ImageContent, ImageSource, MediaType};
use patina::types::{ApiMessageV2, ContentBlock, MessageContent};

// ============================================================================
// Image Content Serialization Tests
// ============================================================================

/// Verifies base64 image content blocks serialize to the correct Claude API format.
#[test]
fn test_vision_request_serialization_base64() {
    let source = ImageSource::Base64 {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
    };

    let block = ContentBlock::image(source);
    let json = serde_json::to_value(&block).expect("Serialization should succeed");

    // Verify the structure matches Claude API format
    assert_eq!(json["type"], "image", "Block type should be 'image'");
    assert!(json["source"].is_object(), "Should have source object");
    assert_eq!(
        json["source"]["type"], "base64",
        "Source type should be 'base64'"
    );
    assert_eq!(
        json["source"]["media_type"], "image/png",
        "Media type should be preserved"
    );
    assert!(
        !json["source"]["data"].as_str().unwrap().is_empty(),
        "Data should be present"
    );
}

/// Verifies URL image content blocks serialize to the correct Claude API format.
#[test]
fn test_vision_request_serialization_url() {
    let source = ImageSource::Url {
        url: "https://example.com/image.png".to_string(),
    };

    let block = ContentBlock::image(source);
    let json = serde_json::to_value(&block).expect("Serialization should succeed");

    assert_eq!(json["type"], "image", "Block type should be 'image'");
    assert_eq!(json["source"]["type"], "url", "Source type should be 'url'");
    assert_eq!(
        json["source"]["url"], "https://example.com/image.png",
        "URL should be preserved"
    );
}

/// Verifies messages with mixed text and image content serialize correctly.
#[test]
fn test_vision_request_serialization_mixed_content() {
    let source = ImageSource::Base64 {
        media_type: "image/jpeg".to_string(),
        data: "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDA".to_string(),
    };

    let message = ApiMessageV2::user_with_content(MessageContent::blocks(vec![
        ContentBlock::text("What do you see in this image?"),
        ContentBlock::image(source),
    ]));

    let json = serde_json::to_value(&message).expect("Serialization should succeed");

    assert_eq!(json["role"], "user", "Role should be 'user'");

    let content = json["content"].as_array().expect("Content should be array");
    assert_eq!(content.len(), 2, "Should have two content blocks");
    assert_eq!(content[0]["type"], "text", "First block should be text");
    assert_eq!(content[1]["type"], "image", "Second block should be image");
}

/// Verifies image blocks can be deserialized from Claude API format.
#[test]
fn test_vision_response_deserialization() {
    // This is the format Claude returns for image input echoes
    let json = r#"{
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "iVBORw0KGgo="
        }
    }"#;

    let block: ContentBlock = serde_json::from_str(json).expect("Deserialization should succeed");

    assert!(block.is_image(), "Should be recognized as image block");

    if let ContentBlock::Image { source } = block {
        match source {
            ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data, "iVBORw0KGgo=");
            }
            _ => panic!("Expected Base64 source"),
        }
    } else {
        panic!("Expected Image block");
    }
}

// ============================================================================
// Vision Model Routing Tests
// ============================================================================

/// Verifies model routing uses default model when no images are present.
#[test]
fn test_vision_model_routing_no_images() {
    let messages = vec![
        ApiMessageV2::user("Hello, how are you?"),
        ApiMessageV2::assistant("I'm doing well, thank you!"),
        ApiMessageV2::user("Can you help me with something?"),
    ];

    let model = select_model_for_content(&messages, "claude-sonnet-4", Some("claude-opus-4"));

    assert_eq!(
        model, "claude-sonnet-4",
        "Should use default model when no images"
    );
}

/// Verifies model routing switches to vision model when images are present.
#[test]
fn test_vision_model_routing_with_images() {
    let source = ImageSource::Base64 {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgo=".to_string(),
    };

    let messages = vec![
        ApiMessageV2::user("Hello"),
        ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::text("What's in this image?"),
            ContentBlock::image(source),
        ])),
    ];

    let model = select_model_for_content(&messages, "claude-sonnet-4", Some("claude-opus-4"));

    assert_eq!(
        model, "claude-opus-4",
        "Should switch to vision model when images present"
    );
}

/// Verifies model routing falls back to default when vision model not configured.
#[test]
fn test_vision_model_routing_no_vision_model_configured() {
    let source = ImageSource::Url {
        url: "https://example.com/photo.jpg".to_string(),
    };

    let messages = vec![ApiMessageV2::user_with_content(MessageContent::blocks(vec![
        ContentBlock::image(source),
    ]))];

    let model = select_model_for_content(&messages, "claude-sonnet-4", None);

    assert_eq!(
        model, "claude-sonnet-4",
        "Should use default model when no vision model configured"
    );
}

/// Verifies image detection works with images in different message positions.
#[test]
fn test_vision_model_routing_image_in_middle() {
    let source = ImageSource::Base64 {
        media_type: "image/gif".to_string(),
        data: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7".to_string(),
    };

    let messages = vec![
        ApiMessageV2::user("Start of conversation"),
        ApiMessageV2::assistant("Hello!"),
        ApiMessageV2::user_with_content(MessageContent::blocks(vec![
            ContentBlock::text("Here's an image:"),
            ContentBlock::image(source),
        ])),
        ApiMessageV2::assistant("I see the image."),
        ApiMessageV2::user("Thanks!"),
    ];

    // Image is in the middle, not at the end
    assert!(
        contains_images(&messages),
        "Should detect images anywhere in conversation"
    );

    let model = select_model_for_content(&messages, "default", Some("vision"));
    assert_eq!(model, "vision", "Should route to vision model");
}

// ============================================================================
// Token Estimation Tests
// ============================================================================

/// Verifies Claude's image token formula: (width * height) / 750.
#[test]
fn test_vision_token_estimation_formula() {
    // Test cases based on Claude's documented formula
    let test_cases = [
        // (width, height, expected minimum, expected maximum)
        (1024, 1024, 1398, 1400), // Standard square ~1399
        (800, 600, 639, 641),     // Landscape
        (600, 800, 639, 641),     // Portrait
        (100, 100, 13, 15),       // Small thumbnail
        (1920, 1080, 2764, 2766), // Full HD
        (3840, 2160, 11058, 11062), // 4K
    ];

    for (width, height, min_expected, max_expected) in test_cases {
        let tokens = estimate_image_tokens(width, height);
        assert!(
            tokens >= min_expected && tokens <= max_expected,
            "Image {}x{} should be {}-{} tokens, got {}",
            width,
            height,
            min_expected,
            max_expected,
            tokens
        );
    }
}

/// Verifies token estimation handles edge cases correctly.
#[test]
fn test_vision_token_estimation_edge_cases() {
    // Single pixel
    let tokens = estimate_image_tokens(1, 1);
    assert_eq!(tokens, 1, "1x1 image should be 1 token (minimum)");

    // Zero dimensions
    let tokens = estimate_image_tokens(0, 100);
    assert_eq!(tokens, 0, "0 width should result in 0 tokens");
    let tokens = estimate_image_tokens(100, 0);
    assert_eq!(tokens, 0, "0 height should result in 0 tokens");
}

/// Verifies the default image token constant is reasonable.
#[test]
fn test_vision_token_estimation_default_constant() {
    // DEFAULT_IMAGE_TOKENS should be close to a 1024x1024 image
    let typical_image_tokens = estimate_image_tokens(1024, 1024);
    let difference = (DEFAULT_IMAGE_TOKENS as i64 - typical_image_tokens as i64).abs();
    assert!(
        difference < 200,
        "Default {} should be close to typical 1024x1024 image tokens {}",
        DEFAULT_IMAGE_TOKENS,
        typical_image_tokens
    );

    // Verify it's in a reasonable range by checking against known sizes
    let small_image = estimate_image_tokens(512, 512);
    let large_image = estimate_image_tokens(2048, 2048);
    assert!(
        DEFAULT_IMAGE_TOKENS > small_image && DEFAULT_IMAGE_TOKENS < large_image,
        "Default should be between small (512x512={}) and large (2048x2048={}) images",
        small_image,
        large_image
    );
}

// ============================================================================
// ImageContent Type Tests
// ============================================================================

/// Verifies ImageContent can be created from bytes with correct encoding.
#[test]
fn test_image_content_from_bytes() {
    // Minimal PNG header bytes
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    ];

    let result = ImageContent::from_bytes(&png_bytes, MediaType::Png);
    assert!(result.is_ok(), "Should create ImageContent from bytes");

    let content = result.unwrap();
    assert_eq!(content.media_type, MediaType::Png);
    match content.source {
        ImageSource::Base64 { media_type, data } => {
            assert_eq!(media_type, "image/png");
            assert!(!data.is_empty(), "Base64 data should not be empty");
        }
        _ => panic!("Expected Base64 source"),
    }
}

/// Verifies ImageContent URL creation with different formats.
#[test]
fn test_image_content_from_url() {
    let test_urls = [
        ("https://example.com/image.png", MediaType::Png),
        ("https://example.com/photo.jpg", MediaType::Jpeg),
        ("https://example.com/photo.jpeg", MediaType::Jpeg),
        ("https://example.com/animation.gif", MediaType::Gif),
        ("https://example.com/modern.webp", MediaType::Webp),
    ];

    for (url, expected_type) in test_urls {
        let result = ImageContent::from_url(url);
        assert!(result.is_ok(), "Should create ImageContent from URL: {}", url);

        let content = result.unwrap();
        assert_eq!(
            content.media_type, expected_type,
            "Media type should match for URL: {}",
            url
        );
    }
}

/// Verifies URL creation fails for unsupported formats.
#[test]
fn test_image_content_from_url_unsupported() {
    let unsupported_urls = [
        "https://example.com/document.pdf",
        "https://example.com/image.bmp",
        "https://example.com/image.tiff",
        "https://example.com/no-extension",
    ];

    for url in unsupported_urls {
        let result = ImageContent::from_url(url);
        assert!(
            result.is_err(),
            "Should fail for unsupported format: {}",
            url
        );
    }
}

/// Verifies MediaType string conversion.
#[test]
fn test_media_type_as_str() {
    assert_eq!(MediaType::Png.as_str(), "image/png");
    assert_eq!(MediaType::Jpeg.as_str(), "image/jpeg");
    assert_eq!(MediaType::Gif.as_str(), "image/gif");
    assert_eq!(MediaType::Webp.as_str(), "image/webp");
}

/// Verifies MediaType serialization matches Claude API format.
#[test]
fn test_media_type_serialization() {
    let types = [
        (MediaType::Png, "\"image/png\""),
        (MediaType::Jpeg, "\"image/jpeg\""),
        (MediaType::Gif, "\"image/gif\""),
        (MediaType::Webp, "\"image/webp\""),
    ];

    for (media_type, expected_json) in types {
        let json = serde_json::to_string(&media_type).expect("Serialization should succeed");
        assert_eq!(json, expected_json, "MediaType should serialize correctly");
    }
}
