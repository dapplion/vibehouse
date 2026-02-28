use crate::{EthSpec, ExecutionPayloadEnvelope, ForkName, test_utils::TestRandom};
use bls::Signature;
use context_deserialize::context_deserialize;
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
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#signedexecutionpayloadenvelope>
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
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
    use crate::{Hash256, MainnetEthSpec, MinimalEthSpec, Slot};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(SignedExecutionPayloadEnvelope<MainnetEthSpec>);

    #[test]
    fn test_empty_signed_execution_payload_envelope() {
        let signed_envelope = SignedExecutionPayloadEnvelope::<MainnetEthSpec>::empty();
        assert_eq!(signed_envelope.message.builder_index, 0);
        assert!(signed_envelope.signature.is_empty());
    }

    #[test]
    fn default_equals_empty() {
        let a = SignedExecutionPayloadEnvelope::<E>::default();
        let b = SignedExecutionPayloadEnvelope::<E>::empty();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_has_default_message() {
        let signed = SignedExecutionPayloadEnvelope::<E>::empty();
        assert_eq!(signed.message, ExecutionPayloadEnvelope::empty());
        assert_eq!(signed.message.slot, Slot::new(0));
        assert_eq!(signed.message.beacon_block_root, Hash256::ZERO);
        assert_eq!(signed.message.state_root, Hash256::ZERO);
    }

    #[test]
    fn ssz_roundtrip_empty() {
        let signed = SignedExecutionPayloadEnvelope::<E>::empty();
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let mut signed = SignedExecutionPayloadEnvelope::<E>::empty();
        signed.message.builder_index = 42;
        signed.message.slot = Slot::new(100);
        signed.message.beacon_block_root = Hash256::repeat_byte(0xaa);
        signed.message.state_root = Hash256::repeat_byte(0xbb);

        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
        assert_eq!(decoded.message.builder_index, 42);
    }

    #[test]
    fn ssz_roundtrip_random() {
        use crate::test_utils::{SeedableRng, TestRandom, XorShiftRng};
        let mut rng = XorShiftRng::from_seed([42; 16]);
        let signed = SignedExecutionPayloadEnvelope::<E>::random_for_test(&mut rng);

        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn self_build_builder_index() {
        let mut signed = SignedExecutionPayloadEnvelope::<E>::empty();
        signed.message.builder_index = u64::MAX; // BUILDER_INDEX_SELF_BUILD

        let bytes = signed.as_ssz_bytes();
        let decoded = SignedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(decoded.message.builder_index, u64::MAX);
    }

    #[test]
    fn tree_hash_changes_with_builder_index() {
        let mut s1 = SignedExecutionPayloadEnvelope::<E>::empty();
        let mut s2 = SignedExecutionPayloadEnvelope::<E>::empty();
        s1.message.builder_index = 1;
        s2.message.builder_index = 2;
        assert_ne!(s1.tree_hash_root(), s2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let mut signed = SignedExecutionPayloadEnvelope::<E>::empty();
        signed.message.builder_index = 7;
        assert_eq!(signed.tree_hash_root(), signed.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let mut signed = SignedExecutionPayloadEnvelope::<E>::empty();
        signed.message.builder_index = 99;
        assert_eq!(signed, signed.clone());
    }

    #[test]
    fn different_messages_not_equal() {
        let mut s1 = SignedExecutionPayloadEnvelope::<E>::empty();
        let mut s2 = SignedExecutionPayloadEnvelope::<E>::empty();
        s1.message.slot = Slot::new(1);
        s2.message.slot = Slot::new(2);
        assert_ne!(s1, s2);
    }
}
