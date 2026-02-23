//! Provides an `ObservedPayloadAttestations` struct which tracks which validators have
//! attested to payload presence, allowing the beacon node to:
//!
//! 1. Prevent duplicate attestations from being propagated
//! 2. Detect equivocation (conflicting attestations from same validator for same slot/block)
//!
//! This serves as equivocation detection for the payload attestation gossip topic.

use derivative::Derivative;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use types::{EthSpec, Hash256, Slot};

/// Maximum number of slots to retain in the cache before pruning.
/// Set to 2 epochs worth of slots.
const MAX_OBSERVED_SLOTS: u64 = 64;

/// Key for tracking payload attestations: (slot, beacon_block_root, validator_index)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AttestationKey {
    slot: Slot,
    beacon_block_root: Hash256,
    validator_index: u64,
}

/// Outcome of observing a payload attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationObservationOutcome {
    /// This is the first attestation from this validator for this slot/block.
    New,
    /// We've already seen an attestation from this validator for this slot/block with the same payload_present value.
    Duplicate,
    /// The validator has already attested with a different payload_present value for this slot/block.
    /// This is equivocation and should be penalized.
    Equivocation {
        existing_payload_present: bool,
        new_payload_present: bool,
    },
}

/// Tracks observed payload attestations to prevent duplicates and detect equivocation.
///
/// Structure: (Slot, BeaconBlockRoot, ValidatorIndex) -> PayloadPresent
/// This allows us to:
/// - Check if we've seen an attestation from a specific validator for a specific block
/// - Detect when a validator submits conflicting attestations (different payload_present values)
#[derive(Debug, Derivative)]
#[derivative(Default(bound = "E: EthSpec"))]
pub struct ObservedPayloadAttestations<E: EthSpec> {
    /// Map of (slot, block_root, validator_index) -> payload_present
    observed_attestations: HashMap<AttestationKey, bool>,
    /// Set of slots we've observed, for efficient pruning
    observed_slots: HashSet<Slot>,
    _phantom: PhantomData<E>,
}

impl<E: EthSpec> ObservedPayloadAttestations<E> {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe an attestation with the given parameters.
    ///
    /// Returns:
    /// - `AttestationObservationOutcome::New` if this is the first attestation from this validator
    /// - `AttestationObservationOutcome::Duplicate` if we've seen this exact attestation before
    /// - `AttestationObservationOutcome::Equivocation` if the validator sent a conflicting attestation
    pub fn observe_attestation(
        &mut self,
        slot: Slot,
        beacon_block_root: Hash256,
        validator_index: u64,
        payload_present: bool,
    ) -> AttestationObservationOutcome {
        let key = AttestationKey {
            slot,
            beacon_block_root,
            validator_index,
        };

        // Track this slot
        self.observed_slots.insert(slot);

        // Check if we've seen an attestation from this validator for this slot/block
        match self.observed_attestations.get(&key) {
            None => {
                // First attestation from this validator for this slot/block
                self.observed_attestations.insert(key, payload_present);
                AttestationObservationOutcome::New
            }
            Some(&existing_payload_present) => {
                if existing_payload_present == payload_present {
                    // Same attestation, already seen
                    AttestationObservationOutcome::Duplicate
                } else {
                    // Conflicting attestation - equivocation!
                    AttestationObservationOutcome::Equivocation {
                        existing_payload_present,
                        new_payload_present: payload_present,
                    }
                }
            }
        }
    }

    /// Prune old slots from the cache to prevent unbounded growth.
    ///
    /// Retains only attestations from the most recent `MAX_OBSERVED_SLOTS` slots.
    pub fn prune_old_slots(&mut self, current_slot: Slot) {
        // Calculate the earliest slot we want to keep
        let earliest_slot = Slot::new(current_slot.as_u64().saturating_sub(MAX_OBSERVED_SLOTS));

        // Remove attestations from slots older than earliest_slot
        self.observed_attestations
            .retain(|key, _| key.slot >= earliest_slot);

        // Also prune the observed_slots set
        self.observed_slots.retain(|&slot| slot >= earliest_slot);
    }

    /// Returns the number of unique slots currently tracked.
    pub fn observed_slot_count(&self) -> usize {
        self.observed_slots.len()
    }

    /// Returns the total number of attestations currently tracked across all slots.
    pub fn observed_attestation_count(&self) -> usize {
        self.observed_attestations.len()
    }

    /// Clear all observed attestations. Useful for testing.
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.observed_attestations.clear();
        self.observed_slots.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FixedBytesExtended, MainnetEthSpec};

    type E = MainnetEthSpec;

    #[test]
    fn test_new_attestation_observed() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator_index = 42;

        let outcome = cache.observe_attestation(slot, block_root, validator_index, true);
        assert_eq!(outcome, AttestationObservationOutcome::New);
        assert_eq!(cache.observed_slot_count(), 1);
        assert_eq!(cache.observed_attestation_count(), 1);
    }

    #[test]
    fn test_duplicate_attestation_detected() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator_index = 42;

        // First observation
        cache.observe_attestation(slot, block_root, validator_index, true);

        // Second observation of same attestation
        let outcome = cache.observe_attestation(slot, block_root, validator_index, true);
        assert_eq!(outcome, AttestationObservationOutcome::Duplicate);
        assert_eq!(cache.observed_attestation_count(), 1);
    }

    #[test]
    fn test_equivocation_detected() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator_index = 42;

        // First attestation with payload_present=true
        cache.observe_attestation(slot, block_root, validator_index, true);

        // Conflicting attestation with payload_present=false
        let outcome = cache.observe_attestation(slot, block_root, validator_index, false);
        match outcome {
            AttestationObservationOutcome::Equivocation {
                existing_payload_present,
                new_payload_present,
            } => {
                assert!(existing_payload_present);
                assert!(!new_payload_present);
            }
            _ => panic!("Expected equivocation, got {:?}", outcome),
        }
    }

    #[test]
    fn test_different_blocks_same_slot() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root_1 = Hash256::from_low_u64_be(1);
        let block_root_2 = Hash256::from_low_u64_be(2);
        let validator_index = 42;

        // Attestation for first block
        cache.observe_attestation(slot, block_root_1, validator_index, true);

        // Attestation for different block in same slot (this is allowed - not equivocation)
        let outcome = cache.observe_attestation(slot, block_root_2, validator_index, true);
        assert_eq!(outcome, AttestationObservationOutcome::New);
        assert_eq!(cache.observed_attestation_count(), 2);
    }

    #[test]
    fn test_multiple_validators_same_block() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator_1 = 1;
        let validator_2 = 2;

        cache.observe_attestation(slot, block_root, validator_1, true);
        let outcome = cache.observe_attestation(slot, block_root, validator_2, true);

        assert_eq!(outcome, AttestationObservationOutcome::New);
        assert_eq!(cache.observed_attestation_count(), 2);
    }

    #[test]
    fn test_pruning() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);

        // Add attestations for slots 0..100
        for slot in 0..100 {
            cache.observe_attestation(
                Slot::new(slot),
                block_root,
                slot, // use slot as validator_index for simplicity
                true,
            );
        }

        assert_eq!(cache.observed_slot_count(), 100);

        // Prune from slot 100 (should keep slots >= 36)
        cache.prune_old_slots(Slot::new(100));

        // Should have pruned everything older than slot 36 (100 - 64)
        assert_eq!(cache.observed_slot_count(), 64);
        assert_eq!(cache.observed_attestation_count(), 64);
    }

    #[test]
    fn test_same_validator_different_slots() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);
        let validator = 42;

        let o1 = cache.observe_attestation(Slot::new(10), block_root, validator, true);
        let o2 = cache.observe_attestation(Slot::new(11), block_root, validator, false);

        // Different slots → both are New (no equivocation across slots)
        assert_eq!(o1, AttestationObservationOutcome::New);
        assert_eq!(o2, AttestationObservationOutcome::New);
        assert_eq!(cache.observed_attestation_count(), 2);
    }

    #[test]
    fn test_equivocation_false_then_true() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator = 42;

        // First: payload_present = false
        cache.observe_attestation(slot, block_root, validator, false);

        // Second: payload_present = true → equivocation
        let outcome = cache.observe_attestation(slot, block_root, validator, true);
        match outcome {
            AttestationObservationOutcome::Equivocation {
                existing_payload_present,
                new_payload_present,
            } => {
                assert!(!existing_payload_present);
                assert!(new_payload_present);
            }
            _ => panic!("Expected equivocation, got {:?}", outcome),
        }
    }

    #[test]
    fn test_duplicate_false() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator = 42;

        cache.observe_attestation(slot, block_root, validator, false);
        let outcome = cache.observe_attestation(slot, block_root, validator, false);
        assert_eq!(outcome, AttestationObservationOutcome::Duplicate);
    }

    #[test]
    fn test_prune_at_zero() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);
        cache.observe_attestation(Slot::new(0), block_root, 1, true);

        cache.prune_old_slots(Slot::new(0));

        // Slot 0 >= 0 - 64 (saturates to 0), so it's retained
        assert_eq!(cache.observed_slot_count(), 1);
    }

    #[test]
    fn test_prune_boundary_slot() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);
        // MAX_OBSERVED_SLOTS = 64, prune(70) keeps slots >= 6
        cache.observe_attestation(Slot::new(6), block_root, 1, true);
        cache.observe_attestation(Slot::new(5), block_root, 2, true);

        cache.prune_old_slots(Slot::new(70));

        assert_eq!(cache.observed_slot_count(), 1);
        assert_eq!(cache.observed_attestation_count(), 1);
    }

    #[test]
    fn test_equivocation_preserves_original() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);
        let validator = 42;

        cache.observe_attestation(slot, block_root, validator, true);

        // Equivocate
        cache.observe_attestation(slot, block_root, validator, false);

        // Third observation with original value → still Duplicate (original preserved)
        let outcome = cache.observe_attestation(slot, block_root, validator, true);
        assert_eq!(outcome, AttestationObservationOutcome::Duplicate);

        // Only one attestation stored
        assert_eq!(cache.observed_attestation_count(), 1);
    }

    #[test]
    fn test_clear_resets_state() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);
        cache.observe_attestation(Slot::new(1), block_root, 1, true);
        cache.observe_attestation(Slot::new(2), block_root, 2, false);

        cache.clear();

        assert_eq!(cache.observed_slot_count(), 0);
        assert_eq!(cache.observed_attestation_count(), 0);

        // After clear, previously seen attestation is New again
        let outcome = cache.observe_attestation(Slot::new(1), block_root, 1, true);
        assert_eq!(outcome, AttestationObservationOutcome::New);
    }

    #[test]
    fn test_many_validators_same_block() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let slot = Slot::new(100);
        let block_root = Hash256::from_low_u64_be(1);

        for v in 0..512 {
            let outcome = cache.observe_attestation(slot, block_root, v, true);
            assert_eq!(outcome, AttestationObservationOutcome::New);
        }

        assert_eq!(cache.observed_attestation_count(), 512);
        assert_eq!(cache.observed_slot_count(), 1);
    }

    #[test]
    fn test_prune_idempotent() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let block_root = Hash256::from_low_u64_be(1);
        cache.observe_attestation(Slot::new(100), block_root, 1, true);

        cache.prune_old_slots(Slot::new(100));
        assert_eq!(cache.observed_attestation_count(), 1);

        cache.prune_old_slots(Slot::new(100));
        assert_eq!(cache.observed_attestation_count(), 1);
    }
}
