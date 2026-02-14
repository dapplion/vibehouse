use crate::EpochProcessingError;
use types::{BeaconState, BuilderPendingPayment, ChainSpec, EthSpec};

/// Processes the builder pending payments from the previous epoch.
///
/// Checks accumulated weights against the quorum threshold. Payments meeting the
/// threshold are moved to the withdrawal queue. The payment window then rotates forward.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_builder_pending_payments
pub fn process_builder_pending_payments<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), EpochProcessingError> {
    let slots_per_epoch = E::slots_per_epoch() as usize;

    // Calculate quorum threshold: get_builder_payment_quorum_threshold
    // per_slot_balance = total_active_balance // SLOTS_PER_EPOCH
    // quorum = per_slot_balance * BUILDER_PAYMENT_THRESHOLD_NUMERATOR // BUILDER_PAYMENT_THRESHOLD_DENOMINATOR
    let total_active_balance = state.get_total_active_balance()?;
    let per_slot_balance = total_active_balance / E::slots_per_epoch();
    let quorum = per_slot_balance.saturating_mul(spec.builder_payment_threshold_numerator)
        / spec.builder_payment_threshold_denominator;

    let state_gloas = state.as_gloas_mut()?;

    // Check first SLOTS_PER_EPOCH entries against quorum, append qualifying withdrawals
    for i in 0..slots_per_epoch {
        if let Some(payment) = state_gloas.builder_pending_payments.get(i) {
            if payment.weight >= quorum {
                let withdrawal = payment.withdrawal.clone();
                state_gloas.builder_pending_withdrawals.push(withdrawal)?;
            }
        }
    }

    // Rotate: move second half to first half, clear second half
    // old_payments = state.builder_pending_payments[SLOTS_PER_EPOCH:]
    // new_payments = [BuilderPendingPayment() for _ in range(SLOTS_PER_EPOCH)]
    // state.builder_pending_payments = old_payments + new_payments
    let total_len = state_gloas.builder_pending_payments.len();
    for i in 0..slots_per_epoch {
        let src_idx = i.saturating_add(slots_per_epoch);
        let new_value = if src_idx < total_len {
            state_gloas
                .builder_pending_payments
                .get(src_idx)
                .cloned()
                .unwrap_or_default()
        } else {
            BuilderPendingPayment::default()
        };
        if let Some(slot) = state_gloas.builder_pending_payments.get_mut(i) {
            *slot = new_value;
        }
    }

    // Clear second half (set to default)
    for i in slots_per_epoch..total_len {
        if let Some(slot) = state_gloas.builder_pending_payments.get_mut(i) {
            *slot = BuilderPendingPayment::default();
        }
    }

    Ok(())
}
