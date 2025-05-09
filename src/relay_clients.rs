use std::{sync::Arc, time::Duration};

use alloy_primitives::U64;
use tokio::{select, sync::Mutex, time};
use tracing::{debug, error, instrument};

use crate::{
    bid_manager::BidManager, config::RelayConfig, errors::Result, relay::client::RelayClient,
    relay::RelayService,
};

pub struct RelayClients {
    pub clients: Vec<Arc<Mutex<dyn RelayService + Send + Sync>>>,
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
                    Arc::new(Mutex::new(RelayClient::new(config)))
                        as Arc<Mutex<dyn RelayService + Send + Sync>>
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
                    Arc::new(Mutex::new(RelayClient::new(config)))
                        as Arc<Mutex<dyn RelayService + Send + Sync>>
                })
                .collect(),
            bid_manager: Arc::new(BidManager::new()),
        }
    }

    #[instrument(skip(self), fields(block = %block_num, interval = ?poll_interval_secs, duration = ?poll_for_secs))]
    pub async fn poll_for(
        &mut self,
        block_num: U64,
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
                        debug!(block = %block_num, "Polling duration exceeded, clearing bids");
                        self.bid_manager.clear_all().await?;
                        break;
                    }

                    debug!(block = %block_num, "Polling relays for bids");
                    let mut handles = Vec::new();
                    for client_mutex in &self.clients {
                        let client_mutex = client_mutex.clone();
                        let bid_manager = self.bid_manager.clone();
                        let block_num = block_num;

                        let handle = tokio::spawn(async move {
                            let mut client = client_mutex.lock().await; // Acquire mutex lock

                            // Check circuit breaker state before polling
                            if client.is_circuit_open() {
                                debug!(client_url = %client.get_url(), block = %block_num, "Circuit breaker open, skipping poll");
                                return; // Skip this client
                            }

                            match client.get_builder_bids(block_num).await {
                                Ok(bid_traces) => {
                                    bid_manager.add_bids(bid_traces).await;
                                }
                                Err(e) => {
                                    // Error handling is now largely within RelayClient::get_builder_bids
                                    // which records failures and trips the circuit breaker.
                                    // We can log here if needed, but the primary error handling
                                    // and circuit breaking is delegated to the client itself.
                                    error!(client_url = %client.get_url(), block = %block_num, error = %e, "Error fetching bids from relay");
                                }
                            }
                        });

                        handles.push(handle);
                    }

                    for handle in handles {
                        if let Err(e) = handle.await {
                            error!(error = ?e, "Error joining relay client task");
                        }
                    }
                } // Closes `_ = interval_timer.tick() => {`
            } // Closes `select!`
        } // Closes `loop`
        Ok(())
    } // Closes `poll_for`

    // Collects performance metrics from all relay clients
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
