//! Configuration types for Patina.
//!
//! This module contains configuration structures used to initialize
//! and configure the application.

use secrecy::SecretString;
use std::path::PathBuf;
use std::time::Duration;

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

/// Configuration for context compression.
///
/// Controls how context is compressed and cached for efficient token usage.
/// These settings affect the behavior of the [`CompressionOrchestrator`].
///
/// # Default Values
///
/// - `cache_ttl`: 5 minutes (300 seconds)
/// - `max_cache_entries`: 100
/// - `max_context_tokens`: 10,000
///
/// # Example
///
/// ```
/// use patina::types::config::CompressionConfig;
/// use std::time::Duration;
///
/// // Use defaults
/// let config = CompressionConfig::default();
/// assert_eq!(config.cache_ttl, Duration::from_secs(300));
///
/// // Custom configuration
/// let config = CompressionConfig {
///     cache_ttl: Duration::from_secs(600),
///     max_cache_entries: 200,
///     max_context_tokens: 20000,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionConfig {
    /// Time-to-live for cached compression results.
    ///
    /// Results are automatically invalidated after this duration.
    /// Note: Results are also invalidated when the repository hash changes.
    pub cache_ttl: Duration,

    /// Maximum number of cache entries to keep.
    ///
    /// When this limit is reached, the oldest 10% of entries are evicted.
    pub max_cache_entries: usize,

    /// Maximum tokens to include in auto-injected context.
    ///
    /// When using `build_context_with_budget()`, this is the default budget.
    pub max_context_tokens: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            cache_ttl: Duration::from_secs(300), // 5 minutes
            max_cache_entries: 100,
            max_context_tokens: 10_000,
        }
    }
}

/// Identifies which LLM provider to use.
///
/// Defaults to [`ProviderKind::Anthropic`] for direct Anthropic API access.
/// Use [`ProviderKind::OpenRouter`] to route through OpenRouter with OpenAI-compatible
/// message format translation.
///
/// # Examples
///
/// ```
/// use patina::types::config::ProviderKind;
///
/// let kind = ProviderKind::default();
/// assert_eq!(kind, ProviderKind::Anthropic);
/// assert_eq!(kind.as_str(), "anthropic");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderKind {
    /// Use the Anthropic API directly.
    #[default]
    Anthropic,

    /// Use OpenRouter (OpenAI-compatible API with message format translation).
    OpenRouter,

    /// Use a fallback chain that tries multiple providers in order.
    Fallback,
}

impl ProviderKind {
    /// Returns the provider name as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::types::config::ProviderKind;
    ///
    /// assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
    /// assert_eq!(ProviderKind::OpenRouter.as_str(), "openrouter");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::Fallback => "fallback",
        }
    }
}

/// Configuration for LLM provider selection.
///
/// Determines which provider is used and holds provider-specific settings.
/// For Anthropic, the API key is inherited from the top-level [`Config::api_key`].
/// For OpenRouter, a separate API key and model are required.
///
/// # Examples
///
/// ```
/// use patina::types::config::{ProviderConfig, ProviderKind};
///
/// // Default: Anthropic
/// let config = ProviderConfig::default();
/// assert_eq!(config.kind, ProviderKind::Anthropic);
///
/// // OpenRouter with API key
/// use secrecy::SecretString;
/// let config = ProviderConfig::openrouter(
///     SecretString::new("sk-or-...".into()),
///     "anthropic/claude-sonnet-4",
/// );
/// assert_eq!(config.kind, ProviderKind::OpenRouter);
/// ```
#[derive(Clone)]
pub struct ProviderConfig {
    /// Which provider to use.
    pub kind: ProviderKind,

    /// API key for OpenRouter (required when `kind` is `OpenRouter`).
    pub openrouter_api_key: Option<SecretString>,

    /// Model identifier for OpenRouter (e.g., "anthropic/claude-sonnet-4").
    ///
    /// Required when `kind` is `OpenRouter`. This is separate from
    /// [`Config::model`] because OpenRouter uses different model identifiers.
    pub openrouter_model: Option<String>,

    /// Site URL sent as `HTTP-Referer` to OpenRouter for analytics.
    pub site_url: Option<String>,

    /// App name sent as `X-Title` to OpenRouter for analytics.
    pub app_name: Option<String>,

    /// Ordered list of provider configs for fallback chains.
    ///
    /// Only used when `kind` is [`ProviderKind::Fallback`]. Providers are
    /// tried in order — the first to succeed handles the request.
    pub fallback_chain: Vec<ProviderConfig>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::anthropic()
    }
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("kind", &self.kind)
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("openrouter_model", &self.openrouter_model)
            .field("site_url", &self.site_url)
            .field("app_name", &self.app_name)
            .field("fallback_chain", &self.fallback_chain)
            .finish()
    }
}

impl ProviderConfig {
    /// Creates a provider config for direct Anthropic API access.
    ///
    /// The API key is inherited from [`Config::api_key`].
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::types::config::{ProviderConfig, ProviderKind};
    ///
    /// let config = ProviderConfig::anthropic();
    /// assert_eq!(config.kind, ProviderKind::Anthropic);
    /// ```
    #[must_use]
    pub fn anthropic() -> Self {
        Self {
            kind: ProviderKind::Anthropic,
            openrouter_api_key: None,
            openrouter_model: None,
            site_url: None,
            app_name: None,
            fallback_chain: Vec::new(),
        }
    }

    /// Creates a provider config for OpenRouter.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The OpenRouter API key
    /// * `model` - The OpenRouter model identifier (e.g., "anthropic/claude-sonnet-4")
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::types::config::{ProviderConfig, ProviderKind};
    /// use secrecy::SecretString;
    ///
    /// let config = ProviderConfig::openrouter(
    ///     SecretString::new("sk-or-...".into()),
    ///     "anthropic/claude-sonnet-4",
    /// );
    /// assert_eq!(config.kind, ProviderKind::OpenRouter);
    /// ```
    #[must_use]
    pub fn openrouter(api_key: SecretString, model: impl Into<String>) -> Self {
        Self {
            kind: ProviderKind::OpenRouter,
            openrouter_api_key: Some(api_key),
            openrouter_model: Some(model.into()),
            site_url: None,
            app_name: None,
            fallback_chain: Vec::new(),
        }
    }

    /// Creates a fallback provider config from an ordered list of provider configs.
    ///
    /// Providers are tried in order — the first to succeed handles the request.
    ///
    /// # Arguments
    ///
    /// * `chain` - Ordered list of provider configs to try
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::types::config::{ProviderConfig, ProviderKind};
    /// use secrecy::SecretString;
    ///
    /// let config = ProviderConfig::fallback(vec![
    ///     ProviderConfig::anthropic(),
    ///     ProviderConfig::openrouter(
    ///         SecretString::new("sk-or-...".into()),
    ///         "anthropic/claude-sonnet-4",
    ///     ),
    /// ]);
    /// assert_eq!(config.kind, ProviderKind::Fallback);
    /// ```
    #[must_use]
    pub fn fallback(chain: Vec<ProviderConfig>) -> Self {
        Self {
            kind: ProviderKind::Fallback,
            openrouter_api_key: None,
            openrouter_model: None,
            site_url: None,
            app_name: None,
            fallback_chain: chain,
        }
    }

    /// Sets the site URL for OpenRouter analytics.
    ///
    /// Sent as the `HTTP-Referer` header with OpenRouter requests.
    #[must_use]
    pub fn with_site_url(mut self, url: impl Into<String>) -> Self {
        self.site_url = Some(url.into());
        self
    }

    /// Sets the app name for OpenRouter analytics.
    ///
    /// Sent as the `X-Title` header with OpenRouter requests.
    #[must_use]
    pub fn with_app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }

    /// Validates the provider configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `kind` is `OpenRouter` but `openrouter_api_key` is `None`
    /// - `kind` is `OpenRouter` but `openrouter_model` is `None`
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::types::config::ProviderConfig;
    ///
    /// let config = ProviderConfig::anthropic();
    /// assert!(config.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        match self.kind {
            ProviderKind::Anthropic => Ok(()),
            ProviderKind::OpenRouter => {
                if self.openrouter_api_key.is_none() {
                    return Err(
                        "OpenRouter provider requires an API key. Set OPENROUTER_API_KEY \
                         environment variable or configure openrouter_api_key in config."
                            .to_string(),
                    );
                }
                if self.openrouter_model.is_none() {
                    return Err("OpenRouter provider requires a model identifier \
                         (e.g., 'anthropic/claude-sonnet-4')."
                        .to_string());
                }
                Ok(())
            }
            ProviderKind::Fallback => {
                if self.fallback_chain.is_empty() {
                    return Err(
                        "Fallback provider requires at least one provider in the chain."
                            .to_string(),
                    );
                }
                for (i, provider) in self.fallback_chain.iter().enumerate() {
                    provider.validate().map_err(|e| {
                        format!(
                            "Fallback chain provider {i} ({}) invalid: {e}",
                            provider.kind.as_str()
                        )
                    })?;
                }
                Ok(())
            }
        }
    }
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
/// use patina::types::config::{CompressionConfig, Config, NarsilMode, ParallelMode, ProviderConfig, ResumeMode};
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
///     initial_images: Vec::new(),
///     plugins_enabled: true,
///     subagents_enabled: false,
///     ide_port: None,
///     auto_context_enabled: true,
///     compression: CompressionConfig::default(),
///     provider: ProviderConfig::default(),
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

    /// Optional image paths to include in the initial message.
    ///
    /// When set, these images are loaded and included as `ContentBlock::Image`
    /// in the first user message. Supports PNG, JPEG, GIF, and WebP formats.
    pub initial_images: Vec<PathBuf>,

    /// Whether to load plugins on startup.
    ///
    /// When true (default), plugins are loaded from standard locations:
    /// - `~/.config/patina/plugins/` (user plugins)
    /// - `./.patina/plugins/` (project-local plugins)
    ///
    /// Disable with `--no-plugins` CLI flag.
    pub plugins_enabled: bool,

    /// Whether subagent orchestration is enabled.
    ///
    /// When true, the `SubagentSpawner` is initialized and subagents can be
    /// spawned for parallel task execution. Disabled by default.
    ///
    /// Enable with `--enable-subagents` CLI flag.
    pub subagents_enabled: bool,

    /// Optional port for IDE integration server.
    ///
    /// When set, starts a TCP server on the specified port for IDE extensions
    /// to communicate with Patina. Used for VS Code and JetBrains integration.
    ///
    /// Enable with `--ide-port <PORT>` CLI flag.
    pub ide_port: Option<u16>,

    /// Whether to auto-inject context suggestions from narsil.
    ///
    /// When true and narsil is connected, code references in user messages
    /// are automatically analyzed and relevant context (callers, dependencies)
    /// is injected before API calls.
    ///
    /// Default: `true` (enabled when narsil is available)
    ///
    /// Disable with `--no-auto-context` CLI flag.
    pub auto_context_enabled: bool,

    /// Context compression configuration.
    ///
    /// Controls cache TTL, max entries, and token budget for context compression.
    /// Used when creating the `CompressionOrchestrator`.
    pub compression: CompressionConfig,

    /// LLM provider configuration.
    ///
    /// Controls which provider is used (Anthropic, OpenRouter) and
    /// provider-specific settings like API keys and model identifiers.
    pub provider: ProviderConfig,
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
            initial_images: Vec::new(),
            plugins_enabled: true,
            subagents_enabled: false,
            ide_port: None,
            auto_context_enabled: true,
            compression: CompressionConfig::default(),
            provider: ProviderConfig::default(),
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

    /// Sets the initial images to include in the first message.
    ///
    /// Images are loaded and sent as `ContentBlock::Image` along with
    /// the initial prompt.
    ///
    /// # Arguments
    ///
    /// * `images` - Paths to image files (PNG, JPEG, GIF, or WebP)
    #[must_use]
    pub fn with_initial_images(mut self, images: Vec<PathBuf>) -> Self {
        self.initial_images = images;
        self
    }

    /// Returns the initial image paths.
    #[must_use]
    pub fn initial_images(&self) -> &[PathBuf] {
        &self.initial_images
    }

    /// Enables subagent orchestration.
    ///
    /// When enabled, the `SubagentSpawner` is initialized and subagents can be
    /// spawned for parallel task execution.
    ///
    /// # Arguments
    ///
    /// * `enabled` - If true, enable subagent orchestration
    #[must_use]
    pub fn with_subagents_enabled(mut self, enabled: bool) -> Self {
        self.subagents_enabled = enabled;
        self
    }

    /// Returns whether subagent orchestration is enabled.
    #[must_use]
    pub fn subagents_enabled(&self) -> bool {
        self.subagents_enabled
    }

    /// Enables or disables auto-context injection from narsil.
    ///
    /// When enabled and narsil is connected, code references in user messages
    /// are automatically analyzed and relevant context is injected before API calls.
    ///
    /// # Arguments
    ///
    /// * `enabled` - If true, enable auto-context injection
    #[must_use]
    pub fn with_auto_context_enabled(mut self, enabled: bool) -> Self {
        self.auto_context_enabled = enabled;
        self
    }

    /// Returns whether auto-context injection is enabled.
    #[must_use]
    pub fn auto_context_enabled(&self) -> bool {
        self.auto_context_enabled
    }

    /// Sets the LLM provider configuration.
    ///
    /// # Arguments
    ///
    /// * `provider` - The provider configuration specifying which provider to use
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use patina::types::config::{Config, ProviderConfig};
    /// use secrecy::SecretString;
    /// use std::path::PathBuf;
    ///
    /// let config = Config::new(
    ///     SecretString::new("sk-ant-api...".into()),
    ///     "claude-sonnet-4",
    ///     PathBuf::from("."),
    /// ).with_provider(ProviderConfig::openrouter(
    ///     SecretString::new("sk-or-...".into()),
    ///     "anthropic/claude-sonnet-4",
    /// ));
    /// ```
    #[must_use]
    pub fn with_provider(mut self, provider: ProviderConfig) -> Self {
        self.provider = provider;
        self
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
            initial_images: Vec::new(),
            plugins_enabled: true,
            subagents_enabled: false,
            ide_port: None,
            auto_context_enabled: true,
            compression: CompressionConfig::default(),
            provider: ProviderConfig::default(),
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
            initial_images: Vec::new(),
            plugins_enabled: true,
            subagents_enabled: false,
            ide_port: None,
            auto_context_enabled: true,
            compression: CompressionConfig::default(),
            provider: ProviderConfig::default(),
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

    // =========================================================================
    // Initial images tests (1.5.1.4)
    // =========================================================================

    #[test]
    fn test_config_default_initial_images() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(config.initial_images().is_empty());
    }

    #[test]
    fn test_config_with_initial_images() {
        let images = vec![PathBuf::from("photo1.png"), PathBuf::from("photo2.jpg")];
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_initial_images(images.clone());

        assert_eq!(config.initial_images(), &images);
    }

    #[test]
    fn test_config_initial_images_empty_vec() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_initial_images(Vec::new());

        assert!(config.initial_images().is_empty());
    }

    // =========================================================================
    // Subagents enabled tests (1.5.4.3)
    // =========================================================================

    #[test]
    fn test_config_default_subagents_disabled() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert!(!config.subagents_enabled());
    }

    #[test]
    fn test_config_with_subagents_enabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_subagents_enabled(true);

        assert!(config.subagents_enabled());
    }

    #[test]
    fn test_config_with_subagents_disabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_subagents_enabled(false);

        assert!(!config.subagents_enabled());
    }

    // =========================================================================
    // Auto-context enabled tests (2.2.4)
    // =========================================================================

    #[test]
    fn test_config_default_auto_context_enabled() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        // Auto-context is enabled by default when narsil is available
        assert!(config.auto_context_enabled());
    }

    #[test]
    fn test_config_with_auto_context_disabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_auto_context_enabled(false);

        assert!(!config.auto_context_enabled());
    }

    #[test]
    fn test_config_with_auto_context_enabled() {
        let config = Config::new(SecretString::new("key".into()), "model", PathBuf::from("."))
            .with_auto_context_enabled(true);

        assert!(config.auto_context_enabled());
    }

    // =========================================================================
    // Phase 3: Compression config tests
    // =========================================================================

    #[test]
    fn test_compression_config_default() {
        let config = CompressionConfig::default();

        assert_eq!(config.cache_ttl, Duration::from_secs(300));
        assert_eq!(config.max_cache_entries, 100);
        assert_eq!(config.max_context_tokens, 10_000);
    }

    #[test]
    fn test_compression_config_custom() {
        let config = CompressionConfig {
            cache_ttl: Duration::from_secs(600),
            max_cache_entries: 200,
            max_context_tokens: 20_000,
        };

        assert_eq!(config.cache_ttl, Duration::from_secs(600));
        assert_eq!(config.max_cache_entries, 200);
        assert_eq!(config.max_context_tokens, 20_000);
    }

    #[test]
    fn test_config_has_compression_section() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        // Config should have default compression settings
        assert_eq!(config.compression.cache_ttl, Duration::from_secs(300));
        assert_eq!(config.compression.max_cache_entries, 100);
    }

    // =========================================================================
    // Phase 2.6: Provider config and selection tests
    // =========================================================================

    #[test]
    fn test_provider_kind_default_is_anthropic() {
        assert_eq!(ProviderKind::default(), ProviderKind::Anthropic);
    }

    #[test]
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::OpenRouter.as_str(), "openrouter");
    }

    #[test]
    fn test_provider_config_default() {
        let config = ProviderConfig::default();
        assert_eq!(config.kind, ProviderKind::Anthropic);
        assert!(config.openrouter_api_key.is_none());
        assert!(config.openrouter_model.is_none());
        assert!(config.site_url.is_none());
        assert!(config.app_name.is_none());
    }

    #[test]
    fn test_provider_config_anthropic_constructor() {
        let config = ProviderConfig::anthropic();
        assert_eq!(config.kind, ProviderKind::Anthropic);
    }

    #[test]
    fn test_provider_config_openrouter_constructor() {
        let config = ProviderConfig::openrouter(
            SecretString::new("sk-or-test".into()),
            "anthropic/claude-sonnet-4",
        );
        assert_eq!(config.kind, ProviderKind::OpenRouter);
        assert!(config.openrouter_api_key.is_some());
        assert_eq!(
            config.openrouter_model.as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
    }

    #[test]
    fn test_provider_config_openrouter_with_site_url() {
        let config = ProviderConfig::openrouter(SecretString::new("sk-or-test".into()), "model")
            .with_site_url("https://patina.dev");

        assert_eq!(config.site_url.as_deref(), Some("https://patina.dev"));
    }

    #[test]
    fn test_provider_config_openrouter_with_app_name() {
        let config = ProviderConfig::openrouter(SecretString::new("sk-or-test".into()), "model")
            .with_app_name("Patina");

        assert_eq!(config.app_name.as_deref(), Some("Patina"));
    }

    #[test]
    fn test_config_default_provider_is_anthropic() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        );

        assert_eq!(config.provider.kind, ProviderKind::Anthropic);
    }

    #[test]
    fn test_config_with_provider() {
        let provider_config = ProviderConfig::openrouter(
            SecretString::new("sk-or-test".into()),
            "anthropic/claude-sonnet-4",
        );
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        )
        .with_provider(provider_config);

        assert_eq!(config.provider.kind, ProviderKind::OpenRouter);
    }

    #[test]
    fn test_provider_config_validate_anthropic_ok() {
        // Anthropic config is always valid (API key is on Config, not ProviderConfig)
        let config = ProviderConfig::anthropic();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_provider_config_validate_openrouter_missing_key() {
        // OpenRouter config without API key should fail validation
        let config = ProviderConfig {
            kind: ProviderKind::OpenRouter,
            openrouter_api_key: None,
            openrouter_model: Some("model".to_string()),
            site_url: None,
            app_name: None,
            fallback_chain: Vec::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_provider_config_validate_openrouter_missing_model() {
        // OpenRouter config without model should fail validation
        let config = ProviderConfig {
            kind: ProviderKind::OpenRouter,
            openrouter_api_key: Some(SecretString::new("key".into())),
            openrouter_model: None,
            site_url: None,
            app_name: None,
            fallback_chain: Vec::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_provider_config_validate_openrouter_ok() {
        let config = ProviderConfig::openrouter(
            SecretString::new("sk-or-test".into()),
            "anthropic/claude-sonnet-4",
        );
        assert!(config.validate().is_ok());
    }

    // =========================================================================
    // Phase 2.7: Fallback provider config tests
    // =========================================================================

    #[test]
    fn test_provider_kind_fallback_as_str() {
        assert_eq!(ProviderKind::Fallback.as_str(), "fallback");
    }

    #[test]
    fn test_provider_config_fallback_constructor() {
        let config = ProviderConfig::fallback(vec![
            ProviderConfig::anthropic(),
            ProviderConfig::openrouter(
                SecretString::new("sk-or-test".into()),
                "anthropic/claude-sonnet-4",
            ),
        ]);
        assert_eq!(config.kind, ProviderKind::Fallback);
        assert_eq!(config.fallback_chain.len(), 2);
    }

    #[test]
    fn test_provider_config_fallback_validate_ok() {
        let config = ProviderConfig::fallback(vec![
            ProviderConfig::anthropic(),
            ProviderConfig::openrouter(
                SecretString::new("sk-or-test".into()),
                "anthropic/claude-sonnet-4",
            ),
        ]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_provider_config_fallback_validate_empty_chain() {
        let config = ProviderConfig::fallback(vec![]);
        assert!(config.validate().is_err());
        let err = config.validate().unwrap_err();
        assert!(err.contains("at least one provider"));
    }

    #[test]
    fn test_provider_config_fallback_validate_invalid_inner() {
        let config = ProviderConfig::fallback(vec![
            ProviderConfig::anthropic(),
            ProviderConfig {
                kind: ProviderKind::OpenRouter,
                openrouter_api_key: None,
                openrouter_model: None,
                site_url: None,
                app_name: None,
                fallback_chain: Vec::new(),
            },
        ]);
        assert!(config.validate().is_err());
        let err = config.validate().unwrap_err();
        assert!(err.contains("Fallback chain provider 1"));
    }

    #[test]
    fn test_provider_config_fallback_single_provider() {
        let config = ProviderConfig::fallback(vec![ProviderConfig::anthropic()]);
        assert!(config.validate().is_ok());
        assert_eq!(config.fallback_chain.len(), 1);
    }

    #[test]
    fn test_config_with_fallback_provider() {
        let config = Config::new(
            SecretString::new("test-key".into()),
            "test-model",
            PathBuf::from("/tmp"),
        )
        .with_provider(ProviderConfig::fallback(vec![ProviderConfig::anthropic()]));

        assert_eq!(config.provider.kind, ProviderKind::Fallback);
    }
}
