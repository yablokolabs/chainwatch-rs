use std::sync::Arc;

use alloy::primitives::b256;
use async_trait::async_trait;

use chainwatch_rs::{
    application::{
        ports::{BlockchainClient, Repository},
        services::AlertingService,
    },
    config::IndexerSettings,
    domain::{Block, BlockNumber, Chain, ChainId, FetchedBlock, Result},
    indexer::{Erc20EventDecoder, Indexer},
    infrastructure::memory::MemoryRepository,
    risk::{RiskEngine, RiskEngineConfig},
};

struct MockClient;

#[async_trait]
impl BlockchainClient for MockClient {
    async fn chain_id(&self) -> Result<ChainId> {
        Ok(ChainId(1))
    }

    async fn latest_block_number(&self) -> Result<BlockNumber> {
        Ok(BlockNumber(3))
    }

    async fn fetch_block(&self, number: BlockNumber) -> Result<FetchedBlock> {
        Ok(FetchedBlock {
            block: Block {
                chain_id: ChainId(1),
                number,
                hash: if number.0 == 1 {
                    b256!("1111111111111111111111111111111111111111111111111111111111111111")
                } else {
                    b256!("2222222222222222222222222222222222222222222222222222222222222222")
                },
                parent_hash: b256!(
                    "0000000000000000000000000000000000000000000000000000000000000000"
                ),
                timestamp: 1_700_000_000 + number.0,
                tx_count: 0,
            },
            transactions: Vec::new(),
            logs: Vec::new(),
        })
    }
}

#[tokio::test]
async fn indexer_persists_safe_blocks_with_mock_client() -> anyhow::Result<()> {
    let repo = Arc::new(MemoryRepository::new()) as Arc<dyn Repository>;
    let risk_engine = RiskEngine::new(RiskEngineConfig {
        large_transfer_threshold_wei: alloy::primitives::U256::from_limbs([100, 0, 0, 0]),
        high_frequency_threshold: 10,
        suspicious_contract_rule_enabled: false,
    });
    let alerting = AlertingService::new(repo.clone(), risk_engine, 60);
    let indexer = Indexer::new(
        Chain {
            id: ChainId(1),
            name: "mock".to_owned(),
            rpc_url_redacted: "mock://".to_owned(),
        },
        Arc::new(MockClient) as Arc<dyn BlockchainClient>,
        repo.clone(),
        Arc::new(Erc20EventDecoder::new()),
        alerting,
        IndexerSettings {
            start_block: 1,
            reorg_confirmations: 1,
            backfill_batch_size: 10,
            rpc_concurrency: 2,
            poll_interval_seconds: 1,
            max_retries: 1,
            initial_retry_delay_ms: 1,
            max_retry_delay_ms: 2,
        },
        None,
    );

    let indexed = indexer.run_once().await?;
    assert_eq!(indexed, 2);
    let latest = repo.latest_block(ChainId(1)).await?;
    assert_eq!(latest.map(|block| block.number), Some(BlockNumber(2)));
    Ok(())
}
