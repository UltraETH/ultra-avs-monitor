use crate::types::BidTrace;
use crate::errors::Result;
use alloy_primitives::U64;
use async_trait::async_trait;

#[async_trait]
pub trait RelayService: Send + Sync {
    /// Fetches builder bids for the specified block number
    async fn get_builder_bids(&self, block_num: U64) -> Result<Vec<BidTrace>>;
    /// Returns the URL of the relay service
    fn get_url(&self) -> &str;
}

pub mod client;
