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

/// Controls parallel tool execution mode.
///
/// Parallel execution improves performance by running read-only tools
/// concurrently. This enum determines the level of parallelization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParallelMode {
    /// Parallel execution enabled with conservative settings.
    ///
    /// Only parallelizes tools that are definitely read-only:
    /// - `read_file`, `glob`, `grep`, `list_files`
    /// - Bash commands from a safe whitelist (e.g., `ls`, `cat`, `git status`)
    #[default]
    Enabled,

    /// Parallel execution disabled.
    ///
    /// All tools run sequentially. Use when debugging race conditions
    /// or when tool order matters.
    Disabled,

    /// Aggressive parallel execution.
    ///
    /// Also parallelizes tools with unknown side effects.
    /// WARNING: Can cause race conditions with MCP tools or bash commands.
    /// Only use when you understand the risks.
    Aggressive,
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
///     parallel_mode: ParallelMode::Enabled,
///     resume_mode: ResumeMode::None,
///     skip_permissions: false,
///     initial_prompt: None,
///     print_mode: false,
///     vision_model: None,
///     oauth_client_id: None,
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

    /// Parallel tool execution mode.
    ///
    /// Controls whether and how tools are executed in parallel.
    pub parallel_mode: ParallelMode,

    /// Session resume mode.
    ///
    /// Controls whether to resume a previous session on startup.
    pub resume_mode: ResumeMode,

    /// Whether to skip all permission prompts.
    ///
    /// When true, all tool executions are allowed without user approval.
    /// Use with caution - this bypasses security protections.
    pub skip_permissions: bool,

    /// Optional initial prompt to start the conversation with.
    ///
    /// When provided in interactive mode, this prompt is automatically
    /// submitted when the TUI starts.
    pub initial_prompt: Option<String>,

    /// Whether to run in print mode (non-interactive).
    ///
    /// When true (and `initial_prompt` is set):
    /// - Sends the prompt to Claude
    /// - Streams and prints the response to stdout
    /// - Executes any requested tools
    /// - Exits when complete
    pub print_mode: bool,

    /// Optional model to use for vision (image) requests.
    ///
    /// When set, messages containing images will automatically use this model
    /// instead of the default model. If not set, the default model is used
    /// for all requests (all Claude 3+ models support vision).
    pub vision_model: Option<String>,

    /// Optional OAuth client ID for subscription authentication.
    ///
    /// When set (via config or `PATINA_OAUTH_CLIENT_ID` environment variable),
    /// enables OAuth flow using the specified client ID. The client ID must be
    /// a valid UUID registered with Anthropic's developer program.
    pub oauth_client_id: Option<String>,
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
            parallel_mode: ParallelMode::Enabled,
            resume_mode: ResumeMode::None,
            skip_permissions: false,
            initial_prompt: None,
            print_mode: false,
            vision_model: None,
            oauth_client_id: None,
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

    /// Sets the parallel execution mode for this configuration.
    ///
    /// # Arguments
    ///
    /// * `mode` - The parallel execution mode to use
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::types::config::{Config, ParallelMode};
    /// use secrecy::SecretString;
    /// use std::path::PathBuf;
    ///
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4-20250514",
    ///     PathBuf::from("."),
    /// ).with_parallel_mode(ParallelMode::Disabled);
    /// ```
    #[must_use]
    pub fn with_parallel_mode(mut self, mode: ParallelMode) -> Self {
        self.parallel_mode = mode;
        self
    }

    /// Returns the parallel execution mode.
    #[must_use]
    pub fn parallel_mode(&self) -> ParallelMode {
        self.parallel_mode
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

    /// Sets whether to skip permission prompts.
    ///
    /// # Arguments
    ///
    /// * `skip` - If true, bypass all permission prompts
    ///
    /// # Security Warning
    ///
    /// This bypasses security protections. Use only when you trust
    /// all tool executions (e.g., in automated testing environments).
    #[must_use]
    pub fn with_skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }

    /// Returns whether permission prompts are being skipped.
    #[must_use]
    pub fn skip_permissions(&self) -> bool {
        self.skip_permissions
    }

    /// Sets an initial prompt to start the conversation with.
    ///
    /// When set in interactive mode, the prompt is automatically submitted.
    /// When combined with `with_print_mode(true)`, runs non-interactively.
    #[must_use]
    pub fn with_initial_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.initial_prompt = Some(prompt.into());
        self
    }

    /// Returns the initial prompt if set.
    #[must_use]
    pub fn initial_prompt(&self) -> Option<&str> {
        self.initial_prompt.as_deref()
    }

    /// Enables print mode (non-interactive).
    ///
    /// In print mode with an initial prompt, the application:
    /// 1. Sends the prompt to Claude
    /// 2. Streams and prints the response to stdout
    /// 3. Executes any requested tools
    /// 4. Exits when complete
    #[must_use]
    pub fn with_print_mode(mut self, enabled: bool) -> Self {
        self.print_mode = enabled;
        self
    }

    /// Returns whether print mode is enabled.
    #[must_use]
    pub fn print_mode(&self) -> bool {
        self.print_mode
    }

    /// Sets the vision model for image requests.
    ///
    /// When set, messages containing images will automatically use this model
    /// instead of the default model.
    ///
    /// # Arguments
    ///
    /// * `model` - The model identifier to use for vision requests
    #[must_use]
    pub fn with_vision_model(mut self, model: impl Into<String>) -> Self {
        self.vision_model = Some(model.into());
        self
    }

    /// Returns the vision model if set.
    #[must_use]
    pub fn vision_model(&self) -> Option<&str> {
        self.vision_model.as_deref()
    }

    /// Sets the OAuth client ID for subscription authentication.
    ///
    /// When set, enables OAuth flow using the specified client ID.
    /// The client ID must be a valid UUID registered with Anthropic.
    ///
    /// # Arguments
    ///
    /// * `client_id` - The OAuth client ID (UUID format)
    #[must_use]
    pub fn with_oauth_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.oauth_client_id = Some(client_id.into());
        self
    }

    /// Returns the OAuth client ID if set.
    #[must_use]
    pub fn oauth_client_id(&self) -> Option<&str> {
        self.oauth_client_id.as_deref()
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
            parallel_mode: ParallelMode::Enabled,
            resume_mode: ResumeMode::None,
            skip_permissions: false,
            initial_prompt: None,
            print_mode: false,
            vision_model: None,
            oauth_client_id: None,
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
            parallel_mode: ParallelMode::Enabled,
            resume_mode: ResumeMode::None,
            skip_permissions: false,
            initial_prompt: None,
            print_mode: false,
            vision_model: None,
            oauth_client_id: None,
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
    // Phase 1.5: Parallel mode tests
    // =========================================================================

    #[test]
    fn test_parallel_mode_default() {
        assert_eq!(ParallelMode::default(), ParallelMode::Enabled);
    }

    #[test]
    fn test_config_default_parallel_mode() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert_eq!(config.parallel_mode(), ParallelMode::Enabled);
    }

    #[test]
    fn test_config_with_parallel_disabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_parallel_mode(ParallelMode::Disabled);

        assert_eq!(config.parallel_mode(), ParallelMode::Disabled);
    }

    #[test]
    fn test_config_with_parallel_aggressive() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_parallel_mode(ParallelMode::Aggressive);

        assert_eq!(config.parallel_mode(), ParallelMode::Aggressive);
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

    // =========================================================================
    // Permission skip tests
    // =========================================================================

    #[test]
    fn test_config_default_skip_permissions() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(!config.skip_permissions());
    }

    #[test]
    fn test_config_with_skip_permissions() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_skip_permissions(true);

        assert!(config.skip_permissions());
    }

    // =========================================================================
    // Print mode and initial prompt tests
    // =========================================================================

    #[test]
    fn test_config_default_initial_prompt() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(config.initial_prompt().is_none());
        assert!(!config.print_mode());
    }

    #[test]
    fn test_config_with_initial_prompt() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_initial_prompt("list the files in this directory");

        assert_eq!(
            config.initial_prompt(),
            Some("list the files in this directory")
        );
    }

    #[test]
    fn test_config_with_print_mode() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_print_mode(true);

        assert!(config.print_mode());
    }

    #[test]
    fn test_config_print_mode_with_prompt() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_initial_prompt("explain this code")
            .with_print_mode(true);

        assert_eq!(config.initial_prompt(), Some("explain this code"));
        assert!(config.print_mode());
    }

    // =========================================================================
    // Vision model tests
    // =========================================================================

    #[test]
    fn test_config_default_vision_model() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(config.vision_model().is_none());
    }

    #[test]
    fn test_config_with_vision_model() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_vision_model("claude-sonnet-4-20250514");

        assert_eq!(config.vision_model(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_config_vision_model_string_conversion() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_vision_model(String::from("claude-opus-4-20250514"));

        assert_eq!(config.vision_model(), Some("claude-opus-4-20250514"));
    }

    // =========================================================================
    // OAuth client_id tests (0.8.4)
    // =========================================================================

    #[test]
    fn test_config_default_oauth_client_id() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(config.oauth_client_id().is_none());
    }

    #[test]
    fn test_config_with_oauth_client_id() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_oauth_client_id("12345678-1234-1234-1234-123456789abc");

        assert_eq!(
            config.oauth_client_id(),
            Some("12345678-1234-1234-1234-123456789abc")
        );
    }

    #[test]
    fn test_config_oauth_client_id_string_conversion() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_oauth_client_id(String::from("uuid-from-string"));

        assert_eq!(config.oauth_client_id(), Some("uuid-from-string"));
    }
}
