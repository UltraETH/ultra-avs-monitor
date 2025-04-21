use std::sync::Arc;
use crate::bid_manager::BidManager;
use crate::errors::Result;

#[cfg(test)]
use alloy_primitives::{Address, U256};
#[cfg(test)]
use crate::types::BidTrace;

pub struct TestServer {
    pub bid_manager: Arc<BidManager>
}

impl TestServer {
    pub async fn new() -> Self {
        Self {
            bid_manager: Arc::new(BidManager::new())
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        // Clean up resources
        let _ = self.bid_manager.clear_all().await;
        Ok(())
    }

    pub fn get_bid_manager(&self) -> Arc<BidManager> {
        self.bid_manager.clone()
    }
}

/// Helper function to create a test BidTrace with specified values
#[cfg(test)]
pub fn create_test_bid_trace(slot: u64, value: u64) -> BidTrace {
    BidTrace {
        slot: U256::from(slot),
        parent_hash: format!("parent_hash_{}", slot),
        block_hash: format!("block_hash_{}", slot),
        builder_pubkey: format!("builder_pubkey_{}", slot),
        proposer_pubkey: format!("proposer_pubkey_{}", slot),
        proposer_fee_recipient: Address::ZERO,
        gas_limit: U256::from(30000000u64),
        gas_used: U256::from(10000000u64),
        value: U256::from(value),
        block_number: U256::from(slot),
        num_tx: U256::from(100u64),
        timestamp: U256::from(1617979455u64),
        timestamp_ms: U256::from(1617979455000u64),
    }
}
