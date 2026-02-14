use crate::ExecutionPayloadBid;
use crate::test_utils::TestRandom;
use crate::{BeaconState, ChainSpec, EthSpec, ForkName, PublicKeyBytes, SignedRoot};
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

    /// Verify the signature of this bid against the builder's public key.
    ///
    /// For self-builds (builder_index == BUILDER_INDEX_SELF_BUILD), this always
    /// returns true since the signature should be empty.
    ///
    /// For external builders, retrieves the builder's pubkey from state and verifies
    /// the signature using DOMAIN_BEACON_BUILDER.
    pub fn verify_signature(
        &self,
        builder_pubkey: &PublicKeyBytes,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> bool {
        builder_pubkey
            .decompress()
            .map(|pubkey| {
                let domain = spec.get_builder_domain();
                let message = self.message.signing_root(domain);
                self.signature.verify(&pubkey, message)
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(SignedExecutionPayloadBid<MainnetEthSpec>);
}
