//! Image content types for Claude Vision API.
//!
//! This module provides types for sending images to Claude through the Messages API.
//! Images can be provided either as base64-encoded data or as URLs.
//!
//! # Supported Formats
//!
//! Claude Vision API supports the following image formats:
//! - PNG (`image/png`)
//! - JPEG (`image/jpeg`)
//! - GIF (`image/gif`)
//! - WebP (`image/webp`)
//!
//! # Limitations
//!
//! - Maximum file size: 20MB per image
//! - Maximum images per request: 100
//!
//! # Example
//!
//! ```rust,ignore
//! use patina::types::image::{ImageContent, MediaType};
//! use std::path::Path;
//!
//! // Load image from file
//! let image = ImageContent::from_file(Path::new("screenshot.png"))?;
//!
//! // Or from URL
//! let image = ImageContent::from_url("https://example.com/image.png")?;
//! ```

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Maximum file size for images (20MB).
pub const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// Image content for the Claude Vision API.
///
/// Represents an image that can be sent as part of a message to Claude.
/// The image can be provided either as base64-encoded data or as a URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageContent {
    /// The source of the image (base64 data or URL).
    pub source: ImageSource,

    /// The media type of the image.
    pub media_type: MediaType,
}

impl ImageContent {
    /// Creates an `ImageContent` from a file path.
    ///
    /// The image format is detected from magic bytes, not the file extension.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the image file.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read
    /// - The file exceeds the 20MB size limit
    /// - The image format is not supported
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::types::image::ImageContent;
    /// use std::path::Path;
    ///
    /// let image = ImageContent::from_file(Path::new("photo.png"))?;
    /// ```
    pub fn from_file(path: &Path) -> Result<Self, ImageError> {
        let bytes = std::fs::read(path)?;

        // Check size limit
        if bytes.len() > MAX_IMAGE_SIZE {
            return Err(ImageError::FileTooLarge);
        }

        // Detect media type from magic bytes
        let media_type = detect_media_type(&bytes).ok_or(ImageError::UnsupportedFormat)?;

        // Encode to base64
        let data = STANDARD.encode(&bytes);

        Ok(Self {
            source: ImageSource::Base64 {
                media_type: media_type.as_str().to_string(),
                data,
            },
            media_type,
        })
    }

    /// Creates an `ImageContent` from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The raw image data.
    /// * `media_type` - The media type of the image.
    ///
    /// # Errors
    ///
    /// Returns an error if the data exceeds the 20MB size limit.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::types::image::{ImageContent, MediaType};
    ///
    /// let png_data = std::fs::read("image.png")?;
    /// let image = ImageContent::from_bytes(&png_data, MediaType::Png)?;
    /// ```
    pub fn from_bytes(bytes: &[u8], media_type: MediaType) -> Result<Self, ImageError> {
        // Check size limit
        if bytes.len() > MAX_IMAGE_SIZE {
            return Err(ImageError::FileTooLarge);
        }

        // Encode to base64
        let data = STANDARD.encode(bytes);

        Ok(Self {
            source: ImageSource::Base64 {
                media_type: media_type.as_str().to_string(),
                data,
            },
            media_type,
        })
    }

    /// Creates an `ImageContent` from a URL.
    ///
    /// The media type is inferred from the URL file extension.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL of the image.
    ///
    /// # Errors
    ///
    /// Returns an error if the media type cannot be inferred from the URL.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::types::image::ImageContent;
    ///
    /// let image = ImageContent::from_url("https://example.com/photo.jpg")?;
    /// ```
    pub fn from_url(url: &str) -> Result<Self, ImageError> {
        // Infer media type from URL extension
        let media_type = infer_media_type_from_url(url).ok_or_else(|| {
            ImageError::InvalidUrl("could not infer media type from URL".to_string())
        })?;

        Ok(Self {
            source: ImageSource::Url {
                url: url.to_string(),
            },
            media_type,
        })
    }
}

/// The source of an image.
///
/// Images can be provided either as base64-encoded data or as URLs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    /// Base64-encoded image data.
    Base64 {
        /// The MIME type of the image (e.g., "image/png").
        media_type: String,
        /// The base64-encoded image data.
        data: String,
    },

    /// A URL pointing to the image.
    Url {
        /// The URL of the image.
        url: String,
    },
}

/// Supported image media types for Claude Vision API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    /// PNG image (`image/png`).
    Png,
    /// JPEG image (`image/jpeg`).
    Jpeg,
    /// GIF image (`image/gif`).
    Gif,
    /// WebP image (`image/webp`).
    Webp,
}

impl MediaType {
    /// Returns the MIME type string for this media type.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::types::image::MediaType;
    ///
    /// assert_eq!(MediaType::Png.as_str(), "image/png");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// Creates a `MediaType` from a file extension.
    ///
    /// # Arguments
    ///
    /// * `ext` - The file extension (case-insensitive).
    ///
    /// # Returns
    ///
    /// Returns `Some(MediaType)` if the extension is recognized, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use patina::types::image::MediaType;
    ///
    /// assert_eq!(MediaType::from_extension("png"), Some(MediaType::Png));
    /// assert_eq!(MediaType::from_extension("bmp"), None);
    /// ```
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

impl Serialize for MediaType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MediaType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "image/png" => Ok(Self::Png),
            "image/jpeg" => Ok(Self::Jpeg),
            "image/gif" => Ok(Self::Gif),
            "image/webp" => Ok(Self::Webp),
            _ => Err(serde::de::Error::custom(format!(
                "unsupported media type: {s}"
            ))),
        }
    }
}

/// Errors that can occur when working with images.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The image file could not be read.
    #[error("failed to read image file: {0}")]
    IoError(#[from] std::io::Error),

    /// The image exceeds the maximum allowed size.
    #[error("image size exceeds maximum of {MAX_IMAGE_SIZE} bytes")]
    FileTooLarge,

    /// The image format is not supported.
    #[error("unsupported image format")]
    UnsupportedFormat,

    /// The URL is invalid.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// Could not detect the image format.
    #[error("could not detect image format from magic bytes")]
    UnknownFormat,
}

/// Detects the media type from image magic bytes.
///
/// # Arguments
///
/// * `bytes` - The raw image data.
///
/// # Returns
///
/// Returns `Some(MediaType)` if the format is recognized, `None` otherwise.
fn detect_media_type(bytes: &[u8]) -> Option<MediaType> {
    if bytes.len() < 4 {
        return None;
    }

    // PNG: 89 50 4E 47 (‰PNG)
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some(MediaType::Png);
    }

    // JPEG: FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(MediaType::Jpeg);
    }

    // GIF: 47 49 46 38 (GIF8)
    if bytes.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return Some(MediaType::Gif);
    }

    // WebP: 52 49 46 46 ... 57 45 42 50 (RIFF...WEBP)
    if bytes.len() >= 12
        && bytes.starts_with(&[0x52, 0x49, 0x46, 0x46])
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return Some(MediaType::Webp);
    }

    None
}

/// Infers the media type from a URL's file extension.
///
/// # Arguments
///
/// * `url` - The URL to parse.
///
/// # Returns
///
/// Returns `Some(MediaType)` if the extension is recognized, `None` otherwise.
fn infer_media_type_from_url(url: &str) -> Option<MediaType> {
    // Extract the path from the URL, handling query strings
    let path = url.split('?').next()?;

    // Get the extension
    let ext = path.rsplit('.').next()?;

    MediaType::from_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_media_type_png() {
        let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_media_type(&png_bytes), Some(MediaType::Png));
    }

    #[test]
    fn test_detect_media_type_jpeg() {
        let jpeg_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_media_type(&jpeg_bytes), Some(MediaType::Jpeg));
    }

    #[test]
    fn test_detect_media_type_gif() {
        let gif_bytes = vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_media_type(&gif_bytes), Some(MediaType::Gif));
    }

    #[test]
    fn test_detect_media_type_webp() {
        let webp_bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
        ];
        assert_eq!(detect_media_type(&webp_bytes), Some(MediaType::Webp));
    }

    #[test]
    fn test_detect_media_type_unknown() {
        let unknown_bytes = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_media_type(&unknown_bytes), None);
    }

    #[test]
    fn test_detect_media_type_too_short() {
        let short_bytes = vec![0x89, 0x50];
        assert_eq!(detect_media_type(&short_bytes), None);
    }

    #[test]
    fn test_infer_media_type_from_url_png() {
        assert_eq!(
            infer_media_type_from_url("https://example.com/image.png"),
            Some(MediaType::Png)
        );
    }

    #[test]
    fn test_infer_media_type_from_url_with_query() {
        assert_eq!(
            infer_media_type_from_url("https://example.com/image.jpg?size=large"),
            Some(MediaType::Jpeg)
        );
    }

    #[test]
    fn test_infer_media_type_from_url_unknown() {
        assert_eq!(
            infer_media_type_from_url("https://example.com/file.txt"),
            None
        );
    }
}
