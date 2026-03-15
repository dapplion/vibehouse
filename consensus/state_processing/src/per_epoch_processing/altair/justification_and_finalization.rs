use crate::per_epoch_processing::Error;
use crate::per_epoch_processing::{
    JustificationAndFinalizationState, weigh_justification_and_finalization,
};
use safe_arith::SafeArith;
use types::{BeaconState, EthSpec};

/// Process justification and finalization using the progressive balances cache.
pub fn process_justification_and_finalization<E: EthSpec>(
    state: &BeaconState<E>,
) -> Result<JustificationAndFinalizationState<E>, Error> {
    let justification_and_finalization_state = JustificationAndFinalizationState::new(state);
    if state.current_epoch() <= E::genesis_epoch().safe_add(1)? {
        return Ok(justification_and_finalization_state);
    }

    // Load cached balances
    let progressive_balances_cache = state.progressive_balances_cache();
    let previous_target_balance =
        progressive_balances_cache.previous_epoch_target_attesting_balance()?;
    let current_target_balance =
        progressive_balances_cache.current_epoch_target_attesting_balance()?;
    let total_active_balance = state.get_total_active_balance()?;

    weigh_justification_and_finalization(
        justification_and_finalization_state,
        total_active_balance,
        previous_target_balance,
        current_target_balance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Eth1Data, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_altair_state_at_epoch(epoch: u64) -> BeaconState<E> {
        let spec = E::default_spec();
        let mut state = BeaconState::<E>::new(0, Eth1Data::default(), &spec);
        let keypairs = types::test_utils::generate_deterministic_keypairs(8);
        for kp in &keypairs {
            let v = types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: spec.max_effective_balance,
                activation_epoch: types::Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                ..types::Validator::default()
            };
            state.validators_mut().push(v).unwrap();
            state
                .balances_mut()
                .push(spec.max_effective_balance)
                .unwrap();
        }
        crate::upgrade::upgrade_to_altair(&mut state, &spec).unwrap();
        *state.slot_mut() = Slot::new(epoch.saturating_mul(E::slots_per_epoch()));
        state.build_all_committee_caches(&spec).unwrap();
        crate::common::update_progressive_balances_cache::initialize_progressive_balances_cache(
            &mut state, &spec,
        )
        .unwrap();
        state.build_total_active_balance_cache(&spec).unwrap();
        state
    }

    #[test]
    fn early_epoch_returns_unchanged_state() {
        let state = make_altair_state_at_epoch(0);
        let result = process_justification_and_finalization::<E>(&state).unwrap();
        assert_eq!(
            result.current_justified_checkpoint(),
            state.current_justified_checkpoint()
        );
        assert_eq!(result.finalized_checkpoint(), state.finalized_checkpoint());
    }

    #[test]
    fn epoch_one_returns_unchanged() {
        let state = make_altair_state_at_epoch(1);
        let result = process_justification_and_finalization::<E>(&state).unwrap();
        assert_eq!(
            result.current_justified_checkpoint(),
            state.current_justified_checkpoint()
        );
    }

    #[test]
    fn epoch_two_runs_justification() {
        let state = make_altair_state_at_epoch(2);
        let _result = process_justification_and_finalization::<E>(&state).unwrap();
    }

    #[test]
    fn returns_previous_justified_checkpoint() {
        let state = make_altair_state_at_epoch(3);
        let result = process_justification_and_finalization::<E>(&state).unwrap();
        assert_eq!(
            result.previous_justified_checkpoint(),
            state.previous_justified_checkpoint()
        );
    }
}
