//! Narsil-mcp integration and auto-detection.
//!
//! This module provides automatic detection of narsil-mcp availability
//! and intelligent enabling based on project characteristics.
//!
//! # Auto-detection Logic
//!
//! Narsil is auto-enabled when:
//! 1. `narsil-mcp` is available in PATH
//! 2. The project contains supported code files (Rust, Python, TypeScript, etc.)
//!
//! # Example
//!
//! ```no_run
//! use patina::plugins::narsil::{is_narsil_available, should_enable_narsil};
//! use std::path::Path;
//!
//! if is_narsil_available() {
//!     println!("narsil-mcp is installed");
//! }
//!
//! if should_enable_narsil(Path::new(".")) {
//!     println!("Narsil should be enabled for this project");
//! }
//! ```

use std::path::Path;
use std::process::Command;

/// File extensions supported by narsil-mcp for code analysis.
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "rs",    // Rust
    "py",    // Python
    "ts",    // TypeScript
    "tsx",   // TypeScript React
    "js",    // JavaScript
    "jsx",   // JavaScript React
    "go",    // Go
    "java",  // Java
    "kt",    // Kotlin
    "c",     // C
    "cpp",   // C++
    "h",     // C/C++ headers
    "hpp",   // C++ headers
    "rb",    // Ruby
    "php",   // PHP
    "cs",    // C#
    "swift", // Swift
];

/// Directories to skip when scanning for code files.
const SKIP_DIRECTORIES: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "build",
    "dist",
    "__pycache__",
    ".venv",
    "venv",
    ".cargo",
];

/// Maximum depth to scan for code files.
const MAX_SCAN_DEPTH: usize = 3;

/// Maximum number of files to check before deciding.
const MAX_FILES_TO_CHECK: usize = 100;

/// Checks if narsil-mcp is available in the system PATH.
///
/// Uses `which narsil-mcp` on Unix or `where narsil-mcp` on Windows
/// to determine availability.
///
/// # Returns
///
/// `true` if narsil-mcp is found and executable, `false` otherwise.
///
/// # Example
///
/// ```no_run
/// use patina::plugins::narsil::is_narsil_available;
///
/// if is_narsil_available() {
///     println!("narsil-mcp is ready to use");
/// }
/// ```
#[must_use]
pub fn is_narsil_available() -> bool {
    #[cfg(unix)]
    let command = "which";
    #[cfg(windows)]
    let command = "where";

    Command::new(command)
        .arg("narsil-mcp")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Checks if a directory contains supported code files.
///
/// Scans the directory (up to `MAX_SCAN_DEPTH` levels) looking for files
/// with extensions supported by narsil-mcp. Skips common build/dependency
/// directories for performance.
///
/// # Arguments
///
/// * `dir` - The directory to scan for code files
///
/// # Returns
///
/// `true` if supported code files are found, `false` otherwise.
#[must_use]
pub fn has_supported_code_files(dir: &Path) -> bool {
    if !dir.exists() || !dir.is_dir() {
        return false;
    }

    has_supported_code_files_recursive(dir, 0, &mut 0)
}

/// Recursive helper for scanning directories.
fn has_supported_code_files_recursive(dir: &Path, depth: usize, files_checked: &mut usize) -> bool {
    if depth > MAX_SCAN_DEPTH || *files_checked >= MAX_FILES_TO_CHECK {
        return false;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Skip hidden files and known non-source directories
        if file_name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            // Skip common build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                continue;
            }

            if has_supported_code_files_recursive(&path, depth + 1, files_checked) {
                return true;
            }
        } else if path.is_file() {
            *files_checked += 1;

            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SUPPORTED_EXTENSIONS.contains(&ext) {
                    return true;
                }
            }

            if *files_checked >= MAX_FILES_TO_CHECK {
                return false;
            }
        }
    }

    false
}

/// Determines if narsil should be automatically enabled for a project.
///
/// Narsil is auto-enabled when both conditions are met:
/// 1. `narsil-mcp` is available in PATH
/// 2. The project contains supported code files
///
/// # Arguments
///
/// * `project_dir` - The root directory of the project
///
/// # Returns
///
/// `true` if narsil should be auto-enabled, `false` otherwise.
///
/// # Example
///
/// ```no_run
/// use patina::plugins::narsil::should_enable_narsil;
/// use std::path::Path;
///
/// let project = Path::new("/path/to/my-project");
/// if should_enable_narsil(project) {
///     println!("Enabling narsil for code intelligence");
/// }
/// ```
#[must_use]
pub fn should_enable_narsil(project_dir: &Path) -> bool {
    is_narsil_available() && has_supported_code_files(project_dir)
}

/// Returns the list of file extensions supported by narsil.
#[must_use]
pub fn supported_extensions() -> &'static [&'static str] {
    SUPPORTED_EXTENSIONS
}

/// Returns the list of directories skipped during code file scanning.
#[must_use]
pub fn skip_directories() -> &'static [&'static str] {
    SKIP_DIRECTORIES
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_supported_extensions_not_empty() {
        assert!(!SUPPORTED_EXTENSIONS.is_empty());
        assert!(SUPPORTED_EXTENSIONS.contains(&"rs"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"py"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"ts"));
    }

    #[test]
    fn test_skip_directories_not_empty() {
        assert!(!SKIP_DIRECTORIES.is_empty());
        assert!(SKIP_DIRECTORIES.contains(&"target"));
        assert!(SKIP_DIRECTORIES.contains(&"node_modules"));
        assert!(SKIP_DIRECTORIES.contains(&".git"));
    }

    #[test]
    fn test_has_supported_code_files_with_rust() {
        let temp_dir = TempDir::new().unwrap();
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();

        assert!(has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_with_python() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("app.py"), "print('hello')").unwrap();

        assert!(has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_with_typescript() {
        let temp_dir = TempDir::new().unwrap();
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("index.ts"), "const x = 1;").unwrap();

        assert!(has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        assert!(!has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_no_code() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("README.md"), "# Hello").unwrap();
        fs::write(temp_dir.path().join("config.toml"), "key = 'value'").unwrap();

        assert!(!has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_skips_target() {
        let temp_dir = TempDir::new().unwrap();
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("main.rs"), "fn main() {}").unwrap();

        // Only code in target directory should not count
        assert!(!has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_skips_node_modules() {
        let temp_dir = TempDir::new().unwrap();
        let node_modules = temp_dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("index.js"), "module.exports = {}").unwrap();

        assert!(!has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_nonexistent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent = temp_dir.path().join("does-not-exist");

        assert!(!has_supported_code_files(&nonexistent));
    }

    #[test]
    fn test_has_supported_code_files_nested() {
        let temp_dir = TempDir::new().unwrap();
        let nested = temp_dir.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("lib.rs"), "pub fn foo() {}").unwrap();

        assert!(has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_has_supported_code_files_respects_depth_limit() {
        let temp_dir = TempDir::new().unwrap();
        // Create a deeply nested directory structure
        let deep = temp_dir.path().join("a/b/c/d/e/f");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("main.rs"), "fn main() {}").unwrap();

        // At depth 3, we shouldn't find files at depth 6
        // The file is at depth 6 (a=1, b=2, c=3, d=4, e=5, f=6)
        // We only scan up to depth 3, so this should return false
        assert!(!has_supported_code_files(temp_dir.path()));
    }

    #[test]
    fn test_supported_extensions_accessor() {
        let exts = supported_extensions();
        assert!(exts.contains(&"rs"));
        assert!(exts.contains(&"go"));
    }

    #[test]
    fn test_skip_directories_accessor() {
        let dirs = skip_directories();
        assert!(dirs.contains(&"target"));
        assert!(dirs.contains(&".git"));
    }

    // Note: is_narsil_available() is not tested here because it depends
    // on the actual system state (whether narsil-mcp is installed).
    // Integration tests can verify this behavior in environments where
    // narsil-mcp is guaranteed to be present or absent.
}
