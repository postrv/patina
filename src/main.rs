//! Patina - High-performance terminal client for Claude API

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Use the library crate
use patina::app;
use patina::auth::{flow::OAuthFlow, storage as auth_storage};
use patina::session::{default_sessions_dir, format_session_list, SessionManager};
use patina::types::config::{NarsilMode, ResumeMode};

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
        resume_mode,
        skip_permissions: args.dangerously_skip_permissions,
        initial_prompt,
        print_mode,
        vision_model: None,
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
