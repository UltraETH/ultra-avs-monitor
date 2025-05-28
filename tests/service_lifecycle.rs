use alloy_primitives::U256;
use ultra_avs_monitor::test_helpers::TestServer;

#[tokio::test]
async fn test_service_startup_and_shutdown() {
    let test_server = TestServer::new().await;

    let bid_manager = test_server.get_bid_manager();
    assert!(bid_manager
        .as_ref()
        .get_bids_for_block(U256::from(1))
        .await
        .is_empty());

    let _ = test_server.shutdown().await;
    assert!(bid_manager
        .as_ref()
        .get_bids_for_block(U256::from(1))
        .await
        .is_empty());
}

#[tokio::test]
async fn test_bid_manager_reset() {
    let test_server = TestServer::new().await;
    let bid_manager = test_server.get_bid_manager();

    assert!(bid_manager
        .as_ref()
        .get_bids_for_block(U256::from(1))
        .await
        .is_empty());

    let _ = bid_manager.clear_all().await;

    assert!(bid_manager
        .as_ref()
        .get_bids_for_block(U256::from(1))
        .await
        .is_empty());

    let _ = test_server.shutdown().await;
}
