use std::sync::Arc;
use std::{net::SocketAddr, collections::HashMap, time::{Duration, Instant}};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock, Mutex, Notify};
use tokio_tungstenite::accept_async;
use uuid::Uuid;

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
                                let connections = broadcast_server.connections.read().await;
                                
                                for conn in connections.values() {
                                    if let Err(e) = conn.send_bid(&bid).await {
                                        eprintln!("Failed to send bid to client: {}", e);
                                    }
                                }
                                
                                // Periodically clean up inactive connections
                                let mut last_cleanup = broadcast_server.last_cleanup.write().await;
                                if last_cleanup.elapsed() > Duration::from_secs(60) {
                                    *last_cleanup = Instant::now();
                                    drop(last_cleanup);
                                    broadcast_server.cleanup_stale_connections().await;
                                }
                            }
                            None => {
                                println!("Bid channel closed, shutting down broadcast task");
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
                        println!("WebSocket accept task shutting down");
                        break;
                    }
                    
                    // Accept new connections
                    socket_result = listener.accept() => {
                        match socket_result {
                            Ok((stream, addr)) => {
                                // Check connection limits
                                let current_connections = accept_server.connections.read().await.len();
                                if current_connections >= accept_server.connection_limits.max_connections {
                                    eprintln!("Connection limit reached, rejecting connection from: {}", addr);
                                    continue;
                                }
                                
                                println!("New WebSocket connection from: {}", addr);
                                
                                let accept_server = accept_server.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_connection(accept_server, stream, addr).await {
                                        eprintln!("Error handling connection: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!("Failed to accept connection: {}", e);
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
        _addr: SocketAddr,
    ) -> Result<()> {
        let ws_stream = accept_async(stream)
            .await
            .map_err(|e| BoostMonitorError::WebSocketServerError(format!("Error during WebSocket handshake: {}", e)))?;
        
        let connection_id = Uuid::new_v4().to_string();
        let connection = Arc::new(Connection::new(ws_stream));
        
        // Store the connection
        {
            let mut connections = server.connections.write().await;
            connections.insert(connection_id.clone(), connection.clone());
        }
        
        // Start listening for client messages
        connection.listen().await;
        
        Ok(())
    }
    
    pub async fn shutdown(&self) {
        self.shutdown_signal.notify_waiters();
        
        // Wait for all connections to clean up
        let mut connections = self.connections.write().await;
        for conn in connections.values() {
            let _ = conn.close().await;
        }
        connections.clear();
        
        println!("WebSocket server shut down successfully");
    }
    
    pub async fn cleanup_stale_connections(&self) {
        let mut connections = self.connections.write().await;
        let conn_timeout = Duration::from_secs(self.connection_limits.connection_timeout_secs);
        let before_count = connections.len();
        
        // Remove connections that haven't seen activity in the timeout period
        connections.retain(|_, conn| !conn.is_stale(conn_timeout));
        
        let removed = before_count - connections.len();
        if removed > 0 {
            println!("Removed {} stale connections", removed);
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
