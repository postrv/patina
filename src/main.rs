//! Rust Claude Terminal - High-performance CLI for Claude API

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Use the library crate
use rct::app;

#[derive(Parser, Debug)]
#[command(name = "rct")]
#[command(about = "High-performance terminal client for Claude API")]
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

    app::run(app::Config {
        api_key,
        model: args.model,
        working_dir: args.directory,
    })
    .await
}
