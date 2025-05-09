use alloy_primitives::{Address, U256};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json;
use std::io::{self, BufRead};
use std::result::Result;
use std::str::FromStr;

mod address_serde {
    use super::*;
    use alloy_primitives::Address;
    use serde::Serializer;

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

fn serialize_u256_as_decimal_string<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BidTrace {
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    slot: U256,
    parent_hash: String,
    block_hash: String,
    builder_pubkey: String,
    proposer_pubkey: String,
    #[serde(with = "address_serde")]
    proposer_fee_recipient: Address,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    gas_limit: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    gas_used: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    value: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    block_number: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    num_tx: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    timestamp: U256,
    #[serde(
        deserialize_with = "deserialize_u256_from_string",
        serialize_with = "serialize_u256_as_decimal_string"
    )]
    timestamp_ms: U256,
}

fn deserialize_u256_from_string<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    use alloy_primitives::U256;
    use std::str::FromStr;

    let s = String::deserialize(deserializer)?;
    U256::from_str(&s).map_err(serde::de::Error::custom)
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<BidTrace>(&line) {
            Ok(bid_trace) => match serde_json::to_string(&bid_trace) {
                Ok(json_output) => {
                    println!("{}", json_output);
                }
                Err(e) => {
                    eprintln!("Error serializing BidTrace: {}", e);
                }
            },
            Err(e) => {
                eprintln!("Error parsing line as BidTrace: {}", e);
                eprintln!("Problematic line: {}", line);
            }
        }
    }

    Ok(())
}
