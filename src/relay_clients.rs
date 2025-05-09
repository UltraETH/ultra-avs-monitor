use std::{sync::Arc, time::Duration};

use alloy_primitives::U64;
use tokio::{select, time};
use tracing::{error, debug, instrument};

use crate::{
    bid_manager::BidManager,
    config::RelayConfig,
    relay::RelayService,
    relay::client::RelayClient,
    errors::Result,
};

pub struct RelayClients {
    pub clients: Vec<Arc<dyn RelayService + Send + Sync>>,
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

    #[instrument(skip(self), fields(block = %block_num, interval = ?poll_interval_secs, duration = ?poll_for_secs))]
    pub async fn poll_for(&mut self, block_num: U64, poll_interval_secs: u64, poll_for_secs: u64) -> Result<()> {
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
                }
            }
        }
        Ok(())
    }
}
