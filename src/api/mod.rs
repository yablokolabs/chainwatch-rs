use std::{sync::Arc, time::Instant};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderName, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{error, instrument};

use crate::{
    application::{ports::Repository, services::build_watchlist_entry},
    config::ApiSettings,
    domain::{
        Alert, Block, ChainId, ChainwatchError, Page as DomainPage, Result, TokenTransfer,
        Transaction, WatchlistEntry,
        codec::{
            address_to_hex, bytes_to_hex, hash_to_hex, parse_address, parse_hash, u256_to_decimal,
        },
    },
};

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub chain_id: ChainId,
    pub metrics: PrometheusHandle,
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    #[must_use]
    pub fn new(
        repository: Arc<dyn Repository>,
        chain_id: ChainId,
        metrics: PrometheusHandle,
        rate_limit_per_second: u64,
    ) -> Self {
        Self {
            repository,
            chain_id,
            metrics,
            rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_second)),
        }
    }
}

#[derive(Debug)]
pub struct RateLimiter {
    capacity: u64,
    refill_per_second: u64,
    bucket: tokio::sync::Mutex<TokenBucket>,
}

#[derive(Debug)]
struct TokenBucket {
    tokens: u64,
    last_refill: Instant,
}

impl RateLimiter {
    #[must_use]
    pub fn new(rate_per_second: u64) -> Self {
        let capacity = rate_per_second.max(1);
        Self {
            capacity,
            refill_per_second: capacity,
            bucket: tokio::sync::Mutex::new(TokenBucket {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    pub async fn allow(&self) -> bool {
        let mut bucket = self.bucket.lock().await;
        let elapsed = bucket.last_refill.elapsed().as_secs_f64();
        let refill = (elapsed * self.refill_per_second as f64).floor() as u64;
        if refill > 0 {
            bucket.tokens = bucket.tokens.saturating_add(refill).min(self.capacity);
            bucket.last_refill = Instant::now();
        }
        if bucket.tokens == 0 {
            return false;
        }
        bucket.tokens = bucket.tokens.saturating_sub(1);
        true
    }
}

async fn rate_limit(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    if state.rate_limiter.allow().await {
        next.run(request).await
    } else {
        ApiError::from(ChainwatchError::RateLimited).into_response()
    }
}

pub fn router(state: AppState, settings: &ApiSettings) -> Result<Router> {
    let request_id_header = HeaderName::from_static("x-request-id");
    let cors = cors_layer(settings)?;
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(request_id_header))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            settings.request_timeout(),
        ))
        .layer(cors);

    let rate_limit_state = state.clone();
    Ok(Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/blocks/latest", get(latest_block))
        .route("/transactions/{hash}", get(transaction_by_hash))
        .route("/addresses/{address}/transfers", get(address_transfers))
        .route("/tokens/{address}/transfers", get(token_transfers))
        .route("/alerts", get(alerts))
        .route("/watchlist", post(add_watchlist))
        .route("/watchlist/{address}", delete(remove_watchlist))
        .with_state(state)
        .layer(middleware::from_fn_with_state(rate_limit_state, rate_limit))
        .layer(middleware))
}

fn cors_layer(settings: &ApiSettings) -> Result<CorsLayer> {
    if settings.cors_allow_origin == "*" {
        Ok(CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any))
    } else {
        let origin = HeaderValue::from_str(&settings.cors_allow_origin).map_err(|err| {
            ChainwatchError::Config(format!("invalid CORS origin header value: {err}"))
        })?;
        Ok(CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(origin))
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "chainwatch-rs",
    })
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render(),
    )
}

#[instrument(skip(state))]
async fn latest_block(State(state): State<AppState>) -> ApiResult<Json<Option<BlockDto>>> {
    let block = state.repository.latest_block(state.chain_id).await?;
    Ok(Json(block.map(BlockDto::from)))
}

#[instrument(skip(state))]
async fn transaction_by_hash(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Option<TransactionDto>>> {
    let hash = parse_hash(&hash)?;
    let tx = state
        .repository
        .get_transaction(state.chain_id, hash)
        .await?;
    Ok(Json(tx.map(TransactionDto::from)))
}

#[instrument(skip(state))]
async fn address_transfers(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(page): Query<PageQuery>,
) -> ApiResult<Json<Vec<TokenTransferDto>>> {
    let address = parse_address(&address)?;
    let transfers = state
        .repository
        .list_transfers_by_address(state.chain_id, address, page.into())
        .await?;
    Ok(Json(
        transfers.into_iter().map(TokenTransferDto::from).collect(),
    ))
}

#[instrument(skip(state))]
async fn token_transfers(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(page): Query<PageQuery>,
) -> ApiResult<Json<Vec<TokenTransferDto>>> {
    let token = parse_address(&address)?;
    let transfers = state
        .repository
        .list_transfers_by_token(state.chain_id, token, page.into())
        .await?;
    Ok(Json(
        transfers.into_iter().map(TokenTransferDto::from).collect(),
    ))
}

#[instrument(skip(state))]
async fn alerts(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> ApiResult<Json<Vec<AlertDto>>> {
    let alerts = state
        .repository
        .list_alerts(state.chain_id, page.into())
        .await?;
    Ok(Json(alerts.into_iter().map(AlertDto::from).collect()))
}

#[derive(Debug, Deserialize)]
struct WatchlistRequest {
    address: String,
    label: Option<String>,
}

#[instrument(skip(state, request))]
async fn add_watchlist(
    State(state): State<AppState>,
    Json(request): Json<WatchlistRequest>,
) -> ApiResult<(StatusCode, Json<WatchlistDto>)> {
    let address = parse_address(&request.address)?;
    let entry = build_watchlist_entry(state.chain_id, address, request.label);
    state.repository.add_watchlist(&entry).await?;
    Ok((StatusCode::CREATED, Json(WatchlistDto::from(entry))))
}

#[instrument(skip(state))]
async fn remove_watchlist(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<StatusCode> {
    let address = parse_address(&address)?;
    let removed = state
        .repository
        .remove_watchlist(state.chain_id, address)
        .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::from(ChainwatchError::NotFound(format!(
            "watchlist address {}",
            address_to_hex(&address)
        ))))
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

impl From<PageQuery> for DomainPage {
    fn from(value: PageQuery) -> Self {
        DomainPage::new(value.limit, value.offset)
    }
}

#[derive(Debug, Serialize)]
pub struct BlockDto {
    chain_id: u64,
    number: u64,
    hash: String,
    parent_hash: String,
    timestamp: u64,
    tx_count: u64,
}

impl From<Block> for BlockDto {
    fn from(block: Block) -> Self {
        Self {
            chain_id: block.chain_id.0,
            number: block.number.0,
            hash: hash_to_hex(&block.hash),
            parent_hash: hash_to_hex(&block.parent_hash),
            timestamp: block.timestamp,
            tx_count: block.tx_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TransactionDto {
    chain_id: u64,
    hash: String,
    block_number: u64,
    tx_index: u64,
    from: String,
    to: Option<String>,
    value_wei: String,
    input: String,
    status: Option<bool>,
    gas_used: Option<u64>,
}

impl From<Transaction> for TransactionDto {
    fn from(tx: Transaction) -> Self {
        Self {
            chain_id: tx.chain_id.0,
            hash: hash_to_hex(&tx.hash),
            block_number: tx.block_number.0,
            tx_index: tx.tx_index,
            from: address_to_hex(&tx.from),
            to: tx.to.as_ref().map(address_to_hex),
            value_wei: u256_to_decimal(&tx.value),
            input: bytes_to_hex(&tx.input),
            status: tx.status,
            gas_used: tx.gas_used,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TokenTransferDto {
    chain_id: u64,
    token_address: String,
    from: String,
    to: String,
    amount_wei: String,
    tx_hash: String,
    block_number: u64,
    log_index: u64,
    timestamp: u64,
}

impl From<TokenTransfer> for TokenTransferDto {
    fn from(transfer: TokenTransfer) -> Self {
        Self {
            chain_id: transfer.chain_id.0,
            token_address: address_to_hex(&transfer.token_address),
            from: address_to_hex(&transfer.from),
            to: address_to_hex(&transfer.to),
            amount_wei: u256_to_decimal(&transfer.amount),
            tx_hash: hash_to_hex(&transfer.tx_hash),
            block_number: transfer.block_number.0,
            log_index: transfer.log_index,
            timestamp: transfer.timestamp,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AlertDto {
    id: uuid::Uuid,
    chain_id: u64,
    rule: String,
    severity: String,
    address: Option<String>,
    tx_hash: Option<String>,
    block_number: Option<u64>,
    message: String,
    metadata: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<Alert> for AlertDto {
    fn from(alert: Alert) -> Self {
        Self {
            id: alert.id,
            chain_id: alert.chain_id.0,
            rule: alert.rule,
            severity: alert.severity.as_str().to_owned(),
            address: alert.address.as_ref().map(address_to_hex),
            tx_hash: alert.tx_hash.as_ref().map(hash_to_hex),
            block_number: alert.block_number.map(|block| block.0),
            message: alert.message,
            metadata: alert.metadata,
            created_at: alert.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WatchlistDto {
    chain_id: u64,
    address: String,
    label: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<WatchlistEntry> for WatchlistDto {
    fn from(entry: WatchlistEntry) -> Self {
        Self {
            chain_id: entry.chain_id.0,
            address: address_to_hex(&entry.address),
            label: entry.label,
            created_at: entry.created_at,
        }
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
struct ApiError(ChainwatchError);

impl From<ChainwatchError> for ApiError {
    fn from(value: ChainwatchError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            ChainwatchError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ChainwatchError::InvalidAddress { address, reason } => (
                StatusCode::BAD_REQUEST,
                format!("invalid address {address}: {reason}"),
            ),
            ChainwatchError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ChainwatchError::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit exceeded".to_owned(),
            ),
            other => {
                error!(error = %other, "internal api error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_owned(),
                )
            }
        };
        let body = Json(serde_json::json!({
            "error": message,
            "status": status.as_u16()
        }));
        (status, body).into_response()
    }
}
