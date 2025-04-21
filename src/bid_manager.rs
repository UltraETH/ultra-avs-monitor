use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashSet},
    sync::Arc,
};

use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    RwLock,
};

use crate::types::BidTrace;
use crate::errors::Result;

// Manages (sort, organize) all bids given by relays
#[derive(Clone)]
pub struct BidManager {
    highest_bid: Arc<RwLock<Option<BidTrace>>>,
    all_bids: Arc<RwLock<BinaryHeap<Reverse<BidTrace>>>>,
    unique_bids: Arc<RwLock<HashSet<BidTrace>>>,
    top_bid_subscribers: Arc<RwLock<Vec<Sender<BidTrace>>>>,
    new_bid_subscribers: Arc<RwLock<Vec<Sender<BidTrace>>>>,
    // WebSocket sender for streaming bids
    websocket_sender: Arc<RwLock<Option<Sender<BidTrace>>>>,
}

impl BidManager {
    pub fn new() -> Self {
        Self {
            highest_bid: Arc::new(RwLock::new(None)),
            all_bids: Arc::new(RwLock::new(BinaryHeap::new())),
            unique_bids: Arc::new(RwLock::new(HashSet::new())),
            top_bid_subscribers: Arc::new(RwLock::new(Vec::new())),
            new_bid_subscribers: Arc::new(RwLock::new(Vec::new())),
            websocket_sender: Arc::new(RwLock::new(None)),
        }
    }

    // Set WebSocket sender for streaming bids
    pub async fn set_websocket_sender(&self, sender: Sender<BidTrace>) {
        let mut websocket_sender = self.websocket_sender.write().await;
        *websocket_sender = Some(sender);
    }

    pub async fn add_bids(&self, new_bids: Vec<BidTrace>) {
        if new_bids.is_empty() {
            return;
        }

        // Acquire all locks at once to prevent deadlocks
        let mut all_bids_guard = self.all_bids.write().await;
        let mut highest_bid_guard = self.highest_bid.write().await;
        let mut unique_bids_guard = self.unique_bids.write().await;
        let top_bid_subscribers_guard = self.top_bid_subscribers.read().await;
        let new_bid_subscribers_guard = self.new_bid_subscribers.read().await;
        let websocket_sender_guard = self.websocket_sender.read().await;

        // Track if we have a new highest bid
        let mut highest_changed = false;
        let mut highest_bid_value = highest_bid_guard.as_ref().map(|b| b.value).unwrap_or_default();
        let mut new_highest_bid = None;

        for bid in new_bids {
            // Skip if we've already seen this bid
            if !unique_bids_guard.insert(bid.clone()) {
                continue;
            }
            
            // Add to sorted heap
            all_bids_guard.push(Reverse(bid.clone()));
            
            // Send to new bid subscribers
            for subscriber in &*new_bid_subscribers_guard {
                let _ = subscriber.try_send(bid.clone()); // Non-blocking send
            }
            
            // Send to WebSocket clients
            if let Some(sender) = &*websocket_sender_guard {
                let _ = sender.try_send(bid.clone()); // Non-blocking send
            }
            
            // Check if this is a new highest bid
            if bid.value > highest_bid_value {
                highest_bid_value = bid.value;
                new_highest_bid = Some(bid.clone());
                highest_changed = true;
            }
        }

        // Update highest bid if changed
        if highest_changed && new_highest_bid.is_some() {
            *highest_bid_guard = new_highest_bid.clone();
            
            // Notify highest bid subscribers
            if let Some(highest_bid) = new_highest_bid {
                for subscriber in &*top_bid_subscribers_guard {
                    let _ = subscriber.try_send(highest_bid.clone()); // Non-blocking send
                }
            }
        }
    }

    pub async fn get_bids(&self) -> Vec<BidTrace> {
        let all_bids_guard = self.all_bids.read().await;
        all_bids_guard.iter().map(|r| r.0.clone()).collect()
    }

    pub async fn get_highest_bid(&self) -> Option<BidTrace> {
        let highest_bid_guard = self.highest_bid.read().await;
        highest_bid_guard.clone()
    }

    pub async fn clear_all(&self) -> Result<()> {
        let mut all_bids_guard = self.all_bids.write().await;
        let mut highest_bid_guard = self.highest_bid.write().await;
        let mut unique_bids_guard = self.unique_bids.write().await;

        all_bids_guard.clear();
        unique_bids_guard.clear();
        *highest_bid_guard = None;
        
        Ok(())
    }

    // Subscribe to new top block bids
    pub async fn subscribe_to_top_bids(&self) -> Receiver<BidTrace> {
        let (tx, rx) = mpsc::channel(100);
        let mut subscribers_guard = self.top_bid_subscribers.write().await;
        subscribers_guard.push(tx);
        rx
    }

    // Subscribe to all new block bids
    pub async fn subscribe_to_all_new_bids(&self) -> Receiver<BidTrace> {
        let (tx, rx) = mpsc::channel(100);
        let mut subscribers_guard = self.new_bid_subscribers.write().await;
        subscribers_guard.push(tx);
        rx
    }
}
