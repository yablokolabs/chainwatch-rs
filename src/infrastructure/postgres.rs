use alloy::primitives::{Address, B256};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder, postgres::PgPoolOptions};

use crate::{
    application::ports::Repository,
    config::DatabaseSettings,
    domain::{
        Alert, AlertSeverity, Block, BlockNumber, Chain, ChainId, ChainwatchError, DecodedEvent,
        IndexerState, Page, RawLog, Result, TokenTransfer, Transaction, WatchlistEntry,
        codec::{
            address_to_hex, checked_i64, checked_u64, hash_to_hex, parse_address, parse_hash,
            parse_u256, u256_to_decimal,
        },
    },
};

#[derive(Clone)]
pub struct PostgresRepository {
    pool: PgPool,
}

impl PostgresRepository {
    pub async fn connect(settings: &DatabaseSettings) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(settings.max_connections)
            .connect(&settings.url)
            .await?;
        if settings.run_migrations {
            sqlx::migrate!("./migrations").run(&pool).await?;
        }
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[derive(FromRow)]
struct DbBlock {
    chain_id: i64,
    number: i64,
    hash: String,
    parent_hash: String,
    timestamp: i64,
    tx_count: i64,
}

#[derive(FromRow)]
struct DbTransaction {
    chain_id: i64,
    hash: String,
    block_number: i64,
    tx_index: i64,
    from_address: String,
    to_address: Option<String>,
    value_wei: String,
    input: Vec<u8>,
    status: Option<bool>,
    gas_used: Option<i64>,
}

#[derive(FromRow)]
struct DbTransfer {
    chain_id: i64,
    token_address: String,
    from_address: String,
    to_address: String,
    amount_wei: String,
    tx_hash: String,
    block_number: i64,
    log_index: i64,
    timestamp: i64,
}

#[derive(FromRow)]
struct DbAlert {
    id: uuid::Uuid,
    chain_id: i64,
    rule: String,
    severity: String,
    address: Option<String>,
    tx_hash: Option<String>,
    block_number: Option<i64>,
    message: String,
    metadata: Value,
    created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct DbWatchlistEntry {
    chain_id: i64,
    address: String,
    label: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct DbIndexerState {
    chain_id: i64,
    latest_block: Option<i64>,
    latest_hash: Option<String>,
    updated_at: DateTime<Utc>,
}

fn chain_i64(chain_id: ChainId) -> Result<i64> {
    checked_i64(chain_id.0, "chain_id")
}

fn block_i64(block_number: BlockNumber) -> Result<i64> {
    checked_i64(block_number.0, "block_number")
}

fn db_block(row: DbBlock) -> Result<Block> {
    Ok(Block {
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        number: BlockNumber(checked_u64(row.number, "block_number")?),
        hash: parse_hash(&row.hash)?,
        parent_hash: parse_hash(&row.parent_hash)?,
        timestamp: checked_u64(row.timestamp, "timestamp")?,
        tx_count: checked_u64(row.tx_count, "tx_count")?,
    })
}

fn db_transaction(row: DbTransaction) -> Result<Transaction> {
    Ok(Transaction {
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        hash: parse_hash(&row.hash)?,
        block_number: BlockNumber(checked_u64(row.block_number, "block_number")?),
        tx_index: checked_u64(row.tx_index, "tx_index")?,
        from: parse_address(&row.from_address)?,
        to: row.to_address.as_deref().map(parse_address).transpose()?,
        value: parse_u256(&row.value_wei)?,
        input: row.input,
        status: row.status,
        gas_used: row
            .gas_used
            .map(|v| checked_u64(v, "gas_used"))
            .transpose()?,
    })
}

fn db_transfer(row: DbTransfer) -> Result<TokenTransfer> {
    Ok(TokenTransfer {
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        token_address: parse_address(&row.token_address)?,
        from: parse_address(&row.from_address)?,
        to: parse_address(&row.to_address)?,
        amount: parse_u256(&row.amount_wei)?,
        tx_hash: parse_hash(&row.tx_hash)?,
        block_number: BlockNumber(checked_u64(row.block_number, "block_number")?),
        log_index: checked_u64(row.log_index, "log_index")?,
        timestamp: checked_u64(row.timestamp, "timestamp")?,
    })
}

fn severity_from_db(value: &str) -> Result<AlertSeverity> {
    match value {
        "low" => Ok(AlertSeverity::Low),
        "medium" => Ok(AlertSeverity::Medium),
        "high" => Ok(AlertSeverity::High),
        "critical" => Ok(AlertSeverity::Critical),
        other => Err(ChainwatchError::Validation(format!(
            "unknown alert severity `{other}`"
        ))),
    }
}

fn db_alert(row: DbAlert) -> Result<Alert> {
    Ok(Alert {
        id: row.id,
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        rule: row.rule,
        severity: severity_from_db(&row.severity)?,
        address: row.address.as_deref().map(parse_address).transpose()?,
        tx_hash: row.tx_hash.as_deref().map(parse_hash).transpose()?,
        block_number: row
            .block_number
            .map(|value| checked_u64(value, "block_number").map(BlockNumber))
            .transpose()?,
        message: row.message,
        metadata: row.metadata,
        created_at: row.created_at,
    })
}

fn db_watchlist(row: DbWatchlistEntry) -> Result<WatchlistEntry> {
    Ok(WatchlistEntry {
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        address: parse_address(&row.address)?,
        label: row.label,
        created_at: row.created_at,
    })
}

fn db_indexer_state(row: DbIndexerState) -> Result<IndexerState> {
    Ok(IndexerState {
        chain_id: ChainId(checked_u64(row.chain_id, "chain_id")?),
        latest_block: row
            .latest_block
            .map(|value| checked_u64(value, "latest_block").map(BlockNumber))
            .transpose()?,
        latest_hash: row.latest_hash.as_deref().map(parse_hash).transpose()?,
        updated_at: row.updated_at,
    })
}

#[async_trait]
impl Repository for PostgresRepository {
    async fn upsert_chain(&self, chain: &Chain) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO chains (chain_id, name, rpc_url_redacted)
            VALUES ($1, $2, $3)
            ON CONFLICT (chain_id) DO UPDATE SET
                name = EXCLUDED.name,
                rpc_url_redacted = EXCLUDED.rpc_url_redacted
            "#,
        )
        .bind(chain_i64(chain.id)?)
        .bind(&chain.name)
        .bind(&chain.rpc_url_redacted)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_blocks(&self, blocks: &[Block]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let rows = blocks
            .iter()
            .map(|block| {
                Ok((
                    chain_i64(block.chain_id)?,
                    block_i64(block.number)?,
                    hash_to_hex(&block.hash),
                    hash_to_hex(&block.parent_hash),
                    checked_i64(block.timestamp, "timestamp")?,
                    checked_i64(block.tx_count, "tx_count")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO blocks (chain_id, number, hash, parent_hash, timestamp, tx_count) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5);
        });
        qb.push(
            " ON CONFLICT (chain_id, number) DO UPDATE SET hash = EXCLUDED.hash, parent_hash = EXCLUDED.parent_hash, timestamp = EXCLUDED.timestamp, tx_count = EXCLUDED.tx_count",
        );
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn upsert_transactions(&self, transactions: &[Transaction]) -> Result<()> {
        if transactions.is_empty() {
            return Ok(());
        }
        let rows = transactions
            .iter()
            .map(|tx| {
                Ok((
                    chain_i64(tx.chain_id)?,
                    hash_to_hex(&tx.hash),
                    block_i64(tx.block_number)?,
                    checked_i64(tx.tx_index, "tx_index")?,
                    address_to_hex(&tx.from),
                    tx.to.as_ref().map(address_to_hex),
                    u256_to_decimal(&tx.value),
                    tx.input.clone(),
                    tx.status,
                    tx.gas_used
                        .map(|value| checked_i64(value, "gas_used"))
                        .transpose()?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO transactions (chain_id, hash, block_number, tx_index, from_address, to_address, value_wei, input, status, gas_used) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7)
                .push_bind(row.8)
                .push_bind(row.9);
        });
        qb.push(
            " ON CONFLICT (chain_id, hash) DO UPDATE SET block_number = EXCLUDED.block_number, tx_index = EXCLUDED.tx_index, from_address = EXCLUDED.from_address, to_address = EXCLUDED.to_address, value_wei = EXCLUDED.value_wei, input = EXCLUDED.input, status = EXCLUDED.status, gas_used = EXCLUDED.gas_used",
        );
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn upsert_logs(&self, logs: &[RawLog]) -> Result<()> {
        if logs.is_empty() {
            return Ok(());
        }
        let rows = logs
            .iter()
            .map(|log| {
                Ok((
                    chain_i64(log.chain_id)?,
                    block_i64(log.block_number)?,
                    hash_to_hex(&log.tx_hash),
                    checked_i64(log.log_index, "log_index")?,
                    address_to_hex(&log.address),
                    json!(log.topics.iter().map(hash_to_hex).collect::<Vec<_>>()),
                    log.data.clone(),
                    log.removed,
                    checked_i64(log.timestamp, "timestamp")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO logs (chain_id, block_number, tx_hash, log_index, address, topics, data, removed, timestamp) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7)
                .push_bind(row.8);
        });
        qb.push(
            " ON CONFLICT (chain_id, tx_hash, log_index) DO UPDATE SET block_number = EXCLUDED.block_number, address = EXCLUDED.address, topics = EXCLUDED.topics, data = EXCLUDED.data, removed = EXCLUDED.removed, timestamp = EXCLUDED.timestamp",
        );
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn upsert_decoded_events(&self, events: &[DecodedEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let rows = events
            .iter()
            .map(|event| {
                Ok((
                    chain_i64(event.chain_id)?,
                    event.kind.as_str().to_owned(),
                    address_to_hex(&event.token_address),
                    hash_to_hex(&event.tx_hash),
                    block_i64(event.block_number)?,
                    checked_i64(event.log_index, "log_index")?,
                    checked_i64(event.timestamp, "timestamp")?,
                    event.payload.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO decoded_events (chain_id, event_type, token_address, tx_hash, block_number, log_index, timestamp, payload) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7);
        });
        qb.push(
            " ON CONFLICT (chain_id, tx_hash, log_index, event_type) DO UPDATE SET token_address = EXCLUDED.token_address, block_number = EXCLUDED.block_number, timestamp = EXCLUDED.timestamp, payload = EXCLUDED.payload",
        );
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn upsert_token_transfers(&self, transfers: &[TokenTransfer]) -> Result<()> {
        if transfers.is_empty() {
            return Ok(());
        }
        let rows = transfers
            .iter()
            .map(|transfer| {
                Ok((
                    chain_i64(transfer.chain_id)?,
                    address_to_hex(&transfer.token_address),
                    address_to_hex(&transfer.from),
                    address_to_hex(&transfer.to),
                    u256_to_decimal(&transfer.amount),
                    hash_to_hex(&transfer.tx_hash),
                    block_i64(transfer.block_number)?,
                    checked_i64(transfer.log_index, "log_index")?,
                    checked_i64(transfer.timestamp, "timestamp")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO token_transfers (chain_id, token_address, from_address, to_address, amount_wei, tx_hash, block_number, log_index, timestamp) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7)
                .push_bind(row.8);
        });
        qb.push(
            " ON CONFLICT (chain_id, tx_hash, log_index) DO UPDATE SET token_address = EXCLUDED.token_address, from_address = EXCLUDED.from_address, to_address = EXCLUDED.to_address, amount_wei = EXCLUDED.amount_wei, block_number = EXCLUDED.block_number, timestamp = EXCLUDED.timestamp",
        );
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn insert_alerts(&self, alerts: &[Alert]) -> Result<()> {
        if alerts.is_empty() {
            return Ok(());
        }
        let rows = alerts
            .iter()
            .map(|alert| {
                Ok((
                    alert.id,
                    chain_i64(alert.chain_id)?,
                    alert.rule.clone(),
                    alert.severity.as_str().to_owned(),
                    alert.address.as_ref().map(address_to_hex),
                    alert.tx_hash.as_ref().map(hash_to_hex),
                    alert.block_number.map(block_i64).transpose()?,
                    alert.message.clone(),
                    alert.metadata.clone(),
                    alert.created_at,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "INSERT INTO alerts (id, chain_id, rule, severity, address, tx_hash, block_number, message, metadata, created_at) ",
        );
        qb.push_values(rows, |mut b, row| {
            b.push_bind(row.0)
                .push_bind(row.1)
                .push_bind(row.2)
                .push_bind(row.3)
                .push_bind(row.4)
                .push_bind(row.5)
                .push_bind(row.6)
                .push_bind(row.7)
                .push_bind(row.8)
                .push_bind(row.9);
        });
        qb.push(" ON CONFLICT (id) DO NOTHING");
        qb.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn get_indexer_state(&self, chain_id: ChainId) -> Result<Option<IndexerState>> {
        let row = sqlx::query_as::<_, DbIndexerState>(
            "SELECT chain_id, latest_block, latest_hash, updated_at FROM indexer_state WHERE chain_id = $1",
        )
        .bind(chain_i64(chain_id)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(db_indexer_state).transpose()
    }

    async fn set_indexer_state(&self, state: &IndexerState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO indexer_state (chain_id, latest_block, latest_hash, updated_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (chain_id) DO UPDATE SET
                latest_block = EXCLUDED.latest_block,
                latest_hash = EXCLUDED.latest_hash,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(chain_i64(state.chain_id)?)
        .bind(state.latest_block.map(block_i64).transpose()?)
        .bind(state.latest_hash.as_ref().map(hash_to_hex))
        .bind(state.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn rollback_to_block(&self, chain_id: ChainId, block_number: BlockNumber) -> Result<()> {
        let chain = chain_i64(chain_id)?;
        let block = block_i64(block_number)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM alerts WHERE chain_id = $1 AND block_number IS NOT NULL AND block_number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM token_transfers WHERE chain_id = $1 AND block_number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM decoded_events WHERE chain_id = $1 AND block_number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM logs WHERE chain_id = $1 AND block_number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM transactions WHERE chain_id = $1 AND block_number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM blocks WHERE chain_id = $1 AND number > $2")
            .bind(chain)
            .bind(block)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            UPDATE indexer_state
            SET latest_block = $2,
                latest_hash = (SELECT hash FROM blocks WHERE chain_id = $1 AND number = $2),
                updated_at = now()
            WHERE chain_id = $1
            "#,
        )
        .bind(chain)
        .bind(block)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn latest_block(&self, chain_id: ChainId) -> Result<Option<Block>> {
        let row = sqlx::query_as::<_, DbBlock>(
            "SELECT chain_id, number, hash, parent_hash, timestamp, tx_count FROM blocks WHERE chain_id = $1 ORDER BY number DESC LIMIT 1",
        )
        .bind(chain_i64(chain_id)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(db_block).transpose()
    }

    async fn get_transaction(&self, chain_id: ChainId, hash: B256) -> Result<Option<Transaction>> {
        let row = sqlx::query_as::<_, DbTransaction>(
            r#"
            SELECT chain_id, hash, block_number, tx_index, from_address, to_address, value_wei, input, status, gas_used
            FROM transactions
            WHERE chain_id = $1 AND hash = $2
            "#,
        )
        .bind(chain_i64(chain_id)?)
        .bind(hash_to_hex(&hash))
        .fetch_optional(&self.pool)
        .await?;
        row.map(db_transaction).transpose()
    }

    async fn list_transfers_by_address(
        &self,
        chain_id: ChainId,
        address: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>> {
        let rows = sqlx::query_as::<_, DbTransfer>(
            r#"
            SELECT chain_id, token_address, from_address, to_address, amount_wei, tx_hash, block_number, log_index, timestamp
            FROM token_transfers
            WHERE chain_id = $1 AND (from_address = $2 OR to_address = $2)
            ORDER BY block_number DESC, log_index DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(chain_i64(chain_id)?)
        .bind(address_to_hex(&address))
        .bind(i64::from(page.limit))
        .bind(i64::from(page.offset))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(db_transfer).collect()
    }

    async fn list_transfers_by_token(
        &self,
        chain_id: ChainId,
        token: Address,
        page: Page,
    ) -> Result<Vec<TokenTransfer>> {
        let rows = sqlx::query_as::<_, DbTransfer>(
            r#"
            SELECT chain_id, token_address, from_address, to_address, amount_wei, tx_hash, block_number, log_index, timestamp
            FROM token_transfers
            WHERE chain_id = $1 AND token_address = $2
            ORDER BY block_number DESC, log_index DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(chain_i64(chain_id)?)
        .bind(address_to_hex(&token))
        .bind(i64::from(page.limit))
        .bind(i64::from(page.offset))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(db_transfer).collect()
    }

    async fn list_alerts(&self, chain_id: ChainId, page: Page) -> Result<Vec<Alert>> {
        let rows = sqlx::query_as::<_, DbAlert>(
            r#"
            SELECT id, chain_id, rule, severity, address, tx_hash, block_number, message, metadata, created_at
            FROM alerts
            WHERE chain_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(chain_i64(chain_id)?)
        .bind(i64::from(page.limit))
        .bind(i64::from(page.offset))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(db_alert).collect()
    }

    async fn add_watchlist(&self, entry: &WatchlistEntry) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO watchlist (chain_id, address, label, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (chain_id, address) DO UPDATE SET
                label = EXCLUDED.label
            "#,
        )
        .bind(chain_i64(entry.chain_id)?)
        .bind(address_to_hex(&entry.address))
        .bind(&entry.label)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_watchlist(&self, chain_id: ChainId, address: Address) -> Result<bool> {
        let result = sqlx::query("DELETE FROM watchlist WHERE chain_id = $1 AND address = $2")
            .bind(chain_i64(chain_id)?)
            .bind(address_to_hex(&address))
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_watchlist(&self, chain_id: ChainId) -> Result<Vec<WatchlistEntry>> {
        let rows = sqlx::query_as::<_, DbWatchlistEntry>(
            "SELECT chain_id, address, label, created_at FROM watchlist WHERE chain_id = $1 ORDER BY created_at DESC",
        )
        .bind(chain_i64(chain_id)?)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(db_watchlist).collect()
    }

    async fn count_transfers_by_address_since(
        &self,
        chain_id: ChainId,
        address: Address,
        since_timestamp: u64,
    ) -> Result<u64> {
        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::BIGINT
            FROM token_transfers
            WHERE chain_id = $1
              AND timestamp >= $2
              AND (from_address = $3 OR to_address = $3)
            "#,
        )
        .bind(chain_i64(chain_id)?)
        .bind(checked_i64(since_timestamp, "since_timestamp")?)
        .bind(address_to_hex(&address))
        .fetch_one(&self.pool)
        .await?;
        checked_u64(row.0, "transfer_count")
    }
}
