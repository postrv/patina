//! Common test utilities and fixtures for RCT.
//!
//! This module provides shared test infrastructure including:
//! - Mock factories for common types
//! - Test fixtures and helpers
//! - Async test utilities
//! - Cross-platform utilities for Windows/Unix compatibility

use std::io;
use std::path::{Path, PathBuf};

/// Test context providing common setup for integration tests.
pub struct TestContext {
    /// Temporary directory for test file operations.
    pub temp_dir: tempfile::TempDir,
}

impl TestContext {
    /// Creates a new test context with a temporary directory.
    ///
    /// # Panics
    ///
    /// Panics if the temporary directory cannot be created.
    #[must_use]
    pub fn new() -> Self {
        Self {
            temp_dir: tempfile::tempdir().expect("failed to create temp dir"),
        }
    }

    /// Returns the path to the temporary directory.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.temp_dir.path().to_path_buf()
    }

    /// Creates a file in the temporary directory with the given content.
    ///
    /// # Panics
    ///
    /// Panics if the file cannot be created or written.
    pub fn create_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.temp_dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, content).expect("failed to write file");
        path
    }

    /// Creates a platform-appropriate script file in the temporary directory.
    ///
    /// On Unix, creates a shell script with `#!/bin/sh` header and executable permissions.
    /// On Windows, creates a batch file with `.bat` extension.
    ///
    /// # Arguments
    ///
    /// * `name` - Base name of the script (without extension on Windows, .bat is added automatically)
    /// * `content` - Script content (should be valid for the target platform)
    ///
    /// # Panics
    ///
    /// Panics if the file cannot be created or permissions cannot be set.
    pub fn temp_script(&self, name: &str, content: &str) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script_content = if content.starts_with("#!") {
                content.to_string()
            } else {
                format!("#!/bin/sh\n{}", content)
            };

            let path = self.temp_dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("failed to create parent dirs");
            }
            std::fs::write(&path, script_content).expect("failed to write script");

            // Make executable
            let mut perms = std::fs::metadata(&path)
                .expect("failed to get metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("failed to set permissions");

            path
        }

        #[cfg(windows)]
        {
            // Add .bat extension if not already present
            let script_name = if name.ends_with(".bat") || name.ends_with(".cmd") {
                name.to_string()
            } else {
                format!("{}.bat", name)
            };

            let path = self.temp_dir.path().join(script_name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("failed to create parent dirs");
            }
            std::fs::write(&path, content).expect("failed to write script");

            path
        }
    }
}

// =============================================================================
// Cross-Platform Helper Functions
// =============================================================================

/// Returns true if running on Windows.
///
/// This is a runtime check, useful for conditionally skipping tests
/// or adjusting behavior based on the platform.
#[must_use]
pub fn is_windows() -> bool {
    cfg!(windows)
}

/// Returns true if running on Unix (Linux, macOS, etc.).
#[must_use]
pub fn is_unix() -> bool {
    cfg!(unix)
}

/// Macro to skip a test on Windows with a reason.
///
/// This should be called at the beginning of a test function to skip
/// execution on Windows platforms.
///
/// # Example
///
/// ```ignore
/// #[test]
/// fn test_unix_only_feature() {
///     skip_on_windows!("symlinks require admin on Windows");
///     // ... rest of test
/// }
/// ```
#[macro_export]
macro_rules! skip_on_windows {
    ($reason:expr) => {
        if cfg!(windows) {
            eprintln!("Skipping test on Windows: {}", $reason);
            return;
        }
    };
}

// =============================================================================
// Permission Helpers
// =============================================================================

/// Makes a file read-only on any platform.
///
/// # Unix
/// Sets permissions to 0o444 (read-only for all).
///
/// # Windows
/// Sets the read-only attribute.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or permissions cannot be changed.
pub fn make_readonly(path: &Path) -> io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o444);
    }

    #[cfg(windows)]
    {
        perms.set_readonly(true);
    }

    std::fs::set_permissions(path, perms)
}

/// Makes a file writable on any platform.
///
/// # Unix
/// Sets permissions to 0o644 (read-write for owner, read for others).
///
/// # Windows
/// Removes the read-only attribute.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or permissions cannot be changed.
pub fn make_writable(path: &Path) -> io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o644);
    }

    #[cfg(windows)]
    {
        perms.set_readonly(false);
    }

    std::fs::set_permissions(path, perms)
}

/// Makes a directory read-only (prevents creating files inside) on any platform.
///
/// # Unix
/// Sets permissions to 0o555 (read and execute, but no write).
///
/// # Windows
/// This is more complex; directories don't have a simple read-only flag.
/// This function will attempt to set the read-only attribute, but it may
/// not fully prevent writes on all Windows versions.
///
/// # Errors
///
/// Returns an error if the directory doesn't exist or permissions cannot be changed.
pub fn make_dir_readonly(path: &Path) -> io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o555);
    }

    #[cfg(windows)]
    {
        perms.set_readonly(true);
    }

    std::fs::set_permissions(path, perms)
}

/// Makes a directory writable on any platform.
///
/// # Errors
///
/// Returns an error if the directory doesn't exist or permissions cannot be changed.
pub fn make_dir_writable(path: &Path) -> io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }

    #[cfg(windows)]
    {
        perms.set_readonly(false);
    }

    std::fs::set_permissions(path, perms)
}

// =============================================================================
// Symlink Helpers
// =============================================================================

/// Creates a symbolic link at `link` pointing to `target`.
///
/// # Platform Differences
///
/// - **Unix**: Uses `std::os::unix::fs::symlink`
/// - **Windows**: Uses `std::os::windows::fs::symlink_dir` or `symlink_file`
///   depending on whether the target is a directory or file.
///
/// # Windows Note
///
/// On Windows, creating symlinks requires either:
/// - Administrator privileges, OR
/// - Developer Mode enabled (Windows 10+)
///
/// Use [`symlinks_available`] to check if symlinks can be created.
///
/// # Errors
///
/// Returns an error if the symlink cannot be created.
pub fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

/// Checks if symbolic links can be created on this system.
///
/// # Unix
/// Always returns `true` (symlinks are always available on Unix).
///
/// # Windows
/// Attempts to create a test symlink in the temp directory.
/// Returns `true` if successful (user has Developer Mode or admin rights).
///
/// This function is useful for conditionally skipping symlink tests on
/// Windows systems without the required permissions.
pub fn symlinks_available() -> bool {
    #[cfg(unix)]
    {
        true
    }

    #[cfg(windows)]
    {
        // Try to create a symlink in temp directory
        let temp_dir = std::env::temp_dir();
        let target = temp_dir.join("symlink_test_target");
        let link = temp_dir.join("symlink_test_link");

        // Clean up any existing test files
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);

        // Create target file
        if std::fs::write(&target, "test").is_err() {
            return false;
        }

        // Try to create symlink
        let can_create = std::os::windows::fs::symlink_file(&target, &link).is_ok();

        // Clean up
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);

        can_create
    }
}

/// Macro to skip a test if symlinks are not available.
///
/// This is primarily useful on Windows where symlinks require special permissions.
///
/// # Example
///
/// ```ignore
/// #[test]
/// fn test_symlink_handling() {
///     skip_if_no_symlinks!();
///     // ... rest of test
/// }
/// ```
#[macro_export]
macro_rules! skip_if_no_symlinks {
    () => {
        if !$crate::common::symlinks_available() {
            eprintln!(
                "Skipping test: symlinks not available (Windows requires Developer Mode or admin)"
            );
            return;
        }
    };
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creates_temp_dir() {
        let ctx = TestContext::new();
        assert!(ctx.path().exists());
    }

    #[test]
    fn test_context_creates_file() {
        let ctx = TestContext::new();
        let file_path = ctx.create_file("test.txt", "hello world");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_context_creates_nested_file() {
        let ctx = TestContext::new();
        let file_path = ctx.create_file("nested/dir/test.txt", "nested content");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "nested content");
    }

    // =============================================================================
    // temp_script tests
    // =============================================================================

    #[cfg(unix)]
    #[test]
    fn test_temp_script_creates_executable_unix() {
        use std::os::unix::fs::PermissionsExt;

        let ctx = TestContext::new();
        let script_path = ctx.temp_script("test_script", "echo 'hello'");

        assert!(script_path.exists());

        // Check content has shebang
        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.starts_with("#!/bin/sh\n"));
        assert!(content.contains("echo 'hello'"));

        // Check executable permission
        let perms = std::fs::metadata(&script_path).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0, "script should be executable");
    }

    #[cfg(unix)]
    #[test]
    fn test_temp_script_preserves_existing_shebang() {
        let ctx = TestContext::new();
        let script_path = ctx.temp_script("test_script", "#!/bin/bash\necho 'hello'");

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.starts_with("#!/bin/bash\n"));
        assert!(!content.contains("#!/bin/sh\n"));
    }

    #[cfg(windows)]
    #[test]
    fn test_temp_script_creates_batch_file_windows() {
        let ctx = TestContext::new();
        let script_path = ctx.temp_script("test_script", "@echo off\necho hello");

        assert!(script_path.exists());
        assert!(
            script_path.extension().map(|e| e == "bat").unwrap_or(false),
            "script should have .bat extension"
        );

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.contains("echo hello"));
    }

    #[cfg(windows)]
    #[test]
    fn test_temp_script_keeps_existing_bat_extension() {
        let ctx = TestContext::new();
        let script_path = ctx.temp_script("test_script.bat", "echo hello");

        assert!(script_path.exists());
        // Should not have .bat.bat
        assert!(!script_path.to_string_lossy().ends_with(".bat.bat"));
    }

    // =============================================================================
    // Platform check tests
    // =============================================================================

    #[test]
    fn test_is_windows_consistent() {
        // This test just verifies the function works and is consistent
        let result = is_windows();
        // On Windows, should be true; on others, false
        #[cfg(windows)]
        assert!(result);
        #[cfg(not(windows))]
        assert!(!result);
    }

    #[test]
    fn test_is_unix_consistent() {
        let result = is_unix();
        #[cfg(unix)]
        assert!(result);
        #[cfg(not(unix))]
        assert!(!result);
    }

    // =============================================================================
    // Permission helper tests
    // =============================================================================

    #[test]
    fn test_make_readonly_and_writable() {
        let ctx = TestContext::new();
        let file_path = ctx.create_file("perm_test.txt", "test content");

        // Make read-only
        make_readonly(&file_path).expect("failed to make readonly");

        // Verify we can't write (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&file_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o222, 0, "file should not be writable");
        }

        // On Windows, check readonly attribute
        #[cfg(windows)]
        {
            let perms = std::fs::metadata(&file_path).unwrap().permissions();
            assert!(perms.readonly(), "file should be readonly");
        }

        // Make writable again
        make_writable(&file_path).expect("failed to make writable");

        // Verify we can write
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&file_path).unwrap().permissions();
            assert_ne!(perms.mode() & 0o200, 0, "file should be writable");
        }

        #[cfg(windows)]
        {
            let perms = std::fs::metadata(&file_path).unwrap().permissions();
            assert!(!perms.readonly(), "file should not be readonly");
        }
    }

    #[test]
    fn test_make_dir_readonly_and_writable() {
        let ctx = TestContext::new();
        let dir_path = ctx.path().join("test_dir");
        std::fs::create_dir(&dir_path).expect("failed to create dir");

        // Make read-only
        make_dir_readonly(&dir_path).expect("failed to make dir readonly");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&dir_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o222, 0, "dir should not be writable");
        }

        // Make writable again (needed for cleanup)
        make_dir_writable(&dir_path).expect("failed to make dir writable");
    }

    // =============================================================================
    // Symlink helper tests
    // =============================================================================

    #[test]
    fn test_symlinks_available_returns_bool() {
        // Just verify it returns a boolean without panicking
        let _available = symlinks_available();
    }

    #[cfg(unix)]
    #[test]
    fn test_create_symlink_unix() {
        let ctx = TestContext::new();
        let target = ctx.create_file("target.txt", "target content");
        let link = ctx.path().join("link.txt");

        create_symlink(&target, &link).expect("failed to create symlink");

        assert!(link.exists());
        assert!(link.is_symlink());

        let content = std::fs::read_to_string(&link).unwrap();
        assert_eq!(content, "target content");
    }

    #[cfg(windows)]
    #[test]
    fn test_create_symlink_windows() {
        if !symlinks_available() {
            eprintln!("Skipping: symlinks require Developer Mode or admin on Windows");
            return;
        }

        let ctx = TestContext::new();
        let target = ctx.create_file("target.txt", "target content");
        let link = ctx.path().join("link.txt");

        create_symlink(&target, &link).expect("failed to create symlink");

        assert!(link.exists());

        let content = std::fs::read_to_string(&link).unwrap();
        assert_eq!(content, "target content");
    }
}
