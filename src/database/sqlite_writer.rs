use std::sync::Arc;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, instrument}; // Removed warn

use crate::errors::{BoostMonitorError, Result};
use crate::types::{BidTrace, DeliveredPayloadTrace}; // Added DeliveredPayloadTrace

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
        info!(database_path = %self.db_path, "Initializing SQLite writer and creating new schema tables if not exist");

        let mut tx = self.pool.begin().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to begin transaction for schema creation: {}", e)))?;

        // Builders Table
        sqlx::query("
            CREATE TABLE IF NOT EXISTS builders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                builder_pubkey TEXT UNIQUE NOT NULL,
                first_seen_ms INTEGER NOT NULL
            );
        ").execute(&mut *tx).await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create builders table: {}", e)))?;
        info!("Table 'builders' ensured");

        // Proposers Table
        sqlx::query("
            CREATE TABLE IF NOT EXISTS proposers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                proposer_pubkey TEXT UNIQUE NOT NULL,
                fee_recipient_address TEXT NOT NULL,
                first_seen_ms INTEGER NOT NULL
            );
        ").execute(&mut *tx).await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create proposers table: {}", e)))?;
        info!("Table 'proposers' ensured");

        // Blocks Table
        sqlx::query("
            CREATE TABLE IF NOT EXISTS blocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_hash TEXT UNIQUE NOT NULL,
                parent_hash TEXT NOT NULL,
                slot TEXT NOT NULL,
                block_number TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                timestamp_ms TEXT,
                num_tx TEXT NOT NULL,
                gas_limit TEXT,
                gas_used TEXT,
                received_at_ms INTEGER NOT NULL
            );
        ").execute(&mut *tx).await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create blocks table: {}", e)))?;
        info!("Table 'blocks' ensured");

        // Bids Table
        sqlx::query("
            CREATE TABLE IF NOT EXISTS bids (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_id INTEGER NOT NULL,
                builder_id INTEGER NOT NULL,
                proposer_id INTEGER NOT NULL,
                value TEXT NOT NULL,
                received_at_ms INTEGER NOT NULL,
                FOREIGN KEY(block_id) REFERENCES blocks(id),
                FOREIGN KEY(builder_id) REFERENCES builders(id),
                FOREIGN KEY(proposer_id) REFERENCES proposers(id),
                UNIQUE (block_id, builder_id, value)
            );
        ").execute(&mut *tx).await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create bids table: {}", e)))?;
        info!("Table 'bids' ensured");

        // Delivered Payloads Table
        sqlx::query("
            CREATE TABLE IF NOT EXISTS delivered_payloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_id INTEGER UNIQUE NOT NULL,
                builder_id INTEGER NOT NULL,
                proposer_id INTEGER NOT NULL,
                value TEXT NOT NULL,
                received_at_ms INTEGER NOT NULL,
                FOREIGN KEY(block_id) REFERENCES blocks(id),
                FOREIGN KEY(builder_id) REFERENCES builders(id),
                FOREIGN KEY(proposer_id) REFERENCES proposers(id)
            );
        ").execute(&mut *tx).await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to create delivered_payloads table: {}", e)))?;
        info!("Table 'delivered_payloads' ensured");

        tx.commit().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to commit schema creation transaction: {}", e)))?;

        info!("New SQLite schema tables ensured");
        Ok(())
    }

    // Helper function to get or create a builder_id
    async fn get_or_create_builder_id(tx: &mut sqlx::Transaction<'_, Sqlite>, builder_pubkey: &str) -> Result<i64> {
        // Attempt to find existing builder
        let existing_builder_id: Option<i64> = sqlx::query_scalar("SELECT id FROM builders WHERE builder_pubkey = ?")
            .bind(builder_pubkey)
            .fetch_optional(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to query builder: {}", e)))?;

        if let Some(id) = existing_builder_id {
            Ok(id)
        } else {
            // Insert new builder if not found
            let new_builder_id: i64 = sqlx::query_scalar("INSERT INTO builders (builder_pubkey, first_seen_ms) VALUES (?, strftime('%s','now')*1000) RETURNING id")
                .bind(builder_pubkey)
                .fetch_one(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
                .await
                .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert builder: {}", e)))?;
            Ok(new_builder_id)
        }
    }

    // Helper function to get or create a proposer_id
    async fn get_or_create_proposer_id(tx: &mut sqlx::Transaction<'_, Sqlite>, proposer_pubkey: &str, fee_recipient: &str) -> Result<i64> {
        // Attempt to find existing proposer
        let existing_proposer_id: Option<i64> = sqlx::query_scalar("SELECT id FROM proposers WHERE proposer_pubkey = ?")
            .bind(proposer_pubkey)
            .fetch_optional(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to query proposer: {}", e)))?;

        if let Some(id) = existing_proposer_id {
            // TODO: Consider if we need to update fee_recipient if it changes for an existing proposer_pubkey.
            // For now, we assume it's fixed or the first one seen is used.
            Ok(id)
        } else {
            // Insert new proposer if not found
            let new_proposer_id: i64 = sqlx::query_scalar("INSERT INTO proposers (proposer_pubkey, fee_recipient_address, first_seen_ms) VALUES (?, ?, strftime('%s','now')*1000) RETURNING id")
                .bind(proposer_pubkey)
                .bind(fee_recipient)
                .fetch_one(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
                .await
                .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert proposer: {}", e)))?;
            Ok(new_proposer_id)
        }
    }

    // Helper function to upsert block information and get its id, tailored for BidTrace
    async fn upsert_block_and_get_id_from_bid(tx: &mut sqlx::Transaction<'_, Sqlite>, bid: &BidTrace) -> Result<i64> {
        let block_id: i64 = sqlx::query_scalar("
            INSERT INTO blocks (
                block_hash, parent_hash, slot, block_number, timestamp, timestamp_ms,
                num_tx, gas_limit, gas_used, received_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, strftime('%s','now')*1000)
            ON CONFLICT(block_hash) DO UPDATE SET
                parent_hash = COALESCE(excluded.parent_hash, parent_hash),
                slot = COALESCE(excluded.slot, slot),
                block_number = COALESCE(excluded.block_number, block_number),
                timestamp = COALESCE(excluded.timestamp, timestamp),
                timestamp_ms = COALESCE(excluded.timestamp_ms, timestamp_ms),
                num_tx = COALESCE(excluded.num_tx, num_tx),
                gas_limit = COALESCE(excluded.gas_limit, gas_limit),
                gas_used = COALESCE(excluded.gas_used, gas_used)
                -- received_at_ms is only set on initial insert
            RETURNING id;
        ")
        .bind(&bid.block_hash)
        .bind(&bid.parent_hash)
        .bind(bid.slot.to_string())
        .bind(bid.block_number.to_string())
        .bind(bid.timestamp.to_string())
        .bind(bid.timestamp_ms.to_string()) // This is specific to BidTrace
        .bind(bid.num_tx.to_string())
        .bind(bid.gas_limit.to_string())    // Specific to BidTrace
        .bind(bid.gas_used.to_string())     // Specific to BidTrace
        .fetch_one(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
        .await
        .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to upsert block: {}", e)))?;
        Ok(block_id)
    }

    // Helper function to upsert block information and get its id, tailored for DeliveredPayloadTrace
    async fn upsert_block_and_get_id_from_payload(tx: &mut sqlx::Transaction<'_, Sqlite>, payload: &DeliveredPayloadTrace) -> Result<i64> {
        let block_id: i64 = sqlx::query_scalar("
            INSERT INTO blocks (
                block_hash, parent_hash, slot, block_number, timestamp, num_tx, received_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, strftime('%s','now')*1000)
            ON CONFLICT(block_hash) DO UPDATE SET
                parent_hash = COALESCE(excluded.parent_hash, parent_hash),
                slot = COALESCE(excluded.slot, slot),
                block_number = COALESCE(excluded.block_number, block_number),
                timestamp = COALESCE(excluded.timestamp, timestamp),
                num_tx = COALESCE(excluded.num_tx, num_tx)
                -- Note: We do not update timestamp_ms, gas_limit, gas_used here,
                -- as DeliveredPayloadTrace doesn't have this info.
                -- This prevents nullifying data that might have come from a BidTrace.
            RETURNING id;
        ")
        .bind(&payload.block_hash)
        .bind(&payload.parent_hash)
        .bind(payload.slot.to_string())
        .bind(payload.block_number.to_string())
        .bind(payload.timestamp.to_string())
        .bind(payload.num_tx.to_string())
        .fetch_one(&mut **tx) // Pass &mut Deref<Target = SqliteConnection>
        .await
        .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to upsert block from payload: {}", e)))?;
        Ok(block_id)
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

        if bids_to_write.is_empty() {
            return Ok(());
        }
        info!(num_bids = bids_to_write.len(), "Flushing bid_traces batch to SQLite (new schema)");

        let mut tx = self.pool.begin().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to begin transaction for flushing bids: {}", e)))?;

        for bid_trace in bids_to_write.iter() {
            // 1. Get or create builder_id
            let builder_id = Self::get_or_create_builder_id(&mut tx, &bid_trace.builder_pubkey).await?;

            // 2. Get or create proposer_id
            let proposer_fee_recipient_str = format!("{:?}", bid_trace.proposer_fee_recipient);
            let proposer_id = Self::get_or_create_proposer_id(&mut tx, &bid_trace.proposer_pubkey, &proposer_fee_recipient_str).await?;

            // 3. Upsert block and get block_id
            let block_id = Self::upsert_block_and_get_id_from_bid(&mut tx, bid_trace).await?;

            // 4. Insert into bids table
            let received_at_ms = chrono::Utc::now().timestamp_millis(); // Capture current time for the bid record
            sqlx::query("
                INSERT OR IGNORE INTO bids (block_id, builder_id, proposer_id, value, received_at_ms)
                VALUES (?, ?, ?, ?, ?)
            ")
            .bind(block_id)
            .bind(builder_id)
            .bind(proposer_id)
            .bind(bid_trace.value.to_string())
            .bind(received_at_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert into bids table: {}", e)))?;
        }

        tx.commit().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to commit flushed bids transaction: {}", e)))?;
        info!(count = bids_to_write.len(), "SQLite bid_traces batch flushed successfully to new schema");
        Ok(())
    }

    // New method for delivered payloads
    #[instrument(skip(self, payload))]
    pub async fn write_delivered_payload(&self, payload: DeliveredPayloadTrace) -> Result<()> {
        debug!(block_hash = %payload.block_hash, value = %payload.value, "Writing delivered_payload to SQLite (new schema)");

        let mut tx = self.pool.begin().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to begin transaction for delivered payload: {}", e)))?;

        // 1. Get or create builder_id
        let builder_id = Self::get_or_create_builder_id(&mut tx, &payload.builder_pubkey).await?;

        // 2. Get or create proposer_id
        let proposer_fee_recipient_str = format!("{:?}", payload.proposer_fee_recipient);
        let proposer_id = Self::get_or_create_proposer_id(&mut tx, &payload.proposer_pubkey, &proposer_fee_recipient_str).await?;

        // 3. Upsert block and get block_id using the payload-specific helper
        let block_id = Self::upsert_block_and_get_id_from_payload(&mut tx, &payload).await?; // Pass by reference

        // 4. Insert into delivered_payloads table
        let received_at_ms = chrono::Utc::now().timestamp_millis();
        sqlx::query("
            INSERT OR IGNORE INTO delivered_payloads (block_id, builder_id, proposer_id, value, received_at_ms)
            VALUES (?, ?, ?, ?, ?)
        ")
        .bind(block_id)
        .bind(builder_id)
        .bind(proposer_id)
        .bind(payload.value.to_string())
        .bind(received_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to insert into delivered_payloads table: {}", e)))?;

        tx.commit().await.map_err(|e| BoostMonitorError::DatabaseError(format!("Failed to commit delivered payload transaction: {}", e)))?;

        info!(block_hash = %payload.block_hash, "Delivered payload written to SQLite (new schema)");
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
