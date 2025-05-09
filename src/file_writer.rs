use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, instrument, warn};

use crate::errors::{BoostMonitorError, Result};
use crate::types::BidTrace;

const DEFAULT_MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const MAX_BATCH_SIZE: usize = 10000;

pub struct FileWriter {
    output_path: String,
    file: Arc<Mutex<Option<File>>>,
    batch_size: usize,
    current_batch: Arc<RwLock<Vec<BidTrace>>>,
    flush_interval: Duration,
    max_file_size: u64,
    error_sender: mpsc::Sender<BoostMonitorError>,
    error_receiver: Arc<Mutex<mpsc::Receiver<BoostMonitorError>>>,
}

impl FileWriter {
    /// Creates a new FileWriter that writes to the specified path
    pub fn new(output_path: String, flush_interval_secs: u64, batch_size: usize) -> Self {
        // Ensure batch size doesn't exceed maximum
        let safe_batch_size = batch_size.min(MAX_BATCH_SIZE);
        if safe_batch_size != batch_size {
            warn!(
                "Requested batch size {} exceeds maximum, using {}",
                batch_size, safe_batch_size
            );
        }

        // Create channel for error communication
        let (error_sender, error_receiver) = mpsc::channel(100);

        Self {
            output_path,
            file: Arc::new(Mutex::new(None)),
            batch_size: safe_batch_size,
            current_batch: Arc::new(RwLock::new(Vec::with_capacity(safe_batch_size))),
            flush_interval: Duration::from_secs(flush_interval_secs),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            error_sender,
            error_receiver: Arc::new(Mutex::new(error_receiver)),
        }
    }

    /// Creates a new FileWriter with custom configuration
    pub fn with_config(
        output_path: String,
        flush_interval_secs: u64,
        batch_size: usize,
        max_file_size: u64,
    ) -> Self {
        let mut writer = Self::new(output_path, flush_interval_secs, batch_size);
        writer.max_file_size = max_file_size;
        writer
    }

    /// Opens the file for writing, creating it if it doesn't exist
    #[instrument(skip(self))]
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing file writer");
        let path = Path::new(&self.output_path);

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| BoostMonitorError::IoError(e))?;
                info!("Created directory structure for output path");
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(path)
            .map_err(|e| BoostMonitorError::IoError(e))?;

        let mut file_guard = self.file.lock().await;
        *file_guard = Some(file);

        info!("File writer initialized successfully: {}", self.output_path);
        Ok(())
    }

    /// Start a background task that periodically flushes the batch to disk
    #[instrument(skip(self))]
    pub async fn start_flush_task(&self) -> Result<()> {
        info!("Starting background flush task");
        // Make sure file is initialized
        if self.file.lock().await.is_none() {
            warn!("File not initialized before starting flush task, initializing now.");
            self.initialize().await?;
        }

        let file_writer = self.clone();
        let error_sender = self.error_sender.clone();

        tokio::spawn(async move {
            let mut flush_interval = interval(file_writer.flush_interval);
            let mut consecutive_errors = 0;
            let max_consecutive_errors = 5;
            let mut backoff_duration = Duration::from_millis(100);

            info!(
                "Background flush task started with interval: {:?}",
                file_writer.flush_interval
            );

            loop {
                flush_interval.tick().await;
                debug!("Flush interval ticked");

                match file_writer.flush().await {
                    Ok(_) => {
                        // Reset error counter on success
                        if consecutive_errors > 0 {
                            info!("Flush succeeded after previous failures");
                            consecutive_errors = 0;
                            backoff_duration = Duration::from_millis(100);
                        }
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        error!(
                            "Error flushing data to file in background task (attempt {}): {}",
                            consecutive_errors, e
                        );

                        // Send error to main application
                        if let Err(send_err) = error_sender.send(e).await {
                            error!("Failed to send error to main application: {}", send_err);
                        }

                        // Apply backoff if we have consecutive errors
                        if consecutive_errors > 1 {
                            let backoff = backoff_duration.min(Duration::from_secs(30));
                            warn!(
                                "Applying backoff of {:?} before next flush attempt",
                                backoff
                            );
                            tokio::time::sleep(backoff).await;
                            // Exponential backoff (capped at 30 seconds)
                            backoff_duration = backoff_duration.mul_f32(1.5);
                        }

                        // If too many consecutive errors, try to rotate or recreate the file
                        if consecutive_errors >= max_consecutive_errors {
                            error!("Too many consecutive flush errors, attempting to rotate file");
                            if let Err(rotate_err) = file_writer.rotate_file().await {
                                error!("Failed to rotate file: {}", rotate_err);
                            } else {
                                info!("File rotated successfully after errors");
                                consecutive_errors = 0;
                            }
                        }
                    }
                }

                // Check if we need to rotate the file based on size
                if let Err(e) = file_writer.check_file_size().await {
                    error!("Error checking file size: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Check if the current file exceeds the maximum size and rotate if needed
    async fn check_file_size(&self) -> Result<()> {
        let file_guard = self.file.lock().await;
        if let Some(file) = file_guard.as_ref() {
            // Get file metadata to check size
            let metadata = file.metadata().map_err(|e| BoostMonitorError::IoError(e))?;
            let file_size = metadata.len();

            if file_size > self.max_file_size {
                debug!(
                    "File size {} exceeds maximum {}, will rotate",
                    file_size, self.max_file_size
                );
                // We need to drop the guard before calling rotate_file
                drop(file_guard);
                self.rotate_file().await?;
            }
        }
        Ok(())
    }

    /// Rotate the current file to a backup and create a new file
    async fn rotate_file(&self) -> Result<()> {
        info!("Rotating file {}", self.output_path);

        // Flush any pending data first
        self.flush().await?;

        // Close the current file
        let mut file_guard = self.file.lock().await;
        *file_guard = None;

        // Rename the current file with timestamp
        let now = chrono::Local::now();
        let timestamp = now.format("%Y%m%d_%H%M%S");
        let path = Path::new(&self.output_path);

        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");

        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("log");

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let rotated_filename = format!("{}_{}.{}", file_stem, timestamp, extension);
        let rotated_path = parent.join(rotated_filename);

        // Rename the file
        if path.exists() {
            std::fs::rename(path, &rotated_path).map_err(|e| BoostMonitorError::IoError(e))?;
            info!("Rotated file to {}", rotated_path.display());
        }

        // Create a new file
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(path)
            .map_err(|e| BoostMonitorError::IoError(e))?;

        *file_guard = Some(file);
        info!("Created new file after rotation");

        Ok(())
    }

    /// Add a bid to the current batch, flushing if batch size is reached
    #[instrument(skip(self, bid))]
    pub async fn write_bid(&self, bid: BidTrace) -> Result<()> {
        let mut batch = self.current_batch.write().await;
        debug!("Adding bid to batch with value: {}", bid.value);

        batch.push(bid);

        // Flush if we've reached the batch size
        if batch.len() >= self.batch_size {
            drop(batch); // Release the lock before flushing
            self.flush().await?;
        }

        Ok(())
    }

    /// Write multiple bids at once, optimizing for batch operations
    #[instrument(skip(self, bids), fields(num_bids = bids.len()))]
    pub async fn write_bids(&self, bids: Vec<BidTrace>) -> Result<()> {
        if bids.is_empty() {
            debug!("write_bids called with empty vector, skipping");
            return Ok(());
        }
        info!("Writing batch of bids: {}", bids.len());

        // Process bids in chunks of batch_size to avoid excessive memory usage
        for chunk in bids.chunks(self.batch_size) {
            let mut batch = self.current_batch.write().await;
            batch.extend(chunk.iter().cloned());

            if batch.len() >= self.batch_size {
                // We have a full batch, flush it
                drop(batch); // Release the lock before flushing
                self.flush().await?;
            }
            // Otherwise, just continue to the next chunk
        }

        Ok(())
    }

    /// Force writing all pending bids to disk
    #[instrument(skip(self))]
    pub async fn flush(&self) -> Result<()> {
        let mut batch = self.current_batch.write().await;
        if batch.is_empty() {
            debug!("Flush called but batch is empty, skipping write");
            return Ok(());
        }
        info!(num_bids = batch.len(), "Flushing bid batch to disk");

        // Take ownership of the batch and replace with empty
        let bids_to_write = std::mem::replace(&mut *batch, Vec::with_capacity(self.batch_size));
        // Release the batch lock early
        drop(batch);

        // Now write the bids to file
        let mut file_guard = self.file.lock().await;
        let file = file_guard.as_mut().ok_or_else(|| {
            BoostMonitorError::ConfigError("File writer not initialized".to_string())
        })?;

        for bid in bids_to_write.iter() {
            // Write bid as a JSON line for easy parsing
            let json = serde_json::to_string(&bid)
                .map_err(|e| BoostMonitorError::SerializationError(e))?;
            writeln!(file, "{}", json).map_err(|e| BoostMonitorError::IoError(e))?;
        }

        // Ensure data is written to disk
        file.flush().map_err(|e| BoostMonitorError::IoError(e))?;
        info!(count = bids_to_write.len(), "Batch flushed successfully");

        Ok(())
    }
    /// Close the file writer, ensuring all data is flushed
    #[instrument(skip(self))]
    pub async fn close(&self) -> Result<()> {
        info!("Closing file writer, flushing remaining data");
        // Flush any remaining data
        self.flush().await?;

        // Close the file
        let mut file_guard = self.file.lock().await;
        *file_guard = None;
        info!("File writer closed");

        Ok(())
    }

    /// Check for any errors reported by the background task
    pub async fn check_for_errors(&self) -> Option<BoostMonitorError> {
        let mut receiver = self.error_receiver.lock().await;
        receiver.try_recv().ok()
    }
}

impl Clone for FileWriter {
    fn clone(&self) -> Self {
        Self {
            output_path: self.output_path.clone(),
            file: self.file.clone(),
            batch_size: self.batch_size,
            current_batch: self.current_batch.clone(),
            flush_interval: self.flush_interval,
            max_file_size: self.max_file_size,
            error_sender: self.error_sender.clone(),
            error_receiver: self.error_receiver.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BidTrace;
    use alloy_primitives::{Address, U256};
    use std::fs;
    use tempfile::tempdir;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_file_writer_basic() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_bids.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer
        let writer = FileWriter::new(file_path_str, 1, 5);
        writer.initialize().await.unwrap();

        // Create a test bid
        let bid = create_test_bid(1000, 1000);

        // Write the bid and flush
        // Write the bid and flush
        writer.write_bid(bid).await.unwrap();
        writer.flush().await.unwrap();

        // Read the file contents
        let contents = fs::read_to_string(&file_path).map_err(|e| BoostMonitorError::IoError(e))?;
        assert!(!contents.is_empty());
        assert!(contents.contains("1000"));

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_file_writer_batch() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_bids_batch.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer with a batch size of 3
        let writer = FileWriter::new(file_path_str, 1, 3);
        writer.initialize().await.unwrap();

        // Create test bids
        let bids = vec![
            create_test_bid(1001, 1001),
            create_test_bid(1002, 1002),
            create_test_bid(1003, 1003),
            create_test_bid(1004, 1004),
        ];

        // Write bids
        writer.write_bids(bids).await.unwrap();

        // Ensure the first 3 were auto-flushed due to batch size
        let contents = fs::read_to_string(&file_path).map_err(|e| BoostMonitorError::IoError(e))?;
        assert!(contents.contains("1001"));
        assert!(contents.contains("1002"));
        assert!(contents.contains("1003"));

        // Flush the remaining bid
        writer.flush().await.unwrap();

        // Verify all bids are in the file
        let contents = fs::read_to_string(&file_path).map_err(|e| BoostMonitorError::IoError(e))?;
        assert!(contents.contains("1004"));

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_file_writer_auto_flush() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_bids_auto_flush.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer with a short flush interval (1 second)
        let writer = FileWriter::new(file_path_str, 1, 10);
        writer.initialize().await.unwrap();
        writer.start_flush_task().await.unwrap();

        // Create a test bid
        let bid = create_test_bid(2000, 2000);

        // Write the bid but don't manually flush
        writer.write_bid(bid).await.unwrap();

        // Wait for auto-flush
        sleep(Duration::from_secs(2)).await;

        // Verify the bid was written
        // Verify the bid was written
        let contents = fs::read_to_string(&file_path)?;
        assert!(contents.contains("2000"));

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_file_rotation() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_rotation.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer with a small max file size to trigger rotation
        let writer = FileWriter::with_config(file_path_str.clone(), 1, 5, 100); // 100 bytes max
        writer.initialize().await.unwrap();

        // Write enough bids to exceed the file size limit
        for i in 0..10 {
            let bid = create_test_bid(3000 + i, 3000 + i);
            writer.write_bid(bid).await.unwrap();
            writer.flush().await.unwrap();
        }

        // Manually trigger rotation
        writer.rotate_file().await.unwrap();

        // Check that the original file exists and is empty or small
        let metadata = fs::metadata(&file_path)?;
        assert!(
            metadata.len() < 100,
            "New file should be small after rotation"
        );

        // Check that at least one rotated file exists
        let parent_dir = dir.path();
        let entries = fs::read_dir(parent_dir).map_err(|e| BoostMonitorError::IoError(e))?;
        let rotated_files: Vec<_> = entries
            .filter_map(|r| r.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("test_rotation_") && name.ends_with(".json")
            })
            .collect();

        assert!(
            !rotated_files.is_empty(),
            "Should have at least one rotated file"
        );

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_error_handling() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_errors.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer
        let writer = FileWriter::new(file_path_str, 1, 5);
        writer.initialize().await.unwrap();

        // No errors should be reported initially
        assert!(writer.check_for_errors().await.is_none());

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_large_batch_handling() -> Result<()> {
        // Create a temporary directory
        let dir = tempdir()?;
        let file_path = dir.path().join("test_large_batch.json");
        let file_path_str = file_path.to_str().unwrap().to_string();

        // Create file writer with a small batch size
        let writer = FileWriter::new(file_path_str, 1, 5);
        writer.initialize().await.unwrap();

        // Create a large batch of bids (more than batch size)
        let large_batch: Vec<_> = (0..20)
            .map(|i| create_test_bid(4000 + i, 4000 + i))
            .collect();

        // Write the large batch
        writer.write_bids(large_batch).await.unwrap();

        // Verify all bids were written (should have been flushed in chunks)
        let contents = fs::read_to_string(&file_path)?;
        for i in 0..20 {
            assert!(
                contents.contains(&format!("4{:03}", i)),
                "Should contain bid {}",
                4000 + i
            );
        }

        // Close the writer
        writer.close().await.unwrap();

        Ok(())
    }

    // Helper function to create a test bid
    fn create_test_bid(block_num: u64, value: u64) -> BidTrace {
        BidTrace {
            slot: U256::from(block_num),
            block_number: U256::from(block_num),
            parent_hash: "0x123456".to_string(),
            block_hash: "0xabcdef".to_string(),
            builder_pubkey: format!("pubkey_{}", block_num),
            proposer_pubkey: "proposer_1".to_string(),
            proposer_fee_recipient: Address::ZERO,
            gas_limit: U256::from(30000000u64),
            gas_used: U256::from(10000000u64),
            num_tx: U256::from(100u64),
            timestamp: U256::from(1617979455u64),
            timestamp_ms: U256::from(1617979455000u64),
            value: U256::from(value),
        }
    }
}
