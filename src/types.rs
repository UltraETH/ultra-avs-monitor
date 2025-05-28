use std::{
    fmt,
    hash::{Hash, Hasher},
};

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize}; // Removed Serializer

use crate::utils::serde_helpers::deserialize_u256_from_string;

mod address_serde {
    use super::*;
    use serde::{Deserializer, Serializer}; // Removed Serialize from here as it's not used locally
    use std::str::FromStr;

    pub fn serialize<S>(address: &Address, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = format!("{:?}", address);
        serializer.serialize_str(&hex_str)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Address, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Address::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct BidTrace {
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub slot: U256,
    pub parent_hash: String,
    pub block_hash: String,
    pub builder_pubkey: String,
    pub proposer_pubkey: String,
    #[serde(with = "address_serde")]
    pub proposer_fee_recipient: Address,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub gas_limit: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub gas_used: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub value: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub block_number: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub num_tx: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub timestamp: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub timestamp_ms: U256,
}

impl fmt::Display for BidTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BidTrace {{ block_number: {}, builder_pubkey: {}, value: {} , num_tx: {}}}",
            self.block_number, self.builder_pubkey, self.value, self.num_tx
        )
    }
}

impl Hash for BidTrace {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        self.builder_pubkey.hash(state);
    }
}

impl Ord for BidTrace {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl PartialOrd for BidTrace {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct DeliveredPayloadTrace {
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub slot: U256,
    pub parent_hash: String,
    pub block_hash: String,
    pub builder_pubkey: String,
    pub proposer_pubkey: String,
    #[serde(with = "address_serde")]
    pub proposer_fee_recipient: Address,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub value: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub block_number: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub num_tx: U256,
    #[serde(deserialize_with = "deserialize_u256_from_string")]
    pub timestamp: U256, // Assuming relays provide this, might be part of execution payload header
                         // timestamp_ms might not be present in delivered payloads, check relay specs
                         // pub timestamp_ms: U256,
}

impl fmt::Display for DeliveredPayloadTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DeliveredPayloadTrace {{ block_number: {}, builder_pubkey: {}, value: {} }}",
            self.block_number, self.builder_pubkey, self.value
        )
    }
}

// Delivered payloads are typically unique by block_hash, so Hash/Ord might not be needed
// in the same way as BidTrace unless we store multiple sources or versions.
// For now, let's assume block_hash is the primary identifier.
