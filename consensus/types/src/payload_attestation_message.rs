use crate::{test_utils::TestRandom, PayloadAttestationData};
use bls::Signature;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;
use types_derive::ValidatorIndex;

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

    ssz_and_tree_hash_tests!(PayloadAttestationMessage);

    #[test]
    fn test_empty_payload_attestation_message() {
        let message = PayloadAttestationMessage::empty();
        assert_eq!(message.validator_index, 0);
        assert!(!message.data.payload_present);
        assert!(!message.data.blob_data_available);
    }
}
