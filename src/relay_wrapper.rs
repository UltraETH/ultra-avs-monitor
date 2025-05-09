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
    // Constructor now accepts Arc<Mutex<RelayClients>>
    pub fn new(relay_clients: Arc<Mutex<RelayClients>>) -> Self {
        Self {
            inner: relay_clients,
        }
    }

    // Returns a clone of the Arc<Mutex<RelayClients>> for sharing
    pub fn get_inner_arc(&self) -> Arc<Mutex<RelayClients>> {
        self.inner.clone()
    }

    // Note: get_inner and get_inner_mut are removed as direct mutable/immutable
    // references to the inner RelayClients are no longer possible without locking.
    // Use get_inner_arc() and lock the mutex instead.

    // Sets the bid manager on the inner RelayClients, acquiring the lock
    pub async fn set_bid_manager(&self, bid_manager: Arc<BidManager>) {
        let mut inner = self.inner.lock().await;
        inner.bid_manager = bid_manager;
    }

    // Polls for bids using the inner RelayClients, acquiring the lock
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

// The From implementation is removed as the constructor signature changed
// impl From<RelayClients> for RelayClientWrapper {
//     fn from(relay_clients: RelayClients) -> Self {
//         Self::new(relay_clients)
//     }
// }
