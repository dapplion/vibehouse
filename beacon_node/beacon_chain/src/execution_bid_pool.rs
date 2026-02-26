//! Provides an `ExecutionBidPool` that stores full verified `SignedExecutionPayloadBid` objects
//! for bid selection during block production.
//!
//! When a proposer produces a Gloas block, they need to select the best bid to include.
//! This pool stores all verified bids so the proposer can choose the highest-value bid.
//!
//! Only one bid per builder per slot is stored (the first valid one, per equivocation rules).
//! Old slots are pruned automatically.

use std::collections::HashMap;
use types::{BuilderIndex, EthSpec, Hash256, SignedExecutionPayloadBid, Slot};

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

    /// Get the best (highest value) bid for a given slot and parent block root.
    ///
    /// Only returns bids whose `parent_block_root` matches, ensuring stale bids
    /// from before a re-org are not selected.
    /// Returns `None` if no matching external bids are available.
    pub fn get_best_bid(
        &self,
        slot: Slot,
        parent_block_root: Hash256,
    ) -> Option<&SignedExecutionPayloadBid<E>> {
        self.bids.get(&slot).and_then(|slot_bids| {
            slot_bids
                .values()
                .filter(|bid| bid.message.parent_block_root == parent_block_root)
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
    use types::{
        ExecutionBlockHash, ExecutionPayloadBid, FixedBytesExtended, Hash256, MainnetEthSpec,
        Signature,
    };

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

        let best = pool.get_best_bid(Slot::new(10), Hash256::zero()).unwrap();
        assert_eq!(best.message.value, 500);
        assert_eq!(best.message.builder_index, 2);
    }

    #[test]
    fn no_bids_returns_none() {
        let pool = ExecutionBidPool::<E>::new();
        assert!(pool.get_best_bid(Slot::new(10), Hash256::zero()).is_none());
    }

    #[test]
    fn does_not_replace_existing_bid_from_same_builder() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));
        // Second bid from same builder should be ignored (equivocation handled elsewhere)
        pool.insert(make_bid(10, 1, 999));

        let best = pool.get_best_bid(Slot::new(10), Hash256::zero()).unwrap();
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
        assert!(pool.get_best_bid(Slot::new(1), Hash256::zero()).is_none());
        assert!(pool.get_best_bid(Slot::new(5), Hash256::zero()).is_none());
        assert!(pool.get_best_bid(Slot::new(10), Hash256::zero()).is_some());
    }

    #[test]
    fn best_bid_per_slot_independent() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));
        pool.insert(make_bid(10, 2, 500));
        pool.insert(make_bid(11, 3, 50));
        pool.insert(make_bid(11, 4, 200));

        assert_eq!(
            pool.get_best_bid(Slot::new(10), Hash256::zero())
                .unwrap()
                .message
                .value,
            500
        );
        assert_eq!(
            pool.get_best_bid(Slot::new(11), Hash256::zero())
                .unwrap()
                .message
                .value,
            200
        );
    }

    #[test]
    fn wrong_slot_returns_none() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));

        assert!(pool.get_best_bid(Slot::new(11), Hash256::zero()).is_none());
        assert!(pool.get_best_bid(Slot::new(9), Hash256::zero()).is_none());
        assert!(pool.get_best_bid(Slot::new(0), Hash256::zero()).is_none());
    }

    #[test]
    fn prune_boundary_slot_retained() {
        let mut pool = ExecutionBidPool::<E>::new();
        // MAX_BID_POOL_SLOTS = 4, so prune(10) keeps slots >= 6
        pool.insert(make_bid(6, 1, 100));
        pool.insert(make_bid(5, 2, 200));

        pool.prune(Slot::new(10));

        // Slot 6 is at the boundary (10 - 4 = 6) — retained
        assert!(pool.get_best_bid(Slot::new(6), Hash256::zero()).is_some());
        // Slot 5 is below the boundary — pruned
        assert!(pool.get_best_bid(Slot::new(5), Hash256::zero()).is_none());
    }

    #[test]
    fn prune_at_zero_keeps_everything() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(0, 1, 100));
        pool.insert(make_bid(1, 2, 200));

        pool.prune(Slot::new(0));

        assert_eq!(pool.total_bid_count(), 2);
    }

    #[test]
    fn single_builder_is_best() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 42));

        let best = pool.get_best_bid(Slot::new(10), Hash256::zero()).unwrap();
        assert_eq!(best.message.value, 42);
        assert_eq!(best.message.builder_index, 1);
    }

    #[test]
    fn insert_then_prune_then_insert() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(1, 1, 100));
        pool.prune(Slot::new(10));
        assert_eq!(pool.total_bid_count(), 0);

        pool.insert(make_bid(10, 2, 500));
        assert_eq!(pool.total_bid_count(), 1);
        assert_eq!(
            pool.get_best_bid(Slot::new(10), Hash256::zero())
                .unwrap()
                .message
                .value,
            500
        );
    }

    #[test]
    fn many_builders_same_slot() {
        let mut pool = ExecutionBidPool::<E>::new();
        for i in 0..100 {
            pool.insert(make_bid(10, i, i * 10));
        }

        assert_eq!(pool.bid_count_for_slot(Slot::new(10)), 100);
        let best = pool.get_best_bid(Slot::new(10), Hash256::zero()).unwrap();
        assert_eq!(best.message.value, 990); // 99 * 10
    }

    #[test]
    fn equal_value_bids_returns_one() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));
        pool.insert(make_bid(10, 2, 100));
        pool.insert(make_bid(10, 3, 100));

        // Should return one of the three (all tied)
        let best = pool.get_best_bid(Slot::new(10), Hash256::zero()).unwrap();
        assert_eq!(best.message.value, 100);
    }

    #[test]
    fn bid_count_for_empty_slot() {
        let pool = ExecutionBidPool::<E>::new();
        assert_eq!(pool.bid_count_for_slot(Slot::new(42)), 0);
    }

    #[test]
    fn prune_idempotent() {
        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid(10, 1, 100));

        pool.prune(Slot::new(10));
        assert_eq!(pool.total_bid_count(), 1);

        pool.prune(Slot::new(10));
        assert_eq!(pool.total_bid_count(), 1);
    }

    fn make_bid_with_parent(
        slot: u64,
        builder_index: u64,
        value: u64,
        parent_block_root: Hash256,
    ) -> SignedExecutionPayloadBid<E> {
        SignedExecutionPayloadBid {
            message: ExecutionPayloadBid {
                slot: Slot::new(slot),
                builder_index,
                value,
                parent_block_hash: ExecutionBlockHash::zero(),
                parent_block_root,
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
    fn best_bid_filters_by_parent_block_root() {
        let root_a = Hash256::from_low_u64_be(0xaa);
        let root_b = Hash256::from_low_u64_be(0xbb);

        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid_with_parent(10, 1, 1000, root_a));
        pool.insert(make_bid_with_parent(10, 2, 500, root_b));

        // Querying with root_a should return only the bid for root_a
        let best = pool.get_best_bid(Slot::new(10), root_a).unwrap();
        assert_eq!(best.message.value, 1000);
        assert_eq!(best.message.builder_index, 1);

        // Querying with root_b should return only the bid for root_b
        let best = pool.get_best_bid(Slot::new(10), root_b).unwrap();
        assert_eq!(best.message.value, 500);
        assert_eq!(best.message.builder_index, 2);
    }

    #[test]
    fn best_bid_wrong_parent_block_root_returns_none() {
        let root_a = Hash256::from_low_u64_be(0xaa);
        let root_b = Hash256::from_low_u64_be(0xbb);

        let mut pool = ExecutionBidPool::<E>::new();
        pool.insert(make_bid_with_parent(10, 1, 1000, root_a));

        // Querying with a different root should return None
        assert!(pool.get_best_bid(Slot::new(10), root_b).is_none());
    }

    #[test]
    fn best_bid_selects_highest_value_among_matching_parent() {
        let root_a = Hash256::from_low_u64_be(0xaa);
        let root_b = Hash256::from_low_u64_be(0xbb);

        let mut pool = ExecutionBidPool::<E>::new();
        // Two bids for root_a with different values
        pool.insert(make_bid_with_parent(10, 1, 100, root_a));
        pool.insert(make_bid_with_parent(10, 2, 900, root_a));
        // One higher-value bid for root_b (should be ignored when querying root_a)
        pool.insert(make_bid_with_parent(10, 3, 5000, root_b));

        let best = pool.get_best_bid(Slot::new(10), root_a).unwrap();
        assert_eq!(best.message.value, 900);
        assert_eq!(best.message.builder_index, 2);
    }
}
