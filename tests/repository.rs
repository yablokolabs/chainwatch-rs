use alloy::primitives::{U256, address, b256};
use chrono::Utc;

use chainwatch_rs::{
    application::ports::Repository,
    config::DatabaseSettings,
    domain::{Block, BlockNumber, Chain, ChainId, Page, TokenTransfer},
    infrastructure::{memory::MemoryRepository, postgres::PostgresRepository},
};

fn chain() -> Chain {
    Chain {
        id: ChainId(1),
        name: "test".to_owned(),
        rpc_url_redacted: "http://localhost:8545".to_owned(),
    }
}

fn block() -> Block {
    Block {
        chain_id: ChainId(1),
        number: BlockNumber(10),
        hash: b256!("1111111111111111111111111111111111111111111111111111111111111111"),
        parent_hash: b256!("0000000000000000000000000000000000000000000000000000000000000000"),
        timestamp: 1_700_000_000,
        tx_count: 0,
    }
}

fn transfer() -> TokenTransfer {
    TokenTransfer {
        chain_id: ChainId(1),
        token_address: address!("0000000000000000000000000000000000001000"),
        from: address!("0000000000000000000000000000000000000001"),
        to: address!("0000000000000000000000000000000000000002"),
        amount: U256::from_limbs([42, 0, 0, 0]),
        tx_hash: b256!("2222222222222222222222222222222222222222222222222222222222222222"),
        block_number: BlockNumber(10),
        log_index: 0,
        timestamp: 1_700_000_000,
    }
}

#[tokio::test]
async fn memory_repository_transfer_queries() -> anyhow::Result<()> {
    let repo = MemoryRepository::new();
    repo.upsert_chain(&chain()).await?;
    repo.upsert_blocks(&[block()]).await?;
    repo.upsert_token_transfers(&[transfer()]).await?;

    let transfers = repo
        .list_transfers_by_address(
            ChainId(1),
            address!("0000000000000000000000000000000000000001"),
            Page::new(Some(10), None),
        )
        .await?;
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].amount, U256::from_limbs([42, 0, 0, 0]));
    Ok(())
}

#[tokio::test]
async fn postgres_repository_connects_when_database_url_is_set() -> anyhow::Result<()> {
    let Ok(url) = std::env::var("CHAINWATCH__DATABASE__URL") else {
        return Ok(());
    };
    let repo = PostgresRepository::connect(&DatabaseSettings {
        url,
        max_connections: 2,
        run_migrations: true,
    })
    .await?;
    repo.upsert_chain(&chain()).await?;
    repo.upsert_blocks(&[block()]).await?;
    let latest = repo.latest_block(ChainId(1)).await?;
    assert!(latest.is_some());
    let entry = chainwatch_rs::application::services::build_watchlist_entry(
        ChainId(1),
        address!("0000000000000000000000000000000000000003"),
        Some(format!("ci-{}", Utc::now().timestamp())),
    );
    repo.add_watchlist(&entry).await?;
    let watchlist = repo.list_watchlist(ChainId(1)).await?;
    assert!(watchlist.iter().any(|item| item.address == entry.address));
    Ok(())
}
