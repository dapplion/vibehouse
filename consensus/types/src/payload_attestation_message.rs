use crate::{ForkName, PayloadAttestationData, test_utils::TestRandom};
use bls::Signature;
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Individual payload attestation message from a PTC member in Gloas ePBS.
///
/// Before aggregation, each PTC member submits their individual attestation
/// as a PayloadAttestationMessage. These are aggregated into PayloadAttestation
/// for inclusion in beacon blocks.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestationmessage
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct PayloadAttestationMessage {
    /// Index of the validator submitting this attestation
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    /// The attestation data being signed
    pub data: PayloadAttestationData,
    /// BLS signature from the individual validator
    pub signature: Signature,
}

impl PayloadAttestationMessage {
    /// Create an empty payload attestation message (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            validator_index: 0,
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

impl Default for PayloadAttestationMessage {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hash256, Slot};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(PayloadAttestationMessage);

    #[test]
    fn test_empty_payload_attestation_message() {
        let message = PayloadAttestationMessage::empty();
        assert_eq!(message.validator_index, 0);
        assert!(!message.data.payload_present);
        assert!(!message.data.blob_data_available);
    }

    #[test]
    fn default_equals_empty() {
        let a = PayloadAttestationMessage::default();
        let b = PayloadAttestationMessage::empty();
        assert_eq!(a, b);
    }

    #[test]
    fn non_zero_validator_index() {
        let mut msg = PayloadAttestationMessage::empty();
        msg.validator_index = 42;
        assert_eq!(msg.validator_index, 42);

        let bytes = msg.as_ssz_bytes();
        let decoded = PayloadAttestationMessage::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.validator_index, 42);
    }

    #[test]
    fn max_validator_index() {
        let mut msg = PayloadAttestationMessage::empty();
        msg.validator_index = u64::MAX;

        let bytes = msg.as_ssz_bytes();
        let decoded = PayloadAttestationMessage::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.validator_index, u64::MAX);
    }

    #[test]
    fn ssz_roundtrip_payload_present() {
        let msg = PayloadAttestationMessage {
            validator_index: 99,
            data: PayloadAttestationData {
                beacon_block_root: Hash256::repeat_byte(0xff),
                slot: Slot::new(100),
                payload_present: true,
                blob_data_available: false,
            },
            signature: Signature::empty(),
        };
        let bytes = msg.as_ssz_bytes();
        let decoded = PayloadAttestationMessage::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        assert!(decoded.data.payload_present);
    }

    #[test]
    fn ssz_roundtrip_blob_data_available() {
        let msg = PayloadAttestationMessage {
            validator_index: 7,
            data: PayloadAttestationData {
                beacon_block_root: Hash256::repeat_byte(0xee),
                slot: Slot::new(200),
                payload_present: false,
                blob_data_available: true,
            },
            signature: Signature::empty(),
        };
        let bytes = msg.as_ssz_bytes();
        let decoded = PayloadAttestationMessage::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        assert!(decoded.data.blob_data_available);
    }

    #[test]
    fn tree_hash_changes_with_validator_index() {
        let msg1 = PayloadAttestationMessage {
            validator_index: 1,
            ..PayloadAttestationMessage::empty()
        };
        let msg2 = PayloadAttestationMessage {
            validator_index: 2,
            ..PayloadAttestationMessage::empty()
        };
        assert_ne!(msg1.tree_hash_root(), msg2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let msg = PayloadAttestationMessage {
            validator_index: 50,
            data: PayloadAttestationData {
                beacon_block_root: Hash256::repeat_byte(0x01),
                slot: Slot::new(10),
                payload_present: true,
                blob_data_available: true,
            },
            signature: Signature::empty(),
        };
        assert_eq!(msg.tree_hash_root(), msg.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let msg = PayloadAttestationMessage {
            validator_index: 77,
            data: PayloadAttestationData {
                beacon_block_root: Hash256::repeat_byte(0xab),
                slot: Slot::new(55),
                payload_present: true,
                blob_data_available: true,
            },
            signature: Signature::empty(),
        };
        assert_eq!(msg, msg.clone());
    }

    #[test]
    fn different_data_not_equal() {
        let mut msg1 = PayloadAttestationMessage::empty();
        let mut msg2 = PayloadAttestationMessage::empty();
        msg1.data.payload_present = true;
        msg2.data.payload_present = false;
        assert_ne!(msg1, msg2);
    }
}
