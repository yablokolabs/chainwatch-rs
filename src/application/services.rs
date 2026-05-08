use std::{collections::HashMap, sync::Arc};

use alloy::primitives::Address;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::{
    application::ports::Repository,
    domain::{Alert, AlertSeverity, ChainId, Result, TokenTransfer, WatchlistEntry},
    risk::{RiskContext, RiskEngine},
};

pub struct AlertingService {
    repository: Arc<dyn Repository>,
    risk_engine: RiskEngine,
    frequency_window_seconds: u64,
}

impl AlertingService {
    #[must_use]
    pub fn new(
        repository: Arc<dyn Repository>,
        risk_engine: RiskEngine,
        frequency_window_seconds: u64,
    ) -> Self {
        Self {
            repository,
            risk_engine,
            frequency_window_seconds,
        }
    }

    pub async fn evaluate_transfers(&self, transfers: &[TokenTransfer]) -> Result<Vec<Alert>> {
        if transfers.is_empty() {
            return Ok(Vec::new());
        }

        let chain_id = transfers[0].chain_id;
        let watchlist = self.repository.list_watchlist(chain_id).await?;
        let watched = watchlist
            .into_iter()
            .map(|entry| (entry.address, entry))
            .collect::<HashMap<_, _>>();

        let mut alerts = Vec::new();
        for transfer in transfers {
            let since = transfer
                .timestamp
                .saturating_sub(self.frequency_window_seconds);
            let from_count = self
                .repository
                .count_transfers_by_address_since(chain_id, transfer.from, since)
                .await?;
            let to_count = self
                .repository
                .count_transfers_by_address_since(chain_id, transfer.to, since)
                .await?;
            let context = RiskContext {
                watched: &watched,
                from_transfer_count: from_count,
                to_transfer_count: to_count,
            };
            alerts.extend(self.risk_engine.evaluate_transfer(transfer, &context)?);
        }
        Ok(alerts)
    }
}

#[must_use]
pub fn build_watchlist_entry(
    chain_id: ChainId,
    address: Address,
    label: Option<String>,
) -> WatchlistEntry {
    WatchlistEntry {
        chain_id,
        address,
        label,
        created_at: Utc::now(),
    }
}

#[must_use]
pub fn operational_alert(chain_id: ChainId, rule: &str, message: String) -> Alert {
    Alert {
        id: Uuid::new_v4(),
        chain_id,
        rule: rule.to_owned(),
        severity: AlertSeverity::Medium,
        address: None,
        tx_hash: None,
        block_number: None,
        message,
        metadata: json!({ "source": "indexer" }),
        created_at: Utc::now(),
    }
}
