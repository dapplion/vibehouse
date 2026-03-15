use crate::EpochProcessingError;
use safe_arith::SafeArith;
use types::historical_summary::HistoricalSummary;
use types::{BeaconState, EthSpec};

pub fn process_historical_summaries_update<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    // Set historical block root accumulator.
    let next_epoch = state.next_epoch()?;
    if next_epoch
        .as_u64()
        .safe_rem((E::slots_per_historical_root() as u64).safe_div(E::slots_per_epoch())?)?
        == 0
    {
        // We need to flush any pending mutations before hashing.
        state.block_roots_mut().apply_updates()?;
        state.state_roots_mut().apply_updates()?;
        let summary = HistoricalSummary::new(state);
        return state
            .historical_summaries_mut()?
            .push(summary)
            .map_err(Into::into);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_hash::TreeHash;
    use types::{Eth1Data, Hash256, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_state_at_slot(slot: u64) -> BeaconState<E> {
        // historical_summaries_update doesn't need validators, just a Capella+ state
        // Use Capella genesis spec so BeaconState::new creates a Capella state directly
        // But BeaconState::new always creates Base — we need to work around that.
        // Actually, historical_summaries is available on Capella+ states.
        // The simplest way: create a Capella state by constructing directly.
        // But let's just use the function and handle the Base state issue.
        let spec = E::default_spec();
        let mut state = BeaconState::<E>::new(0, Eth1Data::default(), &spec);
        // BeaconState::new creates Base. We need Capella for historical_summaries.
        // Add validators first so upgrade_to_altair works.
        let keypairs = types::test_utils::generate_deterministic_keypairs(4);
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
        crate::upgrade::upgrade_to_bellatrix(&mut state, &spec).unwrap();
        crate::upgrade::upgrade_to_capella(&mut state, &spec).unwrap();
        *state.slot_mut() = Slot::new(slot);
        state
    }

    /// For MinimalEthSpec: slots_per_historical_root = 64, slots_per_epoch = 8
    /// So boundary is every 64/8 = 8 epochs. next_epoch must be divisible by 8.
    /// next_epoch = current_epoch + 1, so current_epoch = 7 -> next_epoch = 8.
    /// slot = 7 * 8 = 56.
    fn boundary_slot() -> u64 {
        let epochs_per_period = E::slots_per_historical_root() as u64 / E::slots_per_epoch();
        // next_epoch = slot/8 + 1 needs to be divisible by epochs_per_period
        // so current_epoch = epochs_per_period - 1
        (epochs_per_period.saturating_sub(1)) * E::slots_per_epoch()
    }

    #[test]
    fn no_push_at_non_boundary() {
        let mut state = make_state_at_slot(8); // epoch 1, next_epoch = 2
        let count_before = state.historical_summaries().unwrap().len();
        process_historical_summaries_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_summaries().unwrap().len(), count_before);
    }

    #[test]
    fn push_at_boundary() {
        let mut state = make_state_at_slot(boundary_slot());
        let count_before = state.historical_summaries().unwrap().len();
        process_historical_summaries_update::<E>(&mut state).unwrap();
        assert_eq!(
            state.historical_summaries().unwrap().len(),
            count_before + 1
        );
    }

    #[test]
    fn summary_is_deterministic() {
        let slot = boundary_slot();
        let mut state1 = make_state_at_slot(slot);
        let mut state2 = make_state_at_slot(slot);

        process_historical_summaries_update::<E>(&mut state1).unwrap();
        process_historical_summaries_update::<E>(&mut state2).unwrap();

        let summary1 = state1.historical_summaries().unwrap().get(0).unwrap();
        let summary2 = state2.historical_summaries().unwrap().get(0).unwrap();
        assert_eq!(summary1.tree_hash_root(), summary2.tree_hash_root());
    }

    #[test]
    fn multiple_boundaries_accumulate() {
        let epochs_per_period = E::slots_per_historical_root() as u64 / E::slots_per_epoch();
        let mut state = make_state_at_slot(boundary_slot());
        process_historical_summaries_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_summaries().unwrap().len(), 1);

        // Advance to second boundary
        let second_boundary = (2 * epochs_per_period - 1) * E::slots_per_epoch();
        *state.slot_mut() = Slot::new(second_boundary);
        process_historical_summaries_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_summaries().unwrap().len(), 2);
    }

    #[test]
    fn different_roots_produce_different_summaries() {
        let slot = boundary_slot();
        let mut state1 = make_state_at_slot(slot);
        let mut state2 = make_state_at_slot(slot);

        // Modify a block root in state2
        *state2.block_roots_mut().get_mut(0).unwrap() = Hash256::repeat_byte(0xFF);

        process_historical_summaries_update::<E>(&mut state1).unwrap();
        process_historical_summaries_update::<E>(&mut state2).unwrap();

        let s1 = state1.historical_summaries().unwrap().get(0).unwrap();
        let s2 = state2.historical_summaries().unwrap().get(0).unwrap();
        assert_ne!(s1.tree_hash_root(), s2.tree_hash_root());
    }

    #[test]
    fn no_push_one_epoch_before_boundary() {
        let slot = boundary_slot().saturating_sub(E::slots_per_epoch());
        let mut state = make_state_at_slot(slot);
        let count_before = state.historical_summaries().unwrap().len();
        process_historical_summaries_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_summaries().unwrap().len(), count_before);
    }
}
