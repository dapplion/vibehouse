use super::errors::EpochProcessingError;
use safe_arith::SafeArith;
use tree_hash::TreeHash;
use types::Unsigned;
use types::beacon_state::BeaconState;
use types::eth_spec::EthSpec;

pub fn process_historical_roots_update<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    let next_epoch = state.next_epoch()?;
    if next_epoch.as_u64().safe_rem(
        <E as EthSpec>::SlotsPerHistoricalRoot::to_u64().safe_div(E::slots_per_epoch())?,
    )? == 0
    {
        let historical_batch = state.historical_batch()?;
        state
            .historical_roots_mut()
            .push(historical_batch.tree_hash_root())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::*;

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        // Use Base since historical_roots is a Phase0 feature (replaced by historical_summaries in Capella)
        ForkName::Base.make_genesis_spec(E::default_spec())
    }

    fn make_phase0_state(spec: &ChainSpec) -> BeaconState<E> {
        BeaconState::new(0, Eth1Data::default(), spec)
    }

    #[test]
    fn pushes_root_at_boundary_epoch() {
        let spec = make_spec();
        let mut state = make_phase0_state(&spec);

        // The boundary is when next_epoch % (SlotsPerHistoricalRoot / slots_per_epoch) == 0
        // For minimal: SlotsPerHistoricalRoot = 64, slots_per_epoch = 8, so period = 8 epochs
        // next_epoch == 8 means current slot is at epoch 7
        let period = <E as EthSpec>::SlotsPerHistoricalRoot::to_u64() / E::slots_per_epoch();
        let boundary_epoch = period; // next_epoch = period
        let current_epoch = boundary_epoch - 1;
        *state.slot_mut() = Slot::new(current_epoch * E::slots_per_epoch());

        assert_eq!(state.historical_roots().len(), 0);

        process_historical_roots_update::<E>(&mut state).unwrap();

        assert_eq!(state.historical_roots().len(), 1);
    }

    #[test]
    fn no_push_at_non_boundary_epoch() {
        let spec = make_spec();
        let mut state = make_phase0_state(&spec);

        // Set to epoch 4 (next_epoch = 5), which is not a boundary
        *state.slot_mut() = Slot::new(4 * E::slots_per_epoch());

        process_historical_roots_update::<E>(&mut state).unwrap();

        assert_eq!(state.historical_roots().len(), 0);
    }

    #[test]
    fn multiple_boundaries_accumulate() {
        let spec = make_spec();
        let mut state = make_phase0_state(&spec);
        let period = <E as EthSpec>::SlotsPerHistoricalRoot::to_u64() / E::slots_per_epoch();

        // First boundary
        *state.slot_mut() = Slot::new((period - 1) * E::slots_per_epoch());
        process_historical_roots_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_roots().len(), 1);

        // Second boundary
        *state.slot_mut() = Slot::new((2 * period - 1) * E::slots_per_epoch());
        process_historical_roots_update::<E>(&mut state).unwrap();
        assert_eq!(state.historical_roots().len(), 2);
    }

    #[test]
    fn root_is_deterministic() {
        let spec = make_spec();
        let mut state1 = make_phase0_state(&spec);
        let mut state2 = make_phase0_state(&spec);

        let period = <E as EthSpec>::SlotsPerHistoricalRoot::to_u64() / E::slots_per_epoch();
        let boundary_slot = Slot::new((period - 1) * E::slots_per_epoch());

        *state1.slot_mut() = boundary_slot;
        *state2.slot_mut() = boundary_slot;

        process_historical_roots_update::<E>(&mut state1).unwrap();
        process_historical_roots_update::<E>(&mut state2).unwrap();

        assert_eq!(
            state1.historical_roots().get(0),
            state2.historical_roots().get(0),
        );
    }
}
