use crate::types::BidTrace;
use crate::errors::Result;
use alloy_primitives::U64;
use async_trait::async_trait;

#[async_trait]
pub trait RelayService: Send + Sync {
    async fn get_builder_bids(&mut self, block_num: U64) -> Result<Vec<BidTrace>>;
    fn get_url(&self) -> &str;
    fn get_success_count(&self) -> u32; // Added for metrics
    fn get_failed_requests(&self) -> u32; // Added for metrics
    fn is_circuit_open(&self) -> bool; // Added for circuit breaker check
}

pub mod client;
