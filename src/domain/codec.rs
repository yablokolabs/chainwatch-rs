use std::str::FromStr;

use alloy::primitives::{Address, B256, U256};

use super::{ChainwatchError, Result};

pub fn address_to_hex(address: &Address) -> String {
    address.to_string().to_lowercase()
}

pub fn hash_to_hex(hash: &B256) -> String {
    hash.to_string().to_lowercase()
}

pub fn u256_to_decimal(value: &U256) -> String {
    value.to_string()
}

pub fn parse_address(value: &str) -> Result<Address> {
    Address::from_str(value)
        .map_err(|err| ChainwatchError::Validation(format!("invalid address `{value}`: {err}")))
}

pub fn parse_hash(value: &str) -> Result<B256> {
    B256::from_str(value)
        .map_err(|err| ChainwatchError::Validation(format!("invalid hash `{value}`: {err}")))
}

pub fn parse_u256(value: &str) -> Result<U256> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ChainwatchError::Validation("empty U256 value".to_owned()));
    }

    if let Some(hex) = trimmed.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).map_err(|err| {
            ChainwatchError::Validation(format!("invalid hex U256 `{value}`: {err}"))
        })
    } else {
        U256::from_str_radix(trimmed, 10).map_err(|err| {
            ChainwatchError::Validation(format!("invalid decimal U256 `{value}`: {err}"))
        })
    }
}

pub fn parse_hex_bytes(value: &str) -> Result<Vec<u8>> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    hex::decode(hex).map_err(|err| ChainwatchError::Validation(format!("invalid hex bytes: {err}")))
}

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub fn topic_to_address(topic: &B256) -> Result<Address> {
    let bytes = topic.as_slice();
    if bytes[..12].iter().any(|byte| *byte != 0) {
        return Err(ChainwatchError::Decode(
            "indexed address topic has non-zero high bytes".to_owned(),
        ));
    }
    Ok(Address::from_slice(&bytes[12..]))
}

pub fn address_to_topic(address: Address) -> B256 {
    let mut topic = [0_u8; 32];
    topic[12..].copy_from_slice(address.as_slice());
    B256::from(topic)
}

pub fn hex_u64(value: &serde_json::Value, field: &str) -> Result<u64> {
    let raw = value
        .as_str()
        .ok_or_else(|| ChainwatchError::Rpc(format!("field `{field}` is not a hex string")))?;
    let hex = raw.strip_prefix("0x").unwrap_or(raw);
    u64::from_str_radix(hex, 16)
        .map_err(|err| ChainwatchError::Rpc(format!("invalid `{field}` hex `{raw}`: {err}")))
}

pub fn optional_hex_u64(value: Option<&serde_json::Value>, field: &str) -> Result<Option<u64>> {
    match value {
        Some(v) if !v.is_null() => hex_u64(v, field).map(Some),
        _ => Ok(None),
    }
}

pub fn checked_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|err| {
        ChainwatchError::Validation(format!("{field} value {value} exceeds i64 range: {err}"))
    })
}

pub fn checked_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|err| {
        ChainwatchError::Validation(format!(
            "{field} value {value} is negative or invalid: {err}"
        ))
    })
}
