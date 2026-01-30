//! Common test utilities and fixtures for RCT.
//!
//! This module provides shared test infrastructure including:
//! - Mock factories for common types
//! - Test fixtures and helpers
//! - Async test utilities

use std::path::PathBuf;

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
}
