use alloy::primitives::{Address, B256, U256};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChainId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockNumber(pub u64);

impl BlockNumber {
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    #[must_use]
    pub fn saturating_sub(self, rhs: u64) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    pub id: ChainId,
    pub name: String,
    pub rpc_url_redacted: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub chain_id: ChainId,
    pub number: BlockNumber,
    pub hash: B256,
    pub parent_hash: B256,
    pub timestamp: u64,
    pub tx_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub chain_id: ChainId,
    pub hash: B256,
    pub block_number: BlockNumber,
    pub tx_index: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub input: Vec<u8>,
    pub status: Option<bool>,
    pub gas_used: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawLog {
    pub chain_id: ChainId,
    pub block_number: BlockNumber,
    pub tx_hash: B256,
    pub log_index: u64,
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Vec<u8>,
    pub removed: bool,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecodedEventKind {
    Erc20Transfer,
    Erc20Approval,
}

impl DecodedEventKind {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Erc20Transfer => "erc20_transfer",
            Self::Erc20Approval => "erc20_approval",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedEvent {
    pub chain_id: ChainId,
    pub kind: DecodedEventKind,
    pub token_address: Address,
    pub tx_hash: B256,
    pub block_number: BlockNumber,
    pub log_index: u64,
    pub timestamp: u64,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTransfer {
    pub chain_id: ChainId,
    pub token_address: Address,
    pub from: Address,
    pub to: Address,
    pub amount: U256,
    pub tx_hash: B256,
    pub block_number: BlockNumber,
    pub log_index: u64,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub chain_id: ChainId,
    pub address: Address,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl AlertSeverity {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub id: Uuid,
    pub chain_id: ChainId,
    pub rule: String,
    pub severity: AlertSeverity,
    pub address: Option<Address>,
    pub tx_hash: Option<B256>,
    pub block_number: Option<BlockNumber>,
    pub message: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerState {
    pub chain_id: ChainId,
    pub latest_block: Option<BlockNumber>,
    pub latest_hash: Option<B256>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedBlock {
    pub block: Block,
    pub transactions: Vec<Transaction>,
    pub logs: Vec<RawLog>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub limit: u32,
    pub offset: u32,
}

impl Page {
    #[must_use]
    pub fn new(limit: Option<u32>, offset: Option<u32>) -> Self {
        let limit = limit.unwrap_or(50).clamp(1, 500);
        let offset = offset.unwrap_or(0);
        Self { limit, offset }
    }
}
