use integer_sqrt::IntegerSquareRoot;
use safe_arith::SafeArith;
use smallvec::SmallVec;
use types::{AttestationData, BeaconState, ChainSpec, EthSpec, Slot};
use types::{
    BeaconStateError as Error,
    consts::altair::{
        NUM_FLAG_INDICES, TIMELY_HEAD_FLAG_INDEX, TIMELY_SOURCE_FLAG_INDEX,
        TIMELY_TARGET_FLAG_INDEX,
    },
};

/// [New in Gloas:EIP7732]
/// Check if attestation targets the block proposed at the attestation slot.
pub fn is_attestation_same_slot<E: EthSpec>(
    state: &BeaconState<E>,
    data: &AttestationData,
) -> Result<bool, Error> {
    if data.slot == Slot::new(0) {
        return Ok(true);
    }
    let blockroot = data.beacon_block_root;
    let slot_blockroot = *state.get_block_root(data.slot)?;
    let prev_blockroot = *state.get_block_root(data.slot.safe_sub(1u64)?)?;
    Ok(blockroot == slot_blockroot && blockroot != prev_blockroot)
}

/// Get the participation flags for a valid attestation.
///
/// You should have called `verify_attestation_for_block_inclusion` or similar before
/// calling this function, in order to ensure that the attestation's source is correct.
///
/// This function will return an error if the source of the attestation doesn't match the
/// state's relevant justified checkpoint.
pub fn get_attestation_participation_flag_indices<E: EthSpec>(
    state: &BeaconState<E>,
    data: &AttestationData,
    inclusion_delay: u64,
    spec: &ChainSpec,
) -> Result<SmallVec<[usize; NUM_FLAG_INDICES]>, Error> {
    let justified_checkpoint = if data.target.epoch == state.current_epoch() {
        state.current_justified_checkpoint()
    } else {
        state.previous_justified_checkpoint()
    };

    // Matching roots.
    let is_matching_source = data.source == justified_checkpoint;
    let is_matching_target = is_matching_source
        && data.target.root == *state.get_block_root_at_epoch(data.target.epoch)?;

    let head_root_matches = data.beacon_block_root == *state.get_block_root(data.slot)?;

    // [Modified in Gloas:EIP7732] head flag also requires payload_matches
    let is_matching_head = if state.fork_name_unchecked().gloas_enabled() {
        let is_same_slot = is_attestation_same_slot(state, data)?;
        // [New in Gloas:EIP7732] Same-slot attestations must have data.index == 0
        if is_same_slot && data.index != 0 {
            return Err(Error::IncorrectAttestationIndex);
        }
        let payload_matches = if is_same_slot {
            // Same-slot attestations always match payload
            true
        } else {
            // Historical: check execution_payload_availability
            let slot_index = data
                .slot
                .as_usize()
                .safe_rem(E::slots_per_historical_root())?;
            let availability = state
                .as_gloas()
                .map(|s| {
                    s.execution_payload_availability
                        .get(slot_index)
                        .map(|b| b as u64)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            data.index == availability
        };
        is_matching_target && head_root_matches && payload_matches
    } else {
        is_matching_target && head_root_matches
    };

    if !is_matching_source {
        return Err(Error::IncorrectAttestationSource);
    }

    // Participation flag indices
    let mut participation_flag_indices = SmallVec::new();
    if is_matching_source && inclusion_delay <= E::slots_per_epoch().integer_sqrt() {
        participation_flag_indices.push(TIMELY_SOURCE_FLAG_INDEX);
    }
    if state.fork_name_unchecked().deneb_enabled() {
        if is_matching_target {
            // [Modified in Deneb:EIP7045]
            participation_flag_indices.push(TIMELY_TARGET_FLAG_INDEX);
        }
    } else if is_matching_target && inclusion_delay <= E::slots_per_epoch() {
        participation_flag_indices.push(TIMELY_TARGET_FLAG_INDEX);
    }

    if is_matching_head && inclusion_delay == spec.min_attestation_inclusion_delay {
        participation_flag_indices.push(TIMELY_HEAD_FLAG_INDEX);
    }
    Ok(participation_flag_indices)
}
