use crate::api_error::ApiError;
use crate::state_id::StateId;
use beacon_chain::{BeaconChain, BeaconChainTypes};
use eth2::{
    lighthouse::{GlobalValidatorInclusionData, ValidatorInclusionData},
    types::ValidatorId,
};
use state_processing::per_epoch_processing::{EpochProcessingSummary, process_epoch};
use types::{BeaconState, BeaconStateError, ChainSpec, Epoch, EthSpec};

/// Returns the state in the last slot of `epoch`.
fn end_of_epoch_state<T: BeaconChainTypes>(
    epoch: Epoch,
    chain: &BeaconChain<T>,
) -> Result<BeaconState<T::EthSpec>, ApiError> {
    let target_slot = epoch.end_slot(T::EthSpec::slots_per_epoch());
    // The execution status is not returned, any functions which rely upon this method might return
    // optimistic information without explicitly declaring so.
    let (state, _execution_status, _finalized) = StateId::from_slot(target_slot).state(chain)?;
    Ok(state)
}

/// Generate an `EpochProcessingSummary` for `state`.
///
/// ## Notes
///
/// Will mutate `state`, transitioning it to the next epoch.
fn get_epoch_processing_summary<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<EpochProcessingSummary<E>, ApiError> {
    process_epoch(state, spec).map_err(|e| ApiError::server_error(format!("{:?}", e)))
}

fn convert_cache_error(error: BeaconStateError) -> ApiError {
    ApiError::server_error(format!("{:?}", error))
}

/// Returns information about *all validators* (i.e., global) and how they performed during a given
/// epoch.
pub fn global_validator_inclusion_data<T: BeaconChainTypes>(
    epoch: Epoch,
    chain: &BeaconChain<T>,
) -> Result<GlobalValidatorInclusionData, ApiError> {
    let mut state = end_of_epoch_state(epoch, chain)?;
    let summary = get_epoch_processing_summary(&mut state, &chain.spec)?;

    Ok(GlobalValidatorInclusionData {
        current_epoch_active_gwei: summary.current_epoch_total_active_balance(),
        current_epoch_target_attesting_gwei: summary
            .current_epoch_target_attesting_balance()
            .map_err(convert_cache_error)?,
        previous_epoch_target_attesting_gwei: summary
            .previous_epoch_target_attesting_balance()
            .map_err(convert_cache_error)?,
        previous_epoch_head_attesting_gwei: summary
            .previous_epoch_head_attesting_balance()
            .map_err(convert_cache_error)?,
    })
}

/// Returns information about a single validator and how it performed during a given epoch.
pub fn validator_inclusion_data<T: BeaconChainTypes>(
    epoch: Epoch,
    validator_id: &ValidatorId,
    chain: &BeaconChain<T>,
) -> Result<Option<ValidatorInclusionData>, ApiError> {
    let mut state = end_of_epoch_state(epoch, chain)?;

    state
        .update_pubkey_cache()
        .map_err(ApiError::beacon_state_error)?;

    let validator_index = match validator_id {
        ValidatorId::Index(index) => *index as usize,
        ValidatorId::PublicKey(pubkey) => {
            if let Some(index) = state
                .get_validator_index(pubkey)
                .map_err(ApiError::beacon_state_error)?
            {
                index
            } else {
                return Ok(None);
            }
        }
    };

    // Obtain the validator *before* transitioning the state into the next epoch.
    let validator = if let Ok(validator) = state.get_validator(validator_index) {
        validator.clone()
    } else {
        return Ok(None);
    };

    let summary = get_epoch_processing_summary(&mut state, &chain.spec)?;

    Ok(Some(ValidatorInclusionData {
        is_slashed: validator.slashed,
        is_withdrawable_in_current_epoch: validator.is_withdrawable_at(epoch),
        is_active_unslashed_in_current_epoch: summary
            .is_active_unslashed_in_current_epoch(validator_index),
        is_active_unslashed_in_previous_epoch: summary
            .is_active_unslashed_in_previous_epoch(validator_index),
        current_epoch_effective_balance_gwei: validator.effective_balance,
        is_current_epoch_target_attester: summary
            .is_current_epoch_target_attester(validator_index)
            .map_err(convert_cache_error)?,
        is_previous_epoch_target_attester: summary
            .is_previous_epoch_target_attester(validator_index)
            .map_err(convert_cache_error)?,
        is_previous_epoch_head_attester: summary
            .is_previous_epoch_head_attester(validator_index)
            .map_err(convert_cache_error)?,
    }))
}
