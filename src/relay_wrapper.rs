use crate::bid_manager::BidManager;
use crate::errors::Result;
use crate::relay_clients::RelayClients;
use alloy_primitives::U64;
use std::sync::Arc;
use tokio::sync::Mutex; // Import Mutex

pub struct RelayClientWrapper {
    inner: Arc<Mutex<RelayClients>>, // Wrapped in Arc<Mutex>
}

impl RelayClientWrapper {
    pub fn new(relay_clients: Arc<Mutex<RelayClients>>) -> Self {
        Self {
            inner: relay_clients,
        }
    }

    pub fn get_inner_arc(&self) -> Arc<Mutex<RelayClients>> {
        self.inner.clone()
    }

    pub async fn set_bid_manager(&self, bid_manager: Arc<BidManager>) {
        let mut inner = self.inner.lock().await;
        inner.bid_manager = bid_manager;
    }

    pub async fn poll_for(
        &self,
        block_num: U64,
        interval_secs: u64,
        duration_secs: u64,
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner
            .poll_for(block_num, interval_secs, duration_secs)
            .await
    }
}
