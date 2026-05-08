use alloy::primitives::{Address, B256};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ChainwatchError>;

#[derive(Debug, Error)]
pub enum ChainwatchError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("rpc error: {0}")]
    Rpc(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("rate limited by upstream or local middleware")]
    RateLimited,

    #[error("reorg detected at block {block_number}: local={local_hash}, remote={remote_hash}")]
    ReorgDetected {
        block_number: u64,
        local_hash: B256,
        remote_hash: B256,
    },

    #[error("risk engine error: {0}")]
    Risk(String),

    #[error("address {address} failed validation: {reason}")]
    InvalidAddress { address: Address, reason: String },

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for ChainwatchError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value.to_string())
    }
}
