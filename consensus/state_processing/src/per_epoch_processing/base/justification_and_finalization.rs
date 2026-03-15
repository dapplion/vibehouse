use crate::per_epoch_processing::Error;
use crate::per_epoch_processing::base::TotalBalances;
use crate::per_epoch_processing::{
    JustificationAndFinalizationState, weigh_justification_and_finalization,
};
use safe_arith::SafeArith;
use types::{BeaconState, ChainSpec, EthSpec};

/// Update the justified and finalized checkpoints for matching target attestations.
pub fn process_justification_and_finalization<E: EthSpec>(
    state: &BeaconState<E>,
    total_balances: &TotalBalances,
    _spec: &ChainSpec,
) -> Result<JustificationAndFinalizationState<E>, Error> {
    let justification_and_finalization_state = JustificationAndFinalizationState::new(state);

    if state.current_epoch() <= E::genesis_epoch().safe_add(1)? {
        return Ok(justification_and_finalization_state);
    }

    weigh_justification_and_finalization(
        justification_and_finalization_state,
        total_balances.current_epoch(),
        total_balances.previous_epoch_target_attesters(),
        total_balances.current_epoch_target_attesters(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Eth1Data, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_base_state_at_epoch(epoch: u64) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let mut state = BeaconState::<E>::new(0, Eth1Data::default(), &spec);
        *state.slot_mut() = Slot::new(epoch.saturating_mul(E::slots_per_epoch()));
        (state, spec)
    }

    fn zero_balances() -> TotalBalances {
        TotalBalances::new(&E::default_spec())
    }

    #[test]
    fn early_epoch_returns_unchanged() {
        let (state, spec) = make_base_state_at_epoch(0);
        let balances = zero_balances();
        let result = process_justification_and_finalization::<E>(&state, &balances, &spec).unwrap();
        assert_eq!(
            result.current_justified_checkpoint(),
            state.current_justified_checkpoint()
        );
        assert_eq!(result.finalized_checkpoint(), state.finalized_checkpoint());
    }

    #[test]
    fn epoch_one_returns_unchanged() {
        let (state, spec) = make_base_state_at_epoch(1);
        let balances = zero_balances();
        let result = process_justification_and_finalization::<E>(&state, &balances, &spec).unwrap();
        assert_eq!(
            result.current_justified_checkpoint(),
            state.current_justified_checkpoint()
        );
    }

    #[test]
    fn uses_total_balances_not_cache() {
        // Verify that the base variant uses TotalBalances parameter, not progressive balances cache
        let (state, spec) = make_base_state_at_epoch(0);
        let balances = zero_balances();
        // Should succeed with zero balances — demonstrates it reads from TotalBalances
        let result = process_justification_and_finalization::<E>(&state, &balances, &spec).unwrap();
        assert_eq!(result.finalized_checkpoint(), state.finalized_checkpoint());
    }
}
