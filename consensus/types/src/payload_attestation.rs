use crate::{EthSpec, PayloadAttestationData, test_utils::TestRandom};
use bls::Signature;
use derivative::Derivative;
use safe_arith::ArithError;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use ssz_types::BitVector;
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Aggregated payload attestation from PTC members in Gloas ePBS.
///
/// The PTC (Payload Timeliness Committee) is a subset of 512 validators
/// selected per slot who attest to payload delivery and blob availability.
/// This aggregated attestation combines multiple individual PTC member
/// attestations using a bitvector and aggregate signature.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#payloadattestation
#[derive(
    Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative, TestRandom,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary),
    arbitrary(bound = "E: EthSpec")
)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
pub struct PayloadAttestation<E: EthSpec> {
    /// Bitfield indicating which PTC members signed this attestation
    /// PTC_SIZE = 512 validators per slot
    #[serde(with = "serde_utils::bitvec")]
    pub aggregation_bits: BitVector<E::PtcSize>,
    /// The attestation data being signed
    pub data: PayloadAttestationData,
    /// BLS aggregate signature from all attesting PTC members
    pub signature: Signature,
}

impl<E: EthSpec> PayloadAttestation<E> {
    /// Returns the number of set bits in the aggregation bitfield.
    pub fn num_attesters(&self) -> usize {
        self.aggregation_bits.num_set_bits()
    }

    /// Create an empty payload attestation (used for defaults/testing).
    pub fn empty() -> Self {
        Self {
            aggregation_bits: BitVector::new(),
            data: PayloadAttestationData {
                beacon_block_root: Default::default(),
                slot: Default::default(),
                payload_present: false,
                blob_data_available: false,
            },
            signature: Signature::empty(),
        }
    }
}

impl<E: EthSpec> Default for PayloadAttestation<E> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MainnetEthSpec;

    ssz_and_tree_hash_tests!(PayloadAttestation<MainnetEthSpec>);

    #[test]
    fn test_empty_payload_attestation() {
        let attestation = PayloadAttestation::<MainnetEthSpec>::empty();
        assert_eq!(attestation.num_attesters(), 0);
        assert!(!attestation.data.payload_present);
        assert!(!attestation.data.blob_data_available);
    }
}
