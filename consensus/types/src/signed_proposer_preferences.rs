use crate::test_utils::TestRandom;
use crate::{ForkName, ProposerPreferences};
use bls::Signature;
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Signed proposer preferences for Gloas ePBS.
///
/// Validators sign their preferences to prove authenticity. The signature is
/// verified against the validator's public key before forwarding on gossip.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/dev/specs/gloas/p2p-interface.md#new-signedproposerpreferences
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, TreeHash, TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct SignedProposerPreferences {
    pub message: ProposerPreferences,
    pub signature: Signature,
}

impl SignedProposerPreferences {
    /// Create an empty signed proposer preferences (useful for defaults and testing)
    pub fn empty() -> Self {
        Self {
            message: ProposerPreferences::default(),
            signature: Signature::empty(),
        }
    }
}

impl Default for SignedProposerPreferences {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    ssz_and_tree_hash_tests!(SignedProposerPreferences);
}
