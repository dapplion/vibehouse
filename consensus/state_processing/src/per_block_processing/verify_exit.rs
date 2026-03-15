use super::errors::{BlockOperationError, ExitInvalid};
use crate::per_block_processing::gloas::get_pending_balance_to_withdraw_for_builder;
use crate::per_block_processing::{
    VerifySignatures,
    signature_sets::{exit_signature_set, get_pubkey_from_state},
};
use safe_arith::SafeArith;
use std::borrow::Cow;
use types::consts::gloas::BUILDER_INDEX_FLAG;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<ExitInvalid>>;

fn error(reason: ExitInvalid) -> BlockOperationError<ExitInvalid> {
    BlockOperationError::invalid(reason)
}

/// Returns true if the validator_index has the BUILDER_INDEX_FLAG set,
/// indicating it refers to a builder rather than a validator.
fn is_builder_index(validator_index: u64) -> bool {
    (validator_index & BUILDER_INDEX_FLAG) != 0
}

/// Extract the builder index from a flagged validator_index.
fn to_builder_index(validator_index: u64) -> u64 {
    validator_index & !BUILDER_INDEX_FLAG
}

/// Indicates if a voluntary exit is valid to be included in a block.
///
/// [Modified in Gloas:EIP7732] Now supports builder exits when
/// `exit.validator_index` has `BUILDER_INDEX_FLAG` set.
///
/// Returns `Ok(true)` for builder exits, `Ok(false)` for validator exits.
pub fn verify_exit<E: EthSpec>(
    state: &BeaconState<E>,
    current_epoch: Option<Epoch>,
    signed_exit: &SignedVoluntaryExit,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<bool> {
    let current_epoch = current_epoch.unwrap_or(state.current_epoch());
    let exit = &signed_exit.message;

    // Exits must specify an epoch when they become valid; they are not valid before then.
    verify!(
        current_epoch >= exit.epoch,
        ExitInvalid::FutureEpoch {
            state: current_epoch,
            exit: exit.epoch
        }
    );

    // [New in Gloas:EIP7732] Handle builder exits
    if state.fork_name_unchecked().gloas_enabled() && is_builder_index(exit.validator_index) {
        let builder_index = to_builder_index(exit.validator_index);
        return verify_builder_exit(
            state,
            current_epoch,
            builder_index,
            signed_exit,
            verify_signatures,
            spec,
        )
        .map(|()| true);
    }

    // Validator exit path (unchanged from pre-Gloas)
    let validator = state
        .validators()
        .get(exit.validator_index as usize)
        .ok_or_else(|| error(ExitInvalid::ValidatorUnknown(exit.validator_index)))?;

    // Verify the validator is active.
    verify!(
        validator.is_active_at(current_epoch),
        ExitInvalid::NotActive(exit.validator_index)
    );

    // Verify that the validator has not yet exited.
    verify!(
        validator.exit_epoch == spec.far_future_epoch,
        ExitInvalid::AlreadyExited(exit.validator_index)
    );

    // Verify the validator has been active long enough.
    let earliest_exit_epoch = validator
        .activation_epoch
        .safe_add(spec.shard_committee_period)?;
    verify!(
        current_epoch >= earliest_exit_epoch,
        ExitInvalid::TooYoungToExit {
            current_epoch,
            earliest_exit_epoch,
        }
    );

    if verify_signatures.is_true() {
        verify!(
            exit_signature_set(
                state,
                |i| get_pubkey_from_state(state, i),
                signed_exit,
                spec
            )?
            .verify(),
            ExitInvalid::BadSignature
        );
    }

    // [New in Electra:EIP7251]
    // Only exit validator if it has no pending withdrawals in the queue
    if let Ok(pending_balance_to_withdraw) =
        state.get_pending_balance_to_withdraw(exit.validator_index as usize)
    {
        verify!(
            pending_balance_to_withdraw == 0,
            ExitInvalid::PendingWithdrawalInQueue(exit.validator_index)
        );
    }

    Ok(false)
}

/// Verify a builder voluntary exit.
///
/// Spec: process_voluntary_exit (builder branch) in Gloas
fn verify_builder_exit<E: EthSpec>(
    state: &BeaconState<E>,
    _current_epoch: Epoch,
    builder_index: u64,
    signed_exit: &SignedVoluntaryExit,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<()> {
    let finalized_epoch = state.finalized_checkpoint().epoch;

    let builders = state
        .builders()
        .map_err(|_| error(ExitInvalid::BuilderUnknown(builder_index)))?;

    let builder = builders
        .get(builder_index as usize)
        .ok_or_else(|| error(ExitInvalid::BuilderUnknown(builder_index)))?;

    // Verify the builder is active
    verify!(
        builder.is_active_at_finalized_epoch(finalized_epoch, spec),
        ExitInvalid::BuilderNotActive(builder_index)
    );

    // Only exit builder if it has no pending withdrawals in the queue
    let pending = get_pending_balance_to_withdraw_for_builder(state, builder_index)
        .map_err(BlockOperationError::BeaconStateError)?;
    verify!(
        pending == 0,
        ExitInvalid::BuilderPendingWithdrawalInQueue(builder_index)
    );

    // Verify signature using builder's pubkey
    if verify_signatures.is_true() {
        let get_builder_pubkey = |_i: usize| -> Option<Cow<PublicKey>> {
            builder.pubkey.decompress().ok().map(Cow::Owned)
        };

        verify!(
            exit_signature_set(state, get_builder_pubkey, signed_exit, spec)?.verify(),
            ExitInvalid::BadSignature
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::PublicKeyBytes;
    use std::sync::Arc;
    use tree_hash::TreeHash;
    use types::{
        BeaconState, BeaconStateGloas, BitVector, Builder, BuilderPendingPayment,
        BuilderPendingWithdrawal, Checkpoint, EpochCache, Eth1Data, ExecutionBlockHash,
        ExecutionPayloadBid, ExitCache, ForkName, MinimalEthSpec, PendingPartialWithdrawal,
        ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee, Validator,
        beacon_state::BuilderPubkeyCache,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Create a minimal Gloas state with `num_validators` active validators
    /// and `num_builders` active builders.
    fn make_gloas_state(
        num_validators: usize,
        num_builders: usize,
        spec: &ChainSpec,
    ) -> BeaconState<E> {
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Put state far enough ahead that shard_committee_period is satisfied
        // for validators activated at epoch 0.
        let current_epoch = Epoch::new(spec.shard_committee_period + 10);
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

        let mut builders_list = Vec::with_capacity(num_builders);
        for _ in 0..num_builders {
            builders_list.push(Builder {
                pubkey: PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xBB),
                balance: 64_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(u64::MAX),
            });
        }

        // Finalized checkpoint at epoch 5 so builders deposited at epoch 0 are active
        let finalized_checkpoint = Checkpoint {
            epoch: Epoch::new(5),
            root: Hash256::zero(),
        };

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
            finalized_checkpoint,
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
            builders: List::new(builders_list).unwrap(),
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

    fn make_signed_exit(validator_index: u64, epoch: Epoch) -> SignedVoluntaryExit {
        SignedVoluntaryExit {
            message: VoluntaryExit {
                epoch,
                validator_index,
            },
            signature: Signature::empty(),
        }
    }

    // --- Helper function tests ---

    #[test]
    fn is_builder_index_without_flag() {
        assert!(!is_builder_index(0));
        assert!(!is_builder_index(42));
        assert!(!is_builder_index((1u64 << 40) - 1));
    }

    #[test]
    fn is_builder_index_with_flag() {
        assert!(is_builder_index(BUILDER_INDEX_FLAG));
        assert!(is_builder_index(BUILDER_INDEX_FLAG | 7));
        assert!(is_builder_index(BUILDER_INDEX_FLAG | 999));
    }

    #[test]
    fn to_builder_index_extracts_index() {
        assert_eq!(to_builder_index(BUILDER_INDEX_FLAG), 0);
        assert_eq!(to_builder_index(BUILDER_INDEX_FLAG | 5), 5);
        assert_eq!(to_builder_index(BUILDER_INDEX_FLAG | 42), 42);
    }

    // --- Validator exit tests ---

    #[test]
    fn validator_exit_success() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();
        let exit = make_signed_exit(0, current_epoch);

        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        // Returns Ok(false) for validator exits (not a builder exit)
        assert!(!result.unwrap());
    }

    #[test]
    fn validator_exit_future_epoch() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();
        let future = current_epoch + 1;
        let exit = make_signed_exit(0, future);

        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(
                ExitInvalid::FutureEpoch { .. }
            ))
        ));
    }

    #[test]
    fn validator_exit_unknown_validator() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();
        // Index 999 doesn't exist
        let exit = make_signed_exit(999, current_epoch);

        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::ValidatorUnknown(
                999
            )))
        ));
    }

    #[test]
    fn validator_exit_not_active() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Set validator 0 activation epoch to far future (not yet active)
        state.validators_mut().get_mut(0).unwrap().activation_epoch = spec.far_future_epoch;

        let exit = make_signed_exit(0, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::NotActive(0)))
        ));
    }

    #[test]
    fn validator_exit_already_exited() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Set validator 0 as already exited (but still technically active at current_epoch)
        // exit_epoch must be > current_epoch so is_active_at passes, but != far_future_epoch
        state.validators_mut().get_mut(0).unwrap().exit_epoch = current_epoch + 1;

        let exit = make_signed_exit(0, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::AlreadyExited(0)))
        ));
    }

    #[test]
    fn validator_exit_too_young() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Set validator activation to current epoch so shard_committee_period isn't satisfied
        state.validators_mut().get_mut(0).unwrap().activation_epoch = current_epoch;

        let exit = make_signed_exit(0, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(
                ExitInvalid::TooYoungToExit { .. }
            ))
        ));
    }

    #[test]
    fn validator_exit_pending_withdrawal_in_queue() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Add a pending partial withdrawal for validator 0
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000,
                withdrawable_epoch: current_epoch + 1,
            })
            .unwrap();

        let exit = make_signed_exit(0, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(
                ExitInvalid::PendingWithdrawalInQueue(0)
            ))
        ));
    }

    #[test]
    fn validator_exit_with_explicit_current_epoch() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();
        let exit = make_signed_exit(0, current_epoch);

        // Pass explicit current_epoch matching state
        let result = verify_exit(
            &state,
            Some(current_epoch),
            &exit,
            VerifySignatures::False,
            &spec,
        );
        assert!(!result.unwrap());
    }

    // --- Builder exit tests ---

    #[test]
    fn builder_exit_success() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Builder index 0 with BUILDER_INDEX_FLAG
        let exit = make_signed_exit(BUILDER_INDEX_FLAG, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        // Returns Ok(true) for builder exits
        assert!(result.unwrap());
    }

    #[test]
    fn builder_exit_unknown_builder() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Builder index 99 doesn't exist (only 1 builder)
        let exit = make_signed_exit(BUILDER_INDEX_FLAG | 99, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::BuilderUnknown(
                99
            )))
        ));
    }

    #[test]
    fn builder_exit_not_active() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Set builder as already withdrawable (not active)
        state
            .builders_mut()
            .unwrap()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(10);

        let exit = make_signed_exit(BUILDER_INDEX_FLAG, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::BuilderNotActive(
                0
            )))
        ));
    }

    #[test]
    fn builder_exit_not_active_deposit_not_finalized() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Set builder deposit_epoch after finalized_epoch (not yet finalized)
        state
            .builders_mut()
            .unwrap()
            .get_mut(0)
            .unwrap()
            .deposit_epoch = Epoch::new(100);

        let exit = make_signed_exit(BUILDER_INDEX_FLAG, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::BuilderNotActive(
                0
            )))
        ));
    }

    #[test]
    fn builder_exit_pending_withdrawal_in_queue() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();

        // Add a pending builder withdrawal for builder 0
        state
            .builder_pending_withdrawals_mut()
            .unwrap()
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 1_000_000,
                builder_index: 0,
            })
            .unwrap();

        let exit = make_signed_exit(BUILDER_INDEX_FLAG, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(
                ExitInvalid::BuilderPendingWithdrawalInQueue(0)
            ))
        ));
    }

    #[test]
    fn builder_exit_future_epoch_checked_before_builder_dispatch() {
        let spec = make_spec();
        let state = make_gloas_state(4, 1, &spec);
        let current_epoch = state.current_epoch();
        let future = current_epoch + 1;

        // Future epoch check happens before the builder/validator dispatch
        let exit = make_signed_exit(BUILDER_INDEX_FLAG, future);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(
                ExitInvalid::FutureEpoch { .. }
            ))
        ));
    }

    // --- Pre-Gloas state with builder index ---

    #[test]
    fn builder_index_on_pre_gloas_state_treated_as_validator() {
        // On a pre-Gloas state, the BUILDER_INDEX_FLAG is not recognized,
        // so the large index is treated as a (nonexistent) validator index.
        let spec = ForkName::Fulu.make_genesis_spec(E::default_spec());

        let keypairs = types::test_utils::generate_deterministic_keypairs(4);
        let mut deposit_datas = Vec::with_capacity(4);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = spec.eth1_address_withdrawal_prefix_byte;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            let withdrawal_credentials = Hash256::from_slice(&creds);
            let mut data = types::DepositData {
                pubkey: kp.pk.clone().into(),
                withdrawal_credentials,
                amount: spec.max_effective_balance,
                signature: types::Signature::empty().into(),
            };
            data.signature = data.create_signature(&kp.sk, &spec);
            deposit_datas.push(data);
        }

        let deposit_tree_depth = types::DEPOSIT_TREE_DEPTH;
        let mut tree = crate::common::DepositDataTree::create(&[], 0, deposit_tree_depth);
        let mut deposits = Vec::with_capacity(4);
        for data in deposit_datas {
            tree.push_leaf(data.tree_hash_root())
                .expect("should push leaf");
            let (_leaf, proof_vec) = tree
                .generate_proof(deposits.len())
                .expect("should generate proof");
            let mut proof = types::FixedVector::from(vec![Hash256::zero(); deposit_tree_depth + 1]);
            for (i, node) in proof_vec.iter().enumerate() {
                proof[i] = *node;
            }
            deposits.push(types::Deposit { proof, data });
        }

        let state = crate::initialize_beacon_state_from_eth1::<E>(
            Hash256::repeat_byte(0x42),
            2u64.pow(40),
            deposits,
            None,
            &spec,
        )
        .expect("should initialize state");

        assert!(!state.fork_name_unchecked().gloas_enabled());

        let current_epoch = state.current_epoch();
        // Use builder-flagged index on pre-Gloas state — should be treated as validator index
        let exit = make_signed_exit(BUILDER_INDEX_FLAG, current_epoch);
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        // The large index is out of range for the validator registry
        assert!(matches!(
            result,
            Err(BlockOperationError::Invalid(ExitInvalid::ValidatorUnknown(
                _
            )))
        ));
    }
}
