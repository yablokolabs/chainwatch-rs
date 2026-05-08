use std::{collections::HashMap, sync::RwLock};

use alloy::primitives::{Address, B256};
use async_trait::async_trait;
use chrono::Utc;

use crate::{
    application::ports::Repository,
    domain::{
        Alert, Block, BlockNumber, Chain, ChainId, ChainwatchError, DecodedEvent, IndexerState,
        Page, RawLog, Result, TokenTransfer, Transaction, WatchlistEntry,
    },
};

#[derive(Default)]
pub struct MemoryRepository {
    state: RwLock<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    chains: HashMap<ChainId, Chain>,
    blocks: HashMap<(ChainId, BlockNumber), Block>,
    transactions: HashMap<(ChainId, B256), Transaction>,
    logs: HashMap<(ChainId, B256, u64), RawLog>,
    decoded_events: HashMap<(ChainId, B256, u64, String), DecodedEvent>,
    transfers: HashMap<(ChainId, B256, u64), TokenTransfer>,
    watchlist: HashMap<(ChainId, Address), WatchlistEntry>,
    alerts: HashMap<uuid::Uuid, Alert>,
    indexer_state: HashMap<ChainId, IndexerState>,
}

impl MemoryRepository {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn write_state(&self) -> Result<std::sync::RwLockWriteGuard<'_, MemoryState>> {
        self.state
            .write()
            .map_err(|err| ChainwatchError::Internal(format!("memory repo poisoned: {err}")))
    }

    fn read_state(&self) -> Result<std::sync::RwLockReadGuard<'_, MemoryState>> {
        self.state
            .read()
            .map_err(|err| ChainwatchError::Internal(format!("memory repo poisoned: {err}")))
    }
}

#[async_trait]
impl Repository for MemoryRepository {
    async fn upsert_chain(&self, chain: &Chain) -> Result<()> {
        self.write_state()?.chains.insert(chain.id, chain.clone());
        Ok(())
    }

    async fn upsert_blocks(&self, blocks: &[Block]) -> Result<()> {
        let mut state = self.write_state()?;
        for block in blocks {
            state
                .blocks
                .insert((block.chain_id, block.number), block.clone());
        }
        Ok(())
    }

    async fn upsert_transactions(&self, transactions: &[Transaction]) -> Result<()> {
        let mut state = self.write_state()?;
        for tx in transactions {
            state
                .transactions
                .insert((tx.chain_id, tx.hash), tx.clone());
        }
        Ok(())
    }

    async fn upsert_logs(&self, logs: &[RawLog]) -> Result<()> {
        let mut state = self.write_state()?;
        for log in logs {
            state
                .logs
                .insert((log.chain_id, log.tx_hash, log.log_index), log.clone());
        }
        Ok(())
    }

    async fn upsert_decoded_events(&self, events: &[DecodedEvent]) -> Result<()> {
        let mut state = self.write_state()?;
        for event in events {
            state.decoded_events.insert(
                (
                    event.chain_id,
                    event.tx_hash,
                    event.log_index,
                    event.kind.as_str().to_owned(),
                ),
                event.clone(),
            );
        }
        Ok(())
    }

    async fn upsert_token_transfers(&self, transfers: &[TokenTransfer]) -> Result<()> {
        let mut state = self.write_state()?;
        for transfer in transfers {
            state.transfers.insert(
                (transfer.chain_id, transfer.tx_hash, transfer.log_index),
                transfer.clone(),
            );
        }
        Ok(())
    }

    async fn insert_alerts(&self, alerts: &[Alert]) -> Result<()> {
        let mut state = self.write_state()?;
        for alert in alerts {
            state.alerts.insert(alert.id, alert.clone());
        }
        Ok(())
    }

    async fn get_indexer_state(&self, chain_id: ChainId) -> Result<Option<IndexerState>> {
        Ok(self.read_state()?.indexer_state.get(&chain_id).cloned())
    }

    async fn set_indexer_state(&self, state_update: &IndexerState) -> Result<()> {
        self.write_state()?
            .indexer_state
            .insert(state_update.chain_id, state_update.clone());
        Ok(())
    }

    async fn rollback_to_block(&self, chain_id: ChainId, block_number: BlockNumber) -> Result<()> {
        let mut state = self.write_state()?;
        state
            .blocks
            .retain(|(id, number), _| *id != chain_id || *number <= block_number);
        state
            .transactions
            .retain(|(id, _), tx| *id != chain_id || tx.block_number <= block_number);
        state
            .logs
            .retain(|(id, _, _), log| *id != chain_id || log.block_number <= block_number);
        state
            .decoded_events
            .retain(|(id, _, _, _), event| *id != chain_id || event.block_number <= block_number);
        state.transfers.retain(|(id, _, _), transfer| {
            *id != chain_id || transfer.block_number <= block_number
        });
        let latest_hash = state
            .blocks
            .get(&(chain_id, block_number))
            .map(|block| block.hash);
        if let Some(current) = state.indexer_state.get_mut(&chain_id) {
            current.latest_block = Some(block_number);
            current.latest_hash = latest_hash;
            current.updated_at = Utc::now();
        }
        Ok(())
    }

    async fn latest_block(&self, chain_id: ChainId) -> Result<Option<Block>> {
        Ok(self
            .read_state()?
            .blocks
            .values()
            .filter(|block| block.chain_id == chain_id)
            .max_by_key(|block| block.number)
            .cloned())
    }

    async fn get_transaction(&self, chain_id: ChainId, hash: B256) -> Result<Option<Transaction>> {
        Ok(self
            .read_state()?
            .transactions
            .get(&(chain_id, hash))
            .cloned())
    }

    async fn list_transfers_by_address(
        &self,
        chain_id: ChainId,
        address: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>> {
        let mut transfers = self
            .read_state()?
            .transfers
            .values()
            .filter(|transfer| {
                transfer.chain_id == chain_id
                    && (transfer.from == address || transfer.to == address)
            })
            .cloned()
            .collect::<Vec<_>>();
        transfers
            .sort_by_key(|transfer| (std::cmp::Reverse(transfer.block_number), transfer.log_index));
        Ok(transfers
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect())
    }

    async fn list_transfers_by_token(
        &self,
        chain_id: ChainId,
        token: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>> {
        let mut transfers = self
            .read_state()?
            .transfers
            .values()
            .filter(|transfer| transfer.chain_id == chain_id && transfer.token_address == token)
            .cloned()
            .collect::<Vec<_>>();
        transfers
            .sort_by_key(|transfer| (std::cmp::Reverse(transfer.block_number), transfer.log_index));
        Ok(transfers
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect())
    }

    async fn list_alerts(&self, chain_id: ChainId, page: Page) -> Result<Vec<Alert>> {
        let mut alerts = self
            .read_state()?
            .alerts
            .values()
            .filter(|alert| alert.chain_id == chain_id)
            .cloned()
            .collect::<Vec<_>>();
        alerts.sort_by_key(|alert| std::cmp::Reverse(alert.created_at));
        Ok(alerts
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect())
    }

    async fn add_watchlist(&self, entry: &WatchlistEntry) -> Result<()> {
        self.write_state()?
            .watchlist
            .insert((entry.chain_id, entry.address), entry.clone());
        Ok(())
    }

    async fn remove_watchlist(&self, chain_id: ChainId, address: Address) -> Result<bool> {
        Ok(self
            .write_state()?
            .watchlist
            .remove(&(chain_id, address))
            .is_some())
    }

    async fn list_watchlist(&self, chain_id: ChainId) -> Result<Vec<WatchlistEntry>> {
        Ok(self
            .read_state()?
            .watchlist
            .values()
            .filter(|entry| entry.chain_id == chain_id)
            .cloned()
            .collect())
    }

    async fn count_transfers_by_address_since(
        &self,
        chain_id: ChainId,
        address: Address,
        since_timestamp: u64,
    ) -> Result<u64> {
        let count = self
            .read_state()?
            .transfers
            .values()
            .filter(|transfer| {
                transfer.chain_id == chain_id
                    && transfer.timestamp >= since_timestamp
                    && (transfer.from == address || transfer.to == address)
            })
            .count();
        u64::try_from(count)
            .map_err(|err| ChainwatchError::Internal(format!("count overflow: {err}")))
    }
}
