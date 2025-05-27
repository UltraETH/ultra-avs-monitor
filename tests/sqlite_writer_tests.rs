#[cfg(test)]
mod sqlite_writer_tests {
    use alloy_primitives::{Address, U256};
    use sqlx::{Row, SqlitePool};
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::sleep;
    use ultra_avs_monitor::database::sqlite_writer::SqliteWriter;
    use ultra_avs_monitor::types::BidTrace;

    fn create_test_bid(block_num: u64, value: u64, builder_pubkey_suffix: &str) -> BidTrace {
        BidTrace {
            slot: U256::from(block_num),
            parent_hash: format!("parent_hash_{}", block_num),
            block_hash: format!("block_hash_{}_{}", block_num, builder_pubkey_suffix), // Ensure unique block_hash
            builder_pubkey: format!("builder_pubkey_{}", builder_pubkey_suffix),
            proposer_pubkey: format!("proposer_pubkey_{}", block_num),
            proposer_fee_recipient: Address::ZERO,
            gas_limit: U256::from(30_000_000u64),
            gas_used: U256::from(10_000_000u64),
            value: U256::from(value),
            block_number: U256::from(block_num),
            num_tx: U256::from(100u64),
            timestamp: U256::from(1_617_979_455u64),
            timestamp_ms: U256::from(1_617_979_455_000u64),
        }
    }

    async fn count_rows(pool: &SqlitePool, table_name: &str) -> i64 {
        let row = sqlx::query(&format!("SELECT COUNT(*) FROM {}", table_name))
            .fetch_one(pool)
            .await
            .unwrap();
        row.get(0)
    }

    #[tokio::test]
    async fn test_sqlite_writer_initialize_creates_table() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_init.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(5))
            .await
            .unwrap();
        writer.initialize().await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 0);
        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_writer_write_and_flush_bid() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_write_flush.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(1))
            .await
            .unwrap(); // Batch size 1 for immediate flush
        writer.initialize().await.unwrap();

        let bid1 = create_test_bid(100, 1000, "a");
        writer.write_bid(bid1.clone()).await.unwrap();
        // write_bid should auto-flush due to batch_size = 1

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 1);

        let row = sqlx::query("SELECT block_hash, value FROM bid_traces WHERE block_hash = ?")
            .bind(&bid1.block_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
        let block_hash_db: String = row.get("block_hash");
        let value_db: String = row.get("value");
        assert_eq!(block_hash_db, bid1.block_hash);
        assert_eq!(value_db, bid1.value.to_string());

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_writer_batch_write() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_batch.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(60), Some(3))
            .await
            .unwrap(); // Batch size 3
        writer.initialize().await.unwrap();

        let bid1 = create_test_bid(200, 2000, "a");
        let bid2 = create_test_bid(201, 2001, "b");
        let bid3 = create_test_bid(202, 2002, "c");
        let bid4 = create_test_bid(203, 2003, "d");

        writer.write_bid(bid1).await.unwrap();
        writer.write_bid(bid2).await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let mut row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 0, "Bids should not be flushed yet");

        writer.write_bid(bid3).await.unwrap(); // This should trigger a flush
        row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 3, "Batch of 3 should be flushed");

        writer.write_bid(bid4).await.unwrap();
        row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 3, "Bid 4 should be in batch, not flushed yet");

        writer.flush().await.unwrap(); // Manual flush for the last bid
        row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 4, "All bids should be flushed");

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_writer_auto_flush_task() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_auto_flush.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(10))
            .await
            .unwrap(); // Flush interval 1 sec
        writer.initialize().await.unwrap();
        writer.start_flush_task().await.unwrap();

        let bid1 = create_test_bid(300, 3000, "a");
        writer.write_bid(bid1).await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        // Wait for auto-flush to occur
        sleep(Duration::from_secs(2)).await;

        let row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(
            row_count, 1,
            "Bid should be auto-flushed by background task"
        );

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_writer_ignore_duplicate_block_hash() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_duplicates.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(1))
            .await
            .unwrap();
        writer.initialize().await.unwrap();

        let bid1 = create_test_bid(400, 4000, "a");
        let bid_duplicate = create_test_bid(400, 4001, "a"); // Same block_hash as bid1, different value

        writer.write_bid(bid1.clone()).await.unwrap();
        writer.write_bid(bid_duplicate).await.unwrap(); // Should be ignored due to PRIMARY KEY constraint

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 1);

        let row = sqlx::query("SELECT value FROM bid_traces WHERE block_hash = ?")
            .bind(&bid1.block_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
        let value_db: String = row.get("value");
        assert_eq!(
            value_db,
            bid1.value.to_string(),
            "Original bid's value should be present"
        );

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_sqlite_writer_shutdown_flushes_remaining() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_shutdown_flush.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(60), Some(10))
            .await
            .unwrap(); // Long flush interval
        writer.initialize().await.unwrap();
        // Not starting flush task to test manual shutdown flush

        let bid1 = create_test_bid(500, 5000, "a");
        writer.write_bid(bid1).await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let mut row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 0, "Bid should not be flushed yet");

        writer.shutdown().await.unwrap(); // Shutdown should trigger a flush

        row_count = count_rows(&pool, "bid_traces").await;
        assert_eq!(row_count, 1, "Bid should be flushed on shutdown");

        pool.close().await;
    }
}
