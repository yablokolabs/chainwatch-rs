use redis::AsyncCommands;

use crate::domain::{BlockNumber, ChainId, ChainwatchError, Result};

#[derive(Clone)]
pub struct RedisCache {
    client: redis::Client,
}

impl RedisCache {
    pub fn new(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)
            .map_err(|err| ChainwatchError::Config(format!("redis client: {err}")))?;
        Ok(Self { client })
    }

    pub async fn set_latest_indexed_block(
        &self,
        chain_id: ChainId,
        block_number: BlockNumber,
    ) -> Result<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ChainwatchError::Internal(format!("redis connect: {err}")))?;
        let key = format!("chainwatch:{}:latest_indexed_block", chain_id.0);
        connection
            .set::<_, _, ()>(key, block_number.0)
            .await
            .map_err(|err| ChainwatchError::Internal(format!("redis set latest block: {err}")))
    }

    pub async fn latest_indexed_block(&self, chain_id: ChainId) -> Result<Option<BlockNumber>> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ChainwatchError::Internal(format!("redis connect: {err}")))?;
        let key = format!("chainwatch:{}:latest_indexed_block", chain_id.0);
        let value: Option<u64> = connection
            .get(key)
            .await
            .map_err(|err| ChainwatchError::Internal(format!("redis get latest block: {err}")))?;
        Ok(value.map(BlockNumber))
    }
}
