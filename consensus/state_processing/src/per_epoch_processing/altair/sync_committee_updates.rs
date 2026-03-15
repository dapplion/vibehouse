use crate::EpochProcessingError;
use safe_arith::SafeArith;
use std::sync::Arc;
use types::beacon_state::BeaconState;
use types::chain_spec::ChainSpec;
use types::eth_spec::EthSpec;

pub fn process_sync_committee_updates<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), EpochProcessingError> {
    let next_epoch = state.next_epoch()?;
    if next_epoch.safe_rem(spec.epochs_per_sync_committee_period)? == 0 {
        *state.current_sync_committee_mut()? = state.next_sync_committee()?.clone();

        *state.next_sync_committee_mut()? = Arc::new(state.get_next_sync_committee(spec)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz_types::typenum::Unsigned;
    use types::{Eth1Data, MinimalEthSpec, Slot};

    type E = MinimalEthSpec;

    fn make_state_at_epoch(epoch: u64) -> (BeaconState<E>, ChainSpec) {
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
        let slot = Slot::new(epoch.saturating_mul(E::slots_per_epoch()));
        *state.slot_mut() = slot;
        state
            .build_all_committee_caches(&spec)
            .expect("should build caches");
        (state, spec)
    }

    #[test]
    fn no_update_at_non_boundary_epoch() {
        let (mut state, spec) = make_state_at_epoch(1);
        let current_before = state.current_sync_committee().unwrap().clone();
        let next_before = state.next_sync_committee().unwrap().clone();

        process_sync_committee_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.current_sync_committee().unwrap().pubkeys,
            current_before.pubkeys
        );
        assert_eq!(
            state.next_sync_committee().unwrap().pubkeys,
            next_before.pubkeys
        );
    }

    #[test]
    fn update_at_boundary_epoch() {
        let (mut state, spec) = make_state_at_epoch(7);
        let next_before = state.next_sync_committee().unwrap().clone();

        process_sync_committee_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.current_sync_committee().unwrap().pubkeys,
            next_before.pubkeys
        );
    }

    #[test]
    fn next_committee_changes_at_boundary() {
        let (mut state, spec) = make_state_at_epoch(7);

        process_sync_committee_updates::<E>(&mut state, &spec).unwrap();

        let next_after = state.next_sync_committee().unwrap();
        assert_eq!(
            next_after.pubkeys.len(),
            <E as EthSpec>::SyncCommitteeSize::to_usize()
        );
    }

    #[test]
    fn no_update_at_epoch_zero() {
        let (mut state, spec) = make_state_at_epoch(0);
        let current_before = state.current_sync_committee().unwrap().clone();

        process_sync_committee_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.current_sync_committee().unwrap().pubkeys,
            current_before.pubkeys
        );
    }

    #[test]
    fn update_at_second_boundary() {
        let (mut state, spec) = make_state_at_epoch(15);
        let next_before = state.next_sync_committee().unwrap().clone();

        process_sync_committee_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.current_sync_committee().unwrap().pubkeys,
            next_before.pubkeys
        );
    }
}
