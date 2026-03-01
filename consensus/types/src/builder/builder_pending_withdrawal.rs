use crate::test_utils::TestRandom;
use crate::{Address, ForkName};
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Represents a pending withdrawal for a builder.
///
/// Created when a builder exits and enters the withdrawal queue.
/// Processed at epoch boundaries similar to validator withdrawals.
#[derive(
    Debug,
    PartialEq,
    Eq,
    Hash,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    TreeHash,
    TestRandom,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[context_deserialize(ForkName)]
pub struct BuilderPendingWithdrawal {
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
    #[serde(with = "serde_utils::quoted_u64")]
    pub amount: u64,
    #[serde(with = "serde_utils::quoted_u64")]
    pub builder_index: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(BuilderPendingWithdrawal);

    fn make_withdrawal(amount: u64, builder_index: u64) -> BuilderPendingWithdrawal {
        BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0x42),
            amount,
            builder_index,
        }
    }

    #[test]
    fn default_is_zero() {
        let w = BuilderPendingWithdrawal::default();
        assert_eq!(w.fee_recipient, Address::ZERO);
        assert_eq!(w.amount, 0);
        assert_eq!(w.builder_index, 0);
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let w = make_withdrawal(1_000_000, 7);
        let bytes = w.as_ssz_bytes();
        let decoded = BuilderPendingWithdrawal::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(w, decoded);
    }

    #[test]
    fn ssz_roundtrip_max_values() {
        let w = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xFF),
            amount: u64::MAX,
            builder_index: u64::MAX,
        };
        let bytes = w.as_ssz_bytes();
        let decoded = BuilderPendingWithdrawal::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(w, decoded);
    }

    #[test]
    fn tree_hash_changes_with_fee_recipient() {
        let a = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0x01),
            ..make_withdrawal(1000, 7)
        };
        let b = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0x02),
            ..make_withdrawal(1000, 7)
        };
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_amount() {
        let a = make_withdrawal(1000, 7);
        let b = make_withdrawal(2000, 7);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_builder_index() {
        let a = make_withdrawal(1000, 0);
        let b = make_withdrawal(1000, 1);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let w = make_withdrawal(1000, 7);
        assert_eq!(w.tree_hash_root(), w.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let w = make_withdrawal(500_000, 3);
        assert_eq!(w.clone(), w);
    }

    #[test]
    fn different_amounts_not_equal() {
        assert_ne!(make_withdrawal(1000, 7), make_withdrawal(2000, 7));
    }

    #[test]
    fn different_builder_indices_not_equal() {
        assert_ne!(make_withdrawal(1000, 0), make_withdrawal(1000, 1));
    }

    #[test]
    fn hash_impl_consistent() {
        use std::collections::HashSet;
        let w = make_withdrawal(1000, 7);
        let mut set = HashSet::new();
        set.insert(w);
        assert!(set.contains(&make_withdrawal(1000, 7)));
        assert!(!set.contains(&make_withdrawal(2000, 7)));
    }
}
