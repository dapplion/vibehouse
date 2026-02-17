//! Contains the handler for the `POST validator/duties/ptc/{epoch}` endpoint.

use beacon_chain::{BeaconChain, BeaconChainTypes};
use eth2::types::{self as api_types};
use slot_clock::SlotClock;
use types::{Epoch, EthSpec, Slot};

/// The struct that is returned to the requesting HTTP client.
type ApiDuties = api_types::DutiesResponse<Vec<api_types::PtcDutyData>>;

/// Handles a request from the HTTP API for PTC duties.
pub fn ptc_duties<T: BeaconChainTypes>(
    request_epoch: Epoch,
    request_indices: &[u64],
    chain: &BeaconChain<T>,
) -> Result<ApiDuties, warp::reject::Rejection> {
    if !chain.spec.is_gloas_scheduled() {
        return Err(warp_utils::reject::custom_bad_request(
            "Gloas is not scheduled".to_string(),
        ));
    }

    let current_epoch = chain
        .slot_clock
        .now_or_genesis()
        .map(|slot: Slot| slot.epoch(T::EthSpec::slots_per_epoch()))
        .ok_or(beacon_chain::BeaconChainError::UnableToReadSlot)
        .map_err(warp_utils::reject::unhandled_error)?;

    // Only allow current or next epoch (PTC duties aren't useful for past epochs)
    if request_epoch > current_epoch + 1 {
        return Err(warp_utils::reject::custom_bad_request(format!(
            "request epoch {} is more than one epoch past the current epoch {}",
            request_epoch, current_epoch
        )));
    }

    if request_epoch + 1 < current_epoch {
        return Err(warp_utils::reject::custom_bad_request(format!(
            "request epoch {} is too far in the past (current epoch {})",
            request_epoch, current_epoch
        )));
    }

    let (duties, dependent_root) = chain
        .validator_ptc_duties(request_indices, request_epoch)
        .map_err(warp_utils::reject::unhandled_error)?;

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
