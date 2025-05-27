use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::{
    database::sqlite_writer::SqliteWriter,
    errors::Result,
    file_writer::FileWriter, // Optional: if we also write delivered payloads to a file
    types::DeliveredPayloadTrace,
};

// Configuration for where to send delivered payloads
pub struct DeliveredPayloadSinks {
    pub sqlite_writer: Option<Arc<SqliteWriter>>,
    pub file_writer: Option<Arc<FileWriter>>, // Optional
}

#[derive(Clone)]
pub struct DeliveredPayloadManager {
    sinks: Arc<DeliveredPayloadSinks>,
}

impl DeliveredPayloadManager {
    pub fn new(sinks: DeliveredPayloadSinks) -> Self {
        Self {
            sinks: Arc::new(sinks),
        }
    }

    pub async fn add_payloads(&self, payloads: Vec<DeliveredPayloadTrace>) {
        if payloads.is_empty() {
            debug!("No new delivered payloads to add.");
            return;
        }
        info!("Received {} new delivered payloads.", payloads.len());

        for payload in payloads {
            debug!(block_hash = %payload.block_hash, value = %payload.value, "Processing delivered payload");

            if let Some(ref sqlite_writer) = self.sinks.sqlite_writer {
                // In a real scenario, SqliteWriter would have a method like `write_delivered_payload`
                // For now, we'll adapt by creating a new method or directly inserting.
                // This part needs SqliteWriter to be extended.
                if let Err(e) = sqlite_writer.write_delivered_payload(payload.clone()).await {
                    error!(error = %e, payload = ?payload, "Failed to write delivered payload to SQLite");
                }
            }

            if let Some(ref _file_writer) = self.sinks.file_writer {
                // File writing for delivered payloads is not implemented in this phase.
                // If needed, FileWriter would be extended similarly to SqliteWriter.
                // For now, we just acknowledge it might exist.
                // Example:
                // if let Err(e) = file_writer.write_delivered_payload_to_file(payload.clone()).await {
                //     error!(error = %e, payload = ?payload, "Failed to write delivered payload to file");
                // }
            }
        }
    }
}
