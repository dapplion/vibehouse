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

    ssz_and_tree_hash_tests!(BuilderPendingPayment);
}
