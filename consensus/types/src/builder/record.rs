use crate::test_utils::TestRandom;
use crate::{Address, ChainSpec, Epoch, ForkName};
use bls::PublicKeyBytes;
use context_deserialize::context_deserialize;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

pub type BuilderIndex = u64;

/// Represents a registered builder in the beacon state.
///
/// Builders are separate from validators and can submit execution payload bids.
/// They must register with a deposit and maintain sufficient balance to cover bids.
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode, TestRandom, TreeHash,
)]
#[context_deserialize(ForkName)]
pub struct Builder {
    pub pubkey: PublicKeyBytes,
    #[serde(with = "serde_utils::quoted_u8")]
    pub version: u8,
    pub execution_address: Address,
    #[serde(with = "serde_utils::quoted_u64")]
    pub balance: u64,
    pub deposit_epoch: Epoch,
    pub withdrawable_epoch: Epoch,
}

impl Builder {
    /// Check if a builder is active in a state with `finalized_epoch`.
    ///
    /// This implements `is_active_builder` from the spec.
    /// A builder is active if:
    /// - It was deposited before the finalized epoch
    /// - It is not yet withdrawable (withdrawable_epoch == FAR_FUTURE_EPOCH)
    pub fn is_active_at_finalized_epoch(&self, finalized_epoch: Epoch, spec: &ChainSpec) -> bool {
        self.deposit_epoch < finalized_epoch && self.withdrawable_epoch == spec.far_future_epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FixedBytesExtended;
    use bls::PublicKeyBytes;

    ssz_and_tree_hash_tests!(Builder);

    fn make_builder(deposit_epoch: u64, withdrawable_epoch: u64) -> Builder {
        Builder {
            pubkey: PublicKeyBytes::empty(),
            version: 0,
            execution_address: Address::zero(),
            balance: 1_000_000,
            deposit_epoch: Epoch::new(deposit_epoch),
            withdrawable_epoch: Epoch::new(withdrawable_epoch),
        }
    }

    #[test]
    fn active_builder() {
        let spec = ChainSpec::minimal();
        let builder = make_builder(0, spec.far_future_epoch.as_u64());
        assert!(builder.is_active_at_finalized_epoch(Epoch::new(1), &spec));
    }

    #[test]
    fn inactive_deposit_not_before_finalized() {
        // deposit_epoch == finalized_epoch → not strictly less than
        let spec = ChainSpec::minimal();
        let builder = make_builder(5, spec.far_future_epoch.as_u64());
        assert!(!builder.is_active_at_finalized_epoch(Epoch::new(5), &spec));
    }

    #[test]
    fn inactive_deposit_after_finalized() {
        let spec = ChainSpec::minimal();
        let builder = make_builder(10, spec.far_future_epoch.as_u64());
        assert!(!builder.is_active_at_finalized_epoch(Epoch::new(5), &spec));
    }

    #[test]
    fn inactive_exiting_builder() {
        // withdrawable_epoch != FAR_FUTURE_EPOCH → exiting/exited
        let spec = ChainSpec::minimal();
        let builder = make_builder(0, 100);
        assert!(!builder.is_active_at_finalized_epoch(Epoch::new(5), &spec));
    }

    #[test]
    fn inactive_epoch_zero() {
        // deposit_epoch=0, finalized_epoch=0 → 0 < 0 is false
        let spec = ChainSpec::minimal();
        let builder = make_builder(0, spec.far_future_epoch.as_u64());
        assert!(!builder.is_active_at_finalized_epoch(Epoch::new(0), &spec));
    }
}
