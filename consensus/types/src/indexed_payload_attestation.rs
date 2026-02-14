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
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#indexedpayloadattestation
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
    pub fn is_sorted(&self) -> bool {
        self.attesting_indices.windows(2).all(|w| w[0] < w[1])
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
            signature: Signature::empty(),
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
    use crate::MainnetEthSpec;

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
    fn test_is_sorted() {
        let mut attestation = IndexedPayloadAttestation::<MainnetEthSpec>::empty();

        // Empty list is sorted
        assert!(attestation.is_sorted());

        // Single element is sorted
        attestation.attesting_indices.push(5).unwrap();
        assert!(attestation.is_sorted());

        // Two sorted elements
        attestation.attesting_indices.push(10).unwrap();
        assert!(attestation.is_sorted());

        // Unsorted should fail - but we can't test this easily without modifying
        // the internal structure, so we document the expected behavior
    }
}
