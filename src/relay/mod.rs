use crate::types::BidTrace;
use crate::errors::Result;
use alloy_primitives::U64;
use async_trait::async_trait;

#[async_trait]
pub trait RelayService: Send + Sync {
    async fn get_builder_bids(&self, block_num: U64) -> Result<Vec<BidTrace>>;
    fn get_url(&self) -> &str;
}

pub mod client;
