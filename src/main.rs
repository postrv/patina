//! Patina - High-performance terminal client for Claude API

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Use the library crate
use patina::app;
use patina::auth::{flow::OAuthFlow, storage as auth_storage};
use patina::session::{default_sessions_dir, format_session_list, SessionManager};
use patina::types::config::{NarsilMode, ParallelMode, ResumeMode};

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --list-sessions before any other initialization
    if args.list_sessions {
        return list_sessions().await;
    }

    // Handle --oauth-logout before other initialization
    if args.oauth_logout {
        return oauth_logout().await;
    }

    // Handle --oauth-login before other initialization
    if args.oauth_login {
        return oauth_login().await;
    }

    let filter = if args.debug { "debug" } else { "info" };

    // Determine if we're running in interactive TUI mode
    // TUI mode uses alternate screen which conflicts with stdout logging
    let is_tui_mode = !args.print || args.prompt.is_none();

    if is_tui_mode && args.debug {
        // TUI mode with debug: write logs to file to avoid corrupting display
        let log_path = std::env::temp_dir().join("patina.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .expect("Failed to open log file");

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

        eprintln!("Debug logs written to: {}", log_path.display());
    } else {
        // Print mode or no debug: log to stdout
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| filter.into()),
            )
            .with(tracing_subscriber::fmt::layer().with_target(false))
            .init();
    }

    // Determine authentication method
    // Currently API key only (OAuth is disabled pending client_id registration)
    let api_key = args
        .api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().map(Into::into))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "API key required. Set ANTHROPIC_API_KEY environment variable or use --api-key flag.\n\
                 Get your API key at: https://console.anthropic.com/settings/keys"
            )
        })?;

    // Determine narsil mode from CLI flags
    let narsil_mode = if args.with_narsil {
        NarsilMode::Enabled
    } else if args.no_narsil {
        NarsilMode::Disabled
    } else {
        NarsilMode::Auto
    };

    // Determine parallel mode from CLI flags
    let parallel_mode = if args.no_parallel {
        ParallelMode::Disabled
    } else if args.parallel_aggressive {
        ParallelMode::Aggressive
    } else {
        ParallelMode::Enabled
    };

    // Determine resume mode from CLI flags
    let resume_mode = if args.continue_session {
        ResumeMode::Last
    } else {
        match args.resume.as_deref() {
            Some(session_id) => ResumeMode::SessionId(session_id.to_string()),
            None => ResumeMode::None,
        }
    };

    // Determine execution mode:
    // - print mode (-p) with prompt: non-interactive (send prompt, print response, exit)
    // - prompt only: interactive mode with initial prompt pre-submitted
    // - no prompt: interactive mode
    let (initial_prompt, print_mode) = match (args.prompt, args.print) {
        (Some(prompt), true) => (Some(prompt), true), // Non-interactive
        (Some(prompt), false) => (Some(prompt), false), // Interactive with initial prompt
        (None, true) => {
            // -p without prompt reads from stdin (not yet implemented)
            eprintln!("Error: --print requires a prompt argument or piped input");
            std::process::exit(1);
        }
        (None, false) => (None, false), // Pure interactive
    };

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
    })
    .await
}

/// Lists all available sessions and exits.
async fn list_sessions() -> Result<()> {
    let sessions_dir = default_sessions_dir()?;
    let manager = SessionManager::new(sessions_dir);

    let sessions = manager.list_sorted().await?;
    let output = format_session_list(&sessions);

    println!("{output}");

    Ok(())
}

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

/// Clears stored OAuth credentials.
async fn oauth_logout() -> Result<()> {
    auth_storage::clear_oauth_credentials().await?;
    println!("OAuth credentials cleared from system keychain.");
    Ok(())
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
}
