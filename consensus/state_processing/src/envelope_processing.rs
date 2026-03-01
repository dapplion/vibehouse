use crate::BlockProcessingError;
use crate::VerifySignatures;
use crate::per_block_processing::compute_timestamp_at_slot;
use crate::per_block_processing::process_operations::{
    process_consolidation_requests, process_deposit_requests, process_withdrawal_requests,
};
use crate::signature_sets::execution_payload_envelope_signature_set;
use safe_arith::{ArithError, SafeArith};
use std::borrow::Cow;
use tree_hash::TreeHash;
use types::{
    BeaconState, BeaconStateError, BuilderIndex, BuilderPendingPayment, ChainSpec, EthSpec,
    ExecutionBlockHash, Hash256, PublicKey, SignedExecutionPayloadEnvelope, Slot,
    consts::gloas::BUILDER_INDEX_SELF_BUILD,
};

macro_rules! envelope_verify {
    ($condition: expr, $result: expr) => {
        if !$condition {
            return Err($result);
        }
    };
}

#[derive(Debug, PartialEq, Clone)]
pub enum EnvelopeProcessingError {
    /// Bad Signature
    BadSignature,
    BeaconStateError(BeaconStateError),
    BlockProcessingError(BlockProcessingError),
    ArithError(ArithError),
    /// Envelope doesn't match latest beacon block header
    LatestBlockHeaderMismatch {
        envelope_root: Hash256,
        block_header_root: Hash256,
    },
    /// Envelope doesn't match latest beacon block slot
    SlotMismatch {
        envelope_slot: Slot,
        parent_state_slot: Slot,
    },
    /// The payload withdrawals don't match the state's payload withdrawals.
    WithdrawalsRootMismatch {
        state: Hash256,
        payload: Hash256,
    },
    /// The builder index doesn't match the committed bid.
    BuilderIndexMismatch {
        committed_bid: BuilderIndex,
        envelope: BuilderIndex,
    },
    /// The gas limit doesn't match the committed bid
    GasLimitMismatch {
        committed_bid: u64,
        envelope: u64,
    },
    /// The block hash doesn't match the committed bid
    BlockHashMismatch {
        committed_bid: ExecutionBlockHash,
        envelope: ExecutionBlockHash,
    },
    /// The parent hash doesn't match the previous execution payload
    ParentHashMismatch {
        state: ExecutionBlockHash,
        envelope: ExecutionBlockHash,
    },
    /// The previous randao didn't match the payload
    PrevRandaoMismatch {
        committed_bid: Hash256,
        envelope: Hash256,
    },
    /// The timestamp didn't match the payload
    TimestampMismatch {
        state: u64,
        envelope: u64,
    },
    /// Invalid state root
    InvalidStateRoot {
        state: Hash256,
        envelope: Hash256,
    },
    /// BitFieldError
    BitFieldError(ssz::BitfieldError),
    /// Some kind of error calculating the builder payment index
    BuilderPaymentIndexOutOfBounds(usize),
}

impl From<BeaconStateError> for EnvelopeProcessingError {
    fn from(e: BeaconStateError) -> Self {
        EnvelopeProcessingError::BeaconStateError(e)
    }
}

impl From<BlockProcessingError> for EnvelopeProcessingError {
    fn from(e: BlockProcessingError) -> Self {
        EnvelopeProcessingError::BlockProcessingError(e)
    }
}

impl From<ArithError> for EnvelopeProcessingError {
    fn from(e: ArithError) -> Self {
        EnvelopeProcessingError::ArithError(e)
    }
}

/// Processes a `SignedExecutionPayloadEnvelope` according to the Gloas spec.
///
/// This function does all the state modifications inside `process_execution_payload()`.
/// It is the second half of the two-phase Gloas state transition (first is block processing).
///
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_execution_payload>
pub fn process_execution_payload_envelope<E: EthSpec>(
    state: &mut BeaconState<E>,
    parent_state_root: Option<Hash256>,
    signed_envelope: &SignedExecutionPayloadEnvelope<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), EnvelopeProcessingError> {
    if verify_signatures.is_true() {
        let get_builder_pubkey = |builder_idx: u64| -> Option<Cow<PublicKey>> {
            if builder_idx == BUILDER_INDEX_SELF_BUILD {
                // Self-build: use the proposer's validator pubkey
                let proposer_index = state.latest_block_header().proposer_index as usize;
                state
                    .validators()
                    .get(proposer_index)
                    .and_then(|v| v.pubkey.decompress().ok().map(Cow::Owned))
            } else {
                // Builder-submitted: use the builder's pubkey from the registry
                state
                    .builders()
                    .ok()?
                    .get(builder_idx as usize)
                    .and_then(|builder| builder.pubkey.decompress().ok().map(Cow::Owned))
            }
        };

        let signature_set = execution_payload_envelope_signature_set(
            state,
            get_builder_pubkey,
            signed_envelope,
            spec,
        )
        .map_err(|_| EnvelopeProcessingError::BadSignature)?;

        if !signature_set.verify() {
            return Err(EnvelopeProcessingError::BadSignature);
        }
    }

    let envelope = &signed_envelope.message;
    let payload = &envelope.payload;
    let execution_requests = &envelope.execution_requests;

    // Cache latest block header state root
    if state.latest_block_header().state_root == Hash256::default() {
        let previous_state_root = parent_state_root
            .map(Ok)
            .unwrap_or_else(|| state.canonical_root())?;
        state.latest_block_header_mut().state_root = previous_state_root;
    }

    // Verify consistency with the beacon block
    let latest_block_header_root = state.latest_block_header().tree_hash_root();
    envelope_verify!(
        envelope.beacon_block_root == latest_block_header_root,
        EnvelopeProcessingError::LatestBlockHeaderMismatch {
            envelope_root: envelope.beacon_block_root,
            block_header_root: latest_block_header_root,
        }
    );
    envelope_verify!(
        envelope.slot == state.slot(),
        EnvelopeProcessingError::SlotMismatch {
            envelope_slot: envelope.slot,
            parent_state_slot: state.slot(),
        }
    );

    // Verify consistency with the committed bid
    let committed_bid = state.latest_execution_payload_bid()?;
    envelope_verify!(
        envelope.builder_index == committed_bid.builder_index,
        EnvelopeProcessingError::BuilderIndexMismatch {
            committed_bid: committed_bid.builder_index,
            envelope: envelope.builder_index,
        }
    );
    envelope_verify!(
        committed_bid.prev_randao == payload.prev_randao,
        EnvelopeProcessingError::PrevRandaoMismatch {
            committed_bid: committed_bid.prev_randao,
            envelope: payload.prev_randao,
        }
    );

    // Verify consistency with expected withdrawals
    let expected_withdrawals = state.payload_expected_withdrawals()?;
    envelope_verify!(
        payload.withdrawals.len() == expected_withdrawals.len()
            && payload.withdrawals.iter().eq(expected_withdrawals.iter()),
        EnvelopeProcessingError::WithdrawalsRootMismatch {
            state: expected_withdrawals.tree_hash_root(),
            payload: payload.withdrawals.tree_hash_root(),
        }
    );

    // Verify the gas limit
    envelope_verify!(
        committed_bid.gas_limit == payload.gas_limit,
        EnvelopeProcessingError::GasLimitMismatch {
            committed_bid: committed_bid.gas_limit,
            envelope: payload.gas_limit,
        }
    );

    // Verify the block hash
    envelope_verify!(
        committed_bid.block_hash == payload.block_hash,
        EnvelopeProcessingError::BlockHashMismatch {
            committed_bid: committed_bid.block_hash,
            envelope: payload.block_hash,
        }
    );

    // Verify consistency of the parent hash with respect to the previous execution payload
    let latest_block_hash = *state.latest_block_hash()?;
    envelope_verify!(
        payload.parent_hash == latest_block_hash,
        EnvelopeProcessingError::ParentHashMismatch {
            state: latest_block_hash,
            envelope: payload.parent_hash,
        }
    );

    // Verify timestamp
    let state_timestamp = compute_timestamp_at_slot(state, state.slot(), spec)?;
    envelope_verify!(
        payload.timestamp == state_timestamp,
        EnvelopeProcessingError::TimestampMismatch {
            state: state_timestamp,
            envelope: payload.timestamp,
        }
    );

    // Note: the newPayload EL call is performed in the beacon chain layer
    // (BeaconChain::process_payload_envelope) before this function is invoked.

    // Process execution requests (moved from block processing to envelope processing in Gloas)
    process_deposit_requests(state, &execution_requests.deposits, spec)?;
    process_withdrawal_requests(state, &execution_requests.withdrawals, spec)?;
    process_consolidation_requests(state, &execution_requests.consolidations, spec)?;

    // Process builder payment: move from pending to withdrawal queue
    let payment_index = E::slots_per_epoch()
        .safe_add(state.slot().as_u64().safe_rem(E::slots_per_epoch())?)?
        as usize;
    let payment_mut = state
        .builder_pending_payments_mut()?
        .get_mut(payment_index)
        .ok_or(EnvelopeProcessingError::BuilderPaymentIndexOutOfBounds(
            payment_index,
        ))?;

    // Copy the payment withdrawal before blanking it out
    let payment_withdrawal = payment_mut.withdrawal;
    *payment_mut = BuilderPendingPayment::default();

    let amount = payment_withdrawal.amount;
    if amount > 0 {
        state
            .builder_pending_withdrawals_mut()?
            .push(payment_withdrawal)
            .map_err(|e| EnvelopeProcessingError::BeaconStateError(e.into()))?;
    }

    // Cache the execution payload hash and set availability
    let availability_index = state
        .slot()
        .as_usize()
        .safe_rem(E::slots_per_historical_root())?;
    state
        .execution_payload_availability_mut()?
        .set(availability_index, true)
        .map_err(EnvelopeProcessingError::BitFieldError)?;
    *state.latest_block_hash_mut()? = payload.block_hash;

    // Verify the state root (envelope contains post-state root)
    if verify_signatures.is_true() {
        let state_root = state.canonical_root()?;
        envelope_verify!(
            envelope.state_root == state_root,
            EnvelopeProcessingError::InvalidStateRoot {
                state: state_root,
                envelope: envelope.state_root,
            }
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use ssz_types::VariableList;
    use std::sync::Arc;
    use types::List;
    use types::test_utils::generate_deterministic_keypairs;
    use types::{
        Address, BeaconBlockHeader, BeaconStateGloas, Builder, BuilderPendingWithdrawal,
        BuilderPubkeyCache, CACHED_EPOCHS, Checkpoint, CommitteeCache, ConsolidationRequest,
        DepositRequest, Domain, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExecutionPayloadEnvelope, ExecutionPayloadGloas, ExecutionRequests, ExitCache, FixedVector,
        Fork, MinimalEthSpec, ProgressiveBalancesCache, PubkeyCache, PublicKeyBytes, Signature,
        SignatureBytes, SignedRoot, SlashingsCache, SyncCommittee, Unsigned, Vector,
        WithdrawalRequest,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state identical to the bid processing test helper,
    /// with `n` validators and a single builder at index 0.
    fn make_gloas_state(
        num_validators: usize,
        balance: u64,
        builder_balance: u64,
    ) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8, epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = types::test_utils::generate_deterministic_keypairs(num_validators);
        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: balance,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(balance);
        }

        let builder = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: builder_balance,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };

        let parent_root = Hash256::repeat_byte(0x01);
        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);
        let randao_mix = Hash256::repeat_byte(0x03);

        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let mut randao_mixes = vec![Hash256::zero(); epochs_per_vector];
        let mix_index = epoch.as_usize() % epochs_per_vector;
        randao_mixes[mix_index] = randao_mix;

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root,
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::zero(),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid {
                parent_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x04),
                prev_randao: randao_mix,
                slot,
                ..Default::default()
            },
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
            builders: List::new(vec![builder]).unwrap(),
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
            latest_block_hash: parent_block_hash,
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    /// Build a valid envelope matching the state's committed bid.
    /// The state_root is set to a dummy value; call `fix_envelope_state_root`
    /// to compute and set the real post-processing state root.
    fn make_valid_envelope(state: &BeaconState<E>) -> SignedExecutionPayloadEnvelope<E> {
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let latest_block_hash = *state.latest_block_hash().unwrap();

        // Compute the expected block header root (after fixing state_root)
        // The state's latest_block_header has state_root=0x00..00, so we need to
        // first compute canonical_root which will fill it in. But since the
        // function will do that itself, we just need the final root after filling.
        let mut header = state.latest_block_header().clone();
        header.state_root = state.clone().canonical_root().unwrap();
        let beacon_block_root = header.tree_hash_root();

        // Compute expected timestamp
        let spec = E::default_spec();
        let timestamp = compute_timestamp_at_slot(state, state.slot(), &spec).unwrap();

        let payload = ExecutionPayloadGloas {
            parent_hash: latest_block_hash,
            block_hash: bid.block_hash,
            prev_randao: bid.prev_randao,
            gas_limit: bid.gas_limit,
            timestamp,
            withdrawals: VariableList::default(), // matches empty payload_expected_withdrawals
            ..Default::default()
        };

        SignedExecutionPayloadEnvelope {
            message: ExecutionPayloadEnvelope {
                payload,
                execution_requests: Default::default(),
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: state.slot(),
                state_root: Hash256::zero(), // will be fixed by fix_envelope_state_root
            },
            signature: Signature::empty(),
        }
    }

    /// Run envelope processing on a clone of the state to discover the real
    /// post-processing state root, then set it on the envelope.
    fn fix_envelope_state_root(
        state: &BeaconState<E>,
        envelope: &mut SignedExecutionPayloadEnvelope<E>,
        spec: &ChainSpec,
    ) {
        let mut state_clone = state.clone();
        process_execution_payload_envelope(
            &mut state_clone,
            None,
            envelope,
            VerifySignatures::False,
            spec,
        )
        .expect("fix_envelope_state_root: envelope processing should succeed");
        envelope.message.state_root = state_clone
            .canonical_root()
            .expect("fix_envelope_state_root: canonical_root should succeed");
    }

    // ── Happy path ─────────────────────────────────────────────

    #[test]
    fn valid_envelope_succeeds() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "valid envelope should succeed: {:?}",
            result.unwrap_err()
        );
    }

    // ── Beacon block consistency checks ────────────────────────

    #[test]
    fn wrong_beacon_block_root_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::LatestBlockHeaderMismatch { .. })
            ),
            "wrong beacon_block_root should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_slot_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.slot = Slot::new(999);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::SlotMismatch { .. })),
            "wrong slot should fail: {:?}",
            result,
        );
    }

    // ── Committed bid consistency checks ───────────────────────

    #[test]
    fn wrong_builder_index_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.builder_index = 42;

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::BuilderIndexMismatch { .. })
            ),
            "wrong builder_index should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_prev_randao_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.payload.prev_randao = Hash256::repeat_byte(0xDD);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::PrevRandaoMismatch { .. })
            ),
            "wrong prev_randao should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_gas_limit_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.payload.gas_limit = 999_999;

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::GasLimitMismatch { .. })
            ),
            "wrong gas_limit should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_block_hash_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.payload.block_hash = ExecutionBlockHash::repeat_byte(0xEE);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::BlockHashMismatch { .. })
            ),
            "wrong block_hash should fail: {:?}",
            result,
        );
    }

    // ── Execution payload consistency ──────────────────────────

    #[test]
    fn wrong_parent_hash_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.payload.parent_hash = ExecutionBlockHash::repeat_byte(0xCC);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::ParentHashMismatch { .. })
            ),
            "wrong parent_hash should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_timestamp_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        envelope.message.payload.timestamp = 12345;

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::TimestampMismatch { .. })
            ),
            "wrong timestamp should fail: {:?}",
            result,
        );
    }

    // ── Withdrawals ────────────────────────────────────────────

    #[test]
    fn wrong_withdrawals_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        // State has empty payload_expected_withdrawals, so adding a withdrawal should mismatch
        let fake_withdrawal = types::Withdrawal {
            index: 0,
            validator_index: 0,
            address: Address::repeat_byte(0x11),
            amount: 1_000_000,
        };
        envelope
            .message
            .payload
            .withdrawals
            .push(fake_withdrawal)
            .unwrap();

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::WithdrawalsRootMismatch { .. })
            ),
            "wrong withdrawals should fail: {:?}",
            result,
        );
    }

    // ── State root ─────────────────────────────────────────────

    #[test]
    fn wrong_state_root_rejected() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);
        let builder_kp = &keypairs[8];

        let mut envelope = make_valid_envelope(&state);
        // Set wrong state_root, then sign with the wrong root in the message
        envelope.message.state_root = Hash256::repeat_byte(0x99);
        sign_envelope(&state, &mut envelope, &builder_kp.sk, &spec);

        // State root is only verified when verify_signatures is true (per spec).
        // Signature is valid for this message (including the wrong state_root),
        // so the check should reach and fail on state root mismatch.
        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(
                result,
                Err(EnvelopeProcessingError::InvalidStateRoot { .. })
            ),
            "wrong state_root should fail: {:?}",
            result,
        );
    }

    #[test]
    fn wrong_state_root_skipped_when_verify_false() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        // Set wrong state_root — but verify=false so it should be ignored
        envelope.message.state_root = Hash256::repeat_byte(0x99);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "state_root check should be skipped when verify=false: {:?}",
            result.unwrap_err(),
        );
    }

    // ── State mutations ────────────────────────────────────────

    #[test]
    fn envelope_updates_latest_block_hash() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let new_block_hash = envelope.message.payload.block_hash;
        assert_ne!(
            *state.latest_block_hash().unwrap(),
            new_block_hash,
            "sanity: block hash should differ before processing"
        );

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        assert_eq!(
            *state.latest_block_hash().unwrap(),
            new_block_hash,
            "latest_block_hash should be updated to payload's block_hash"
        );
    }

    #[test]
    fn envelope_sets_availability_bit() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Clear the availability bit for this slot
        let slot = state.slot();
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let avail_index = slot.as_usize() % slots_per_hist;
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(avail_index, false)
            .unwrap();
        assert!(
            !state
                .execution_payload_availability()
                .unwrap()
                .get(avail_index)
                .unwrap(),
            "sanity: availability bit should be cleared"
        );

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        assert!(
            state
                .execution_payload_availability()
                .unwrap()
                .get(avail_index)
                .unwrap(),
            "availability bit should be set after envelope processing"
        );
    }

    #[test]
    fn envelope_moves_builder_payment_to_withdrawals() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Set up a pending payment for this slot
        let slot = state.slot();
        let slots_per_epoch = E::slots_per_epoch();
        let payment_index = (slots_per_epoch + slot.as_u64() % slots_per_epoch) as usize;

        let payment = BuilderPendingPayment {
            weight: 100,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xCC),
                amount: 5_000_000_000,
                builder_index: 0,
            },
        };
        *state
            .builder_pending_payments_mut()
            .unwrap()
            .get_mut(payment_index)
            .unwrap() = payment;

        assert!(
            state.builder_pending_withdrawals().unwrap().is_empty(),
            "sanity: no pending withdrawals before processing"
        );

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Payment slot should be cleared
        let cleared_payment = *state
            .builder_pending_payments()
            .unwrap()
            .get(payment_index)
            .unwrap();
        assert_eq!(
            cleared_payment,
            BuilderPendingPayment::default(),
            "pending payment should be blanked after processing"
        );

        // Withdrawal should be queued
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert_eq!(withdrawals.len(), 1, "should have one pending withdrawal");
        assert_eq!(withdrawals.get(0).unwrap().amount, 5_000_000_000);
        assert_eq!(
            withdrawals.get(0).unwrap().fee_recipient,
            Address::repeat_byte(0xCC)
        );
    }

    #[test]
    fn envelope_zero_payment_not_queued() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Default pending payment has amount=0, so no withdrawal should be queued
        assert!(
            state.builder_pending_withdrawals().unwrap().is_empty(),
            "sanity: no pending withdrawals"
        );

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        assert!(
            state.builder_pending_withdrawals().unwrap().is_empty(),
            "zero-amount payment should not queue a withdrawal"
        );
    }

    #[test]
    fn envelope_fills_block_header_state_root() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Verify header state_root starts as zero (unfilled)
        assert_eq!(
            state.latest_block_header().state_root,
            Hash256::default(),
            "sanity: header state_root should be zero before processing"
        );

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        assert_ne!(
            state.latest_block_header().state_root,
            Hash256::default(),
            "header state_root should be filled after envelope processing"
        );
    }

    #[test]
    fn parent_state_root_override() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Provide an explicit parent_state_root
        let explicit_root = Hash256::repeat_byte(0x77);
        let mut envelope = make_valid_envelope_with_parent_state_root(&state, Some(explicit_root));
        fix_envelope_state_root_with_parent(&state, &mut envelope, Some(explicit_root), &spec);

        process_execution_payload_envelope(
            &mut state,
            Some(explicit_root),
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        assert_eq!(
            state.latest_block_header().state_root,
            explicit_root,
            "header state_root should use provided parent_state_root"
        );
    }

    /// Build a valid envelope; used when we need to pass parent_state_root
    fn make_valid_envelope_with_parent_state_root(
        state: &BeaconState<E>,
        parent_state_root: Option<Hash256>,
    ) -> SignedExecutionPayloadEnvelope<E> {
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let latest_block_hash = *state.latest_block_hash().unwrap();

        // Compute the beacon_block_root accounting for parent_state_root
        let mut header = state.latest_block_header().clone();
        if header.state_root == Hash256::default() {
            header.state_root =
                parent_state_root.unwrap_or_else(|| state.clone().canonical_root().unwrap());
        }
        let beacon_block_root = header.tree_hash_root();

        let spec = E::default_spec();
        let timestamp = compute_timestamp_at_slot(state, state.slot(), &spec).unwrap();

        let payload = ExecutionPayloadGloas {
            parent_hash: latest_block_hash,
            block_hash: bid.block_hash,
            prev_randao: bid.prev_randao,
            gas_limit: bid.gas_limit,
            timestamp,
            withdrawals: VariableList::default(),
            ..Default::default()
        };

        SignedExecutionPayloadEnvelope {
            message: ExecutionPayloadEnvelope {
                payload,
                execution_requests: Default::default(),
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: state.slot(),
                state_root: Hash256::zero(),
            },
            signature: Signature::empty(),
        }
    }

    fn fix_envelope_state_root_with_parent(
        state: &BeaconState<E>,
        envelope: &mut SignedExecutionPayloadEnvelope<E>,
        parent_state_root: Option<Hash256>,
        spec: &ChainSpec,
    ) {
        let mut state_clone = state.clone();
        process_execution_payload_envelope(
            &mut state_clone,
            parent_state_root,
            envelope,
            VerifySignatures::False,
            spec,
        )
        .expect("fix_envelope_state_root_with_parent: envelope processing should succeed");
        envelope.message.state_root = state_clone
            .canonical_root()
            .expect("fix_envelope_state_root_with_parent: canonical_root should succeed");
    }

    // ── Helpers for signature verification tests ──────────────

    /// Build a Gloas state with real keypairs for signature verification.
    /// Builder at index 0 uses keypairs[num_validators] (the extra keypair).
    /// Returns (state, spec, keypairs) where keypairs has num_validators + 1 entries.
    fn make_gloas_state_with_keys(
        num_validators: usize,
        builder_balance: u64,
    ) -> (BeaconState<E>, ChainSpec, Vec<types::Keypair>) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8, epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        // Generate one extra keypair for the builder
        let keypairs = generate_deterministic_keypairs(num_validators + 1);
        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for kp in &keypairs[..num_validators] {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: 32_000_000_000,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(32_000_000_000);
        }

        // Builder uses the extra keypair
        let builder_kp = &keypairs[num_validators];
        let builder = Builder {
            pubkey: builder_kp.pk.compress(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: builder_balance,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };

        let parent_root = Hash256::repeat_byte(0x01);
        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);
        let randao_mix = Hash256::repeat_byte(0x03);

        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let mut randao_mixes = vec![Hash256::zero(); epochs_per_vector];
        let mix_index = epoch.as_usize() % epochs_per_vector;
        randao_mixes[mix_index] = randao_mix;

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root,
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::zero(),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid {
                parent_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x04),
                prev_randao: randao_mix,
                slot,
                builder_index: 0,
                ..Default::default()
            },
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
            builders: List::new(vec![builder]).unwrap(),
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
            latest_block_hash: parent_block_hash,
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec, keypairs)
    }

    /// Sign an envelope with the given secret key using BeaconBuilder domain.
    fn sign_envelope(
        state: &BeaconState<E>,
        envelope: &mut SignedExecutionPayloadEnvelope<E>,
        sk: &bls::SecretKey,
        spec: &ChainSpec,
    ) {
        let epoch = envelope.message.slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = envelope.message.signing_root(domain);
        envelope.signature = sk.sign(signing_root);
    }

    // ── Signature verification tests ─────────────────────────

    #[test]
    fn valid_builder_signature_accepted() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);
        let builder_kp = &keypairs[8]; // the extra keypair used for builder

        let mut envelope = make_valid_envelope(&state);
        sign_envelope(&state, &mut envelope, &builder_kp.sk, &spec);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        // Re-sign after state_root fix (state_root change alters the state, but not
        // the envelope message's signing root since state_root is part of the message
        // which is already set before signing)
        sign_envelope(&state, &mut envelope, &builder_kp.sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            result.is_ok(),
            "valid builder signature should be accepted: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn invalid_builder_signature_rejected() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);
        // Sign with validator key 0 instead of builder key
        let wrong_kp = &keypairs[0];

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        sign_envelope(&state, &mut envelope, &wrong_kp.sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::BadSignature)),
            "wrong builder signature should be rejected: {:?}",
            result,
        );
    }

    #[test]
    fn self_build_envelope_with_proposer_signature_accepted() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);

        // Set the bid to self-build (builder_index = BUILDER_INDEX_SELF_BUILD)
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .builder_index = BUILDER_INDEX_SELF_BUILD;

        let mut envelope = make_valid_envelope(&state);
        assert_eq!(
            envelope.message.builder_index, BUILDER_INDEX_SELF_BUILD,
            "sanity: envelope should have self-build builder_index"
        );

        // Sign with the proposer's key (proposer_index = 0)
        let proposer_kp = &keypairs[0];
        sign_envelope(&state, &mut envelope, &proposer_kp.sk, &spec);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        // Re-sign after state_root fix
        sign_envelope(&state, &mut envelope, &proposer_kp.sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            result.is_ok(),
            "self-build envelope with proposer signature should be accepted: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn self_build_envelope_with_wrong_signature_rejected() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);

        // Set the bid to self-build
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .builder_index = BUILDER_INDEX_SELF_BUILD;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        // Sign with a non-proposer key (validator 1 instead of proposer at index 0)
        let wrong_kp = &keypairs[1];
        sign_envelope(&state, &mut envelope, &wrong_kp.sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::BadSignature)),
            "self-build envelope with wrong signature should be rejected: {:?}",
            result,
        );
    }

    #[test]
    fn self_build_envelope_with_builder_key_rejected() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);

        // Set the bid to self-build
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .builder_index = BUILDER_INDEX_SELF_BUILD;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        // Sign with the builder's key (not the proposer's key)
        let builder_kp = &keypairs[8];
        sign_envelope(&state, &mut envelope, &builder_kp.sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::BadSignature)),
            "self-build envelope signed by builder (not proposer) should be rejected: {:?}",
            result,
        );
    }

    #[test]
    fn empty_signature_rejected_with_verify() {
        let (mut state, spec, _keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        // envelope.signature is already Signature::empty() from make_valid_envelope

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::BadSignature)),
            "empty signature should be rejected when verifying: {:?}",
            result,
        );
    }

    // ── Edge case: header state_root already filled ───────────

    #[test]
    fn nonzero_header_state_root_preserved() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Pre-fill the header state_root with a non-zero value
        let preset_root = Hash256::repeat_byte(0x55);
        state.latest_block_header_mut().state_root = preset_root;

        // Build envelope using the already-filled header (no state_root override needed)
        let mut envelope = make_valid_envelope_with_parent_state_root(&state, None);
        fix_envelope_state_root_with_parent(&state, &mut envelope, None, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // The header state_root should remain as the preset value, not overwritten
        assert_eq!(
            state.latest_block_header().state_root,
            preset_root,
            "non-zero header state_root should be preserved (not overwritten by envelope processing)"
        );
    }

    // ── Edge case: payment with amount>0 queued regardless of weight ──

    #[test]
    fn nonzero_payment_queued_regardless_of_weight() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Set up a pending payment with amount > 0 but weight = 0
        // This is the builder payment from bid processing — the weight field is for
        // PTC quorum tracking in epoch processing, but envelope processing moves
        // the payment unconditionally (only skipping if amount == 0)
        let slot = state.slot();
        let slots_per_epoch = E::slots_per_epoch();
        let payment_index = (slots_per_epoch + slot.as_u64() % slots_per_epoch) as usize;

        let payment = BuilderPendingPayment {
            weight: 0, // zero weight — but amount is non-zero
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 3_000_000_000,
                builder_index: 0,
            },
        };
        *state
            .builder_pending_payments_mut()
            .unwrap()
            .get_mut(payment_index)
            .unwrap() = payment;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Payment should still be queued — envelope processing checks amount > 0, not weight
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert_eq!(
            withdrawals.len(),
            1,
            "non-zero amount payment should be queued even with weight=0"
        );
        assert_eq!(withdrawals.get(0).unwrap().amount, 3_000_000_000);
    }

    // ── Edge case: payment appends to existing withdrawals ────

    #[test]
    fn payment_appends_to_existing_pending_withdrawals() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Pre-populate pending withdrawals with two existing entries
        let existing1 = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xA1),
            amount: 1_000_000_000,
            builder_index: 0,
        };
        let existing2 = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xA2),
            amount: 2_000_000_000,
            builder_index: 0,
        };
        state
            .builder_pending_withdrawals_mut()
            .unwrap()
            .push(existing1)
            .unwrap();
        state
            .builder_pending_withdrawals_mut()
            .unwrap()
            .push(existing2)
            .unwrap();

        // Set up a new payment for this slot
        let slot = state.slot();
        let slots_per_epoch = E::slots_per_epoch();
        let payment_index = (slots_per_epoch + slot.as_u64() % slots_per_epoch) as usize;
        let payment = BuilderPendingPayment {
            weight: 100,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xA3),
                amount: 7_000_000_000,
                builder_index: 0,
            },
        };
        *state
            .builder_pending_payments_mut()
            .unwrap()
            .get_mut(payment_index)
            .unwrap() = payment;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Should have 3 withdrawals: 2 existing + 1 new
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert_eq!(
            withdrawals.len(),
            3,
            "new payment should be appended to existing pending withdrawals"
        );
        assert_eq!(
            withdrawals.get(0).unwrap().fee_recipient,
            Address::repeat_byte(0xA1),
            "first existing withdrawal preserved"
        );
        assert_eq!(
            withdrawals.get(1).unwrap().fee_recipient,
            Address::repeat_byte(0xA2),
            "second existing withdrawal preserved"
        );
        assert_eq!(
            withdrawals.get(2).unwrap().fee_recipient,
            Address::repeat_byte(0xA3),
            "new payment appended at end"
        );
        assert_eq!(withdrawals.get(2).unwrap().amount, 7_000_000_000);
    }

    // ── Edge case: availability bit at slot 0 ──────────────────

    #[test]
    fn availability_bit_set_at_slot_zero_index() {
        // Create state at slot 0 (epoch 0) to test availability index = 0
        let spec = E::default_spec();
        let slot = Slot::new(0);
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = types::test_utils::generate_deterministic_keypairs(8);
        let mut validators = Vec::with_capacity(8);
        let mut balances = Vec::with_capacity(8);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: 32_000_000_000,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(32_000_000_000);
        }

        let builder = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: 64_000_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };

        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);
        let randao_mix = Hash256::repeat_byte(0x03);
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let mut randao_mixes = vec![Hash256::zero(); epochs_per_vector];
        randao_mixes[epoch.as_usize() % epochs_per_vector] = randao_mix;

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Start with availability bit at index 0 CLEARED
        let mut avail_bytes = vec![0xFFu8; slots_per_hist / 8];
        avail_bytes[0] = 0xFE; // clear bit 0

        let mut state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot,
                proposer_index: 0,
                parent_root: Hash256::repeat_byte(0x01),
                state_root: Hash256::repeat_byte(0x77), // non-zero to avoid filling
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(0),
                root: Hash256::zero(),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid {
                parent_block_hash,
                parent_block_root: Hash256::repeat_byte(0x01),
                block_hash: ExecutionBlockHash::repeat_byte(0x04),
                prev_randao: randao_mix,
                slot,
                ..Default::default()
            },
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
            builders: List::new(vec![builder]).unwrap(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::from_bytes(avail_bytes.into()).unwrap(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: parent_block_hash,
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        // Verify bit 0 is initially cleared
        assert!(
            !state
                .execution_payload_availability()
                .unwrap()
                .get(0)
                .unwrap(),
            "sanity: availability bit 0 should be cleared before processing"
        );

        // Use the parent_state_root variant since header already has non-zero state_root
        let mut envelope = make_valid_envelope_with_parent_state_root(&state, None);
        fix_envelope_state_root_with_parent(&state, &mut envelope, None, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Bit 0 should now be set (slot 0 % 64 = 0)
        assert!(
            state
                .execution_payload_availability()
                .unwrap()
                .get(0)
                .unwrap(),
            "availability bit at index 0 should be set after envelope at slot 0"
        );
    }

    // ── Edge case: builder index out of bounds in signature path ──

    #[test]
    fn builder_index_out_of_bounds_rejected_with_verify() {
        let (mut state, spec, keypairs) = make_gloas_state_with_keys(8, 64_000_000_000);

        // Set bid's builder_index to one beyond the builder registry length
        let builders_len = state.builders().unwrap().len() as u64;
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .builder_index = builders_len; // out of bounds (registry has 1 builder at index 0)

        let mut envelope = make_valid_envelope(&state);
        // Sign with any key — doesn't matter since pubkey lookup should fail
        sign_envelope(&state, &mut envelope, &keypairs[0].sk, &spec);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        sign_envelope(&state, &mut envelope, &keypairs[0].sk, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(result, Err(EnvelopeProcessingError::BadSignature)),
            "builder_index beyond registry length should be rejected: {:?}",
            result,
        );
    }

    // ── Error value verification tests ─────────────────────────

    #[test]
    fn slot_mismatch_reports_correct_values() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        let bad_slot = Slot::new(999);
        envelope.message.slot = bad_slot;

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        match result {
            Err(EnvelopeProcessingError::SlotMismatch {
                envelope_slot,
                parent_state_slot,
            }) => {
                assert_eq!(
                    envelope_slot, bad_slot,
                    "error should report the envelope's slot"
                );
                assert_eq!(
                    parent_state_slot,
                    state.slot(),
                    "error should report the state's slot"
                );
            }
            other => panic!(
                "expected SlotMismatch with correct values, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn latest_block_header_mismatch_reports_correct_values() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        let bad_root = Hash256::repeat_byte(0xDE);
        envelope.message.beacon_block_root = bad_root;

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        match result {
            Err(EnvelopeProcessingError::LatestBlockHeaderMismatch {
                envelope_root,
                block_header_root,
            }) => {
                assert_eq!(
                    envelope_root, bad_root,
                    "error should report the envelope's beacon_block_root"
                );
                // The block_header_root should be the tree_hash of the latest block header
                // (after state_root filling since it was zero)
                assert_ne!(
                    block_header_root,
                    Hash256::zero(),
                    "block_header_root should be non-zero (header was filled)"
                );
                assert_ne!(
                    block_header_root, bad_root,
                    "block_header_root should differ from the bad root"
                );
            }
            other => panic!(
                "expected LatestBlockHeaderMismatch with correct values, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn timestamp_mismatch_reports_spec_computed_value() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut envelope = make_valid_envelope(&state);
        let bad_timestamp = 12345u64;
        envelope.message.payload.timestamp = bad_timestamp;

        // Compute the expected spec-derived timestamp
        let expected_timestamp = compute_timestamp_at_slot(&state, state.slot(), &spec).unwrap();

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        match result {
            Err(EnvelopeProcessingError::TimestampMismatch {
                state: state_ts,
                envelope: envelope_ts,
            }) => {
                assert_eq!(
                    state_ts, expected_timestamp,
                    "error should report the spec-computed timestamp"
                );
                assert_eq!(
                    envelope_ts, bad_timestamp,
                    "error should report the envelope's timestamp"
                );
            }
            other => panic!(
                "expected TimestampMismatch with correct values, got: {:?}",
                other
            ),
        }
    }

    // ── Payment index boundary tests ────────────────────────────

    #[test]
    fn envelope_at_last_slot_of_epoch_uses_correct_payment_index() {
        // Test at the last slot of epoch 1 (slot 15 for minimal with 8 slots/epoch)
        // payment_index = slots_per_epoch + (slot % slots_per_epoch) = 8 + 7 = 15
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Move state to the last slot of epoch 1 (slot 15)
        let last_slot = Slot::new(E::slots_per_epoch().saturating_mul(2).saturating_sub(1));
        state.as_gloas_mut().unwrap().slot = last_slot;
        state.as_gloas_mut().unwrap().latest_block_header.slot = last_slot.saturating_sub(1u64);
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .slot = last_slot;

        // Set up a pending payment at the expected index (15)
        let slots_per_epoch = E::slots_per_epoch();
        let expected_index = (slots_per_epoch + last_slot.as_u64() % slots_per_epoch) as usize;
        assert_eq!(
            expected_index, 15,
            "sanity: payment index for slot 15 should be 15"
        );

        let payment = BuilderPendingPayment {
            weight: 50,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xEE),
                amount: 9_000_000_000,
                builder_index: 0,
            },
        };
        *state
            .builder_pending_payments_mut()
            .unwrap()
            .get_mut(expected_index)
            .unwrap() = payment;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Payment at index 15 should be cleared
        let cleared = *state
            .builder_pending_payments()
            .unwrap()
            .get(expected_index)
            .unwrap();
        assert_eq!(
            cleared,
            BuilderPendingPayment::default(),
            "payment at last-slot-of-epoch index should be cleared"
        );

        // Withdrawal should be queued
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert_eq!(withdrawals.len(), 1);
        assert_eq!(withdrawals.get(0).unwrap().amount, 9_000_000_000);
    }

    #[test]
    fn all_state_mutations_applied_together() {
        // Verify that a single valid envelope processing applies all mutations:
        // 1. latest_block_hash updated
        // 2. availability bit set
        // 3. pending payment cleared and moved to withdrawals
        // 4. latest_block_header state_root filled
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Clear availability bit for this slot
        let slot = state.slot();
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let avail_index = slot.as_usize() % slots_per_hist;
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(avail_index, false)
            .unwrap();

        // Set up a pending payment
        let slots_per_epoch = E::slots_per_epoch();
        let payment_index = (slots_per_epoch + slot.as_u64() % slots_per_epoch) as usize;
        let payment = BuilderPendingPayment {
            weight: 42,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xF0),
                amount: 2_500_000_000,
                builder_index: 0,
            },
        };
        *state
            .builder_pending_payments_mut()
            .unwrap()
            .get_mut(payment_index)
            .unwrap() = payment;

        // Capture pre-processing state
        let old_block_hash = *state.latest_block_hash().unwrap();
        assert_eq!(
            state.latest_block_header().state_root,
            Hash256::default(),
            "sanity: header state_root should be zero"
        );
        assert!(
            !state
                .execution_payload_availability()
                .unwrap()
                .get(avail_index)
                .unwrap(),
            "sanity: availability bit should be cleared"
        );
        assert!(
            state.builder_pending_withdrawals().unwrap().is_empty(),
            "sanity: no pending withdrawals"
        );

        let mut envelope = make_valid_envelope(&state);
        let expected_block_hash = envelope.message.payload.block_hash;
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // 1. latest_block_hash updated
        assert_eq!(
            *state.latest_block_hash().unwrap(),
            expected_block_hash,
            "latest_block_hash should be updated"
        );
        assert_ne!(
            *state.latest_block_hash().unwrap(),
            old_block_hash,
            "latest_block_hash should differ from old value"
        );

        // 2. availability bit set
        assert!(
            state
                .execution_payload_availability()
                .unwrap()
                .get(avail_index)
                .unwrap(),
            "availability bit should be set"
        );

        // 3. pending payment cleared and withdrawal queued
        let cleared = *state
            .builder_pending_payments()
            .unwrap()
            .get(payment_index)
            .unwrap();
        assert_eq!(cleared, BuilderPendingPayment::default());
        let withdrawals = state.builder_pending_withdrawals().unwrap();
        assert_eq!(withdrawals.len(), 1);
        assert_eq!(withdrawals.get(0).unwrap().amount, 2_500_000_000);
        assert_eq!(
            withdrawals.get(0).unwrap().fee_recipient,
            Address::repeat_byte(0xF0)
        );

        // 4. header state_root filled
        assert_ne!(
            state.latest_block_header().state_root,
            Hash256::default(),
            "header state_root should be filled"
        );
    }

    // ── Execution requests tests ──────────────────────────────

    /// Build a valid envelope with custom execution_requests.
    fn make_valid_envelope_with_requests(
        state: &BeaconState<E>,
        execution_requests: ExecutionRequests<E>,
    ) -> SignedExecutionPayloadEnvelope<E> {
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let latest_block_hash = *state.latest_block_hash().unwrap();

        let mut header = state.latest_block_header().clone();
        header.state_root = state.clone().canonical_root().unwrap();
        let beacon_block_root = header.tree_hash_root();

        let spec = E::default_spec();
        let timestamp = compute_timestamp_at_slot(state, state.slot(), &spec).unwrap();

        let payload = ExecutionPayloadGloas {
            parent_hash: latest_block_hash,
            block_hash: bid.block_hash,
            prev_randao: bid.prev_randao,
            gas_limit: bid.gas_limit,
            timestamp,
            withdrawals: VariableList::default(),
            ..Default::default()
        };

        SignedExecutionPayloadEnvelope {
            message: ExecutionPayloadEnvelope {
                payload,
                execution_requests,
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: state.slot(),
                state_root: Hash256::zero(),
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn envelope_with_deposit_request_adds_to_pending_deposits() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Build the pubkey cache so deposit processing can look up validators
        state.update_pubkey_cache().unwrap();

        // Create a deposit request with a new (unknown) pubkey and 0x01 withdrawal creds
        // This should route to pending_deposits (not builder) since pubkey is unknown
        // and credentials don't have builder prefix (0x03).
        let deposit_pubkey =
            PublicKeyBytes::deserialize(&[0xAB; 48]).unwrap_or_else(|_| PublicKeyBytes::empty());
        let mut withdrawal_creds = [0u8; 32];
        withdrawal_creds[0] = 0x01; // validator withdrawal prefix
        let deposit_request = DepositRequest {
            pubkey: deposit_pubkey,
            withdrawal_credentials: Hash256::from_slice(&withdrawal_creds),
            amount: 32_000_000_000,
            signature: SignatureBytes::empty(),
            index: 0,
        };

        let mut requests = ExecutionRequests::default();
        requests.deposits.push(deposit_request.clone()).unwrap();

        assert!(
            state.pending_deposits().unwrap().is_empty(),
            "sanity: no pending deposits before processing"
        );

        let mut envelope = make_valid_envelope_with_requests(&state, requests);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // The deposit request should have been routed to pending_deposits
        let pending = state.pending_deposits().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "deposit request should be added to pending_deposits"
        );
        assert_eq!(pending.get(0).unwrap().pubkey, deposit_pubkey);
        assert_eq!(pending.get(0).unwrap().amount, 32_000_000_000);
    }

    #[test]
    fn envelope_with_deposit_request_tops_up_builder() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Build caches
        state.update_pubkey_cache().unwrap();
        state.update_builder_pubkey_cache().unwrap();

        // Get the builder's pubkey (builder at index 0)
        let builder_pubkey = state.builders().unwrap().get(0).unwrap().pubkey;
        let initial_balance = state.builders().unwrap().get(0).unwrap().balance;

        // Create a deposit with the builder's pubkey — should route to builder top-up
        let mut withdrawal_creds = [0u8; 32];
        withdrawal_creds[0] = 0x03; // builder prefix
        let deposit_request = DepositRequest {
            pubkey: builder_pubkey,
            withdrawal_credentials: Hash256::from_slice(&withdrawal_creds),
            amount: 5_000_000_000,
            signature: SignatureBytes::empty(),
            index: 0,
        };

        let mut requests = ExecutionRequests::default();
        requests.deposits.push(deposit_request).unwrap();

        let mut envelope = make_valid_envelope_with_requests(&state, requests);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Builder balance should increase by deposit amount
        let new_balance = state.builders().unwrap().get(0).unwrap().balance;
        assert_eq!(
            new_balance,
            initial_balance + 5_000_000_000,
            "builder balance should increase by deposit amount"
        );

        // Pending deposits should remain empty (deposit went to builder)
        assert!(
            state.pending_deposits().unwrap().is_empty(),
            "builder deposit should not go to pending_deposits"
        );
    }

    #[test]
    fn envelope_with_withdrawal_request_unknown_validator_succeeds() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Build pubkey cache for validator lookup
        state.update_pubkey_cache().unwrap();

        // Create a withdrawal request with an unknown validator pubkey
        // This should be a no-op (silently skipped) but the envelope should still succeed
        let withdrawal_request = WithdrawalRequest {
            source_address: Address::repeat_byte(0x11),
            validator_pubkey: PublicKeyBytes::deserialize(&[0xCD; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
            amount: 1_000_000_000,
        };

        let mut requests = ExecutionRequests::default();
        requests.withdrawals.push(withdrawal_request).unwrap();

        let mut envelope = make_valid_envelope_with_requests(&state, requests);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "envelope with withdrawal request for unknown validator should succeed: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn envelope_with_consolidation_request_unknown_source_succeeds() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Build caches (consolidation processing needs total active balance)
        state.update_pubkey_cache().unwrap();
        state.build_total_active_balance_cache(&spec).unwrap();

        // Create a consolidation request with unknown source
        // This should be a no-op but envelope processing should still succeed
        let consolidation_request = ConsolidationRequest {
            source_address: Address::repeat_byte(0x22),
            source_pubkey: PublicKeyBytes::deserialize(&[0xEF; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
            target_pubkey: PublicKeyBytes::deserialize(&[0xFE; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
        };

        let mut requests = ExecutionRequests::default();
        requests.consolidations.push(consolidation_request).unwrap();

        let mut envelope = make_valid_envelope_with_requests(&state, requests);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "envelope with consolidation request for unknown source should succeed: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn envelope_with_all_three_request_types() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Build caches (consolidation processing needs total active balance)
        state.update_pubkey_cache().unwrap();
        state.update_builder_pubkey_cache().unwrap();
        state.build_total_active_balance_cache(&spec).unwrap();

        // Deposit: unknown pubkey → pending_deposits
        let mut withdrawal_creds = [0u8; 32];
        withdrawal_creds[0] = 0x01;
        let deposit_request = DepositRequest {
            pubkey: PublicKeyBytes::deserialize(&[0xAB; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
            withdrawal_credentials: Hash256::from_slice(&withdrawal_creds),
            amount: 32_000_000_000,
            signature: SignatureBytes::empty(),
            index: 0,
        };

        // Withdrawal: unknown validator → no-op
        let withdrawal_request = WithdrawalRequest {
            source_address: Address::repeat_byte(0x33),
            validator_pubkey: PublicKeyBytes::deserialize(&[0xCD; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
            amount: 1_000_000_000,
        };

        // Consolidation: unknown source → no-op
        let consolidation_request = ConsolidationRequest {
            source_address: Address::repeat_byte(0x44),
            source_pubkey: PublicKeyBytes::deserialize(&[0xEF; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
            target_pubkey: PublicKeyBytes::deserialize(&[0xFE; 48])
                .unwrap_or_else(|_| PublicKeyBytes::empty()),
        };

        let mut requests = ExecutionRequests::default();
        requests.deposits.push(deposit_request).unwrap();
        requests.withdrawals.push(withdrawal_request).unwrap();
        requests.consolidations.push(consolidation_request).unwrap();

        let mut envelope = make_valid_envelope_with_requests(&state, requests);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let result = process_execution_payload_envelope(
            &mut state,
            None,
            &envelope,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "envelope with all three request types should succeed: {:?}",
            result.unwrap_err()
        );

        // Only the deposit should have had a visible effect
        assert_eq!(
            state.pending_deposits().unwrap().len(),
            1,
            "deposit request should be added to pending_deposits"
        );
    }
}
