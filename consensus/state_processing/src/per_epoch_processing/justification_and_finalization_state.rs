use types::{BeaconState, BeaconStateError, BitVector, Checkpoint, Epoch, EthSpec, Hash256};

/// This is a subset of the `BeaconState` which is used to compute justification and finality
/// without modifying the `BeaconState`.
///
/// A `JustificationAndFinalizationState` can be created from a `BeaconState` to compute
/// justification/finality changes and then applied to a `BeaconState` to enshrine those changes.
#[must_use = "this value must be applied to a state or explicitly dropped"]
pub struct JustificationAndFinalizationState<E: EthSpec> {
    /*
     * Immutable fields.
     */
    previous_epoch: Epoch,
    previous_epoch_target_root: Result<Hash256, BeaconStateError>,
    current_epoch: Epoch,
    current_epoch_target_root: Result<Hash256, BeaconStateError>,
    /*
     * Mutable fields.
     */
    previous_justified_checkpoint: Checkpoint,
    current_justified_checkpoint: Checkpoint,
    finalized_checkpoint: Checkpoint,
    justification_bits: BitVector<E::JustificationBitsLength>,
}

impl<E: EthSpec> JustificationAndFinalizationState<E> {
    pub fn new(state: &BeaconState<E>) -> Self {
        let previous_epoch = state.previous_epoch();
        let current_epoch = state.current_epoch();
        Self {
            previous_epoch,
            previous_epoch_target_root: state.get_block_root_at_epoch(previous_epoch).copied(),
            current_epoch,
            current_epoch_target_root: state.get_block_root_at_epoch(current_epoch).copied(),
            previous_justified_checkpoint: state.previous_justified_checkpoint(),
            current_justified_checkpoint: state.current_justified_checkpoint(),
            finalized_checkpoint: state.finalized_checkpoint(),
            justification_bits: state.justification_bits().clone(),
        }
    }

    pub fn apply_changes_to_state(self, state: &mut BeaconState<E>) {
        let Self {
            /*
             * Immutable fields do not need to be used.
             */
            previous_epoch: _,
            previous_epoch_target_root: _,
            current_epoch: _,
            current_epoch_target_root: _,
            /*
             * Mutable fields *must* be used.
             */
            previous_justified_checkpoint,
            current_justified_checkpoint,
            finalized_checkpoint,
            justification_bits,
        } = self;

        *state.previous_justified_checkpoint_mut() = previous_justified_checkpoint;
        *state.current_justified_checkpoint_mut() = current_justified_checkpoint;
        *state.finalized_checkpoint_mut() = finalized_checkpoint;
        *state.justification_bits_mut() = justification_bits;
    }

    pub fn previous_epoch(&self) -> Epoch {
        self.previous_epoch
    }

    pub fn current_epoch(&self) -> Epoch {
        self.current_epoch
    }

    pub fn get_block_root_at_epoch(&self, epoch: Epoch) -> Result<Hash256, BeaconStateError> {
        if epoch == self.previous_epoch {
            self.previous_epoch_target_root.clone()
        } else if epoch == self.current_epoch {
            self.current_epoch_target_root.clone()
        } else {
            Err(BeaconStateError::SlotOutOfBounds)
        }
    }

    pub fn previous_justified_checkpoint(&self) -> Checkpoint {
        self.previous_justified_checkpoint
    }

    pub fn previous_justified_checkpoint_mut(&mut self) -> &mut Checkpoint {
        &mut self.previous_justified_checkpoint
    }

    pub fn current_justified_checkpoint_mut(&mut self) -> &mut Checkpoint {
        &mut self.current_justified_checkpoint
    }

    pub fn current_justified_checkpoint(&self) -> Checkpoint {
        self.current_justified_checkpoint
    }

    pub fn finalized_checkpoint(&self) -> Checkpoint {
        self.finalized_checkpoint
    }

    pub fn finalized_checkpoint_mut(&mut self) -> &mut Checkpoint {
        &mut self.finalized_checkpoint
    }

    pub fn justification_bits(&self) -> &BitVector<E::JustificationBitsLength> {
        &self.justification_bits
    }

    pub fn justification_bits_mut(&mut self) -> &mut BitVector<E::JustificationBitsLength> {
        &mut self.justification_bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ChainSpec, Eth1Data, FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn spec() -> ChainSpec {
        E::default_spec()
    }

    fn make_state() -> BeaconState<E> {
        BeaconState::new(0, Eth1Data::default(), &spec())
    }

    #[test]
    fn new_extracts_epochs() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        assert_eq!(jf.previous_epoch(), state.previous_epoch());
        assert_eq!(jf.current_epoch(), state.current_epoch());
    }

    #[test]
    fn new_extracts_checkpoints() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        assert_eq!(
            jf.previous_justified_checkpoint(),
            state.previous_justified_checkpoint()
        );
        assert_eq!(
            jf.current_justified_checkpoint(),
            state.current_justified_checkpoint()
        );
        assert_eq!(jf.finalized_checkpoint(), state.finalized_checkpoint());
    }

    #[test]
    fn new_extracts_justification_bits() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        assert_eq!(jf.justification_bits(), state.justification_bits());
    }

    #[test]
    fn get_block_root_at_previous_epoch_delegates() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        let epoch = jf.previous_epoch();
        // The result should match what the state returns
        let expected = state.get_block_root_at_epoch(epoch).copied();
        let actual = jf.get_block_root_at_epoch(epoch);
        assert_eq!(actual.is_ok(), expected.is_ok());
        if actual.is_ok() {
            assert_eq!(actual.unwrap(), expected.unwrap());
        }
    }

    #[test]
    fn get_block_root_at_current_epoch_delegates() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        let epoch = jf.current_epoch();
        let expected = state.get_block_root_at_epoch(epoch).copied();
        let actual = jf.get_block_root_at_epoch(epoch);
        assert_eq!(actual.is_ok(), expected.is_ok());
    }

    #[test]
    fn get_block_root_at_unknown_epoch_errors() {
        let state = make_state();
        let jf = JustificationAndFinalizationState::new(&state);
        // An epoch that is neither previous nor current should error
        let far_epoch = Epoch::new(9999);
        let result = jf.get_block_root_at_epoch(far_epoch);
        assert!(result.is_err());
    }

    #[test]
    fn mutable_checkpoint_setters() {
        let state = make_state();
        let mut jf = JustificationAndFinalizationState::new(&state);

        let new_cp = Checkpoint {
            epoch: Epoch::new(42),
            root: Hash256::from_low_u64_be(123),
        };

        *jf.previous_justified_checkpoint_mut() = new_cp;
        assert_eq!(jf.previous_justified_checkpoint(), new_cp);

        *jf.current_justified_checkpoint_mut() = new_cp;
        assert_eq!(jf.current_justified_checkpoint(), new_cp);

        *jf.finalized_checkpoint_mut() = new_cp;
        assert_eq!(jf.finalized_checkpoint(), new_cp);
    }

    #[test]
    fn justification_bits_mutable() {
        let state = make_state();
        let mut jf = JustificationAndFinalizationState::new(&state);

        // Initially all bits should be false
        assert!(!jf.justification_bits().get(0).unwrap());

        // Set bit 0
        jf.justification_bits_mut().set(0, true).unwrap();
        assert!(jf.justification_bits().get(0).unwrap());
    }

    #[test]
    fn apply_changes_updates_state() {
        let mut state = make_state();

        let mut jf = JustificationAndFinalizationState::new(&state);

        let new_cp = Checkpoint {
            epoch: Epoch::new(7),
            root: Hash256::from_low_u64_be(999),
        };

        *jf.previous_justified_checkpoint_mut() = new_cp;
        *jf.current_justified_checkpoint_mut() = new_cp;
        *jf.finalized_checkpoint_mut() = new_cp;
        jf.justification_bits_mut().set(1, true).unwrap();

        jf.apply_changes_to_state(&mut state);

        assert_eq!(state.previous_justified_checkpoint(), new_cp);
        assert_eq!(state.current_justified_checkpoint(), new_cp);
        assert_eq!(state.finalized_checkpoint(), new_cp);
        assert!(state.justification_bits().get(1).unwrap());
    }

    #[test]
    fn apply_changes_does_not_alter_immutable_fields() {
        let mut state = make_state();
        let slot_before = state.slot();

        let jf = JustificationAndFinalizationState::new(&state);
        jf.apply_changes_to_state(&mut state);

        // Slot and other non-J&F fields should be unchanged
        assert_eq!(state.slot(), slot_before);
    }

    #[test]
    fn roundtrip_no_modifications_is_identity() {
        let mut state = make_state();

        let prev_jc = state.previous_justified_checkpoint();
        let curr_jc = state.current_justified_checkpoint();
        let fin = state.finalized_checkpoint();
        let bits = state.justification_bits().clone();

        let jf = JustificationAndFinalizationState::new(&state);
        jf.apply_changes_to_state(&mut state);

        assert_eq!(state.previous_justified_checkpoint(), prev_jc);
        assert_eq!(state.current_justified_checkpoint(), curr_jc);
        assert_eq!(state.finalized_checkpoint(), fin);
        assert_eq!(state.justification_bits(), &bits);
    }
}
