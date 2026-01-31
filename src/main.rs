//! Patina - High-performance terminal client for Claude API

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Use the library crate
use patina::app;
use patina::session::{default_sessions_dir, format_session_list, SessionManager};
use patina::types::config::{NarsilMode, ResumeMode};

#[derive(Parser, Debug)]
#[command(name = "patina")]
#[command(about = "Patina - High-performance terminal client for Claude API")]
#[command(version)]
struct Args {
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

    /// Resume a previous session. Use "last" to resume the most recent session,
    /// or provide a specific session ID.
    #[arg(long, value_name = "SESSION")]
    resume: Option<String>,

    /// List all available sessions and exit.
    #[arg(long)]
    list_sessions: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --list-sessions before any other initialization
    if args.list_sessions {
        return list_sessions().await;
    }

    let filter = if args.debug { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let api_key = args
        .api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().map(Into::into))
        .ok_or_else(|| {
            anyhow::anyhow!("API key required. Set ANTHROPIC_API_KEY or use --api-key")
        })?;

    // Determine narsil mode from CLI flags
    let narsil_mode = if args.with_narsil {
        NarsilMode::Enabled
    } else if args.no_narsil {
        NarsilMode::Disabled
    } else {
        NarsilMode::Auto
    };

    // Determine resume mode from CLI flag
    let resume_mode = match args.resume.as_deref() {
        Some("last") => ResumeMode::Last,
        Some(session_id) => ResumeMode::SessionId(session_id.to_string()),
        None => ResumeMode::None,
    };

    app::run(app::Config {
        api_key,
        model: args.model,
        working_dir: args.directory,
        narsil_mode,
        resume_mode,
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
