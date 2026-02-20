use crate::{
    EthSpec, ExecutionPayloadGloas, ExecutionRequests, ForkName, Hash256, SignedRoot, Slot,
    test_utils::TestRandom,
};
use context_deserialize::context_deserialize;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use tree_hash_derive::TreeHash;

/// Execution payload envelope submitted by builders in Gloas ePBS.
///
/// After the proposer commits to a bid, the builder reveals the actual
/// execution payload by submitting an ExecutionPayloadEnvelope. This contains
/// the full payload that was committed to in the bid's block_hash.
///
/// The envelope is signed by the builder to prove they authorized the reveal.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#executionpayloadenvelope
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct ExecutionPayloadEnvelope<E: EthSpec> {
    /// The execution payload being revealed
    pub payload: ExecutionPayloadGloas<E>,
    /// Execution layer requests (deposits, withdrawals, consolidations)
    pub execution_requests: ExecutionRequests<E>,
    /// Index of the builder revealing this payload
    #[serde(with = "serde_utils::quoted_u64")]
    pub builder_index: u64,
    /// Root of the beacon block this payload is for
    pub beacon_block_root: Hash256,
    /// Slot this payload is for (must match the committed bid)
    pub slot: Slot,
    /// Beacon state root after processing this payload
    pub state_root: Hash256,
}

impl<E: EthSpec> SignedRoot for ExecutionPayloadEnvelope<E> {}

impl<E: EthSpec> ExecutionPayloadEnvelope<E> {
    /// Create an empty execution payload envelope (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            payload: ExecutionPayloadGloas::default(),
            execution_requests: ExecutionRequests::default(),
            builder_index: 0,
            beacon_block_root: Hash256::ZERO,
            slot: Slot::new(0),
            state_root: Hash256::ZERO,
        }
    }
}

impl<E: EthSpec> Default for ExecutionPayloadEnvelope<E> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<E: EthSpec> TestRandom for ExecutionPayloadEnvelope<E> {
    fn random_for_test(rng: &mut impl rand::RngCore) -> Self {
        Self {
            payload: ExecutionPayloadGloas::random_for_test(rng),
            execution_requests: ExecutionRequests::random_for_test(rng),
            builder_index: u64::random_for_test(rng),
            beacon_block_root: Hash256::random_for_test(rng),
            slot: Slot::random_for_test(rng),
            state_root: Hash256::random_for_test(rng),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionBlockHash, MainnetEthSpec, MinimalEthSpec};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(ExecutionPayloadEnvelope<MainnetEthSpec>);

    #[test]
    fn test_empty_execution_payload_envelope() {
        let envelope = ExecutionPayloadEnvelope::<MainnetEthSpec>::empty();
        assert_eq!(envelope.builder_index, 0);
        assert_eq!(envelope.slot, Slot::new(0));
        assert_eq!(envelope.beacon_block_root, Hash256::ZERO);
        assert_eq!(envelope.state_root, Hash256::ZERO);
    }

    #[test]
    fn default_equals_empty() {
        let a = ExecutionPayloadEnvelope::<E>::default();
        let b = ExecutionPayloadEnvelope::<E>::empty();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_payload_is_default() {
        let envelope = ExecutionPayloadEnvelope::<E>::empty();
        assert_eq!(envelope.payload, ExecutionPayloadGloas::default());
        assert_eq!(envelope.execution_requests, ExecutionRequests::default());
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 42;
        envelope.slot = Slot::new(100);
        envelope.beacon_block_root = Hash256::repeat_byte(0xaa);
        envelope.state_root = Hash256::repeat_byte(0xbb);
        envelope.payload.block_hash = ExecutionBlockHash::repeat_byte(0xcc);

        let bytes = envelope.as_ssz_bytes();
        let decoded = ExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(envelope, decoded);
        assert_eq!(decoded.builder_index, 42);
        assert_eq!(decoded.slot, Slot::new(100));
        assert_eq!(
            decoded.payload.block_hash,
            ExecutionBlockHash::repeat_byte(0xcc)
        );
    }

    #[test]
    fn ssz_roundtrip_self_build() {
        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = u64::MAX; // BUILDER_INDEX_SELF_BUILD

        let bytes = envelope.as_ssz_bytes();
        let decoded = ExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.builder_index, u64::MAX);
    }

    #[test]
    fn ssz_roundtrip_random() {
        use crate::test_utils::{SeedableRng, TestRandom, XorShiftRng};
        let mut rng = XorShiftRng::from_seed([42; 16]);
        let envelope = ExecutionPayloadEnvelope::<E>::random_for_test(&mut rng);

        let bytes = envelope.as_ssz_bytes();
        let decoded = ExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(envelope, decoded);
    }

    #[test]
    fn tree_hash_changes_with_builder_index() {
        let mut env1 = ExecutionPayloadEnvelope::<E>::empty();
        let mut env2 = ExecutionPayloadEnvelope::<E>::empty();
        env1.builder_index = 1;
        env2.builder_index = 2;
        assert_ne!(env1.tree_hash_root(), env2.tree_hash_root());
    }

    #[test]
    fn tree_hash_changes_with_state_root() {
        let mut env1 = ExecutionPayloadEnvelope::<E>::empty();
        let mut env2 = ExecutionPayloadEnvelope::<E>::empty();
        env1.state_root = Hash256::repeat_byte(0x01);
        env2.state_root = Hash256::repeat_byte(0x02);
        assert_ne!(env1.tree_hash_root(), env2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 7;
        envelope.slot = Slot::new(42);
        assert_eq!(envelope.tree_hash_root(), envelope.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let mut envelope = ExecutionPayloadEnvelope::<E>::empty();
        envelope.builder_index = 99;
        envelope.beacon_block_root = Hash256::repeat_byte(0xdd);
        assert_eq!(envelope, envelope.clone());
    }

    #[test]
    fn different_slots_not_equal() {
        let mut env1 = ExecutionPayloadEnvelope::<E>::empty();
        let mut env2 = ExecutionPayloadEnvelope::<E>::empty();
        env1.slot = Slot::new(1);
        env2.slot = Slot::new(2);
        assert_ne!(env1, env2);
    }
}
