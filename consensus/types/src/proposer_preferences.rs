use crate::{Address, ForkName, SignedRoot, test_utils::TestRandom};
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
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/p2p-interface.md>
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

impl SignedRoot for ProposerPreferences {}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(ProposerPreferences);

    fn make_prefs(slot: u64, index: u64, gas: u64) -> ProposerPreferences {
        ProposerPreferences {
            proposal_slot: slot,
            validator_index: index,
            fee_recipient: Address::repeat_byte(0x42),
            gas_limit: gas,
        }
    }

    #[test]
    fn default_fields_are_zero() {
        let prefs = ProposerPreferences {
            proposal_slot: 0,
            validator_index: 0,
            fee_recipient: Address::ZERO,
            gas_limit: 0,
        };
        assert_eq!(prefs.proposal_slot, 0);
        assert_eq!(prefs.validator_index, 0);
        assert_eq!(prefs.fee_recipient, Address::ZERO);
        assert_eq!(prefs.gas_limit, 0);
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let prefs = make_prefs(42, 7, 30_000_000);
        let bytes = prefs.as_ssz_bytes();
        let decoded = ProposerPreferences::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(prefs, decoded);
    }

    #[test]
    fn ssz_roundtrip_max_values() {
        let prefs = ProposerPreferences {
            proposal_slot: u64::MAX,
            validator_index: u64::MAX,
            fee_recipient: Address::repeat_byte(0xFF),
            gas_limit: u64::MAX,
        };
        let bytes = prefs.as_ssz_bytes();
        let decoded = ProposerPreferences::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(prefs, decoded);
    }

    #[test]
    fn tree_hash_changes_with_slot() {
        let a = make_prefs(1, 7, 30_000_000);
        let b = make_prefs(2, 7, 30_000_000);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_validator_index() {
        let a = make_prefs(42, 0, 30_000_000);
        let b = make_prefs(42, 1, 30_000_000);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_fee_recipient() {
        let a = ProposerPreferences {
            fee_recipient: Address::repeat_byte(0x01),
            ..make_prefs(42, 7, 30_000_000)
        };
        let b = ProposerPreferences {
            fee_recipient: Address::repeat_byte(0x02),
            ..make_prefs(42, 7, 30_000_000)
        };
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_gas_limit() {
        let a = make_prefs(42, 7, 30_000_000);
        let b = make_prefs(42, 7, 60_000_000);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let prefs = make_prefs(42, 7, 30_000_000);
        assert_eq!(prefs.tree_hash_root(), prefs.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let prefs = make_prefs(100, 200, 50_000_000);
        assert_eq!(prefs.clone(), prefs);
    }

    #[test]
    fn different_slots_not_equal() {
        assert_ne!(make_prefs(1, 7, 30_000_000), make_prefs(2, 7, 30_000_000));
    }

    #[test]
    fn different_gas_limits_not_equal() {
        assert_ne!(make_prefs(42, 7, 30_000_000), make_prefs(42, 7, 60_000_000));
    }

    #[test]
    fn hash_impl_consistent() {
        use std::collections::HashSet;
        let prefs = make_prefs(42, 7, 30_000_000);
        let mut set = HashSet::new();
        set.insert(prefs.clone());
        assert!(set.contains(&make_prefs(42, 7, 30_000_000)));
        assert!(!set.contains(&make_prefs(42, 7, 60_000_000)));
    }
}
