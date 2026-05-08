use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::domain::{
    Alert, AlertSeverity, Result, TokenTransfer, WatchlistEntry, codec::address_to_hex,
    codec::u256_to_decimal,
};

#[derive(Clone, Debug)]
pub struct RiskEngineConfig {
    pub large_transfer_threshold_wei: U256,
    pub high_frequency_threshold: u64,
    pub suspicious_contract_rule_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct RiskEngine {
    config: RiskEngineConfig,
}

pub struct RiskContext<'a> {
    pub watched: &'a HashMap<Address, WatchlistEntry>,
    pub from_transfer_count: u64,
    pub to_transfer_count: u64,
}

impl RiskEngine {
    #[must_use]
    pub const fn new(config: RiskEngineConfig) -> Self {
        Self { config }
    }

    pub fn evaluate_transfer(
        &self,
        transfer: &TokenTransfer,
        context: &RiskContext<'_>,
    ) -> Result<Vec<Alert>> {
        let mut alerts = Vec::new();

        if transfer.amount >= self.config.large_transfer_threshold_wei {
            alerts.push(Alert {
                id: Uuid::new_v4(),
                chain_id: transfer.chain_id,
                rule: "large_transfer_threshold".to_owned(),
                severity: AlertSeverity::High,
                address: Some(transfer.from),
                tx_hash: Some(transfer.tx_hash),
                block_number: Some(transfer.block_number),
                message: format!(
                    "Large ERC20 transfer of {} wei from {} to {}",
                    u256_to_decimal(&transfer.amount),
                    address_to_hex(&transfer.from),
                    address_to_hex(&transfer.to)
                ),
                metadata: json!({
                    "token": address_to_hex(&transfer.token_address),
                    "from": address_to_hex(&transfer.from),
                    "to": address_to_hex(&transfer.to),
                    "amount_wei": u256_to_decimal(&transfer.amount),
                    "threshold_wei": u256_to_decimal(&self.config.large_transfer_threshold_wei)
                }),
                created_at: Utc::now(),
            });
        }

        for (side, address) in [("from", transfer.from), ("to", transfer.to)] {
            if let Some(entry) = context.watched.get(&address) {
                alerts.push(Alert {
                    id: Uuid::new_v4(),
                    chain_id: transfer.chain_id,
                    rule: "watched_wallet_activity".to_owned(),
                    severity: AlertSeverity::Critical,
                    address: Some(address),
                    tx_hash: Some(transfer.tx_hash),
                    block_number: Some(transfer.block_number),
                    message: format!(
                        "Watched wallet {} appeared as {side} in ERC20 transfer",
                        address_to_hex(&address)
                    ),
                    metadata: json!({
                        "side": side,
                        "label": entry.label,
                        "token": address_to_hex(&transfer.token_address),
                        "amount_wei": u256_to_decimal(&transfer.amount)
                    }),
                    created_at: Utc::now(),
                });
            }
        }

        let max_frequency = context.from_transfer_count.max(context.to_transfer_count);
        if max_frequency >= self.config.high_frequency_threshold {
            alerts.push(Alert {
                id: Uuid::new_v4(),
                chain_id: transfer.chain_id,
                rule: "high_frequency_transfers".to_owned(),
                severity: AlertSeverity::Medium,
                address: Some(
                    if context.from_transfer_count >= context.to_transfer_count {
                        transfer.from
                    } else {
                        transfer.to
                    },
                ),
                tx_hash: Some(transfer.tx_hash),
                block_number: Some(transfer.block_number),
                message: format!(
                    "High-frequency ERC20 activity detected: {max_frequency} transfers in window"
                ),
                metadata: json!({
                    "from_count": context.from_transfer_count,
                    "to_count": context.to_transfer_count,
                    "threshold": self.config.high_frequency_threshold,
                    "token": address_to_hex(&transfer.token_address)
                }),
                created_at: Utc::now(),
            });
        }

        if self.config.suspicious_contract_rule_enabled && is_zero_address(transfer.from) {
            alerts.push(Alert {
                id: Uuid::new_v4(),
                chain_id: transfer.chain_id,
                rule: "suspicious_contract_interaction".to_owned(),
                severity: AlertSeverity::Low,
                address: Some(transfer.token_address),
                tx_hash: Some(transfer.tx_hash),
                block_number: Some(transfer.block_number),
                message: "Token mint-like transfer observed; route to contract-risk model"
                    .to_owned(),
                metadata: json!({
                    "reason": "erc20_from_zero_address",
                    "future_model_hook": "contract_risk_score_v1",
                    "token": address_to_hex(&transfer.token_address)
                }),
                created_at: Utc::now(),
            });
        }

        Ok(alerts)
    }
}

fn is_zero_address(address: Address) -> bool {
    address.as_slice().iter().all(|byte| *byte == 0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy::primitives::{U256, address, b256};
    use chrono::Utc;

    use super::*;
    use crate::domain::{BlockNumber, ChainId, WatchlistEntry};

    fn sample_transfer(amount: U256) -> TokenTransfer {
        TokenTransfer {
            chain_id: ChainId(1),
            token_address: address!("0000000000000000000000000000000000001000"),
            from: address!("0000000000000000000000000000000000000001"),
            to: address!("0000000000000000000000000000000000000002"),
            amount,
            tx_hash: b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            block_number: BlockNumber(42),
            log_index: 0,
            timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn detects_large_transfer() -> Result<()> {
        let engine = RiskEngine::new(RiskEngineConfig {
            large_transfer_threshold_wei: U256::from_limbs([100, 0, 0, 0]),
            high_frequency_threshold: 10,
            suspicious_contract_rule_enabled: false,
        });
        let transfer = sample_transfer(U256::from_limbs([101, 0, 0, 0]));
        let watched = HashMap::new();
        let alerts = engine.evaluate_transfer(
            &transfer,
            &RiskContext {
                watched: &watched,
                from_transfer_count: 0,
                to_transfer_count: 0,
            },
        )?;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].rule, "large_transfer_threshold");
        Ok(())
    }

    #[test]
    fn detects_watchlist_activity() -> Result<()> {
        let engine = RiskEngine::new(RiskEngineConfig {
            large_transfer_threshold_wei: U256::from_limbs([10_000, 0, 0, 0]),
            high_frequency_threshold: 10,
            suspicious_contract_rule_enabled: false,
        });
        let transfer = sample_transfer(U256::from_limbs([1, 0, 0, 0]));
        let mut watched = HashMap::new();
        watched.insert(
            transfer.to,
            WatchlistEntry {
                chain_id: transfer.chain_id,
                address: transfer.to,
                label: Some("case-123".to_owned()),
                created_at: Utc::now(),
            },
        );
        let alerts = engine.evaluate_transfer(
            &transfer,
            &RiskContext {
                watched: &watched,
                from_transfer_count: 0,
                to_transfer_count: 0,
            },
        )?;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].rule, "watched_wallet_activity");
        Ok(())
    }
}
