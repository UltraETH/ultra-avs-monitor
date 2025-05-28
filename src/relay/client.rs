use alloy_primitives::U64;
use reqwest::Client;
use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};
use tracing::{debug, error, warn};

use super::RelayService;
use crate::config::RelayConfig;
use crate::errors::{BoostMonitorError, Result};
use crate::types::{BidTrace, DeliveredPayloadTrace};

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(500);
const CIRCUIT_BREAKER_COOL_DOWN: Duration = Duration::from_secs(30);

pub struct RelayClient {
    base_url: String,
    // url: String, // Unused field
    client: Client,
    request_timeout: Duration,
    failed_requests: u32,
    success_count: u32, // Added for monitoring/future use
    circuit_breaker_threshold: u32,
    last_attempt_time: Instant,
}

impl RelayClient {
    pub fn new(config: RelayConfig) -> Self {
        let base_url = config.url.clone();
        Self {
            base_url,
            // url: format!( // This was for the unused field
            //     "{}/relay/v1/data/bidtraces/builder_blocks_received",
            //     config.url
            // ),
            client: Client::new(),
            request_timeout: config.request_timeout,
            failed_requests: 0,
            success_count: 0,
            circuit_breaker_threshold: config.circuit_breaker_threshold,
            last_attempt_time: Instant::now(),
        }
    }

    pub fn new_with_url(base_url: String, request_timeout: Duration) -> Self {
        // let full_url = format!( // This was for the unused field
        //     "{}/relay/v1/data/bidtraces/builder_blocks_received",
        //     base_url
        // );
        Self {
            base_url,
            // url: full_url, // Unused field
            client: Client::new(),
            request_timeout,
            failed_requests: 0,
            success_count: 0,
            circuit_breaker_threshold: 3, // Default threshold
            last_attempt_time: Instant::now(),
        }
    }

    // This method now strictly checks if the circuit is tripped (threshold met).
    // The cool-down logic is handled in get_builder_bids.
    pub fn is_circuit_tripped(&self) -> bool {
        self.failed_requests >= self.circuit_breaker_threshold
    }

    fn record_failure(&mut self) {
        self.failed_requests += 1;
        self.last_attempt_time = Instant::now();
        warn!(url = %self.base_url, failed_requests = self.failed_requests, threshold = self.circuit_breaker_threshold, "Relay request failed");
    }

    fn record_success(&mut self) {
        self.failed_requests = 0;
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
        if self.is_circuit_open() {
            // Use the trait method which checks tripped and cool-down
            debug!(url = %self.base_url, "Circuit breaker open, skipping get_builder_bids request");
            return Err(BoostMonitorError::RelayConnectionError(
                "Circuit breaker open, skipping request".to_string(),
            ));
        }

        let request_url = format!(
            "{}/relay/v1/data/bidtraces/builder_blocks_received?block_number={}",
            &self.base_url, block_num
        );

        for attempt in 1..=MAX_RETRIES {
            debug!(url = %self.base_url, block = %block_num, attempt = attempt, "Attempting to fetch builder bids");
            self.last_attempt_time = Instant::now();

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
                                error!(url = %self.base_url, block = %block_num, error = %e, "Failed to parse JSON response for builder bids");
                                self.record_failure();
                                if attempt == MAX_RETRIES {
                                    return Err(BoostMonitorError::InvalidResponseError(format!(
                                        "Failed to parse JSON for builder bids after {} attempts: {}",
                                        MAX_RETRIES, e
                                    )));
                                }
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_else(|_| "N/A".to_string());
                        error!(url = %self.base_url, block = %block_num, status = %status, body = %body, "Relay returned non-success status for builder bids");
                        self.record_failure();
                        if attempt == MAX_RETRIES {
                            return Err(BoostMonitorError::RelayConnectionError(format!(
                                "Relay returned status {} for builder bids after {} attempts: {}",
                                status, MAX_RETRIES, body
                            )));
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!(url = %self.base_url, block = %block_num, error = %e, "Builder bids request failed");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::RequestError(e));
                    }
                }
                Err(_) => {
                    error!(url = %self.base_url, block = %block_num, timeout = ?self.request_timeout, "Builder bids request timed out");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::TimeoutError(self.request_timeout));
                    }
                }
            }

            if attempt < MAX_RETRIES {
                sleep(RETRY_DELAY).await;
            }
        }
        unreachable!();
    }

    async fn get_delivered_payloads(&mut self, slot: U64) -> Result<Vec<DeliveredPayloadTrace>> {
        if self.is_circuit_open() {
            debug!(url = %self.base_url, "Circuit breaker open, skipping get_delivered_payloads request");
            return Err(BoostMonitorError::RelayConnectionError(
                "Circuit breaker open, skipping request".to_string(),
            ));
        }

        let request_url = format!(
            "{}/relay/v1/data/bidtraces/proposer_payload_delivered?slot={}",
            &self.base_url, slot
        );

        for attempt in 1..=MAX_RETRIES {
            debug!(url = %self.base_url, slot = %slot, attempt = attempt, "Attempting to fetch delivered payloads");
            self.last_attempt_time = Instant::now();

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
                        match response.json::<Vec<DeliveredPayloadTrace>>().await {
                            Ok(data) => {
                                self.record_success();
                                return Ok(data);
                            }
                            Err(e) => {
                                error!(url = %self.base_url, slot = %slot, error = %e, "Failed to parse JSON response for delivered payloads");
                                self.record_failure();
                                if attempt == MAX_RETRIES {
                                    return Err(BoostMonitorError::InvalidResponseError(format!(
                                        "Failed to parse JSON for delivered payloads after {} attempts: {}",
                                        MAX_RETRIES, e
                                    )));
                                }
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_else(|_| "N/A".to_string());
                        error!(url = %self.base_url, slot = %slot, status = %status, body = %body, "Relay returned non-success status for delivered payloads");
                        self.record_failure();
                        if attempt == MAX_RETRIES {
                            return Err(BoostMonitorError::RelayConnectionError(format!(
                                "Relay returned status {} for delivered payloads after {} attempts: {}",
                                status, MAX_RETRIES, body
                            )));
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!(url = %self.base_url, slot = %slot, error = %e, "Delivered payloads request failed");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::RequestError(e));
                    }
                }
                Err(_) => {
                    error!(url = %self.base_url, slot = %slot, timeout = ?self.request_timeout, "Delivered payloads request timed out");
                    self.record_failure();
                    if attempt == MAX_RETRIES {
                        return Err(BoostMonitorError::TimeoutError(self.request_timeout));
                    }
                }
            }

            if attempt < MAX_RETRIES {
                sleep(RETRY_DELAY).await;
            }
        }
        unreachable!();
    }

    fn get_success_count(&self) -> u32 {
        self.success_count
    }

    fn get_failed_requests(&self) -> u32 {
        self.failed_requests
    }

    // This now calls the renamed is_circuit_tripped method.
    // The actual decision to skip a request due to cool-down is in get_builder_bids.
    fn is_circuit_open(&self) -> bool {
        if self.is_circuit_tripped() {
            // If tripped, check if we are still in cool-down.
            // If not in cool-down, the circuit is effectively "half-open" or ready for a test request.
            // The get_builder_bids method will allow one attempt if cool-down has passed.
            self.last_attempt_time.elapsed() <= CIRCUIT_BREAKER_COOL_DOWN
        } else {
            false // Not tripped, so not open in the sense of preventing requests.
        }
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
        assert!(!client.is_circuit_open());

        client.failed_requests = 2; // Circuit is now tripped
        assert!(client.is_circuit_tripped());
        // Immediately after tripping, it's in cool-down, so is_circuit_open() should be true
        assert!(
            client.is_circuit_open(),
            "Circuit should be open (tripped and in cool-down)"
        );

        // Wait for cool-down to pass
        tokio::time::sleep(CIRCUIT_BREAKER_COOL_DOWN + Duration::from_secs(1)).await;

        // After cool-down, is_circuit_tripped() is still true, but is_circuit_open() should be false (half-open state)
        assert!(
            client.is_circuit_tripped(),
            "Circuit should still be tripped after cool-down"
        );
        assert!(
            !client.is_circuit_open(),
            "Circuit should NOT be open after cool-down (half-open state, ready for test request)"
        );

        // Simulate a successful request to reset the circuit
        client.record_success();
        assert!(
            !client.is_circuit_tripped(),
            "Circuit should not be tripped after success"
        );
        assert!(
            !client.is_circuit_open(),
            "Circuit should not be open after success"
        );
    }
}
