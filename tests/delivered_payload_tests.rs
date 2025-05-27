#[cfg(test)]
mod delivered_payload_tests {
    use alloy_primitives::{Address, U256};
    use sqlx::{Row, SqlitePool};
    use std::sync::Arc;
    use tempfile::tempdir;
    use ultra_avs_monitor::{
        database::sqlite_writer::SqliteWriter,
        delivered_payload_manager::{DeliveredPayloadManager, DeliveredPayloadSinks},
        test_helpers::create_test_bid_trace, // We'll adapt this or create a new one
        types::DeliveredPayloadTrace,
    };

    // Helper to create a DeliveredPayloadTrace (adapt from BidTrace or create new)
    fn create_test_delivered_payload(
        slot: u64,
        value: u64,
        block_hash_suffix: &str,
    ) -> DeliveredPayloadTrace {
        DeliveredPayloadTrace {
            slot: U256::from(slot),
            parent_hash: format!("payload_parent_hash_{}", slot),
            block_hash: format!("payload_block_hash_{}_{}", slot, block_hash_suffix),
            builder_pubkey: format!("payload_builder_pubkey_{}", slot),
            proposer_pubkey: format!("payload_proposer_pubkey_{}", slot),
            proposer_fee_recipient: Address::ZERO,
            value: U256::from(value),
            block_number: U256::from(slot), // Assuming slot is block_number for simplicity
            num_tx: U256::from(50u64),
            timestamp: U256::from(1_700_000_000u64 + slot),
        }
    }

    async fn count_delivered_payload_rows(pool: &SqlitePool) -> i64 {
        let row = sqlx::query("SELECT COUNT(*) FROM delivered_payload_traces")
            .fetch_one(pool)
            .await
            .unwrap();
        row.get(0)
    }

    #[tokio::test]
    async fn test_write_delivered_payload_to_sqlite() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_delivered_payloads.db")
            .to_str()
            .unwrap()
            .to_string();

        let sqlite_writer = Arc::new(
            SqliteWriter::new(db_path.clone(), Some(1), Some(1))
                .await
                .unwrap(),
        );
        sqlite_writer.initialize().await.unwrap(); // This now creates both tables

        let sinks = DeliveredPayloadSinks {
            sqlite_writer: Some(sqlite_writer.clone()),
            file_writer: None,
        };
        let payload_manager = DeliveredPayloadManager::new(sinks);

        let payload1 = create_test_delivered_payload(1001, 10000, "a");
        payload_manager.add_payloads(vec![payload1.clone()]).await;

        // SqliteWriter::write_delivered_payload writes directly, no batching for now
        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let row_count = count_delivered_payload_rows(&pool).await;
        assert_eq!(
            row_count, 1,
            "Delivered payload should be written to SQLite"
        );

        let row = sqlx::query(
            "SELECT block_hash, value FROM delivered_payload_traces WHERE block_hash = ?",
        )
        .bind(&payload1.block_hash)
        .fetch_one(&pool)
        .await
        .unwrap();
        let block_hash_db: String = row.get("block_hash");
        let value_db: String = row.get("value");
        assert_eq!(block_hash_db, payload1.block_hash);
        assert_eq!(value_db, payload1.value.to_string());

        pool.close().await;
        sqlite_writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_add_multiple_delivered_payloads() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_multi_delivered_payloads.db")
            .to_str()
            .unwrap()
            .to_string();
        let sqlite_writer = Arc::new(
            SqliteWriter::new(db_path.clone(), Some(1), Some(5))
                .await
                .unwrap(),
        );
        sqlite_writer.initialize().await.unwrap();

        let sinks = DeliveredPayloadSinks {
            sqlite_writer: Some(sqlite_writer.clone()),
            file_writer: None,
        };
        let manager = DeliveredPayloadManager::new(sinks);

        let payloads = vec![
            create_test_delivered_payload(2001, 200, "a"),
            create_test_delivered_payload(2002, 201, "b"),
            create_test_delivered_payload(2003, 202, "c"),
        ];
        manager.add_payloads(payloads).await;

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let count = count_delivered_payload_rows(&pool).await;
        assert_eq!(count, 3);
        pool.close().await;
        sqlite_writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_ignore_duplicate_delivered_payloads() {
        let dir = tempdir().unwrap();
        let db_path = dir
            .path()
            .join("test_dup_delivered_payloads.db")
            .to_str()
            .unwrap()
            .to_string();
        let sqlite_writer = Arc::new(
            SqliteWriter::new(db_path.clone(), Some(1), Some(1))
                .await
                .unwrap(),
        );
        sqlite_writer.initialize().await.unwrap();

        let sinks = DeliveredPayloadSinks {
            sqlite_writer: Some(sqlite_writer.clone()),
            file_writer: None,
        };
        let manager = DeliveredPayloadManager::new(sinks);

        let payload1 = create_test_delivered_payload(3001, 300, "a");
        let payload_dup = create_test_delivered_payload(3002, 301, "a"); // Same block_hash as payload1 due to suffix "a"

        manager.add_payloads(vec![payload1.clone()]).await;
        manager.add_payloads(vec![payload_dup.clone()]).await; // Should be ignored

        let pool = SqlitePool::connect(&format!("sqlite:{}", db_path))
            .await
            .unwrap();
        let count = count_delivered_payload_rows(&pool).await;
        assert_eq!(count, 1, "Duplicate delivered payload should be ignored");

        let row = sqlx::query("SELECT value FROM delivered_payload_traces WHERE block_hash = ?")
            .bind(&payload1.block_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
        let value_db: String = row.get("value");
        assert_eq!(
            value_db,
            payload1.value.to_string(),
            "Original payload's value should be present"
        );

        pool.close().await;
        sqlite_writer.shutdown().await.unwrap();
    }
}
