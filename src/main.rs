//! Patina - High-performance terminal client for Claude API

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Use the library crate
use patina::app;
use patina::types::config::NarsilMode;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

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

    app::run(app::Config {
        api_key,
        model: args.model,
        working_dir: args.directory,
        narsil_mode,
    })
    .await
}
