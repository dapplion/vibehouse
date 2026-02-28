use crate::{EthSpec, ForkName, PayloadAttestationData, test_utils::TestRandom};
use bls::AggregateSignature;
use context_deserialize::context_deserialize;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Indexed (unpacked) payload attestation for verification in Gloas ePBS.
///
/// Similar to IndexedAttestation for regular attestations, this unpacks
/// the aggregation bitfield into an explicit list of attesting validator
/// indices. This makes signature verification more efficient.
///
/// The attesting_indices list must be sorted for efficient verification.
///
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#indexedpayloadattestation>
#[derive(
    Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative, TestRandom,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct IndexedPayloadAttestation<E: EthSpec> {
    /// Sorted list of validator indices that signed this attestation
    /// Maximum size is PTC_SIZE (512)
    pub attesting_indices: VariableList<u64, E::PtcSize>,
    /// The attestation data being signed
    pub data: PayloadAttestationData,
    /// BLS aggregate signature from all attesting validators
    pub signature: AggregateSignature,
}

impl<E: EthSpec> IndexedPayloadAttestation<E> {
    /// Returns the number of attesting validators.
    pub fn num_attesters(&self) -> usize {
        self.attesting_indices.len()
    }

    /// Checks if the attesting_indices list is sorted (required for validity).
    /// Uses non-decreasing order (duplicates allowed) to match the spec's `sorted()`.
    pub fn is_sorted(&self) -> bool {
        self.attesting_indices
            .windows(2)
            .all(|w| matches!(w, [a, b] if a <= b))
    }

    /// Create an empty indexed payload attestation (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            attesting_indices: VariableList::empty(),
            data: PayloadAttestationData {
                beacon_block_root: Default::default(),
                slot: Default::default(),
                payload_present: false,
                blob_data_available: false,
            },
            signature: AggregateSignature::empty(),
        }
    }
}

impl<E: EthSpec> Default for IndexedPayloadAttestation<E> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hash256, MainnetEthSpec, MinimalEthSpec, Slot};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(IndexedPayloadAttestation<MainnetEthSpec>);

    #[test]
    fn test_empty_indexed_payload_attestation() {
        let attestation = IndexedPayloadAttestation::<MainnetEthSpec>::empty();
        assert_eq!(attestation.num_attesters(), 0);
        assert!(attestation.is_sorted());
        assert!(!attestation.data.payload_present);
        assert!(!attestation.data.blob_data_available);
    }

    #[test]
    fn default_equals_empty() {
        let a = IndexedPayloadAttestation::<E>::default();
        let b = IndexedPayloadAttestation::<E>::empty();
        assert_eq!(a, b);
    }

    #[test]
    fn is_sorted_empty() {
        let att = IndexedPayloadAttestation::<E>::empty();
        assert!(att.is_sorted());
    }

    #[test]
    fn is_sorted_single_element() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(5).unwrap();
        assert!(att.is_sorted());
    }

    #[test]
    fn is_sorted_ascending() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(1).unwrap();
        att.attesting_indices.push(5).unwrap();
        assert!(att.is_sorted());
    }

    #[test]
    fn is_sorted_unsorted_via_ssz() {
        // Build a sorted attestation, encode to SSZ, manually swap bytes to create
        // unsorted indices, then decode and verify is_sorted() returns false.
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(10).unwrap();
        att.attesting_indices.push(5).unwrap(); // 10, 5 — descending

        // Construct via SSZ decode to bypass push ordering
        let bytes = att.as_ssz_bytes();
        let decoded = IndexedPayloadAttestation::<E>::from_ssz_bytes(&bytes).unwrap();
        assert!(
            !decoded.is_sorted(),
            "descending [10, 5] should not be sorted"
        );
    }

    #[test]
    fn is_sorted_duplicate_indices() {
        // Duplicate indices should pass is_sorted (spec allows non-decreasing order)
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(5).unwrap();
        att.attesting_indices.push(5).unwrap(); // duplicate

        let bytes = att.as_ssz_bytes();
        let decoded = IndexedPayloadAttestation::<E>::from_ssz_bytes(&bytes).unwrap();
        assert!(
            decoded.is_sorted(),
            "duplicate indices [5, 5] should be sorted (spec uses non-decreasing order)"
        );
    }

    #[test]
    fn num_attesters_counts_indices() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        assert_eq!(att.num_attesters(), 0);

        att.attesting_indices.push(1).unwrap();
        assert_eq!(att.num_attesters(), 1);

        att.attesting_indices.push(2).unwrap();
        assert_eq!(att.num_attesters(), 2);
    }

    #[test]
    fn ssz_roundtrip_with_indices() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(0).unwrap();
        att.attesting_indices.push(42).unwrap();
        att.data.slot = Slot::new(99);
        att.data.beacon_block_root = Hash256::repeat_byte(0xab);
        att.data.payload_present = true;

        let bytes = att.as_ssz_bytes();
        let decoded = IndexedPayloadAttestation::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(att, decoded);
        assert_eq!(decoded.num_attesters(), 2);
        assert!(decoded.data.payload_present);
    }

    #[test]
    fn ssz_roundtrip_both_flags() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.data.payload_present = true;
        att.data.blob_data_available = true;
        att.attesting_indices.push(7).unwrap();

        let bytes = att.as_ssz_bytes();
        let decoded = IndexedPayloadAttestation::<E>::from_ssz_bytes(&bytes).unwrap();
        assert!(decoded.data.payload_present);
        assert!(decoded.data.blob_data_available);
    }

    #[test]
    fn tree_hash_changes_with_indices() {
        let att1 = IndexedPayloadAttestation::<E>::empty();
        let mut att2 = IndexedPayloadAttestation::<E>::empty();
        att2.attesting_indices.push(1).unwrap();

        assert_ne!(att1.tree_hash_root(), att2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(5).unwrap();
        att.data.payload_present = true;
        assert_eq!(att.tree_hash_root(), att.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let mut att = IndexedPayloadAttestation::<E>::empty();
        att.attesting_indices.push(10).unwrap();
        att.data.blob_data_available = true;
        assert_eq!(att, att.clone());
    }

    #[test]
    fn different_indices_not_equal() {
        let mut att1 = IndexedPayloadAttestation::<E>::empty();
        let mut att2 = IndexedPayloadAttestation::<E>::empty();
        att1.attesting_indices.push(1).unwrap();
        att2.attesting_indices.push(2).unwrap();
        assert_ne!(att1, att2);
    }
}
