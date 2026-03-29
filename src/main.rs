//! Patina - High-performance terminal client for Claude API

use std::io::IsTerminal;

use anyhow::Result;
use clap::{Parser, Subcommand};

// Use the library crate
use patina::app;
#[cfg(feature = "oauth")]
use patina::auth::{flow::OAuthFlow, storage as auth_storage};
use patina::plugins::registry::{PluginInstaller, PluginSource};
use patina::session::{default_sessions_dir, format_session_list, SessionManager};
use patina::types::config::{CompressionConfig, NarsilMode, ParallelMode, ResumeMode};
use patina::util::get_cache_dir;

#[derive(Parser, Debug)]
#[command(name = "patina")]
#[command(about = "Patina - High-performance terminal client for Claude API")]
#[command(version)]
struct Args {
    /// Initial prompt to start the conversation with.
    /// Starts interactive mode with this prompt pre-submitted.
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,

    /// Print mode: send prompt, print response, then exit (non-interactive).
    /// When combined with a prompt, runs in headless mode.
    #[arg(short = 'p', long)]
    print: bool,

    /// API key (or set ANTHROPIC_API_KEY env var)
    #[arg(long, env = "ANTHROPIC_API_KEY", hide_env_values = true)]
    api_key: Option<secrecy::SecretString>,

    /// Model to use
    #[arg(short, long, default_value = "claude-sonnet-4-20250514")]
    model: String,

    /// Working directory
    #[arg(short = 'C', long, default_value = ".")]
    directory: std::path::PathBuf,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    /// Enable narsil-mcp integration (overrides auto-detection)
    #[arg(long, conflicts_with = "no_narsil")]
    with_narsil: bool,

    /// Disable narsil-mcp integration
    #[arg(long, conflicts_with = "with_narsil")]
    no_narsil: bool,

    /// Disable parallel tool execution (run all tools sequentially)
    #[arg(long, conflicts_with = "parallel_aggressive")]
    no_parallel: bool,

    /// Enable aggressive parallel execution (includes MCP tools)
    ///
    /// WARNING: Can cause race conditions with external tools.
    #[arg(long, conflicts_with = "no_parallel")]
    parallel_aggressive: bool,

    /// Continue the most recent conversation in the current directory.
    #[arg(short = 'c', long = "continue")]
    continue_session: bool,

    /// Resume a specific session by ID or name.
    #[arg(
        short = 'r',
        long,
        value_name = "SESSION",
        conflicts_with = "continue_session"
    )]
    resume: Option<String>,

    /// List all available sessions and exit.
    #[arg(long)]
    list_sessions: bool,

    /// Bypass all permission prompts (DANGEROUS: allows all tool executions without approval).
    #[arg(long)]
    dangerously_skip_permissions: bool,

    /// Start OAuth login flow for Claude subscription authentication.
    /// NOTE: OAuth is currently disabled pending client_id registration with Anthropic.
    #[arg(long, hide = true)]
    oauth_login: bool,

    /// Clear stored OAuth credentials and exit.
    /// NOTE: OAuth is currently disabled pending client_id registration with Anthropic.
    #[arg(long, hide = true)]
    oauth_logout: bool,

    /// Force use of API key even if OAuth credentials are available.
    /// NOTE: OAuth is currently disabled, so this flag has no effect.
    #[arg(long, hide = true)]
    use_api_key: bool,

    /// OAuth client ID for subscription authentication.
    /// Must be a valid UUID registered with Anthropic's developer program.
    #[arg(long, env = "PATINA_OAUTH_CLIENT_ID")]
    oauth_client_id: Option<String>,

    /// Image file(s) to include in the initial message.
    ///
    /// Can be specified multiple times to include multiple images.
    /// Supported formats: PNG, JPEG, GIF, WebP (max 20MB each).
    ///
    /// Example: patina --image screenshot.png "What's in this image?"
    #[arg(long, value_name = "PATH")]
    image: Vec<std::path::PathBuf>,

    /// Disable plugin loading on startup.
    ///
    /// Skips loading plugins from ~/.config/patina/plugins/ and ./.patina/plugins/.
    #[arg(long)]
    no_plugins: bool,

    /// Enable subagent orchestration for parallel task execution.
    ///
    /// When enabled, subagents can be spawned to handle complex tasks
    /// that benefit from parallel exploration or specialized roles.
    #[arg(long)]
    enable_subagents: bool,

    /// Start IDE integration server on the specified port.
    ///
    /// When set, a TCP server is started on 127.0.0.1:<PORT> for IDE
    /// extensions (VS Code, JetBrains) to communicate with Patina.
    #[arg(long, value_name = "PORT")]
    ide_port: Option<u16>,

    /// Disable auto-context injection from narsil.
    ///
    /// When set, code references in user messages are not automatically
    /// analyzed for context suggestions (callers, dependencies).
    #[arg(long)]
    no_auto_context: bool,

    /// Disable automatic update checking on startup.
    #[arg(long)]
    no_update_check: bool,

    /// Bare mode: skip hooks, plugins, skills, auto-memory, narsil for faster startup.
    /// Still loads CLAUDE.md and permissions for safety.
    #[arg(long)]
    bare: bool,

    /// Reasoning effort level: auto, low, medium, high.
    ///
    /// Controls how much thinking the model applies per turn.
    /// Low = fast/cheap, High = thorough/expensive.
    #[arg(long, value_name = "LEVEL", default_value = "auto")]
    effort: String,

    /// LLM provider to use: "anthropic" (default), "openrouter", or "fallback".
    #[arg(long, value_name = "PROVIDER")]
    provider: Option<String>,

    /// Fallback provider chain (e.g., --fallback anthropic,openrouter).
    ///
    /// Comma-separated list of providers to try in order.
    /// Required when --provider=fallback.
    #[arg(long, value_name = "CHAIN", value_delimiter = ',')]
    fallback: Vec<String>,

    /// API key for OpenRouter (or set OPENROUTER_API_KEY env var).
    ///
    /// Required when --provider=openrouter.
    #[arg(long, env = "OPENROUTER_API_KEY", hide_env_values = true)]
    openrouter_key: Option<secrecy::SecretString>,

    /// Model identifier for OpenRouter (e.g., "anthropic/claude-sonnet-4").
    ///
    /// Required when --provider=openrouter.
    #[arg(long, value_name = "MODEL")]
    openrouter_model: Option<String>,

    /// Site URL sent to OpenRouter for analytics (HTTP-Referer header).
    #[arg(long, value_name = "URL")]
    openrouter_site_url: Option<String>,

    /// App name sent to OpenRouter for analytics (X-Title header).
    #[arg(long, value_name = "NAME")]
    openrouter_app_name: Option<String>,

    /// Subcommand for plugin and other operations.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Manage installed plugins.
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
}

/// Plugin management actions.
#[derive(Subcommand, Debug)]
enum PluginAction {
    /// Install a plugin from GitHub or local path.
    ///
    /// Examples:
    ///   patina plugin install gh:user/repo
    ///   patina plugin install gh:user/repo@v1.0.0
    ///   patina plugin install ./my-local-plugin
    Install {
        /// Plugin source (gh:user/repo[@version] or path).
        source: String,
    },

    /// List installed plugins.
    List,

    /// Update all installed plugins.
    Update,

    /// Remove an installed plugin.
    Remove {
        /// Name of the plugin to remove.
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle subcommands first
    if let Some(cmd) = args.command {
        return handle_command(cmd).await;
    }

    // Handle --list-sessions before any other initialization
    if args.list_sessions {
        return list_sessions().await;
    }

    // Handle --oauth-logout before other initialization
    #[cfg(feature = "oauth")]
    if args.oauth_logout {
        return oauth_logout().await;
    }

    // Handle --oauth-login before other initialization
    #[cfg(feature = "oauth")]
    if args.oauth_login {
        return oauth_login().await;
    }

    // Detect piped stdin (non-terminal input)
    let piped_input = if !std::io::stdin().is_terminal() {
        std::io::read_to_string(std::io::stdin())
            .ok()
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    setup_logging(args.debug, !args.print || args.prompt.is_none());

    let narsil_mode = resolve_narsil_mode(args.with_narsil, args.no_narsil);
    let parallel_mode = resolve_parallel_mode(args.no_parallel, args.parallel_aggressive);
    let resume_mode = resolve_resume_mode(args.continue_session, args.resume.as_deref());
    let provider_config = build_provider_config(&args)?;
    let api_key = resolve_api_key(args.api_key)?;
    let effort_level: patina::types::config::EffortLevel = args
        .effort
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;
    let (initial_prompt, print_mode) =
        resolve_execution_mode(args.prompt, args.print, piped_input)?;

    app::run(app::Config {
        api_key,
        model: args.model,
        working_dir: args.directory,
        narsil_mode,
        parallel_mode,
        resume_mode,
        skip_permissions: args.dangerously_skip_permissions,
        initial_prompt,
        print_mode,
        vision_model: None,
        oauth_client_id: args.oauth_client_id,
        initial_images: args.image,
        plugins_enabled: !args.no_plugins,
        subagents_enabled: args.enable_subagents,
        ide_port: args.ide_port,
        auto_context_enabled: !args.no_auto_context,
        effort: effort_level,
        thinking_budget: None,
        compression: CompressionConfig::default(),
        provider: provider_config,
        performance: patina::types::config::PerformanceConfig::default(),
        update_check_enabled: !args.no_update_check,
        bare_mode: args.bare,
    })
    .await
}

/// Attempts to open a log file at the given path.
///
/// Returns `Ok(file)` on success, or `Err` if the file cannot be opened.
/// This is extracted for testability of the fallback path.
///
/// # Errors
///
/// Returns an `std::io::Error` if the file cannot be created or opened.
fn open_log_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
}

/// Initializes the tracing subscriber for logging.
///
/// In TUI mode, logs go to a file (`$TMPDIR/patina.log`) to avoid corrupting
/// the ratatui alternate screen. If the log file cannot be opened, falls back
/// to stderr with a warning. In print mode, logs go to stderr.
fn setup_logging(debug: bool, is_tui_mode: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = if debug { "debug" } else { "info" };

    if is_tui_mode {
        let log_path = std::env::temp_dir().join("patina.log");
        match open_log_file(&log_path) {
            Ok(file) => {
                tracing_subscriber::registry()
                    .with(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| filter.into()),
                    )
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_target(false)
                            .with_ansi(false)
                            .with_writer(std::sync::Mutex::new(file)),
                    )
                    .init();
            }
            Err(e) => {
                eprintln!(
                    "Warning: Could not open log file {}: {e}. Falling back to stderr logging.",
                    log_path.display()
                );
                tracing_subscriber::registry()
                    .with(
                        tracing_subscriber::EnvFilter::try_from_default_env()
                            .unwrap_or_else(|_| filter.into()),
                    )
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_target(false)
                            .with_writer(std::io::stderr),
                    )
                    .init();
            }
        }
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| filter.into()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_writer(std::io::stderr),
            )
            .init();
    }
}

/// Resolves the API key from CLI argument or environment variable.
///
/// # Errors
///
/// Returns an error if no API key is found.
fn resolve_api_key(cli_key: Option<secrecy::SecretString>) -> Result<secrecy::SecretString> {
    cli_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().map(Into::into))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "API key required. Set ANTHROPIC_API_KEY environment variable or use --api-key flag.\n\
                 Get your API key at: https://console.anthropic.com/settings/keys"
            )
        })
}

/// Maps CLI narsil flags to `NarsilMode`.
#[must_use]
fn resolve_narsil_mode(with: bool, without: bool) -> NarsilMode {
    if with {
        NarsilMode::Enabled
    } else if without {
        NarsilMode::Disabled
    } else {
        NarsilMode::Auto
    }
}

/// Maps CLI parallel flags to `ParallelMode`.
#[must_use]
fn resolve_parallel_mode(no_parallel: bool, aggressive: bool) -> ParallelMode {
    if no_parallel {
        ParallelMode::Disabled
    } else if aggressive {
        ParallelMode::Aggressive
    } else {
        ParallelMode::Enabled
    }
}

/// Maps CLI resume flags to `ResumeMode`.
#[must_use]
fn resolve_resume_mode(continue_session: bool, resume: Option<&str>) -> ResumeMode {
    if continue_session {
        ResumeMode::Last
    } else {
        match resume {
            Some(session_id) => ResumeMode::SessionId(session_id.to_string()),
            None => ResumeMode::None,
        }
    }
}

/// Determines the execution mode from prompt, print flag, and piped stdin.
///
/// When piped input is present without an explicit prompt, print mode is
/// auto-enabled. When both piped input and a prompt are provided, they are
/// concatenated with the pipe content first.
///
/// # Errors
///
/// Returns an error if `--print` is used without a prompt and without
/// piped input.
fn resolve_execution_mode(
    prompt: Option<String>,
    print: bool,
    piped_input: Option<String>,
) -> Result<(Option<String>, bool)> {
    match (prompt, print, piped_input) {
        // Explicit prompt + piped input: combine (pipe first, then prompt)
        (Some(p), mode, Some(piped)) => {
            let combined = format!("{piped}\n\n{p}");
            Ok((Some(combined), mode))
        }
        // Explicit prompt, no pipe
        (Some(p), true, None) => Ok((Some(p), true)),
        (Some(p), false, None) => Ok((Some(p), false)),
        // No prompt, piped input: use as prompt, auto-enable print mode
        (None, _, Some(piped)) => Ok((Some(piped), true)),
        // No prompt, --print, no pipe: error
        (None, true, None) => {
            anyhow::bail!("--print requires a prompt argument or piped input");
        }
        // Interactive mode
        (None, false, None) => Ok((None, false)),
    }
}

/// Builds the provider configuration from CLI arguments.
///
/// # Errors
///
/// Returns an error for unknown providers or missing required fields.
fn build_provider_config(args: &Args) -> Result<patina::types::config::ProviderConfig> {
    match args.provider.as_deref() {
        Some("openrouter") => {
            let or_key = args
                .openrouter_key
                .clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok().map(Into::into))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OpenRouter provider requires an API key.\n\
                         Set OPENROUTER_API_KEY environment variable or use --openrouter-key flag."
                    )
                })?;
            let or_model = args.openrouter_model.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "OpenRouter provider requires a model.\n\
                     Use --openrouter-model (e.g., --openrouter-model anthropic/claude-sonnet-4)."
                )
            })?;
            let mut config = patina::types::config::ProviderConfig::openrouter(or_key, &or_model);
            if let Some(ref url) = args.openrouter_site_url {
                config = config.with_site_url(url.clone());
            }
            if let Some(ref name) = args.openrouter_app_name {
                config = config.with_app_name(name.clone());
            }
            Ok(config)
        }
        Some("fallback") => {
            if args.fallback.is_empty() {
                anyhow::bail!(
                    "Fallback provider requires --fallback flag.\n\
                     Example: --provider fallback --fallback anthropic,openrouter"
                );
            }

            let mut chain = Vec::new();
            for provider_name in &args.fallback {
                let pc = match provider_name.as_str() {
                    "anthropic" => patina::types::config::ProviderConfig::anthropic(),
                    "openrouter" => {
                        let or_key = args
                            .openrouter_key
                            .clone()
                            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok().map(Into::into))
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "OpenRouter in fallback chain requires an API key.\n\
                                     Set OPENROUTER_API_KEY or use --openrouter-key flag."
                                )
                            })?;
                        let or_model = args.openrouter_model.clone().ok_or_else(|| {
                            anyhow::anyhow!(
                                "OpenRouter in fallback chain requires a model.\n\
                                 Use --openrouter-model flag."
                            )
                        })?;
                        let mut pc =
                            patina::types::config::ProviderConfig::openrouter(or_key, &or_model);
                        if let Some(ref url) = args.openrouter_site_url {
                            pc = pc.with_site_url(url.clone());
                        }
                        if let Some(ref name) = args.openrouter_app_name {
                            pc = pc.with_app_name(name.clone());
                        }
                        pc
                    }
                    other => {
                        anyhow::bail!(
                            "Unknown provider '{}' in fallback chain. \
                             Supported: anthropic, openrouter",
                            other
                        );
                    }
                };
                chain.push(pc);
            }

            Ok(patina::types::config::ProviderConfig::fallback(chain))
        }
        Some("anthropic") | None => Ok(patina::types::config::ProviderConfig::anthropic()),
        Some(other) => {
            anyhow::bail!(
                "Unknown provider '{}'. Supported providers: anthropic, openrouter, fallback",
                other
            );
        }
    }
}

async fn list_sessions() -> Result<()> {
    let sessions_dir = default_sessions_dir()?;
    let manager = SessionManager::new(sessions_dir);

    let sessions = manager.list_sorted().await?;
    let output = format_session_list(&sessions);

    println!("{output}");

    Ok(())
}

#[cfg(feature = "oauth")]
/// Runs the OAuth login flow and stores credentials.
///
/// Note: OAuth is currently disabled pending client_id registration with Anthropic.
async fn oauth_login() -> Result<()> {
    let flow = OAuthFlow::new();

    // This will return an error explaining OAuth is disabled
    let credentials = flow.run().await?;

    println!("\nOAuth login successful!");
    println!("Access token stored in system keychain.");
    println!("Token expires at: {:?}", credentials.expires_at());

    Ok(())
}

#[cfg(feature = "oauth")]
/// Clears stored OAuth credentials.
async fn oauth_logout() -> Result<()> {
    auth_storage::clear_oauth_credentials().await?;
    println!("OAuth credentials cleared from system keychain.");
    Ok(())
}

/// Handles subcommands.
async fn handle_command(cmd: Command) -> Result<()> {
    match cmd {
        Command::Plugin { action } => handle_plugin_action(action).await,
    }
}

/// Returns the default plugin cache directory.
fn plugin_cache_dir() -> Result<std::path::PathBuf> {
    let cache_dir = get_cache_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?
        .join("installed-plugins");
    Ok(cache_dir)
}

/// Handles plugin management actions.
async fn handle_plugin_action(action: PluginAction) -> Result<()> {
    let cache_dir = plugin_cache_dir()?;

    match action {
        PluginAction::Install { source } => {
            let plugin_source = PluginSource::parse(&source)
                .map_err(|e| anyhow::anyhow!("Invalid plugin source: {e}"))?;

            let mut installer = PluginInstaller::new(&cache_dir)?;
            let installed = installer.install(&plugin_source)?;

            println!(
                "✓ Installed {} v{} to {}",
                installed.name,
                installed.version,
                installed.path.display()
            );
            Ok(())
        }

        PluginAction::List => {
            if !cache_dir.exists() {
                println!("No plugins installed.");
                return Ok(());
            }

            let installer = PluginInstaller::new(&cache_dir)?;
            let plugins = installer.list();

            if plugins.is_empty() {
                println!("No plugins installed.");
            } else {
                println!("Installed plugins:\n");
                for plugin in plugins {
                    println!("  {} v{}", plugin.name, plugin.version);
                    println!("    Path: {}", plugin.path.display());
                    println!();
                }
            }
            Ok(())
        }

        PluginAction::Update => {
            if !cache_dir.exists() {
                println!("No plugins installed.");
                return Ok(());
            }

            let mut installer = PluginInstaller::new(&cache_dir)?;
            let updated = installer.update_all()?;

            if updated.is_empty() {
                println!("All plugins are up to date.");
            } else {
                println!("Updated {} plugin(s):", updated.len());
                for name in updated {
                    println!("  ✓ {name}");
                }
            }
            Ok(())
        }

        PluginAction::Remove { name } => {
            if !cache_dir.exists() {
                anyhow::bail!("Plugin '{name}' not found.");
            }

            let mut installer = PluginInstaller::new(&cache_dir)?;
            let removed = installer.remove(&name)?;

            if removed {
                println!("✓ Removed plugin '{name}'");
            } else {
                anyhow::bail!("Plugin '{name}' not found.");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Test that the --image flag parses a single image path correctly.
    ///
    /// This test documents the expected CLI interface for vision support:
    /// - `--image <PATH>` should accept a file path
    /// - The path should be stored for later processing
    #[test]
    fn test_cli_image_flag_parsing() {
        let args = Args::parse_from(["patina", "--image", "screenshot.png"]);

        assert_eq!(args.image.len(), 1);
        assert_eq!(args.image[0], std::path::PathBuf::from("screenshot.png"));
    }

    /// Test that multiple --image flags can be used to pass multiple images.
    ///
    /// Claude Vision API supports up to 100 images per request, so users
    /// should be able to specify multiple images on the command line:
    /// - `patina --image a.png --image b.jpg --image c.gif`
    #[test]
    fn test_cli_image_multiple_images() {
        let args = Args::parse_from([
            "patina",
            "--image",
            "photo1.png",
            "--image",
            "photo2.jpg",
            "--image",
            "photo3.webp",
        ]);

        assert_eq!(args.image.len(), 3);
        assert_eq!(args.image[0], std::path::PathBuf::from("photo1.png"));
        assert_eq!(args.image[1], std::path::PathBuf::from("photo2.jpg"));
        assert_eq!(args.image[2], std::path::PathBuf::from("photo3.webp"));
    }

    /// Test that --image flag is optional (no images by default).
    #[test]
    fn test_cli_image_flag_optional() {
        let args = Args::parse_from(["patina"]);

        assert!(args.image.is_empty());
    }

    /// Test that --image can be combined with a prompt.
    ///
    /// Common use case: `patina --image photo.png "What's in this image?"`
    #[test]
    fn test_cli_image_with_prompt() {
        let args = Args::parse_from([
            "patina",
            "--image",
            "diagram.png",
            "Explain this architecture diagram",
        ]);

        assert_eq!(args.image.len(), 1);
        assert_eq!(args.image[0], std::path::PathBuf::from("diagram.png"));
        assert_eq!(
            args.prompt,
            Some("Explain this architecture diagram".to_string())
        );
    }

    // B4 tests: resolve_execution_mode with piped input

    /// Piped input with no prompt or flags should auto-enable print mode.
    #[test]
    fn test_resolve_piped_input_auto_print() {
        let (prompt, print) =
            resolve_execution_mode(None, false, Some("piped text".to_string())).unwrap();
        assert_eq!(prompt.as_deref(), Some("piped text"));
        assert!(print, "piped input should auto-enable print mode");
    }

    /// Piped input combined with an explicit prompt should concatenate them.
    #[test]
    fn test_resolve_piped_with_prompt() {
        let (prompt, print) = resolve_execution_mode(
            Some("fix the bug".to_string()),
            false,
            Some("diff output here".to_string()),
        )
        .unwrap();
        let text = prompt.unwrap();
        assert!(text.contains("diff output here"));
        assert!(text.contains("fix the bug"));
        assert!(!print);
    }

    /// Piped input with --print flag should use print mode.
    #[test]
    fn test_resolve_piped_with_print_flag() {
        let (prompt, print) =
            resolve_execution_mode(None, true, Some("piped text".to_string())).unwrap();
        assert_eq!(prompt.as_deref(), Some("piped text"));
        assert!(print);
    }

    /// No pipe, no prompt, --print should fail.
    #[test]
    fn test_resolve_no_input_print_fails() {
        let result = resolve_execution_mode(None, true, None);
        assert!(result.is_err());
    }

    /// No pipe, no prompt, no --print should be interactive mode.
    #[test]
    fn test_resolve_interactive_no_pipe() {
        let (prompt, print) = resolve_execution_mode(None, false, None).unwrap();
        assert!(prompt.is_none());
        assert!(!print);
    }

    /// Explicit prompt with --print and no pipe should work as before.
    #[test]
    fn test_resolve_prompt_print_no_pipe() {
        let (prompt, print) =
            resolve_execution_mode(Some("hello".to_string()), true, None).unwrap();
        assert_eq!(prompt.as_deref(), Some("hello"));
        assert!(print);
    }

    /// Explicit prompt without --print and no pipe should work as before.
    #[test]
    fn test_resolve_prompt_no_print_no_pipe() {
        let (prompt, print) =
            resolve_execution_mode(Some("hello".to_string()), false, None).unwrap();
        assert_eq!(prompt.as_deref(), Some("hello"));
        assert!(!print);
    }

    // =========================================================================
    // Bare mode CLI tests (Phase 2E)
    // =========================================================================

    /// Test that the --bare flag is recognized by the CLI parser.
    #[test]
    fn test_cli_bare_flag_parsing() {
        let args = Args::parse_from(["patina", "--bare"]);
        assert!(args.bare);
    }

    /// Test that --bare defaults to false when not specified.
    #[test]
    fn test_cli_bare_flag_default_false() {
        let args = Args::parse_from(["patina"]);
        assert!(!args.bare);
    }

    /// Test that --bare and --print can be combined.
    #[test]
    fn test_cli_bare_with_print_flag() {
        let args = Args::parse_from(["patina", "--bare", "--print", "hello"]);
        assert!(args.bare);
        assert!(args.print);
        assert_eq!(args.prompt, Some("hello".to_string()));
    }

    // =========================================================================
    // Log file open fallback tests (E-1)
    // =========================================================================

    /// Test that `open_log_file` succeeds with a valid writable path.
    #[test]
    fn test_open_log_file_success() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        let result = open_log_file(&path);
        assert!(result.is_ok(), "Should succeed for a valid writable path");
    }

    /// Test that `open_log_file` returns an error for an invalid path
    /// instead of panicking, verifying the fallback path is reachable.
    #[test]
    fn test_open_log_file_failure_returns_error() {
        let path = std::path::Path::new("/nonexistent/deeply/nested/dir/patina.log");
        let result = open_log_file(path);
        assert!(
            result.is_err(),
            "Should return Err for a path in a nonexistent directory"
        );
    }
}
