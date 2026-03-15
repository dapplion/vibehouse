use types::*;

pub mod attesting_indices_base {
    use crate::per_block_processing::errors::{AttestationInvalid as Invalid, BlockOperationError};
    use types::*;

    /// Convert `attestation` to (almost) indexed-verifiable form.
    ///
    /// Spec v0.12.1
    pub fn get_indexed_attestation<E: EthSpec>(
        committee: &[usize],
        attestation: &AttestationBase<E>,
    ) -> Result<IndexedAttestation<E>, BlockOperationError<Invalid>> {
        let attesting_indices =
            get_attesting_indices::<E>(committee, &attestation.aggregation_bits)?;
        Ok(IndexedAttestation::Base(IndexedAttestationBase {
            attesting_indices: VariableList::new(attesting_indices)?,
            data: attestation.data,
            signature: attestation.signature.clone(),
        }))
    }

    /// Returns validator indices which participated in the attestation, sorted by increasing index.
    pub fn get_attesting_indices<E: EthSpec>(
        committee: &[usize],
        bitlist: &BitList<E::MaxValidatorsPerCommittee>,
    ) -> Result<Vec<u64>, BeaconStateError> {
        if bitlist.len() != committee.len() {
            return Err(BeaconStateError::InvalidBitfield);
        }

        let mut indices = Vec::with_capacity(bitlist.num_set_bits());

        for (i, validator_index) in committee.iter().enumerate() {
            if let Ok(true) = bitlist.get(i) {
                indices.push(*validator_index as u64)
            }
        }

        indices.sort_unstable();

        Ok(indices)
    }
}

pub mod attesting_indices_electra {
    use crate::per_block_processing::errors::{AttestationInvalid as Invalid, BlockOperationError};
    use safe_arith::SafeArith;
    use types::*;

    /// Compute an Electra IndexedAttestation given a list of committees.
    ///
    /// Committees must be sorted by ascending order 0..committees_per_slot
    pub fn get_indexed_attestation<E: EthSpec>(
        committees: &[BeaconCommittee<'_>],
        attestation: &AttestationElectra<E>,
    ) -> Result<IndexedAttestation<E>, BlockOperationError<Invalid>> {
        let attesting_indices = get_attesting_indices::<E>(
            committees,
            &attestation.aggregation_bits,
            &attestation.committee_bits,
        )?;

        Ok(IndexedAttestation::Electra(IndexedAttestationElectra {
            attesting_indices: VariableList::new(attesting_indices)?,
            data: attestation.data,
            signature: attestation.signature.clone(),
        }))
    }

    pub fn get_indexed_attestation_from_state<E: EthSpec>(
        beacon_state: &BeaconState<E>,
        attestation: &AttestationElectra<E>,
    ) -> Result<IndexedAttestation<E>, BlockOperationError<Invalid>> {
        let committees = beacon_state.get_beacon_committees_at_slot(attestation.data.slot)?;
        get_indexed_attestation(&committees, attestation)
    }

    /// Shortcut for getting the attesting indices while fetching the committee from the state's cache.
    pub fn get_attesting_indices_from_state<E: EthSpec>(
        state: &BeaconState<E>,
        att: &AttestationElectra<E>,
    ) -> Result<Vec<u64>, BeaconStateError> {
        let committees = state.get_beacon_committees_at_slot(att.data.slot)?;
        get_attesting_indices::<E>(&committees, &att.aggregation_bits, &att.committee_bits)
    }

    /// Returns validator indices which participated in the attestation, sorted by increasing index.
    ///
    /// Committees must be sorted by ascending order 0..committees_per_slot.
    /// Each validator appears in at most one committee per slot, so we collect
    /// directly into a Vec (no HashSet needed) and sort at the end.
    pub fn get_attesting_indices<E: EthSpec>(
        committees: &[BeaconCommittee<'_>],
        aggregation_bits: &BitList<E::MaxValidatorsPerSlot>,
        committee_bits: &BitVector<E::MaxCommitteesPerSlot>,
    ) -> Result<Vec<u64>, BeaconStateError> {
        let mut attesting_indices = Vec::with_capacity(aggregation_bits.num_set_bits());

        let mut committee_offset = 0;

        let committee_count_per_slot = committees.len() as u64;
        let mut participant_count = 0;
        for (committee_index, _) in committee_bits.iter().enumerate().filter(|(_, bit)| *bit) {
            let committee_index = committee_index as u64;
            let beacon_committee = committees
                .get(committee_index as usize)
                .ok_or(Error::NoCommitteeFound(committee_index))?;

            // This check is new to the spec's `process_attestation` in Electra.
            if committee_index >= committee_count_per_slot {
                return Err(BeaconStateError::InvalidCommitteeIndex(committee_index));
            }
            participant_count.safe_add_assign(beacon_committee.committee.len() as u64)?;

            let count_before = attesting_indices.len();
            for (i, &index) in beacon_committee.committee.iter().enumerate() {
                if let Ok(aggregation_bit_index) = committee_offset.safe_add(i)
                    && aggregation_bits.get(aggregation_bit_index).unwrap_or(false)
                {
                    attesting_indices.push(index as u64);
                }
            }

            // Require at least a single non-zero bit for each attesting committee bitfield.
            // This check is new to the spec's `process_attestation` in Electra.
            if attesting_indices.len() == count_before {
                return Err(BeaconStateError::EmptyCommittee);
            }

            committee_offset.safe_add_assign(beacon_committee.committee.len())?;
        }

        // This check is new to the spec's `process_attestation` in Electra.
        if participant_count as usize != aggregation_bits.len() {
            return Err(BeaconStateError::InvalidBitfield);
        }

        attesting_indices.sort_unstable();

        Ok(attesting_indices)
    }

    pub fn get_committee_indices<E: EthSpec>(
        committee_bits: &BitVector<E::MaxCommitteesPerSlot>,
    ) -> Vec<CommitteeIndex> {
        committee_bits
            .iter()
            .enumerate()
            .filter_map(|(index, bit)| if bit { Some(index as u64) } else { None })
            .collect()
    }
}

/// Shortcut for getting the attesting indices while fetching the committee from the state's cache.
pub fn get_attesting_indices_from_state<E: EthSpec>(
    state: &BeaconState<E>,
    att: AttestationRef<E>,
) -> Result<Vec<u64>, BeaconStateError> {
    match att {
        AttestationRef::Base(att) => {
            let committee = state.get_beacon_committee(att.data.slot, att.data.index)?;
            attesting_indices_base::get_attesting_indices::<E>(
                committee.committee,
                &att.aggregation_bits,
            )
        }
        AttestationRef::Electra(att) => {
            attesting_indices_electra::get_attesting_indices_from_state::<E>(state, att)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::attesting_indices_base;
    use super::attesting_indices_electra;
    use safe_arith::SafeArith;
    use types::*;

    type E = MinimalEthSpec;

    type ElectraSetup = (
        Vec<Vec<usize>>,
        BitList<<E as EthSpec>::MaxValidatorsPerSlot>,
        BitVector<<E as EthSpec>::MaxCommitteesPerSlot>,
    );

    // ── Base get_attesting_indices ──

    #[test]
    fn base_all_bits_set() {
        let committee = vec![10, 20, 30];
        let mut bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(3).unwrap();
        bits.set(0, true).unwrap();
        bits.set(1, true).unwrap();
        bits.set(2, true).unwrap();

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        assert_eq!(result, vec![10, 20, 30]);
    }

    #[test]
    fn base_no_bits_set() {
        let committee = vec![5, 15, 25];
        let bits = BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(3).unwrap();

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn base_partial_bits() {
        let committee = vec![3, 1, 4, 1, 5];
        let mut bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(5).unwrap();
        bits.set(1, true).unwrap(); // validator 1
        bits.set(3, true).unwrap(); // validator 1 (duplicate index)

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        // Both map to index 1, sorted
        assert_eq!(result, vec![1, 1]);
    }

    #[test]
    fn base_sorted_output() {
        // Committee has indices in descending order
        let committee = vec![99, 50, 10, 5, 1];
        let mut bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(5).unwrap();
        bits.set(0, true).unwrap(); // 99
        bits.set(2, true).unwrap(); // 10
        bits.set(4, true).unwrap(); // 1

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        // Must be sorted ascending
        assert_eq!(result, vec![1, 10, 99]);
    }

    #[test]
    fn base_length_mismatch_error() {
        let committee = vec![1, 2, 3];
        let bits = BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(5).unwrap();

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits);
        assert!(matches!(result, Err(BeaconStateError::InvalidBitfield)));
    }

    #[test]
    fn base_single_validator() {
        let committee = vec![42];
        let mut bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(1).unwrap();
        bits.set(0, true).unwrap();

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        assert_eq!(result, vec![42]);
    }

    #[test]
    fn base_empty_committee() {
        let committee: Vec<usize> = vec![];
        let bits = BitList::<<E as EthSpec>::MaxValidatorsPerCommittee>::with_capacity(0).unwrap();

        let result = attesting_indices_base::get_attesting_indices::<E>(&committee, &bits).unwrap();
        assert!(result.is_empty());
    }

    // ── Electra get_attesting_indices ──

    // Helper: build committees and aggregation bits for electra tests.
    fn electra_setup(
        committee_members: &[&[usize]],
        active_committees: &[usize],
        agg_bits_per_committee: &[&[bool]],
    ) -> ElectraSetup {
        let storage: Vec<Vec<usize>> = committee_members.iter().map(|c| c.to_vec()).collect();

        let mut committee_bits = BitVector::new();
        for &idx in active_committees {
            committee_bits.set(idx, true).unwrap();
        }

        // Compute total aggregation bits length
        let mut total_len: usize = 0;
        for &idx in active_committees {
            total_len = total_len.saturating_add(committee_members[idx].len());
        }

        let mut agg_bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(total_len).unwrap();
        let mut offset = 0;
        for (i, &committee_idx) in active_committees.iter().enumerate() {
            let bits = agg_bits_per_committee[i];
            for (j, &bit) in bits.iter().enumerate() {
                if bit {
                    agg_bits.set(offset.safe_add(j).unwrap(), true).unwrap();
                }
            }
            offset = offset
                .safe_add(committee_members[committee_idx].len())
                .unwrap();
        }

        (storage, agg_bits, committee_bits)
    }

    fn make_committees_from_storage(storage: &[Vec<usize>]) -> Vec<BeaconCommittee<'_>> {
        storage
            .iter()
            .enumerate()
            .map(|(i, committee_members)| BeaconCommittee {
                slot: Slot::new(0),
                index: i as u64,
                committee: committee_members.as_slice(),
            })
            .collect()
    }

    #[test]
    fn electra_single_committee_all_attesting() {
        let (storage, agg_bits, committee_bits) =
            electra_setup(&[&[10, 20, 30]], &[0], &[&[true, true, true]]);
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert_eq!(result, vec![10, 20, 30]);
    }

    #[test]
    fn electra_single_committee_partial() {
        let (storage, agg_bits, committee_bits) =
            electra_setup(&[&[5, 15, 25]], &[0], &[&[true, false, true]]);
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert_eq!(result, vec![5, 25]);
    }

    #[test]
    fn electra_multiple_committees() {
        let (storage, agg_bits, committee_bits) = electra_setup(
            &[&[1, 2], &[3, 4]],
            &[0, 1],
            &[&[true, false], &[false, true]],
        );
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert_eq!(result, vec![1, 4]);
    }

    #[test]
    fn electra_non_contiguous_committees() {
        // 3 committees, only committee 0 and 2 active
        let (storage, agg_bits, committee_bits) = electra_setup(
            &[&[10, 20], &[30, 40], &[50, 60]],
            &[0, 2],
            &[&[true, true], &[true, false]],
        );
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert_eq!(result, vec![10, 20, 50]);
    }

    #[test]
    fn electra_sorted_output() {
        // Committee members in descending order
        let (storage, agg_bits, committee_bits) = electra_setup(
            &[&[99, 50], &[10, 5]],
            &[0, 1],
            &[&[true, true], &[true, true]],
        );
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert_eq!(result, vec![5, 10, 50, 99]);
    }

    #[test]
    fn electra_empty_committee_error() {
        // All bits false for one committee → EmptyCommittee
        let (storage, agg_bits, committee_bits) =
            electra_setup(&[&[1, 2, 3]], &[0], &[&[false, false, false]]);
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        );
        assert!(matches!(result, Err(BeaconStateError::EmptyCommittee)));
    }

    #[test]
    fn electra_bitfield_length_mismatch() {
        // aggregation_bits length doesn't match sum of committee sizes
        let mut committee_bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        committee_bits.set(0, true).unwrap();

        // Make agg_bits with wrong length (3 instead of 2)
        let mut agg_bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(3).unwrap();
        agg_bits.set(0, true).unwrap();

        let storage = vec![vec![1usize, 2]];
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        );
        assert!(matches!(result, Err(BeaconStateError::InvalidBitfield)));
    }

    #[test]
    fn electra_no_active_committees() {
        // No committee bits set, aggregation_bits length must be 0
        let committee_bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        let agg_bits = BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(0).unwrap();

        let storage = vec![vec![1usize, 2]];
        let committees = make_committees_from_storage(&storage);

        let result = attesting_indices_electra::get_attesting_indices::<E>(
            &committees,
            &agg_bits,
            &committee_bits,
        )
        .unwrap();
        assert!(result.is_empty());
    }

    // ── Electra get_committee_indices ──

    #[test]
    fn electra_get_committee_indices_none() {
        let bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        let result = attesting_indices_electra::get_committee_indices::<E>(&bits);
        assert!(result.is_empty());
    }

    #[test]
    fn electra_get_committee_indices_some() {
        let mut bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        bits.set(0, true).unwrap();
        bits.set(2, true).unwrap();
        bits.set(3, true).unwrap();
        let result = attesting_indices_electra::get_committee_indices::<E>(&bits);
        assert_eq!(result, vec![0, 2, 3]);
    }
}
