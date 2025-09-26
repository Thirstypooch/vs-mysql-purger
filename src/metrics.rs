use prometheus::{
    register_counter, register_gauge, register_histogram,
    Counter, Gauge, Histogram, TextEncoder, Encoder
};
use axum::{Router, routing::get};
use tracing::info;

#[derive(Clone)]
pub struct Metrics {
    pub rows_deleted: Counter,
    pub batches_processed: Counter,
    pub errors_total: Counter,
    pub bytes_freed: Counter,
    pub current_lag: Gauge,
    pub active_connections: Gauge,
    pub delete_duration: Histogram,
    pub batch_size: Gauge,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            rows_deleted: register_counter!(
                "purger_rows_deleted_total",
                "Total number of rows deleted"
            )?,
            batches_processed: register_counter!(
                "purger_batches_processed_total",
                "Total number of batches processed"
            )?,
            errors_total: register_counter!(
                "purger_errors_total",
                "Total number of errors encountered"
            )?,
            bytes_freed: register_counter!(
                "purger_bytes_freed_total",
                "Estimated bytes freed"
            )?,
            current_lag: register_gauge!(
                "purger_replication_lag_seconds",
                "Current replication lag in seconds"
            )?,
            active_connections: register_gauge!(
                "purger_active_connections",
                "Current active database connections"
            )?,
            delete_duration: register_histogram!(
                "purger_delete_duration_seconds",
                "Delete operation duration"
            )?,
            batch_size: register_gauge!(
                "purger_batch_size",
                "Current batch size"
            )?,
        })
    }

    pub async fn serve(self, port: u16) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/metrics", get(move || async move {
                let encoder = TextEncoder::new();
                let metric_families = prometheus::gather();
                let mut buffer = Vec::new();
                if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
                    return format!("Error encoding metrics: {}", e);
                }
                String::from_utf8(buffer).unwrap_or_else(|_| "Error: Invalid UTF-8 in metrics".to_string())
            }))
            .route("/health", get(|| async { "OK" }));

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
            .await?;

        info!("Metrics server listening on port {}", port);

        axum::serve(listener, app).await?;
        Ok(())
    }
}