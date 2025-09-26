use thiserror::Error;

#[derive(Error, Debug)]
pub enum PurgerError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Server overloaded: {current}/{max} connections")]
    ServerOverload { current: u32, max: u32 },

    #[error("Deadlock detected, retrying...")]
    Deadlock,

    #[error("Table locked for too long")]
    TableLocked,

    #[error("Disk space low: {available_gb}GB remaining")]
    LowDiskSpace { available_gb: f64 },

    #[error("Replication lag too high: {seconds} seconds")]
    ReplicationLag { seconds: u64 },

    #[error("Purge interrupted by user")]
    Interrupted,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Metrics error: {0}")]
    Metrics(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, PurgerError>;

impl PurgerError {
    pub fn is_retryable(&self) -> bool {
        matches!(self,
            PurgerError::Deadlock |
            PurgerError::TableLocked |
            PurgerError::ServerOverload { .. }
        )
    }

    pub fn suggested_wait_seconds(&self) -> u64 {
        match self {
            PurgerError::Deadlock => 1,
            PurgerError::TableLocked => 5,
            PurgerError::ServerOverload { .. } => 30,
            PurgerError::ReplicationLag { seconds } => (*seconds).min(60),
            _ => 10,
        }
    }
}