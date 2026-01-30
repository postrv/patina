//! Configuration types for RCT.
//!
//! This module contains configuration structures used to initialize
//! and configure the application.

use secrecy::SecretString;
use std::path::PathBuf;

/// Application configuration.
///
/// Contains all settings needed to initialize and run the RCT application.
///
/// # Security Note
///
/// The `api_key` field uses [`SecretString`] from the `secrecy` crate
/// to prevent accidental logging or exposure of sensitive credentials.
///
/// # Examples
///
/// ```no_run
/// use rct::types::config::Config;
/// use secrecy::SecretString;
/// use std::path::PathBuf;
///
/// let config = Config {
///     api_key: SecretString::new("sk-ant-api...".into()),
///     model: "claude-sonnet-4-20250514".to_string(),
///     working_dir: PathBuf::from("."),
/// };
/// ```
pub struct Config {
    /// API key for authentication with the Anthropic API.
    ///
    /// This is stored as a [`SecretString`] to prevent accidental exposure.
    pub api_key: SecretString,

    /// Model identifier to use for API requests.
    ///
    /// Examples: "claude-sonnet-4-20250514", "claude-opus-4-20250514"
    pub model: String,

    /// Working directory for file operations.
    ///
    /// All relative paths will be resolved relative to this directory.
    pub working_dir: PathBuf,
}

impl Config {
    /// Creates a new configuration with the given settings.
    ///
    /// # Arguments
    ///
    /// * `api_key` - Anthropic API key
    /// * `model` - Model identifier (e.g., "claude-sonnet-4-20250514")
    /// * `working_dir` - Base directory for file operations
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rct::types::config::Config;
    /// use secrecy::SecretString;
    /// use std::path::PathBuf;
    ///
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4-20250514",
    ///     PathBuf::from("/home/user/project"),
    /// );
    /// ```
    #[must_use]
    pub fn new(api_key: SecretString, model: impl Into<String>, working_dir: PathBuf) -> Self {
        Self {
            api_key,
            model: model.into(),
            working_dir,
        }
    }

    /// Returns the model identifier.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the working directory path.
    #[must_use]
    pub fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert_eq!(config.model(), "test-model");
        assert_eq!(config.working_dir(), &PathBuf::from("/tmp"));
    }

    #[test]
    fn test_config_model_accessor() {
        let config = Config {
            api_key: SecretString::new("key".into()),
            model: "claude-opus-4-20250514".to_string(),
            working_dir: PathBuf::from("."),
        };

        assert_eq!(config.model(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_config_working_dir_accessor() {
        let path = PathBuf::from("/home/user/project");
        let config = Config {
            api_key: SecretString::new("key".into()),
            model: "model".to_string(),
            working_dir: path.clone(),
        };

        assert_eq!(config.working_dir(), &path);
    }
}
