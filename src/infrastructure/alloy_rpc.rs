use std::sync::Arc;

use alloy::primitives::{Address, B256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use serde_json::{Value, json};
use tracing::{debug, instrument};

use crate::{
    application::ports::BlockchainClient,
    domain::{
        Block, BlockNumber, ChainId, ChainwatchError, FetchedBlock, RawLog, Result, Transaction,
        codec::{
            hex_u64, optional_hex_u64, parse_address, parse_hash, parse_hex_bytes, parse_u256,
        },
    },
};

#[derive(Clone)]
pub struct AlloyBlockchainClient {
    provider: Arc<DynProvider>,
    chain_id: ChainId,
    receipt_concurrency: usize,
}

impl AlloyBlockchainClient {
    pub async fn connect(
        rpc_url: &str,
        chain_id_override: Option<u64>,
        receipt_concurrency: usize,
    ) -> Result<Self> {
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .map_err(|err| ChainwatchError::Rpc(format!("connect provider: {err}")))?
            .erased();
        let chain_id = match chain_id_override {
            Some(value) => ChainId(value),
            None => ChainId(
                provider
                    .get_chain_id()
                    .await
                    .map_err(|err| ChainwatchError::Rpc(format!("eth_chainId failed: {err}")))?,
            ),
        };
        Ok(Self {
            provider: Arc::new(provider),
            chain_id,
            receipt_concurrency,
        })
    }

    async fn raw_request(&self, method: &'static str, params: Value) -> Result<Value> {
        debug!(method, "rpc request");
        self.provider
            .raw_request(method.into(), params)
            .await
            .map_err(|err| ChainwatchError::Rpc(format!("{method} failed: {err}")))
    }

    async fn receipt(&self, tx_hash: B256) -> Result<Option<Value>> {
        let value = self
            .raw_request("eth_getTransactionReceipt", json!([tx_hash.to_string()]))
            .await?;
        if value.is_null() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }
}

#[async_trait]
impl BlockchainClient for AlloyBlockchainClient {
    async fn chain_id(&self) -> Result<ChainId> {
        Ok(self.chain_id)
    }

    async fn latest_block_number(&self) -> Result<BlockNumber> {
        let latest = self
            .provider
            .get_block_number()
            .await
            .map_err(|err| ChainwatchError::Rpc(format!("eth_blockNumber failed: {err}")))?;
        Ok(BlockNumber(latest))
    }

    #[instrument(skip(self), fields(block_number = number.0, chain_id = self.chain_id.0))]
    async fn fetch_block(&self, number: BlockNumber) -> Result<FetchedBlock> {
        let block_tag = format!("0x{:x}", number.0);
        let raw_block = self
            .raw_request("eth_getBlockByNumber", json!([block_tag, true]))
            .await?;
        parse_fetched_block(self.chain_id, raw_block, self).await
    }
}

async fn parse_fetched_block(
    chain_id: ChainId,
    raw_block: Value,
    client: &AlloyBlockchainClient,
) -> Result<FetchedBlock> {
    if raw_block.is_null() {
        return Err(ChainwatchError::Rpc("block not found".to_owned()));
    }

    let hash = parse_required_hash(&raw_block, "hash")?;
    let parent_hash = parse_required_hash(&raw_block, "parentHash")?;
    let number = BlockNumber(hex_u64(
        raw_block
            .get("number")
            .ok_or_else(|| ChainwatchError::Rpc("missing block.number".to_owned()))?,
        "block.number",
    )?);
    let timestamp = hex_u64(
        raw_block
            .get("timestamp")
            .ok_or_else(|| ChainwatchError::Rpc("missing block.timestamp".to_owned()))?,
        "block.timestamp",
    )?;

    let tx_values = raw_block
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or_else(|| ChainwatchError::Rpc("block.transactions must be an array".to_owned()))?;

    let tx_hashes = tx_values
        .iter()
        .map(|tx| parse_required_hash(tx, "hash"))
        .collect::<Result<Vec<_>>>()?;

    let receipts = futures::stream::iter(tx_hashes.iter().copied())
        .map(|hash| async move { client.receipt(hash).await.map(|receipt| (hash, receipt)) })
        .buffer_unordered(client.receipt_concurrency)
        .try_collect::<Vec<_>>()
        .await?;

    let receipt_map = receipts
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();

    let mut transactions = Vec::with_capacity(tx_values.len());
    let mut logs = Vec::new();

    for tx_value in tx_values {
        let tx_hash = parse_required_hash(tx_value, "hash")?;
        let receipt = receipt_map
            .get(&tx_hash)
            .and_then(std::option::Option::as_ref);
        let status = receipt
            .and_then(|value| value.get("status"))
            .map(|status_value| hex_u64(status_value, "receipt.status").map(|raw| raw == 1))
            .transpose()?;
        let gas_used = receipt
            .and_then(|value| value.get("gasUsed"))
            .map(|gas_value| hex_u64(gas_value, "receipt.gasUsed"))
            .transpose()?;

        transactions.push(parse_transaction(
            chain_id, number, tx_value, status, gas_used,
        )?);

        if let Some(receipt_value) = receipt {
            let receipt_logs = receipt_value
                .get("logs")
                .and_then(Value::as_array)
                .ok_or_else(|| ChainwatchError::Rpc("receipt.logs must be an array".to_owned()))?;
            for raw_log in receipt_logs {
                logs.push(parse_log(chain_id, timestamp, raw_log)?);
            }
        }
    }

    let block = Block {
        chain_id,
        number,
        hash,
        parent_hash,
        timestamp,
        tx_count: u64::try_from(tx_values.len()).map_err(|err| {
            ChainwatchError::Internal(format!("transaction count overflow: {err}"))
        })?,
    };

    Ok(FetchedBlock {
        block,
        transactions,
        logs,
    })
}

fn parse_transaction(
    chain_id: ChainId,
    block_number: BlockNumber,
    tx_value: &Value,
    status: Option<bool>,
    gas_used: Option<u64>,
) -> Result<Transaction> {
    let hash = parse_required_hash(tx_value, "hash")?;
    let tx_index =
        optional_hex_u64(tx_value.get("transactionIndex"), "transactionIndex")?.unwrap_or(0);
    let from = parse_required_address(tx_value, "from")?;
    let to = match tx_value.get("to") {
        Some(value) if !value.is_null() => Some(parse_value_address(value, "to")?),
        _ => None,
    };
    let value = tx_value
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| ChainwatchError::Rpc("transaction.value missing".to_owned()))?;
    let input = tx_value
        .get("input")
        .or_else(|| tx_value.get("data"))
        .and_then(Value::as_str)
        .unwrap_or("0x");
    Ok(Transaction {
        chain_id,
        hash,
        block_number,
        tx_index,
        from,
        to,
        value: parse_u256(value)?,
        input: parse_hex_bytes(input)?,
        status,
        gas_used,
    })
}

fn parse_log(chain_id: ChainId, fallback_timestamp: u64, raw_log: &Value) -> Result<RawLog> {
    let block_number = BlockNumber(hex_u64(
        raw_log
            .get("blockNumber")
            .ok_or_else(|| ChainwatchError::Rpc("log.blockNumber missing".to_owned()))?,
        "log.blockNumber",
    )?);
    let tx_hash = parse_required_hash(raw_log, "transactionHash")?;
    let log_index = optional_hex_u64(raw_log.get("logIndex"), "log.logIndex")?.unwrap_or(0);
    let address = parse_required_address(raw_log, "address")?;
    let topics = raw_log
        .get("topics")
        .and_then(Value::as_array)
        .ok_or_else(|| ChainwatchError::Rpc("log.topics must be an array".to_owned()))?
        .iter()
        .map(|topic| {
            topic
                .as_str()
                .ok_or_else(|| ChainwatchError::Rpc("log topic is not a string".to_owned()))
                .and_then(parse_hash)
        })
        .collect::<Result<Vec<_>>>()?;
    let data = raw_log.get("data").and_then(Value::as_str).unwrap_or("0x");
    let timestamp = optional_hex_u64(raw_log.get("blockTimestamp"), "log.blockTimestamp")?
        .unwrap_or(fallback_timestamp);
    let removed = raw_log
        .get("removed")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(RawLog {
        chain_id,
        block_number,
        tx_hash,
        log_index,
        address,
        topics,
        data: parse_hex_bytes(data)?,
        removed,
        timestamp,
    })
}

fn parse_required_hash(value: &Value, field: &str) -> Result<B256> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ChainwatchError::Rpc(format!("missing hash field `{field}`")))
        .and_then(parse_hash)
}

fn parse_required_address(value: &Value, field: &str) -> Result<Address> {
    value
        .get(field)
        .ok_or_else(|| ChainwatchError::Rpc(format!("missing address field `{field}`")))
        .and_then(|address| parse_value_address(address, field))
}

fn parse_value_address(value: &Value, field: &str) -> Result<Address> {
    value
        .as_str()
        .ok_or_else(|| ChainwatchError::Rpc(format!("address field `{field}` is not a string")))
        .and_then(parse_address)
}
