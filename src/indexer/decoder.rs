use alloy::{primitives::U256, sol, sol_types::SolEvent};
use serde_json::json;

use crate::{
    application::ports::EventDecoder,
    domain::{
        DecodedEvent, DecodedEventKind, RawLog, Result, codec::address_to_hex,
        codec::topic_to_address, codec::u256_to_decimal,
    },
};

sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

#[derive(Clone, Default)]
pub struct Erc20EventDecoder;

impl Erc20EventDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl EventDecoder for Erc20EventDecoder {
    fn decode(&self, log: &RawLog) -> Result<Vec<DecodedEvent>> {
        if log.removed || log.topics.is_empty() {
            return Ok(Vec::new());
        }

        if log.topics[0] == Transfer::SIGNATURE_HASH && log.topics.len() == 3 {
            let from = topic_to_address(&log.topics[1])?;
            let to = topic_to_address(&log.topics[2])?;
            let amount = decode_u256_word(&log.data)?;
            return Ok(vec![DecodedEvent {
                chain_id: log.chain_id,
                kind: DecodedEventKind::Erc20Transfer,
                token_address: log.address,
                tx_hash: log.tx_hash,
                block_number: log.block_number,
                log_index: log.log_index,
                timestamp: log.timestamp,
                payload: json!({
                    "from": address_to_hex(&from),
                    "to": address_to_hex(&to),
                    "amount_wei": u256_to_decimal(&amount),
                }),
            }]);
        }

        if log.topics[0] == Approval::SIGNATURE_HASH && log.topics.len() == 3 {
            let owner = topic_to_address(&log.topics[1])?;
            let spender = topic_to_address(&log.topics[2])?;
            let amount = decode_u256_word(&log.data)?;
            return Ok(vec![DecodedEvent {
                chain_id: log.chain_id,
                kind: DecodedEventKind::Erc20Approval,
                token_address: log.address,
                tx_hash: log.tx_hash,
                block_number: log.block_number,
                log_index: log.log_index,
                timestamp: log.timestamp,
                payload: json!({
                    "owner": address_to_hex(&owner),
                    "spender": address_to_hex(&spender),
                    "amount_wei": u256_to_decimal(&amount),
                }),
            }]);
        }

        Ok(Vec::new())
    }
}

fn decode_u256_word(data: &[u8]) -> Result<U256> {
    if data.len() < 32 {
        return Err(crate::domain::ChainwatchError::Decode(format!(
            "ERC20 event data must contain at least 32 bytes, got {}",
            data.len()
        )));
    }
    Ok(U256::from_be_slice(&data[..32]))
}

#[cfg(test)]
mod tests {
    use alloy::primitives::{address, b256};

    use super::*;
    use crate::domain::{BlockNumber, ChainId, codec::address_to_topic};

    #[test]
    fn decodes_erc20_transfer() -> Result<()> {
        let from = address!("0000000000000000000000000000000000000001");
        let to = address!("0000000000000000000000000000000000000002");
        let mut data = vec![0_u8; 32];
        data[31] = 123;
        let log = RawLog {
            chain_id: ChainId(1),
            block_number: BlockNumber(1),
            tx_hash: b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            log_index: 0,
            address: address!("0000000000000000000000000000000000001000"),
            topics: vec![
                Transfer::SIGNATURE_HASH,
                address_to_topic(from),
                address_to_topic(to),
            ],
            data,
            removed: false,
            timestamp: 1_700_000_000,
        };
        let events = Erc20EventDecoder::new().decode(&log)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, DecodedEventKind::Erc20Transfer);
        assert_eq!(events[0].payload["amount_wei"], "123");
        Ok(())
    }
}
