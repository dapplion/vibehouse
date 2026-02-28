//! Tracks which `beacon_block_root` values have had a valid
//! `SignedExecutionPayloadEnvelope` accepted via gossip.
//!
//! Spec: `[IGNORE] The node has not seen another valid
//! SignedExecutionPayloadEnvelope for this block root`
//!
//! Without deduplication a peer can replay valid envelopes and trigger
//! repeated `newPayload` EL calls (mild DoS vector).

use derivative::Derivative;
use std::collections::HashSet;
use std::marker::PhantomData;
use types::{EthSpec, Hash256};

/// Maximum number of block roots to retain before pruning.
const MAX_OBSERVED_ROOTS: usize = 256;

/// Tracks block roots for which a valid envelope has been seen.
#[derive(Debug, Derivative)]
#[derivative(Default(bound = "E: EthSpec"))]
pub struct ObservedPayloadEnvelopes<E: EthSpec> {
    /// Block roots for which we've accepted a valid envelope.
    observed_roots: HashSet<Hash256>,
    /// Insertion-ordered roots for FIFO pruning.
    insertion_order: Vec<Hash256>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> ObservedPayloadEnvelopes<E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if a valid envelope has already been recorded for this root.
    pub fn is_known(&self, beacon_block_root: &Hash256) -> bool {
        self.observed_roots.contains(beacon_block_root)
    }

    /// Record that a valid envelope has been accepted for the given root.
    ///
    /// Call this only after full validation succeeds, so that invalid
    /// envelopes don't prevent a later valid one from being processed.
    pub fn observe_envelope(&mut self, beacon_block_root: Hash256) {
        if self.observed_roots.insert(beacon_block_root) {
            self.insertion_order.push(beacon_block_root);
        }
    }

    /// Keep only the most recent `MAX_OBSERVED_ROOTS` entries (FIFO).
    pub fn prune(&mut self) {
        if self.insertion_order.len() <= MAX_OBSERVED_ROOTS {
            return;
        }
        let to_remove = self
            .insertion_order
            .len()
            .saturating_sub(MAX_OBSERVED_ROOTS);
        let removed: Vec<_> = self.insertion_order.drain(..to_remove).collect();
        for root in removed {
            self.observed_roots.remove(&root);
        }
    }

    /// Number of block roots currently tracked.
    pub fn len(&self) -> usize {
        self.observed_roots.len()
    }

    /// Returns true if no roots are tracked.
    pub fn is_empty(&self) -> bool {
        self.observed_roots.is_empty()
    }

    /// Clear all tracked roots. Useful for testing.
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.observed_roots.clear();
        self.insertion_order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MainnetEthSpec};

    type E = MainnetEthSpec;

    #[test]
    fn new_root_not_known() {
        let cache = ObservedPayloadEnvelopes::<E>::new();
        assert!(!cache.is_known(&Hash256::from_low_u64_be(1)));
    }

    #[test]
    fn observed_root_is_known() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();
        let root = Hash256::from_low_u64_be(1);

        cache.observe_envelope(root);
        assert!(cache.is_known(&root));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn duplicate_observe_is_idempotent() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();
        let root = Hash256::from_low_u64_be(1);

        cache.observe_envelope(root);
        cache.observe_envelope(root);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn different_roots_tracked_independently() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();
        let root1 = Hash256::from_low_u64_be(1);
        let root2 = Hash256::from_low_u64_be(2);

        cache.observe_envelope(root1);
        cache.observe_envelope(root2);
        assert!(cache.is_known(&root1));
        assert!(cache.is_known(&root2));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn prune_keeps_recent_entries() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();

        for i in 0..(MAX_OBSERVED_ROOTS + 50) {
            cache.observe_envelope(Hash256::from_low_u64_be(i as u64));
        }

        assert_eq!(cache.len(), MAX_OBSERVED_ROOTS + 50);

        cache.prune();

        assert_eq!(cache.len(), MAX_OBSERVED_ROOTS);

        // Oldest entries pruned
        assert!(
            !cache.is_known(&Hash256::from_low_u64_be(0)),
            "pruned entry should not be known"
        );
        assert!(
            cache.is_known(&Hash256::from_low_u64_be(100)),
            "recent entry should still be known"
        );
    }

    #[test]
    fn prune_noop_when_under_limit() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();
        cache.observe_envelope(Hash256::from_low_u64_be(1));
        cache.observe_envelope(Hash256::from_low_u64_be(2));

        cache.prune();

        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_resets_state() {
        let mut cache = ObservedPayloadEnvelopes::<E>::new();
        cache.observe_envelope(Hash256::from_low_u64_be(1));
        cache.observe_envelope(Hash256::from_low_u64_be(2));

        cache.clear();

        assert!(cache.is_empty());
        assert!(!cache.is_known(&Hash256::from_low_u64_be(1)));
    }
}
