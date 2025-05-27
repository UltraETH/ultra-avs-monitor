#[cfg(test)]
mod bid_manager_tests {
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use ultra_avs_monitor::bid_manager::BidManager;
    use ultra_avs_monitor::types::BidTrace;
    use alloy_primitives::{Address, U256};

    fn create_test_bid(block_num: u64, value: u64, builder_pubkey_suffix: &str) -> BidTrace {
        BidTrace {
            slot: U256::from(block_num),
            parent_hash: format!("parent_hash_{}", block_num),
            block_hash: format!("block_hash_{}_{}", block_num, builder_pubkey_suffix),
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

    #[tokio::test]
    async fn test_prune_old_blocks() {
        let bid_manager = Arc::new(BidManager::new());

        // Add bids for blocks 100, 101, 200, 201
        bid_manager.add_bids(vec![
            create_test_bid(100, 10, "a"),
            create_test_bid(101, 11, "b"),
        ]).await;
        bid_manager.add_bids(vec![
            create_test_bid(200, 20, "c"),
            create_test_bid(201, 21, "d"),
        ]).await;

        // Check initial state
        assert_eq!(bid_manager.get_bids_for_block(U256::from(100)).await.len(), 1);
        assert!(bid_manager.get_highest_bid_for_block(U256::from(100)).await.is_some());
        assert_eq!(bid_manager.get_bids_for_block(U256::from(200)).await.len(), 1);
        assert!(bid_manager.get_highest_bid_for_block(U256::from(200)).await.is_some());

        // Prune blocks older than current_block_number - retention_blocks
        // Current block 201, retention 100. Threshold = 201 - 100 = 101.
        // Blocks < 101 (i.e., block 100) should be pruned.
        bid_manager.prune_old_blocks(U256::from(201), 100).await;

        assert_eq!(bid_manager.get_bids_for_block(U256::from(100)).await.len(), 0, "Block 100 should be pruned");
        assert!(bid_manager.get_highest_bid_for_block(U256::from(100)).await.is_none(), "Highest bid for block 100 should be pruned");

        assert_eq!(bid_manager.get_bids_for_block(U256::from(101)).await.len(), 1, "Block 101 should NOT be pruned");
        assert!(bid_manager.get_highest_bid_for_block(U256::from(101)).await.is_some(), "Highest bid for block 101 should NOT be pruned");

        assert_eq!(bid_manager.get_bids_for_block(U256::from(200)).await.len(), 1, "Block 200 should NOT be pruned");
        assert!(bid_manager.get_highest_bid_for_block(U256::from(200)).await.is_some(), "Highest bid for block 200 should NOT be pruned");

        assert_eq!(bid_manager.get_bids_for_block(U256::from(201)).await.len(), 1, "Block 201 should NOT be pruned");
        assert!(bid_manager.get_highest_bid_for_block(U256::from(201)).await.is_some(), "Highest bid for block 201 should NOT be pruned");


        // Prune with retention 0 (should do nothing if logic is to skip)
        // Or if it means retain 0 old blocks, it would prune everything older than current.
        // Current implementation: retention_blocks == 0 skips pruning.
        bid_manager.prune_old_blocks(U256::from(201), 0).await;
        assert_eq!(bid_manager.get_bids_for_block(U256::from(101)).await.len(), 1, "Block 101 should still exist after retention 0");


        // Prune with large retention (should not prune anything recent)
        bid_manager.prune_old_blocks(U256::from(201), 500).await;
         assert_eq!(bid_manager.get_bids_for_block(U256::from(101)).await.len(), 1, "Block 101 should still exist with large retention");


        // Prune making current block the threshold (e.g. current 101, retention 1, threshold 100)
        bid_manager.add_bids(vec![create_test_bid(50, 5, "e")]).await;
        bid_manager.prune_old_blocks(U256::from(101), 1).await; // Threshold = 100
        assert_eq!(bid_manager.get_bids_for_block(U256::from(50)).await.len(), 0, "Block 50 should be pruned");
    }

    #[tokio::test]
    async fn test_add_bids_and_subscriptions() {
        let bid_manager = Arc::new(BidManager::new());
        let (ws_tx, mut ws_rx) = mpsc::channel(10);
        bid_manager.set_websocket_sender(ws_tx).await;

        let mut top_bid_rx = bid_manager.subscribe_to_top_bids().await;
        let mut new_bid_rx = bid_manager.subscribe_to_all_new_bids().await;

        let bid1_block1 = create_test_bid(1, 100, "a");
        let bid2_block1 = create_test_bid(1, 200, "b"); // Higher value for same block
        let bid1_block2 = create_test_bid(2, 50, "c");

        bid_manager.add_bids(vec![bid1_block1.clone()]).await;

        // Check new_bid_rx and ws_rx
        assert_eq!(new_bid_rx.recv().await.unwrap().value, bid1_block1.value);
        assert_eq!(ws_rx.recv().await.unwrap().value, bid1_block1.value);
        // Check top_bid_rx
        assert_eq!(top_bid_rx.recv().await.unwrap().value, bid1_block1.value);


        bid_manager.add_bids(vec![bid2_block1.clone()]).await;
         // Check new_bid_rx and ws_rx for bid2_block1
        assert_eq!(new_bid_rx.recv().await.unwrap().value, bid2_block1.value);
        assert_eq!(ws_rx.recv().await.unwrap().value, bid2_block1.value);
        // Check top_bid_rx for bid2_block1 (new highest for block 1)
        assert_eq!(top_bid_rx.recv().await.unwrap().value, bid2_block1.value);


        bid_manager.add_bids(vec![bid1_block2.clone()]).await;
        // Check new_bid_rx and ws_rx for bid1_block2
        assert_eq!(new_bid_rx.recv().await.unwrap().value, bid1_block2.value);
        assert_eq!(ws_rx.recv().await.unwrap().value, bid1_block2.value);
        // Check top_bid_rx for bid1_block2 (new highest for block 2)
        assert_eq!(top_bid_rx.recv().await.unwrap().value, bid1_block2.value);

        // Check internal state
        assert_eq!(bid_manager.get_highest_bid_for_block(U256::from(1)).await.unwrap().value, bid2_block1.value);
        assert_eq!(bid_manager.get_bids_for_block(U256::from(1)).await.len(), 2);
        assert_eq!(bid_manager.get_highest_bid_for_block(U256::from(2)).await.unwrap().value, bid1_block2.value);
    }
}
