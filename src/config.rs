use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::errors::{BoostMonitorError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub relays: Vec<RelayConfig>,
    #[serde(default)]
    pub polling: PollingConfig,
    #[serde(default)]
    pub output: OutputConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            relays: vec![
                RelayConfig {
                    url: "https://relay.ultrasound.money".to_string(),
                    request_timeout: Duration::from_secs(5),
                    circuit_breaker_threshold: 3,
                },
                RelayConfig {
                    url: "https://agnostic-relay.net".to_string(),
                    request_timeout: Duration::from_secs(5),
                    circuit_breaker_threshold: 3,
                },
                RelayConfig {
                    url: "https://boost-relay.flashbots.net".to_string(),
                    request_timeout: Duration::from_secs(5),
                    circuit_breaker_threshold: 3,
                },
            ],
            polling: PollingConfig::default(),
            output: OutputConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_websocket_port")]
    pub websocket_port: u16,
    
    #[serde(default = "default_websocket_address")]
    pub websocket_address: IpAddr,
    
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            websocket_port: default_websocket_port(),
            websocket_address: default_websocket_address(),
            max_connections: default_max_connections(),
            connection_timeout_secs: default_connection_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayConfig {
    pub url: String,
    
    #[serde(with = "humantime_serde", default = "default_request_timeout")]
    pub request_timeout: Duration,
    
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PollingConfig {
    #[serde(with = "humantime_serde", default = "default_poll_interval")]
    pub interval: Duration,
    
    #[serde(with = "humantime_serde", default = "default_poll_duration")]
    pub duration: Duration,
    
    #[serde(default = "default_rpc_url")]
    pub ethereum_rpc_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputConfig {
    #[serde(default = "default_output_enabled")]
    pub file_output_enabled: bool,
    
    #[serde(default = "default_output_path")]
    pub file_output_path: String,
    
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    
    #[serde(with = "humantime_serde", default = "default_flush_interval")]
    pub flush_interval: Duration,
    
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
    
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
    
    #[serde(default = "default_metrics_host")]
    pub metrics_host: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            file_output_enabled: default_output_enabled(),
            file_output_path: default_output_path(),
            batch_size: default_batch_size(),
            flush_interval: default_flush_interval(),
            metrics_enabled: default_metrics_enabled(),
            metrics_port: default_metrics_port(),
            metrics_host: default_metrics_host(),
        }
    }
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval: default_poll_interval(),
            duration: default_poll_duration(),
            ethereum_rpc_url: default_rpc_url(),
        }
    }
}

fn default_websocket_port() -> u16 {
    8090
}

fn default_websocket_address() -> IpAddr {
    IpAddr::from_str("0.0.0.0").unwrap()
}

fn default_max_connections() -> usize {
    100
}

fn default_connection_timeout() -> u64 {
    60
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_circuit_breaker_threshold() -> u32 {
    3
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(1)
}

fn default_poll_duration() -> Duration {
    Duration::from_secs(12)
}

fn default_rpc_url() -> String {
    "https://eth.llamarpc.com".to_string()
}

fn default_output_enabled() -> bool {
    true
}

fn default_output_path() -> String {
    "./data/bids.jsonl".to_string()
}

fn default_batch_size() -> usize {
    100
}

fn default_flush_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_metrics_enabled() -> bool {
    true
}

fn default_metrics_port() -> u16 {
    9090
}

fn default_metrics_host() -> String {
    "0.0.0.0".to_string()
}

impl Config {
    pub fn get_websocket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.server.websocket_address, self.server.websocket_port)
    }
    
    pub fn get_metrics_addr(&self) -> SocketAddr {
        let host = IpAddr::from_str(&self.output.metrics_host).unwrap_or_else(|_| {
            eprintln!("Invalid metrics host IP, using default 0.0.0.0");
            IpAddr::from_str("0.0.0.0").unwrap()
        });
        SocketAddr::new(host, self.output.metrics_port)
    }
    
    pub fn from_file(path: PathBuf) -> Result<Self> {
        let config_str = std::fs::read_to_string(path)
            .map_err(|e| BoostMonitorError::ConfigError(format!("Failed to read config file: {}", e)))?;
            
        let config: Config = serde_json::from_str(&config_str)
            .map_err(|e| BoostMonitorError::ConfigError(format!("Failed to parse config: {}", e)))?;
            
        Ok(config)
    }
    
    pub fn from_env() -> Result<Self> {
        let mut config = Config::default();
        
        if let Ok(port_str) = std::env::var("WEBSOCKET_PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                config.server.websocket_port = port;
            }
        }
        
        if let Ok(rpc_url) = std::env::var("ETHEREUM_RPC_URL") {
            config.polling.ethereum_rpc_url = rpc_url;
        }
        
        if let Ok(relays) = std::env::var("RELAY_URLS") {
            config.relays = relays
                .split(',')
                .map(|url| RelayConfig {
                    url: url.trim().to_string(),
                    request_timeout: default_request_timeout(),
                    circuit_breaker_threshold: default_circuit_breaker_threshold(),
                })
                .collect();
        }
        
        if let Ok(output_path) = std::env::var("FILE_OUTPUT_PATH") {
            config.output.file_output_path = output_path;
        }
        
        if let Ok(metrics_port_str) = std::env::var("METRICS_PORT") {
            if let Ok(port) = metrics_port_str.parse::<u16>() {
                config.output.metrics_port = port;
            }
        }
        
        Ok(config)
    }
}
