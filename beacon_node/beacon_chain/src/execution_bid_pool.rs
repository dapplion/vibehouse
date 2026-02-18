//! Provides an `ExecutionBidPool` that stores full verified `SignedExecutionPayloadBid` objects
//! for bid selection during block production.
//!
//! When a proposer produces a Gloas block, they need to select the best bid to include.
//! This pool stores all verified bids so the proposer can choose the highest-value bid.
//!
//! Only one bid per builder per slot is stored (the first valid one, per equivocation rules).
//! Old slots are pruned automatically.

use std::collections::HashMap;
use types::{BuilderIndex, EthSpec, SignedExecutionPayloadBid, Slot};

/// Maximum number of slots to retain. Bids are only useful for current/next slot,
/// but we keep a small buffer for edge cases around slot boundaries.
const MAX_BID_POOL_SLOTS: u64 = 4;

/// A pool of verified execution payload bids available for block production.
///
/// Structure: Slot -> BuilderIndex -> SignedExecutionPayloadBid
pub struct ExecutionBidPool<E: EthSpec> {
    bids: HashMap<Slot, HashMap<BuilderIndex, SignedExecutionPayloadBid<E>>>,
}

impl<E: EthSpec> Default for ExecutionBidPool<E> {
    fn default() -> Self {
        Self {
            bids: HashMap::new(),
        }
    }
}

impl<E: EthSpec> ExecutionBidPool<E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a verified bid into the pool.
    ///
    /// Only stores one bid per (slot, builder_index). If a bid from this builder
    /// already exists for this slot, it is not replaced (equivocation is rejected
    /// at the gossip validation layer).
    pub fn insert(&mut self, bid: SignedExecutionPayloadBid<E>) {
        let slot = bid.message.slot;
        let builder_index = bid.message.builder_index;

        self.bids
            .entry(slot)
            .or_default()
            .entry(builder_index)
            .or_insert(bid);
    }

    /// Get the best (highest value) bid for a given slot.
    ///
    /// Returns `None` if no external bids are available for this slot.
    pub fn get_best_bid(&self, slot: Slot) -> Option<&SignedExecutionPayloadBid<E>> {
        self.bids.get(&slot).and_then(|slot_bids| {
            slot_bids
                .values()
                .max_by_key(|bid| bid.message.value)
        })
    }

    /// Remove all bids older than `current_slot - MAX_BID_POOL_SLOTS`.
    pub fn prune(&mut self, current_slot: Slot) {
        let earliest = Slot::new(current_slot.as_u64().saturating_sub(MAX_BID_POOL_SLOTS));
        self.bids.retain(|&slot, _| slot >= earliest);
    }

    /// Returns the number of bids stored for a given slot.
    #[cfg(test)]
    pub fn bid_count_for_slot(&self, slot: Slot) -> usize {
        self.bids.get(&slot).map_or(0, |m| m.len())
    }

    /// Returns the total number of bids across all slots.
    #[cfg(test)]
    pub fn total_bid_count(&self) -> usize {
        self.bids.values().map(|m| m.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ExecutionBlockHash, ExecutionPayloadBid, Hash256, MainnetEthSpec, Signature};

    type E = MainnetEthSpec;

    fn make_bid(slot: u64, builder_index: u64, value: u64) -> SignedExecutionPayloadBid<E> {
        SignedExecutionPayloadBid {
            message: ExecutionPayloadBid {
                slot: Slot::new(slot),
                builder_index,
                value,
                parent_block_hash: ExecutionBlockHash::zero(),
                parent_block_root: Hash256::zero(),
                block_hash: ExecutionBlockHash(Hash256::zero()),
                prev_randao: Hash256::zero(),
                fee_recipient: Default::default(),
                gas_limit: 30_000_000,
                execution_payment: value,
                blob_kzg_commitments: Default::default(),
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn best_bid_selects_highest_value() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));
        pool.insert(make_bid(10, 2, 500));
        pool.insert(make_bid(10, 3, 200));

        let best = pool.get_best_bid(Slot::new(10)).unwrap();
        assert_eq!(best.message.value, 500);
        assert_eq!(best.message.builder_index, 2);
    }

    #[test]
    fn no_bids_returns_none() {
        let pool = ExecutionBidPool::<E>::new();
        assert!(pool.get_best_bid(Slot::new(10)).is_none());
    }

    #[test]
    fn does_not_replace_existing_bid_from_same_builder() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));
        // Second bid from same builder should be ignored (equivocation handled elsewhere)
        pool.insert(make_bid(10, 1, 999));

        let best = pool.get_best_bid(Slot::new(10)).unwrap();
        assert_eq!(best.message.value, 100); // First bid kept
        assert_eq!(pool.bid_count_for_slot(Slot::new(10)), 1);
    }

    #[test]
    fn pruning_removes_old_slots() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(1, 1, 100));
        pool.insert(make_bid(5, 2, 200));
        pool.insert(make_bid(10, 3, 300));

        pool.prune(Slot::new(10));

        // Slots 1 and 5 are older than 10 - 4 = 6
        assert_eq!(pool.total_bid_count(), 1);
        assert!(pool.get_best_bid(Slot::new(1)).is_none());
        assert!(pool.get_best_bid(Slot::new(5)).is_none());
        assert!(pool.get_best_bid(Slot::new(10)).is_some());
    }
}
