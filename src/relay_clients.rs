use std::{sync::Arc, time::Duration};

use alloy_primitives::U64;
use tokio::{select, sync::Mutex, time};
use tracing::{debug, error, instrument};

use crate::{
    bid_manager::BidManager,
    config::RelayConfig,
    delivered_payload_manager::DeliveredPayloadManager, // Added
    errors::Result,
    relay::client::RelayClient,
    relay::RelayService,
};

pub struct RelayClients {
    pub clients: Vec<Arc<Mutex<dyn RelayService + Send + Sync>>>,
    pub bid_manager: Arc<BidManager>,
    pub delivered_payload_manager: Arc<DeliveredPayloadManager>, // Added
}

impl RelayClients {
    // Constructor needs to be updated to accept DeliveredPayloadManager
    pub fn new(
        relay_urls: Vec<String>,
        bid_manager: Arc<BidManager>,
        delivered_payload_manager: Arc<DeliveredPayloadManager>,
    ) -> Self {
        Self {
            clients: relay_urls
                .into_iter()
                .map(|url| {
                    let config = RelayConfig {
                        url,
                        request_timeout: Duration::from_secs(5),
                        circuit_breaker_threshold: 3,
                    };
                    Arc::new(Mutex::new(RelayClient::new(config)))
                        as Arc<Mutex<dyn RelayService + Send + Sync>>
                })
                .collect(),
            bid_manager,
            delivered_payload_manager,
        }
    }

    // Constructor needs to be updated
    pub fn with_configs(
        configs: Vec<RelayConfig>,
        bid_manager: Arc<BidManager>,
        delivered_payload_manager: Arc<DeliveredPayloadManager>,
    ) -> Self {
        Self {
            clients: configs
                .into_iter()
                .map(|config| {
                    Arc::new(Mutex::new(RelayClient::new(config)))
                        as Arc<Mutex<dyn RelayService + Send + Sync>>
                })
                .collect(),
            bid_manager,
            delivered_payload_manager,
        }
    }

    #[instrument(skip(self), fields(slot_or_block_num = %slot_or_block_num, interval = ?poll_interval_secs, duration = ?poll_for_secs))]
    pub async fn poll_for(
        &mut self,
        slot_or_block_num: U64, // This can be used as slot for delivered payloads and block_num for bids
        poll_interval_secs: u64,
        poll_for_secs: u64,
    ) -> Result<()> {
        let poll_interval = Duration::from_secs(poll_interval_secs);
        let mut interval_timer = time::interval(poll_interval);
        let start_time = time::Instant::now();
        let duration = Duration::from_secs(poll_for_secs);

        loop {
            select! {
                _ = interval_timer.tick() => {
                    if time::Instant::now().duration_since(start_time) >= duration {
                        debug!(slot_or_block = %slot_or_block_num, "Polling duration exceeded");
                        break;
                    }

                    debug!(slot_or_block = %slot_or_block_num, "Polling relays for bids and delivered payloads");
                    let mut handles = Vec::new();

                    for client_mutex in &self.clients {
                        let client_arc = client_mutex.clone(); // Clone Arc for the new task
                        let bid_manager_arc = self.bid_manager.clone();
                        let delivered_payload_manager_arc = self.delivered_payload_manager.clone();
                        let current_slot_or_block = slot_or_block_num;

                        let handle = tokio::spawn(async move {
                            let mut client_guard = client_arc.lock().await;

                            // Fetch Builder Bids
                            match client_guard.get_builder_bids(current_slot_or_block).await {
                                Ok(bid_traces) => {
                                    if !bid_traces.is_empty() {
                                        bid_manager_arc.add_bids(bid_traces).await;
                                    }
                                }
                                Err(e) => {
                                    error!(client_url = %client_guard.get_url(), slot_or_block = %current_slot_or_block, error = %e, "Error fetching builder bids from relay");
                                }
                            }

                            // Fetch Delivered Payloads
                            // Assuming slot_or_block_num can be used as slot here.
                            // If relays require a different parameter or logic, this needs adjustment.
                            match client_guard.get_delivered_payloads(current_slot_or_block).await {
                                Ok(payload_traces) => {
                                    if !payload_traces.is_empty() {
                                        delivered_payload_manager_arc.add_payloads(payload_traces).await;
                                    }
                                }
                                Err(e) => {
                                    error!(client_url = %client_guard.get_url(), slot = %current_slot_or_block, error = %e, "Error fetching delivered payloads from relay");
                                }
                            }
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        if let Err(e) = handle.await {
                            error!(error = ?e, "Error joining relay client polling task");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn get_client_metrics(&self) -> Vec<(String, u32, u32)> {
        let mut metrics = Vec::new();
        for client_mutex in &self.clients {
            let client = client_mutex.lock().await; // Acquire mutex lock
            let url = client.get_url().to_string();
            let success_count = client.get_success_count();
            let failed_requests = client.get_failed_requests();
            metrics.push((url, success_count, failed_requests));
        }
        metrics
    } // Closes `get_client_metrics`
} // Closes `impl RelayClients`
