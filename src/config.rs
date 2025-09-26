use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use chrono::Duration;

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[clap(name = "log-sdk-purger", version, author, about)]
pub struct Config {
    /// Database URL (can use env: DATABASE_URL)
    #[clap(long, env = "DATABASE_URL", hide_env_values = true)]
    pub database_url: String,

    /// Deletion strategy
    #[clap(long, default_value = "adaptive", value_enum)]
    pub strategy: PurgeStrategy,

    /// Batch size (rows per deletion)
    #[clap(short, long, default_value = "500")]
    pub batch_size: u32,

    /// Base sleep between batches (milliseconds)
    #[clap(long, default_value = "200")]
    pub sleep_ms: u64,

    /// Maximum server connections before throttling
    #[clap(long, default_value = "30")]
    pub max_connections: u32,

    /// Data retention period (days)
    #[clap(short, long, default_value = "90")]
    pub retention_days: i64,

    /// Dry run mode (no actual deletions)
    #[clap(long)]
    pub dry_run: bool,

    /// Filter by proceso
    #[clap(long)]
    pub proceso: Option<String>,

    /// Filter by user
    #[clap(long)]
    pub user: Option<String>,

    /// Maximum runtime in minutes (0 = unlimited)
    #[clap(long, default_value = "0")]
    pub max_runtime_minutes: u64,

    /// Enable detailed progress bar
    #[clap(long)]
    pub progress: bool,

    /// Log level
    #[clap(long, default_value = "info")]
    pub log_level: String,

    /// Metrics port (0 = disabled)
    #[clap(long, default_value = "9090")]
    pub metrics_port: u16,

    /// Configuration file path
    #[clap(long)]
    pub config_file: Option<PathBuf>,

    /// Maximum replication lag in seconds
    #[clap(long, default_value = "60")]
    pub max_replication_lag: u64,

    /// Minimum free disk space in GB
    #[clap(long, default_value = "10")]
    pub min_disk_space_gb: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum)]
pub enum PurgeStrategy {
    /// Use ID-based deletion (fastest)
    Id,
    /// Use fecha-based deletion (respects date order)
    Fecha,
    /// Adaptive strategy (best of both)
    Adaptive,
    /// Partition-based (if table is partitioned)
    Partition,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        dotenv::dotenv().ok();

        let mut config = Config::parse();

        // Load from config file if specified
        if let Some(path) = &config.config_file {
            let contents = std::fs::read_to_string(path)?;
            let file_config: Config = toml::from_str(&contents)?;
            config.merge(file_config);
        }

        config.validate()?;
        Ok(config)
    }

    fn merge(&mut self, other: Config) {
        // Merge configuration from file with CLI args
        // CLI args take precedence
        if self.database_url.is_empty() {
            self.database_url = other.database_url;
        }
        // ... merge other fields
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.batch_size == 0 || self.batch_size > 10000 {
            anyhow::bail!("Batch size must be between 1 and 10000");
        }

        if self.retention_days < 1 {
            anyhow::bail!("Retention days must be at least 1");
        }

        Ok(())
    }

    pub fn retention_date(&self) -> chrono::NaiveDateTime {
        let duration = Duration::days(self.retention_days);
        (chrono::Utc::now() - duration).naive_utc()
    }
}