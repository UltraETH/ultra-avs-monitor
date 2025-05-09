use std::sync::Arc;
use alloy_primitives::U64;
use crate::relay_clients::RelayClients;
use crate::errors::Result;
use crate::bid_manager::BidManager;

pub struct RelayClientWrapper {
    inner: RelayClients,
}

impl RelayClientWrapper {
    pub fn new(relay_clients: RelayClients) -> Self {
        Self { inner: relay_clients }
    }

    pub fn get_inner(&self) -> &RelayClients {
        &self.inner
    }

    pub fn get_inner_mut(&mut self) -> &mut RelayClients {
        &mut self.inner
    }

    pub fn set_bid_manager(&mut self, bid_manager: Arc<BidManager>) {
        self.inner.bid_manager = bid_manager;
    }

    pub async fn poll_for(&mut self, block_num: U64, interval_secs: u64, duration_secs: u64) -> Result<()> {
        self.inner.poll_for(block_num, interval_secs, duration_secs).await
    }
}

impl From<RelayClients> for RelayClientWrapper {
    fn from(relay_clients: RelayClients) -> Self {
        Self::new(relay_clients)
    }
}
