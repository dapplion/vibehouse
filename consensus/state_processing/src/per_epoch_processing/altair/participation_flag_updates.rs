use crate::EpochProcessingError;
use types::List;
use types::beacon_state::BeaconState;
use types::eth_spec::EthSpec;
use types::participation_flags::ParticipationFlags;

pub fn process_participation_flag_updates<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    *state.previous_epoch_participation_mut()? =
        std::mem::take(state.current_epoch_participation_mut()?);
    *state.current_epoch_participation_mut()? =
        List::repeat(ParticipationFlags::default(), state.validators().len())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Eth1Data, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn make_state() -> BeaconState<E> {
        let spec = E::default_spec();
        let mut state = BeaconState::<E>::new(0, Eth1Data::default(), &spec);
        // Add validators to Base state, then upgrade to Altair
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
        state
    }

    #[test]
    fn current_moves_to_previous() {
        let mut state = make_state();
        let num_vals = state.validators().len();

        // Set some flags on current epoch participation
        for i in 0..num_vals {
            state
                .current_epoch_participation_mut()
                .unwrap()
                .get_mut(i)
                .unwrap()
                .add_flag(0)
                .unwrap();
        }

        let current_before: Vec<_> = state
            .current_epoch_participation()
            .unwrap()
            .iter()
            .cloned()
            .collect();

        process_participation_flag_updates::<E>(&mut state).unwrap();

        // Previous should now match what current was
        let previous_after: Vec<_> = state
            .previous_epoch_participation()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        assert_eq!(current_before, previous_after);
    }

    #[test]
    fn current_reset_to_defaults() {
        let mut state = make_state();
        let num_vals = state.validators().len();

        // Set flags on current
        for i in 0..num_vals {
            state
                .current_epoch_participation_mut()
                .unwrap()
                .get_mut(i)
                .unwrap()
                .add_flag(1)
                .unwrap();
        }

        process_participation_flag_updates::<E>(&mut state).unwrap();

        // Current should be all defaults (zero)
        for flag in state.current_epoch_participation().unwrap().iter() {
            assert_eq!(*flag, ParticipationFlags::default());
        }
    }

    #[test]
    fn current_length_matches_validators() {
        let mut state = make_state();
        process_participation_flag_updates::<E>(&mut state).unwrap();
        assert_eq!(
            state.current_epoch_participation().unwrap().len(),
            state.validators().len()
        );
    }

    #[test]
    fn previous_length_matches_validators() {
        let mut state = make_state();
        process_participation_flag_updates::<E>(&mut state).unwrap();
        assert_eq!(
            state.previous_epoch_participation().unwrap().len(),
            state.validators().len()
        );
    }

    #[test]
    fn double_update_clears_previous() {
        let mut state = make_state();
        let num_vals = state.validators().len();

        // Set flags on current
        for i in 0..num_vals {
            state
                .current_epoch_participation_mut()
                .unwrap()
                .get_mut(i)
                .unwrap()
                .add_flag(0)
                .unwrap();
        }

        // First update moves flags to previous
        process_participation_flag_updates::<E>(&mut state).unwrap();
        // Second update: previous should now be all defaults (the reset current)
        process_participation_flag_updates::<E>(&mut state).unwrap();

        for flag in state.previous_epoch_participation().unwrap().iter() {
            assert_eq!(*flag, ParticipationFlags::default());
        }
    }

    #[test]
    fn preserves_multiple_flags() {
        let mut state = make_state();

        // Set multiple flags on first validator
        let flags = state
            .current_epoch_participation_mut()
            .unwrap()
            .get_mut(0)
            .unwrap();
        flags.add_flag(0).unwrap();
        flags.add_flag(1).unwrap();
        flags.add_flag(2).unwrap();

        let expected = *state.current_epoch_participation().unwrap().get(0).unwrap();

        process_participation_flag_updates::<E>(&mut state).unwrap();

        let actual = *state
            .previous_epoch_participation()
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn mixed_flags_across_validators() {
        let mut state = make_state();
        let num_vals = state.validators().len();

        // Set different flags for different validators
        for i in 0..num_vals {
            let flag_index = i % 3;
            state
                .current_epoch_participation_mut()
                .unwrap()
                .get_mut(i)
                .unwrap()
                .add_flag(flag_index)
                .unwrap();
        }

        let current_snapshot: Vec<_> = state
            .current_epoch_participation()
            .unwrap()
            .iter()
            .cloned()
            .collect();

        process_participation_flag_updates::<E>(&mut state).unwrap();

        let previous_after: Vec<_> = state
            .previous_epoch_participation()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        assert_eq!(current_snapshot, previous_after);
    }
}
