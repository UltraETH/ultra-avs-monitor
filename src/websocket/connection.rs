use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::{tungstenite::protocol::Message, WebSocketStream};
use tokio::net::TcpStream;

use crate::types::BidTrace;
use crate::errors::{Result, BoostMonitorError};

pub struct Connection {
    stream: Arc<Mutex<WebSocketStream<TcpStream>>>,
    last_activity: Arc<Mutex<Instant>>,
}

impl Connection {
    pub fn new(stream: WebSocketStream<TcpStream>) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            last_activity: Arc::new(Mutex::new(Instant::now())),
        }
    }
    
    pub async fn send_bid(&self, bid: &BidTrace) -> Result<()> {
        let mut stream = self.stream.lock().await;
        
        // Serialize bid to JSON for sending
        let message = serde_json::to_string(bid)
            .map_err(|e| BoostMonitorError::SerializationError(e))?;
        
        stream
            .send(Message::Text(message))
            .await
            .map_err(|e| BoostMonitorError::WebSocketError(e))?;
        
        // Update last activity timestamp
        let mut last_activity = self.last_activity.lock().await;
        *last_activity = Instant::now();
        
        Ok(())
    }
    
    pub async fn listen(&self) {
        let stream_clone = self.stream.clone();
        let last_activity_clone = self.last_activity.clone();
        
        // Spawn a task to handle incoming messages
        tokio::spawn(async move {
            let mut stream = stream_clone.lock().await;
            
            while let Some(result) = stream.next().await {
                // Update last activity on any message received
                let mut last_activity = last_activity_clone.lock().await;
                *last_activity = Instant::now();
                
                match result {
                    Ok(msg) => {
                        if msg.is_close() {
                            println!("Client sent close frame");
                            break;
                        }
                        // Handle client messages if needed
                    }
                    Err(e) => {
                        eprintln!("Error receiving message: {}", e);
                        break;
                    }
                }
            }
            
            println!("Client disconnected");
        });
    }
    
    pub async fn close(&self) -> Result<()> {
        let mut stream = self.stream.lock().await;
        
        stream
            .send(Message::Close(None))
            .await
            .map_err(|e| BoostMonitorError::WebSocketError(e))?;
            
        Ok(())
    }
    
    pub fn is_stale(&self, timeout: Duration) -> bool {
        let last_activity = self.last_activity.try_lock();
        if let Ok(last_active) = last_activity {
            last_active.elapsed() > timeout
        } else {
            // If we can't get the lock, consider it active
            false
        }
    }
}
