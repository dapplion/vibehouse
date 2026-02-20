use crate::{EthSpec, ForkName, PayloadAttestationData, test_utils::TestRandom};
use bls::AggregateSignature;
use context_deserialize::context_deserialize;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use ssz_types::BitVector;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Aggregated payload attestation from PTC members in Gloas ePBS.
///
/// The PTC (Payload Timeliness Committee) is a subset of 512 validators
/// selected per slot who attest to payload delivery and blob availability.
/// This aggregated attestation combines multiple individual PTC member
/// attestations using a bitvector and aggregate signature.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestation
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
pub struct PayloadAttestation<E: EthSpec> {
    /// Bitfield indicating which PTC members signed this attestation
    /// PTC_SIZE = 512 validators per slot
    pub aggregation_bits: BitVector<E::PtcSize>,
    /// The attestation data being signed
    pub data: PayloadAttestationData,
    /// BLS aggregate signature from all attesting PTC members
    pub signature: AggregateSignature,
}

impl<E: EthSpec> PayloadAttestation<E> {
    /// Returns the number of set bits in the aggregation bitfield.
    pub fn num_attesters(&self) -> usize {
        self.aggregation_bits.num_set_bits()
    }

    /// Create an empty payload attestation (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            aggregation_bits: BitVector::new(),
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

impl<E: EthSpec> Default for PayloadAttestation<E> {
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

    ssz_and_tree_hash_tests!(PayloadAttestation<MainnetEthSpec>);

    #[test]
    fn test_empty_payload_attestation() {
        let attestation = PayloadAttestation::<MainnetEthSpec>::empty();
        assert_eq!(attestation.num_attesters(), 0);
        assert!(!attestation.data.payload_present);
        assert!(!attestation.data.blob_data_available);
    }

    #[test]
    fn default_equals_empty() {
        let a = PayloadAttestation::<E>::default();
        let b = PayloadAttestation::<E>::empty();
        assert_eq!(a, b);
    }

    #[test]
    fn num_attesters_with_bits_set() {
        // MainnetEthSpec has PtcSize=512 so we can test larger bitvectors
        let mut att = PayloadAttestation::<MainnetEthSpec>::empty();
        assert_eq!(att.num_attesters(), 0);

        att.aggregation_bits.set(0, true).unwrap();
        assert_eq!(att.num_attesters(), 1);

        att.aggregation_bits.set(3, true).unwrap();
        assert_eq!(att.num_attesters(), 2);

        att.aggregation_bits.set(7, true).unwrap();
        assert_eq!(att.num_attesters(), 3);
    }

    #[test]
    fn num_attesters_all_bits_set() {
        let mut att = PayloadAttestation::<E>::empty();
        let ptc_size = att.aggregation_bits.len();
        for i in 0..ptc_size {
            att.aggregation_bits.set(i, true).unwrap();
        }
        assert_eq!(att.num_attesters(), ptc_size);
    }

    #[test]
    fn payload_present_true() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.payload_present = true;
        assert!(att.data.payload_present);
        assert!(!att.data.blob_data_available);
    }

    #[test]
    fn blob_data_available_true() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.blob_data_available = true;
        assert!(!att.data.payload_present);
        assert!(att.data.blob_data_available);
    }

    #[test]
    fn both_flags_true() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.payload_present = true;
        att.data.blob_data_available = true;
        assert!(att.data.payload_present);
        assert!(att.data.blob_data_available);
    }

    #[test]
    fn ssz_roundtrip_with_set_bits() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.slot = Slot::new(42);
        att.data.beacon_block_root = Hash256::repeat_byte(0xab);
        att.data.payload_present = true;
        att.data.blob_data_available = true;
        att.aggregation_bits.set(0, true).unwrap();
        att.aggregation_bits.set(1, true).unwrap();

        let bytes = att.as_ssz_bytes();
        let decoded = PayloadAttestation::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(att, decoded);
        assert_eq!(decoded.num_attesters(), 2);
        assert!(decoded.data.payload_present);
        assert!(decoded.data.blob_data_available);
    }

    #[test]
    fn tree_hash_changes_with_different_bits() {
        let mut att1 = PayloadAttestation::<E>::empty();
        let mut att2 = PayloadAttestation::<E>::empty();

        let root1 = att1.tree_hash_root();
        let root2 = att2.tree_hash_root();
        assert_eq!(root1, root2, "identical attestations should hash equal");

        att1.aggregation_bits.set(0, true).unwrap();
        let root1_modified = att1.tree_hash_root();
        assert_ne!(root1, root1_modified, "setting a bit should change hash");

        att2.data.payload_present = true;
        let root2_modified = att2.tree_hash_root();
        assert_ne!(root2, root2_modified, "changing flag should change hash");
        assert_ne!(
            root1_modified, root2_modified,
            "different changes should produce different hashes"
        );
    }

    #[test]
    fn tree_hash_deterministic() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.slot = Slot::new(99);
        att.aggregation_bits.set(0, true).unwrap();
        let root1 = att.tree_hash_root();
        let root2 = att.tree_hash_root();
        assert_eq!(root1, root2);
    }

    #[test]
    fn clone_preserves_equality() {
        let mut att = PayloadAttestation::<E>::empty();
        att.data.payload_present = true;
        att.aggregation_bits.set(1, true).unwrap();
        let cloned = att.clone();
        assert_eq!(att, cloned);
    }

    #[test]
    fn different_slots_not_equal() {
        let mut att1 = PayloadAttestation::<E>::empty();
        let mut att2 = PayloadAttestation::<E>::empty();
        att1.data.slot = Slot::new(1);
        att2.data.slot = Slot::new(2);
        assert_ne!(att1, att2);
    }
}
