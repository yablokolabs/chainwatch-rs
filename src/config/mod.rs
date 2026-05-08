use std::{net::SocketAddr, time::Duration};

use alloy::primitives::U256;
use config_rs::{Config, Environment};
use serde::Deserialize;
use url::Url;

use crate::domain::{ChainwatchError, Result, codec::parse_u256};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub app: AppSettings,
    pub api: ApiSettings,
    pub database: DatabaseSettings,
    pub redis: RedisSettings,
    pub evm: EvmSettings,
    pub indexer: IndexerSettings,
    pub risk: RiskSettings,
    pub telemetry: TelemetrySettings,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();
        let config = Config::builder()
            .add_source(Environment::with_prefix("CHAINWATCH").separator("__"))
            .build()
            .map_err(|err| ChainwatchError::Config(err.to_string()))?;
        let mut settings: Self = config
            .try_deserialize()
            .map_err(|err| ChainwatchError::Config(err.to_string()))?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&mut self) -> Result<()> {
        Url::parse(&self.evm.rpc_url)
            .map_err(|err| ChainwatchError::Config(format!("invalid EVM RPC URL: {err}")))?;
        if let Some(redis_url) = &self.redis.url {
            Url::parse(redis_url)
                .map_err(|err| ChainwatchError::Config(format!("invalid Redis URL: {err}")))?;
        }
        if self.indexer.reorg_confirmations == 0 {
            return Err(ChainwatchError::Config(
                "indexer.reorg_confirmations must be greater than zero".to_owned(),
            ));
        }
        if self.indexer.backfill_batch_size == 0 {
            return Err(ChainwatchError::Config(
                "indexer.backfill_batch_size must be greater than zero".to_owned(),
            ));
        }
        if self.indexer.rpc_concurrency == 0 {
            return Err(ChainwatchError::Config(
                "indexer.rpc_concurrency must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub name: String,
    pub mode: RuntimeMode,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            name: "chainwatch-rs".to_owned(),
            mode: RuntimeMode::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Api,
    Indexer,
    #[default]
    All,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    pub bind_addr: SocketAddr,
    pub cors_allow_origin: String,
    pub rate_limit_per_second: u64,
    pub request_timeout_seconds: u64,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 8080)),
            cors_allow_origin: "*".to_owned(),
            rate_limit_per_second: 100,
            request_timeout_seconds: 30,
        }
    }
}

impl ApiSettings {
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_seconds)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseSettings {
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
}

impl Default for DatabaseSettings {
    fn default() -> Self {
        Self {
            url: "postgres://chainwatch_app@localhost:5432/chainwatch".to_owned(),
            max_connections: 10,
            run_migrations: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct RedisSettings {
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EvmSettings {
    pub chain_name: String,
    pub rpc_url: String,
    pub chain_id_override: Option<u64>,
}

impl Default for EvmSettings {
    fn default() -> Self {
        Self {
            chain_name: "ethereum".to_owned(),
            rpc_url: "http://localhost:8545".to_owned(),
            chain_id_override: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct IndexerSettings {
    pub start_block: u64,
    pub reorg_confirmations: u64,
    pub backfill_batch_size: u64,
    pub rpc_concurrency: usize,
    pub poll_interval_seconds: u64,
    pub max_retries: u32,
    pub initial_retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
}

impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            start_block: 0,
            reorg_confirmations: 12,
            backfill_batch_size: 25,
            rpc_concurrency: 8,
            poll_interval_seconds: 12,
            max_retries: 5,
            initial_retry_delay_ms: 250,
            max_retry_delay_ms: 5_000,
        }
    }
}

impl IndexerSettings {
    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_seconds)
    }

    #[must_use]
    pub const fn initial_retry_delay(&self) -> Duration {
        Duration::from_millis(self.initial_retry_delay_ms)
    }

    #[must_use]
    pub const fn max_retry_delay(&self) -> Duration {
        Duration::from_millis(self.max_retry_delay_ms)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RiskSettings {
    pub large_transfer_threshold_wei: String,
    pub high_frequency_threshold: u64,
    pub high_frequency_window_seconds: u64,
    pub suspicious_contract_rule_enabled: bool,
}

impl Default for RiskSettings {
    fn default() -> Self {
        Self {
            large_transfer_threshold_wei: "100000000000000000000000".to_owned(),
            high_frequency_threshold: 25,
            high_frequency_window_seconds: 3_600,
            suspicious_contract_rule_enabled: true,
        }
    }
}

impl RiskSettings {
    pub fn large_transfer_threshold(&self) -> Result<U256> {
        parse_u256(&self.large_transfer_threshold_wei)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct TelemetrySettings {
    pub log_level: String,
    pub json_logs: bool,
    pub otlp_endpoint: Option<String>,
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            log_level: "info,sqlx=warn,tower_http=info".to_owned(),
            json_logs: true,
            otlp_endpoint: None,
        }
    }
}
