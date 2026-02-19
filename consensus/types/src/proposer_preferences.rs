use crate::{Address, ForkName, test_utils::TestRandom};
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Proposer preferences for Gloas ePBS.
///
/// Allows validators to communicate their preferred fee_recipient and gas_limit
/// to builders for a specific proposal slot.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/p2p-interface.md
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct ProposerPreferences {
    #[serde(with = "serde_utils::quoted_u64")]
    pub proposal_slot: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    pub fee_recipient: Address,
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(ProposerPreferences);
}
