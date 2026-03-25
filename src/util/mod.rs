//! Utility functions and helpers

pub mod url_validation;

use directories::ProjectDirs;
use std::path::PathBuf;

pub fn get_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct").map(|dirs| dirs.config_dir().to_path_buf())
}

pub fn get_data_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct").map(|dirs| dirs.data_dir().to_path_buf())
}

pub fn get_cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct").map(|dirs| dirs.cache_dir().to_path_buf())
}

pub fn get_plugins_dir() -> Option<PathBuf> {
    get_config_dir().map(|d| d.join("plugins"))
}

/// Truncates a string to at most `max_len` characters (not bytes).
///
/// If truncation is needed, adds "..." suffix and ensures the result
/// is at most `max_len` characters total.
///
/// # UTF-8 Safety
///
/// This function correctly handles multi-byte UTF-8 characters and will
/// never panic due to slicing at non-character boundaries.
///
/// # Examples
///
/// ```
/// use patina::util::truncate_string;
///
/// assert_eq!(truncate_string("hello", 10), "hello");
/// assert_eq!(truncate_string("hello world", 8), "hello...");
/// assert_eq!(truncate_string("日本語テスト", 5), "日本...");
/// ```
pub fn truncate_string(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncate_at = max_len.saturating_sub(3);
        let truncated: String = s.chars().take(truncate_at).collect();
        format!("{}...", truncated)
    }
}

/// Truncates a string to at most `max_bytes` bytes, respecting UTF-8 boundaries.
///
/// If truncation is needed, adds a suffix and ensures the result respects
/// character boundaries (will truncate to the last complete character before the limit).
///
/// # UTF-8 Safety
///
/// This function correctly handles multi-byte UTF-8 characters and will
/// never panic due to slicing at non-character boundaries.
///
/// # Examples
///
/// ```
/// use patina::util::truncate_string_bytes;
///
/// assert_eq!(truncate_string_bytes("hello", 10, "..."), "hello");
/// assert_eq!(truncate_string_bytes("hello world", 8, "..."), "hello...");
/// // The ─ character is 3 bytes, so we stop before it
/// assert_eq!(truncate_string_bytes("test─more", 5, "..."), "te...");
/// ```
pub fn truncate_string_bytes(s: &str, max_bytes: usize, suffix: &str) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let suffix_len = suffix.len();
    let target_len = max_bytes.saturating_sub(suffix_len);

    // Find the last valid character boundary at or before target_len
    let boundary = floor_char_boundary(s, target_len);

    format!("{}{}", &s[..boundary], suffix)
}

/// Finds the largest byte index that is a character boundary and <= `index`.
///
/// This is a polyfill for the nightly `str::floor_char_boundary` method.
#[inline]
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        // Find the last char boundary at or before index
        let mut boundary = index;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        boundary
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

pub mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const ITALIC: &str = "\x1b[3m";
    pub const UNDERLINE: &str = "\x1b[4m";

    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";
}

pub mod text {
    use unicode_width::UnicodeWidthStr;

    pub fn visible_width(s: &str) -> usize {
        s.width()
    }

    pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
        textwrap::wrap(text, width)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_string_short() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_string_exact() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_string_long() {
        assert_eq!(truncate_string("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_string_utf8_multibyte() {
        // Japanese characters are 3 bytes each
        let s = "日本語テスト";
        // Should truncate by character count, not bytes
        assert_eq!(truncate_string(s, 5), "日本...");
    }

    #[test]
    fn test_truncate_string_empty() {
        assert_eq!(truncate_string("", 10), "");
    }

    #[test]
    fn test_truncate_string_bytes_short() {
        assert_eq!(truncate_string_bytes("hello", 10, "..."), "hello");
    }

    #[test]
    fn test_truncate_string_bytes_exact() {
        assert_eq!(truncate_string_bytes("hello", 5, "..."), "hello");
    }

    #[test]
    fn test_truncate_string_bytes_long() {
        assert_eq!(truncate_string_bytes("hello world", 8, "..."), "hello...");
    }

    #[test]
    fn test_truncate_string_bytes_utf8_boundary() {
        // The ─ character (U+2500) is 3 bytes in UTF-8
        // "test" is 4 bytes, "─" is bytes 4-6, "more" is bytes 7-10
        let s = "test─more";
        // Truncate at byte 5 should stop before ─ since it starts at byte 4
        assert_eq!(truncate_string_bytes(s, 7, "..."), "test...");
    }

    #[test]
    fn test_truncate_string_bytes_box_drawing() {
        // Box drawing characters are 3 bytes each
        let s = "hello───world";
        // "hello" is 5 bytes, each ─ is 3 bytes (total 9 bytes for the 3 dashes)
        // Truncate at 10 bytes: "hello" (5) + part of first ─ (3) = 8, so we get "hello"
        let result = truncate_string_bytes(s, 10, "...");
        // Should truncate to "hello" + "..." since 10-3=7 bytes and ─ starts at byte 5
        assert!(result.starts_with("hello"));
        assert!(result.ends_with("..."));
        // Should not panic!
    }

    #[test]
    fn test_truncate_string_bytes_emoji() {
        // Emoji can be 4 bytes
        let s = "hello🎉world";
        // "hello" is 5 bytes, 🎉 is 4 bytes
        let result = truncate_string_bytes(s, 8, "...");
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_string_bytes_empty() {
        assert_eq!(truncate_string_bytes("", 10, "..."), "");
    }

    #[test]
    fn test_truncate_string_bytes_suffix_longer_than_max() {
        // If suffix is longer than max, we still don't panic
        let result = truncate_string_bytes("hello world", 2, "...");
        // saturating_sub(2, 3) = 0, so we get just the suffix
        assert_eq!(result, "...");
    }

    #[test]
    fn test_floor_char_boundary_ascii() {
        let s = "hello";
        assert_eq!(floor_char_boundary(s, 3), 3);
        assert_eq!(floor_char_boundary(s, 5), 5);
        assert_eq!(floor_char_boundary(s, 10), 5); // Beyond string length
    }

    #[test]
    fn test_floor_char_boundary_multibyte() {
        // "─" is bytes 0-2 (3 bytes)
        let s = "─";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 0); // Middle of char
        assert_eq!(floor_char_boundary(s, 2), 0); // Middle of char
        assert_eq!(floor_char_boundary(s, 3), 3); // End of char
    }

    #[test]
    fn test_floor_char_boundary_mixed() {
        // "a─b" = 'a' (byte 0), '─' (bytes 1-3), 'b' (byte 4)
        let s = "a─b";
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 1), 1); // At ─
        assert_eq!(floor_char_boundary(s, 2), 1); // Middle of ─
        assert_eq!(floor_char_boundary(s, 3), 1); // Middle of ─
        assert_eq!(floor_char_boundary(s, 4), 4); // At 'b'
        assert_eq!(floor_char_boundary(s, 5), 5); // End of string
    }
}
