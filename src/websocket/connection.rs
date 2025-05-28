use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use tokio_tungstenite::{tungstenite::protocol::Message, WebSocketStream};
use tracing::{debug, error, info};

use crate::errors::{BoostMonitorError, Result};
use crate::types::BidTrace;

pub struct Connection {
    stream: Arc<Mutex<WebSocketStream<TcpStream>>>,
    last_activity: Arc<Mutex<Instant>>,
    shutdown_signal: Arc<Notify>,
}

impl Connection {
    pub fn new(stream: WebSocketStream<TcpStream>, shutdown_signal: Arc<Notify>) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            shutdown_signal,
        }
    }

    pub async fn send_bid(&self, bid: &BidTrace) -> Result<()> {
        let mut stream = self.stream.lock().await;

        let message =
            serde_json::to_string(bid).map_err(|e| BoostMonitorError::SerializationError(e))?;

        stream
            .send(Message::Text(message.into()))
            .await
            .map_err(|e| BoostMonitorError::WebSocketError(e))?;

        let mut last_activity = self.last_activity.lock().await;
        *last_activity = Instant::now();

        Ok(())
    }

    pub async fn listen(&self) {
        let stream_clone = self.stream.clone();
        let last_activity_clone = self.last_activity.clone();
        let shutdown_signal_clone = self.shutdown_signal.clone();

        tokio::spawn(async move {
            let mut stream = stream_clone.lock().await;

            loop {
                tokio::select! {
                    maybe_result = stream.next() => {
                        match maybe_result {
                            Some(Ok(msg)) => {
                                let mut last_activity = last_activity_clone.lock().await;
                                *last_activity = Instant::now();

                                if msg.is_close() {
                                    info!("Client sent close frame");
                                    if let Err(e) = stream.send(Message::Close(None)).await {
                                        debug!("Error sending close frame back to client: {}", e);
                                    }
                                    break;
                                }
                                debug!("Received message from client: {:?}", msg);
                            }
                            Some(Err(e)) => {
                                error!("Error receiving message: {}", e);
                                break;
                            }
                            None => {
                                info!("Client stream closed");
                                break;
                            }
                        }
                    }
                    _ = shutdown_signal_clone.notified() => {
                        info!("Connection received shutdown signal, closing");
                        if let Err(e) = stream.send(Message::Close(None)).await {
                            debug!("Error sending close frame on shutdown: {}", e);
                        }
                        break;
                    }
                }
            }

            info!("Connection task finished");
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
            false
        }
    }
}
