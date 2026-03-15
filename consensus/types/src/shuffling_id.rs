use crate::*;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use std::hash::Hash;

/// Can be used to key (ID) the shuffling in some chain, in some epoch.
///
/// ## Reasoning
///
/// We say that the ID of some shuffling is always equal to a 2-tuple:
///
/// - The epoch for which the shuffling should be effective.
/// - A block root, where this is the root at the *last* slot of the penultimate epoch. I.e., the
///   final block which contributed a randao reveal to the seed for the shuffling.
///
/// The struct stores exactly that 2-tuple.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct AttestationShufflingId {
    pub shuffling_epoch: Epoch,
    pub shuffling_decision_block: Hash256,
}

impl AttestationShufflingId {
    /// Using the given `state`, return the shuffling id for the shuffling at the given
    /// `relative_epoch`.
    ///
    /// The `block_root` provided should be either:
    ///
    /// - The root of the block which produced this state.
    /// - If the state is from a skip slot, the root of the latest block in that state.
    pub fn new<E: EthSpec>(
        block_root: Hash256,
        state: &BeaconState<E>,
        relative_epoch: RelativeEpoch,
    ) -> Result<Self, BeaconStateError> {
        let shuffling_epoch = relative_epoch.into_epoch(state.current_epoch());

        let shuffling_decision_block =
            state.attester_shuffling_decision_root(block_root, relative_epoch)?;

        Ok(Self {
            shuffling_epoch,
            shuffling_decision_block,
        })
    }

    pub fn from_components(shuffling_epoch: Epoch, shuffling_decision_block: Hash256) -> Self {
        Self {
            shuffling_epoch,
            shuffling_decision_block,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn from_components_roundtrip() {
        let epoch = Epoch::new(42);
        let block = Hash256::from_low_u64_be(99);
        let id = AttestationShufflingId::from_components(epoch, block);
        assert_eq!(id.shuffling_epoch, epoch);
        assert_eq!(id.shuffling_decision_block, block);
    }

    #[test]
    fn equality() {
        let id1 =
            AttestationShufflingId::from_components(Epoch::new(5), Hash256::from_low_u64_be(10));
        let id2 =
            AttestationShufflingId::from_components(Epoch::new(5), Hash256::from_low_u64_be(10));
        assert_eq!(id1, id2);
    }

    #[test]
    fn inequality_different_epoch() {
        let id1 =
            AttestationShufflingId::from_components(Epoch::new(5), Hash256::from_low_u64_be(10));
        let id2 =
            AttestationShufflingId::from_components(Epoch::new(6), Hash256::from_low_u64_be(10));
        assert_ne!(id1, id2);
    }

    #[test]
    fn inequality_different_block() {
        let id1 =
            AttestationShufflingId::from_components(Epoch::new(5), Hash256::from_low_u64_be(10));
        let id2 =
            AttestationShufflingId::from_components(Epoch::new(5), Hash256::from_low_u64_be(11));
        assert_ne!(id1, id2);
    }

    #[test]
    fn hash_set_dedup() {
        let id1 =
            AttestationShufflingId::from_components(Epoch::new(1), Hash256::from_low_u64_be(1));
        let id2 =
            AttestationShufflingId::from_components(Epoch::new(1), Hash256::from_low_u64_be(1));
        let id3 =
            AttestationShufflingId::from_components(Epoch::new(2), Hash256::from_low_u64_be(1));
        let mut set = HashSet::new();
        set.insert(id1);
        set.insert(id2);
        set.insert(id3);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn clone_and_copy() {
        let id =
            AttestationShufflingId::from_components(Epoch::new(7), Hash256::from_low_u64_be(42));
        let cloned = id;
        assert_eq!(id, cloned);
    }

    #[test]
    fn ssz_roundtrip() {
        use ssz::{Decode, Encode};
        let id =
            AttestationShufflingId::from_components(Epoch::new(100), Hash256::from_low_u64_be(200));
        let encoded = id.as_ssz_bytes();
        let decoded = AttestationShufflingId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(id, decoded);
    }
}
