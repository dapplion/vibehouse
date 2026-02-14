use crate::{EthSpec, ExecutionPayloadEnvelope, test_utils::TestRandom};
use bls::Signature;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use tree_hash_derive::TreeHash;

/// Signed execution payload envelope in Gloas ePBS.
///
/// The builder signs the execution payload envelope to prove they are
/// authorizing the reveal of the payload. This signature is verified
/// using the DOMAIN_BEACON_BUILDER domain.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#signedexecutionpayloadenvelope
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
pub struct SignedExecutionPayloadEnvelope<E: EthSpec> {
    /// The execution payload envelope being signed
    pub message: ExecutionPayloadEnvelope<E>,
    /// BLS signature from the builder
    pub signature: Signature,
}

impl<E: EthSpec> SignedExecutionPayloadEnvelope<E> {
    /// Create an empty signed execution payload envelope (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            message: ExecutionPayloadEnvelope::empty(),
            signature: Signature::empty(),
        }
    }
}

impl<E: EthSpec> Default for SignedExecutionPayloadEnvelope<E> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<E: EthSpec> TestRandom for SignedExecutionPayloadEnvelope<E> {
    fn random_for_test(rng: &mut impl rand::RngCore) -> Self {
        Self {
            message: ExecutionPayloadEnvelope::random_for_test(rng),
            signature: Signature::random_for_test(rng),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(SignedExecutionPayloadEnvelope<MainnetEthSpec>);

    #[test]
    fn test_empty_signed_execution_payload_envelope() {
        let signed_envelope = SignedExecutionPayloadEnvelope::<MainnetEthSpec>::empty();
        assert_eq!(signed_envelope.message.builder_index, 0);
        assert!(signed_envelope.signature.is_empty());
    }
}
