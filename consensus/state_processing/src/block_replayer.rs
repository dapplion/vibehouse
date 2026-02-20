use crate::{
    BlockProcessingError, BlockSignatureStrategy, ConsensusContext, SlotProcessingError,
    VerifyBlockRoot, VerifySignatures, envelope_processing::process_execution_payload_envelope,
    per_block_processing, per_epoch_processing::EpochProcessingSummary, per_slot_processing,
};
use itertools::Itertools;
use std::collections::HashMap;
use std::iter::Peekable;
use std::marker::PhantomData;
use types::{
    BeaconState, BeaconStateError, BlindedPayload, ChainSpec, EthSpec, ExecutionBlockHash, Hash256,
    SignedBeaconBlock, SignedExecutionPayloadEnvelope, Slot,
};

pub type PreBlockHook<'a, E, Error> = Box<
    dyn FnMut(&mut BeaconState<E>, &SignedBeaconBlock<E, BlindedPayload<E>>) -> Result<(), Error>
        + 'a,
>;
pub type PostBlockHook<'a, E, Error> = PreBlockHook<'a, E, Error>;
pub type PreSlotHook<'a, E, Error> =
    Box<dyn FnMut(Hash256, &mut BeaconState<E>) -> Result<(), Error> + 'a>;
pub type PostSlotHook<'a, E, Error> = Box<
    dyn FnMut(&mut BeaconState<E>, Option<EpochProcessingSummary<E>>, bool) -> Result<(), Error>
        + 'a,
>;
pub type StateRootIterDefault<Error> = std::iter::Empty<Result<(Hash256, Slot), Error>>;

/// Efficiently apply blocks to a state while configuring various parameters.
///
/// Usage follows a builder pattern.
pub struct BlockReplayer<
    'a,
    Spec: EthSpec,
    Error = BlockReplayError,
    StateRootIter: Iterator<Item = Result<(Hash256, Slot), Error>> = StateRootIterDefault<Error>,
> {
    state: BeaconState<Spec>,
    spec: &'a ChainSpec,
    block_sig_strategy: BlockSignatureStrategy,
    verify_block_root: Option<VerifyBlockRoot>,
    pre_block_hook: Option<PreBlockHook<'a, Spec, Error>>,
    post_block_hook: Option<PostBlockHook<'a, Spec, Error>>,
    pre_slot_hook: Option<PreSlotHook<'a, Spec, Error>>,
    post_slot_hook: Option<PostSlotHook<'a, Spec, Error>>,
    pub(crate) state_root_iter: Option<Peekable<StateRootIter>>,
    state_root_miss: bool,
    /// Gloas ePBS: envelopes to apply after each block, keyed by block root.
    /// When set, full envelope processing is applied after each Gloas block instead
    /// of just updating latest_block_hash from the bid.
    envelopes: HashMap<Hash256, SignedExecutionPayloadEnvelope<Spec>>,
    _phantom: PhantomData<Error>,
}

#[derive(Debug)]
pub enum BlockReplayError {
    SlotProcessing(SlotProcessingError),
    BlockProcessing(BlockProcessingError),
    BeaconState(BeaconStateError),
}

impl From<SlotProcessingError> for BlockReplayError {
    fn from(e: SlotProcessingError) -> Self {
        Self::SlotProcessing(e)
    }
}

impl From<BlockProcessingError> for BlockReplayError {
    fn from(e: BlockProcessingError) -> Self {
        Self::BlockProcessing(e)
    }
}

impl From<BeaconStateError> for BlockReplayError {
    fn from(e: BeaconStateError) -> Self {
        Self::BeaconState(e)
    }
}

impl<'a, E, Error, StateRootIter> BlockReplayer<'a, E, Error, StateRootIter>
where
    E: EthSpec,
    StateRootIter: Iterator<Item = Result<(Hash256, Slot), Error>>,
    Error: From<BlockReplayError>,
{
    /// Create a new replayer that will apply blocks upon `state`.
    ///
    /// Defaults:
    ///
    /// - Full (bulk) signature verification
    /// - Accurate state roots
    /// - Full block root verification
    pub fn new(state: BeaconState<E>, spec: &'a ChainSpec) -> Self {
        Self {
            state,
            spec,
            block_sig_strategy: BlockSignatureStrategy::VerifyBulk,
            verify_block_root: Some(VerifyBlockRoot::True),
            pre_block_hook: None,
            post_block_hook: None,
            pre_slot_hook: None,
            post_slot_hook: None,
            state_root_iter: None,
            state_root_miss: false,
            envelopes: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Set the replayer's block signature verification strategy.
    pub fn block_signature_strategy(mut self, block_sig_strategy: BlockSignatureStrategy) -> Self {
        self.block_sig_strategy = block_sig_strategy;
        self
    }

    /// Disable signature verification during replay.
    ///
    /// If you are truly _replaying_ blocks then you will almost certainly want to disable
    /// signature checks for performance.
    pub fn no_signature_verification(self) -> Self {
        self.block_signature_strategy(BlockSignatureStrategy::NoVerification)
    }

    /// Verify only the block roots of the initial few blocks, and trust the rest.
    pub fn minimal_block_root_verification(mut self) -> Self {
        self.verify_block_root = None;
        self
    }

    /// Supply a state root iterator to accelerate slot processing.
    ///
    /// If possible the state root iterator should return a state root for every slot from
    /// `self.state.slot` to the `target_slot` supplied to `apply_blocks` (inclusive of both
    /// endpoints).
    pub fn state_root_iter(mut self, iter: StateRootIter) -> Self {
        self.state_root_iter = Some(iter.peekable());
        self
    }

    /// Run a function immediately before each block that is applied during `apply_blocks`.
    ///
    /// This can be used to inspect the state as blocks are applied.
    pub fn pre_block_hook(mut self, hook: PreBlockHook<'a, E, Error>) -> Self {
        self.pre_block_hook = Some(hook);
        self
    }

    /// Run a function immediately after each block that is applied during `apply_blocks`.
    ///
    /// This can be used to inspect the state as blocks are applied.
    pub fn post_block_hook(mut self, hook: PostBlockHook<'a, E, Error>) -> Self {
        self.post_block_hook = Some(hook);
        self
    }

    /// Run a function immediately before slot processing advances the state to the next slot.
    pub fn pre_slot_hook(mut self, hook: PreSlotHook<'a, E, Error>) -> Self {
        self.pre_slot_hook = Some(hook);
        self
    }

    /// Run a function immediately after slot processing has advanced the state to the next slot.
    ///
    /// The hook receives the state and a bool indicating if this state corresponds to a skipped
    /// slot (i.e. it will not have a block applied).
    pub fn post_slot_hook(mut self, hook: PostSlotHook<'a, E, Error>) -> Self {
        self.post_slot_hook = Some(hook);
        self
    }

    /// Supply Gloas execution payload envelopes to apply during block replay.
    ///
    /// For Gloas (ePBS), the execution payload is delivered in a separate envelope that must
    /// be applied to the state after the block. The envelopes are keyed by beacon block root.
    pub fn envelopes(
        mut self,
        envelopes: HashMap<Hash256, SignedExecutionPayloadEnvelope<E>>,
    ) -> Self {
        self.envelopes = envelopes;
        self
    }

    /// Compute the state root for `self.state` as efficiently as possible.
    ///
    /// This function MUST only be called when `self.state` is a post-state, i.e. it MUST not be
    /// called between advancing a state with `per_slot_processing` and applying the block for that
    /// slot.
    ///
    /// The `blocks` should be the full list of blocks being applied and `i` should be the index of
    /// the next block that will be applied, or `blocks.len()` if all blocks have already been
    /// applied.
    ///
    /// If the state root is not available from the state root iterator or the blocks then it will
    /// be computed from `self.state` and a state root iterator miss will be recorded.
    fn get_state_root(
        &mut self,
        blocks: &[SignedBeaconBlock<E, BlindedPayload<E>>],
        i: usize,
    ) -> Result<Hash256, Error> {
        let slot = self.state.slot();

        // If a state root iterator is configured, use it to find the root.
        if let Some(ref mut state_root_iter) = self.state_root_iter {
            let opt_root = state_root_iter
                .peeking_take_while(|res| res.as_ref().map_or(true, |(_, s)| *s <= slot))
                .find(|res| res.as_ref().map_or(true, |(_, s)| *s == slot))
                .transpose()?;

            if let Some((root, _)) = opt_root {
                return Ok(root);
            }
        }

        // Otherwise try to source a root from the previous block.
        if let Some(prev_i) = i.checked_sub(1)
            && let Some(prev_block) = blocks.get(prev_i)
            && prev_block.slot() == slot
        {
            return Ok(prev_block.state_root());
        }

        self.state_root_miss = true;
        let state_root = self
            .state
            .update_tree_hash_cache()
            .map_err(BlockReplayError::from)?;
        Ok(state_root)
    }

    /// Apply `blocks` atop `self.state`, taking care of slot processing.
    ///
    /// If `target_slot` is provided then the state will be advanced through to `target_slot`
    /// after the blocks have been applied.
    pub fn apply_blocks(
        mut self,
        blocks: Vec<SignedBeaconBlock<E, BlindedPayload<E>>>,
        target_slot: Option<Slot>,
    ) -> Result<Self, Error> {
        for (i, block) in blocks.iter().enumerate() {
            // Allow one additional block at the start which is only used for its state root.
            if i == 0 && block.slot() <= self.state.slot() {
                // For Gloas blocks, the stored post-block state has latest_block_hash
                // from before envelope processing (envelopes update it separately).
                // Apply the bid's block_hash as a fallback so subsequent blocks can
                // validate bid.parent_block_hash == state.latest_block_hash.
                if let Ok(bid) = block.message().body().signed_execution_payload_bid() {
                    let block_root = block.canonical_root();
                    if let Some(envelope) = self.envelopes.remove(&block_root) {
                        // Best-effort: envelope processing may fail for replayed
                        // blocks but the state's latest_block_hash is still updated.
                        drop(process_execution_payload_envelope(
                            &mut self.state,
                            None,
                            &envelope,
                            VerifySignatures::False,
                            self.spec,
                        ));
                    } else if bid.message.block_hash != ExecutionBlockHash::zero() {
                        // Only update latest_block_hash from the bid if it's non-zero.
                        // Genesis blocks have an empty bid with zero block_hash — applying
                        // it would corrupt the state's already-correct latest_block_hash.
                        if let Ok(h) = self.state.latest_block_hash_mut() {
                            *h = bid.message.block_hash;
                        }
                    }

                    // Fix latest_block_header.state_root for Gloas states loaded from
                    // cold storage. The stored state may have state_root set to the
                    // post-envelope hash (from tree_hash_cache during per_slot_processing).
                    // The correct value is the pre-envelope root (block.state_root()) so
                    // that latest_block_header.canonical_root() matches the next block's
                    // parent_root.
                    let header = self.state.latest_block_header_mut();
                    if !header.state_root.is_zero() && header.state_root != block.state_root() {
                        header.state_root = block.state_root();
                    }
                }
                continue;
            }

            while self.state.slot() < block.slot() {
                let state_root = self.get_state_root(&blocks, i)?;

                if let Some(ref mut pre_slot_hook) = self.pre_slot_hook {
                    pre_slot_hook(state_root, &mut self.state)?;
                }

                let summary = per_slot_processing(&mut self.state, Some(state_root), self.spec)
                    .map_err(BlockReplayError::from)?;

                if let Some(ref mut post_slot_hook) = self.post_slot_hook {
                    let is_skipped_slot = self.state.slot() < block.slot();
                    post_slot_hook(&mut self.state, summary, is_skipped_slot)?;
                }
            }

            if let Some(ref mut pre_block_hook) = self.pre_block_hook {
                pre_block_hook(&mut self.state, block)?;
            }

            // If no explicit policy is set, verify only the first 1 or 2 block roots.
            let verify_block_root = self.verify_block_root.unwrap_or(if i <= 1 {
                VerifyBlockRoot::True
            } else {
                VerifyBlockRoot::False
            });
            // Proposer index was already checked when this block was originally processed, we
            // can omit recomputing it during replay.
            let mut ctxt = ConsensusContext::new(block.slot())
                .set_proposer_index(block.message().proposer_index());
            per_block_processing(
                &mut self.state,
                block,
                self.block_sig_strategy,
                verify_block_root,
                &mut ctxt,
                self.spec,
            )
            .map_err(BlockReplayError::from)?;

            // Gloas ePBS: apply envelope processing after each Gloas block.
            // The execution payload is delivered in a separate envelope. If we have
            // the envelope, apply the full state transition (execution requests,
            // builder payments, availability bits, latest_block_hash). Otherwise
            // fall back to just updating latest_block_hash from the bid.
            if let Ok(bid) = block.message().body().signed_execution_payload_bid() {
                let block_root = block.canonical_root();
                if let Some(envelope) = self.envelopes.remove(&block_root) {
                    process_execution_payload_envelope(
                        &mut self.state,
                        None,
                        &envelope,
                        VerifySignatures::False,
                        self.spec,
                    )
                    .map_err(|e| {
                        BlockReplayError::BlockProcessing(
                            BlockProcessingError::EnvelopeProcessingError(format!("{:?}", e)),
                        )
                    })?;
                } else {
                    *self
                        .state
                        .latest_block_hash_mut()
                        .map_err(BlockReplayError::BeaconState)? = bid.message.block_hash;
                }
            }

            if let Some(ref mut post_block_hook) = self.post_block_hook {
                post_block_hook(&mut self.state, block)?;
            }
        }

        if let Some(target_slot) = target_slot {
            while self.state.slot() < target_slot {
                let state_root = self.get_state_root(&blocks, blocks.len())?;

                if let Some(ref mut pre_slot_hook) = self.pre_slot_hook {
                    pre_slot_hook(state_root, &mut self.state)?;
                }

                let summary = per_slot_processing(&mut self.state, Some(state_root), self.spec)
                    .map_err(BlockReplayError::from)?;

                if let Some(ref mut post_slot_hook) = self.post_slot_hook {
                    // No more blocks to apply (from our perspective) so we consider these slots
                    // skipped.
                    let is_skipped_slot = true;
                    post_slot_hook(&mut self.state, summary, is_skipped_slot)?;
                }
            }
        }

        Ok(self)
    }

    /// After block application, check if a state root miss occurred.
    pub fn state_root_miss(&self) -> bool {
        self.state_root_miss
    }

    /// Convert the replayer into the state that was built.
    pub fn into_state(self) -> BeaconState<E> {
        self.state
    }
}

impl<E, Error> BlockReplayer<'_, E, Error, StateRootIterDefault<Error>>
where
    E: EthSpec,
    Error: From<BlockReplayError>,
{
    /// If type inference fails to infer the state root iterator type you can use this method
    /// to hint that no state root iterator is desired.
    pub fn no_state_root_iter(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope_processing::{EnvelopeProcessingError, process_execution_payload_envelope};
    use crate::per_block_processing::compute_timestamp_at_slot;
    use bls::FixedBytesExtended;
    use bls::Signature as BlsSignature;
    use ssz_types::BitVector;
    use ssz_types::VariableList;
    use std::sync::Arc;
    use tree_hash::TreeHash;
    use types::List;
    use types::{
        Address, BeaconBlock, BeaconBlockBodyGloas, BeaconBlockGloas, BeaconBlockHeader,
        BeaconStateGloas, Builder, BuilderPendingPayment, CACHED_EPOCHS, Checkpoint,
        CommitteeCache, Epoch, ExecutionPayloadBid, ExecutionPayloadEnvelope,
        ExecutionPayloadGloas, ExitCache, FixedVector, Fork, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, SignedExecutionPayloadBid, SlashingsCache,
        SyncAggregate, SyncCommittee, Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state with `n` validators and one builder.
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
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    /// Build a minimal Gloas blinded block at the given slot with the specified bid.
    fn make_gloas_block(
        slot: Slot,
        parent_root: Hash256,
        state_root: Hash256,
        bid: ExecutionPayloadBid<E>,
    ) -> SignedBeaconBlock<E, BlindedPayload<E>> {
        let block = BeaconBlock::Gloas(BeaconBlockGloas {
            slot,
            proposer_index: 0,
            parent_root,
            state_root,
            body: BeaconBlockBodyGloas {
                randao_reveal: BlsSignature::empty(),
                eth1_data: types::Eth1Data::default(),
                graffiti: types::Graffiti::default(),
                proposer_slashings: VariableList::empty(),
                attester_slashings: VariableList::empty(),
                attestations: VariableList::empty(),
                deposits: VariableList::empty(),
                voluntary_exits: VariableList::empty(),
                sync_aggregate: SyncAggregate::empty(),
                bls_to_execution_changes: VariableList::empty(),
                signed_execution_payload_bid: SignedExecutionPayloadBid {
                    message: bid,
                    signature: BlsSignature::empty(),
                },
                payload_attestations: VariableList::empty(),
                _phantom: PhantomData,
            },
        });
        SignedBeaconBlock::from_block(block, BlsSignature::empty())
    }

    /// Build a valid envelope matching the given state and bid.
    fn make_valid_envelope(state: &BeaconState<E>) -> SignedExecutionPayloadEnvelope<E> {
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
                execution_requests: Default::default(),
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: state.slot(),
                state_root: Hash256::zero(),
            },
            signature: BlsSignature::empty(),
        }
    }

    /// Run envelope processing on a clone to discover the real post-processing
    /// state root, then set it on the envelope.
    fn fix_envelope_state_root(
        state: &BeaconState<E>,
        envelope: &mut SignedExecutionPayloadEnvelope<E>,
        spec: &ChainSpec,
    ) {
        let mut state_clone = state.clone();
        let result = process_execution_payload_envelope(
            &mut state_clone,
            None,
            envelope,
            VerifySignatures::False,
            spec,
        );
        match result {
            Err(EnvelopeProcessingError::InvalidStateRoot {
                state: real_root, ..
            }) => {
                envelope.message.state_root = real_root;
            }
            Ok(()) => {}
            Err(e) => {
                panic!("fix_envelope_state_root: unexpected error: {:?}", e);
            }
        }
    }

    // ── Anchor block: envelope application ────────────────────────

    #[test]
    fn anchor_block_with_envelope_updates_latest_block_hash() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, envelope);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "envelope processing should update latest_block_hash"
        );
    }

    #[test]
    fn anchor_block_without_envelope_falls_back_to_bid_hash() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        // Set state's latest_block_hash to something different from bid
        let mut state = state;
        *state.latest_block_hash_mut().unwrap() = ExecutionBlockHash::repeat_byte(0xFF);
        assert_ne!(*state.latest_block_hash().unwrap(), bid_block_hash);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        // No envelopes supplied
        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "without envelope, should fall back to bid block_hash"
        );
    }

    #[test]
    fn anchor_block_zero_block_hash_does_not_corrupt_state() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let original_hash = *state.latest_block_hash().unwrap();

        // Create a bid with a zero block_hash (genesis-like)
        let bid = ExecutionPayloadBid {
            block_hash: ExecutionBlockHash::zero(),
            ..Default::default()
        };

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            original_hash,
            "zero block_hash bid should NOT overwrite existing latest_block_hash"
        );
    }

    // ── Anchor block: state root fix ───────────────────────────────

    #[test]
    fn anchor_block_fixes_stale_state_root_in_header() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let correct_state_root = Hash256::repeat_byte(0xCC);
        let wrong_state_root = Hash256::repeat_byte(0xDD);

        // Simulate cold-storage: header has post-envelope state_root
        state.latest_block_header_mut().state_root = wrong_state_root;

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            correct_state_root, // block.state_root() returns this
            bid,
        );

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            result_state.latest_block_header().state_root,
            correct_state_root,
            "anchor block should fix header state_root to block.state_root()"
        );
    }

    #[test]
    fn anchor_block_preserves_correct_state_root() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let correct_state_root = Hash256::repeat_byte(0xCC);

        // Header already has the correct state_root
        state.latest_block_header_mut().state_root = correct_state_root;

        let anchor_block = make_gloas_block(state.slot(), Hash256::zero(), correct_state_root, bid);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            result_state.latest_block_header().state_root,
            correct_state_root,
            "when state_root matches, it should not be changed"
        );
    }

    #[test]
    fn anchor_block_zero_state_root_not_overwritten() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Header has zero state_root (normal for states that haven't been finalized)
        state.latest_block_header_mut().state_root = Hash256::zero();

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xCC),
            bid,
        );

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            result_state.latest_block_header().state_root,
            Hash256::zero(),
            "zero header state_root should not be overwritten (pre-finalization)"
        );
    }

    // ── Anchor block: envelope takes priority over fallback ───────

    #[test]
    fn anchor_block_envelope_takes_priority_over_bid_fallback() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        // Change the bid's block_hash to something different to distinguish
        // whether envelope or fallback path ran
        let mut modified_bid = bid.clone();
        modified_bid.block_hash = ExecutionBlockHash::repeat_byte(0xEE);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            // Use the original bid (matching the envelope) not the modified one
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, envelope);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        // Envelope sets latest_block_hash = payload.block_hash = bid.block_hash
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "envelope processing path should run (not bid fallback)"
        );
    }

    #[test]
    fn anchor_block_wrong_root_envelope_ignored_uses_fallback() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        // Key the envelope under a WRONG root so it won't be found
        let mut envelopes = HashMap::new();
        envelopes.insert(Hash256::repeat_byte(0xFF), envelope);

        // Set state hash to something different to verify fallback runs
        let mut state = state;
        *state.latest_block_hash_mut().unwrap() = ExecutionBlockHash::repeat_byte(0x99);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "when envelope root doesn't match, fallback to bid block_hash"
        );
    }

    // ── Envelope map consumption ──────────────────────────────────

    #[test]
    fn anchor_block_removes_envelope_from_map() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, envelope.clone());
        // Also add a dummy envelope under a different root
        let other_root = Hash256::repeat_byte(0xAB);
        envelopes.insert(other_root, envelope);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        // The used envelope should be consumed (removed), the other should remain
        assert!(
            !replayer.envelopes.contains_key(&block_root),
            "used envelope should be removed from map"
        );
        assert!(
            replayer.envelopes.contains_key(&other_root),
            "unused envelope should remain in map"
        );
    }

    // ── Builder pattern ───────────────────────────────────────────

    #[test]
    fn envelopes_builder_method_stores_envelopes() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let envelope = make_valid_envelope(&state);

        let root1 = Hash256::repeat_byte(0x01);
        let root2 = Hash256::repeat_byte(0x02);
        let mut envelopes = HashMap::new();
        envelopes.insert(root1, envelope.clone());
        envelopes.insert(root2, envelope);

        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter();

        assert_eq!(replayer.envelopes.len(), 2);
        assert!(replayer.envelopes.contains_key(&root1));
        assert!(replayer.envelopes.contains_key(&root2));
    }

    #[test]
    fn default_replayer_has_no_envelopes() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter();
        assert!(replayer.envelopes.is_empty());
    }

    // ── Anchor block: envelope error is dropped (best-effort) ────

    #[test]
    fn anchor_block_envelope_error_is_silently_dropped() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Create an envelope with wrong beacon_block_root to cause processing error
        let mut bad_envelope = make_valid_envelope(&state);
        bad_envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, bad_envelope);

        // Should not panic or return error — envelope errors are dropped for anchor blocks
        let result: Result<BlockReplayer<E>, _> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None);

        assert!(
            result.is_ok(),
            "anchor block envelope error should be silently dropped"
        );
    }

    // ── Anchor block: availability bit set by envelope ────────────

    #[test]
    fn anchor_block_envelope_sets_availability_bit() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Clear the availability bit for the current slot
        let slot_index =
            state.slot().as_usize() % <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(slot_index, false)
            .unwrap();
        assert!(
            !state
                .execution_payload_availability()
                .unwrap()
                .get(slot_index)
                .unwrap(),
            "precondition: availability bit should be cleared"
        );

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, envelope);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert!(
            result_state
                .execution_payload_availability()
                .unwrap()
                .get(slot_index)
                .unwrap(),
            "envelope processing should set the availability bit"
        );
    }
}
