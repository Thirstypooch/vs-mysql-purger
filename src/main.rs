mod config;
mod database;
mod error;
mod metrics;
mod purger;

use anyhow::Result;
use signal_hook::{consts::SIGINT, consts::SIGTERM, iterator::Signals};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_tracing()?;

    // Load configuration
    let config = config::Config::load()?;

    // Print banner
    print_banner();

    // Create purger
    let purger = Arc::new(purger::LogPurger::new(config).await?);

    // Setup signal handlers
    let purger_signals = Arc::clone(&purger);
    tokio::spawn(async move {
        let mut signals = Signals::new(&[SIGINT, SIGTERM])
            .expect("Failed to create signal handler");
        for sig in signals.forever() {
            match sig {
                SIGINT | SIGTERM => {
                    info!("Received shutdown signal");
                    purger_signals.stop();
                    break;
                }
                _ => {}
            }
        }
    });

    // Run purger
    match purger.run().await {
        Ok(()) => {
            info!("✅ Purge completed successfully");
            std::process::exit(0);
        }
        Err(e) => {
            error!("❌ Purge failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn init_tracing() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact();

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();

    Ok(())
}

fn print_banner() {
    println!(r#"
╔═══════════════════════════════════════╗
║     LOG SDK PURGER v1.0.0            ║
║     High-Performance Rust Edition     ║
╚═══════════════════════════════════════╝
    "#);
}