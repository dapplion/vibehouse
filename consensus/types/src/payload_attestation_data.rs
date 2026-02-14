use crate::{Hash256, Slot};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Data for a payload timeliness attestation in Gloas ePBS.
///
/// PTC (Payload Timeliness Committee) members attest to whether the execution
/// payload was revealed on time and blob data is available.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestationdata
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
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

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(PayloadAttestationData);
}
