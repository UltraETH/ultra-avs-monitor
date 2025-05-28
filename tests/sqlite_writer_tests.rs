#[cfg(test)]
mod sqlite_writer_tests {
    use alloy_primitives::{Address, U256};
    use sqlx::{Row, SqlitePool};
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::sleep;
    use ultra_avs_monitor::database::sqlite_writer::SqliteWriter;
    use ultra_avs_monitor::types::{BidTrace, DeliveredPayloadTrace}; // Added DeliveredPayloadTrace

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

    fn create_test_delivered_payload(block_num: u64, value: u64, builder_pubkey_suffix: &str) -> DeliveredPayloadTrace {
        DeliveredPayloadTrace {
            slot: U256::from(block_num),
            parent_hash: format!("parent_hash_{}", block_num),
            block_hash: format!("block_hash_{}_{}", block_num, builder_pubkey_suffix), // Ensure unique block_hash for linking
            builder_pubkey: format!("builder_pubkey_{}", builder_pubkey_suffix),
            proposer_pubkey: format!("proposer_pubkey_{}", block_num),
            proposer_fee_recipient: Address::ZERO,
            value: U256::from(value),
            block_number: U256::from(block_num),
            num_tx: U256::from(90u64), // Slightly different from bid for distinction
            timestamp: U256::from(1_617_979_455u64),
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
    async fn test_sqlite_writer_initialize_creates_tables_normalized() { // Renamed test
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_init_normalized.db") // New db name for clarity
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

        // Check new tables exist and are empty
        assert_eq!(count_rows(&pool, "builders").await, 0);
        assert_eq!(count_rows(&pool, "proposers").await, 0);
        assert_eq!(count_rows(&pool, "blocks").await, 0);
        assert_eq!(count_rows(&pool, "bids").await, 0);
        assert_eq!(count_rows(&pool, "delivered_payloads").await, 0);

        // Check old tables do not exist
        let old_bid_traces_count: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='bid_traces'")
            .fetch_one(&pool)
            .await
            .expect("Query to sqlite_master for bid_traces should not fail");
        assert_eq!(old_bid_traces_count, 0, "Old 'bid_traces' table should not exist");

        let old_delivered_payload_traces_count: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='delivered_payload_traces'")
            .fetch_one(&pool)
            .await
            .expect("Query to sqlite_master for delivered_payload_traces should not fail");
        assert_eq!(old_delivered_payload_traces_count, 0, "Old 'delivered_payload_traces' table should not exist");

        pool.close().await;
        writer.shutdown().await.unwrap(); // Shutdown should be clean with new schema
    }

    #[tokio::test]
    async fn test_write_bid_normalized() { // Renamed and refactored
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_write_bid_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(1)) // Batch size 1
            .await
            .unwrap();
        writer.initialize().await.unwrap();

        let test_bid = create_test_bid(100, 1000, "builder_xyz");
        writer.write_bid_trace(test_bid.clone()).await.unwrap(); // Assuming write_bid_trace is the new public method

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        // Verify counts in normalized tables
        assert_eq!(count_rows(&pool, "builders").await, 1, "Should be 1 builder");
        assert_eq!(count_rows(&pool, "proposers").await, 1, "Should be 1 proposer");
        assert_eq!(count_rows(&pool, "blocks").await, 1, "Should be 1 block");
        assert_eq!(count_rows(&pool, "bids").await, 1, "Should be 1 bid");

        // Verify builder data
        let builder_row = sqlx::query("SELECT builder_pubkey FROM builders WHERE builder_pubkey = ?")
            .bind(&test_bid.builder_pubkey)
            .fetch_one(&pool).await.unwrap();
        let builder_pubkey_db: String = builder_row.get("builder_pubkey");
        assert_eq!(builder_pubkey_db, test_bid.builder_pubkey);

        // Verify proposer data
        let proposer_row = sqlx::query("SELECT proposer_pubkey, fee_recipient_address FROM proposers WHERE proposer_pubkey = ?")
            .bind(&test_bid.proposer_pubkey)
            .fetch_one(&pool).await.unwrap();
        let proposer_pubkey_db: String = proposer_row.get("proposer_pubkey");
        let fee_recipient_db: String = proposer_row.get("fee_recipient_address");
        assert_eq!(proposer_pubkey_db, test_bid.proposer_pubkey);
        assert_eq!(fee_recipient_db, format!("{:?}", test_bid.proposer_fee_recipient));

        // Verify block data
        let block_row = sqlx::query("SELECT block_hash, slot, gas_limit, gas_used, timestamp_ms FROM blocks WHERE block_hash = ?")
            .bind(&test_bid.block_hash)
            .fetch_one(&pool).await.unwrap();
        let block_hash_db: String = block_row.get("block_hash");
        let slot_db: String = block_row.get("slot");
        let gas_limit_db: String = block_row.get("gas_limit");
        let gas_used_db: String = block_row.get("gas_used");
        let timestamp_ms_db: String = block_row.get("timestamp_ms");

        assert_eq!(block_hash_db, test_bid.block_hash);
        assert_eq!(slot_db, test_bid.slot.to_string());
        assert_eq!(gas_limit_db, test_bid.gas_limit.to_string());
        assert_eq!(gas_used_db, test_bid.gas_used.to_string());
        assert_eq!(timestamp_ms_db, test_bid.timestamp_ms.to_string());

        // Verify bid data (joining for more robust check is good, but direct check for now)
        let bid_row = sqlx::query("SELECT value FROM bids LIMIT 1") // Assuming only one bid
            .fetch_one(&pool).await.unwrap();
        let value_db: String = bid_row.get("value");
        assert_eq!(value_db, test_bid.value.to_string());

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_write_delivered_payload_normalized() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_write_payload_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(1))
            .await
            .unwrap();
        writer.initialize().await.unwrap();

        let test_payload = create_test_delivered_payload(150, 1500, "builder_payload_xyz");
        writer.write_delivered_payload(test_payload.clone()).await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        // Verify counts
        assert_eq!(count_rows(&pool, "builders").await, 1, "Should be 1 builder from payload");
        assert_eq!(count_rows(&pool, "proposers").await, 1, "Should be 1 proposer from payload");
        assert_eq!(count_rows(&pool, "blocks").await, 1, "Should be 1 block from payload");
        assert_eq!(count_rows(&pool, "delivered_payloads").await, 1, "Should be 1 delivered_payload");
        assert_eq!(count_rows(&pool, "bids").await, 0, "Should be 0 bids");


        // Verify builder data
        let builder_row = sqlx::query("SELECT builder_pubkey FROM builders WHERE builder_pubkey = ?")
            .bind(&test_payload.builder_pubkey)
            .fetch_one(&pool).await.unwrap();
        assert_eq!(builder_row.get::<String, _>("builder_pubkey"), test_payload.builder_pubkey);

        // Verify proposer data
        let proposer_row = sqlx::query("SELECT proposer_pubkey, fee_recipient_address FROM proposers WHERE proposer_pubkey = ?")
            .bind(&test_payload.proposer_pubkey)
            .fetch_one(&pool).await.unwrap();
        assert_eq!(proposer_row.get::<String, _>("proposer_pubkey"), test_payload.proposer_pubkey);
        assert_eq!(proposer_row.get::<String, _>("fee_recipient_address"), format!("{:?}", test_payload.proposer_fee_recipient));

        // Verify block data (fields specific to DeliveredPayloadTrace)
        let block_row = sqlx::query("SELECT block_hash, slot, num_tx, timestamp, gas_limit, gas_used, timestamp_ms FROM blocks WHERE block_hash = ?")
            .bind(&test_payload.block_hash)
            .fetch_one(&pool).await.unwrap();
        assert_eq!(block_row.get::<String, _>("block_hash"), test_payload.block_hash);
        assert_eq!(block_row.get::<String, _>("slot"), test_payload.slot.to_string());
        assert_eq!(block_row.get::<String, _>("num_tx"), test_payload.num_tx.to_string());
        assert_eq!(block_row.get::<String, _>("timestamp"), test_payload.timestamp.to_string());
        // Fields not in DeliveredPayloadTrace should be NULL in blocks table if only payload inserted
        assert!(block_row.get::<Option<String>, _>("gas_limit").is_none());
        assert!(block_row.get::<Option<String>, _>("gas_used").is_none());
        assert!(block_row.get::<Option<String>, _>("timestamp_ms").is_none());

        // Verify delivered_payloads data
        let payload_row = sqlx::query("SELECT value FROM delivered_payloads LIMIT 1")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(payload_row.get::<String, _>("value"), test_payload.value.to_string());

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_batch_write_normalized() { // Renamed and refactored
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_batch_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(60), Some(3)) // Batch size 3
            .await
            .unwrap();
        writer.initialize().await.unwrap();

        // Create bids with some distinct and some shared builder/proposer pubkeys for variety
        let bid1 = create_test_bid(200, 2000, "builder_1");
        let bid2 = create_test_bid(201, 2001, "builder_2"); // Different builder
        let bid3 = create_test_bid(202, 2002, "builder_1"); // Same builder as bid1
        let bid4 = create_test_bid(203, 2003, "builder_3");

        writer.write_bid_trace(bid1).await.unwrap();
        writer.write_bid_trace(bid2).await.unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        // Before flush (batch not full)
        assert_eq!(count_rows(&pool, "bids").await, 0, "Bids should not be flushed yet");
        assert_eq!(count_rows(&pool, "blocks").await, 0, "Blocks should not be created yet");
        assert_eq!(count_rows(&pool, "builders").await, 0, "Builders should not be created yet");
        assert_eq!(count_rows(&pool, "proposers").await, 0, "Proposers should not be created yet");

        writer.write_bid_trace(bid3).await.unwrap(); // This should trigger a flush (batch size 3)

        // After first flush (3 bids)
        assert_eq!(count_rows(&pool, "bids").await, 3, "Batch of 3 bids should be flushed");
        assert_eq!(count_rows(&pool, "blocks").await, 3, "3 blocks should be created"); // Each bid is for a new block_hash
        assert_eq!(count_rows(&pool, "builders").await, 2, "Should be 2 unique builders (builder_1, builder_2)");
        assert_eq!(count_rows(&pool, "proposers").await, 3, "Should be 3 unique proposers (one per block_num)");


        writer.write_bid_trace(bid4).await.unwrap(); // Bid 4 added to batch
        // Counts should remain same as bid4 is not yet flushed
        assert_eq!(count_rows(&pool, "bids").await, 3, "Bid 4 should be in batch, not flushed yet");
        assert_eq!(count_rows(&pool, "blocks").await, 3);
        assert_eq!(count_rows(&pool, "builders").await, 2);
        assert_eq!(count_rows(&pool, "proposers").await, 3);


        writer.flush_bid_traces().await.unwrap(); // Manual flush for the last bid (bid4)

        // After final flush
        assert_eq!(count_rows(&pool, "bids").await, 4, "All 4 bids should be flushed");
        assert_eq!(count_rows(&pool, "blocks").await, 4, "All 4 blocks should be created");
        assert_eq!(count_rows(&pool, "builders").await, 3, "Should be 3 unique builders (builder_1, builder_2, builder_3)");
        assert_eq!(count_rows(&pool, "proposers").await, 4, "Should be 4 unique proposers");

        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_auto_flush_normalized() { // Renamed and refactored
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_auto_flush_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(10)) // Flush interval 1 sec, batch size 10
            .await
            .unwrap();
        writer.initialize().await.unwrap();
        writer.start_flush_task().await.unwrap();

        let test_bid = create_test_bid(300, 3000, "builder_autoflush");
        writer.write_bid_trace(test_bid.clone()).await.unwrap();

        // Wait for auto-flush to occur (flush interval is 1s, wait for 2s to be safe)
        sleep(Duration::from_secs(2)).await;

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        assert_eq!(count_rows(&pool, "bids").await, 1, "Bid should be auto-flushed");
        assert_eq!(count_rows(&pool, "blocks").await, 1, "Block should be created by auto-flush");
        assert_eq!(count_rows(&pool, "builders").await, 1, "Builder should be created by auto-flush");
        assert_eq!(count_rows(&pool, "proposers").await, 1, "Proposer should be created by auto-flush");

        // Verify some data to be sure
        let bid_row = sqlx::query("SELECT value FROM bids WHERE value = ?")
            .bind(test_bid.value.to_string())
            .fetch_optional(&pool).await.unwrap();
        assert!(bid_row.is_some(), "Auto-flushed bid data not found or incorrect");

        pool.close().await;
        writer.shutdown().await.unwrap(); // This will also flush any remaining, but batch should be empty
    }

    #[tokio::test]
    async fn test_duplicate_handling_normalized() { // Renamed and refactored
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_duplicates_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(1), Some(5)) // Batch size 5, flush interval 1s
            .await
            .unwrap();
        writer.initialize().await.unwrap();
        // writer.start_flush_task().await.unwrap(); // Auto-flush can make exact count assertions tricky mid-test

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        // Scenario 1: Duplicate bids (same block_id, builder_id, value)
        let bid_orig = create_test_bid(400, 4000, "builder_dup_test");
        writer.write_bid_trace(bid_orig.clone()).await.unwrap();
        writer.write_bid_trace(bid_orig.clone()).await.unwrap(); // Exact duplicate
        writer.flush_bid_traces().await.unwrap(); // Flush the batch

        assert_eq!(count_rows(&pool, "bids").await, 1, "Exact duplicate bid should be ignored");
        assert_eq!(count_rows(&pool, "blocks").await, 1);
        assert_eq!(count_rows(&pool, "builders").await, 1);
        assert_eq!(count_rows(&pool, "proposers").await, 1);

        // Scenario 2: Bid for same block/builder but different value
        let bid_diff_value = BidTrace { value: U256::from(4001), ..bid_orig.clone() };
        writer.write_bid_trace(bid_diff_value.clone()).await.unwrap();
        writer.flush_bid_traces().await.unwrap();
        assert_eq!(count_rows(&pool, "bids").await, 2, "Bid with different value for same block/builder should be inserted");
        // Counts for blocks, builders, proposers should remain 1 as they are the same entities
        assert_eq!(count_rows(&pool, "blocks").await, 1);
        assert_eq!(count_rows(&pool, "builders").await, 1);
        assert_eq!(count_rows(&pool, "proposers").await, 1);


        // Scenario 3: Duplicate delivered payloads (same block_id)
        let payload_orig = create_test_delivered_payload(401, 4010, "builder_payload_dup");
        // Ensure block_hash is unique for this new scenario to avoid interference from previous block
        let payload_orig = DeliveredPayloadTrace { block_hash: "block_hash_401_payload_dup".to_string(), ..payload_orig };

        writer.write_delivered_payload(payload_orig.clone()).await.unwrap();
        writer.write_delivered_payload(payload_orig.clone()).await.unwrap(); // Exact duplicate payload for same block

        assert_eq!(count_rows(&pool, "delivered_payloads").await, 1, "Duplicate delivered payload for same block_id should be ignored");
        // A new block, builder, and proposer would have been created for this payload
        assert_eq!(count_rows(&pool, "blocks").await, 2); // block 400 and block 401
        assert_eq!(count_rows(&pool, "builders").await, 2); // builder_dup_test and builder_payload_dup
        assert_eq!(count_rows(&pool, "proposers").await, 2); // proposer for 400 and 401


        // Scenario 4: Block upsert logic - Bid first, then Payload for same block
        let bid_for_upsert = create_test_bid(402, 4020, "builder_upsert");
        writer.write_bid_trace(bid_for_upsert.clone()).await.unwrap();
        writer.flush_bid_traces().await.unwrap();

        assert_eq!(count_rows(&pool, "blocks").await, 3); // block 400, 401, 402
        let block_before_payload_row = sqlx::query("SELECT gas_limit, gas_used, timestamp_ms, num_tx FROM blocks WHERE block_hash = ?")
            .bind(&bid_for_upsert.block_hash).fetch_one(&pool).await.unwrap();
        assert_eq!(block_before_payload_row.get::<String, _>("gas_limit"), bid_for_upsert.gas_limit.to_string());
        assert_eq!(block_before_payload_row.get::<String, _>("num_tx"), bid_for_upsert.num_tx.to_string());


        let payload_for_upsert = DeliveredPayloadTrace {
            slot: bid_for_upsert.slot, // Same block identifiers
            parent_hash: bid_for_upsert.parent_hash.clone(),
            block_hash: bid_for_upsert.block_hash.clone(),
            builder_pubkey: bid_for_upsert.builder_pubkey.clone(), // Can be same or different builder
            proposer_pubkey: bid_for_upsert.proposer_pubkey.clone(),
            proposer_fee_recipient: bid_for_upsert.proposer_fee_recipient,
            value: U256::from(4021), // Different value for payload
            block_number: bid_for_upsert.block_number,
            num_tx: U256::from(95), // Potentially different num_tx from payload
            timestamp: bid_for_upsert.timestamp,
        };
        writer.write_delivered_payload(payload_for_upsert.clone()).await.unwrap();

        assert_eq!(count_rows(&pool, "blocks").await, 3, "Block count should remain 3 after upsert");
        assert_eq!(count_rows(&pool, "delivered_payloads").await, 2); // One from scenario 3, one new

        let block_after_payload_row = sqlx::query("SELECT gas_limit, gas_used, timestamp_ms, num_tx FROM blocks WHERE block_hash = ?")
            .bind(&bid_for_upsert.block_hash).fetch_one(&pool).await.unwrap();
        // Fields from original bid should persist if not in payload
        assert_eq!(block_after_payload_row.get::<String, _>("gas_limit"), bid_for_upsert.gas_limit.to_string(), "gas_limit from bid should persist");
        assert!(block_after_payload_row.get::<Option<String>, _>("gas_used").is_some(), "gas_used from bid should persist"); // Assuming it was set by bid
        assert!(block_after_payload_row.get::<Option<String>, _>("timestamp_ms").is_some(), "timestamp_ms from bid should persist");
        // num_tx should be updated from the payload's value if COALESCE logic is correct (excluded.num_tx)
        assert_eq!(block_after_payload_row.get::<String, _>("num_tx"), payload_for_upsert.num_tx.to_string(), "num_tx should be updated from payload");


        pool.close().await;
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_shutdown_flushes_normalized() { // Renamed and refactored
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_shutdown_flush_normalized.db")
            .to_str()
            .unwrap()
            .to_string();

        let writer = SqliteWriter::new(db_path.clone(), Some(60), Some(10)) // Long flush interval, batch size 10
            .await
            .unwrap();
        writer.initialize().await.unwrap();
        // Deliberately not starting the auto-flush task to isolate shutdown flush

        let test_bid = create_test_bid(500, 5000, "builder_shutdown");
        writer.write_bid_trace(test_bid.clone()).await.unwrap(); // Bid is in batch, not flushed

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();

        assert_eq!(count_rows(&pool, "bids").await, 0, "Bid should not be flushed yet");
        assert_eq!(count_rows(&pool, "blocks").await, 0, "Block should not be created yet");

        writer.shutdown().await.unwrap(); // Shutdown should trigger a flush of the current_batch

        // Verify data is flushed to new tables
        assert_eq!(count_rows(&pool, "bids").await, 1, "Bid should be flushed on shutdown");
        assert_eq!(count_rows(&pool, "blocks").await, 1, "Block should be created on shutdown flush");
        assert_eq!(count_rows(&pool, "builders").await, 1, "Builder should be created on shutdown flush");
        assert_eq!(count_rows(&pool, "proposers").await, 1, "Proposer should be created on shutdown flush");

        // Verify specific bid data
        let bid_row = sqlx::query("SELECT value FROM bids WHERE value = ?")
            .bind(test_bid.value.to_string())
            .fetch_optional(&pool).await.unwrap();
        assert!(bid_row.is_some(), "Shutdown-flushed bid data not found or incorrect");

        pool.close().await;
    }
}
