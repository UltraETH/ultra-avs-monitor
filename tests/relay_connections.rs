use alloy_primitives::{Address, U256};
use ultra_avs_monitor::test_helpers::TestServer;

#[tokio::test]
async fn test_bid_processing_flow() {
    let test_server = TestServer::new().await;
    let bid_manager = test_server.get_bid_manager();

    let block_num_1000 = U256::from(1000u64);
    assert!(bid_manager
        .get_bids_for_block(block_num_1000)
        .await
        .is_empty());

    let bid = ultra_avs_monitor::types::BidTrace {
        slot: U256::from(1000u64),
        block_number: U256::from(1000u64),
        parent_hash: "0x123456".to_string(),
        block_hash: "0xabcdef".to_string(),
        builder_pubkey: String::from("pub_key_1"),
        proposer_pubkey: String::from("proposer_1"),
        proposer_fee_recipient: Address::ZERO,
        gas_limit: U256::from(30000000u64),
        gas_used: U256::from(10000000u64),
        num_tx: U256::from(100u64),
        timestamp: U256::from(1617979455u64),
        timestamp_ms: U256::from(1617979455000u64),
        value: U256::from(1000u64),
    };

    bid_manager.add_bids(vec![bid.clone()]).await;

    let all_bids = bid_manager.get_bids_for_block(block_num_1000).await;
    assert_eq!(all_bids.len(), 1);

    let _ = test_server.shutdown().await;
}

#[tokio::test]
async fn test_clearing_bids() {
    let test_server = TestServer::new().await;
    let bid_manager = test_server.get_bid_manager();

    let block_num_1000 = U256::from(1000u64);
    assert!(bid_manager
        .get_bids_for_block(block_num_1000)
        .await
        .is_empty());

    let bid = ultra_avs_monitor::types::BidTrace {
        slot: U256::from(1000u64),
        block_number: U256::from(1000u64),
        parent_hash: "0x123456".to_string(),
        block_hash: "0xabcdef".to_string(),
        builder_pubkey: String::from("pub_key_1"),
        proposer_pubkey: String::from("proposer_1"),
        proposer_fee_recipient: Address::ZERO,
        gas_limit: U256::from(30000000u64),
        gas_used: U256::from(10000000u64),
        num_tx: U256::from(100u64),
        timestamp: U256::from(1617979455u64),
        timestamp_ms: U256::from(1617979455000u64),
        value: U256::from(1000u64),
    };

    bid_manager.add_bids(vec![bid.clone()]).await;
    assert_eq!(
        bid_manager.get_bids_for_block(block_num_1000).await.len(),
        1
    );

    let _ = bid_manager.clear_all().await;
    assert!(bid_manager
        .get_bids_for_block(block_num_1000)
        .await
        .is_empty());

    let _ = test_server.shutdown().await;
}

#[tokio::test]
async fn test_processing_multiple_bids() {
    let test_server = TestServer::new().await;
    let bid_manager = test_server.get_bid_manager();

    let bid1 = ultra_avs_monitor::types::BidTrace {
        slot: U256::from(1000u64),
        block_number: U256::from(1000u64),
        parent_hash: "0x123456".to_string(),
        block_hash: "0xabcdef".to_string(),
        builder_pubkey: String::from("pub_key_1"),
        proposer_pubkey: String::from("proposer_1"),
        proposer_fee_recipient: Address::ZERO,
        gas_limit: U256::from(30000000u64),
        gas_used: U256::from(10000000u64),
        num_tx: U256::from(100u64),
        timestamp: U256::from(1617979455u64),
        timestamp_ms: U256::from(1617979455000u64),
        value: U256::from(1000u64),
    };

    let bid2 = ultra_avs_monitor::types::BidTrace {
        slot: U256::from(1000u64),
        block_number: U256::from(1000u64),
        parent_hash: "0x123456".to_string(),
        block_hash: "0xfedcba".to_string(),
        builder_pubkey: String::from("pub_key_2"),
        proposer_pubkey: String::from("proposer_2"),
        proposer_fee_recipient: Address::ZERO,
        gas_limit: U256::from(30000000u64),
        gas_used: U256::from(10000000u64),
        num_tx: U256::from(120u64),
        timestamp: U256::from(1617979456u64),
        timestamp_ms: U256::from(1617979456000u64),
        value: U256::from(2000u64),
    };

    bid_manager.add_bids(vec![bid1.clone(), bid2.clone()]).await;

    let block_num_1000 = U256::from(1000u64);
    assert_eq!(
        bid_manager.get_bids_for_block(block_num_1000).await.len(),
        2
    );

    let highest = bid_manager.get_highest_bid_for_block(block_num_1000).await;
    assert!(highest.is_some());
    assert_eq!(highest.unwrap().value, U256::from(2000u64));

    let _ = test_server.shutdown().await;
}
