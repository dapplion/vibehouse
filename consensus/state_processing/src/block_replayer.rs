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
    BeaconState, BeaconStateError, BlindedPayload, ChainSpec, EthSpec, Hash256, SignedBeaconBlock,
    SignedBlindedExecutionPayloadEnvelope, SignedExecutionPayloadEnvelope, Slot,
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
    /// Gloas ePBS: full envelopes to apply after each block, keyed by block root.
    /// When set, full envelope processing is applied after each Gloas block instead
    /// of just updating latest_block_hash from the bid.
    envelopes: HashMap<Hash256, SignedExecutionPayloadEnvelope<Spec>>,
    /// Gloas ePBS: blinded envelopes for finalized blocks where the full payload
    /// has been pruned. The replayer reconstructs a sufficient envelope by combining
    /// the blinded header with expected withdrawals from state.
    blinded_envelopes: HashMap<Hash256, SignedBlindedExecutionPayloadEnvelope<Spec>>,
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
            blinded_envelopes: HashMap::new(),
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

    /// Supply blinded Gloas execution payload envelopes for finalized blocks.
    ///
    /// When the full payload has been pruned from the database, blinded
    /// envelopes retain enough metadata (header fields, execution_requests,
    /// builder_index, etc.) to reconstruct a sufficient envelope for state
    /// replay by combining with expected withdrawals from the state.
    pub fn blinded_envelopes(
        mut self,
        blinded_envelopes: HashMap<Hash256, SignedBlindedExecutionPayloadEnvelope<E>>,
    ) -> Self {
        self.blinded_envelopes = blinded_envelopes;
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
                // Apply the envelope to bring latest_block_hash up to date so the next
                // block can validate bid.parent_block_hash == state.latest_block_hash.
                // If no envelope exists, the anchor block took the EMPTY path and
                // latest_block_hash is already correct as-is.
                if let Ok(_bid) = block.message().body().signed_execution_payload_bid() {
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
                    } else if let Some(blinded) = self.blinded_envelopes.remove(&block_root) {
                        // Reconstruct envelope from blinded + state's expected withdrawals.
                        // Convert milhouse List → ssz_types VariableList via collect.
                        let withdrawals = self
                            .state
                            .payload_expected_withdrawals()
                            .map(|w| w.iter().cloned().collect::<Vec<_>>().into())
                            .unwrap_or_default();
                        let envelope = blinded.into_full_with_withdrawals(withdrawals);
                        drop(process_execution_payload_envelope(
                            &mut self.state,
                            None,
                            &envelope,
                            VerifySignatures::False,
                            self.spec,
                        ));
                    }
                    // No envelope and no blinded envelope: anchor block took the EMPTY path.
                    // latest_block_hash is left unchanged — it already reflects the last FULL block.

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
            // The execution payload is delivered in a separate envelope. Apply the
            // full state transition (execution requests, builder payments,
            // availability bits, latest_block_hash) using the envelope if available.
            // If no envelope is found, the block took the EMPTY path (builder withheld
            // the payload) — in that case latest_block_hash is NOT updated, which is
            // correct: the EMPTY path leaves state.latest_block_hash unchanged.
            if let Ok(_bid) = block.message().body().signed_execution_payload_bid() {
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
                } else if let Some(blinded) = self.blinded_envelopes.remove(&block_root) {
                    // Reconstruct from blinded envelope + state's expected withdrawals.
                    // Convert milhouse List → ssz_types VariableList via collect.
                    let withdrawals = self
                        .state
                        .payload_expected_withdrawals()
                        .map(|w| w.iter().cloned().collect::<Vec<_>>().into())
                        .unwrap_or_default();
                    let envelope = blinded.into_full_with_withdrawals(withdrawals);
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
                }
                // No envelope and no blinded envelope: EMPTY path was taken.
                // latest_block_hash is left unchanged — correct for the EMPTY path.
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
        BeaconStateGloas, Builder, BuilderPendingPayment, BuilderPubkeyCache, CACHED_EPOCHS,
        Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExecutionPayloadEnvelope, ExecutionPayloadGloas, ExitCache, FixedVector, Fork,
        MinimalEthSpec, ProgressiveBalancesCache, PubkeyCache,
        SignedBlindedExecutionPayloadEnvelope, SignedExecutionPayloadBid, SlashingsCache,
        SyncAggregate, SyncCommittee, Unsigned, Vector, Withdrawal,
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
            builder_pubkey_cache: BuilderPubkeyCache::default(),
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
    fn anchor_block_without_envelope_leaves_hash_unchanged() {
        // No envelope supplied means the EMPTY path was taken.
        // latest_block_hash should be left unchanged (not overwritten with bid.block_hash).
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let original_hash = ExecutionBlockHash::repeat_byte(0xFF);
        let mut state = state;
        *state.latest_block_hash_mut().unwrap() = original_hash;
        assert_ne!(*state.latest_block_hash().unwrap(), bid.block_hash);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        // No envelopes supplied — simulates the EMPTY path
        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            original_hash,
            "without envelope (EMPTY path), latest_block_hash should be unchanged"
        );
    }

    #[test]
    fn anchor_block_no_envelope_does_not_change_latest_block_hash() {
        // Without an envelope (EMPTY path), latest_block_hash is never updated
        // regardless of the bid's block_hash value (including zero genesis bids).
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
            "without envelope, latest_block_hash should not be changed"
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

    // ── Anchor block: envelope processing updates latest_block_hash ─

    #[test]
    fn anchor_block_envelope_updates_latest_block_hash_correctly() {
        // When an envelope IS supplied, it should update latest_block_hash.
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
        // Envelope processing sets latest_block_hash = payload.block_hash
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "envelope processing should update latest_block_hash"
        );
    }

    #[test]
    fn anchor_block_wrong_root_envelope_leaves_hash_unchanged() {
        // When the envelope map doesn't have the block root (wrong key),
        // it behaves as if no envelope was provided (EMPTY path).
        // latest_block_hash should be left unchanged.
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

        // Key the envelope under a WRONG root so it won't be found
        let mut envelopes = HashMap::new();
        envelopes.insert(Hash256::repeat_byte(0xFF), envelope);

        let original_hash = ExecutionBlockHash::repeat_byte(0x99);
        let mut state = state;
        *state.latest_block_hash_mut().unwrap() = original_hash;

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            original_hash,
            "when no envelope found for block root, latest_block_hash should be unchanged"
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

    /// Build a blinded envelope from a full envelope.
    fn make_blinded_envelope(
        full: &SignedExecutionPayloadEnvelope<E>,
    ) -> SignedBlindedExecutionPayloadEnvelope<E> {
        SignedBlindedExecutionPayloadEnvelope::from_full(full)
    }

    // ── Blinded envelope builder method ──────────────────────────

    #[test]
    fn blinded_envelopes_builder_method_stores_blinded() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let envelope = make_valid_envelope(&state);
        let blinded = make_blinded_envelope(&envelope);

        let root1 = Hash256::repeat_byte(0x01);
        let root2 = Hash256::repeat_byte(0x02);
        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(root1, blinded.clone());
        blinded_envelopes.insert(root2, blinded);

        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter();

        assert_eq!(replayer.blinded_envelopes.len(), 2);
        assert!(replayer.blinded_envelopes.contains_key(&root1));
        assert!(replayer.blinded_envelopes.contains_key(&root2));
    }

    #[test]
    fn default_replayer_has_no_blinded_envelopes() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter();
        assert!(replayer.blinded_envelopes.is_empty());
    }

    // ── Anchor block: blinded envelope fallback ──────────────────

    #[test]
    fn anchor_block_with_blinded_envelope_updates_latest_block_hash() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);

        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "blinded envelope reconstruction should update latest_block_hash"
        );
    }

    #[test]
    fn anchor_block_blinded_envelope_removes_from_map() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let other_root = Hash256::repeat_byte(0xAB);
        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded.clone());
        blinded_envelopes.insert(other_root, blinded);

        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        // Check map before consuming replayer
        let used_removed = !replayer.blinded_envelopes.contains_key(&block_root);
        let unused_remains = replayer.blinded_envelopes.contains_key(&other_root);
        drop(replayer);

        assert!(
            used_removed,
            "used blinded envelope should be removed from map"
        );
        assert!(
            unused_remains,
            "unused blinded envelope should remain in map"
        );
    }

    #[test]
    fn anchor_block_full_envelope_preferred_over_blinded() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        // Supply both full and blinded envelopes — full should win
        let mut envelopes = HashMap::new();
        envelopes.insert(block_root, envelope);
        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        let replayer = BlockReplayer::<E>::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        // Check map state before consuming replayer
        let full_consumed = !replayer.envelopes.contains_key(&block_root);
        let blinded_remains = replayer.blinded_envelopes.contains_key(&block_root);
        let result_state = replayer.into_state();

        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "full envelope should be used (not blinded)"
        );
        assert!(full_consumed, "full envelope should be consumed");
        assert!(
            blinded_remains,
            "blinded envelope should remain unused when full is available"
        );
    }

    #[test]
    fn anchor_block_blinded_envelope_error_is_silently_dropped() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Create a blinded envelope with wrong beacon_block_root to cause processing error
        let mut envelope = make_valid_envelope(&state);
        envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        // Should not panic or return error — blinded envelope errors are dropped for anchor blocks
        let result: Result<BlockReplayer<E>, _> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None);

        assert!(
            result.is_ok(),
            "anchor block blinded envelope error should be silently dropped"
        );
    }

    #[test]
    fn anchor_block_blinded_envelope_sets_availability_bit() {
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

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
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
            "blinded envelope reconstruction should set the availability bit"
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

    // ── Blinded envelope: wrong root fallback ─────────────────────

    #[test]
    fn anchor_block_wrong_root_blinded_envelope_leaves_hash_unchanged() {
        // When the blinded envelope map doesn't have the correct block root,
        // the EMPTY path is taken and latest_block_hash stays unchanged.
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        let original_hash = ExecutionBlockHash::repeat_byte(0xEE);
        let mut state = state;
        *state.latest_block_hash_mut().unwrap() = original_hash;

        let mut envelope = make_valid_envelope(&state);
        fix_envelope_state_root(&state, &mut envelope, &spec);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        // Key the blinded envelope under a WRONG root
        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(Hash256::repeat_byte(0xFF), blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            original_hash,
            "wrong-root blinded envelope should leave latest_block_hash unchanged (EMPTY path)"
        );
    }

    // ── Blinded envelope: non-empty state withdrawals ─────────────

    #[test]
    fn anchor_block_blinded_envelope_uses_state_withdrawals() {
        // When the state has non-empty payload_expected_withdrawals,
        // the blinded envelope reconstruction should pass them through
        // to the reconstructed envelope's payload.withdrawals field.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();
        let bid_block_hash = bid.block_hash;

        // Set non-empty expected withdrawals in state
        let withdrawal = Withdrawal {
            index: 0,
            validator_index: 0,
            address: Address::repeat_byte(0xAA),
            amount: 1_000_000_000,
        };
        *state.payload_expected_withdrawals_mut().unwrap() = List::new(vec![withdrawal]).unwrap();

        // Build envelope with matching withdrawals (so envelope processing succeeds)
        let mut header = state.latest_block_header().clone();
        header.state_root = state.clone().canonical_root().unwrap();
        let beacon_block_root = header.tree_hash_root();
        let timestamp = compute_timestamp_at_slot(&state, state.slot(), &spec).unwrap();

        let payload = ExecutionPayloadGloas {
            parent_hash: *state.latest_block_hash().unwrap(),
            block_hash: bid.block_hash,
            prev_randao: bid.prev_randao,
            gas_limit: bid.gas_limit,
            timestamp,
            withdrawals: VariableList::new(vec![Withdrawal {
                index: 0,
                validator_index: 0,
                address: Address::repeat_byte(0xAA),
                amount: 1_000_000_000,
            }])
            .unwrap(),
            ..Default::default()
        };

        let mut envelope = SignedExecutionPayloadEnvelope {
            message: ExecutionPayloadEnvelope {
                payload,
                execution_requests: Default::default(),
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: state.slot(),
                state_root: Hash256::zero(),
            },
            signature: BlsSignature::empty(),
        };
        fix_envelope_state_root(&state, &mut envelope, &spec);

        // Create blinded envelope (strips payload, keeps header)
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "blinded envelope with state withdrawals should update latest_block_hash"
        );
    }

    // ── Availability bit: error and empty paths ───────────────────

    #[test]
    fn anchor_block_envelope_error_does_not_set_availability_bit() {
        // When envelope processing fails (e.g. wrong beacon_block_root),
        // the availability bit should NOT be set.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Clear the availability bit
        let slot_index =
            state.slot().as_usize() % <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(slot_index, false)
            .unwrap();

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

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        // Envelope processing errors are dropped for anchor blocks, but a failed
        // envelope should NOT set the availability bit.
        assert!(
            !result_state
                .execution_payload_availability()
                .unwrap()
                .get(slot_index)
                .unwrap(),
            "failed envelope processing should not set the availability bit"
        );
    }

    #[test]
    fn anchor_block_blinded_envelope_error_does_not_set_availability_bit() {
        // When blinded envelope reconstruction/processing fails,
        // the availability bit should NOT be set.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Clear the availability bit
        let slot_index =
            state.slot().as_usize() % <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(slot_index, false)
            .unwrap();

        // Create a blinded envelope with wrong beacon_block_root
        let mut envelope = make_valid_envelope(&state);
        envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);
        let blinded = make_blinded_envelope(&envelope);

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );
        let block_root = anchor_block.canonical_root();

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(block_root, blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert!(
            !result_state
                .execution_payload_availability()
                .unwrap()
                .get(slot_index)
                .unwrap(),
            "failed blinded envelope processing should not set the availability bit"
        );
    }

    #[test]
    fn anchor_block_empty_path_does_not_set_availability_bit() {
        // When no envelope is provided (EMPTY path), the availability bit
        // should remain cleared — the payload was never delivered.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = state.latest_execution_payload_bid().unwrap().clone();

        // Clear the availability bit
        let slot_index =
            state.slot().as_usize() % <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        state
            .execution_payload_availability_mut()
            .unwrap()
            .set(slot_index, false)
            .unwrap();

        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            bid,
        );

        // No envelopes at all — EMPTY path
        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert!(
            !result_state
                .execution_payload_availability()
                .unwrap()
                .get(slot_index)
                .unwrap(),
            "EMPTY path (no envelope) should not set the availability bit"
        );
    }

    // ── Non-anchor block: envelope processing ─────────────────────
    //
    // The non-anchor path (i > 0 in apply_blocks, lines 352-398) differs from
    // the anchor path: errors propagate instead of being silently dropped, and
    // full per_block_processing runs before envelope processing.

    /// Build a two-block sequence (anchor + non-anchor) where per_block_processing
    /// succeeds for the non-anchor block using a self-build bid.
    ///
    /// Returns (state, spec, anchor_block, non_anchor_block, non_anchor_block_root).
    type TwoBlockResult = (
        BeaconState<E>,
        ChainSpec,
        SignedBeaconBlock<E, BlindedPayload<E>>,
        SignedBeaconBlock<E, BlindedPayload<E>>,
        Hash256,
    );

    fn make_two_block_sequence() -> TwoBlockResult {
        let (mut state, mut spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Set all fork epochs to 0 so per_slot_processing recognizes the Gloas state.
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        spec.gloas_fork_epoch = Some(Epoch::new(0));

        // Set epoch participation and inactivity scores (needed for epoch/committee cache)
        let num_validators = state.validators().len();
        let gloas = state.as_gloas_mut().unwrap();
        gloas.previous_epoch_participation =
            List::new(vec![types::ParticipationFlags::default(); num_validators]).unwrap();
        gloas.current_epoch_participation =
            List::new(vec![types::ParticipationFlags::default(); num_validators]).unwrap();
        gloas.inactivity_scores = List::new(vec![0u64; num_validators]).unwrap();

        // Build the pubkey cache (needed for proposer index computation during per_block_processing)
        state.update_pubkey_cache().unwrap();

        // Fix the sync committee to use real validator pubkeys (process_sync_aggregate
        // looks up sync committee members in the pubkey cache, so empty pubkeys fail).
        let first_validator_pk = state.validators().get(0).unwrap().pubkey;
        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                first_validator_pk;
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: first_validator_pk,
        });
        let gloas = state.as_gloas_mut().unwrap();
        gloas.current_sync_committee = sync_committee.clone();
        gloas.next_sync_committee = sync_committee;

        let anchor_bid = state.latest_execution_payload_bid().unwrap().clone();

        // Build anchor block at state.slot() (slot 8). It just provides a state_root
        // and gets `continue`d.
        let anchor_block = make_gloas_block(
            state.slot(),
            Hash256::zero(),
            Hash256::repeat_byte(0xDD),
            anchor_bid,
        );

        // To get the correct parent_root for the non-anchor block, replay the anchor
        // block with target_slot=9 to advance the state. This runs cache_state and
        // per_slot_processing exactly as the real replayer would.
        let non_anchor_slot = Slot::new(9);
        let replayer: BlockReplayer<E> = BlockReplayer::new(state.clone(), &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block.clone()], Some(non_anchor_slot))
            .expect("anchor-only replay should succeed");
        let advanced_state = replayer.into_state();

        // The expected parent_root is what process_block_header will check against
        let expected_parent_root = advanced_state.latest_block_header().tree_hash_root();

        // Use a self-build bid (builder_index=u64::MAX) to avoid builder balance checks.
        let non_anchor_bid = ExecutionPayloadBid {
            parent_block_hash: *advanced_state.latest_block_hash().unwrap(),
            parent_block_root: expected_parent_root,
            block_hash: ExecutionBlockHash::repeat_byte(0xAA),
            prev_randao: *advanced_state
                .get_randao_mix(advanced_state.current_epoch())
                .unwrap(),
            slot: non_anchor_slot,
            builder_index: spec.builder_index_self_build,
            value: 0,
            ..Default::default()
        };

        // Build the non-anchor block manually so the bid uses infinity signature
        // (required for self-build bids by process_execution_payload_bid).
        let non_anchor_block_inner = BeaconBlock::Gloas(BeaconBlockGloas {
            slot: non_anchor_slot,
            proposer_index: 0,
            parent_root: expected_parent_root,
            state_root: Hash256::repeat_byte(0xEE),
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
                    message: non_anchor_bid,
                    signature: BlsSignature::infinity().unwrap(),
                },
                payload_attestations: VariableList::empty(),
                _phantom: PhantomData,
            },
        });
        let non_anchor_block =
            SignedBeaconBlock::from_block(non_anchor_block_inner, BlsSignature::empty());
        let non_anchor_root = non_anchor_block.canonical_root();

        (state, spec, anchor_block, non_anchor_block, non_anchor_root)
    }

    /// Build a valid envelope for the non-anchor block's post-processing state.
    fn make_non_anchor_envelope(
        state: &BeaconState<E>,
        spec: &ChainSpec,
        anchor_block: &SignedBeaconBlock<E, BlindedPayload<E>>,
        non_anchor_block: &SignedBeaconBlock<E, BlindedPayload<E>>,
    ) -> SignedExecutionPayloadEnvelope<E> {
        // We need the post-block state to build a valid envelope. Run the
        // replayer up to block processing without envelope, then build envelope
        // from the resulting state.
        let replayer: BlockReplayer<E> = BlockReplayer::new(state.clone(), spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block.clone(), non_anchor_block.clone()], None)
            .expect("two-block replay without envelopes should succeed");
        let post_block_state = replayer.into_state();

        let bid = post_block_state
            .latest_execution_payload_bid()
            .unwrap()
            .clone();
        let latest_block_hash = *post_block_state.latest_block_hash().unwrap();

        let mut header = post_block_state.latest_block_header().clone();
        header.state_root = post_block_state.clone().canonical_root().unwrap();
        let beacon_block_root = header.tree_hash_root();

        let timestamp =
            compute_timestamp_at_slot(&post_block_state, post_block_state.slot(), spec).unwrap();

        let payload = ExecutionPayloadGloas {
            parent_hash: latest_block_hash,
            block_hash: bid.block_hash,
            prev_randao: bid.prev_randao,
            gas_limit: bid.gas_limit,
            timestamp,
            withdrawals: VariableList::default(),
            ..Default::default()
        };

        let mut envelope = SignedExecutionPayloadEnvelope {
            message: ExecutionPayloadEnvelope {
                payload,
                execution_requests: Default::default(),
                builder_index: bid.builder_index,
                beacon_block_root,
                slot: post_block_state.slot(),
                state_root: Hash256::zero(),
            },
            signature: BlsSignature::empty(),
        };
        fix_envelope_state_root(&post_block_state, &mut envelope, spec);
        envelope
    }

    #[test]
    fn non_anchor_block_with_envelope_updates_latest_block_hash() {
        // Non-anchor blocks (i>0) should apply envelope processing after
        // per_block_processing, updating latest_block_hash to the bid's block_hash.
        let (state, spec, anchor_block, non_anchor_block, non_anchor_root) =
            make_two_block_sequence();

        let envelope = make_non_anchor_envelope(&state, &spec, &anchor_block, &non_anchor_block);
        let bid_block_hash = ExecutionBlockHash::repeat_byte(0xAA);

        let mut envelopes = HashMap::new();
        envelopes.insert(non_anchor_root, envelope);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block, non_anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "non-anchor envelope processing should update latest_block_hash"
        );
    }

    #[test]
    fn non_anchor_block_with_blinded_envelope_updates_latest_block_hash() {
        // Non-anchor blocks should also support blinded envelopes reconstructed
        // from the state's expected withdrawals (cold storage replay path).
        let (state, spec, anchor_block, non_anchor_block, non_anchor_root) =
            make_two_block_sequence();

        let envelope = make_non_anchor_envelope(&state, &spec, &anchor_block, &non_anchor_block);
        let bid_block_hash = ExecutionBlockHash::repeat_byte(0xAA);

        let blinded = make_blinded_envelope(&envelope);

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(non_anchor_root, blinded);

        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block, non_anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            bid_block_hash,
            "non-anchor blinded envelope should update latest_block_hash"
        );
    }

    #[test]
    fn non_anchor_block_envelope_error_propagates() {
        // Unlike anchor blocks where errors are silently dropped, non-anchor blocks
        // should propagate envelope processing errors as BlockReplayError.
        let (state, spec, anchor_block, non_anchor_block, non_anchor_root) =
            make_two_block_sequence();

        // Create an envelope with wrong beacon_block_root to trigger an error
        let mut bad_envelope =
            make_non_anchor_envelope(&state, &spec, &anchor_block, &non_anchor_block);
        bad_envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);

        let mut envelopes = HashMap::new();
        envelopes.insert(non_anchor_root, bad_envelope);

        let result: Result<BlockReplayer<E>, _> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .envelopes(envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block, non_anchor_block], None);

        match result {
            Ok(_) => panic!("non-anchor block should propagate envelope processing errors"),
            Err(err) => {
                let err_str = format!("{:?}", err);
                assert!(
                    err_str.contains("EnvelopeProcessingError"),
                    "error should be an EnvelopeProcessingError, got: {}",
                    err_str
                );
            }
        }
    }

    #[test]
    fn non_anchor_block_empty_path_leaves_hash_unchanged() {
        // When no envelope is supplied for a non-anchor block, the EMPTY path is
        // taken: latest_block_hash should NOT be updated to the bid's block_hash.
        let (state, spec, anchor_block, non_anchor_block, _non_anchor_root) =
            make_two_block_sequence();

        let original_hash = ExecutionBlockHash::repeat_byte(0x02);

        // No envelopes supplied — simulates EMPTY path (builder withheld payload)
        let replayer: BlockReplayer<E> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block, non_anchor_block], None)
            .unwrap();

        let result_state = replayer.into_state();
        assert_eq!(
            *result_state.latest_block_hash().unwrap(),
            original_hash,
            "without envelope (EMPTY path), latest_block_hash should be unchanged"
        );
    }

    #[test]
    fn non_anchor_block_blinded_envelope_error_propagates() {
        // Blinded envelope reconstruction errors should also propagate for
        // non-anchor blocks (unlike anchor blocks where they're dropped).
        let (state, spec, anchor_block, non_anchor_block, non_anchor_root) =
            make_two_block_sequence();

        // Create a blinded envelope with wrong beacon_block_root
        let mut bad_envelope =
            make_non_anchor_envelope(&state, &spec, &anchor_block, &non_anchor_block);
        bad_envelope.message.beacon_block_root = Hash256::repeat_byte(0xFF);
        let blinded = make_blinded_envelope(&bad_envelope);

        let mut blinded_envelopes = HashMap::new();
        blinded_envelopes.insert(non_anchor_root, blinded);

        let result: Result<BlockReplayer<E>, _> = BlockReplayer::new(state, &spec)
            .no_signature_verification()
            .blinded_envelopes(blinded_envelopes)
            .no_state_root_iter()
            .apply_blocks(vec![anchor_block, non_anchor_block], None);

        match result {
            Ok(_) => panic!("non-anchor block should propagate blinded envelope errors"),
            Err(err) => {
                let err_str = format!("{:?}", err);
                assert!(
                    err_str.contains("EnvelopeProcessingError"),
                    "error should be an EnvelopeProcessingError, got: {}",
                    err_str
                );
            }
        }
    }
}
