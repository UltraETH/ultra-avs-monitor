use std::{sync::Arc, time::Duration};

use alloy_primitives::U64;
use tokio::{select, time};
use tracing::{error, debug, instrument}; // Import tracing macros

use crate::{
    bid_manager::BidManager,
    config::RelayConfig,
    relay::RelayService,
    relay::client::RelayClient,
    errors::Result, // Import Result for error handling
};

pub struct RelayClients {
    // All relay clients to read block builder bids from.
    pub clients: Vec<Arc<dyn RelayService + Send + Sync>>,
    // Bid manager to merge and sort bids.
    pub bid_manager: Arc<BidManager>,
}

impl RelayClients {
    pub fn new(relay_urls: Vec<String>) -> Self {
        Self {
            clients: relay_urls
                .into_iter()
                .map(|url| {
                    let config = RelayConfig {
                        url,
                        request_timeout: Duration::from_secs(5),
                        circuit_breaker_threshold: 3,
                    };
                    Arc::new(RelayClient::new(config)) as Arc<dyn RelayService + Send + Sync>
                })
                .collect(),
            bid_manager: Arc::new(BidManager::new()),
        }
    }

    pub fn with_configs(configs: Vec<RelayConfig>) -> Self {
        Self {
            clients: configs
                .into_iter()
                .map(|config| {
                    Arc::new(RelayClient::new(config)) as Arc<dyn RelayService + Send + Sync>
                })
                .collect(),
            bid_manager: Arc::new(BidManager::new()),
        }
    }

    // Polls for builder bids every `poll_interval_secs` second for `poll_for_secs` seconds.
    #[instrument(skip(self), fields(block = %block_num, interval = ?poll_interval_secs, duration = ?poll_for_secs))]
    pub async fn poll_for(&mut self, block_num: U64, poll_interval_secs: u64, poll_for_secs: u64) -> Result<()> { // Return Result
        let poll_interval = Duration::from_secs(poll_interval_secs);
        let mut interval_timer = time::interval(poll_interval);
        let start_time = time::Instant::now();
        let duration = Duration::from_secs(poll_for_secs);

        loop {
            select! {
                _ = interval_timer.tick() => {
                    // Check if the total polling duration has been exceeded
                    if time::Instant::now().duration_since(start_time) >= duration { // Use >= for clarity
                        debug!(block = %block_num, "Polling duration exceeded, clearing bids");
                        self.bid_manager.clear_all().await?; // Use ? for error propagation
                        break;
                    }

                    debug!(block = %block_num, "Polling relays for bids");
                    let mut handles = Vec::new();
                    for client in &self.clients {
                        let client = client.clone();
                        let bid_manager = self.bid_manager.clone();
                        let block_num = block_num;

                        let handle = tokio::spawn(async move {
                            match client.get_builder_bids(block_num).await {
                                Ok(bid_traces) => {
                                    bid_manager.add_bids(bid_traces).await;
                                }
                                Err(e) => {
                                    // Use error! macro for logging errors
                                    error!(client_url = %client.get_url(), block = %block_num, error = %e, "Error fetching bids from relay");
                                }
                            }
                        });

                        handles.push(handle);
                    }

                    // Await all handles to ensure all bid traces are processed before the next interval
                    for handle in handles {
                        if let Err(e) = handle.await {
                            // Log potential join errors (e.g., panic in spawned task)
                            error!(error = ?e, "Error joining relay client task");
                        }
                    }
                }
                // This sleep branch seems redundant now with the check inside the tick branch.
                // If the intention was a hard timeout, the outer check handles it.
                // Let's remove this branch to simplify.
                // _ = time::sleep(duration) => {
                //     debug!(block = %block_num, "Polling duration reached via sleep, clearing bids");
                //     self.bid_manager.clear_all().await?;
                //     break;
                // }
            }
        }
        Ok(()) // Return Ok(()) at the end
    }
}
