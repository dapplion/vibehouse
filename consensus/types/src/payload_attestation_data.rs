use crate::{ForkName, Hash256, SignedRoot, Slot, test_utils::TestRandom};
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use tree_hash_derive::TreeHash;

/// Data for a payload timeliness attestation in Gloas ePBS.
///
/// PTC (Payload Timeliness Committee) members attest to whether the execution
/// payload was revealed on time and blob data is available.
///
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestationdata>
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct PayloadAttestationData {
    /// Root of the beacon block being attested to
    pub beacon_block_root: Hash256,
    /// Slot of the beacon block
    pub slot: Slot,
    /// Whether the execution payload was revealed (present)
    pub payload_present: bool,
    /// Whether blob data is available
    pub blob_data_available: bool,
}

impl SignedRoot for PayloadAttestationData {}

impl TestRandom for PayloadAttestationData {
    fn random_for_test(rng: &mut impl rand::RngCore) -> Self {
        Self {
            beacon_block_root: Hash256::random_for_test(rng),
            slot: Slot::random_for_test(rng),
            payload_present: bool::random_for_test(rng),
            blob_data_available: bool::random_for_test(rng),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(PayloadAttestationData);

    #[test]
    fn ssz_roundtrip_payload_present_true() {
        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xaa),
            slot: Slot::new(123),
            payload_present: true,
            blob_data_available: false,
        };
        let bytes = data.as_ssz_bytes();
        let decoded = PayloadAttestationData::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(data, decoded);
        assert!(decoded.payload_present);
        assert!(!decoded.blob_data_available);
    }

    #[test]
    fn ssz_roundtrip_blob_data_available_true() {
        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xbb),
            slot: Slot::new(456),
            payload_present: false,
            blob_data_available: true,
        };
        let bytes = data.as_ssz_bytes();
        let decoded = PayloadAttestationData::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(data, decoded);
        assert!(!decoded.payload_present);
        assert!(decoded.blob_data_available);
    }

    #[test]
    fn ssz_roundtrip_both_flags_true() {
        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xcc),
            slot: Slot::new(789),
            payload_present: true,
            blob_data_available: true,
        };
        let bytes = data.as_ssz_bytes();
        let decoded = PayloadAttestationData::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn tree_hash_changes_with_payload_present() {
        let data_false = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0x01),
            slot: Slot::new(1),
            payload_present: false,
            blob_data_available: false,
        };
        let data_true = PayloadAttestationData {
            payload_present: true,
            ..data_false.clone()
        };
        assert_ne!(data_false.tree_hash_root(), data_true.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_blob_data_available() {
        let data_false = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0x02),
            slot: Slot::new(2),
            payload_present: false,
            blob_data_available: false,
        };
        let data_true = PayloadAttestationData {
            blob_data_available: true,
            ..data_false.clone()
        };
        assert_ne!(data_false.tree_hash_root(), data_true.tree_hash_root());
    }

    #[test]
    fn equality_and_clone() {
        let data = PayloadAttestationData {
            beacon_block_root: Hash256::repeat_byte(0xdd),
            slot: Slot::new(42),
            payload_present: true,
            blob_data_available: true,
        };
        let cloned = data.clone();
        assert_eq!(data, cloned);

        let different = PayloadAttestationData {
            slot: Slot::new(43),
            ..data.clone()
        };
        assert_ne!(data, different);
    }

    #[test]
    fn default_fields_are_zero() {
        let data = PayloadAttestationData {
            beacon_block_root: Hash256::ZERO,
            slot: Slot::new(0),
            payload_present: false,
            blob_data_available: false,
        };
        assert_eq!(data.beacon_block_root, Hash256::ZERO);
        assert_eq!(data.slot, Slot::new(0));
    }
}
