use crate::ExecutionPayloadBid;
use crate::test_utils::TestRandom;
use crate::{EthSpec, ForkName};
use bls::Signature;
use context_deserialize::context_deserialize;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Signed execution payload bid for Gloas ePBS.
///
/// Builders sign their bids to prove authenticity. The proposer verifies the
/// signature against the builder's registered public key before selecting a bid.
///
/// For self-builds (builder_index == BUILDER_INDEX_SELF_BUILD), the signature
/// must be the infinity point (empty signature) and value must be 0.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#signedexecutionpayloadbid
#[derive(
    TestRandom, TreeHash, Debug, Clone, Encode, Decode, Serialize, Deserialize, Derivative,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash(bound = "E: EthSpec"))]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct SignedExecutionPayloadBid<E: EthSpec> {
    pub message: ExecutionPayloadBid<E>,
    pub signature: Signature,
}

impl<E: EthSpec> SignedExecutionPayloadBid<E> {
    /// Create an empty signed bid (useful for defaults and testing)
    pub fn empty() -> Self {
        Self {
            message: ExecutionPayloadBid::default(),
            signature: Signature::empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionBlockHash, Hash256, MainnetEthSpec, MinimalEthSpec, Slot};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(SignedExecutionPayloadBid<MainnetEthSpec>);

    #[test]
    fn empty_fields() {
        let signed = SignedExecutionPayloadBid::<E>::empty();
        assert_eq!(signed.message.builder_index, 0);
        assert_eq!(signed.message.value, 0);
        assert_eq!(signed.message.slot, Slot::new(0));
        assert!(signed.signature.is_empty());
    }

    #[test]
    fn ssz_roundtrip_empty() {
        let signed = SignedExecutionPayloadBid::<E>::empty();
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadBid::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn ssz_roundtrip_non_default_bid() {
        let mut signed = SignedExecutionPayloadBid::<E>::empty();
        signed.message.builder_index = 42;
        signed.message.value = 1_000_000;
        signed.message.slot = Slot::new(99);
        signed.message.block_hash = ExecutionBlockHash::repeat_byte(0xaa);
        signed.message.parent_block_root = Hash256::repeat_byte(0xbb);

        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadBid::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
        assert_eq!(decoded.message.builder_index, 42);
        assert_eq!(decoded.message.value, 1_000_000);
    }

    #[test]
    fn self_build_bid() {
        let mut signed = SignedExecutionPayloadBid::<E>::empty();
        signed.message.builder_index = u64::MAX; // BUILDER_INDEX_SELF_BUILD
        signed.message.value = 0;
        // Self-build uses empty (infinity point) signature
        assert!(signed.signature.is_empty());

        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadBid::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.message.builder_index, u64::MAX);
        assert_eq!(decoded.message.value, 0);
    }

    #[test]
    fn tree_hash_changes_with_bid_value() {
        let mut signed1 = SignedExecutionPayloadBid::<E>::empty();
        let mut signed2 = SignedExecutionPayloadBid::<E>::empty();
        signed1.message.value = 100;
        signed2.message.value = 200;
        assert_ne!(signed1.tree_hash_root(), signed2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let mut signed = SignedExecutionPayloadBid::<E>::empty();
        signed.message.builder_index = 7;
        assert_eq!(signed.tree_hash_root(), signed.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let mut signed = SignedExecutionPayloadBid::<E>::empty();
        signed.message.value = 999;
        assert_eq!(signed, signed.clone());
    }
}
