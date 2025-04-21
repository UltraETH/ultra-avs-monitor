use thiserror::Error;
use std::time::Duration;

#[derive(Error, Debug)]
pub enum BoostMonitorError {
    #[error("HTTP request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    
    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] tokio_tungstenite::tungstenite::Error),
    
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    
    #[error("Failed to connect to relay: {0}")]
    RelayConnectionError(String),
    
    #[error("Invalid response from relay: {0}")]
    InvalidResponseError(String),
    
    #[error("Operation timed out after {0:?}")]
    TimeoutError(Duration),
    
    #[error("WebSocket server error: {0}")]
    WebSocketServerError(String),
    
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Metrics error: {0}")] // Added MetricsError variant
    MetricsError(String),

    #[error("URL parse error: {0}")] // Add missing UrlParseError from main.rs change
    UrlParseError(#[from] url::ParseError),

    // Updated alloy provider error
    #[error("Alloy Provider error: {0}")]
    AlloyProviderError(String),
}

pub type Result<T> = std::result::Result<T, BoostMonitorError>;
