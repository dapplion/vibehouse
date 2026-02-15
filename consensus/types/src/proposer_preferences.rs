use crate::test_utils::TestRandom;
use crate::{Address, Slot, SignedRoot};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Proposer preferences for a specific slot in Gloas ePBS.
///
/// Validators publish their preferred `fee_recipient` and `gas_limit` for
/// upcoming proposal slots. Builders use these to construct valid bids that
/// match the proposer's requirements.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/dev/specs/gloas/p2p-interface.md#new-proposerpreferences
#[derive(
    Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash,
    TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ProposerPreferences {
    /// The slot this proposer is assigned to propose
    pub proposal_slot: Slot,
    /// Index of the validator publishing preferences
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    /// Preferred fee recipient address for execution payload
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
    /// Preferred gas limit for execution payload
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
}

impl SignedRoot for ProposerPreferences {}

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(ProposerPreferences);
}
