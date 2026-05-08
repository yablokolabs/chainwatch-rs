use std::sync::Arc;

use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use url::Url;

use chainwatch_rs::{
    api::{AppState, router as api_router},
    application::{
        ports::{BlockchainClient, Repository},
        services::AlertingService,
    },
    config::{AppConfig, RuntimeMode},
    domain::{Chain, ChainId, ChainwatchError, Result},
    indexer::{Erc20EventDecoder, Indexer},
    infrastructure::{
        alloy_rpc::AlloyBlockchainClient, cache::RedisCache, postgres::PostgresRepository,
    },
    risk::{RiskEngine, RiskEngineConfig},
    telemetry,
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let (metrics_handle, _telemetry_guard) = telemetry::init(&config.telemetry)?;

    let repository =
        Arc::new(PostgresRepository::connect(&config.database).await?) as Arc<dyn Repository>;
    let rpc_client = Arc::new(
        AlloyBlockchainClient::connect(
            &config.evm.rpc_url,
            config.evm.chain_id_override,
            config.indexer.rpc_concurrency,
        )
        .await?,
    );
    let chain_id = rpc_client.chain_id().await?;
    let chain = Chain {
        id: chain_id,
        name: config.evm.chain_name.clone(),
        rpc_url_redacted: redact_url(&config.evm.rpc_url),
    };
    repository.upsert_chain(&chain).await?;

    let shutdown = CancellationToken::new();
    install_shutdown_handler(shutdown.clone());

    match config.app.mode {
        RuntimeMode::Api => {
            let router = build_api_router(repository, chain_id, metrics_handle, &config)?;
            serve_api(router, &config, shutdown).await
        }
        RuntimeMode::Indexer => {
            let indexer = build_indexer(repository, rpc_client, chain, &config)?;
            indexer.run_until_cancelled(shutdown).await
        }
        RuntimeMode::All => {
            let router = build_api_router(repository.clone(), chain_id, metrics_handle, &config)?;
            let indexer = build_indexer(repository, rpc_client, chain, &config)?;
            run_all(router, indexer, &config, shutdown).await
        }
    }
}

fn build_api_router(
    repository: Arc<dyn Repository>,
    chain_id: ChainId,
    metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
    config: &AppConfig,
) -> Result<Router> {
    let state = AppState::new(
        repository,
        chain_id,
        metrics_handle,
        config.api.rate_limit_per_second,
    );
    api_router(state, &config.api)
}

fn build_indexer(
    repository: Arc<dyn Repository>,
    rpc_client: Arc<AlloyBlockchainClient>,
    chain: Chain,
    config: &AppConfig,
) -> Result<Indexer> {
    let threshold = config.risk.large_transfer_threshold()?;
    let risk_engine = RiskEngine::new(RiskEngineConfig {
        large_transfer_threshold_wei: threshold,
        high_frequency_threshold: config.risk.high_frequency_threshold,
        suspicious_contract_rule_enabled: config.risk.suspicious_contract_rule_enabled,
    });
    let alerting = AlertingService::new(
        repository.clone(),
        risk_engine,
        config.risk.high_frequency_window_seconds,
    );
    let cache = config
        .redis
        .url
        .as_deref()
        .map(RedisCache::new)
        .transpose()?;
    Ok(Indexer::new(
        chain,
        rpc_client as Arc<dyn BlockchainClient>,
        repository,
        Arc::new(Erc20EventDecoder::new()),
        alerting,
        config.indexer.clone(),
        cache,
    ))
}

async fn run_all(
    router: Router,
    indexer: Indexer,
    config: &AppConfig,
    shutdown: CancellationToken,
) -> Result<()> {
    let indexer_shutdown = shutdown.clone();
    let indexer_task =
        tokio::spawn(async move { indexer.run_until_cancelled(indexer_shutdown).await });

    let api_result = serve_api(router, config, shutdown.clone()).await;
    shutdown.cancel();

    let indexer_result = indexer_task
        .await
        .map_err(|err| ChainwatchError::Internal(format!("indexer task join: {err}")))?;

    api_result.and(indexer_result)
}

async fn serve_api(router: Router, config: &AppConfig, shutdown: CancellationToken) -> Result<()> {
    let listener = TcpListener::bind(config.api.bind_addr)
        .await
        .map_err(|err| ChainwatchError::Internal(format!("bind api: {err}")))?;
    info!(addr = %config.api.bind_addr, "chainwatch API listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.cancelled().await;
            info!("api shutdown requested");
        })
        .await
        .map_err(|err| ChainwatchError::Internal(format!("api server: {err}")))
}

fn install_shutdown_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(error = %err, "failed to listen for ctrl-c");
        }
        shutdown.cancel();
    });
}

fn redact_url(value: &str) -> String {
    match Url::parse(value) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _username_result = url.set_username("redacted");
            }
            let _password_result = url.set_password(None);
            url.set_query(None);
            url.to_string()
        }
        Err(_) => "<invalid-url>".to_owned(),
    }
}
