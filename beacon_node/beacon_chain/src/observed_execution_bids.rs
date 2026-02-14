//! Provides an `ObservedExecutionBids` struct which allows us to reject duplicate or equivocating
//! execution payload bids.
//!
//! In Gloas ePBS, builders submit execution payload bids for slots. We need to track:
//! - Which bids we've already seen (to avoid reprocessing)
//! - Conflicting bids from the same builder for the same slot (equivocation detection)

use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use types::{EthSpec, Hash256, Slot};

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The slot is finalized and cannot be modified.
    FinalizedSlot { slot: Slot, finalized_slot: Slot },
    /// The maximum number of bids per slot has been reached.
    ReachedMaxBidsPerSlot(usize),
}

#[derive(Eq, Hash, PartialEq, Debug, Clone, Copy)]
struct BidKey {
    builder_index: u64,
    slot: Slot,
}

/// Stores a record of execution bids seen by the beacon chain.
///
/// ## Behavior
///
/// - When a new bid is observed for `(builder_index, slot)`:
///   - If we've never seen a bid for this (builder, slot), record it and return `None`
///   - If we've seen the SAME bid (same root), return `Some(root)` (duplicate)
///   - If we've seen a DIFFERENT bid (different root), return `Some(prev_root)` (equivocation!)
///
/// ## Equivocation Detection
///
/// If `observe_bid` returns `Some(prev_root)` where `prev_root != new_root`, then the builder
/// has equivocated by submitting two different bids for the same slot. This is slashable.
///
/// ## Pruning
///
/// The cache is pruned when `prune` is called with a finalized slot. All bids for slots
/// at or before the finalized slot are removed.
pub struct ObservedExecutionBids<E: EthSpec> {
    finalized_slot: Slot,
    /// Map from (builder_index, slot) to the bid root we've seen.
    ///
    /// If we see a second bid for the same (builder, slot) with a different root,
    /// that's equivocation.
    items: HashMap<BidKey, Hash256>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> Default for ObservedExecutionBids<E> {
    fn default() -> Self {
        Self {
            finalized_slot: Slot::new(0),
            items: HashMap::new(),
            _phantom: PhantomData,
        }
    }
}

impl<E: EthSpec> ObservedExecutionBids<E> {
    /// Observe an execution bid for `(builder_index, slot)`.
    ///
    /// ## Returns
    ///
    /// - `Ok(None)`: This is a new bid, not seen before. Proceed with validation.
    /// - `Ok(Some(root))` where `root == bid_root`: Duplicate bid, already seen. Reject without further processing.
    /// - `Ok(Some(root))` where `root != bid_root`: **EQUIVOCATION DETECTED**. The builder submitted conflicting bids.
    /// - `Err(...)`: The slot is finalized or some other error occurred.
    pub fn observe_bid(
        &mut self,
        builder_index: u64,
        slot: Slot,
        bid_root: Hash256,
    ) -> Result<Option<Hash256>, Error> {
        // Reject bids for finalized slots
        if slot <= self.finalized_slot {
            return Err(Error::FinalizedSlot {
                slot,
                finalized_slot: self.finalized_slot,
            });
        }

        let key = BidKey {
            builder_index,
            slot,
        };

        // Check if we've already seen a bid for this (builder, slot)
        match self.items.get(&key) {
            Some(&prev_root) => {
                // We've seen a bid before. Return the previous root.
                // The caller must check: if prev_root != bid_root, it's equivocation.
                Ok(Some(prev_root))
            }
            None => {
                // First time seeing a bid for this (builder, slot). Record it.
                self.items.insert(key, bid_root);
                Ok(None)
            }
        }
    }

    /// Prune the cache by removing all bids for slots at or before `finalized_slot`.
    pub fn prune(&mut self, finalized_slot: Slot) {
        if finalized_slot <= self.finalized_slot {
            return;
        }

        self.finalized_slot = finalized_slot;

        // Remove all bids for slots <= finalized_slot
        self.items.retain(|key, _| key.slot > finalized_slot);
    }

    /// Returns the number of observed bids currently in the cache.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{EthSpec, Hash256, MainnetEthSpec, Slot};

    type E = MainnetEthSpec;

    #[test]
    fn basic_observe() {
        let mut cache = ObservedExecutionBids::<E>::default();
        let builder_index = 42;
        let slot = Slot::new(100);
        let bid_root = Hash256::from_low_u64_be(1);

        // First observation: should return None (new bid)
        let result = cache.observe_bid(builder_index, slot, bid_root);
        assert_eq!(result, Ok(None));

        // Second observation with same root: should return Some(root) (duplicate)
        let result = cache.observe_bid(builder_index, slot, bid_root);
        assert_eq!(result, Ok(Some(bid_root)));

        // Third observation with DIFFERENT root: should return Some(prev_root) (equivocation!)
        let different_root = Hash256::from_low_u64_be(2);
        let result = cache.observe_bid(builder_index, slot, different_root);
        assert_eq!(result, Ok(Some(bid_root))); // Returns the FIRST root
    }

    #[test]
    fn different_builders_same_slot() {
        let mut cache = ObservedExecutionBids::<E>::default();
        let slot = Slot::new(100);
        let bid_root_1 = Hash256::from_low_u64_be(1);
        let bid_root_2 = Hash256::from_low_u64_be(2);

        // Builder 1 bids
        assert_eq!(cache.observe_bid(1, slot, bid_root_1), Ok(None));

        // Builder 2 bids (different root, but different builder, so NOT equivocation)
        assert_eq!(cache.observe_bid(2, slot, bid_root_2), Ok(None));

        // Builder 1 bids again with same root
        assert_eq!(cache.observe_bid(1, slot, bid_root_1), Ok(Some(bid_root_1)));
    }

    #[test]
    fn prune_finalized() {
        let mut cache = ObservedExecutionBids::<E>::default();
        let builder_index = 42;

        // Add bids for slots 10, 20, 30
        cache.observe_bid(builder_index, Slot::new(10), Hash256::from_low_u64_be(1)).unwrap();
        cache.observe_bid(builder_index, Slot::new(20), Hash256::from_low_u64_be(2)).unwrap();
        cache.observe_bid(builder_index, Slot::new(30), Hash256::from_low_u64_be(3)).unwrap();

        assert_eq!(cache.len(), 3);

        // Finalize slot 20 - should remove slots 10 and 20
        cache.prune(Slot::new(20));
        assert_eq!(cache.len(), 1);

        // Only slot 30 should remain
        let result = cache.observe_bid(builder_index, Slot::new(30), Hash256::from_low_u64_be(3));
        assert_eq!(result, Ok(Some(Hash256::from_low_u64_be(3)))); // Duplicate

        // Slot 20 should be finalized (error)
        let result = cache.observe_bid(builder_index, Slot::new(20), Hash256::from_low_u64_be(999));
        assert!(matches!(result, Err(Error::FinalizedSlot { .. })));
    }

    #[test]
    fn equivocation_detection() {
        let mut cache = ObservedExecutionBids::<E>::default();
        let builder_index = 7;
        let slot = Slot::new(50);
        let first_bid = Hash256::from_low_u64_be(100);
        let second_bid = Hash256::from_low_u64_be(200);

        // Submit first bid
        assert_eq!(cache.observe_bid(builder_index, slot, first_bid), Ok(None));

        // Submit second bid (CONFLICTING!) - this is equivocation
        let result = cache.observe_bid(builder_index, slot, second_bid);
        assert_eq!(result, Ok(Some(first_bid))); // Returns the first root, signaling equivocation

        // The cache should still have the FIRST bid recorded
        let result = cache.observe_bid(builder_index, slot, first_bid);
        assert_eq!(result, Ok(Some(first_bid))); // First bid is still what's stored
    }
}
