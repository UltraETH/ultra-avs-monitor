use alloy_primitives::U256;
use serde::{Deserializer, Deserialize};
use std::str::FromStr;

pub fn deserialize_u256_from_string<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    U256::from_str(&s).map_err(serde::de::Error::custom)
}
