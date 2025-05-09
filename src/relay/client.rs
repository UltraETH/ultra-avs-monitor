use alloy_primitives::U64;
use reqwest::Client;
use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, warn};

use super::RelayService;
use crate::config::RelayConfig;
use crate::errors::{BoostMonitorError, Result};
use crate::types::BidTrace;

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(500);
const CIRCUIT_BREAKER_COOL_DOWN: Duration = Duration::from_secs(30);

pub struct RelayClient {
    base_url: String,
    url: String,
    client: Client,
    request_timeout: Duration,
    failed_requests: u32,
    success_count: u32, // Added for monitoring/future use
    circuit_breaker_threshold: u32,
    last_attempt_time: Instant, // Added for circuit breaker cool-down
}

impl RelayClient {
    pub fn new(config: RelayConfig) -> Self {
        let base_url = config.url.clone();
        Self {
            base_url,
            url: format!(
                "{}/relay/v1/data/bidtraces/builder_blocks_received",
                config.url
            ),
            client: Client::new(),
            request_timeout: config.request_timeout,
            failed_requests: 0,
            success_count: 0,
            circuit_breaker_threshold: config.circuit_breaker_threshold,
            last_attempt_time: Instant::now(),
        }
    }

    pub fn new_with_url(base_url: String, request_timeout: Duration) -> Self {
        let full_url = format!(
            "{}/relay/v1/data/bidtraces/builder_blocks_received",
            base_url
        );
        Self {
            base_url,
            url: full_url,
            client: Client::new(),
            request_timeout,
            failed_requests: 0,
            success_count: 0,
            circuit_breaker_threshold: 3, // Default threshold
            last_attempt_time: Instant::now(),
        }
    }

    // Checks if the circuit is open or in a half-open state
    pub fn is_circuit_open(&self) -> bool {
        if self.failed_requests >= self.circuit_breaker_threshold {
            // Circuit is open, check if it's time to attempt a half-open request
            self.last_attempt_time.elapsed() > CIRCUIT_BREAKER_COOL_DOWN
        } else {
            // Circuit is closed
            false
        }
    }

    // Resets the circuit breaker state
    fn reset_circuit(&mut self) {
        self.failed_requests = 0;
        self.success_count = 0; // Reset success count on circuit reset
        self.last_attempt_time = Instant::now();
        debug!(url = %self.base_url, "Circuit breaker reset");
    }

    // Increments failed requests and updates last attempt time
    fn record_failure(&mut self) {
        self.failed_requests += 1;
        self.last_attempt_time = Instant::now();
        warn!(url = %self.base_url, failed_requests = self.failed_requests, threshold = self.circuit_breaker_threshold, "Relay request failed");
    }

    // Resets failed requests and increments success count on success
    fn record_success(&mut self) {
        self.failed_requests = 0; // Reset failures on success
        self.success_count += 1;
        self.last_attempt_time = Instant::now();
        debug!(url = %self.base_url, "Relay request successful");
    }
}

#[async_trait::async_trait]
impl RelayService for RelayClient {
    fn get_url(&self) -> &str {
        &self.base_url
    }

    async fn get_builder_bids(&mut self, block_num: U64) -> Result<Vec<BidTrace>> {
        // Check circuit breaker state
        if self.failed_requests >= self.circuit_breaker_threshold
            && self.last_attempt_time.elapsed() <= CIRCUIT_BREAKER_COOL_DOWN
        {
            debug!(url = %self.base_url, "Circuit breaker open, skipping request");
            return Err(BoostMonitorError::RelayConnectionError(
                "Circuit breaker open, skipping request".to_string(),
            ));
        }

        let request_url = format!("{}?block_number={}", &self.url, block_num);

        for attempt in 1..=MAX_RETRIES {
            debug!(url = %self.base_url, block = %block_num, attempt = attempt, "Attempting to fetch bids");
            self.last_attempt_time = Instant::now(); // Update last attempt time before each try

            let response_result = timeout(
                self.request_timeout,
                self.client
                    .get(&request_url)
                    .header("accept", "application/json")
                    .send(),
            )
            .await;

            match response_result {
                Ok(Ok(response)) => {
                    if response.status().is_success() {
                        match response.json::<Vec<BidTrace>>().await {
                            Ok(data) => {
                                self.record_success();
                                return Ok(data);
                            }
                            Err(e) => {
                                error!(url = %self.base_url, block = %block_num, error = %e, "Failed to parse JSON response");
                                self.record_failure();
                                if attempt == MAX_RETRIES {
                                    return Err(BoostMonitorError::InvalidResponseError(format!(
                                        "Failed to parse JSON response after {} attempts: {}",
                                        MAX_RETRIES, e
                                    )));
                                }
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_else(|_| "N/A".to_string());
                        error!(url = %self.base_url, block = %block_num, status = %status, body = %body, "Relay returned non-success status");
                        self.record_failure();
                        if attempt == MAX_RETRIES {
                            return Err(BoostMonitorError::RelayConnectionError(format!(
                                "Relay returned status {} after {} attempts: {}",
                                status, MAX_RETRIES, body
                            )));
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!(url = %self.base_url, block = %block_num, error = %e, "Relay request failed");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::RequestError(e));
                    }
                }
                Err(_) => {
                    error!(url = %self.base_url, block = %block_num, timeout = ?self.request_timeout, "Relay request timed out");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::TimeoutError(self.request_timeout));
                    }
                }
            }

            // Wait before retrying
            if attempt < MAX_RETRIES {
                sleep(RETRY_DELAY).await;
            }
        }

        // Should not be reached if MAX_RETRIES > 0
        unreachable!();
    }

    fn get_success_count(&self) -> u32 {
        self.success_count
    }

    fn get_failed_requests(&self) -> u32 {
        self.failed_requests
    }

    // Implement the trait method by calling the existing struct method
    fn is_circuit_open(&self) -> bool {
        self.is_circuit_open()
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

        let mut client = RelayClient::new(config);

        assert!(!client.is_circuit_open());

        client.failed_requests = 1;
        assert!(!client.is_circuit_open()); // Not open yet

        client.failed_requests = 2;
        // Circuit is open, but not enough time has passed for half-open attempt
        assert!(client.failed_requests >= client.circuit_breaker_threshold);
        assert!(client.last_attempt_time.elapsed() <= CIRCUIT_BREAKER_COOL_DOWN);
        assert!(!client.is_circuit_open()); // Still considered closed by the method logic

        // Simulate time passing
        tokio::time::sleep(CIRCUIT_BREAKER_COOL_DOWN + Duration::from_secs(1)).await;
        assert!(client.is_circuit_open()); // Now it's time for a half-open attempt

        client.reset_circuit();
        assert!(!client.is_circuit_open());
    }
}
