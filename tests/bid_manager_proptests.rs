#![cfg(test)]

use ultra_avs_monitor::bid_manager::BidManager;
use ultra_avs_monitor::types::BidTrace;
use alloy_primitives::{Address, U256};

// Helper to create test bids
fn create_test_bid(value: u64, block_num: u64) -> BidTrace {
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

#[tokio::test]
async fn test_highest_bid_is_correct() {
    let bid_manager = BidManager::new();

    // Create some test bids
    let bid1 = create_test_bid(1000, 100);
    let bid2 = create_test_bid(2000, 101);
    let bid3 = create_test_bid(1500, 102);

    let bids = vec![bid1.clone(), bid2.clone(), bid3.clone()];

    // Add bids to manager
    bid_manager.add_bids(bids.clone()).await;

    // Check highest bid for block 101
    let highest_bid = bid_manager.get_highest_bid_for_block(U256::from(101)).await;

    // The highest bid should be bid2 (value 2000)
    assert!(highest_bid.is_some());
    assert_eq!(highest_bid.as_ref().unwrap().value, U256::from(2000u64));
}

#[tokio::test]
async fn test_all_unique_bids_stored() {
    let bid_manager = BidManager::new();

    // Create some test bids, including duplicates
    let bid1 = create_test_bid(1000, 100);
    let bid2 = create_test_bid(2000, 101);
    let bid1_dup = create_test_bid(1000, 100); // Duplicate of bid1

    let bids = vec![bid1.clone(), bid2.clone(), bid1_dup.clone()];

    // Add bids to manager
    bid_manager.add_bids(bids.clone()).await;

    // Get bids for each block
    let bids_100 = bid_manager.get_bids_for_block(U256::from(100)).await;
    let bids_101 = bid_manager.get_bids_for_block(U256::from(101)).await;

    // Should contain 1 unique bid for block 100 and 1 for block 101
    assert_eq!(bids_100.len(), 1);
    assert_eq!(bids_101.len(), 1);

    // Highest bid should be bid2 (for block 101)
    let highest_bid = bid_manager.get_highest_bid_for_block(U256::from(101)).await;
    assert!(highest_bid.is_some());
    assert_eq!(highest_bid.as_ref().unwrap().value, U256::from(2000u64));
}

#[tokio::test]
async fn test_clear_all() {
    let bid_manager = BidManager::new();

    // Create and add some test bids
    let bid1 = create_test_bid(1000, 100);
    let bid2 = create_test_bid(2000, 101);

    let bids = vec![bid1.clone(), bid2.clone()];
    bid_manager.add_bids(bids).await;

    // Verify bids are there for specific blocks
    assert_eq!(bid_manager.get_bids_for_block(U256::from(100)).await.len(), 1);
    assert_eq!(bid_manager.get_bids_for_block(U256::from(101)).await.len(), 1);
    assert!(bid_manager.get_highest_bid_for_block(U256::from(100)).await.is_some());
    assert!(bid_manager.get_highest_bid_for_block(U256::from(101)).await.is_some());

    // Clear all bids
    let clear_result = bid_manager.clear_all().await;
    assert!(clear_result.is_ok());

    // Verify bids are gone for specific blocks
    assert!(bid_manager.get_bids_for_block(U256::from(100)).await.is_empty());
    assert!(bid_manager.get_bids_for_block(U256::from(101)).await.is_empty());
    assert!(bid_manager.get_highest_bid_for_block(U256::from(100)).await.is_none());
    assert!(bid_manager.get_highest_bid_for_block(U256::from(101)).await.is_none());
}
