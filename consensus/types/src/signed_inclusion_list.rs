use crate::test_utils::TestRandom;
use crate::{EthSpec, ForkName, InclusionList, SignedRoot};
use bls::Signature;
use context_deserialize::context_deserialize;
use educe::Educe;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Signed inclusion list for Heze FOCIL.
///
/// Committee members sign their inclusion lists to prove authenticity.
/// The signature is verified against the validator's public key using
/// the `DOMAIN_INCLUSION_LIST_COMMITTEE` domain.
///
/// Spec: <https://github.com/ethereum/consensus-specs/blob/master/specs/heze/beacon-chain.md#signedinclusionlist>
#[derive(TestRandom, TreeHash, Debug, Clone, Encode, Decode, Serialize, Deserialize, Educe)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound(E: EthSpec))
)]
#[educe(PartialEq, Hash(bound(E: EthSpec)))]
#[serde(bound = "E: EthSpec")]
#[context_deserialize(ForkName)]
pub struct SignedInclusionList<E: EthSpec> {
    pub message: InclusionList<E>,
    pub signature: Signature,
}

impl<E: EthSpec> SignedRoot for InclusionList<E> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hash256, MainnetEthSpec, MinimalEthSpec, Slot};
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    type E = MinimalEthSpec;

    ssz_and_tree_hash_tests!(SignedInclusionList<MainnetEthSpec>);

    #[test]
    fn ssz_roundtrip_empty() {
        let signed = SignedInclusionList::<E> {
            message: InclusionList {
                slot: Slot::new(0),
                validator_index: 0,
                inclusion_list_committee_root: Hash256::ZERO,
                transactions: <_>::default(),
            },
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedInclusionList::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
    }

    #[test]
    fn ssz_roundtrip_non_default() {
        let signed = SignedInclusionList::<E> {
            message: InclusionList {
                slot: Slot::new(42),
                validator_index: 7,
                inclusion_list_committee_root: Hash256::repeat_byte(0xab),
                transactions: <_>::default(),
            },
            signature: Signature::empty(),
        };
        let bytes = signed.as_ssz_bytes();
        let decoded = SignedInclusionList::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(signed, decoded);
        assert_eq!(decoded.message.validator_index, 7);
    }

    #[test]
    fn tree_hash_changes_with_validator_index() {
        let il1 = SignedInclusionList::<E> {
            message: InclusionList {
                slot: Slot::new(1),
                validator_index: 0,
                inclusion_list_committee_root: Hash256::ZERO,
                transactions: <_>::default(),
            },
            signature: Signature::empty(),
        };
        let il2 = SignedInclusionList::<E> {
            message: InclusionList {
                validator_index: 1,
                ..il1.message.clone()
            },
            signature: Signature::empty(),
        };
        assert_ne!(il1.tree_hash_root(), il2.tree_hash_root());
    }

    #[test]
    fn tree_hash_deterministic() {
        let signed = SignedInclusionList::<E> {
            message: InclusionList {
                slot: Slot::new(10),
                validator_index: 5,
                inclusion_list_committee_root: Hash256::repeat_byte(0x01),
                transactions: <_>::default(),
            },
            signature: Signature::empty(),
        };
        assert_eq!(signed.tree_hash_root(), signed.tree_hash_root());
    }

    #[test]
    fn clone_preserves_equality() {
        let signed = SignedInclusionList::<E> {
            message: InclusionList {
                slot: Slot::new(42),
                validator_index: 7,
                inclusion_list_committee_root: Hash256::repeat_byte(0xab),
                transactions: <_>::default(),
            },
            signature: Signature::empty(),
        };
        assert_eq!(signed, signed.clone());
    }
}
