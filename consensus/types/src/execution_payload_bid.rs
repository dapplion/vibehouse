use crate::beacon_block_body::KzgCommitments;
use crate::test_utils::TestRandom;
use crate::{Address, EthSpec, ExecutionBlockHash, ForkName, Hash256, SignedRoot, Slot};
use context_deserialize::context_deserialize;
use educe::Educe;
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
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#executionpayloadbid
#[derive(
    Default, Debug, Clone, Serialize, Encode, Decode, Deserialize, TreeHash, Educe, TestRandom,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[educe(PartialEq, Hash)]
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
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(ExecutionPayloadBid<MainnetEthSpec>);
}
