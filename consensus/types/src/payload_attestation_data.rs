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
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestationdata
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

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(PayloadAttestationData);
}

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
