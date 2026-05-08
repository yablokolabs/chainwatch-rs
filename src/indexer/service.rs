use std::{sync::Arc, time::Duration};

use chrono::Utc;
use metrics::{counter, gauge};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::{
    application::{
        ports::{BlockchainClient, EventDecoder, Repository},
        services::AlertingService,
    },
    config::IndexerSettings,
    domain::{
        BlockNumber, Chain, ChainId, ChainwatchError, DecodedEvent, DecodedEventKind, FetchedBlock,
        IndexerState, Result, TokenTransfer,
        codec::{parse_address, parse_u256},
    },
    infrastructure::cache::RedisCache,
    telemetry::{
        METRIC_ALERTS_GENERATED_TOTAL, METRIC_BLOCKS_INDEXED_TOTAL, METRIC_INDEXING_LAG,
        METRIC_LATEST_INDEXED_BLOCK, METRIC_RPC_ERRORS_TOTAL, METRIC_TX_INDEXED_TOTAL,
    },
};

pub struct Indexer {
    chain: Chain,
    client: Arc<dyn BlockchainClient>,
    repository: Arc<dyn Repository>,
    decoder: Arc<dyn EventDecoder>,
    alerting: AlertingService,
    settings: IndexerSettings,
    cache: Option<RedisCache>,
}

impl Indexer {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        chain: Chain,
        client: Arc<dyn BlockchainClient>,
        repository: Arc<dyn Repository>,
        decoder: Arc<dyn EventDecoder>,
        alerting: AlertingService,
        settings: IndexerSettings,
        cache: Option<RedisCache>,
    ) -> Self {
        Self {
            chain,
            client,
            repository,
            decoder,
            alerting,
            settings,
            cache,
        }
    }

    pub async fn run_until_cancelled(&self, shutdown: CancellationToken) -> Result<()> {
        self.repository.upsert_chain(&self.chain).await?;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    info!(chain_id = self.chain.id.0, "indexer shutdown requested");
                    return Ok(());
                }
                result = self.run_once() => {
                    match result {
                        Ok(indexed) => {
                            if indexed > 0 {
                                info!(indexed, chain_id = self.chain.id.0, "indexer batch completed");
                            }
                        }
                        Err(err) => {
                            counter!(METRIC_RPC_ERRORS_TOTAL).increment(1);
                            error!(error = %err, "indexer loop failed; backing off");
                        }
                    }
                    sleep(self.settings.poll_interval()).await;
                }
            }
        }
    }

    #[instrument(skip(self), fields(chain_id = self.chain.id.0))]
    pub async fn run_once(&self) -> Result<u64> {
        self.repair_reorg_if_needed().await?;

        let latest = self.client.latest_block_number().await?;
        if latest.0 <= self.settings.reorg_confirmations {
            gauge!(METRIC_INDEXING_LAG).set(latest.0 as f64);
            return Ok(0);
        }
        let safe_latest = latest.saturating_sub(self.settings.reorg_confirmations);

        let state = self.repository.get_indexer_state(self.chain.id).await?;
        let next = state
            .and_then(|state| state.latest_block)
            .map(BlockNumber::next)
            .unwrap_or(BlockNumber(self.settings.start_block));

        if next > safe_latest {
            let latest_indexed = next.0.saturating_sub(1);
            gauge!(METRIC_LATEST_INDEXED_BLOCK).set(latest_indexed as f64);
            gauge!(METRIC_INDEXING_LAG).set(latest.0.saturating_sub(latest_indexed) as f64);
            return Ok(0);
        }

        let end = BlockNumber(
            next.0
                .saturating_add(self.settings.backfill_batch_size.saturating_sub(1))
                .min(safe_latest.0),
        );
        let mut fetched = Vec::new();
        for number in next.0..=end.0 {
            fetched.push(self.fetch_with_retry(BlockNumber(number)).await?);
        }
        fetched.sort_by_key(|item| item.block.number);
        self.persist_batch(&fetched, latest).await?;
        u64::try_from(fetched.len())
            .map_err(|err| ChainwatchError::Internal(format!("batch length overflow: {err}")))
    }

    async fn repair_reorg_if_needed(&self) -> Result<()> {
        let Some(state) = self.repository.get_indexer_state(self.chain.id).await? else {
            return Ok(());
        };
        let (Some(latest_block), Some(local_hash)) = (state.latest_block, state.latest_hash) else {
            return Ok(());
        };
        if latest_block.0 < self.settings.start_block {
            return Ok(());
        }
        let remote = self.fetch_with_retry(latest_block).await?;
        if remote.block.hash != local_hash {
            let rollback_to = latest_block.saturating_sub(self.settings.reorg_confirmations);
            warn!(
                chain_id = self.chain.id.0,
                latest_block = latest_block.0,
                rollback_to = rollback_to.0,
                local_hash = %local_hash,
                remote_hash = %remote.block.hash,
                "reorg detected; rolling back indexed data"
            );
            self.repository
                .rollback_to_block(self.chain.id, rollback_to)
                .await?;
        }
        Ok(())
    }

    async fn fetch_with_retry(&self, number: BlockNumber) -> Result<FetchedBlock> {
        let mut attempt = 0_u32;
        let mut delay = self.settings.initial_retry_delay();
        loop {
            match self.client.fetch_block(number).await {
                Ok(block) => return Ok(block),
                Err(err) if attempt < self.settings.max_retries => {
                    attempt = attempt.saturating_add(1);
                    counter!(METRIC_RPC_ERRORS_TOTAL).increment(1);
                    warn!(
                        block_number = number.0,
                        attempt,
                        delay_ms = delay.as_millis() as u64,
                        error = %err,
                        "rpc fetch failed; retrying"
                    );
                    sleep(delay).await;
                    delay = next_delay(delay, self.settings.max_retry_delay());
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn persist_batch(&self, fetched: &[FetchedBlock], chain_tip: BlockNumber) -> Result<()> {
        if fetched.is_empty() {
            return Ok(());
        }

        let blocks = fetched
            .iter()
            .map(|item| item.block.clone())
            .collect::<Vec<_>>();
        let transactions = fetched
            .iter()
            .flat_map(|item| item.transactions.clone())
            .collect::<Vec<_>>();
        let logs = fetched
            .iter()
            .flat_map(|item| item.logs.clone())
            .collect::<Vec<_>>();

        self.repository.upsert_blocks(&blocks).await?;
        self.repository.upsert_transactions(&transactions).await?;
        self.repository.upsert_logs(&logs).await?;

        let mut decoded = Vec::new();
        for log in &logs {
            decoded.extend(self.decoder.decode(log)?);
        }
        let transfers = decoded
            .iter()
            .filter_map(decoded_event_to_transfer)
            .collect::<Result<Vec<_>>>()?;

        self.repository.upsert_decoded_events(&decoded).await?;
        self.repository.upsert_token_transfers(&transfers).await?;

        let alerts = self.alerting.evaluate_transfers(&transfers).await?;
        self.repository.insert_alerts(&alerts).await?;

        let latest_block = blocks
            .last()
            .ok_or_else(|| ChainwatchError::Internal("batch has no last block".to_owned()))?;
        self.repository
            .set_indexer_state(&IndexerState {
                chain_id: self.chain.id,
                latest_block: Some(latest_block.number),
                latest_hash: Some(latest_block.hash),
                updated_at: Utc::now(),
            })
            .await?;

        if let Some(cache) = &self.cache {
            cache
                .set_latest_indexed_block(self.chain.id, latest_block.number)
                .await?;
        }

        counter!(METRIC_BLOCKS_INDEXED_TOTAL).increment(blocks.len() as u64);
        counter!(METRIC_TX_INDEXED_TOTAL).increment(transactions.len() as u64);
        counter!(METRIC_ALERTS_GENERATED_TOTAL).increment(alerts.len() as u64);
        gauge!(METRIC_LATEST_INDEXED_BLOCK).set(latest_block.number.0 as f64);
        gauge!(METRIC_INDEXING_LAG).set(chain_tip.0.saturating_sub(latest_block.number.0) as f64);
        Ok(())
    }
}

fn decoded_event_to_transfer(event: &DecodedEvent) -> Option<Result<TokenTransfer>> {
    if event.kind != DecodedEventKind::Erc20Transfer {
        return None;
    }
    let result = (|| {
        let from = event
            .payload
            .get("from")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChainwatchError::Decode("transfer payload missing from".to_owned()))?;
        let to = event
            .payload
            .get("to")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChainwatchError::Decode("transfer payload missing to".to_owned()))?;
        let amount = event
            .payload
            .get("amount_wei")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ChainwatchError::Decode("transfer payload missing amount".to_owned()))?;
        Ok(TokenTransfer {
            chain_id: event.chain_id,
            token_address: event.token_address,
            from: parse_address(from)?,
            to: parse_address(to)?,
            amount: parse_u256(amount)?,
            tx_hash: event.tx_hash,
            block_number: event.block_number,
            log_index: event.log_index,
            timestamp: event.timestamp,
        })
    })();
    Some(result)
}

fn next_delay(current: Duration, max: Duration) -> Duration {
    let doubled = current.checked_mul(2).unwrap_or(max);
    doubled.min(max)
}

#[allow(dead_code)]
fn _chain_id_for_docs(chain_id: ChainId) -> ChainId {
    chain_id
}
