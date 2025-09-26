use crate::{
    config::{Config, PurgeStrategy},
    database::{DatabaseManager, ServerHealth},
    error::{Result, PurgerError},
    metrics::Metrics,
};
use chrono::NaiveDateTime;
use indicatif::{ProgressBar, ProgressStyle};
use parking_lot::Mutex;
use sqlx::Row;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::timeout;
use tracing::{debug, error, info, warn, instrument};

pub struct LogPurger {
    config: Arc<Config>,
    db: Arc<DatabaseManager>,
    metrics: Arc<Metrics>,
    running: Arc<AtomicBool>,
    stats: Arc<PurgeStats>,
    progress_bar: Option<Arc<Mutex<ProgressBar>>>,
}

#[derive(Default)]
pub struct PurgeStats {
    pub rows_deleted: AtomicU64,
    pub batches_completed: AtomicU64,
    pub bytes_freed: AtomicU64,
    pub errors: AtomicU64,
    pub retries: AtomicU64,
    pub start_time: Mutex<Option<Instant>>,
}

impl LogPurger {
    pub async fn new(config: Config) -> Result<Self> {
        let db = Arc::new(DatabaseManager::new(&config.database_url).await?);
        let metrics = Arc::new(Metrics::new()?);

        let progress_bar = if config.progress {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} [{elapsed_precise}] {msg}")
                    .map_err(|e| PurgerError::Config(format!("Progress bar template error: {}", e)))?,
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            Some(Arc::new(Mutex::new(pb)))
        } else {
            None
        };

        Ok(Self {
            config: Arc::new(config),
            db,
            metrics,
            running: Arc::new(AtomicBool::new(true)),
            stats: Arc::new(PurgeStats::default()),
            progress_bar,
        })
    }

    #[instrument(skip(self))]
    pub async fn run(&self) -> Result<()> {
        info!("Starting log_sdk purge process");
        info!("Strategy: {:?}, Batch size: {}, Retention: {} days",
            self.config.strategy, self.config.batch_size, self.config.retention_days
        );

        if self.config.dry_run {
            warn!("🔸 DRY RUN MODE - No actual deletions will occur");
        }

        *self.stats.start_time.lock() = Some(Instant::now());

        // Start metrics server if enabled
        if self.config.metrics_port > 0 {
            let metrics = (*self.metrics).clone();
            let port = self.config.metrics_port;
            tokio::spawn(async move {
                if let Err(e) = metrics.serve(port).await {
                    error!("Metrics server error: {}", e);
                }
            });
        }

        // Analyze table before starting
        let analysis = self.analyze_table().await?;
        info!("Table analysis: {} rows to process, ~{:.2} GB",
            analysis.total_rows, analysis.estimated_size_gb
        );

        if analysis.total_rows == 0 {
            info!("✅ No rows to purge");
            return Ok(());
        }

        // Setup progress bar if enabled
        if let Some(ref pb) = self.progress_bar {
            pb.lock().set_length(analysis.total_rows as u64);
        }

        // Execute purge based on strategy
        let result = match self.config.strategy {
            PurgeStrategy::Id => self.purge_by_id(analysis).await,
            PurgeStrategy::Fecha => self.purge_by_fecha(analysis).await,
            PurgeStrategy::Adaptive => self.purge_adaptive(analysis).await,
            PurgeStrategy::Partition => self.purge_by_partition(analysis).await,
        };

        // Final statistics
        self.print_summary();

        result
    }

    async fn analyze_table(&self) -> Result<TableAnalysis> {
        info!("Analyzing table...");

        let retention_date = self.config.retention_date();
        // Build query with parameterized filters to prevent SQL injection
        let mut query = String::from(
            "SELECT
                COUNT(*) as total_rows,
                MIN(id) as min_id,
                MAX(id) as max_id,
                MIN(fecha) as min_fecha,
                MAX(fecha) as max_fecha,
                AVG(LENGTH(parametros)) as avg_param_size,
                AVG(LENGTH(output)) as avg_output_size,
                SUM(LENGTH(parametros) + LENGTH(output)) / 1024 / 1024 / 1024 as total_gb
             FROM log_sdk
             WHERE fecha < ?"
        );

        // Add optional filters with parameterized queries
        if self.config.proceso.is_some() {
            query.push_str(" AND proceso = ?");
        }
        if self.config.user.is_some() {
            query.push_str(" AND user = ?");
        }

        // Build query with proper parameter binding
        let mut query_builder = sqlx::query(&query)
            .bind(retention_date);

        if let Some(ref proceso) = self.config.proceso {
            query_builder = query_builder.bind(proceso);
        }
        if let Some(ref user) = self.config.user {
            query_builder = query_builder.bind(user);
        }

        let row = timeout(
            Duration::from_secs(60), // 60 second timeout for analysis query
            query_builder.fetch_one(self.db.pool())
        )
            .await
            .map_err(|_| PurgerError::Config("Table analysis timed out after 60 seconds".to_string()))??;

        Ok(TableAnalysis {
            total_rows: row.get("total_rows"),
            min_id: row.get("min_id"),
            max_id: row.get("max_id"),
            min_fecha: row.get("min_fecha"),
            max_fecha: row.get("max_fecha"),
            avg_row_size: row.get::<f64, _>("avg_param_size") + row.get::<f64, _>("avg_output_size"),
            estimated_size_gb: row.get("total_gb"),
        })
    }

    #[instrument(skip(self, _analysis))]
    async fn purge_adaptive(&self, _analysis: TableAnalysis) -> Result<()> {
        info!("Using adaptive purge strategy");

        // Start with smaller batches, increase if successful
        let mut current_batch_size = (self.config.batch_size / 2).max(100);
        let mut consecutive_successes = 0;
        let mut consecutive_failures = 0;

        while self.running.load(Ordering::Relaxed) {
            // Health check
            self.health_check().await?;

            // Get next batch of IDs
            let ids = self.get_next_batch_ids(current_batch_size).await?;
            if ids.is_empty() {
                info!("No more rows to delete");
                break;
            }

            let batch_start = Instant::now();

            match self.delete_by_ids(&ids).await {
                Ok(deleted) => {
                    let duration = batch_start.elapsed();

                    // Record metrics
                    self.metrics.rows_deleted.inc_by(deleted as f64);
                    self.metrics.batches_processed.inc();
                    self.metrics.delete_duration.observe(duration.as_secs_f64());

                    // Update stats
                    self.stats.rows_deleted.fetch_add(deleted, Ordering::Relaxed);
                    self.stats.batches_completed.fetch_add(1, Ordering::Relaxed);

                    // Update progress bar
                    if let Some(ref pb) = self.progress_bar {
                        let pb = pb.lock();
                        pb.inc(deleted);
                        pb.set_message(format!(
                            "Deleted {} rows | Batch size: {} | Speed: {:.0} rows/sec",
                            self.stats.rows_deleted.load(Ordering::Relaxed),
                            current_batch_size,
                            deleted as f64 / duration.as_secs_f64()
                        ));
                    }

                    // Adaptive batch sizing
                    consecutive_successes += 1;
                    consecutive_failures = 0;

                    if consecutive_successes >= 5 && current_batch_size < self.config.batch_size {
                        current_batch_size = (current_batch_size * 150 / 100).min(self.config.batch_size);
                        debug!("Increasing batch size to {}", current_batch_size);
                        consecutive_successes = 0;
                    }

                    // Adaptive sleep
                    let sleep_ms = self.calculate_sleep_time(duration, current_batch_size);
                    tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                }
                Err(e) if e.is_retryable() => {
                    warn!("Retryable error: {}", e);
                    self.stats.retries.fetch_add(1, Ordering::Relaxed);

                    // Reduce batch size on failures
                    consecutive_failures += 1;
                    consecutive_successes = 0;

                    if consecutive_failures >= 3 {
                        current_batch_size = (current_batch_size * 75 / 100).max(100);
                        warn!("Reducing batch size to {} due to errors", current_batch_size);
                        consecutive_failures = 0;
                    }

                    tokio::time::sleep(Duration::from_secs(e.suggested_wait_seconds())).await;
                }
                Err(e) => {
                    error!("Fatal error: {}", e);
                    self.stats.errors.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
            }

            // Check max runtime
            if self.check_max_runtime() {
                warn!("Maximum runtime reached, stopping");
                break;
            }
        }

        Ok(())
    }

    async fn get_next_batch_ids(&self, batch_size: u32) -> Result<Vec<i64>> {
        let retention_date = self.config.retention_date();

        // Build parameterized query to prevent SQL injection
        let mut query = String::from(
            "SELECT id FROM log_sdk
             WHERE fecha < ?"
        );

        // Add optional filters with parameterized queries
        if self.config.proceso.is_some() {
            query.push_str(" AND proceso = ?");
        }
        if self.config.user.is_some() {
            query.push_str(" AND user = ?");
        }

        query.push_str(" ORDER BY id LIMIT ?");

        // Build query with proper parameter binding
        let mut query_builder = sqlx::query_scalar(&query)
            .bind(retention_date);

        if let Some(ref proceso) = self.config.proceso {
            query_builder = query_builder.bind(proceso);
        }
        if let Some(ref user) = self.config.user {
            query_builder = query_builder.bind(user);
        }

        let ids = timeout(
            Duration::from_secs(30), // 30 second timeout for SELECT operations
            query_builder
                .bind(batch_size)
                .fetch_all(self.db.pool())
        )
            .await
            .map_err(|_| PurgerError::Config("SELECT operation timed out after 30 seconds".to_string()))??;

        Ok(ids)
    }

    async fn delete_by_ids(&self, ids: &[i64]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }

        if self.config.dry_run {
            info!("DRY RUN: Would delete {} rows", ids.len());
            return Ok(ids.len() as u64);
        }

        let min_id = *ids.first()
            .ok_or_else(|| PurgerError::Config("Empty batch - no IDs to delete".to_string()))?;
        let max_id = *ids.last()
            .ok_or_else(|| PurgerError::Config("Empty batch - no IDs to delete".to_string()))?;

        // Use more efficient range deletion with timeout
        let result = timeout(
            Duration::from_secs(60), // 60 second timeout for DELETE operations
            sqlx::query(
                "DELETE FROM log_sdk
                 WHERE id >= ? AND id <= ?"
            )
                .bind(min_id)
                .bind(max_id)
                .execute(self.db.pool())
        )
            .await
            .map_err(|_| PurgerError::Config("DELETE operation timed out after 60 seconds".to_string()))?
            .map_err(|e| {
                if e.to_string().contains("Deadlock") {
                    PurgerError::Deadlock
                } else if e.to_string().contains("Lock wait timeout") {
                    PurgerError::TableLocked
                } else {
                    e.into()
                }
            })?;

        Ok(result.rows_affected())
    }

    async fn purge_by_id(&self, analysis: TableAnalysis) -> Result<()> {
        info!("Using ID-based purge strategy");

        let mut current_id = analysis.min_id.unwrap_or(0);
        let max_id = analysis.max_id.unwrap_or(0);
        let chunk_size = self.config.batch_size as i64;

        while current_id <= max_id && self.running.load(Ordering::Relaxed) {
            self.health_check().await?;

            let chunk_end = (current_id + chunk_size - 1).min(max_id);

            let result = if self.config.dry_run {
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM log_sdk
                     WHERE id BETWEEN ? AND ? AND fecha < ?"
                )
                    .bind(current_id)
                    .bind(chunk_end)
                    .bind(self.config.retention_date())
                    .fetch_one(self.db.pool())
                    .await?;

                info!("DRY RUN: Would delete {} rows (ID {}-{})", count, current_id, chunk_end);
                count as u64
            } else {
                let result = sqlx::query(
                    "DELETE FROM log_sdk
                     WHERE id BETWEEN ? AND ? AND fecha < ?"
                )
                    .bind(current_id)
                    .bind(chunk_end)
                    .bind(self.config.retention_date())
                    .execute(self.db.pool())
                    .await?;

                result.rows_affected()
            };

            if result > 0 {
                self.stats.rows_deleted.fetch_add(result, Ordering::Relaxed);
                self.metrics.rows_deleted.inc_by(result as f64);
            }

            current_id = chunk_end + 1;
            tokio::time::sleep(Duration::from_millis(self.config.sleep_ms)).await;
        }

        Ok(())
    }

    async fn purge_by_fecha(&self, _analysis: TableAnalysis) -> Result<()> {
        info!("Using fecha-based purge strategy");

        let retention_date = self.config.retention_date();

        loop {
            self.health_check().await?;

            let deleted = if self.config.dry_run {
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM log_sdk WHERE fecha < ? LIMIT ?"
                )
                    .bind(retention_date)
                    .bind(self.config.batch_size)
                    .fetch_one(self.db.pool())
                    .await?;

                info!("DRY RUN: Would delete {} rows", count);
                count as u64
            } else {
                let result = sqlx::query(
                    "DELETE FROM log_sdk
                     WHERE fecha < ?
                     ORDER BY fecha, id
                     LIMIT ?"
                )
                    .bind(retention_date)
                    .bind(self.config.batch_size)
                    .execute(self.db.pool())
                    .await?;

                result.rows_affected()
            };

            if deleted == 0 {
                break;
            }

            self.stats.rows_deleted.fetch_add(deleted, Ordering::Relaxed);
            self.metrics.rows_deleted.inc_by(deleted as f64);

            tokio::time::sleep(Duration::from_millis(self.config.sleep_ms)).await;
        }

        Ok(())
    }

    async fn purge_by_partition(&self, _analysis: TableAnalysis) -> Result<()> {
        info!("Checking for partitioned table...");

        // Check if table is partitioned
        let partitions: Vec<String> = sqlx::query_scalar(
            "SELECT partition_name
             FROM information_schema.partitions
             WHERE table_schema = DATABASE()
             AND table_name = 'log_sdk'
             AND partition_name IS NOT NULL"
        )
            .fetch_all(self.db.pool())
            .await?;

        if partitions.is_empty() {
            warn!("Table is not partitioned, falling back to adaptive strategy");
            return self.purge_adaptive(_analysis).await;
        }

        info!("Found {} partitions", partitions.len());

        for partition in partitions {
            // Check partition date
            if self.should_drop_partition(&partition).await? {
                if self.config.dry_run {
                    info!("DRY RUN: Would drop partition {}", partition);
                } else {
                    info!("Dropping partition {}", partition);
                    sqlx::query(&format!("ALTER TABLE log_sdk DROP PARTITION {}", partition))
                        .execute(self.db.pool())
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn should_drop_partition(&self, partition_name: &str) -> Result<bool> {
        // Extract date from partition name (assuming format like p_2024_01)
        // This is a simplified check - adjust based on your partition naming
        Ok(partition_name.contains("2024") || partition_name.contains("2023"))
    }

    async fn health_check(&self) -> Result<()> {
        let health = self.db.check_server_health().await?;

        // Update metrics
        self.metrics.active_connections.set(health.active_connections as f64);
        self.metrics.current_lag.set(health.replication_lag_seconds as f64);

        // Check health
        health.is_healthy(&self.config)?;

        Ok(())
    }

    fn calculate_sleep_time(&self, operation_duration: Duration, batch_size: u32) -> u64 {
        let base_sleep = self.config.sleep_ms;
        let operation_ms = operation_duration.as_millis() as u64;

        // If operation was very fast, use base sleep
        if operation_ms < 100 {
            return base_sleep;
        }

        // If operation was slow, sleep proportionally less
        if operation_ms > 1000 {
            return (base_sleep / 2).max(50);
        }

        // Adaptive sleep based on batch size and duration
        let rows_per_second = (batch_size as f64 * 1000.0) / operation_ms as f64;

        if rows_per_second > 10000.0 {
            // Very fast, increase sleep
            base_sleep * 2
        } else if rows_per_second < 1000.0 {
            // Slow, reduce sleep
            (base_sleep / 2).max(50)
        } else {
            base_sleep
        }
    }

    fn check_max_runtime(&self) -> bool {
        if self.config.max_runtime_minutes == 0 {
            return false;
        }

        if let Some(start) = *self.stats.start_time.lock() {
            let elapsed = start.elapsed();
            elapsed.as_secs() / 60 >= self.config.max_runtime_minutes
        } else {
            false
        }
    }

    fn print_summary(&self) {
        let elapsed = self.stats.start_time.lock()
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default();

        let rows = self.stats.rows_deleted.load(Ordering::Relaxed);
        let batches = self.stats.batches_completed.load(Ordering::Relaxed);
        let errors = self.stats.errors.load(Ordering::Relaxed);
        let retries = self.stats.retries.load(Ordering::Relaxed);
        let bytes = self.stats.bytes_freed.load(Ordering::Relaxed);

        info!("╔══════════════════════════════════════╗");
        info!("║         PURGE SUMMARY                ║");
        info!("╠══════════════════════════════════════╣");
        info!("║ Rows deleted:     {:>18} ║", rows);
        info!("║ Batches:          {:>18} ║", batches);
        info!("║ Errors:           {:>18} ║", errors);
        info!("║ Retries:          {:>18} ║", retries);
        info!("║ Bytes freed:      {:>18} ║", humansize::format_size(bytes, humansize::DECIMAL));
        info!("║ Duration:         {:>18?} ║", elapsed);
        info!("║ Rate:             {:>15.2} rows/s ║", rows as f64 / elapsed.as_secs_f64());
        info!("╚══════════════════════════════════════╝");
    }

    pub fn stop(&self) {
        info!("Stopping purger...");
        self.running.store(false, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct TableAnalysis {
    total_rows: i64,
    min_id: Option<i64>,
    max_id: Option<i64>,
    min_fecha: Option<NaiveDateTime>,
    max_fecha: Option<NaiveDateTime>,
    avg_row_size: f64,
    estimated_size_gb: f64,
}