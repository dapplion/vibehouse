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

/// Convert a `SingleAttestation` to an `Attestation` by looking up the attester's position
/// in the committee.
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

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        AggregateSignature, AttestationData, Checkpoint, Epoch, Hash256, MinimalEthSpec, Slot,
    };

    type E = MinimalEthSpec;

    fn make_single_attestation(
        committee_index: u64,
        attester_index: u64,
        slot: u64,
    ) -> SingleAttestation {
        SingleAttestation {
            committee_index,
            attester_index,
            data: AttestationData {
                slot: Slot::new(slot),
                index: 0,
                beacon_block_root: Hash256::repeat_byte(0xaa),
                source: Checkpoint {
                    epoch: Epoch::new(0),
                    root: Hash256::ZERO,
                },
                target: Checkpoint {
                    epoch: Epoch::new(1),
                    root: Hash256::repeat_byte(0xbb),
                },
            },
            signature: AggregateSignature::empty(),
        }
    }

    // --- build_attestation_from_single ---

    #[test]
    fn build_from_single_base_fork() {
        let sa = make_single_attestation(0, 42, 1);
        let att = build_attestation_from_single::<E>(&sa, 2, 5, ForkName::Base).unwrap();
        match &att {
            Attestation::Base(base) => {
                assert_eq!(base.aggregation_bits.len(), 5);
                assert!(base.aggregation_bits.get(2).unwrap());
                assert!(!base.aggregation_bits.get(0).unwrap());
                assert!(!base.aggregation_bits.get(1).unwrap());
                assert_eq!(base.data, sa.data);
            }
            _ => panic!("Expected Base attestation"),
        }
    }

    #[test]
    fn build_from_single_electra_fork() {
        let sa = make_single_attestation(2, 42, 1);
        let att = build_attestation_from_single::<E>(&sa, 0, 3, ForkName::Electra).unwrap();
        match &att {
            Attestation::Electra(electra) => {
                assert_eq!(electra.aggregation_bits.len(), 3);
                assert!(electra.aggregation_bits.get(0).unwrap());
                assert!(!electra.aggregation_bits.get(1).unwrap());
                assert!(electra.committee_bits.get(2).unwrap());
                assert!(!electra.committee_bits.get(0).unwrap());
                assert_eq!(electra.data, sa.data);
            }
            _ => panic!("Expected Electra attestation"),
        }
    }

    #[test]
    fn build_from_single_gloas_fork() {
        let sa = make_single_attestation(1, 42, 1);
        let att = build_attestation_from_single::<E>(&sa, 1, 4, ForkName::Gloas).unwrap();
        match &att {
            Attestation::Electra(electra) => {
                assert_eq!(electra.aggregation_bits.len(), 4);
                assert!(electra.aggregation_bits.get(1).unwrap());
                assert!(electra.committee_bits.get(1).unwrap());
            }
            _ => panic!("Expected Electra attestation for Gloas fork"),
        }
    }

    #[test]
    fn build_from_single_aggregation_bit_out_of_bounds() {
        let sa = make_single_attestation(0, 42, 1);
        let result = build_attestation_from_single::<E>(&sa, 5, 3, ForkName::Base);
        assert!(result.is_err());
    }

    #[test]
    fn build_from_single_committee_index_out_of_bounds() {
        // MinimalEthSpec has MaxCommitteesPerSlot = U4, so committee_index 10 should fail
        let sa = make_single_attestation(10, 42, 1);
        let result = build_attestation_from_single::<E>(&sa, 0, 3, ForkName::Electra);
        assert!(result.is_err());
    }

    // --- single_attestation_to_attestation ---

    #[test]
    fn to_attestation_base_fork_attester_found() {
        let sa = make_single_attestation(0, 42, 1);
        let committee = vec![10, 20, 42, 50];
        let att = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base).unwrap();
        match &att {
            Attestation::Base(base) => {
                assert_eq!(base.aggregation_bits.len(), 4);
                assert!(base.aggregation_bits.get(2).unwrap());
                assert!(!base.aggregation_bits.get(0).unwrap());
            }
            _ => panic!("Expected Base attestation"),
        }
    }

    #[test]
    fn to_attestation_electra_fork_attester_found() {
        let sa = make_single_attestation(1, 42, 1);
        let committee = vec![42, 10, 20];
        let att =
            single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Electra).unwrap();
        match &att {
            Attestation::Electra(electra) => {
                assert_eq!(electra.aggregation_bits.len(), 3);
                assert!(electra.aggregation_bits.get(0).unwrap());
                assert!(electra.committee_bits.get(1).unwrap());
            }
            _ => panic!("Expected Electra attestation"),
        }
    }

    #[test]
    fn to_attestation_attester_not_in_committee() {
        let sa = make_single_attestation(0, 42, 5);
        let committee = vec![10, 20, 30];
        let result = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base);
        match result {
            Err(Error::AttesterNotInCommittee {
                attester_index,
                committee_index,
                slot,
            }) => {
                assert_eq!(attester_index, 42);
                assert_eq!(committee_index, 0);
                assert_eq!(slot, Slot::new(5));
            }
            _ => panic!("Expected AttesterNotInCommittee error"),
        }
    }

    #[test]
    fn to_attestation_attester_at_first_position() {
        let sa = make_single_attestation(0, 99, 1);
        let committee = vec![99, 100, 101];
        let att = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base).unwrap();
        match &att {
            Attestation::Base(base) => {
                assert!(base.aggregation_bits.get(0).unwrap());
                assert!(!base.aggregation_bits.get(1).unwrap());
                assert!(!base.aggregation_bits.get(2).unwrap());
            }
            _ => panic!("Expected Base attestation"),
        }
    }

    #[test]
    fn to_attestation_attester_at_last_position() {
        let sa = make_single_attestation(0, 101, 1);
        let committee = vec![99, 100, 101];
        let att = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base).unwrap();
        match &att {
            Attestation::Base(base) => {
                assert!(!base.aggregation_bits.get(0).unwrap());
                assert!(!base.aggregation_bits.get(1).unwrap());
                assert!(base.aggregation_bits.get(2).unwrap());
            }
            _ => panic!("Expected Base attestation"),
        }
    }

    #[test]
    fn to_attestation_single_member_committee() {
        let sa = make_single_attestation(0, 42, 1);
        let committee = vec![42];
        let att = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base).unwrap();
        match &att {
            Attestation::Base(base) => {
                assert_eq!(base.aggregation_bits.len(), 1);
                assert!(base.aggregation_bits.get(0).unwrap());
            }
            _ => panic!("Expected Base attestation"),
        }
    }

    #[test]
    fn to_attestation_empty_committee() {
        let sa = make_single_attestation(0, 42, 1);
        let committee: Vec<usize> = vec![];
        let result = single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Base);
        assert!(matches!(result, Err(Error::AttesterNotInCommittee { .. })));
    }

    #[test]
    fn to_attestation_data_preserved() {
        let sa = make_single_attestation(2, 42, 100);
        let committee = vec![42];
        let att =
            single_attestation_to_attestation::<E>(&sa, &committee, ForkName::Electra).unwrap();
        match &att {
            Attestation::Electra(electra) => {
                assert_eq!(electra.data.slot, Slot::new(100));
                assert_eq!(electra.data.beacon_block_root, Hash256::repeat_byte(0xaa));
                assert_eq!(electra.data.target.epoch, Epoch::new(1));
            }
            _ => panic!("Expected Electra attestation"),
        }
    }
}
