//! Provides an `ObservedExecutionBids` struct which tracks which builder bids have been
//! observed by the beacon node, allowing it to:
//!
//! 1. Prevent duplicate bids from being propagated
//! 2. Detect equivocation (conflicting bids from same builder for same slot)
//!
//! This serves as equivocation detection for the execution payload bid gossip topic.

use derivative::Derivative;
use fixed_bytes::FixedBytesExtended;
use std::collections::HashMap;
use std::marker::PhantomData;
use tree_hash::TreeHash;
use types::{BuilderIndex, EthSpec, Hash256, Slot};

/// Maximum number of slots to retain in the cache before pruning.
/// Set to 2 epochs worth of slots.
const MAX_OBSERVED_SLOTS: u64 = 64;

/// Outcome of observing an execution bid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BidObservationOutcome {
    /// This is the first bid we've seen from this builder for this slot.
    New,
    /// We've already seen this exact bid (same root).
    Duplicate,
    /// The builder has already submitted a different bid for this slot.
    /// This is equivocation and should be penalized.
    Equivocation {
        existing_bid_root: Hash256,
        new_bid_root: Hash256,
    },
}

/// Tracks observed execution bids to prevent duplicates and detect equivocation.
///
/// Structure: Slot -> BuilderIndex -> BidRoot
/// This allows us to:
/// - Check if we've seen a bid from a specific builder in a specific slot
/// - Detect when a builder submits two different bids for the same slot
#[derive(Debug, Derivative)]
#[derivative(Default(bound = "E: EthSpec"))]
pub struct ObservedExecutionBids<E: EthSpec> {
    /// Map of slot -> (builder_index -> bid_root)
    observed_bids: HashMap<Slot, HashMap<BuilderIndex, Hash256>>,
    /// Slots we've observed, in insertion order for efficient pruning
    observed_slots: Vec<Slot>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> ObservedExecutionBids<E> {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe a bid with the given slot, builder index, and tree hash root.
    ///
    /// Returns:
    /// - `BidObservationOutcome::New` if this is the first bid from this builder for this slot
    /// - `BidObservationOutcome::Duplicate` if we've seen this exact bid before
    /// - `BidObservationOutcome::Equivocation` if the builder sent a different bid for this slot
    pub fn observe_bid(
        &mut self,
        slot: Slot,
        builder_index: BuilderIndex,
        bid_root: Hash256,
    ) -> BidObservationOutcome {
        // Get or create the entry for this slot
        let slot_bids = self.observed_bids.entry(slot).or_insert_with(|| {
            // Track this as a new slot
            self.observed_slots.push(slot);
            HashMap::new()
        });

        // Check if we've seen a bid from this builder for this slot
        match slot_bids.get(&builder_index) {
            None => {
                // First bid from this builder for this slot
                slot_bids.insert(builder_index, bid_root);
                BidObservationOutcome::New
            }
            Some(&existing_bid_root) => {
                if existing_bid_root == bid_root {
                    // Same bid, already seen
                    BidObservationOutcome::Duplicate
                } else {
                    // Different bid from same builder for same slot - equivocation!
                    BidObservationOutcome::Equivocation {
                        existing_bid_root,
                        new_bid_root: bid_root,
                    }
                }
            }
        }
    }

    /// Prune old slots from the cache to prevent unbounded growth.
    ///
    /// Retains only the most recent `MAX_OBSERVED_SLOTS` slots.
    pub fn prune_old_slots(&mut self, current_slot: Slot) {
        // Calculate the earliest slot we want to keep
        let earliest_slot = Slot::new(current_slot.as_u64().saturating_sub(MAX_OBSERVED_SLOTS));

        // Remove slots older than earliest_slot
        self.observed_bids
            .retain(|&slot, _| slot >= earliest_slot);

        // Also prune the observed_slots vector
        self.observed_slots
            .retain(|&slot| slot >= earliest_slot);
    }

    /// Returns the number of unique slots currently tracked.
    pub fn observed_slot_count(&self) -> usize {
        self.observed_bids.len()
    }

    /// Returns the total number of bids currently tracked across all slots.
    pub fn observed_bid_count(&self) -> usize {
        self.observed_bids.values().map(|m| m.len()).sum()
    }

    /// Clear all observed bids. Useful for testing.
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.observed_bids.clear();
        self.observed_slots.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MainnetEthSpec;

    type E = MainnetEthSpec;

    #[test]
    fn test_new_bid_observed() {
        let mut cache = ObservedExecutionBids::<E>::new();
        let slot = Slot::new(100);
        let builder_index = 42;
        let bid_root = Hash256::from_low_u64_be(1);

        let outcome = cache.observe_bid(slot, builder_index, bid_root);
        assert_eq!(outcome, BidObservationOutcome::New);
        assert_eq!(cache.observed_slot_count(), 1);
        assert_eq!(cache.observed_bid_count(), 1);
    }

    #[test]
    fn test_duplicate_bid_detected() {
        let mut cache = ObservedExecutionBids::<E>::new();
        let slot = Slot::new(100);
        let builder_index = 42;
        let bid_root = Hash256::from_low_u64_be(1);

        // First observation
        cache.observe_bid(slot, builder_index, bid_root);

        // Second observation of same bid
        let outcome = cache.observe_bid(slot, builder_index, bid_root);
        assert_eq!(outcome, BidObservationOutcome::Duplicate);
        assert_eq!(cache.observed_bid_count(), 1); // Still just one bid
    }

    #[test]
    fn test_equivocation_detected() {
        let mut cache = ObservedExecutionBids::<E>::new();
        let slot = Slot::new(100);
        let builder_index = 42;
        let bid_root_1 = Hash256::from_low_u64_be(1);
        let bid_root_2 = Hash256::from_low_u64_be(2);

        // First bid
        cache.observe_bid(slot, builder_index, bid_root_1);

        // Different bid from same builder for same slot
        let outcome = cache.observe_bid(slot, builder_index, bid_root_2);
        match outcome {
            BidObservationOutcome::Equivocation {
                existing_bid_root,
                new_bid_root,
            } => {
                assert_eq!(existing_bid_root, bid_root_1);
                assert_eq!(new_bid_root, bid_root_2);
            }
            _ => panic!("Expected equivocation, got {:?}", outcome),
        }
    }

    #[test]
    fn test_multiple_builders_same_slot() {
        let mut cache = ObservedExecutionBids::<E>::new();
        let slot = Slot::new(100);
        let builder_1 = 1;
        let builder_2 = 2;
        let bid_root_1 = Hash256::from_low_u64_be(1);
        let bid_root_2 = Hash256::from_low_u64_be(2);

        cache.observe_bid(slot, builder_1, bid_root_1);
        let outcome = cache.observe_bid(slot, builder_2, bid_root_2);

        assert_eq!(outcome, BidObservationOutcome::New);
        assert_eq!(cache.observed_slot_count(), 1);
        assert_eq!(cache.observed_bid_count(), 2);
    }

    #[test]
    fn test_pruning() {
        let mut cache = ObservedExecutionBids::<E>::new();

        // Add bids for slots 0..100
        for slot in 0..100 {
            cache.observe_bid(
                Slot::new(slot),
                slot, // use slot as builder_index for simplicity
                Hash256::from_low_u64_be(slot),
            );
        }

        assert_eq!(cache.observed_slot_count(), 100);

        // Prune from slot 100 (should keep slots >= 36)
        cache.prune_old_slots(Slot::new(100));

        // Should have pruned everything older than slot 36 (100 - 64)
        assert_eq!(cache.observed_slot_count(), 64);
        assert_eq!(cache.observed_bid_count(), 64);
    }
}
