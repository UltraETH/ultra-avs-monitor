use std::sync::Arc;
use std::{net::SocketAddr, collections::HashMap, time::{Duration, Instant}};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock, Mutex, Notify};
use tokio_tungstenite::accept_async;
use uuid::Uuid;
use tracing::{info, error, warn, debug}; // Added tracing imports

use crate::types::BidTrace;
use crate::errors::{Result, BoostMonitorError};
use crate::config::ServerConfig;
use super::connection::Connection;

pub struct WebSocketServer {
    connections: Arc<RwLock<HashMap<String, Arc<Connection>>>>,
    connection_limits: ServerConfig,
    bid_sender: mpsc::Sender<BidTrace>,
    bid_receiver: Arc<Mutex<mpsc::Receiver<BidTrace>>>,
    shutdown_signal: Arc<Notify>,
    last_cleanup: Arc<RwLock<Instant>>,
}

impl WebSocketServer {
    pub fn new(config: ServerConfig) -> Self {
        let (bid_sender, bid_receiver) = mpsc::channel(100);

        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            connection_limits: config,
            bid_sender,
            bid_receiver: Arc::new(Mutex::new(bid_receiver)),
            shutdown_signal: Arc::new(Notify::new()),
            last_cleanup: Arc::new(RwLock::new(Instant::now())),
        }
    }

    pub fn bid_sender(&self) -> mpsc::Sender<BidTrace> {
        self.bid_sender.clone()
    }

    pub async fn start(&self, addr: SocketAddr) -> Result<()> {
        let server = Arc::new(self.clone());

        // Set up the TCP listener
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| BoostMonitorError::WebSocketServerError(format!("Failed to bind to address: {}", e)))?;

        println!("WebSocket server listening on: {}", addr);

        // Spawn task to broadcast bids to all connections
        let broadcast_server = server.clone();
        tokio::spawn(async move {
            let mut receiver = broadcast_server.bid_receiver.lock().await;

            loop {
                tokio::select! {
                    // Handle shutdown signal
                    _ = broadcast_server.shutdown_signal.notified() => {
                        println!("WebSocket broadcast task shutting down");
                        break;
                    }

                    // Handle incoming bids
                    bid = receiver.recv() => {
                        match bid {
                            Some(bid) => {
                                let connections_read = broadcast_server.connections.read().await;
                                let mut failed_connection_ids = Vec::new();

                                for (id, conn) in connections_read.iter() {
                                    if let Err(e) = conn.send_bid(&bid).await {
                                        error!(connection_id = %id, error = %e, "Failed to send bid to client, marking for removal");
                                        failed_connection_ids.push(id.clone());
                                    }
                                }
                                drop(connections_read); // Drop read lock before acquiring write lock

                                if !failed_connection_ids.is_empty() {
                                    let mut connections_write = broadcast_server.connections.write().await;
                                    for id in failed_connection_ids {
                                        connections_write.remove(&id);
                                        info!(connection_id = %id, "Removed failed connection");
                                    }
                                }


                                // Periodically clean up inactive connections
                                let mut last_cleanup = broadcast_server.last_cleanup.write().await;
                                if last_cleanup.elapsed() > Duration::from_secs(60) { // Cleanup every 60 seconds
                                    *last_cleanup = Instant::now();
                                    drop(last_cleanup); // Drop write lock before calling cleanup
                                    broadcast_server.cleanup_stale_connections().await;
                                }
                            }
                            None => {
                                info!("Bid channel closed, shutting down broadcast task");
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Accept incoming WebSocket connections
        let accept_server = server.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Handle shutdown signal
                    _ = accept_server.shutdown_signal.notified() => {
                        info!("WebSocket accept task shutting down");
                        break;
                    }

                    // Accept new connections
                    socket_result = listener.accept() => {
                        match socket_result {
                            Ok((stream, addr)) => {
                                // Check connection limits
                                let current_connections = accept_server.connections.read().await.len();
                                if current_connections >= accept_server.connection_limits.max_connections {
                                    warn!(address = %addr, current = current_connections, limit = accept_server.connection_limits.max_connections, "Connection limit reached, rejecting connection");
                                    continue;
                                }

                                info!("New WebSocket connection from: {}", addr);

                                let accept_server = accept_server.clone();
                                tokio::spawn(async move {
                                    // Pass the shutdown signal to handle_connection
                                    if let Err(e) = Self::handle_connection(accept_server, stream, addr).await {
                                        error!(address = %addr, error = %e, "Error handling connection");
                                    }
                                });
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to accept connection");
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    async fn handle_connection(
        server: Arc<WebSocketServer>,
        stream: TcpStream,
        addr: SocketAddr, // Keep addr for logging
    ) -> Result<()> {
        let ws_stream = accept_async(stream)
            .await
            .map_err(|e| BoostMonitorError::WebSocketServerError(format!("Error during WebSocket handshake: {}", e)))?;

        let connection_id = Uuid::new_v4().to_string();
        // Pass the server's shutdown signal to the new Connection
        let connection = Arc::new(Connection::new(ws_stream, server.shutdown_signal.clone()));

        info!(connection_id = %connection_id, address = %addr, "Connection established");

        // Store the connection
        {
            let mut connections = server.connections.write().await;
            connections.insert(connection_id.clone(), connection.clone());
        }

        // Start listening for client messages - pass shutdown signal
        connection.listen().await;

        // Connection task finished, remove from map
        {
            let mut connections = server.connections.write().await;
            connections.remove(&connection_id);
            info!(connection_id = %connection_id, address = %addr, "Connection closed or task finished, removed from map");
        }


        Ok(())
    }

    pub async fn shutdown(&self) {
        info!("Initiating WebSocket server shutdown");
        self.shutdown_signal.notify_waiters();

        // Give connection tasks a moment to receive the signal and shut down gracefully
        tokio::time::sleep(Duration::from_secs(1)).await; // Small delay

        // Attempt to close remaining connections and clear the map
        let mut connections = self.connections.write().await;
        let count = connections.len();
        if count > 0 {
            warn!("Forcing close of {} remaining connections during shutdown", count);
            for conn in connections.values() {
                let _ = conn.close().await; // Attempt to send close frame
            }
        }
        connections.clear();

        info!("WebSocket server shut down successfully");
    }

    pub async fn cleanup_stale_connections(&self) {
        let mut connections = self.connections.write().await;
        let conn_timeout = Duration::from_secs(self.connection_limits.connection_timeout_secs);
        let before_count = connections.len();

        // Remove connections that haven't seen activity in the timeout period
        connections.retain(|id, conn| {
            if conn.is_stale(conn_timeout) {
                debug!(connection_id = %id, "Connection is stale, removing");
                // Optionally send a close frame here before removing
                let conn_clone = conn.clone();
                let id_clone = id.clone(); // Clone id for the spawned task
                tokio::spawn(async move {
                    if let Err(e) = conn_clone.close().await {
                        error!(connection_id = %id_clone, error = %e, "Error sending close frame to stale connection");
                    }
                });
                false // Remove the connection
            } else {
                true // Keep the connection
            }
        });

        let removed = before_count - connections.len();
        if removed > 0 {
            info!("Removed {} stale connections", removed);
        }
    }
}

impl Clone for WebSocketServer {
    fn clone(&self) -> Self {
        Self {
            connections: self.connections.clone(),
            connection_limits: self.connection_limits.clone(),
            bid_sender: self.bid_sender.clone(),
            bid_receiver: self.bid_receiver.clone(),
            shutdown_signal: self.shutdown_signal.clone(),
            last_cleanup: self.last_cleanup.clone(),
        }
    }
}
