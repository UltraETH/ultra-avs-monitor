use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use alloy_primitives::U256; // Ensure U256 is imported and U64 is not
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    RwLock,
};
use tracing::{debug, info};

use crate::errors::Result;
use crate::types::BidTrace;

#[derive(Clone)]
pub struct BidManager {
    // Store all unique bids received, grouped by block number (using U256)
    all_bids: Arc<RwLock<HashMap<U256, HashSet<BidTrace>>>>,
    // Store the highest bid for each block number (using U256)
    highest_bids: Arc<RwLock<HashMap<U256, BidTrace>>>,
    // Subscribers for the highest bid updates (per block)
    top_bid_subscribers: Arc<RwLock<Vec<Sender<BidTrace>>>>,
    // Subscribers for all new unique bids received
    new_bid_subscribers: Arc<RwLock<Vec<Sender<BidTrace>>>>,
    // Sender to the WebSocket server for broadcasting new bids
    websocket_sender: Arc<RwLock<Option<Sender<BidTrace>>>>,
}

impl BidManager {
    pub fn new() -> Self {
        Self {
            all_bids: Arc::new(RwLock::new(HashMap::new())),
            highest_bids: Arc::new(RwLock::new(HashMap::new())),
            top_bid_subscribers: Arc::new(RwLock::new(Vec::new())),
            new_bid_subscribers: Arc::new(RwLock::new(Vec::new())),
            websocket_sender: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn set_websocket_sender(&self, sender: Sender<BidTrace>) {
        let mut websocket_sender = self.websocket_sender.write().await;
        *websocket_sender = Some(sender);
        info!("WebSocket sender set in BidManager");
    }

    // Adds new bids, stores unique ones, updates highest bid per block, and notifies subscribers
    pub async fn add_bids(&self, new_bids: Vec<BidTrace>) {
        if new_bids.is_empty() {
            debug!("No new bids to add");
            return;
        }

        let mut all_bids_guard = self.all_bids.write().await;
        let mut highest_bids_guard = self.highest_bids.write().await;
        let new_bid_subscribers_guard = self.new_bid_subscribers.read().await;
        let top_bid_subscribers_guard = self.top_bid_subscribers.read().await;
        let websocket_sender_guard = self.websocket_sender.read().await;

        debug!("Adding {} new bids", new_bids.len());

        for bid in new_bids {
            let block_number: U256 = bid.block_number; // Explicitly U256

            // Get or insert the HashSet for this block number
            let block_bids = all_bids_guard
                .entry(block_number)
                .or_insert_with(HashSet::new);

            // Insert the bid into the HashSet. If it's new, proceed.
            if block_bids.insert(bid.clone()) {
                debug!(block = %block_number, bid = ?bid, "Added new unique bid");

                // Notify all_new_bid subscribers
                for subscriber in &*new_bid_subscribers_guard {
                    let _ = subscriber.try_send(bid.clone());
                }

                // Send to WebSocket if sender is available
                if let Some(sender) = &*websocket_sender_guard {
                    let _ = sender.try_send(bid.clone());
                }

                // Check if this is the new highest bid for this block
                let current_highest = highest_bids_guard.get(&block_number);

                if current_highest.is_none() || bid.value > current_highest.unwrap().value {
                    debug!(block = %block_number, new_highest = ?bid, "New highest bid for block");
                    highest_bids_guard.insert(block_number, bid.clone());

                    // Notify top_bid subscribers
                    for subscriber in &*top_bid_subscribers_guard {
                        let _ = subscriber.try_send(bid.clone());
                    }
                }
            } else {
                debug!(block = %block_number, bid = ?bid, "Bid is a duplicate, skipping");
            }
        }
    }

    // Gets all unique bids for a specific block number (parameter changed to U256)
    pub async fn get_bids_for_block(&self, block_num: U256) -> Vec<BidTrace> {
        let all_bids_guard = self.all_bids.read().await;
        all_bids_guard
            .get(&block_num)
            .map(|bids| bids.iter().cloned().collect())
            .unwrap_or_else(Vec::new)
    }

    // Gets the highest bid for a specific block number (parameter changed to U256)
    pub async fn get_highest_bid_for_block(&self, block_num: U256) -> Option<BidTrace> {
        let highest_bids_guard = self.highest_bids.read().await;
        highest_bids_guard.get(&block_num).cloned()
    }

    // Clears all bids and highest bids
    pub async fn clear_all(&self) -> Result<()> {
        let mut all_bids_guard = self.all_bids.write().await;
        let mut highest_bids_guard = self.highest_bids.write().await;

        all_bids_guard.clear();
        highest_bids_guard.clear();

        info!("Cleared all bids and highest bids");

        Ok(())
    }

    // Subscribes to updates for the highest bid for any block
    pub async fn subscribe_to_top_bids(&self) -> Receiver<BidTrace> {
        let (tx, rx) = mpsc::channel(100);
        let mut subscribers_guard = self.top_bid_subscribers.write().await;
        subscribers_guard.push(tx);
        info!("New subscriber for top bids");
        rx
    }

    // Subscribes to all new unique bids received
    pub async fn subscribe_to_all_new_bids(&self) -> Receiver<BidTrace> {
        let (tx, rx) = mpsc::channel(100);
        let mut subscribers_guard = self.new_bid_subscribers.write().await;
        subscribers_guard.push(tx);
        info!("New subscriber for all new bids");
        rx
    }

    pub async fn prune_old_blocks(&self, current_block_number: U256, retention_blocks: u64) {
        if retention_blocks == 0 {
            debug!("Block retention is disabled (retention_blocks = 0), skipping pruning.");
            return;
        }

        let threshold_block = if current_block_number.to::<u64>() > retention_blocks {
            current_block_number - U256::from(retention_blocks)
        } else {
            U256::ZERO
        };

        let mut all_bids_guard = self.all_bids.write().await;
        let mut highest_bids_guard = self.highest_bids.write().await;

        let mut removed_all_bids = 0;
        let mut removed_highest_bids = 0;

        all_bids_guard.retain(|&block_num, _| {
            if block_num < threshold_block {
                removed_all_bids += 1;
                false
            } else {
                true
            }
        });

        highest_bids_guard.retain(|&block_num, _| {
            if block_num < threshold_block {
                removed_highest_bids += 1;
                false
            } else {
                true
            }
        });

        if removed_all_bids > 0 || removed_highest_bids > 0 {
            info!(
                current_block = %current_block_number,
                threshold_block = %threshold_block,
                removed_all_bids,
                removed_highest_bids,
                "Pruned old blocks from BidManager"
            );
        } else {
            debug!(
                current_block = %current_block_number,
                threshold_block = %threshold_block,
                "No old blocks to prune in BidManager"
            );
        }
    }
}
