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
    use crate::Address;
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(SignedProposerPreferences);

    fn make_signed_prefs(slot: u64, index: u64) -> SignedProposerPreferences {
        SignedProposerPreferences {
            message: ProposerPreferences {
                proposal_slot: slot,
                validator_index: index,
                fee_recipient: Address::repeat_byte(0x42),
                gas_limit: 30_000_000,
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn empty_signature_roundtrips() {
        let signed = make_signed_prefs(10, 5);
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedProposerPreferences::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn ssz_roundtrip_non_default_message() {
        let signed = SignedProposerPreferences {
            message: ProposerPreferences {
                proposal_slot: 999,
                validator_index: 42,
                fee_recipient: Address::repeat_byte(0xAB),
                gas_limit: 50_000_000,
            },
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedProposerPreferences::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn tree_hash_changes_with_message() {
        let a = make_signed_prefs(1, 5);
        let b = make_signed_prefs(2, 5);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let signed = make_signed_prefs(42, 7);
        assert_eq!(signed.tree_hash_root(), signed.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let signed = make_signed_prefs(100, 200);
        assert_eq!(signed.clone(), signed);
    }

    #[test]
    fn different_messages_not_equal() {
        assert_ne!(make_signed_prefs(1, 5), make_signed_prefs(2, 5));
    }

    #[test]
    fn message_accessible() {
        let signed = make_signed_prefs(42, 7);
        assert_eq!(signed.message.proposal_slot, 42);
        assert_eq!(signed.message.validator_index, 7);
        assert_eq!(signed.message.gas_limit, 30_000_000);
    }

    #[test]
    fn hash_impl_consistent() {
        use std::collections::HashSet;
        let signed = make_signed_prefs(42, 7);
        let mut set = HashSet::new();
        set.insert(signed.clone());
        assert!(set.contains(&make_signed_prefs(42, 7)));
        assert!(!set.contains(&make_signed_prefs(43, 7)));
    }
}
