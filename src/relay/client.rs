use std::time::Duration;
use tokio::time::timeout;
use alloy_primitives::U64;
use reqwest::Client;

use crate::errors::{BoostMonitorError, Result};
use crate::types::BidTrace;
use crate::config::RelayConfig;
use super::RelayService;

pub struct RelayClient {
    base_url: String,
    url: String,
    client: Client,
    request_timeout: Duration,
    failed_requests: u32,
    circuit_breaker_threshold: u32,
}

impl RelayClient {
    pub fn new(config: RelayConfig) -> Self {
        let base_url = config.url.clone();
        Self {
            base_url,
            url: format!("{}/relay/v1/data/bidtraces/builder_blocks_received", config.url),
            client: Client::new(),
            request_timeout: config.request_timeout,
            failed_requests: 0,
            circuit_breaker_threshold: config.circuit_breaker_threshold,
        }
    }

    pub fn new_with_url(base_url: String, request_timeout: Duration) -> Self {
        let full_url = format!("{}/relay/v1/data/bidtraces/builder_blocks_received", base_url);
        Self {
            base_url,
            url: full_url,
            client: Client::new(),
            request_timeout,
            failed_requests: 0,
            circuit_breaker_threshold: 3,
        }
    }

    pub fn is_circuit_open(&self) -> bool {
        self.failed_requests >= self.circuit_breaker_threshold
    }

    #[allow(dead_code)]
    fn reset_circuit(&mut self) {
        self.failed_requests = 0;
    }
}

#[async_trait::async_trait]
impl RelayService for RelayClient {
    fn get_url(&self) -> &str {
        &self.base_url
    }

    async fn get_builder_bids(&self, block_num: U64) -> Result<Vec<BidTrace>> {
        if self.is_circuit_open() {
            return Err(BoostMonitorError::RelayConnectionError(
                "Circuit breaker open, skipping request".to_string()
            ));
        }

        let request_url = format!("{}?block_number={}", &self.url, block_num);

        let response = match timeout(
            self.request_timeout,
            self.client
                .get(&request_url)
                .header("accept", "application/json")
                .send()
        ).await {
            Ok(response_result) => {
                match response_result {
                    Ok(response) => response,
                    Err(e) => {
                        return Err(BoostMonitorError::RequestError(e));
                    }
                }
            },
            Err(_) => {
                return Err(BoostMonitorError::TimeoutError(self.request_timeout));
            }
        };

        if !response.status().is_success() {
            return Err(BoostMonitorError::RelayConnectionError(
                format!("Relay returned status: {}", response.status())
            ));
        }

        let bid_traces = match response.json::<Vec<BidTrace>>().await {
            Ok(data) => data,
            Err(e) => {
                return Err(BoostMonitorError::InvalidResponseError(
                    format!("Failed to parse JSON response: {}", e)
                ));
            }
        };

        Ok(bid_traces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RelayConfig;
    use std::time::Duration;

    #[tokio::test]
    async fn test_circuit_breaker() {
        let config = RelayConfig {
            url: "https://example.com".to_string(),
            request_timeout: Duration::from_millis(100),
            circuit_breaker_threshold: 2,
        };

        let client = RelayClient::new(config);

        assert!(!client.is_circuit_open());

        let mut client = client;
        client.failed_requests = 2;
        assert!(client.is_circuit_open());

        client.reset_circuit();
        assert!(!client.is_circuit_open());
    }
}
