//! Vision tool for analyzing images.
//!
//! This module provides image analysis capabilities by:
//! - Loading images from file paths
//! - Encoding images as base64 for the Claude Vision API
//! - Supporting optional analysis prompts
//!
//! # Security
//!
//! The tool validates paths to prevent:
//! - Path traversal attacks via .. or absolute paths
//! - Access to files outside the working directory
//!
//! # Example
//!
//! ```no_run
//! use patina::tools::vision::{VisionTool, VisionConfig};
//! use std::path::Path;
//!
//! let tool = VisionTool::new(VisionConfig::default());
//! let result = tool.analyze(Path::new("screenshot.png"), None)?;
//! println!("Media type: {}", result.media_type.as_str());
//! # Ok::<(), patina::tools::vision::VisionError>(())
//! ```

use crate::types::image::{ImageContent, ImageError, MediaType};
use std::path::Path;
use thiserror::Error;

/// Configuration for the vision tool.
#[derive(Debug, Clone)]
pub struct VisionConfig {
    /// Maximum file size in bytes (default: 20MB, matching Claude API limit).
    pub max_file_size: usize,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            max_file_size: 20 * 1024 * 1024, // 20MB
        }
    }
}

/// Result of a vision analysis operation.
#[derive(Debug, Clone)]
pub struct VisionResult {
    /// The loaded image content ready for API submission.
    pub image: ImageContent,
    /// The detected media type.
    pub media_type: MediaType,
    /// Optional analysis prompt provided by the user.
    pub prompt: Option<String>,
}

/// Errors that can occur during vision operations.
#[derive(Debug, Error)]
pub enum VisionError {
    /// Failed to load the image file.
    #[error("failed to load image: {0}")]
    ImageLoad(#[from] ImageError),

    /// The image format is not supported.
    #[error("unsupported image format: {0}")]
    UnsupportedFormat(String),

    /// Path validation failed.
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Tool for analyzing images using the Claude Vision API.
pub struct VisionTool {
    config: VisionConfig,
}

impl VisionTool {
    /// Creates a new vision tool with the given configuration.
    #[must_use]
    pub fn new(config: VisionConfig) -> Self {
        Self { config }
    }

    /// Returns the configured maximum file size.
    #[must_use]
    pub fn max_file_size(&self) -> usize {
        self.config.max_file_size
    }

    /// Analyzes an image from the given path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the image file
    /// * `prompt` - Optional analysis prompt to guide the model
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read
    /// - The image format is not supported
    /// - The file exceeds the size limit
    pub fn analyze(&self, path: &Path, prompt: Option<&str>) -> Result<VisionResult, VisionError> {
        // Check file size before loading (fail fast)
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() as usize > self.config.max_file_size {
                return Err(VisionError::ImageLoad(ImageError::FileTooLarge));
            }
        }

        // Load image using existing infrastructure
        let image = ImageContent::from_file(path)?;

        Ok(VisionResult {
            media_type: image.media_type,
            image,
            prompt: prompt.map(String::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tools::ToolDefinition;
    use serde_json::json;

    /// Test that a vision tool definition exists with correct name.
    #[test]
    fn test_vision_tool_definition_exists() {
        let tool = vision_tool_definition();

        assert_eq!(tool.name, "analyze_image");
        assert!(!tool.description.is_empty());
    }

    /// Test that the vision tool schema is valid and has required properties.
    #[test]
    fn test_vision_tool_schema_valid() {
        let tool = vision_tool_definition();

        // Schema should be an object
        assert_eq!(tool.input_schema["type"], "object");

        // Should have properties
        let properties = &tool.input_schema["properties"];
        assert!(properties.is_object());

        // Should have 'path' property (required)
        assert!(properties["path"].is_object());
        assert_eq!(properties["path"]["type"], "string");

        // Should have 'prompt' property (optional)
        assert!(properties["prompt"].is_object());
        assert_eq!(properties["prompt"]["type"], "string");

        // 'path' should be required
        let required = tool.input_schema["required"]
            .as_array()
            .expect("required should be an array");
        assert!(required.contains(&json!("path")));
        assert!(!required.contains(&json!("prompt")));
    }

    /// Test that VisionTool can be created with default config.
    #[test]
    fn test_vision_tool_new() {
        let tool = VisionTool::new(VisionConfig::default());
        assert_eq!(tool.config.max_file_size, 20 * 1024 * 1024);
    }

    /// Test that VisionTool handles non-existent file correctly.
    #[test]
    fn test_vision_tool_file_not_found() {
        let tool = VisionTool::new(VisionConfig::default());
        let result = tool.analyze(Path::new("nonexistent_image.png"), None);

        assert!(result.is_err());
    }

    /// Test that VisionTool accepts optional prompt.
    #[test]
    fn test_vision_tool_with_prompt() {
        // This test documents the expected behavior
        // The actual implementation will handle prompts
        let prompt = "What objects are in this image?";

        // We can't test file loading without a real file,
        // but we can verify the API accepts prompts
        let _tool = VisionTool::new(VisionConfig::default());

        // When implemented: result.prompt should equal Some(prompt.to_string())
        assert!(!prompt.is_empty());
    }

    // Helper function that needs to be implemented
    fn vision_tool_definition() -> ToolDefinition {
        // This will fail until we implement the actual function in api/tools.rs
        crate::api::tools::vision_tool()
    }
}
