use std::sync::Arc;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, instrument}; // Removed warn

use crate::errors::{BoostMonitorError, Result};
use crate::types::BidTrace;

const MAX_BATCH_SIZE: usize = 1000; // Max bids to insert in one transaction
const DEFAULT_FLUSH_INTERVAL_SECS: u64 = 5;

pub struct SqliteWriter {
    db_path: String,
    pool: Arc<Pool<Sqlite>>,
    current_batch: Arc<RwLock<Vec<BidTrace>>>,
    flush_interval: Duration,
    batch_size: usize,
    // For reporting errors from the background task
    error_sender: mpsc::Sender<BoostMonitorError>,
    error_receiver: Arc<RwLock<mpsc::Receiver<BoostMonitorError>>>,
}

impl SqliteWriter {
    pub async fn new(
        db_path: String,
        flush_interval_secs: Option<u64>,
        batch_size: Option<usize>,
    ) -> Result<Self> {
        let connect_options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5) // Adjust as needed
            .connect_with(connect_options)
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to connect to SQLite: {}", e)))?;

        let (error_sender, error_receiver) = mpsc::channel(100);

        Ok(Self {
            db_path,
            pool: Arc::new(pool),
            current_batch: Arc::new(RwLock::new(Vec::with_capacity(
                batch_size.unwrap_or(MAX_BATCH_SIZE),
            ))),
            flush_interval: Duration::from_secs(
                flush_interval_secs.unwrap_or(DEFAULT_FLUSH_INTERVAL_SECS),
            ),
            batch_size: batch_size.unwrap_or(MAX_BATCH_SIZE).min(MAX_BATCH_SIZE),
            error_sender,
            error_receiver: Arc::new(RwLock::new(error_receiver)),
        })
    }

    #[instrument(skip(self))]
    pub async fn initialize(&self) -> Result<()> {
        info!(database_path = %self.db_path, "Initializing SQLite writer and creating table if not exists");

        let create_table_query = "
        CREATE TABLE IF NOT EXISTS bid_traces (
            slot TEXT NOT NULL,
            parent_hash TEXT NOT NULL,
            block_hash TEXT NOT NULL PRIMARY KEY,
            builder_pubkey TEXT NOT NULL,
            proposer_pubkey TEXT NOT NULL,
            proposer_fee_recipient TEXT NOT NULL,
            gas_limit TEXT NOT NULL,
            gas_used TEXT NOT NULL,
            value TEXT NOT NULL,
            block_number TEXT NOT NULL,
            num_tx TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            timestamp_ms TEXT NOT NULL,
            received_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );";

        sqlx::query(create_table_query)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create bid_traces table: {}", e)))?;

        info!("SQLite bid_traces table ensured");

        // Also ensure delivered_payload_traces table exists
        let create_delivered_payloads_table_query = "
        CREATE TABLE IF NOT EXISTS delivered_payload_traces (
            slot TEXT NOT NULL,
            parent_hash TEXT NOT NULL,
            block_hash TEXT NOT NULL PRIMARY KEY,
            builder_pubkey TEXT NOT NULL,
            proposer_pubkey TEXT NOT NULL,
            proposer_fee_recipient TEXT NOT NULL,
            value TEXT NOT NULL,
            block_number TEXT NOT NULL,
            num_tx TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            received_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );";

        sqlx::query(create_delivered_payloads_table_query)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create delivered_payload_traces table: {}", e)))?;

        info!("SQLite delivered_payload_traces table ensured");
        Ok(())
    }

    #[instrument(skip(self, bid))]
    pub async fn write_bid_trace(&self, bid: BidTrace) -> Result<()> {
        let mut batch = self.current_batch.write().await;
        debug!(value = %bid.value, "Adding bid_trace to SQLite batch");
        batch.push(bid);

        if batch.len() >= self.batch_size {
            drop(batch); // Release lock before flushing
            self.flush_bid_traces().await?;
        }
        Ok(())
    }

    // Renamed from flush to be specific to bid_traces
    #[instrument(skip(self))]
    pub async fn flush_bid_traces(&self) -> Result<()> {
        let mut batch_guard = self.current_batch.write().await;
        if batch_guard.is_empty() {
            debug!("SQLite flush_bid_traces called but batch is empty, skipping write");
            return Ok(());
        }

        let bids_to_write = std::mem::replace(&mut *batch_guard, Vec::with_capacity(self.batch_size));
        drop(batch_guard); // Release lock before database operation

        info!(num_bids = bids_to_write.len(), "Flushing bid_traces batch to SQLite");

        let mut tx = self.pool.begin().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to begin transaction for bid_traces: {}", e)))?;

        for bid in bids_to_write.iter() {
            let query = "
            INSERT OR IGNORE INTO bid_traces (
                slot, parent_hash, block_hash, builder_pubkey, proposer_pubkey,
                proposer_fee_recipient, gas_limit, gas_used, value,
                block_number, num_tx, timestamp, timestamp_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);";

            sqlx::query(query)
                .bind(bid.slot.to_string())
                .bind(&bid.parent_hash)
                .bind(&bid.block_hash)
                .bind(&bid.builder_pubkey)
                .bind(&bid.proposer_pubkey)
                .bind(format!("{:?}", bid.proposer_fee_recipient)) // Address to hex string
                .bind(bid.gas_limit.to_string())
                .bind(bid.gas_used.to_string())
                .bind(bid.value.to_string())
                .bind(bid.block_number.to_string())
                .bind(bid.num_tx.to_string())
                .bind(bid.timestamp.to_string())
                .bind(bid.timestamp_ms.to_string())
                .execute(&mut *tx) // Use &mut *tx
                .await
                .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert bid_trace: {}", e)))?;
        }

        tx.commit().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to commit bid_traces transaction: {}", e)))?;
        info!(count = bids_to_write.len(), "SQLite bid_traces batch flushed successfully");
        Ok(())
    }

    // New method for delivered payloads
    #[instrument(skip(self, payload))]
    pub async fn write_delivered_payload(&self, payload: DeliveredPayloadTrace) -> Result<()> {
        // For simplicity, directly insert without batching for now.
        // Production systems might want batching similar to bid_traces.
        debug!(block_hash = %payload.block_hash, value = %payload.value, "Writing delivered_payload to SQLite");

        let query = "
        INSERT OR IGNORE INTO delivered_payload_traces (
            slot, parent_hash, block_hash, builder_pubkey, proposer_pubkey,
            proposer_fee_recipient, value, block_number, num_tx, timestamp
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?);";

        sqlx::query(query)
            .bind(payload.slot.to_string())
            .bind(&payload.parent_hash)
            .bind(&payload.block_hash)
            .bind(&payload.builder_pubkey)
            .bind(&payload.proposer_pubkey)
            .bind(format!("{:?}", payload.proposer_fee_recipient))
            .bind(payload.value.to_string())
            .bind(payload.block_number.to_string())
            .bind(payload.num_tx.to_string())
            .bind(payload.timestamp.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert delivered_payload: {}", e)))?;

        info!(block_hash = %payload.block_hash, "Delivered payload written to SQLite");
        Ok(())
    }


    #[instrument(skip(self))]
    pub async fn start_flush_task(&self) -> Result<()> {
        info!("Starting SQLite background flush task for bid_traces");
        let writer_clone = self.clone();

        tokio::spawn(async move {
            let mut flush_timer = interval(writer_clone.flush_interval);
            loop {
                flush_timer.tick().await;
                debug!("SQLite bid_traces flush interval ticked");
                if let Err(e) = writer_clone.flush_bid_traces().await { // Call specific flush
                    error!(error = %e, "Error flushing bid_traces to SQLite in background task");
                    if let Err(send_err) = writer_clone.error_sender.send(e).await {
                         error!("Failed to send SQLite error (bid_traces) to main application: {}", send_err);
                    }
                }
            }
        });
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down SQLite writer, flushing remaining bid_traces");
        self.flush_bid_traces().await?; // Flush bid_traces specifically
        // Delivered payloads are written directly for now, so no separate flush needed here.
        self.pool.close().await;
        info!("SQLite writer shut down");
        Ok(())
    }

    pub async fn check_for_errors(&self) -> Option<BoostMonitorError> {
        let mut receiver = self.error_receiver.write().await;
        receiver.try_recv().ok()
    }
}

impl Clone for SqliteWriter {
    fn clone(&self) -> Self {
        Self {
            db_path: self.db_path.clone(),
            pool: self.pool.clone(),
            current_batch: self.current_batch.clone(),
            flush_interval: self.flush_interval,
            batch_size: self.batch_size,
            error_sender: self.error_sender.clone(),
            error_receiver: self.error_receiver.clone(),
        }
    }
}
