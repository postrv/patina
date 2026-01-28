//! Utility functions and helpers

use std::path::PathBuf;
use directories::ProjectDirs;

pub fn get_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct")
        .map(|dirs| dirs.config_dir().to_path_buf())
}

pub fn get_data_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct")
        .map(|dirs| dirs.data_dir().to_path_buf())
}

pub fn get_cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "rct", "rct")
        .map(|dirs| dirs.cache_dir().to_path_buf())
}

pub fn get_plugins_dir() -> Option<PathBuf> {
    get_config_dir().map(|d| d.join("plugins"))
}

pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
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
