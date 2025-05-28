use alloy_primitives::{U256, U64};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use clap::Parser;
use metrics;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::{
    signal,
    sync::{broadcast, Mutex},
    time,
};
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::{fmt, EnvFilter};

use ultra_avs_monitor::{
    bid_manager::BidManager,
    config::Config,
    database::sqlite_writer::SqliteWriter,
    delivered_payload_manager::{DeliveredPayloadManager, DeliveredPayloadSinks},
    errors::{BoostMonitorError, Result},
    file_writer::FileWriter,
    relay_clients::RelayClients,
    relay_wrapper::RelayClientWrapper,
    websocket::WebSocketServer,
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt::Subscriber::builder().with_env_filter(filter).init();
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
    let mut file_writer_arc: Option<Arc<FileWriter>> = None;
    let mut sqlite_writer_arc: Option<Arc<SqliteWriter>> = None;

    if config.output.file_output_enabled {
        info!("File output enabled: {}", config.output.file_output_path);
        let fw = FileWriter::new(
            config.output.file_output_path.clone(),
            config.output.flush_interval.as_secs(),
            config.output.batch_size,
        );
        fw.initialize().await?;
        fw.start_flush_task().await?;
        file_writer_arc = Some(Arc::new(fw));

        let mut bid_subscription_file = bid_manager.subscribe_to_all_new_bids().await;
        let fw_clone = file_writer_arc.as_ref().unwrap().clone();
        tokio::spawn(async move {
            while let Some(bid) = bid_subscription_file.recv().await {
                if let Err(e) = fw_clone.write_bid(bid.clone()).await {
                    // Assuming write_bid exists
                    error!(bid = ?bid, error = %e, "Error writing bid_trace to file");
                }
            }
        });
    }

    if config.output.sqlite_output_enabled {
        info!(
            "SQLite output enabled: {}",
            config.output.sqlite_database_path
        );
        let sw = SqliteWriter::new(
            config.output.sqlite_database_path.clone(),
            Some(config.output.sqlite_flush_interval_secs),
            Some(config.output.sqlite_batch_size),
        )
        .await?;
        sw.initialize().await?; // This creates bid_traces table
                                // We need to ensure delivered_payload_traces table is also created.
                                // For now, SqliteWriter::initialize only creates bid_traces.
                                // This will be handled when SqliteWriter is fully updated for delivered payloads.
        sw.start_flush_task().await?; // This starts flush for bid_traces
        sqlite_writer_arc = Some(Arc::new(sw));

        let mut bid_subscription_sqlite = bid_manager.subscribe_to_all_new_bids().await;
        let sw_clone_bids = sqlite_writer_arc.as_ref().unwrap().clone();
        tokio::spawn(async move {
            while let Some(bid) = bid_subscription_sqlite.recv().await {
                if let Err(e) = sw_clone_bids.write_bid_trace(bid.clone()).await {
                    error!(bid = ?bid, error = %e, "Error writing bid_trace to SQLite");
                }
            }
            // Gracefully shutdown the sqlite_writer part for bids when the bid channel closes
            if let Err(e) = sw_clone_bids.shutdown().await {
                // This currently shuts down the whole pool
                error!(error = %e, "Error shutting down SQLite writer for bids");
            }
        });

        let sw_error_check = sqlite_writer_arc.as_ref().unwrap().clone();
        tokio::spawn(async move {
            let mut error_check_interval = time::interval(Duration::from_secs(30));
            loop {
                error_check_interval.tick().await;
                if let Some(err) = sw_error_check.check_for_errors().await {
                    error!(sqlite_background_error = %err, "Error from SQLite background task");
                }
            }
        });
    }

    // Setup DeliveredPayloadManager
    let delivered_payload_sinks = DeliveredPayloadSinks {
        sqlite_writer: sqlite_writer_arc.clone(), // Pass the Arc<SqliteWriter>
        file_writer: file_writer_arc.clone(),     // Pass the Arc<FileWriter>
    };
    let delivered_payload_manager = Arc::new(DeliveredPayloadManager::new(delivered_payload_sinks));

    // Pass Arc<BidManager> and Arc<DeliveredPayloadManager> to RelayClients
    let relay_clients = RelayClients::with_configs(
        config.relays.clone(),
        bid_manager.clone(),               // Pass the Arc
        delivered_payload_manager.clone(), // Pass the Arc
    );
    let relay_clients_arc = Arc::new(Mutex::new(relay_clients));

    {
        let mut clients = relay_clients_arc.lock().await;
        clients.bid_manager = bid_manager.clone();
    }

    let relay_wrapper = RelayClientWrapper::new(relay_clients_arc.clone());

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

    let provider = ProviderBuilder::new().connect_client(RpcClient::new_http(eth_rpc_url));

    let mut interval = tokio::time::interval(config.polling.interval);

    let mut last_processed_block: u64 = 0;

    let shutdown = signal::ctrl_c();
    tokio::pin!(shutdown);

    let (shutdown_tx, _) = broadcast::channel(1);

    info!("Service started successfully, press Ctrl+C to stop");

    if config.output.metrics_enabled {
        let metrics_addr = config.get_metrics_addr();
        setup_metrics_server(metrics_addr).await?;

        let metrics_relay_clients_arc = relay_clients_arc.clone();
        let mut metrics_shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut metrics_interval = time::interval(Duration::from_secs(10));
            metrics_interval.tick().await;

            loop {
                tokio::select! {
                    _ = metrics_interval.tick() => {
                        let relay_clients = metrics_relay_clients_arc.lock().await;
                        let metrics_data = relay_clients.get_client_metrics().await;

                        for (url, success, failed) in metrics_data {
                            metrics::gauge!("relay_success_total", "url" => url.clone()).set(success as f64);
                            metrics::gauge!("relay_failed_total", "url" => url).set(failed as f64);
                        }
                    }
                    _ = metrics_shutdown_rx.recv() => {
                        info!("Relay metrics task received shutdown signal, stopping...");
                        break;
                    }
                }
            }
            info!("Relay metrics task stopped.");
        });
    }

    loop {
        tokio::select! {
                _ = shutdown.as_mut() => {
                    info!("Shutdown signal received, stopping service...");
                    let _ = shutdown_tx.send(());
                    websocket_server.shutdown().await;
                    // Note: SqliteWriter will shutdown when its bid_subscription channel closes.
                    // File writer also stops when its bid_subscription channel closes.
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

                                // Prune old blocks from BidManager
                                bid_manager.prune_old_blocks(
                                    U256::from(latest_u64),
                                    config.polling.block_retention_blocks
                                ).await;

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
    let handle = builder.install_recorder().map_err(|e| {
        BoostMonitorError::MetricsError(format!("Failed to install Prometheus recorder: {}", e))
    })?;

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
            .route(
                "/metrics",
                axum::routing::get(|| async move {
                    let metrics = handle.render();
                    axum::response::Html(metrics)
                }),
            )
            .route("/health", axum::routing::get(|| async { "OK" }));

        if let Err(e) = axum::serve(listener, app.into_make_service()).await {
            error!(address = %addr, error = %e, "Metrics server failed");
        }
    });

    Ok(())
}
