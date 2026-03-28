//! Contains the handler for the `POST validator/duties/inclusion_list/{epoch}` endpoint.

use crate::api_error::ApiError;
use beacon_chain::{BeaconChain, BeaconChainTypes};
use eth2::types as api_types;
use slot_clock::SlotClock;
use types::{Epoch, EthSpec, Slot};

/// The struct that is returned to the requesting HTTP client.
type ApiDuties = api_types::DutiesResponse<Vec<api_types::InclusionListDutyData>>;

/// Handles a request from the HTTP API for inclusion list committee duties.
pub(crate) fn inclusion_list_duties<T: BeaconChainTypes>(
    request_epoch: Epoch,
    request_indices: &[u64],
    chain: &BeaconChain<T>,
) -> Result<ApiDuties, ApiError> {
    if !chain.spec.is_heze_scheduled() {
        return Err(ApiError::bad_request("Heze is not scheduled".to_string()));
    }

    let current_epoch = chain
        .slot_clock
        .now_or_genesis()
        .map(|slot: Slot| slot.epoch(T::EthSpec::slots_per_epoch()))
        .ok_or(beacon_chain::BeaconChainError::UnableToReadSlot)
        .map_err(ApiError::unhandled_error)?;

    // Only allow current or next epoch
    if request_epoch > current_epoch + 1 {
        return Err(ApiError::bad_request(format!(
            "request epoch {request_epoch} is more than one epoch past the current epoch {current_epoch}"
        )));
    }

    if request_epoch + 1 < current_epoch {
        return Err(ApiError::bad_request(format!(
            "request epoch {request_epoch} is too far in the past (current epoch {current_epoch})"
        )));
    }

    let (duties, dependent_root) = chain
        .validator_inclusion_list_duties(request_indices, request_epoch)
        .map_err(ApiError::unhandled_error)?;

    let execution_optimistic = chain
        .canonical_head
        .head_execution_status()
        .map(|s| s.is_optimistic_or_invalid())
        .unwrap_or(false);

    Ok(api_types::DutiesResponse {
        dependent_root,
        execution_optimistic: Some(execution_optimistic),
        data: duties,
    })
}
