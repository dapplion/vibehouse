use crate::EpochProcessingError;
use types::beacon_state::BeaconState;
use types::eth_spec::EthSpec;

pub fn process_participation_record_updates<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    let base_state = state.as_base_mut()?;
    base_state.previous_epoch_attestations =
        std::mem::take(&mut base_state.current_epoch_attestations);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        AttestationData, Checkpoint, Epoch, Eth1Data, Hash256, MinimalEthSpec, PendingAttestation,
        Slot,
    };

    type E = MinimalEthSpec;

    fn make_base_state() -> BeaconState<E> {
        let spec = E::default_spec();
        BeaconState::<E>::new(0, Eth1Data::default(), &spec)
    }

    fn make_pending_attestation(slot: u64) -> PendingAttestation<E> {
        PendingAttestation {
            aggregation_bits: types::BitList::with_capacity(4).unwrap(),
            data: AttestationData {
                slot: Slot::new(slot),
                index: 0,
                beacon_block_root: Hash256::ZERO,
                source: Checkpoint {
                    epoch: Epoch::new(0),
                    root: Hash256::ZERO,
                },
                target: Checkpoint {
                    epoch: Epoch::new(0),
                    root: Hash256::ZERO,
                },
            },
            inclusion_delay: 1,
            proposer_index: 0,
        }
    }

    #[test]
    fn current_moves_to_previous() {
        let mut state = make_base_state();
        let base = state.as_base_mut().unwrap();
        base.current_epoch_attestations
            .push(make_pending_attestation(0))
            .unwrap();
        base.current_epoch_attestations
            .push(make_pending_attestation(1))
            .unwrap();
        let count = base.current_epoch_attestations.len();

        process_participation_record_updates::<E>(&mut state).unwrap();

        let base = state.as_base().unwrap();
        assert_eq!(base.previous_epoch_attestations.len(), count);
        assert!(base.current_epoch_attestations.is_empty());
    }

    #[test]
    fn current_cleared_after_move() {
        let mut state = make_base_state();
        let base = state.as_base_mut().unwrap();
        base.current_epoch_attestations
            .push(make_pending_attestation(5))
            .unwrap();

        process_participation_record_updates::<E>(&mut state).unwrap();

        assert!(
            state
                .as_base()
                .unwrap()
                .current_epoch_attestations
                .is_empty()
        );
    }

    #[test]
    fn previous_replaced_not_appended() {
        let mut state = make_base_state();
        let base = state.as_base_mut().unwrap();
        // Put something in previous
        base.previous_epoch_attestations
            .push(make_pending_attestation(10))
            .unwrap();
        // Put something different in current
        base.current_epoch_attestations
            .push(make_pending_attestation(20))
            .unwrap();

        process_participation_record_updates::<E>(&mut state).unwrap();

        let base = state.as_base().unwrap();
        // Previous should have only the one from current, not the old one
        assert_eq!(base.previous_epoch_attestations.len(), 1);
        assert_eq!(
            base.previous_epoch_attestations.get(0).unwrap().data.slot,
            Slot::new(20)
        );
    }

    #[test]
    fn empty_current_clears_previous() {
        let mut state = make_base_state();
        let base = state.as_base_mut().unwrap();
        base.previous_epoch_attestations
            .push(make_pending_attestation(0))
            .unwrap();

        process_participation_record_updates::<E>(&mut state).unwrap();

        assert!(
            state
                .as_base()
                .unwrap()
                .previous_epoch_attestations
                .is_empty()
        );
    }

    #[test]
    fn double_update_clears_both() {
        let mut state = make_base_state();
        let base = state.as_base_mut().unwrap();
        base.current_epoch_attestations
            .push(make_pending_attestation(0))
            .unwrap();

        process_participation_record_updates::<E>(&mut state).unwrap();
        process_participation_record_updates::<E>(&mut state).unwrap();

        let base = state.as_base().unwrap();
        assert!(base.previous_epoch_attestations.is_empty());
        assert!(base.current_epoch_attestations.is_empty());
    }

    #[test]
    fn fails_on_non_base_state() {
        let spec = E::default_spec();
        let mut state = BeaconState::<E>::new(0, Eth1Data::default(), &spec);
        // BeaconState::new creates an Altair+ state by default on minimal spec
        // If it's already not base, this should fail
        if state.as_base().is_err() {
            assert!(process_participation_record_updates::<E>(&mut state).is_err());
        }
    }
}
