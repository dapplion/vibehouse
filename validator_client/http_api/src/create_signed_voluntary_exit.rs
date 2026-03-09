use crate::api_error::ApiError;
use bls::{PublicKey, PublicKeyBytes};
use eth2::types::GenericResponse;
use slot_clock::SlotClock;
use std::sync::Arc;
use tracing::info;
use types::{Epoch, EthSpec, SignedVoluntaryExit, VoluntaryExit};
use validator_store::ValidatorStore;
use vibehouse_validator_store::VibehouseValidatorStore;

pub async fn create_signed_voluntary_exit<T: 'static + SlotClock + Clone, E: EthSpec>(
    pubkey: PublicKey,
    maybe_epoch: Option<Epoch>,
    validator_store: Arc<VibehouseValidatorStore<T, E>>,
    slot_clock: T,
) -> Result<GenericResponse<SignedVoluntaryExit>, ApiError> {
    let epoch = match maybe_epoch {
        Some(epoch) => epoch,
        None => get_current_epoch::<T, E>(slot_clock).ok_or_else(|| {
            ApiError::ServerError("Unable to determine current epoch".to_string())
        })?,
    };

    let pubkey_bytes = PublicKeyBytes::from(pubkey);
    if !validator_store.has_validator(&pubkey_bytes) {
        return Err(ApiError::NotFound(format!(
            "{} is disabled or not managed by this validator client",
            pubkey_bytes.as_hex_string()
        )));
    }

    let validator_index = validator_store
        .validator_index(&pubkey_bytes)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "The validator index for {} is not known. The validator client \
                may still be initializing or the validator has not yet had a \
                deposit processed.",
                pubkey_bytes.as_hex_string()
            ))
        })?;

    let voluntary_exit = VoluntaryExit {
        epoch,
        validator_index,
    };

    info!(
        validator = pubkey_bytes.as_hex_string(),
        %epoch,
        "Signing voluntary exit"
    );

    let signed_voluntary_exit = validator_store
        .sign_voluntary_exit(pubkey_bytes, voluntary_exit)
        .await
        .map_err(|e| ApiError::ServerError(format!("Failed to sign voluntary exit: {:?}", e)))?;

    Ok(GenericResponse::from(signed_voluntary_exit))
}

/// Calculates the current epoch from the genesis time and current time.
fn get_current_epoch<T: 'static + SlotClock + Clone, E: EthSpec>(slot_clock: T) -> Option<Epoch> {
    slot_clock.now().map(|s| s.epoch(E::slots_per_epoch()))
}
