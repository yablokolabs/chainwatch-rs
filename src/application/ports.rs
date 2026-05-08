use alloy::primitives::{Address, B256};
use async_trait::async_trait;

use crate::domain::{
    Alert, Block, BlockNumber, Chain, ChainId, DecodedEvent, FetchedBlock, IndexerState, Page,
    RawLog, Result, TokenTransfer, Transaction, WatchlistEntry,
};

#[async_trait]
pub trait BlockchainClient: Send + Sync {
    async fn chain_id(&self) -> Result<ChainId>;
    async fn latest_block_number(&self) -> Result<BlockNumber>;
    async fn fetch_block(&self, number: BlockNumber) -> Result<FetchedBlock>;
}

pub trait EventDecoder: Send + Sync {
    fn decode(&self, log: &RawLog) -> Result<Vec<DecodedEvent>>;
}

#[async_trait]
pub trait Repository: Send + Sync {
    async fn upsert_chain(&self, chain: &Chain) -> Result<()>;
    async fn upsert_blocks(&self, blocks: &[Block]) -> Result<()>;
    async fn upsert_transactions(&self, transactions: &[Transaction]) -> Result<()>;
    async fn upsert_logs(&self, logs: &[RawLog]) -> Result<()>;
    async fn upsert_decoded_events(&self, events: &[DecodedEvent]) -> Result<()>;
    async fn upsert_token_transfers(&self, transfers: &[TokenTransfer]) -> Result<()>;
    async fn insert_alerts(&self, alerts: &[Alert]) -> Result<()>;

    async fn get_indexer_state(&self, chain_id: ChainId) -> Result<Option<IndexerState>>;
    async fn set_indexer_state(&self, state: &IndexerState) -> Result<()>;
    async fn rollback_to_block(&self, chain_id: ChainId, block_number: BlockNumber) -> Result<()>;

    async fn latest_block(&self, chain_id: ChainId) -> Result<Option<Block>>;
    async fn get_transaction(&self, chain_id: ChainId, hash: B256) -> Result<Option<Transaction>>;
    async fn list_transfers_by_address(
        &self,
        chain_id: ChainId,
        address: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>>;
    async fn list_transfers_by_token(
        &self,
        chain_id: ChainId,
        token: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>>;
    async fn list_alerts(&self, chain_id: ChainId, page: Page) -> Result<Vec<Alert>>;

    async fn add_watchlist(&self, entry: &WatchlistEntry) -> Result<()>;
    async fn remove_watchlist(&self, chain_id: ChainId, address: Address) -> Result<bool>;
    async fn list_watchlist(&self, chain_id: ChainId) -> Result<Vec<WatchlistEntry>>;
    async fn count_transfers_by_address_since(
        &self,
        chain_id: ChainId,
        address: Address,
        since_timestamp: u64,
    ) -> Result<u64>;
}
