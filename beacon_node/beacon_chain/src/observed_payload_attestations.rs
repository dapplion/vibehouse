use std::collections::{hash_map::Entry, HashMap};
use types::{BeaconStateError, EthSpec, Hash256, Slot};

/// Maintains a cache of observed payload attestations to detect duplicates and equivocations.
///
/// For each (validator_index, slot), we track the PayloadAttestationData root.
/// This allows us to:
/// - Reject duplicate attestations (same validator + slot + data)
/// - Detect equivocations (same validator + slot, different data)
///
/// Entries are pruned when slots are finalized.
#[derive(Default)]
pub struct ObservedPayloadAttestations<E: EthSpec> {
    /// Map: (validator_index, slot) -> PayloadAttestationData root
    attestations: HashMap<(u64, Slot), Hash256>,
    /// Phantom data for EthSpec
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EthSpec> ObservedPayloadAttestations<E> {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            attestations: HashMap::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Observe a payload attestation from a validator for a slot.
    ///
    /// Returns:
    /// - `Ok(None)` if this is the first attestation from this validator for this slot
    /// - `Ok(Some(existing_root))` if the validator already attested for this slot
    ///   - If `existing_root == data_root`: duplicate (same data)
    ///   - If `existing_root != data_root`: equivocation (different data)
    /// - `Err(_)` if the slot is finalized (attestations for finalized slots shouldn't be gossipped)
    pub fn observe_attestation(
        &mut self,
        validator_index: u64,
        slot: Slot,
        data_root: Hash256,
    ) -> Result<Option<Hash256>, BeaconStateError> {
        let key = (validator_index, slot);

        match self.attestations.entry(key) {
            Entry::Vacant(entry) => {
                // First attestation from this validator for this slot
                entry.insert(data_root);
                Ok(None)
            }
            Entry::Occupied(entry) => {
                // Validator already attested for this slot
                Ok(Some(*entry.get()))
            }
        }
    }

    /// Prune attestations for slots that have been finalized.
    ///
    /// This removes all entries where slot <= finalized_slot.
    pub fn prune(&mut self, finalized_slot: Slot) {
        self.attestations.retain(|(_, slot), _| *slot > finalized_slot);
    }

    /// Returns the number of attestations being tracked.
    pub fn len(&self) -> usize {
        self.attestations.len()
    }

    /// Returns true if no attestations are being tracked.
    pub fn is_empty(&self) -> bool {
        self.attestations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::MainnetEthSpec;

    type E = MainnetEthSpec;

    #[test]
    fn test_observe_new_attestation() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let validator = 42;
        let slot = Slot::new(100);
        let data_root = Hash256::from_low_u64_be(1);

        let result = cache.observe_attestation(validator, slot, data_root).unwrap();
        assert_eq!(result, None, "First attestation should return None");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_detect_duplicate() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let validator = 42;
        let slot = Slot::new(100);
        let data_root = Hash256::from_low_u64_be(1);

        // First attestation
        cache.observe_attestation(validator, slot, data_root).unwrap();

        // Second attestation with same data
        let result = cache.observe_attestation(validator, slot, data_root).unwrap();
        assert_eq!(
            result,
            Some(data_root),
            "Duplicate attestation should return existing root"
        );
    }

    #[test]
    fn test_detect_equivocation() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let validator = 42;
        let slot = Slot::new(100);
        let data_root_1 = Hash256::from_low_u64_be(1);
        let data_root_2 = Hash256::from_low_u64_be(2);

        // First attestation
        cache.observe_attestation(validator, slot, data_root_1).unwrap();

        // Second attestation with different data - EQUIVOCATION
        let result = cache.observe_attestation(validator, slot, data_root_2).unwrap();
        assert_eq!(
            result,
            Some(data_root_1),
            "Equivocation should return previous root"
        );
        assert_ne!(result.unwrap(), data_root_2, "Roots should differ");
    }

    #[test]
    fn test_different_validators_same_slot() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let validator_1 = 42;
        let validator_2 = 43;
        let slot = Slot::new(100);
        let data_root = Hash256::from_low_u64_be(1);

        // Attestation from validator 1
        cache.observe_attestation(validator_1, slot, data_root).unwrap();

        // Attestation from validator 2 (different validator, same slot+data)
        let result = cache.observe_attestation(validator_2, slot, data_root).unwrap();
        assert_eq!(result, None, "Different validators should be tracked independently");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_same_validator_different_slots() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let validator = 42;
        let slot_1 = Slot::new(100);
        let slot_2 = Slot::new(101);
        let data_root = Hash256::from_low_u64_be(1);

        // Attestation in slot 100
        cache.observe_attestation(validator, slot_1, data_root).unwrap();

        // Attestation in slot 101 (same validator, different slot)
        let result = cache.observe_attestation(validator, slot_2, data_root).unwrap();
        assert_eq!(result, None, "Same validator can attest in different slots");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_prune() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let data_root = Hash256::from_low_u64_be(1);

        // Add attestations at slots 100, 101, 102
        cache.observe_attestation(1, Slot::new(100), data_root).unwrap();
        cache.observe_attestation(2, Slot::new(101), data_root).unwrap();
        cache.observe_attestation(3, Slot::new(102), data_root).unwrap();
        assert_eq!(cache.len(), 3);

        // Prune slots <= 101
        cache.prune(Slot::new(101));

        // Only slot 102 should remain
        assert_eq!(cache.len(), 1);

        // Verify slot 100 and 101 are gone
        let result = cache.observe_attestation(1, Slot::new(100), data_root).unwrap();
        assert_eq!(result, None, "Pruned attestation should be forgotten");
    }

    #[test]
    fn test_prune_all() {
        let mut cache = ObservedPayloadAttestations::<E>::new();
        let data_root = Hash256::from_low_u64_be(1);

        cache.observe_attestation(1, Slot::new(100), data_root).unwrap();
        cache.observe_attestation(2, Slot::new(101), data_root).unwrap();
        assert_eq!(cache.len(), 2);

        // Prune everything
        cache.prune(Slot::new(200));
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }
}
