use super::errors::{BlockOperationError, ExitInvalid};
use crate::per_block_processing::gloas::get_pending_balance_to_withdraw_for_builder;
use crate::per_block_processing::{
    VerifySignatures,
    signature_sets::{exit_signature_set, get_pubkey_from_state},
};
use safe_arith::SafeArith;
use std::borrow::Cow;
use types::consts::gloas::BUILDER_INDEX_FLAG;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<ExitInvalid>>;

fn error(reason: ExitInvalid) -> BlockOperationError<ExitInvalid> {
    BlockOperationError::invalid(reason)
}

/// Returns true if the validator_index has the BUILDER_INDEX_FLAG set,
/// indicating it refers to a builder rather than a validator.
fn is_builder_index(validator_index: u64) -> bool {
    (validator_index & BUILDER_INDEX_FLAG) != 0
}

/// Extract the builder index from a flagged validator_index.
fn to_builder_index(validator_index: u64) -> u64 {
    validator_index & !BUILDER_INDEX_FLAG
}

/// Indicates if a voluntary exit is valid to be included in a block.
///
/// [Modified in Gloas:EIP7732] Now supports builder exits when
/// `exit.validator_index` has `BUILDER_INDEX_FLAG` set.
///
/// Returns `Ok(true)` for builder exits, `Ok(false)` for validator exits.
pub fn verify_exit<E: EthSpec>(
    state: &BeaconState<E>,
    current_epoch: Option<Epoch>,
    signed_exit: &SignedVoluntaryExit,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<bool> {
    let current_epoch = current_epoch.unwrap_or(state.current_epoch());
    let exit = &signed_exit.message;

    // Exits must specify an epoch when they become valid; they are not valid before then.
    verify!(
        current_epoch >= exit.epoch,
        ExitInvalid::FutureEpoch {
            state: current_epoch,
            exit: exit.epoch
        }
    );

    // [New in Gloas:EIP7732] Handle builder exits
    if state.fork_name_unchecked().gloas_enabled() && is_builder_index(exit.validator_index) {
        let builder_index = to_builder_index(exit.validator_index);
        return verify_builder_exit(
            state,
            current_epoch,
            builder_index,
            signed_exit,
            verify_signatures,
            spec,
        )
        .map(|()| true);
    }

    // Validator exit path (unchanged from pre-Gloas)
    let validator = state
        .validators()
        .get(exit.validator_index as usize)
        .ok_or_else(|| error(ExitInvalid::ValidatorUnknown(exit.validator_index)))?;

    // Verify the validator is active.
    verify!(
        validator.is_active_at(current_epoch),
        ExitInvalid::NotActive(exit.validator_index)
    );

    // Verify that the validator has not yet exited.
    verify!(
        validator.exit_epoch == spec.far_future_epoch,
        ExitInvalid::AlreadyExited(exit.validator_index)
    );

    // Verify the validator has been active long enough.
    let earliest_exit_epoch = validator
        .activation_epoch
        .safe_add(spec.shard_committee_period)?;
    verify!(
        current_epoch >= earliest_exit_epoch,
        ExitInvalid::TooYoungToExit {
            current_epoch,
            earliest_exit_epoch,
        }
    );

    if verify_signatures.is_true() {
        verify!(
            exit_signature_set(
                state,
                |i| get_pubkey_from_state(state, i),
                signed_exit,
                spec
            )?
            .verify(),
            ExitInvalid::BadSignature
        );
    }

    // [New in Electra:EIP7251]
    // Only exit validator if it has no pending withdrawals in the queue
    if let Ok(pending_balance_to_withdraw) =
        state.get_pending_balance_to_withdraw(exit.validator_index as usize)
    {
        verify!(
            pending_balance_to_withdraw == 0,
            ExitInvalid::PendingWithdrawalInQueue(exit.validator_index)
        );
    }

    Ok(false)
}

/// Verify a builder voluntary exit.
///
/// Spec: process_voluntary_exit (builder branch) in Gloas
fn verify_builder_exit<E: EthSpec>(
    state: &BeaconState<E>,
    _current_epoch: Epoch,
    builder_index: u64,
    signed_exit: &SignedVoluntaryExit,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<()> {
    let finalized_epoch = state.finalized_checkpoint().epoch;

    let builders = state
        .builders()
        .map_err(|_| error(ExitInvalid::BuilderUnknown(builder_index)))?;

    let builder = builders
        .get(builder_index as usize)
        .ok_or_else(|| error(ExitInvalid::BuilderUnknown(builder_index)))?;

    // Verify the builder is active
    verify!(
        builder.is_active_at_finalized_epoch(finalized_epoch, spec),
        ExitInvalid::BuilderNotActive(builder_index)
    );

    // Only exit builder if it has no pending withdrawals in the queue
    let pending = get_pending_balance_to_withdraw_for_builder(state, builder_index)
        .map_err(BlockOperationError::BeaconStateError)?;
    verify!(
        pending == 0,
        ExitInvalid::BuilderPendingWithdrawalInQueue(builder_index)
    );

    // Verify signature using builder's pubkey
    if verify_signatures.is_true() {
        let get_builder_pubkey = |_i: usize| -> Option<Cow<PublicKey>> {
            builder.pubkey.decompress().ok().map(Cow::Owned)
        };

        verify!(
            exit_signature_set(state, get_builder_pubkey, signed_exit, spec)?.verify(),
            ExitInvalid::BadSignature
        );
    }

    Ok(())
}
