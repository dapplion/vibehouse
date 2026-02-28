use crate::beacon_block_body::KzgCommitments;
use crate::test_utils::TestRandom;
use crate::{Address, EthSpec, ExecutionBlockHash, ForkName, Hash256, SignedRoot, Slot};
use context_deserialize::context_deserialize;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Execution payload bid submitted by builders in Gloas ePBS.
///
/// The proposer selects the highest bid (or self-build with bid value 0).
/// The bid commits to the execution payload content via `block_hash` and
/// blob commitments. The actual payload is revealed later by the builder.
///
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#executionpayloadbid>
#[derive(
    Default, Debug, Clone, Serialize, Encode, Decode, Deserialize, TreeHash, Derivative, TestRandom,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash(bound = "E: EthSpec"))]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct ExecutionPayloadBid<E: EthSpec> {
    /// Hash of the parent execution block
    pub parent_block_hash: ExecutionBlockHash,
    /// Root of the parent beacon block
    pub parent_block_root: Hash256,
    /// Hash of the execution payload being bid on
    pub block_hash: ExecutionBlockHash,
    /// Previous RANDAO value from beacon state
    pub prev_randao: Hash256,
    /// Fee recipient address for this bid
    #[serde(with = "serde_utils::address_hex")]
    pub fee_recipient: Address,
    /// Gas limit for the execution payload
    #[serde(with = "serde_utils::quoted_u64")]
    pub gas_limit: u64,
    /// Index of the builder submitting this bid (or BUILDER_INDEX_SELF_BUILD for proposer)
    #[serde(with = "serde_utils::quoted_u64")]
    pub builder_index: u64,
    /// Slot this bid is for
    pub slot: Slot,
    /// Bid value in Gwei (amount builder pays to proposer)
    #[serde(with = "serde_utils::quoted_u64")]
    pub value: u64,
    /// Payment amount for execution (distinct from proposer payment)
    #[serde(with = "serde_utils::quoted_u64")]
    pub execution_payment: u64,
    /// KZG commitments for blobs included in this payload
    pub blob_kzg_commitments: KzgCommitments<E>,
}

impl<E: EthSpec> SignedRoot for ExecutionPayloadBid<E> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MainnetEthSpec, MinimalEthSpec};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(ExecutionPayloadBid<MainnetEthSpec>);

    #[test]
    fn default_fields_are_zero() {
        let bid = ExecutionPayloadBid::<E>::default();
        assert_eq!(bid.parent_block_hash, ExecutionBlockHash::zero());
        assert_eq!(bid.parent_block_root, Hash256::ZERO);
        assert_eq!(bid.block_hash, ExecutionBlockHash::zero());
        assert_eq!(bid.prev_randao, Hash256::ZERO);
        assert_eq!(bid.fee_recipient, Address::ZERO);
        assert_eq!(bid.gas_limit, 0);
        assert_eq!(bid.builder_index, 0);
        assert_eq!(bid.slot, Slot::new(0));
        assert_eq!(bid.value, 0);
        assert_eq!(bid.execution_payment, 0);
        assert!(bid.blob_kzg_commitments.is_empty());
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let bid = ExecutionPayloadBid::<E> {
            parent_block_hash: ExecutionBlockHash::repeat_byte(0x11),
            parent_block_root: Hash256::repeat_byte(0x22),
            block_hash: ExecutionBlockHash::repeat_byte(0x33),
            prev_randao: Hash256::repeat_byte(0x44),
            fee_recipient: Address::repeat_byte(0x55),
            gas_limit: 30_000_000,
            builder_index: 42,
            slot: Slot::new(100),
            value: 1_000_000_000,
            execution_payment: 500_000,
            blob_kzg_commitments: <_>::default(),
        };
        let bytes = bid.as_ssz_bytes();
        let decoded = ExecutionPayloadBid::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(bid, decoded);
    }

    #[test]
    fn self_build_sentinel_value() {
        // BUILDER_INDEX_SELF_BUILD = u64::MAX
        let bid = ExecutionPayloadBid::<E> {
            builder_index: u64::MAX,
            value: 0,
            ..Default::default()
        };

        let bytes = bid.as_ssz_bytes();
        let decoded = ExecutionPayloadBid::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.builder_index, u64::MAX);
        assert_eq!(decoded.value, 0);
    }

    #[test]
    fn tree_hash_changes_with_value() {
        let bid1 = ExecutionPayloadBid::<E> {
            value: 100,
            ..Default::default()
        };
        let bid2 = ExecutionPayloadBid::<E> {
            value: 200,
            ..Default::default()
        };
        assert_ne!(bid1.tree_hash_root(), bid2.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_block_hash() {
        let bid1 = ExecutionPayloadBid::<E> {
            block_hash: ExecutionBlockHash::repeat_byte(0x01),
            ..Default::default()
        };
        let bid2 = ExecutionPayloadBid::<E> {
            block_hash: ExecutionBlockHash::repeat_byte(0x02),
            ..Default::default()
        };
        assert_ne!(bid1.tree_hash_root(), bid2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let bid = ExecutionPayloadBid::<E> {
            slot: Slot::new(42),
            builder_index: 7,
            ..Default::default()
        };
        assert_eq!(bid.tree_hash_root(), bid.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let bid = ExecutionPayloadBid::<E> {
            gas_limit: 30_000_000,
            builder_index: 99,
            value: 1_000,
            ..Default::default()
        };
        assert_eq!(bid, bid.clone());
    }

    #[test]
    fn different_slots_not_equal() {
        let bid1 = ExecutionPayloadBid::<E> {
            slot: Slot::new(1),
            ..Default::default()
        };
        let bid2 = ExecutionPayloadBid::<E> {
            slot: Slot::new(2),
            ..Default::default()
        };
        assert_ne!(bid1, bid2);
    }

    #[test]
    fn different_builder_index_not_equal() {
        let bid1 = ExecutionPayloadBid::<E> {
            builder_index: 0,
            ..Default::default()
        };
        let bid2 = ExecutionPayloadBid::<E> {
            builder_index: 1,
            ..Default::default()
        };
        assert_ne!(bid1, bid2);
    }
}
