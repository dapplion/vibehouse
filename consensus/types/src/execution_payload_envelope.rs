use crate::{
    EthSpec, ExecutionPayload, ExecutionRequests, ForkName, Hash256, Slot, test_utils::TestRandom,
};
use context_deserialize::context_deserialize;
use educe::Educe;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
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
#[derive(Debug, Clone, Serialize, Encode, Decode, Deserialize, TreeHash, Educe, TestRandom)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[educe(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct ExecutionPayloadEnvelope<E: EthSpec> {
    /// The execution payload being revealed
    pub payload: ExecutionPayload<E>,
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

impl<E: EthSpec> ExecutionPayloadEnvelope<E> {
    /// Create an empty execution payload envelope (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            payload: ExecutionPayload::default(),
            execution_requests: ExecutionRequests::default(),
            builder_index: 0,
            beacon_block_root: Hash256::zero(),
            slot: Slot::new(0),
            state_root: Hash256::zero(),
        }
    }
}

impl<E: EthSpec> Default for ExecutionPayloadEnvelope<E> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(ExecutionPayloadEnvelope<MainnetEthSpec>);

    #[test]
    fn test_empty_execution_payload_envelope() {
        let envelope = ExecutionPayloadEnvelope::<MainnetEthSpec>::empty();
        assert_eq!(envelope.builder_index, 0);
        assert_eq!(envelope.slot, Slot::new(0));
        assert_eq!(envelope.beacon_block_root, Hash256::zero());
        assert_eq!(envelope.state_root, Hash256::zero());
    }
}
