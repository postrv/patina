//! Configuration types for Patina.
//!
//! This module contains configuration structures used to initialize
//! and configure the application.

use secrecy::SecretString;
use std::path::PathBuf;

/// Controls session resume behavior.
///
/// When starting Patina, users can optionally resume a previous session
/// instead of starting fresh. This enum specifies which session to resume.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ResumeMode {
    /// Start a new session (default behavior).
    #[default]
    None,

    /// Resume the most recently updated session.
    Last,

    /// Resume a specific session by its ID.
    SessionId(String),
}

impl ResumeMode {
    /// Returns `true` if a session should be resumed.
    ///
    /// Returns `false` only for `ResumeMode::None`.
    #[must_use]
    pub fn is_resuming(&self) -> bool {
        !matches!(self, ResumeMode::None)
    }
}

/// Controls how narsil-mcp integration is enabled.
///
/// Narsil provides code intelligence and security scanning capabilities.
/// This enum determines whether narsil is enabled for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NarsilMode {
    /// Auto-detect narsil availability and project compatibility.
    ///
    /// Narsil is enabled if:
    /// 1. `narsil-mcp` is available in PATH
    /// 2. The project contains supported code files
    #[default]
    Auto,

    /// Always enable narsil (fails if not available).
    Enabled,

    /// Never enable narsil, even if available.
    Disabled,
}

/// Application configuration.
///
/// Contains all settings needed to initialize and run the Patina application.
///
/// # Security Note
///
/// The `api_key` field uses [`SecretString`] from the `secrecy` crate
/// to prevent accidental logging or exposure of sensitive credentials.
///
/// # Examples
///
/// ```no_run
/// use patina::types::config::{Config, NarsilMode, ResumeMode};
/// use secrecy::SecretString;
/// use std::path::PathBuf;
///
/// let config = Config {
///     api_key: SecretString::new("sk-ant-api...".into()),
///     model: "claude-sonnet-4-20250514".to_string(),
///     working_dir: PathBuf::from("."),
///     narsil_mode: NarsilMode::Auto,
///     resume_mode: ResumeMode::None,
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

    /// Narsil-mcp integration mode.
    ///
    /// Controls whether narsil code intelligence is enabled.
    pub narsil_mode: NarsilMode,

    /// Session resume mode.
    ///
    /// Controls whether to resume a previous session on startup.
    pub resume_mode: ResumeMode,
}

impl Config {
    /// Creates a new configuration with the given settings.
    ///
    /// Uses `NarsilMode::Auto` by default. Use [`Config::with_narsil_mode`]
    /// to specify a different mode.
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
    /// use patina::types::config::Config;
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
            narsil_mode: NarsilMode::Auto,
            resume_mode: ResumeMode::None,
        }
    }

    /// Sets the narsil mode for this configuration.
    ///
    /// # Arguments
    ///
    /// * `mode` - The narsil integration mode to use
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::types::config::{Config, NarsilMode};
    /// use secrecy::SecretString;
    /// use std::path::PathBuf;
    ///
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4-20250514",
    ///     PathBuf::from("."),
    /// ).with_narsil_mode(NarsilMode::Enabled);
    /// ```
    #[must_use]
    pub fn with_narsil_mode(mut self, mode: NarsilMode) -> Self {
        self.narsil_mode = mode;
        self
    }

    /// Returns the narsil integration mode.
    #[must_use]
    pub fn narsil_mode(&self) -> NarsilMode {
        self.narsil_mode
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

    /// Sets the resume mode for this configuration.
    ///
    /// # Arguments
    ///
    /// * `mode` - The session resume mode to use
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::types::config::{Config, ResumeMode};
    /// use secrecy::SecretString;
    /// use std::path::PathBuf;
    ///
    /// // Resume the most recent session
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4-20250514",
    ///     PathBuf::from("."),
    /// ).with_resume_mode(ResumeMode::Last);
    ///
    /// // Resume a specific session
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4-20250514",
    ///     PathBuf::from("."),
    /// ).with_resume_mode(ResumeMode::SessionId("abc-123".to_string()));
    /// ```
    #[must_use]
    pub fn with_resume_mode(mut self, mode: ResumeMode) -> Self {
        self.resume_mode = mode;
        self
    }

    /// Returns the session resume mode.
    #[must_use]
    pub fn resume_mode(&self) -> &ResumeMode {
        &self.resume_mode
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
        assert_eq!(config.narsil_mode(), NarsilMode::Auto);
    }

    #[test]
    fn test_config_model_accessor() {
        let config = Config {
            api_key: SecretString::new("key".into()),
            model: "claude-opus-4-20250514".to_string(),
            working_dir: PathBuf::from("."),
            narsil_mode: NarsilMode::Auto,
            resume_mode: ResumeMode::None,
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
            narsil_mode: NarsilMode::Auto,
            resume_mode: ResumeMode::None,
        };

        assert_eq!(config.working_dir(), &path);
    }

    #[test]
    fn test_narsil_mode_default() {
        assert_eq!(NarsilMode::default(), NarsilMode::Auto);
    }

    #[test]
    fn test_config_with_narsil_mode() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_narsil_mode(NarsilMode::Enabled);

        assert_eq!(config.narsil_mode(), NarsilMode::Enabled);
    }

    #[test]
    fn test_config_narsil_disabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_narsil_mode(NarsilMode::Disabled);

        assert_eq!(config.narsil_mode(), NarsilMode::Disabled);
    }

    // =========================================================================
    // Phase 10.3.1: Resume mode tests
    // =========================================================================

    #[test]
    fn test_resume_mode_default() {
        assert_eq!(ResumeMode::default(), ResumeMode::None);
    }

    #[test]
    fn test_resume_mode_last() {
        let mode = ResumeMode::Last;
        assert!(matches!(mode, ResumeMode::Last));
    }

    #[test]
    fn test_resume_mode_session_id() {
        let mode = ResumeMode::SessionId("abc-123".to_string());
        if let ResumeMode::SessionId(id) = mode {
            assert_eq!(id, "abc-123");
        } else {
            panic!("Expected SessionId variant");
        }
    }

    #[test]
    fn test_config_default_resume_mode() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert_eq!(config.resume_mode(), &ResumeMode::None);
    }

    #[test]
    fn test_config_with_resume_last() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_resume_mode(ResumeMode::Last);

        assert_eq!(config.resume_mode(), &ResumeMode::Last);
    }

    #[test]
    fn test_config_with_resume_session_id() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_resume_mode(ResumeMode::SessionId("session-123".to_string()));

        assert_eq!(
            config.resume_mode(),
            &ResumeMode::SessionId("session-123".to_string())
        );
    }

    #[test]
    fn test_resume_mode_is_resuming() {
        assert!(!ResumeMode::None.is_resuming());
        assert!(ResumeMode::Last.is_resuming());
        assert!(ResumeMode::SessionId("abc".to_string()).is_resuming());
    }
}
