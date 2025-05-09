use std::{net::SocketAddr, sync::Arc, path::Path};
use clap::Parser;
use tokio::signal;
use metrics;
use metrics_exporter_prometheus::PrometheusBuilder;
use tracing::{info, warn, error, debug, instrument};
use tracing_subscriber::{fmt, EnvFilter};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_primitives::U64;

use ultra_avs_monitor::{
    config::Config,
    errors::{Result, BoostMonitorError},
    file_writer::FileWriter,
    relay_clients::RelayClients,
    relay_wrapper::RelayClientWrapper,
    websocket::WebSocketServer,
    bid_manager::BidManager,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    config: Option<String>,

    #[arg(short, long)]
    output: Option<String>,

    #[arg(short, long)]
    port: Option<u16>,

    #[arg(short, long, default_value_t = true)]
    metrics: bool,

    #[arg(long)]
    metrics_port: Option<u16>,
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt::Subscriber::builder()
        .with_env_filter(filter)
        .init();
}

#[tokio::main]
#[instrument]
async fn main() -> Result<()> {
    init_tracing();

    let args = Args::parse();

    let mut config = match args.config {
        Some(path) => Config::from_file(path.into())?,
        None => Config::from_env()?,
    };

    if let Some(port) = args.port {
        config.server.websocket_port = port;
    }

    if let Some(output_path) = args.output {
        config.output.file_output_path = output_path;
        config.output.file_output_enabled = true;
    }

    if let Some(metrics_port) = args.metrics_port {
        config.output.metrics_port = metrics_port;
    }

    config.output.metrics_enabled = args.metrics;

    info!("Starting AVS Boost Monitor");
    info!("WebSocket server: {}", config.get_websocket_addr());
    if config.output.metrics_enabled {
        info!("Metrics server: {}", config.get_metrics_addr());
    }
    if config.output.file_output_enabled {
        info!("File output: {}", config.output.file_output_path);

        if let Some(parent) = Path::new(&config.output.file_output_path).parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    warn!(path = ?parent, error = %e, "Could not create directory for file output");
                }
            }
        }
    }

    let bid_manager = Arc::new(BidManager::new());

    if config.output.file_output_enabled {
        let file_writer = FileWriter::new(
            config.output.file_output_path.clone(),
            config.output.flush_interval.as_secs(),
            config.output.batch_size,
        );

        file_writer.initialize().await?;

        file_writer.start_flush_task().await?;

        let mut bid_subscription = bid_manager.subscribe_to_all_new_bids().await;

        let writer_clone = file_writer.clone();

        tokio::spawn(async move {
            while let Some(bid) = bid_subscription.recv().await {
                if let Err(e) = writer_clone.write_bid(bid.clone()).await {
                    error!(bid = ?bid, error = %e, "Error writing bid to file");
                }
            }
        });
    }

    if config.output.metrics_enabled {
        let metrics_addr = config.get_metrics_addr();
        setup_metrics_server(metrics_addr).await?;

        let mut top_bid_subscription = bid_manager.subscribe_to_top_bids().await;

        tokio::spawn(async move {
            while let Some(bid) = top_bid_subscription.recv().await {
                let block_number = bid.block_number.to_string();

                match bid.value.to_string().parse::<f64>() {
                    Ok(value_f64) => {
                        let key = format!("highest_bid_value_block_{}", block_number);
                        metrics::gauge!(key).set(value_f64);
                    }
                    Err(_) => {
                        warn!(value = %bid.value, "Failed to parse bid value for metrics");
                    }
                }

                match bid.gas_used.to_string().parse::<f64>() {
                    Ok(gas_used_f64) => {
                        let key = format!("highest_bid_gas_used_block_{}", block_number);
                        metrics::gauge!(key).set(gas_used_f64);
                    }
                    Err(_) => {
                        warn!(gas_used = %bid.gas_used, "Failed to parse bid gas_used for metrics");
                    }
                }

                match bid.num_tx.to_string().parse::<f64>() {
                    Ok(num_tx_f64) => {
                        let key = format!("highest_bid_num_tx_block_{}", block_number);
                        metrics::gauge!(key).set(num_tx_f64);
                    }
                    Err(_) => {
                        warn!(num_tx = %bid.num_tx, "Failed to parse bid num_tx for metrics");
                    }
                }
            }
        });
    }

    let mut relay_clients = RelayClients::with_configs(config.relays.clone());
    relay_clients.bid_manager = bid_manager.clone();
    let mut relay_wrapper = RelayClientWrapper::new(relay_clients);

    let websocket_server = WebSocketServer::new(config.server.clone());
    let websocket_sender = websocket_server.bid_sender();

    bid_manager.set_websocket_sender(websocket_sender).await;

    let websocket_addr = config.get_websocket_addr();
    websocket_server.start(websocket_addr).await?;

    let mut bid_manager_receiver = bid_manager.subscribe_to_top_bids().await;

    tokio::spawn(async move {
        while let Some(data) = bid_manager_receiver.recv().await {
            info!(bid = ?data, "New Highest Bid Received");
        }
    });

    let eth_rpc_url = config.polling.ethereum_rpc_url.parse()?;

    let provider = ProviderBuilder::new()
        .connect_client(RpcClient::new_http(eth_rpc_url));

    let mut interval = tokio::time::interval(config.polling.interval);

    let mut last_processed_block: u64 = 0;

    let shutdown = signal::ctrl_c();
    tokio::pin!(shutdown);

    info!("Service started successfully, press Ctrl+C to stop");

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("Shutdown signal received, stopping service...");
                websocket_server.shutdown().await;
                break;
            }

            _ = interval.tick() => {
    match provider.get_block_number().await {
                    Ok(latest_block) => {
                        let latest_u64 = latest_block.to_string().parse::<u64>().unwrap_or(0);

                        if latest_u64 > last_processed_block {
                            last_processed_block = latest_u64;

                            info!(block_number = latest_u64, "New block detected");

                            metrics::counter!("blocks_processed").increment(1);

                            let block_for_relay = U64::from(latest_u64);

                            if let Err(e) = relay_wrapper.poll_for(
                                block_for_relay,
                                config.polling.interval.as_secs(),
                                config.polling.duration.as_secs()
                            ).await {
                                error!(block_number = latest_u64, error = %e,
                                      "Error during relay polling");
                            }
                        } else {
                            debug!(block_number = last_processed_block, "No new block detected");
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Error requesting block number via RPC");
                        continue;
                    }
                }
            }
        }
    }

    info!("Service stopped");
    Ok(())
}

#[instrument(skip(addr))]
async fn setup_metrics_server(addr: SocketAddr) -> Result<()> {
    let builder = PrometheusBuilder::new();
    let handle = builder.install_recorder()
        .map_err(|e| BoostMonitorError::MetricsError(format!("Failed to install Prometheus recorder: {}", e)))?;

    tokio::spawn(async move {
        info!(address = %addr, "Starting metrics server");
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                error!(address = %addr, error = %e, "Failed to bind metrics server listener");
                return;
            }
        };
        let app = axum::Router::new()
            .route("/metrics", axum::routing::get(|| async move {
                let metrics = handle.render();
                axum::response::Html(metrics)
            }))
            .route("/health", axum::routing::get(|| async { "OK" }));

        if let Err(e) = axum::serve(listener, app.into_make_service()).await {
            error!(address = %addr, error = %e, "Metrics server failed");
        }
    });

    Ok(())
}
