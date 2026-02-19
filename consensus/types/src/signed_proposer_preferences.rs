use crate::{ForkName, ProposerPreferences, test_utils::TestRandom};
use bls::Signature;
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Signed proposer preferences for Gloas ePBS.
///
/// Validators sign their preferences to prove authenticity. The signature is
/// verified against the validator's public key using DOMAIN_PROPOSER_PREFERENCES.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/p2p-interface.md
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct SignedProposerPreferences {
    pub message: ProposerPreferences,
    pub signature: Signature,
}

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(SignedProposerPreferences);
}
