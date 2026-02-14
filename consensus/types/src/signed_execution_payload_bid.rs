use crate::ExecutionPayloadBid;
use crate::test_utils::TestRandom;
use crate::{EthSpec, ForkName};
use bls::Signature;
use context_deserialize::context_deserialize;
use educe::Educe;
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
#[derive(TestRandom, TreeHash, Debug, Clone, Encode, Decode, Serialize, Deserialize, Educe)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[educe(PartialEq, Hash)]
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
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(SignedExecutionPayloadBid<MainnetEthSpec>);
}
