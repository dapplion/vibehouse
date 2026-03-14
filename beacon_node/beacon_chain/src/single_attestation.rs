use crate::attestation_verification::Error;
use types::{
    Attestation, AttestationBase, AttestationElectra, BitList, BitVector, EthSpec, ForkName,
    SingleAttestation,
};

/// Build an `Attestation` from a `SingleAttestation` using pre-computed aggregation bit position
/// and committee length, avoiding a full committee Vec allocation.
pub fn build_attestation_from_single<E: EthSpec>(
    single_attestation: &SingleAttestation,
    aggregation_bit: usize,
    committee_len: usize,
    fork_name: ForkName,
) -> Result<Attestation<E>, Error> {
    if fork_name.electra_enabled() {
        let mut committee_bits: BitVector<E::MaxCommitteesPerSlot> = BitVector::default();
        committee_bits
            .set(single_attestation.committee_index as usize, true)
            .map_err(|e| Error::Invalid(e.into()))?;

        let mut aggregation_bits =
            BitList::with_capacity(committee_len).map_err(|e| Error::Invalid(e.into()))?;
        aggregation_bits
            .set(aggregation_bit, true)
            .map_err(|e| Error::Invalid(e.into()))?;
        Ok(Attestation::Electra(AttestationElectra {
            aggregation_bits,
            committee_bits,
            data: single_attestation.data,
            signature: single_attestation.signature.clone(),
        }))
    } else {
        let mut aggregation_bits =
            BitList::with_capacity(committee_len).map_err(|e| Error::Invalid(e.into()))?;
        aggregation_bits
            .set(aggregation_bit, true)
            .map_err(|e| Error::Invalid(e.into()))?;
        Ok(Attestation::Base(AttestationBase {
            aggregation_bits,
            data: single_attestation.data,
            signature: single_attestation.signature.clone(),
        }))
    }
}

pub fn single_attestation_to_attestation<E: EthSpec>(
    single_attestation: &SingleAttestation,
    committee: &[usize],
    fork_name: ForkName,
) -> Result<Attestation<E>, Error> {
    let attester_index = single_attestation.attester_index;
    let committee_index = single_attestation.committee_index;
    let slot = single_attestation.data.slot;

    let aggregation_bit = committee
        .iter()
        .enumerate()
        .find_map(|(i, &validator_index)| {
            if attester_index as usize == validator_index {
                return Some(i);
            }
            None
        })
        .ok_or(Error::AttesterNotInCommittee {
            attester_index,
            committee_index,
            slot,
        })?;

    if fork_name.electra_enabled() {
        let mut committee_bits: BitVector<E::MaxCommitteesPerSlot> = BitVector::default();
        committee_bits
            .set(committee_index as usize, true)
            .map_err(|e| Error::Invalid(e.into()))?;

        let mut aggregation_bits =
            BitList::with_capacity(committee.len()).map_err(|e| Error::Invalid(e.into()))?;
        aggregation_bits
            .set(aggregation_bit, true)
            .map_err(|e| Error::Invalid(e.into()))?;
        Ok(Attestation::Electra(AttestationElectra {
            aggregation_bits,
            committee_bits,
            data: single_attestation.data,
            signature: single_attestation.signature.clone(),
        }))
    } else {
        let mut aggregation_bits =
            BitList::with_capacity(committee.len()).map_err(|e| Error::Invalid(e.into()))?;
        aggregation_bits
            .set(aggregation_bit, true)
            .map_err(|e| Error::Invalid(e.into()))?;
        Ok(Attestation::Base(AttestationBase {
            aggregation_bits,
            data: single_attestation.data,
            signature: single_attestation.signature.clone(),
        }))
    }
}
