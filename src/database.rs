use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use sqlx::{Row, Executor};
use std::time::Duration;
use tracing::info;
use crate::error::{Result, PurgerError};

pub struct DatabaseManager {
    pool: MySqlPool,
}

impl DatabaseManager {
    pub async fn new(database_url: &str) -> Result<Self> {
        info!("Establishing database connection...");

        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(3600))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // Optimize session for bulk operations
                    conn.execute("SET SESSION foreign_key_checks = 0").await?;
                    conn.execute("SET SESSION unique_checks = 0").await?;
                    conn.execute("SET SESSION sql_mode = 'NO_ENGINE_SUBSTITUTION'").await?;
                    conn.execute("SET SESSION innodb_lock_wait_timeout = 50").await?;
                    conn.execute("SET SESSION transaction_isolation = 'READ COMMITTED'").await?;
                    conn.execute("SET SESSION autocommit = 1").await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await?;

        // Verify table structure
        Self::verify_table_structure(&pool).await?;

        Ok(Self { pool })
    }

    async fn verify_table_structure(pool: &MySqlPool) -> Result<()> {
        let result = sqlx::query(
            "SELECT COLUMN_NAME, DATA_TYPE, COLUMN_KEY
             FROM INFORMATION_SCHEMA.COLUMNS
             WHERE TABLE_SCHEMA = DATABASE()
             AND TABLE_NAME = 'log_sdk'
             ORDER BY ORDINAL_POSITION"
        )
            .fetch_all(pool)
            .await?;

        if result.is_empty() {
            return Err(PurgerError::Config("Table log_sdk not found".to_string()));
        }

        info!("Verified log_sdk table structure with {} columns", result.len());
        Ok(())
    }

    pub async fn check_server_health(&self) -> Result<ServerHealth> {
        let mut health = ServerHealth::default();

        // Check active connections
        let connections: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.PROCESSLIST
             WHERE COMMAND != 'Sleep' AND USER != 'event_scheduler'"
        )
            .fetch_one(&self.pool)
            .await?;

        health.active_connections = connections as u32;

        // Check replication lag (if applicable)
        let lag_result = sqlx::query("SHOW SLAVE STATUS")
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = lag_result {
            health.replication_lag_seconds = row.try_get("Seconds_Behind_Master").unwrap_or(0);
        }

        // Check disk space
        let disk_space: f64 = sqlx::query_scalar(
            "SELECT (data_free + data_length + index_length) / 1024 / 1024 / 1024
             FROM information_schema.tables
             WHERE table_schema = DATABASE() AND table_name = 'log_sdk'"
        )
            .fetch_one(&self.pool)
            .await?;

        health.free_disk_gb = disk_space;

        // Check for long-running queries
        let long_queries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.PROCESSLIST
             WHERE TIME > 60 AND COMMAND != 'Sleep'"
        )
            .fetch_one(&self.pool)
            .await?;

        health.long_running_queries = long_queries as u32;

        Ok(health)
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }
}

#[derive(Debug, Default)]
pub struct ServerHealth {
    pub active_connections: u32,
    pub replication_lag_seconds: u64,
    pub free_disk_gb: f64,
    pub long_running_queries: u32,
}

impl ServerHealth {
    pub fn is_healthy(&self, config: &crate::config::Config) -> Result<()> {
        if self.active_connections > config.max_connections {
            return Err(PurgerError::ServerOverload {
                current: self.active_connections,
                max: config.max_connections,
            });
        }

        if self.replication_lag_seconds > config.max_replication_lag {
            return Err(PurgerError::ReplicationLag {
                seconds: self.replication_lag_seconds,
            });
        }

        if self.free_disk_gb < config.min_disk_space_gb {
            return Err(PurgerError::LowDiskSpace {
                available_gb: self.free_disk_gb,
            });
        }

        Ok(())
    }
}