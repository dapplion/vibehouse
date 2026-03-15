use super::errors::{BlockOperationError, ProposerSlashingInvalid as Invalid};
use super::signature_sets::{get_pubkey_from_state, proposer_slashing_signature_set};
use crate::VerifySignatures;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<Invalid>>;

fn error(reason: Invalid) -> BlockOperationError<Invalid> {
    BlockOperationError::invalid(reason)
}

/// Indicates if a `ProposerSlashing` is valid to be included in a block in the current epoch of the given
/// state.
///
/// Returns `Ok(())` if the `ProposerSlashing` is valid, otherwise indicates the reason for invalidity.
///
/// Spec v0.12.1
pub fn verify_proposer_slashing<E: EthSpec>(
    proposer_slashing: &ProposerSlashing,
    state: &BeaconState<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<()> {
    let header_1 = &proposer_slashing.signed_header_1.message;
    let header_2 = &proposer_slashing.signed_header_2.message;

    // Verify slots match
    verify!(
        header_1.slot == header_2.slot,
        Invalid::ProposalSlotMismatch(header_1.slot, header_2.slot)
    );

    // Verify header proposer indices match
    verify!(
        header_1.proposer_index == header_2.proposer_index,
        Invalid::ProposerIndexMismatch(header_1.proposer_index, header_2.proposer_index)
    );

    // But the headers are different
    verify!(header_1 != header_2, Invalid::ProposalsIdentical);

    // Check proposer is slashable
    let proposer = state
        .validators()
        .get(header_1.proposer_index as usize)
        .ok_or_else(|| error(Invalid::ProposerUnknown(header_1.proposer_index)))?;

    verify!(
        proposer.is_slashable_at(state.current_epoch()),
        Invalid::ProposerNotSlashable(header_1.proposer_index)
    );

    if verify_signatures.is_true() {
        let (signature_set_1, signature_set_2) = proposer_slashing_signature_set(
            state,
            |i| get_pubkey_from_state(state, i),
            proposer_slashing,
            spec,
        )?;
        verify!(signature_set_1.verify(), Invalid::BadProposal1Signature);
        verify!(signature_set_2.verify(), Invalid::BadProposal2Signature);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{BeaconBlockHeader, ForkName, MinimalEthSpec, Signature, SignedBeaconBlockHeader};

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Create a minimal Gloas state with `num_validators` active validators.
    fn make_state(num_validators: usize, spec: &ChainSpec) -> BeaconState<E> {
        use bls::PublicKeyBytes;
        use std::sync::Arc;
        use types::{
            BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint, EpochCache, Eth1Data,
            ExecutionBlockHash, ExecutionPayloadBid, ExitCache, Fork, ProgressiveBalancesCache,
            PubkeyCache, SlashingsCache, SyncCommittee, beacon_state::BuilderPubkeyCache,
        };

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();
        let current_epoch = Epoch::new(10);
        let slot = current_epoch.start_slot(E::slots_per_epoch());

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for _ in 0..num_validators {
            validators.push(Validator {
                pubkey: PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::zero(),
                effective_balance: spec.max_effective_balance,
                slashed: false,
                activation_eligibility_epoch: Epoch::new(0),
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            });
            balances.push(spec.max_effective_balance);
        }

        BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: current_epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid::default(),
            next_withdrawal_index: 0,
            next_withdrawal_validator_index: 0,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 0,
            exit_balance_to_consume: 0,
            earliest_exit_epoch: Epoch::new(0),
            consolidation_balance_to_consume: 0,
            earliest_consolidation_epoch: Epoch::new(0),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::default(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::from_bytes(
                vec![0xFFu8; slots_per_hist / 8].into(),
            )
            .unwrap(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: ExecutionBlockHash::zero(),
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        })
    }

    fn make_header(slot: u64, proposer_index: u64, body_root: Hash256) -> BeaconBlockHeader {
        BeaconBlockHeader {
            slot: Slot::new(slot),
            proposer_index,
            parent_root: Hash256::zero(),
            state_root: Hash256::zero(),
            body_root,
        }
    }

    fn make_proposer_slashing(
        slot: u64,
        proposer_index: u64,
        body_root_1: Hash256,
        body_root_2: Hash256,
    ) -> ProposerSlashing {
        ProposerSlashing {
            signed_header_1: SignedBeaconBlockHeader {
                message: make_header(slot, proposer_index, body_root_1),
                signature: Signature::empty(),
            },
            signed_header_2: SignedBeaconBlockHeader {
                message: make_header(slot, proposer_index, body_root_2),
                signature: Signature::empty(),
            },
        }
    }

    // --- Valid slashing ---

    #[test]
    fn valid_proposer_slashing() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        let result = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec);
        assert!(result.is_ok());
    }

    // --- Slot mismatch ---

    #[test]
    fn slot_mismatch_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let mut slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        slashing.signed_header_2.message.slot = Slot::new(6);
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(
            matches!(
                err,
                BlockOperationError::Invalid(Invalid::ProposalSlotMismatch(_, _))
            ),
            "expected ProposalSlotMismatch, got {:?}",
            err
        );
    }

    // --- Proposer index mismatch ---

    #[test]
    fn proposer_index_mismatch_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let mut slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        slashing.signed_header_2.message.proposer_index = 1;
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposerIndexMismatch(0, 1))
        ));
    }

    // --- Identical proposals ---

    #[test]
    fn identical_proposals_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let body_root = Hash256::repeat_byte(0x01);
        let slashing = make_proposer_slashing(5, 0, body_root, body_root);
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposalsIdentical)
        ));
    }

    // --- Unknown proposer ---

    #[test]
    fn unknown_proposer_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec); // indices 0..3
        let slashing = make_proposer_slashing(
            5,
            99, // doesn't exist
            Hash256::repeat_byte(0x01),
            Hash256::repeat_byte(0x02),
        );
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposerUnknown(99))
        ));
    }

    // --- Already slashed proposer ---

    #[test]
    fn already_slashed_proposer_returns_error() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        let slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposerNotSlashable(0))
        ));
    }

    // --- Exited (withdrawable) proposer ---

    #[test]
    fn withdrawable_proposer_returns_error() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        let current_epoch = state.current_epoch();
        let v = state.validators_mut().get_mut(0).unwrap();
        v.exit_epoch = current_epoch - 2;
        v.withdrawable_epoch = current_epoch - 1;
        let slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposerNotSlashable(0))
        ));
    }

    // --- Exited but still slashable (exit_epoch < current but withdrawable_epoch > current) ---

    #[test]
    fn exited_but_not_withdrawable_is_still_slashable() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        let current_epoch = state.current_epoch();
        let v = state.validators_mut().get_mut(0).unwrap();
        v.exit_epoch = current_epoch - 1;
        v.withdrawable_epoch = current_epoch + 10; // not withdrawable yet
        let slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        let result = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "exited but not-yet-withdrawable should be slashable"
        );
    }

    // --- Not yet activated ---

    #[test]
    fn not_yet_activated_proposer_returns_error() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        let current_epoch = state.current_epoch();
        state.validators_mut().get_mut(0).unwrap().activation_epoch = current_epoch + 1;
        let slashing =
            make_proposer_slashing(5, 0, Hash256::repeat_byte(0x01), Hash256::repeat_byte(0x02));
        let err = verify_proposer_slashing(&slashing, &state, VerifySignatures::False, &spec)
            .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::ProposerNotSlashable(0))
        ));
    }
}
