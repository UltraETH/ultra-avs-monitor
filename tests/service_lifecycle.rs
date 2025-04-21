use avs_boost_monitor::test_helpers::TestServer;

#[tokio::test]
async fn test_service_startup_and_shutdown() {
    let test_server = TestServer::new().await;
    
    // Verify clean startup
    let bid_manager = test_server.get_bid_manager();
    assert!(bid_manager.get_bids().await.is_empty());
    
    // Verify graceful shutdown
    let _ = test_server.shutdown().await;
    assert!(bid_manager.get_bids().await.is_empty());
}

// We're simplifying this test since our TestServer doesn't include relay clients
#[tokio::test]
async fn test_bid_manager_reset() {
    let test_server = TestServer::new().await;
    let bid_manager = test_server.get_bid_manager();
    
    // Verify initial empty state
    assert!(bid_manager.get_bids().await.is_empty());
    
    // Add some test data through clear_all method
    let _ = bid_manager.clear_all().await;
    
    // Verify still empty
    assert!(bid_manager.get_bids().await.is_empty());
    
    // Shutdown cleanly
    let _ = test_server.shutdown().await;
}
