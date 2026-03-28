use crate::test_utils::TestRandom;
use crate::{EthSpec, ForkName, Hash256, Slot};
use context_deserialize::context_deserialize;
use educe::Educe;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// A single transaction in an inclusion list.
///
/// Bounded by `MaxBytesPerTransaction`.
pub type InclusionListTransaction<E> = VariableList<u8, <E as EthSpec>::MaxBytesPerTransaction>;

/// Inclusion list submitted by a member of the inclusion list committee.
///
/// Contains the transactions that the committee member asserts should be included
/// in the execution payload. Validators in the inclusion list committee (16 per slot)
/// broadcast these during the slot to enforce transaction inclusion.
///
/// Spec: <https://github.com/ethereum/consensus-specs/blob/master/specs/heze/beacon-chain.md#inclusionlist>
#[derive(TestRandom, TreeHash, Debug, Clone, Encode, Decode, Serialize, Deserialize, Educe)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound(E: EthSpec))
)]
#[educe(PartialEq, Eq, Hash(bound(E: EthSpec)))]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct InclusionList<E: EthSpec> {
    pub slot: Slot,
    #[serde(with = "serde_utils::quoted_u64")]
    pub validator_index: u64,
    pub inclusion_list_committee_root: Hash256,
    pub transactions:
        VariableList<InclusionListTransaction<E>, <E as EthSpec>::MaxTransactionsPerPayload>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MainnetEthSpec, MinimalEthSpec};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(InclusionList<MainnetEthSpec>);

    #[test]
    fn default_roundtrip() {
        let il = InclusionList::<E> {
            slot: Slot::new(0),
            validator_index: 0,
            inclusion_list_committee_root: Hash256::ZERO,
            transactions: <_>::default(),
        };
        let bytes = il.as_ssz_bytes();
        let decoded = InclusionList::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(il, decoded);
    }

    #[test]
    fn non_default_roundtrip() {
        let il = InclusionList::<E> {
            slot: Slot::new(42),
            validator_index: 7,
            inclusion_list_committee_root: Hash256::repeat_byte(0xab),
            transactions: <_>::default(),
        };
        let bytes = il.as_ssz_bytes();
        let decoded = InclusionList::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(il, decoded);
        assert_eq!(decoded.slot, Slot::new(42));
        assert_eq!(decoded.validator_index, 7);
    }

    #[test]
    fn tree_hash_changes_with_slot() {
        let il1 = InclusionList::<E> {
            slot: Slot::new(1),
            validator_index: 0,
            inclusion_list_committee_root: Hash256::ZERO,
            transactions: <_>::default(),
        };
        let il2 = InclusionList::<E> {
            slot: Slot::new(2),
            ..il1.clone()
        };
        assert_ne!(il1.tree_hash_root(), il2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let il = InclusionList::<E> {
            slot: Slot::new(10),
            validator_index: 5,
            inclusion_list_committee_root: Hash256::repeat_byte(0x01),
            transactions: <_>::default(),
        };
        assert_eq!(il.tree_hash_root(), il.tree_hash_root());
    }
}
