use crate::test_utils::TestRandom;
use crate::{BuilderPendingWithdrawal, ForkName};
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Represents a pending payment to a builder.
///
/// Created when a proposer selects a builder's bid. The payment is held
/// pending until the Payload Timeliness Committee (PTC) attests to payload
/// delivery. The `weight` field accumulates attestations from PTC members.
///
/// Payments are processed at epoch boundaries if the PTC quorum is met
/// (typically 60% of PTC stake).
#[derive(
    Debug,
    PartialEq,
    Eq,
    Hash,
    Clone,
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
pub struct BuilderPendingPayment {
    /// Accumulated weight from PTC attestations. When weight ≥ quorum threshold,
    /// the payment is released to the builder.
    #[serde(with = "serde_utils::quoted_u64")]
    pub weight: u64,
    /// The withdrawal details: recipient address, amount, and builder index.
    pub withdrawal: BuilderPendingWithdrawal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Address;
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    ssz_and_tree_hash_tests!(BuilderPendingPayment);

    fn make_payment(weight: u64, amount: u64, builder_index: u64) -> BuilderPendingPayment {
        BuilderPendingPayment {
            weight,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0x42),
                amount,
                builder_index,
            },
        }
    }

    #[test]
    fn default_is_zero() {
        let payment = BuilderPendingPayment::default();
        assert_eq!(payment.weight, 0);
        assert_eq!(payment.withdrawal.amount, 0);
        assert_eq!(payment.withdrawal.builder_index, 0);
        assert_eq!(payment.withdrawal.fee_recipient, Address::ZERO);
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let payment = make_payment(100, 1_000_000, 7);
        let bytes = payment.as_ssz_bytes();
        let decoded = BuilderPendingPayment::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(payment, decoded);
    }

    #[test]
    fn ssz_roundtrip_max_values() {
        let payment = BuilderPendingPayment {
            weight: u64::MAX,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xFF),
                amount: u64::MAX,
                builder_index: u64::MAX,
            },
        };
        let bytes = payment.as_ssz_bytes();
        let decoded = BuilderPendingPayment::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(payment, decoded);
    }

    #[test]
    fn tree_hash_changes_with_weight() {
        let a = make_payment(0, 1000, 7);
        let b = make_payment(100, 1000, 7);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_amount() {
        let a = make_payment(100, 1000, 7);
        let b = make_payment(100, 2000, 7);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_builder_index() {
        let a = make_payment(100, 1000, 0);
        let b = make_payment(100, 1000, 1);
        assert_ne!(a.tree_hash_root(), b.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let payment = make_payment(100, 1000, 7);
        assert_eq!(payment.tree_hash_root(), payment.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let payment = make_payment(50, 500_000, 3);
        assert_eq!(payment.clone(), payment);
    }

    #[test]
    fn different_weights_not_equal() {
        assert_ne!(make_payment(0, 1000, 7), make_payment(1, 1000, 7));
    }

    #[test]
    fn hash_impl_consistent() {
        use std::collections::HashSet;
        let payment = make_payment(100, 1000, 7);
        let mut set = HashSet::new();
        set.insert(payment.clone());
        assert!(set.contains(&make_payment(100, 1000, 7)));
        assert!(!set.contains(&make_payment(200, 1000, 7)));
    }
}
