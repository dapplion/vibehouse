use crate::per_block_processing::errors::BlockProcessingError;
use safe_arith::SafeArith;
use tree_hash::TreeHash;
use types::{BeaconState, ChainSpec, Domain, EthSpec, SignedInclusionList, SigningData, Slot};

/// Get the inclusion list committee for a given slot.
///
/// Returns a vector of INCLUSION_LIST_COMMITTEE_SIZE validator indices selected
/// by cycling through the concatenation of all beacon committees for the slot.
///
/// Spec: get_inclusion_list_committee(state, slot)
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/heze/beacon-chain.md>
pub fn get_inclusion_list_committee<E: EthSpec>(
    state: &BeaconState<E>,
    slot: Slot,
    spec: &ChainSpec,
) -> Result<Vec<u64>, BlockProcessingError> {
    let committees = state
        .get_beacon_committees_at_slot(slot)
        .map_err(BlockProcessingError::BeaconStateError)?;

    // Concatenate all committees for this slot in order
    let mut indices: Vec<u64> = Vec::new();
    for committee in &committees {
        for &idx in committee.committee {
            indices.push(idx as u64);
        }
    }

    if indices.is_empty() {
        return Err(BlockProcessingError::InclusionListInvalid {
            reason: "no validators in committees for slot".into(),
        });
    }

    // Cycle through to fill INCLUSION_LIST_COMMITTEE_SIZE slots
    let il_committee_size = spec.inclusion_list_committee_size as usize;
    let mut result = Vec::with_capacity(il_committee_size);
    for i in 0..il_committee_size {
        let idx = i.safe_rem(indices.len())?;
        let validator_index =
            *indices
                .get(idx)
                .ok_or_else(|| BlockProcessingError::InclusionListInvalid {
                    reason: "index out of bounds in committee computation".into(),
                })?;
        result.push(validator_index);
    }

    Ok(result)
}

/// Validate the signature on a signed inclusion list.
///
/// Checks that the signature is valid for the inclusion list's validator using
/// the `DOMAIN_INCLUSION_LIST_COMMITTEE` domain.
///
/// Spec: is_valid_inclusion_list_signature(state, signed_inclusion_list)
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/heze/beacon-chain.md>
pub fn is_valid_inclusion_list_signature<E: EthSpec>(
    state: &BeaconState<E>,
    signed_il: &SignedInclusionList<E>,
    spec: &ChainSpec,
) -> Result<bool, BlockProcessingError> {
    let il = &signed_il.message;
    let validator_index = il.validator_index as usize;

    let validator = state.validators().get(validator_index).ok_or_else(|| {
        BlockProcessingError::InclusionListInvalid {
            reason: format!("validator index {validator_index} out of bounds"),
        }
    })?;

    let pubkey =
        validator
            .pubkey
            .decompress()
            .map_err(|_| BlockProcessingError::InclusionListInvalid {
                reason: format!("failed to decompress validator {validator_index} pubkey"),
            })?;

    let epoch = il.slot.epoch(E::slots_per_epoch());
    let fork = state.fork();
    let genesis_validators_root = state.genesis_validators_root();

    let domain = spec.get_domain(
        epoch,
        Domain::InclusionListCommittee,
        &fork,
        genesis_validators_root,
    );

    let signing_root = SigningData {
        object_root: il.tree_hash_root(),
        domain,
    }
    .tree_hash_root();

    Ok(signed_il.signature.verify(&pubkey, signing_root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::per_block_processing::gloas::tests::make_gloas_state_with_committees;
    use types::MinimalEthSpec;

    type E = MinimalEthSpec;

    #[test]
    fn inclusion_list_committee_correct_size() {
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(64, max_eb, 64_000_000_000);
        let slot = state.slot();

        let committee = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        assert_eq!(committee.len(), spec.inclusion_list_committee_size as usize);
    }

    #[test]
    fn inclusion_list_committee_deterministic() {
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(64, max_eb, 64_000_000_000);
        let slot = state.slot();

        let c1 = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        let c2 = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        assert_eq!(
            c1, c2,
            "committee should be deterministic for same state+slot"
        );
    }

    #[test]
    fn inclusion_list_committee_valid_indices() {
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let num_validators = 128;
        let (state, spec) =
            make_gloas_state_with_committees(num_validators, max_eb, 64_000_000_000);
        let slot = state.slot();

        let committee = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        for &idx in &committee {
            assert!(
                (idx as usize) < num_validators,
                "committee member {idx} exceeds validator count {num_validators}"
            );
        }
    }

    #[test]
    fn inclusion_list_committee_wraps_with_few_validators() {
        // With 8 validators in minimal spec, a single slot may only have 1 committee member.
        // The algorithm should wrap around via modulo.
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(8, max_eb, 64_000_000_000);
        let slot = state.slot();

        let committee = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        assert_eq!(committee.len(), spec.inclusion_list_committee_size as usize);

        // All members should be valid
        for &idx in &committee {
            assert!((idx as usize) < 8, "index {idx} out of range");
        }
    }

    #[test]
    fn inclusion_list_committee_large_validator_set_distinct() {
        // With many validators, committee members should be mostly distinct
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(256, max_eb, 64_000_000_000);
        let slot = state.slot();

        let committee = get_inclusion_list_committee::<E>(&state, slot, &spec).unwrap();
        assert_eq!(committee.len(), spec.inclusion_list_committee_size as usize);

        // With 256 validators and committee_size=16, each slot should have enough
        // distinct validators that we don't need to wrap
        let unique: std::collections::HashSet<u64> = committee.iter().copied().collect();
        assert_eq!(
            unique.len(),
            committee.len(),
            "expected distinct members with 256 validators"
        );
    }
}
