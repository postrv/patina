//! Tests for utility functions

use patina::util::{format_bytes, sanitize_filename, truncate_string};

mod truncate_tests {
    use super::*;

    #[test]
    fn test_truncate_string_shorter_than_max() {
        let result = truncate_string("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_string_equal_to_max() {
        let result = truncate_string("hello", 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_string_longer_than_max() {
        let result = truncate_string("hello world", 8);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_string_empty() {
        let result = truncate_string("", 10);
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_string_very_short_max() {
        let result = truncate_string("hello", 3);
        assert_eq!(result, "...");
    }
}

mod format_bytes_tests {
    use super::*;

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn test_format_bytes_kilobytes() {
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1023), "1023.00 KB");
    }

    #[test]
    fn test_format_bytes_megabytes() {
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 100), "100.00 MB");
    }

    #[test]
    fn test_format_bytes_gigabytes() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 10), "10.00 GB");
    }
}

mod sanitize_filename_tests {
    use super::*;

    #[test]
    fn test_sanitize_filename_clean() {
        assert_eq!(sanitize_filename("hello.txt"), "hello.txt");
        assert_eq!(sanitize_filename("my-file_123"), "my-file_123");
    }

    #[test]
    fn test_sanitize_filename_with_slashes() {
        assert_eq!(sanitize_filename("path/to/file"), "path_to_file");
        assert_eq!(sanitize_filename("path\\to\\file"), "path_to_file");
    }

    #[test]
    fn test_sanitize_filename_with_special_chars() {
        assert_eq!(sanitize_filename("file:name"), "file_name");
        assert_eq!(sanitize_filename("file*name"), "file_name");
        assert_eq!(sanitize_filename("file?name"), "file_name");
        assert_eq!(sanitize_filename("file\"name"), "file_name");
        assert_eq!(sanitize_filename("file<name>"), "file_name_");
        assert_eq!(sanitize_filename("file|name"), "file_name");
    }

    #[test]
    fn test_sanitize_filename_multiple_special() {
        assert_eq!(sanitize_filename("a/b\\c:d*e?f"), "a_b_c_d_e_f");
    }
}

mod text_tests {
    use patina::util::text::{visible_width, wrap_text};

    #[test]
    fn test_visible_width_ascii() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width(""), 0);
        assert_eq!(visible_width("hello world"), 11);
    }

    #[test]
    fn test_visible_width_unicode() {
        // CJK characters are typically 2 cells wide
        assert_eq!(visible_width("日本語"), 6);
        // Emoji
        assert_eq!(visible_width("🦀"), 2);
    }

    #[test]
    fn test_wrap_text_no_wrap_needed() {
        let result = wrap_text("hello", 80);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_text_single_long_line() {
        let result = wrap_text("hello world foo bar", 10);
        // Multiple lines expected
        assert!(result.len() >= 2);
        // Each line should respect width limit
        for line in &result {
            assert!(line.len() <= 10);
        }
    }

    #[test]
    fn test_wrap_text_preserves_words() {
        let result = wrap_text("the quick brown fox", 10);
        // Should wrap at word boundaries
        for line in &result {
            assert!(!line.starts_with(' '));
        }
    }
}

mod directory_tests {
    use patina::util::{get_cache_dir, get_config_dir, get_data_dir, get_plugins_dir};

    #[test]
    fn test_get_config_dir() {
        let dir = get_config_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.to_string_lossy().contains("rct"));
    }

    #[test]
    fn test_get_data_dir() {
        let dir = get_data_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.to_string_lossy().contains("rct"));
    }

    #[test]
    fn test_get_cache_dir() {
        let dir = get_cache_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.to_string_lossy().contains("rct"));
    }

    #[test]
    fn test_get_plugins_dir() {
        let dir = get_plugins_dir();
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.ends_with("plugins"));
    }
}
