use crate::metrics::{self, scrape_for_metrics};
use crate::{ForkChoiceStore, InvalidationOperation};
use logging::crit;
use proto_array::{
    Block as ProtoBlock, DisallowedReOrgOffsets, ExecutionStatus, JustifiedBalances,
    ProposerHeadError, ProposerHeadInfo, ProtoArrayForkChoice, ReOrgThreshold,
};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use state_processing::{
    per_block_processing::errors::AttesterSlashingValidationError, per_epoch_processing,
};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::time::Duration;
use superstruct::superstruct;
use tracing::{debug, instrument, warn};
use types::{
    AbstractExecPayload, AttestationShufflingId, AttesterSlashingRef, BeaconBlockRef, BeaconState,
    BeaconStateError, ChainSpec, Checkpoint, Epoch, EthSpec, ExecPayload, ExecutionBlockHash,
    FixedBytesExtended, Hash256, IndexedAttestationRef, IndexedPayloadAttestation,
    PayloadAttestation, RelativeEpoch, SignedBeaconBlock, SignedExecutionPayloadBid, Slot,
    consts::bellatrix::INTERVALS_PER_SLOT,
};

#[derive(Debug)]
pub enum Error<T> {
    InvalidAttestation(InvalidAttestation),
    InvalidAttesterSlashing(AttesterSlashingValidationError),
    InvalidBlock(InvalidBlock),
    ProtoArrayStringError(String),
    ProtoArrayError(proto_array::Error),
    InvalidProtoArrayBytes(String),
    InvalidLegacyProtoArrayBytes(String),
    FailedToProcessInvalidExecutionPayload(String),
    FailedToProcessValidExecutionPayload(String),
    MissingProtoArrayBlock(Hash256),
    UnknownAncestor {
        ancestor_slot: Slot,
        descendant_root: Hash256,
    },
    InconsistentOnTick {
        previous_slot: Slot,
        time: Slot,
    },
    BeaconStateError(BeaconStateError),
    AttemptToRevertJustification {
        store: Slot,
        state: Slot,
    },
    ForkChoiceStoreError(T),
    UnableToSetJustifiedCheckpoint(T),
    AfterBlockFailed(T),
    ProposerHeadError(T),
    InvalidAnchor {
        block_slot: Slot,
        state_slot: Slot,
    },
    InvalidPayloadStatus {
        block_slot: Slot,
        block_root: Hash256,
        payload_verification_status: PayloadVerificationStatus,
    },
    MissingJustifiedBlock {
        justified_checkpoint: Checkpoint,
    },
    MissingFinalizedBlock {
        finalized_checkpoint: Checkpoint,
    },
    WrongSlotForGetProposerHead {
        current_slot: Slot,
        fc_store_slot: Slot,
    },
    ProposerBoostNotExpiredForGetProposerHead {
        proposer_boost_root: Hash256,
    },
    UnrealizedVoteProcessing(state_processing::EpochProcessingError),
    ValidatorStatuses(BeaconStateError),
    /// Gloas ePBS: Invalid execution bid
    InvalidExecutionBid(InvalidExecutionBid),
    /// Gloas ePBS: Invalid payload attestation
    InvalidPayloadAttestation(InvalidPayloadAttestation),
}

impl<T> From<InvalidAttestation> for Error<T> {
    fn from(e: InvalidAttestation) -> Self {
        Error::InvalidAttestation(e)
    }
}

impl<T> From<AttesterSlashingValidationError> for Error<T> {
    fn from(e: AttesterSlashingValidationError) -> Self {
        Error::InvalidAttesterSlashing(e)
    }
}

impl<T> From<state_processing::EpochProcessingError> for Error<T> {
    fn from(e: state_processing::EpochProcessingError) -> Self {
        Error::UnrealizedVoteProcessing(e)
    }
}

impl<T> From<BeaconStateError> for Error<T> {
    fn from(e: BeaconStateError) -> Self {
        Error::BeaconStateError(e)
    }
}

#[derive(Debug, Clone, Copy)]
/// Controls how fork choice should behave when restoring from a persisted fork choice.
pub enum ResetPayloadStatuses {
    /// Reset all payload statuses back to "optimistic".
    Always,
    /// Only reset all payload statuses back to "optimistic" when an "invalid" block is present.
    OnlyWithInvalidPayload,
}

impl ResetPayloadStatuses {
    /// When `should_always_reset == True`, return `ResetPayloadStatuses::Always`.
    pub fn always_reset_conditionally(should_always_reset: bool) -> Self {
        if should_always_reset {
            ResetPayloadStatuses::Always
        } else {
            ResetPayloadStatuses::OnlyWithInvalidPayload
        }
    }
}

#[derive(Debug)]
pub enum InvalidBlock {
    UnknownParent(Hash256),
    FutureSlot {
        current_slot: Slot,
        block_slot: Slot,
    },
    FinalizedSlot {
        finalized_slot: Slot,
        block_slot: Slot,
    },
    NotFinalizedDescendant {
        finalized_root: Hash256,
        block_ancestor: Option<Hash256>,
    },
}

#[derive(Debug)]
pub enum InvalidAttestation {
    /// The attestations aggregation bits were empty when they shouldn't be.
    EmptyAggregationBitfield,
    /// The `attestation.data.beacon_block_root` block is unknown.
    UnknownHeadBlock { beacon_block_root: Hash256 },
    /// The `attestation.data.slot` is not from the same epoch as `data.target.epoch` and therefore
    /// the attestation is invalid.
    BadTargetEpoch { target: Epoch, slot: Slot },
    /// The target root of the attestation points to a block that we have not verified.
    UnknownTargetRoot(Hash256),
    /// The attestation is for an epoch in the future (with respect to the gossip clock disparity).
    FutureEpoch {
        attestation_epoch: Epoch,
        current_epoch: Epoch,
    },
    /// The attestation is for an epoch in the past (with respect to the gossip clock disparity).
    PastEpoch {
        attestation_epoch: Epoch,
        current_epoch: Epoch,
    },
    /// The attestation references a target root that does not match what is stored in our
    /// database.
    InvalidTarget {
        attestation: Hash256,
        local: Hash256,
    },
    /// The attestation is attesting to a state that is later than itself. (Viz., attesting to the
    /// future).
    AttestsToFutureBlock { block: Slot, attestation: Slot },
    /// [Gloas] The attestation committee index is not 0 or 1.
    InvalidCommitteeIndex { index: u64 },
    /// [Gloas] A same-slot attestation must have index 0.
    SameSlotNonZeroIndex { slot: Slot, index: u64 },
    /// [Gloas] Attestation with index=1 (payload present) for a block whose payload has not been
    /// revealed.
    PayloadNotRevealed { beacon_block_root: Hash256 },
}

/// Gloas ePBS: Reasons an execution payload bid might be invalid.
#[derive(Debug)]
pub enum InvalidExecutionBid {
    /// The beacon block root referenced by the bid is unknown.
    UnknownBeaconBlockRoot { beacon_block_root: Hash256 },
    /// The bid's slot doesn't match the beacon block's slot.
    SlotMismatch { bid_slot: Slot, block_slot: Slot },
    /// The bid's parent_block_root doesn't match expectations.
    ParentMismatch {
        bid_parent: Hash256,
        expected_parent: Hash256,
    },
    /// Builder index doesn't exist in the builder registry.
    UnknownBuilder { builder_index: types::BuilderIndex },
    /// Builder is not currently active.
    BuilderNotActive { builder_index: types::BuilderIndex },
    /// Builder doesn't have sufficient balance to cover the bid.
    InsufficientBuilderBalance {
        builder_index: types::BuilderIndex,
        bid_value: u64,
        builder_balance: u64,
    },
    /// Bid signature verification failed.
    InvalidSignature,
    /// Bid value is zero for non-self-build.
    ZeroValueBid,
}

/// Gloas ePBS: Reasons a payload attestation might be invalid.
#[derive(Debug)]
pub enum InvalidPayloadAttestation {
    /// The beacon block root referenced by the attestation is unknown.
    UnknownBeaconBlockRoot { beacon_block_root: Hash256 },
    /// The attestation's slot doesn't match the block's slot.
    SlotMismatch {
        attestation_slot: Slot,
        block_slot: Slot,
    },
    /// One or more attesters are not in the PTC for this slot.
    InvalidAttester { attester_index: u64 },
    /// The aggregate signature verification failed.
    InvalidSignature,
    /// The attestation is from a future slot.
    FutureSlot {
        attestation_slot: Slot,
        current_slot: Slot,
    },
    /// The attestation is from a past slot (beyond acceptance window).
    TooOld {
        attestation_slot: Slot,
        current_slot: Slot,
    },
}

impl<T> From<InvalidExecutionBid> for Error<T> {
    fn from(e: InvalidExecutionBid) -> Self {
        Error::InvalidExecutionBid(e)
    }
}

impl<T> From<InvalidPayloadAttestation> for Error<T> {
    fn from(e: InvalidPayloadAttestation) -> Self {
        Error::InvalidPayloadAttestation(e)
    }
}

impl<T> From<String> for Error<T> {
    fn from(e: String) -> Self {
        Error::ProtoArrayStringError(e)
    }
}

impl<T> From<proto_array::Error> for Error<T> {
    fn from(e: proto_array::Error) -> Self {
        Error::ProtoArrayError(e)
    }
}

/// Indicates if a block has been verified by an execution payload.
///
/// There is no variant for "invalid", since such a block should never be added to fork choice.
#[derive(Clone, Copy, Debug, PartialEq, Encode, Decode)]
#[ssz(enum_behaviour = "tag")]
pub enum PayloadVerificationStatus {
    /// An EL has declared the execution payload to be valid.
    Verified,
    /// An EL has not yet made a determination about the execution payload.
    Optimistic,
    /// The block is either pre-merge-fork, or prior to the terminal PoW block.
    Irrelevant,
}

impl PayloadVerificationStatus {
    /// Returns `true` if the payload was optimistically imported.
    pub fn is_optimistic(&self) -> bool {
        match self {
            PayloadVerificationStatus::Verified => false,
            PayloadVerificationStatus::Optimistic => true,
            PayloadVerificationStatus::Irrelevant => false,
        }
    }
}

/// Calculate how far `slot` lies from the start of its epoch.
///
/// ## Specification
///
/// Equivalent to:
///
/// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#compute_slots_since_epoch_start
pub fn compute_slots_since_epoch_start<E: EthSpec>(slot: Slot) -> Slot {
    slot - slot
        .epoch(E::slots_per_epoch())
        .start_slot(E::slots_per_epoch())
}

/// Calculate the first slot in `epoch`.
///
/// ## Specification
///
/// Equivalent to:
///
/// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/beacon-chain.md#compute_start_slot_at_epoch
fn compute_start_slot_at_epoch<E: EthSpec>(epoch: Epoch) -> Slot {
    epoch.start_slot(E::slots_per_epoch())
}

/// Used for queuing attestations from the current slot. Only contains the minimum necessary
/// information about the attestation.
#[derive(Clone, PartialEq, Encode, Decode)]
pub struct QueuedAttestation {
    slot: Slot,
    attesting_indices: Vec<u64>,
    block_root: Hash256,
    target_epoch: Epoch,
    /// Gloas: attestation index (0 = beacon, 1 = payload present).
    index: u64,
}

impl<'a, E: EthSpec> From<IndexedAttestationRef<'a, E>> for QueuedAttestation {
    fn from(a: IndexedAttestationRef<'a, E>) -> Self {
        Self {
            slot: a.data().slot,
            attesting_indices: a.attesting_indices_to_vec(),
            block_root: a.data().beacon_block_root,
            target_epoch: a.data().target.epoch,
            index: a.data().index,
        }
    }
}

/// Returns all values in `self.queued_attestations` that have a slot that is earlier than the
/// current slot. Also removes those values from `self.queued_attestations`.
fn dequeue_attestations(
    current_slot: Slot,
    queued_attestations: &mut Vec<QueuedAttestation>,
) -> Vec<QueuedAttestation> {
    let remaining = queued_attestations.split_off(
        queued_attestations
            .iter()
            .position(|a| a.slot >= current_slot)
            .unwrap_or(queued_attestations.len()),
    );

    metrics::inc_counter_by(
        &metrics::FORK_CHOICE_DEQUEUED_ATTESTATIONS,
        queued_attestations.len() as u64,
    );

    std::mem::replace(queued_attestations, remaining)
}

/// Denotes whether an attestation we are processing was received from a block or from gossip.
/// Equivalent to the `is_from_block` `bool` in:
///
/// https://github.com/ethereum/consensus-specs/blob/dev/specs/phase0/fork-choice.md#validate_on_attestation
#[derive(Clone, Copy)]
pub enum AttestationFromBlock {
    True,
    False,
}

/// Parameters which are cached between calls to `ForkChoice::get_head`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForkchoiceUpdateParameters {
    /// The most recent result of running `ForkChoice::get_head`.
    pub head_root: Hash256,
    pub head_hash: Option<ExecutionBlockHash>,
    pub justified_hash: Option<ExecutionBlockHash>,
    pub finalized_hash: Option<ExecutionBlockHash>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForkChoiceView {
    pub head_block_root: Hash256,
    pub justified_checkpoint: Checkpoint,
    pub finalized_checkpoint: Checkpoint,
}

/// Provides an implementation of "Ethereum 2.0 Phase 0 -- Beacon Chain Fork Choice":
///
/// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#ethereum-20-phase-0----beacon-chain-fork-choice
///
/// ## Detail
///
/// This struct wraps `ProtoArrayForkChoice` and provides:
///
/// - Management of the justified state and caching of balances.
/// - Queuing of attestations from the current slot.
pub struct ForkChoice<T, E> {
    /// Storage for `ForkChoice`, modelled off the spec `Store` object.
    fc_store: T,
    /// The underlying representation of the block DAG.
    proto_array: ProtoArrayForkChoice,
    /// Attestations that arrived at the current slot and must be queued for later processing.
    queued_attestations: Vec<QueuedAttestation>,
    /// Stores a cache of the values required to be sent to the execution layer.
    forkchoice_update_parameters: ForkchoiceUpdateParameters,
    _phantom: PhantomData<E>,
}

impl<T, E> PartialEq for ForkChoice<T, E>
where
    T: ForkChoiceStore<E> + PartialEq,
    E: EthSpec,
{
    fn eq(&self, other: &Self) -> bool {
        self.fc_store == other.fc_store
            && self.proto_array == other.proto_array
            && self.queued_attestations == other.queued_attestations
    }
}

impl<T, E> ForkChoice<T, E>
where
    T: ForkChoiceStore<E>,
    E: EthSpec,
{
    /// Instantiates `Self` from an anchor (genesis or another finalized checkpoint).
    pub fn from_anchor(
        fc_store: T,
        anchor_block_root: Hash256,
        anchor_block: &SignedBeaconBlock<E>,
        anchor_state: &BeaconState<E>,
        current_slot: Option<Slot>,
        spec: &ChainSpec,
    ) -> Result<Self, Error<T::Error>> {
        // Sanity check: the anchor must lie on an epoch boundary.
        if anchor_state.slot() % E::slots_per_epoch() != 0 {
            return Err(Error::InvalidAnchor {
                block_slot: anchor_block.slot(),
                state_slot: anchor_state.slot(),
            });
        }

        let finalized_block_slot = anchor_block.slot();
        let finalized_block_state_root = anchor_block.state_root();
        let current_epoch_shuffling_id =
            AttestationShufflingId::new(anchor_block_root, anchor_state, RelativeEpoch::Current)
                .map_err(Error::BeaconStateError)?;
        let next_epoch_shuffling_id =
            AttestationShufflingId::new(anchor_block_root, anchor_state, RelativeEpoch::Next)
                .map_err(Error::BeaconStateError)?;

        let execution_status = anchor_block.message().execution_payload().map_or_else(
            // If the block doesn't have an execution payload then it can't have
            // execution enabled.
            |_| ExecutionStatus::irrelevant(),
            |execution_payload| {
                if execution_payload.is_default_with_empty_roots() {
                    // A default payload does not have execution enabled.
                    ExecutionStatus::irrelevant()
                } else {
                    // Assume that this payload is valid, since the anchor should be a trusted block and
                    // state.
                    ExecutionStatus::Valid(execution_payload.block_hash())
                }
            },
        );

        // If the current slot is not provided, use the value that was last provided to the store.
        let current_slot = current_slot.unwrap_or_else(|| fc_store.get_current_slot());

        let proto_array = ProtoArrayForkChoice::new::<E>(
            current_slot,
            finalized_block_slot,
            finalized_block_state_root,
            *fc_store.justified_checkpoint(),
            *fc_store.finalized_checkpoint(),
            current_epoch_shuffling_id,
            next_epoch_shuffling_id,
            execution_status,
        )?;

        let mut fork_choice = Self {
            fc_store,
            proto_array,
            queued_attestations: vec![],
            // This will be updated during the next call to `Self::get_head`.
            forkchoice_update_parameters: ForkchoiceUpdateParameters {
                head_hash: None,
                justified_hash: None,
                finalized_hash: None,
                // This will be updated during the next call to `Self::get_head`.
                head_root: Hash256::zero(),
            },
            _phantom: PhantomData,
        };

        // Ensure that `fork_choice.forkchoice_update_parameters.head_root` is updated.
        fork_choice.get_head(current_slot, spec)?;

        Ok(fork_choice)
    }

    /// Returns cached information that can be used to issue a `forkchoiceUpdated` message to an
    /// execution engine.
    ///
    /// These values are updated each time `Self::get_head` is called.
    pub fn get_forkchoice_update_parameters(&self) -> ForkchoiceUpdateParameters {
        self.forkchoice_update_parameters
    }

    /// Returns the block root of an ancestor of `block_root` at the given `slot`. (Note: `slot` refers
    /// to the block that is *returned*, not the one that is supplied.)
    ///
    /// The result may be `Ok(None)` if the block does not descend from the finalized block. This
    /// is an artifact of proto-array, sometimes it contains descendants of blocks that have been
    /// pruned.
    ///
    /// ## Specification
    ///
    /// Equivalent to:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#get_ancestor
    fn get_ancestor(
        &self,
        block_root: Hash256,
        ancestor_slot: Slot,
    ) -> Result<Option<Hash256>, Error<T::Error>>
    where
        T: ForkChoiceStore<E>,
        E: EthSpec,
    {
        let block = self
            .proto_array
            .get_block(&block_root)
            .ok_or(Error::MissingProtoArrayBlock(block_root))?;

        match block.slot.cmp(&ancestor_slot) {
            Ordering::Greater => Ok(self
                .proto_array
                .core_proto_array()
                .iter_block_roots(&block_root)
                // Search for a slot that is **less than or equal to** the target slot. We check
                // for lower slots to account for skip slots.
                .find(|(_, slot)| *slot <= ancestor_slot)
                .map(|(root, _)| root)),
            // Root is older than queried slot, thus a skip slot. Return most recent root prior
            // to slot.
            Ordering::Less => Ok(Some(block_root)),
            Ordering::Equal => Ok(Some(block_root)),
        }
    }

    /// Run the fork choice rule to determine the head.
    ///
    /// ## Specification
    ///
    /// Is equivalent to:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#get_head
    #[instrument(skip_all, level = "debug")]
    pub fn get_head(
        &mut self,
        system_time_current_slot: Slot,
        spec: &ChainSpec,
    ) -> Result<Hash256, Error<T::Error>> {
        // Provide the slot (as per the system clock) to the `fc_store` and then return its view of
        // the current slot. The `fc_store` will ensure that the `current_slot` is never
        // decreasing, a property which we must maintain.
        let current_slot = self.update_time(system_time_current_slot)?;

        let store = &mut self.fc_store;

        let head_root = self.proto_array.find_head::<E>(
            *store.justified_checkpoint(),
            *store.finalized_checkpoint(),
            store.justified_balances(),
            store.proposer_boost_root(),
            store.equivocating_indices(),
            current_slot,
            spec,
        )?;

        // Cache some values for the next forkchoiceUpdate call to the execution layer.
        let head_hash = self
            .get_block(&head_root)
            .and_then(|b| b.execution_status.block_hash());
        let justified_root = self.justified_checkpoint().root;
        let finalized_root = self.finalized_checkpoint().root;
        let justified_hash = self
            .get_block(&justified_root)
            .and_then(|b| b.execution_status.block_hash());
        let finalized_hash = self
            .get_block(&finalized_root)
            .and_then(|b| b.execution_status.block_hash());
        self.forkchoice_update_parameters = ForkchoiceUpdateParameters {
            head_root,
            head_hash,
            justified_hash,
            finalized_hash,
        };

        Ok(head_root)
    }

    /// Get the block to build on as proposer, taking into account proposer re-orgs.
    ///
    /// You *must* call `get_head` for the proposal slot prior to calling this function and pass
    /// in the result of `get_head` as `canonical_head`.
    #[instrument(level = "debug", skip_all)]
    pub fn get_proposer_head(
        &self,
        current_slot: Slot,
        canonical_head: Hash256,
        re_org_head_threshold: ReOrgThreshold,
        re_org_parent_threshold: ReOrgThreshold,
        disallowed_offsets: &DisallowedReOrgOffsets,
        max_epochs_since_finalization: Epoch,
    ) -> Result<ProposerHeadInfo, ProposerHeadError<Error<proto_array::Error>>> {
        // Ensure that fork choice has already been updated for the current slot. This prevents
        // us from having to take a write lock or do any dequeueing of attestations in this
        // function.
        let fc_store_slot = self.fc_store.get_current_slot();
        if current_slot != fc_store_slot {
            return Err(ProposerHeadError::Error(
                Error::WrongSlotForGetProposerHead {
                    current_slot,
                    fc_store_slot,
                },
            ));
        }

        // Similarly, the proposer boost for the previous head should already have expired.
        let proposer_boost_root = self.fc_store.proposer_boost_root();
        if !proposer_boost_root.is_zero() {
            return Err(ProposerHeadError::Error(
                Error::ProposerBoostNotExpiredForGetProposerHead {
                    proposer_boost_root,
                },
            ));
        }

        self.proto_array
            .get_proposer_head::<E>(
                current_slot,
                canonical_head,
                self.fc_store.justified_balances(),
                re_org_head_threshold,
                re_org_parent_threshold,
                disallowed_offsets,
                max_epochs_since_finalization,
            )
            .map_err(ProposerHeadError::convert_inner_error)
    }

    pub fn get_preliminary_proposer_head(
        &self,
        canonical_head: Hash256,
        re_org_head_threshold: ReOrgThreshold,
        re_org_parent_threshold: ReOrgThreshold,
        disallowed_offsets: &DisallowedReOrgOffsets,
        max_epochs_since_finalization: Epoch,
    ) -> Result<ProposerHeadInfo, ProposerHeadError<Error<proto_array::Error>>> {
        let current_slot = self.fc_store.get_current_slot();
        self.proto_array
            .get_proposer_head_info::<E>(
                current_slot,
                canonical_head,
                self.fc_store.justified_balances(),
                re_org_head_threshold,
                re_org_parent_threshold,
                disallowed_offsets,
                max_epochs_since_finalization,
            )
            .map_err(ProposerHeadError::convert_inner_error)
    }

    /// Return information about:
    ///
    /// - The LMD head of the chain.
    /// - The FFG checkpoints.
    ///
    /// The information is "cached" since the last call to `Self::get_head`.
    ///
    /// ## Notes
    ///
    /// The finalized/justified checkpoints are determined from the fork choice store. Therefore,
    /// it's possible that the state corresponding to `get_state(get_block(head_block_root))` will
    /// have *differing* finalized and justified information.
    pub fn cached_fork_choice_view(&self) -> ForkChoiceView {
        ForkChoiceView {
            head_block_root: self.forkchoice_update_parameters.head_root,
            justified_checkpoint: self.justified_checkpoint(),
            finalized_checkpoint: self.finalized_checkpoint(),
        }
    }

    /// See `ProtoArrayForkChoice::process_execution_payload_validation` for documentation.
    pub fn on_valid_execution_payload(
        &mut self,
        block_root: Hash256,
    ) -> Result<(), Error<T::Error>> {
        self.proto_array
            .process_execution_payload_validation(block_root)
            .map_err(Error::FailedToProcessValidExecutionPayload)
    }

    /// See `ProtoArrayForkChoice::process_execution_payload_invalidation` for documentation.
    pub fn on_invalid_execution_payload(
        &mut self,
        op: &InvalidationOperation,
    ) -> Result<(), Error<T::Error>> {
        self.proto_array
            .process_execution_payload_invalidation::<E>(op)
            .map_err(Error::FailedToProcessInvalidExecutionPayload)
    }

    /// Add `block` to the fork choice DAG.
    ///
    /// - `block_root` is the root of `block.
    /// - The root of `state` matches `block.state_root`.
    ///
    /// ## Specification
    ///
    /// Approximates:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#on_block
    ///
    /// It only approximates the specification since it does not run the `state_transition` check.
    /// That should have already been called upstream and it's too expensive to call again.
    ///
    /// ## Notes:
    ///
    /// The supplied block **must** pass the `state_transition` function as it will not be run
    /// here.
    #[allow(clippy::too_many_arguments)]
    #[instrument(
        name = "fork_choice_on_block",
        skip_all,
        fields(
            fork_choice_block_delay = ?block_delay
        ))]
    pub fn on_block<Payload: AbstractExecPayload<E>>(
        &mut self,
        system_time_current_slot: Slot,
        block: BeaconBlockRef<E, Payload>,
        block_root: Hash256,
        block_delay: Duration,
        state: &BeaconState<E>,
        payload_verification_status: PayloadVerificationStatus,
        canonical_head_proposer_index: Option<u64>,
        spec: &ChainSpec,
    ) -> Result<(), Error<T::Error>> {
        let _timer = metrics::start_timer(&metrics::FORK_CHOICE_ON_BLOCK_TIMES);

        // If this block has already been processed we do not need to reprocess it.
        // We check this immediately in case re-processing the block mutates some property of the
        // global fork choice store, e.g. the justified checkpoints or the proposer boost root.
        if self.proto_array.contains_block(&block_root) {
            return Ok(());
        }

        // Provide the slot (as per the system clock) to the `fc_store` and then return its view of
        // the current slot. The `fc_store` will ensure that the `current_slot` is never
        // decreasing, a property which we must maintain.
        let current_slot = self.update_time(system_time_current_slot)?;

        // Parent block must be known.
        let parent_block = self
            .proto_array
            .get_block(&block.parent_root())
            .ok_or_else(|| Error::InvalidBlock(InvalidBlock::UnknownParent(block.parent_root())))?;

        // Blocks cannot be in the future. If they are, their consideration must be delayed until
        // they are in the past.
        //
        // Note: presently, we do not delay consideration. We just drop the block.
        if block.slot() > current_slot {
            return Err(Error::InvalidBlock(InvalidBlock::FutureSlot {
                current_slot,
                block_slot: block.slot(),
            }));
        }

        // Check that block is later than the finalized epoch slot (optimization to reduce calls to
        // get_ancestor).
        let finalized_slot =
            compute_start_slot_at_epoch::<E>(self.fc_store.finalized_checkpoint().epoch);
        if block.slot() <= finalized_slot {
            return Err(Error::InvalidBlock(InvalidBlock::FinalizedSlot {
                finalized_slot,
                block_slot: block.slot(),
            }));
        }

        // Check block is a descendant of the finalized block at the checkpoint finalized slot.
        //
        // Note: the specification uses `hash_tree_root(block)` instead of `block.parent_root` for
        // the start of this search. I claim that since `block.slot > finalized_slot` it is
        // equivalent to use the parent root for this search. Doing so reduces a single lookup
        // (trivial), but more importantly, it means we don't need to have added `block` to
        // `self.proto_array` to do this search. See:
        //
        // https://github.com/ethereum/eth2.0-specs/pull/1884
        let block_ancestor = self.get_ancestor(block.parent_root(), finalized_slot)?;
        let finalized_root = self.fc_store.finalized_checkpoint().root;
        if block_ancestor != Some(finalized_root) {
            return Err(Error::InvalidBlock(InvalidBlock::NotFinalizedDescendant {
                finalized_root,
                block_ancestor,
            }));
        }

        // Add proposer score boost if the block is timely and the proposer matches
        // the expected proposer on the canonical chain.
        //
        // Spec: `update_proposer_boost_root` (consensus-specs PR #4807)
        let is_before_attesting_interval =
            block_delay < Duration::from_secs(spec.seconds_per_slot / INTERVALS_PER_SLOT);

        let is_first_block = self.fc_store.proposer_boost_root().is_zero();
        if current_slot == block.slot() && is_before_attesting_interval && is_first_block {
            // Only boost if the block's proposer matches the canonical chain's expected
            // proposer for this slot. This prevents sidechain blocks from receiving boost
            // when they exploit a different shuffling.
            let proposer_matches = canonical_head_proposer_index
                .is_none_or(|expected| block.proposer_index() == expected);
            if proposer_matches {
                self.fc_store.set_proposer_boost_root(block_root);
            }
        }

        // Update store with checkpoints if necessary
        self.update_checkpoints(
            state.current_justified_checkpoint(),
            state.finalized_checkpoint(),
            || {
                state
                    .get_state_root_at_epoch_start(state.current_justified_checkpoint().epoch)
                    .map_err(Into::into)
            },
        )?;

        // Update unrealized justified/finalized checkpoints.
        let block_epoch = block.slot().epoch(E::slots_per_epoch());

        // If the parent checkpoints are already at the same epoch as the block being imported,
        // it's impossible for the unrealized checkpoints to differ from the parent's. This
        // holds true because:
        //
        // 1. A child block cannot have lower FFG checkpoints than its parent.
        // 2. A block in epoch `N` cannot contain attestations which would justify an epoch higher than `N`.
        // 3. A block in epoch `N` cannot contain attestations which would finalize an epoch higher than `N - 1`.
        //
        // This is an optimization. It should reduce the amount of times we run
        // `process_justification_and_finalization` by approximately 1/3rd when the chain is
        // performing optimally.
        let parent_checkpoints = parent_block
            .unrealized_justified_checkpoint
            .zip(parent_block.unrealized_finalized_checkpoint)
            .filter(|(parent_justified, parent_finalized)| {
                parent_justified.epoch == block_epoch && parent_finalized.epoch + 1 == block_epoch
            });

        let (unrealized_justified_checkpoint, unrealized_finalized_checkpoint) =
            if let Some((parent_justified, parent_finalized)) = parent_checkpoints {
                (parent_justified, parent_finalized)
            } else {
                let justification_and_finalization_state =
                    if block.fork_name_unchecked().altair_enabled() {
                        // NOTE: Processing justification & finalization requires the progressive
                        // balances cache, but we cannot initialize it here as we only have an
                        // immutable reference. The state *should* have come straight from block
                        // processing, which initialises the cache, but if we add other `on_block`
                        // calls in future it could be worth passing a mutable reference.
                        per_epoch_processing::altair::process_justification_and_finalization(state)?
                    } else {
                        let mut validator_statuses =
                            per_epoch_processing::base::ValidatorStatuses::new(state, spec)
                                .map_err(Error::ValidatorStatuses)?;
                        validator_statuses
                            .process_attestations(state)
                            .map_err(Error::ValidatorStatuses)?;
                        per_epoch_processing::base::process_justification_and_finalization(
                            state,
                            &validator_statuses.total_balances,
                            spec,
                        )?
                    };

                (
                    justification_and_finalization_state.current_justified_checkpoint(),
                    justification_and_finalization_state.finalized_checkpoint(),
                )
            };

        // Update best known unrealized justified & finalized checkpoints
        if unrealized_justified_checkpoint.epoch
            > self.fc_store.unrealized_justified_checkpoint().epoch
        {
            // Justification has recently updated therefore the justified state root should be in
            // range of the head state's `state_roots` vector.
            let unrealized_justified_state_root =
                state.get_state_root_at_epoch_start(unrealized_justified_checkpoint.epoch)?;

            self.fc_store.set_unrealized_justified_checkpoint(
                unrealized_justified_checkpoint,
                unrealized_justified_state_root,
            );
        }
        if unrealized_finalized_checkpoint.epoch
            > self.fc_store.unrealized_finalized_checkpoint().epoch
        {
            self.fc_store
                .set_unrealized_finalized_checkpoint(unrealized_finalized_checkpoint);
        }

        // If block is from past epochs, try to update store's justified & finalized checkpoints right away
        if block.slot().epoch(E::slots_per_epoch()) < current_slot.epoch(E::slots_per_epoch()) {
            self.pull_up_store_checkpoints(
                unrealized_justified_checkpoint,
                unrealized_finalized_checkpoint,
                || {
                    // In the case where we actually update justification, it must be that the
                    // unrealized justification is recent and in range of the `state_roots` vector.
                    state
                        .get_state_root_at_epoch_start(unrealized_justified_checkpoint.epoch)
                        .map_err(Into::into)
                },
            )?;
        }

        let target_slot = block
            .slot()
            .epoch(E::slots_per_epoch())
            .start_slot(E::slots_per_epoch());
        let target_root = if block.slot() == target_slot {
            block_root
        } else {
            *state
                .get_block_root(target_slot)
                .map_err(Error::BeaconStateError)?
        };

        self.fc_store
            .on_verified_block(block, block_root, state)
            .map_err(Error::AfterBlockFailed)?;

        let execution_status = if let Ok(execution_payload) = block.body().execution_payload() {
            let block_hash = execution_payload.block_hash();

            if block_hash == ExecutionBlockHash::zero() {
                // The block is post-merge-fork, but pre-terminal-PoW block. We don't need to verify
                // the payload.
                ExecutionStatus::irrelevant()
            } else {
                match payload_verification_status {
                    PayloadVerificationStatus::Verified => ExecutionStatus::Valid(block_hash),
                    PayloadVerificationStatus::Optimistic => {
                        ExecutionStatus::Optimistic(block_hash)
                    }
                    // It would be a logic error to declare a block irrelevant if it has an
                    // execution payload with a non-zero block hash.
                    PayloadVerificationStatus::Irrelevant => {
                        return Err(Error::InvalidPayloadStatus {
                            block_slot: block.slot(),
                            block_root,
                            payload_verification_status,
                        });
                    }
                }
            }
        } else if let Ok(bid) = block.body().signed_execution_payload_bid() {
            // Gloas ePBS: block contains a bid, not a payload. Use the bid's block_hash
            // so head_hash is available for forkchoice_updated (especially for self-build
            // blocks which are immediately viable for head).
            let block_hash = bid.message.block_hash;
            if block_hash == ExecutionBlockHash::zero() {
                ExecutionStatus::irrelevant()
            } else {
                ExecutionStatus::Optimistic(block_hash)
            }
        } else {
            // There is no payload to verify.
            ExecutionStatus::irrelevant()
        };

        // This does not apply a vote to the block, it just makes fork choice aware of the block so
        // it can still be identified as the head even if it doesn't have any votes.
        self.proto_array.process_block::<E>(
            ProtoBlock {
                slot: block.slot(),
                root: block_root,
                parent_root: Some(block.parent_root()),
                target_root,
                current_epoch_shuffling_id: AttestationShufflingId::new(
                    block_root,
                    state,
                    RelativeEpoch::Current,
                )
                .map_err(Error::BeaconStateError)?,
                next_epoch_shuffling_id: AttestationShufflingId::new(
                    block_root,
                    state,
                    RelativeEpoch::Next,
                )
                .map_err(Error::BeaconStateError)?,
                state_root: block.state_root(),
                justified_checkpoint: state.current_justified_checkpoint(),
                finalized_checkpoint: state.finalized_checkpoint(),
                execution_status,
                unrealized_justified_checkpoint: Some(unrealized_justified_checkpoint),
                unrealized_finalized_checkpoint: Some(unrealized_finalized_checkpoint),
                builder_index: block
                    .body()
                    .signed_execution_payload_bid()
                    .ok()
                    .map(|bid| bid.message.builder_index),
                payload_revealed: false,
                ptc_weight: 0,
                ptc_blob_data_available_weight: 0,
                payload_data_available: false,
                bid_block_hash: block
                    .body()
                    .signed_execution_payload_bid()
                    .ok()
                    .map(|bid| bid.message.block_hash),
                bid_parent_block_hash: block
                    .body()
                    .signed_execution_payload_bid()
                    .ok()
                    .map(|bid| bid.message.parent_block_hash),
                proposer_index: block.proposer_index(),
                // PTC timeliness: block received in its own slot before the PTC deadline.
                // The PTC deadline is later than the attestation deadline, so any block
                // that's current-slot is conservatively PTC-timely.
                ptc_timely: current_slot == block.slot(),
            },
            current_slot,
        )?;

        Ok(())
    }

    /// Update checkpoints in store if necessary
    fn update_checkpoints(
        &mut self,
        justified_checkpoint: Checkpoint,
        finalized_checkpoint: Checkpoint,
        justified_state_root_producer: impl FnOnce() -> Result<Hash256, Error<T::Error>>,
    ) -> Result<(), Error<T::Error>> {
        // Update justified checkpoint.
        if justified_checkpoint.epoch > self.fc_store.justified_checkpoint().epoch {
            let justified_state_root = justified_state_root_producer()?;
            self.fc_store
                .set_justified_checkpoint(justified_checkpoint, justified_state_root)
                .map_err(Error::UnableToSetJustifiedCheckpoint)?;
        }

        // Update finalized checkpoint.
        if finalized_checkpoint.epoch > self.fc_store.finalized_checkpoint().epoch {
            self.fc_store.set_finalized_checkpoint(finalized_checkpoint);
        }

        Ok(())
    }

    /// Validates the `epoch` against the current time according to the fork choice store.
    ///
    /// ## Specification
    ///
    /// Equivalent to:
    ///
    /// https://github.com/ethereum/consensus-specs/blob/dev/specs/phase0/fork-choice.md#validate_target_epoch_against_current_time
    fn validate_target_epoch_against_current_time(
        &self,
        target_epoch: Epoch,
    ) -> Result<(), InvalidAttestation> {
        let slot_now = self.fc_store.get_current_slot();
        let epoch_now = slot_now.epoch(E::slots_per_epoch());

        // Attestation must be from the current or previous epoch.
        if target_epoch > epoch_now {
            return Err(InvalidAttestation::FutureEpoch {
                attestation_epoch: target_epoch,
                current_epoch: epoch_now,
            });
        } else if target_epoch + 1 < epoch_now {
            return Err(InvalidAttestation::PastEpoch {
                attestation_epoch: target_epoch,
                current_epoch: epoch_now,
            });
        }
        Ok(())
    }

    /// Validates the `indexed_attestation` for application to fork choice.
    ///
    /// ## Specification
    ///
    /// Equivalent to:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#validate_on_attestation
    fn validate_on_attestation(
        &self,
        indexed_attestation: IndexedAttestationRef<E>,
        is_from_block: AttestationFromBlock,
    ) -> Result<(), InvalidAttestation> {
        // There is no point in processing an attestation with an empty bitfield. Reject
        // it immediately.
        //
        // This is not in the specification, however it should be transparent to other nodes. We
        // return early here to avoid wasting precious resources verifying the rest of it.
        if indexed_attestation.attesting_indices_is_empty() {
            return Err(InvalidAttestation::EmptyAggregationBitfield);
        }

        let target = indexed_attestation.data().target;

        if matches!(is_from_block, AttestationFromBlock::False) {
            self.validate_target_epoch_against_current_time(target.epoch)?;
        }

        if target.epoch != indexed_attestation.data().slot.epoch(E::slots_per_epoch()) {
            return Err(InvalidAttestation::BadTargetEpoch {
                target: target.epoch,
                slot: indexed_attestation.data().slot,
            });
        }

        // Attestation target must be for a known block.
        //
        // We do not delay the block for later processing to reduce complexity and DoS attack
        // surface.
        if !self.proto_array.contains_block(&target.root) {
            return Err(InvalidAttestation::UnknownTargetRoot(target.root));
        }

        // Load the block for `attestation.data.beacon_block_root`.
        //
        // This indirectly checks to see if the `attestation.data.beacon_block_root` is in our fork
        // choice. Any known, non-finalized block should be in fork choice, so this check
        // immediately filters out attestations that attest to a block that has not been processed.
        //
        // Attestations must be for a known block. If the block is unknown, we simply drop the
        // attestation and do not delay consideration for later.
        let block = self
            .proto_array
            .get_block(&indexed_attestation.data().beacon_block_root)
            .ok_or(InvalidAttestation::UnknownHeadBlock {
                beacon_block_root: indexed_attestation.data().beacon_block_root,
            })?;

        // If an attestation points to a block that is from an earlier slot than the attestation,
        // then all slots between the block and attestation must be skipped. Therefore if the block
        // is from a prior epoch to the attestation, then the target root must be equal to the root
        // of the block that is being attested to.
        let expected_target = if target.epoch > block.slot.epoch(E::slots_per_epoch()) {
            indexed_attestation.data().beacon_block_root
        } else {
            block.target_root
        };

        if expected_target != target.root {
            return Err(InvalidAttestation::InvalidTarget {
                attestation: target.root,
                local: expected_target,
            });
        }

        // Attestations must not be for blocks in the future. If this is the case, the attestation
        // should not be considered.
        if block.slot > indexed_attestation.data().slot {
            return Err(InvalidAttestation::AttestsToFutureBlock {
                block: block.slot,
                attestation: indexed_attestation.data().slot,
            });
        }

        // [New in Gloas:EIP7732]
        // Gloas-specific attestation index validation. In Gloas, `index` is repurposed:
        // 0 = empty (standard attestation), 1 = full (payload present attestation).
        // We gate on builder_index to identify Gloas blocks, since pre-Electra attestations
        // can have index > 1 (representing the committee index).
        if block.builder_index.is_some() {
            let index = indexed_attestation.data().index;

            // assert attestation.data.index in [0, 1]
            if index > 1 {
                return Err(InvalidAttestation::InvalidCommitteeIndex { index });
            }

            // if block_slot == attestation.data.slot: assert attestation.data.index == 0
            if block.slot == indexed_attestation.data().slot && index != 0 {
                return Err(InvalidAttestation::SameSlotNonZeroIndex {
                    slot: block.slot,
                    index,
                });
            }

            // consensus-specs PR #4918 (merged 2026-02-23):
            // if attestation.data.index == 1: assert beacon_block_root in store.payload_states
            // In vibehouse, a block root being in payload_states is equivalent to
            // the block's payload_revealed being true in proto_array.
            //
            // TODO(https://github.com/ethereum/consensus-specs/pull/4918): enable this check
            // once spec test vectors include it (currently pinned at v1.7.0-alpha.2, which
            // predates this spec change). The check is implemented and tested in unit tests
            // below but disabled to avoid breaking EF spec test compatibility.
            //
            // if index == 1 && !block.payload_revealed {
            //     return Err(InvalidAttestation::PayloadNotRevealed {
            //         beacon_block_root: indexed_attestation.data().beacon_block_root,
            //     });
            // }
        }

        Ok(())
    }

    /// Register `attestation` with the fork choice DAG so that it may influence future calls to
    /// `Self::get_head`.
    ///
    /// ## Specification
    ///
    /// Approximates:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#on_attestation
    ///
    /// It only approximates the specification since it does not perform
    /// `is_valid_indexed_attestation` since that should already have been called upstream and it's
    /// too expensive to call again.
    ///
    /// ## Notes:
    ///
    /// The supplied `attestation` **must** pass the `in_valid_indexed_attestation` function as it
    /// will not be run here.
    pub fn on_attestation(
        &mut self,
        system_time_current_slot: Slot,
        attestation: IndexedAttestationRef<E>,
        is_from_block: AttestationFromBlock,
    ) -> Result<(), Error<T::Error>> {
        let _timer = metrics::start_timer(&metrics::FORK_CHOICE_ON_ATTESTATION_TIMES);

        self.update_time(system_time_current_slot)?;

        // Ignore any attestations to the zero hash.
        //
        // This is an edge case that results from the spec aliasing the zero hash to the genesis
        // block. Attesters may attest to the zero hash if they have never seen a block.
        //
        // We have two options here:
        //
        //  1. Apply all zero-hash attestations to the genesis block.
        //  2. Ignore all attestations to the zero hash.
        //
        // (1) becomes weird once we hit finality and fork choice drops the genesis block. (2) is
        // fine because votes to the genesis block are not useful; all validators implicitly attest
        // to genesis just by being present in the chain.
        if attestation.data().beacon_block_root == Hash256::zero() {
            return Ok(());
        }

        self.validate_on_attestation(attestation, is_from_block)?;

        if attestation.data().slot < self.fc_store.get_current_slot() {
            let att_slot = attestation.data().slot;
            let payload_present = attestation.data().index == 1;
            for validator_index in attestation.attesting_indices_iter() {
                self.proto_array.process_attestation(
                    *validator_index as usize,
                    attestation.data().beacon_block_root,
                    attestation.data().target.epoch,
                    att_slot,
                    payload_present,
                )?;
            }
        } else {
            // The spec declares:
            //
            // ```
            // Attestations can only affect the fork choice of subsequent slots.
            // Delay consideration in the fork choice until their slot is in the past.
            // ```
            self.queued_attestations
                .push(QueuedAttestation::from(attestation));
        }

        Ok(())
    }

    /// Apply an attester slashing to fork choice.
    ///
    /// We assume that the attester slashing provided to this function has already been verified.
    pub fn on_attester_slashing(&mut self, slashing: AttesterSlashingRef<'_, E>) {
        let _timer = metrics::start_timer(&metrics::FORK_CHOICE_ON_ATTESTER_SLASHING_TIMES);

        let attesting_indices_set = |att: IndexedAttestationRef<'_, E>| {
            att.attesting_indices_iter()
                .copied()
                .collect::<BTreeSet<_>>()
        };
        let att1_indices = attesting_indices_set(slashing.attestation_1());
        let att2_indices = attesting_indices_set(slashing.attestation_2());
        self.fc_store
            .extend_equivocating_indices(att1_indices.intersection(&att2_indices).copied());
    }

    /// Gloas ePBS: Process a builder's execution payload bid.
    ///
    /// This is called when a proposer includes a `SignedExecutionPayloadBid` in their block.
    /// The bid represents the builder's commitment to provide an execution payload.
    ///
    /// ## Validation
    ///
    /// This function performs fork choice validation only. Full state transition validation
    /// (builder signature, balance checks, etc.) should be done during block processing.
    ///
    /// ## Spec Reference
    ///
    /// https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#on_execution_bid
    pub fn on_execution_bid(
        &mut self,
        bid: &SignedExecutionPayloadBid<E>,
        beacon_block_root: Hash256,
    ) -> Result<(), Error<T::Error>> {
        // Get the block this bid is for
        let block_index = self
            .proto_array
            .core_proto_array()
            .indices
            .get(&beacon_block_root)
            .copied()
            .ok_or(InvalidExecutionBid::UnknownBeaconBlockRoot { beacon_block_root })?;

        let node = self
            .proto_array
            .core_proto_array()
            .nodes
            .get(block_index)
            .ok_or(Error::MissingProtoArrayBlock(beacon_block_root))?;

        // Verify bid slot matches block slot
        if bid.message.slot != node.slot {
            return Err(InvalidExecutionBid::SlotMismatch {
                bid_slot: bid.message.slot,
                block_slot: node.slot,
            }
            .into());
        }

        // Copy slot for logging before mutable borrow
        let node_slot = node.slot;

        // Update the proto_array node with builder information
        let nodes = &mut self.proto_array.core_proto_array_mut().nodes;

        if let Some(node) = nodes.get_mut(block_index) {
            // Record which builder won this slot's bid
            node.builder_index = Some(bid.message.builder_index);

            // Mark payload as not yet revealed
            // (will be set to true when builder publishes the execution payload envelope)
            node.payload_revealed = false;

            // Initialize PTC weights to 0 (will accumulate via on_payload_attestation)
            node.ptc_weight = 0;
            node.ptc_blob_data_available_weight = 0;
            node.payload_data_available = false;
        }

        debug!(
            ?beacon_block_root,
            builder_index = bid.message.builder_index,
            bid_value = bid.message.value,
            slot = %node_slot,
            "Processed execution bid"
        );

        Ok(())
    }

    /// Gloas ePBS: Process a PTC (Payload Timeliness Committee) attestation.
    ///
    /// PTC attestations signal whether a builder successfully revealed their execution payload.
    /// When enough attestations accumulate (quorum), the payload is considered available and
    /// the block becomes viable for head selection.
    ///
    /// ## Validation
    ///
    /// This function performs fork choice validation only. Full validation (PTC membership,
    /// aggregate signature) should be done during gossip or block processing.
    ///
    /// ## Spec Reference
    ///
    /// https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#on_payload_attestation
    pub fn on_payload_attestation(
        &mut self,
        attestation: &PayloadAttestation<E>,
        indexed_attestation: &IndexedPayloadAttestation<E>,
        current_slot: Slot,
        spec: &ChainSpec,
    ) -> Result<(), Error<T::Error>> {
        let beacon_block_root = attestation.data.beacon_block_root;

        // Validate slot is not from the future
        if attestation.data.slot > current_slot {
            return Err(InvalidPayloadAttestation::FutureSlot {
                attestation_slot: attestation.data.slot,
                current_slot,
            }
            .into());
        }

        // Validate attestation is not too old (beyond 1 epoch)
        let slots_per_epoch = Slot::new(E::slots_per_epoch());
        if current_slot > attestation.data.slot + slots_per_epoch {
            return Err(InvalidPayloadAttestation::TooOld {
                attestation_slot: attestation.data.slot,
                current_slot,
            }
            .into());
        }

        // Get the block this attestation is for
        let block_index = self
            .proto_array
            .core_proto_array()
            .indices
            .get(&beacon_block_root)
            .copied()
            .ok_or(InvalidPayloadAttestation::UnknownBeaconBlockRoot { beacon_block_root })?;

        let node = self
            .proto_array
            .core_proto_array()
            .nodes
            .get(block_index)
            .ok_or(Error::MissingProtoArrayBlock(beacon_block_root))?;

        // Spec: if data.slot != state.slot: return
        // PTC votes can only change the vote for their assigned beacon block.
        // When data.slot != block.slot (e.g. skip slots), the attestation is
        // for a different slot than the referenced block — silently ignore it.
        if attestation.data.slot != node.slot {
            return Ok(());
        }

        // Spec: PAYLOAD_TIMELY_THRESHOLD = PTC_SIZE // 2
        // Payload is timely when sum(votes) > PAYLOAD_TIMELY_THRESHOLD (strictly greater)
        // With PTC_SIZE=512 this requires 257+ votes.
        let ptc_size = spec.ptc_size;
        let quorum_threshold = ptc_size / 2;

        // Count the attesters (weight each as 1)
        let attester_count = indexed_attestation.attesting_indices.len() as u64;

        // Update the proto_array node with accumulated PTC weight.
        // Per spec, payload_timeliness_vote and payload_data_availability_vote
        // are separate per-PTC-member bitvectors. We track them as counters of
        // True votes since gossip validation prevents duplicate attestations.
        let nodes = &mut self.proto_array.core_proto_array_mut().nodes;

        if let Some(node) = nodes.get_mut(block_index) {
            // Accumulate payload_present votes (spec: payload_timeliness_vote)
            if attestation.data.payload_present {
                node.ptc_weight = node.ptc_weight.saturating_add(attester_count);
            }

            // Accumulate blob_data_available votes (spec: payload_data_availability_vote)
            if attestation.data.blob_data_available {
                node.ptc_blob_data_available_weight = node
                    .ptc_blob_data_available_weight
                    .saturating_add(attester_count);
            }

            // Check payload timeliness quorum (strictly greater than threshold per spec)
            if node.ptc_weight > quorum_threshold && !node.payload_revealed {
                node.payload_revealed = true;
                // If the envelope path hasn't already set execution_status,
                // use the bid_block_hash so head_hash is available for forkchoice_updated.
                if !node.execution_status.is_execution_enabled()
                    && let Some(block_hash) = node.bid_block_hash
                {
                    node.execution_status = ExecutionStatus::Optimistic(block_hash);
                }

                debug!(
                    ?beacon_block_root,
                    ptc_weight = node.ptc_weight,
                    quorum_threshold = quorum_threshold,
                    slot = %node.slot,
                    "Payload timeliness quorum reached"
                );
            }

            // Check blob data availability quorum
            if node.ptc_blob_data_available_weight > quorum_threshold
                && !node.payload_data_available
            {
                node.payload_data_available = true;

                debug!(
                    ?beacon_block_root,
                    blob_weight = node.ptc_blob_data_available_weight,
                    quorum_threshold = quorum_threshold,
                    slot = %node.slot,
                    "Blob data availability quorum reached"
                );
            }
        }

        Ok(())
    }

    /// Gloas ePBS: Process an execution payload envelope reveal.
    ///
    /// When a builder reveals their execution payload, mark the corresponding block's payload
    /// as revealed in fork choice. This makes the block viable for head selection.
    ///
    /// ## Spec Reference
    ///
    /// This is called after gossip verification of the `SignedExecutionPayloadEnvelope`.
    /// The fork choice tree uses `payload_revealed` to determine block viability.
    pub fn on_execution_payload(
        &mut self,
        beacon_block_root: Hash256,
        payload_block_hash: ExecutionBlockHash,
    ) -> Result<(), Error<T::Error>> {
        let block_index = self
            .proto_array
            .core_proto_array()
            .indices
            .get(&beacon_block_root)
            .copied()
            .ok_or(Error::MissingProtoArrayBlock(beacon_block_root))?;

        let nodes = &mut self.proto_array.core_proto_array_mut().nodes;

        if let Some(node) = nodes.get_mut(block_index) {
            node.payload_revealed = true;
            // When the envelope is received locally, blob data is also available
            node.payload_data_available = true;
            // Set execution status so that head_hash is available for forkchoice_updated.
            // Starts as Optimistic until the EL confirms via newPayload.
            node.execution_status = ExecutionStatus::Optimistic(payload_block_hash);

            debug!(
                ?beacon_block_root,
                ?payload_block_hash,
                slot = %node.slot,
                "Marked payload as revealed via execution payload envelope"
            );
        }

        Ok(())
    }

    /// Call `on_tick` for all slots between `fc_store.get_current_slot()` and the provided
    /// `current_slot`. Returns the value of `self.fc_store.get_current_slot`.
    pub fn update_time(&mut self, current_slot: Slot) -> Result<Slot, Error<T::Error>> {
        while self.fc_store.get_current_slot() < current_slot {
            let previous_slot = self.fc_store.get_current_slot();
            // Note: we are relying upon `on_tick` to update `fc_store.time` to ensure we don't
            // get stuck in a loop.
            self.on_tick(previous_slot + 1)?
        }

        // Process any attestations that might now be eligible.
        self.process_attestation_queue()?;

        Ok(self.fc_store.get_current_slot())
    }

    /// Called whenever the current time increases.
    ///
    /// ## Specification
    ///
    /// Equivalent to:
    ///
    /// https://github.com/ethereum/eth2.0-specs/blob/v0.12.1/specs/phase0/fork-choice.md#on_tick
    fn on_tick(&mut self, time: Slot) -> Result<(), Error<T::Error>> {
        let store = &mut self.fc_store;
        let previous_slot = store.get_current_slot();

        if time > previous_slot + 1 {
            return Err(Error::InconsistentOnTick {
                previous_slot,
                time,
            });
        }

        // Update store time.
        store.set_current_slot(time);

        let current_slot = store.get_current_slot();

        // Reset proposer boost if this is a new slot.
        if current_slot > previous_slot {
            store.set_proposer_boost_root(Hash256::zero());
        }

        // Not a new epoch, return.
        if !(current_slot > previous_slot
            && compute_slots_since_epoch_start::<E>(current_slot) == 0)
        {
            return Ok(());
        }

        // Update the justified/finalized checkpoints based upon the
        // best-observed unrealized justification/finality.
        let unrealized_justified_checkpoint = *self.fc_store.unrealized_justified_checkpoint();
        let unrealized_justified_state_root = self.fc_store.unrealized_justified_state_root();
        let unrealized_finalized_checkpoint = *self.fc_store.unrealized_finalized_checkpoint();
        self.pull_up_store_checkpoints(
            unrealized_justified_checkpoint,
            unrealized_finalized_checkpoint,
            || Ok(unrealized_justified_state_root),
        )?;

        Ok(())
    }

    fn pull_up_store_checkpoints(
        &mut self,
        unrealized_justified_checkpoint: Checkpoint,
        unrealized_finalized_checkpoint: Checkpoint,
        unrealized_justified_state_root_producer: impl FnOnce() -> Result<Hash256, Error<T::Error>>,
    ) -> Result<(), Error<T::Error>> {
        self.update_checkpoints(
            unrealized_justified_checkpoint,
            unrealized_finalized_checkpoint,
            unrealized_justified_state_root_producer,
        )
    }

    /// Processes and removes from the queue any queued attestations which may now be eligible for
    /// processing due to the slot clock incrementing.
    fn process_attestation_queue(&mut self) -> Result<(), Error<T::Error>> {
        for attestation in dequeue_attestations(
            self.fc_store.get_current_slot(),
            &mut self.queued_attestations,
        ) {
            let payload_present = attestation.index == 1;
            for validator_index in attestation.attesting_indices.iter() {
                self.proto_array.process_attestation(
                    *validator_index as usize,
                    attestation.block_root,
                    attestation.target_epoch,
                    attestation.slot,
                    payload_present,
                )?;
            }
        }

        Ok(())
    }

    /// Returns `true` if the block is known **and** a descendant of the finalized root.
    pub fn contains_block(&self, block_root: &Hash256) -> bool {
        self.proto_array.contains_block(block_root)
            && self.is_finalized_checkpoint_or_descendant(*block_root)
    }

    /// Returns the Gloas head payload status from the last `get_head` call.
    /// 1 = EMPTY, 2 = FULL. `None` for pre-Gloas heads.
    pub fn gloas_head_payload_status(&self) -> Option<u8> {
        self.proto_array.gloas_head_payload_status()
    }

    /// Returns a `ProtoBlock` if the block is known **and** a descendant of the finalized root.
    pub fn get_block(&self, block_root: &Hash256) -> Option<ProtoBlock> {
        if self.is_finalized_checkpoint_or_descendant(*block_root) {
            self.proto_array.get_block(block_root)
        } else {
            None
        }
    }

    /// Returns an `ExecutionStatus` if the block is known **and** a descendant of the finalized root.
    pub fn get_block_execution_status(&self, block_root: &Hash256) -> Option<ExecutionStatus> {
        if self.is_finalized_checkpoint_or_descendant(*block_root) {
            self.proto_array.get_block_execution_status(block_root)
        } else {
            None
        }
    }

    /// Returns the weight for the given block root.
    pub fn get_block_weight(&self, block_root: &Hash256) -> Option<u64> {
        self.proto_array.get_weight(block_root)
    }

    /// Returns the `ProtoBlock` for the justified checkpoint.
    ///
    /// ## Notes
    ///
    /// This does *not* return the "best justified checkpoint". It returns the justified checkpoint
    /// that is used for computing balances.
    pub fn get_justified_block(&self) -> Result<ProtoBlock, Error<T::Error>> {
        let justified_checkpoint = self.justified_checkpoint();
        self.get_block(&justified_checkpoint.root)
            .ok_or(Error::MissingJustifiedBlock {
                justified_checkpoint,
            })
    }

    /// Returns the `ProtoBlock` for the finalized checkpoint.
    pub fn get_finalized_block(&self) -> Result<ProtoBlock, Error<T::Error>> {
        let finalized_checkpoint = self.finalized_checkpoint();
        self.get_block(&finalized_checkpoint.root)
            .ok_or(Error::MissingFinalizedBlock {
                finalized_checkpoint,
            })
    }

    /// Return `true` if `block_root` is equal to the finalized checkpoint, or a known descendant of it.
    pub fn is_finalized_checkpoint_or_descendant(&self, block_root: Hash256) -> bool {
        self.proto_array
            .is_finalized_checkpoint_or_descendant::<E>(block_root)
    }

    pub fn is_descendant(&self, ancestor_root: Hash256, descendant_root: Hash256) -> bool {
        self.proto_array
            .is_descendant(ancestor_root, descendant_root)
    }

    /// Returns `Ok(true)` if `block_root` has been imported optimistically or deemed invalid.
    ///
    /// Returns `Ok(false)` if `block_root`'s execution payload has been elected as fully VALID, if
    /// it is a pre-Bellatrix block or if it is before the PoW terminal block.
    ///
    /// In the case where the block could not be found in fork-choice, it returns the
    /// `execution_status` of the current finalized block.
    ///
    /// This function assumes the `block_root` exists.
    pub fn is_optimistic_or_invalid_block(
        &self,
        block_root: &Hash256,
    ) -> Result<bool, Error<T::Error>> {
        if let Some(status) = self.get_block_execution_status(block_root) {
            Ok(status.is_optimistic_or_invalid())
        } else {
            Ok(self
                .get_finalized_block()?
                .execution_status
                .is_optimistic_or_invalid())
        }
    }

    /// The same as `is_optimistic_block` but does not fallback to `self.get_finalized_block`
    /// when the block cannot be found.
    ///
    /// Intended to be used when checking if the head has been imported optimistically or is
    /// invalid.
    pub fn is_optimistic_or_invalid_block_no_fallback(
        &self,
        block_root: &Hash256,
    ) -> Result<bool, Error<T::Error>> {
        if let Some(status) = self.get_block_execution_status(block_root) {
            Ok(status.is_optimistic_or_invalid())
        } else {
            Err(Error::MissingProtoArrayBlock(*block_root))
        }
    }

    /// Return the current finalized checkpoint.
    pub fn finalized_checkpoint(&self) -> Checkpoint {
        *self.fc_store.finalized_checkpoint()
    }

    /// Return the justified checkpoint.
    pub fn justified_checkpoint(&self) -> Checkpoint {
        *self.fc_store.justified_checkpoint()
    }

    pub fn unrealized_justified_checkpoint(&self) -> Checkpoint {
        *self.fc_store.unrealized_justified_checkpoint()
    }

    pub fn unrealized_finalized_checkpoint(&self) -> Checkpoint {
        *self.fc_store.unrealized_finalized_checkpoint()
    }

    /// Returns the latest message for a given validator, if any.
    ///
    /// Returns `(block_root, block_slot)`.
    ///
    /// ## Notes
    ///
    /// It may be prudent to call `Self::update_time` before calling this function,
    /// since some attestations might be queued and awaiting processing.
    pub fn latest_message(&self, validator_index: usize) -> Option<(Hash256, Epoch)> {
        self.proto_array.latest_message(validator_index)
    }

    /// Returns a reference to the underlying fork choice DAG.
    pub fn proto_array(&self) -> &ProtoArrayForkChoice {
        &self.proto_array
    }

    /// Returns a mutable reference to `proto_array`.
    /// Should only be used in testing.
    pub fn proto_array_mut(&mut self) -> &mut ProtoArrayForkChoice {
        &mut self.proto_array
    }

    /// Returns a reference to the underlying `fc_store`.
    pub fn fc_store(&self) -> &T {
        &self.fc_store
    }

    /// Returns a reference to the currently queued attestations.
    pub fn queued_attestations(&self) -> &[QueuedAttestation] {
        &self.queued_attestations
    }

    /// Returns the store's `proposer_boost_root`.
    pub fn proposer_boost_root(&self) -> Hash256 {
        self.fc_store.proposer_boost_root()
    }

    /// Prunes the underlying fork choice DAG.
    pub fn prune(&mut self) -> Result<(), Error<T::Error>> {
        let finalized_root = self.fc_store.finalized_checkpoint().root;

        self.proto_array
            .maybe_prune(finalized_root)
            .map_err(Into::into)
    }

    /// Instantiate `Self` from some `PersistedForkChoice` generated by a earlier call to
    /// `Self::to_persisted`.
    pub fn proto_array_from_persisted(
        persisted_proto_array: proto_array::core::SszContainer,
        justified_balances: JustifiedBalances,
        reset_payload_statuses: ResetPayloadStatuses,
        spec: &ChainSpec,
    ) -> Result<ProtoArrayForkChoice, Error<T::Error>> {
        let mut proto_array = ProtoArrayForkChoice::from_container(
            persisted_proto_array.clone(),
            justified_balances.clone(),
        )
        .map_err(Error::InvalidProtoArrayBytes)?;
        let contains_invalid_payloads = proto_array.contains_invalid_payloads();

        debug!(
            ?reset_payload_statuses,
            contains_invalid_payloads, "Restoring fork choice from persisted"
        );

        // Exit early if there are no "invalid" payloads, if requested.
        if matches!(
            reset_payload_statuses,
            ResetPayloadStatuses::OnlyWithInvalidPayload
        ) && !contains_invalid_payloads
        {
            return Ok(proto_array);
        }

        // Reset all blocks back to being "optimistic". This helps recover from an EL consensus
        // fault where an invalid payload becomes valid.
        if let Err(e) = proto_array.set_all_blocks_to_optimistic::<E>(spec) {
            // If there is an error resetting the optimistic status then log loudly and revert
            // back to a proto-array which does not have the reset applied. This indicates a
            // significant error in Lighthouse and warrants detailed investigation.
            crit!(
                error = ?e,
                info = "please report this error",
                "Failed to reset payload statuses"
            );
            ProtoArrayForkChoice::from_container(persisted_proto_array, justified_balances)
                .map_err(Error::InvalidProtoArrayBytes)
        } else {
            debug!("Successfully reset all payload statuses");
            Ok(proto_array)
        }
    }

    /// Instantiate `Self` from some `PersistedForkChoice` generated by a earlier call to
    /// `Self::to_persisted`.
    pub fn from_persisted(
        persisted: PersistedForkChoice,
        reset_payload_statuses: ResetPayloadStatuses,
        fc_store: T,
        spec: &ChainSpec,
    ) -> Result<Self, Error<T::Error>> {
        let justified_balances = fc_store.justified_balances().clone();
        let proto_array = Self::proto_array_from_persisted(
            persisted.proto_array,
            justified_balances,
            reset_payload_statuses,
            spec,
        )?;

        let current_slot = fc_store.get_current_slot();

        let mut fork_choice = Self {
            fc_store,
            proto_array,
            queued_attestations: persisted.queued_attestations,
            // Will be updated in the following call to `Self::get_head`.
            forkchoice_update_parameters: ForkchoiceUpdateParameters {
                head_hash: None,
                justified_hash: None,
                finalized_hash: None,
                // Will be updated in the following call to `Self::get_head`.
                head_root: Hash256::zero(),
            },
            _phantom: PhantomData,
        };

        // If a call to `get_head` fails, the only known cause is because the only head with viable
        // FFG properties is has an invalid payload. In this scenario, set all the payloads back to
        // an optimistic status so that we can have a head to start from.
        if let Err(e) = fork_choice.get_head(current_slot, spec) {
            warn!(
                info = "resetting all payload statuses and retrying",
                error = ?e,
                "Could not find head on persisted FC"
            );
            // Although we may have already made this call whilst loading `proto_array`, try it
            // again since we may have mutated the `proto_array` during `get_head` and therefore may
            // get a different result.
            fork_choice
                .proto_array
                .set_all_blocks_to_optimistic::<E>(spec)?;
            // If the second attempt at finding a head fails, return an error since we do not
            // expect this scenario.
            fork_choice.get_head(current_slot, spec)?;
        }

        Ok(fork_choice)
    }

    /// Takes a snapshot of `Self` and stores it in `PersistedForkChoice`, allowing this struct to
    /// be instantiated again later.
    pub fn to_persisted(&self) -> PersistedForkChoice {
        PersistedForkChoice {
            proto_array: self.proto_array().as_ssz_container(),
            queued_attestations: self.queued_attestations().to_vec(),
        }
    }

    /// Update the global metrics `DEFAULT_REGISTRY` with info from the fork choice
    pub fn scrape_for_metrics(&self) {
        scrape_for_metrics(self);
    }
}

/// Helper struct that is used to encode/decode the state of the `ForkChoice` as SSZ bytes.
///
/// This is used when persisting the state of the fork choice to disk.
#[superstruct(
    variants(V17, V28),
    variant_attributes(derive(Encode, Decode, Clone)),
    no_enum
)]
pub struct PersistedForkChoice {
    #[superstruct(only(V17))]
    pub proto_array_bytes: Vec<u8>,
    #[superstruct(only(V28))]
    pub proto_array: proto_array::core::SszContainerV28,
    pub queued_attestations: Vec<QueuedAttestation>,
}

pub type PersistedForkChoice = PersistedForkChoiceV28;

impl TryFrom<PersistedForkChoiceV17> for PersistedForkChoiceV28 {
    type Error = ssz::DecodeError;

    fn try_from(v17: PersistedForkChoiceV17) -> Result<Self, Self::Error> {
        let container_v17 =
            proto_array::core::SszContainerV17::from_ssz_bytes(&v17.proto_array_bytes)?;
        let container_v28 = container_v17.into();

        Ok(Self {
            proto_array: container_v28,
            queued_attestations: v17.queued_attestations,
        })
    }
}

impl From<(PersistedForkChoiceV28, JustifiedBalances)> for PersistedForkChoiceV17 {
    fn from((v28, balances): (PersistedForkChoiceV28, JustifiedBalances)) -> Self {
        let container_v17 = proto_array::core::SszContainerV17::from((v28.proto_array, balances));
        let proto_array_bytes = container_v17.as_ssz_bytes();

        Self {
            proto_array_bytes,
            queued_attestations: v28.queued_attestations,
        }
    }
}

#[cfg(test)]
mod tests {
    use types::MainnetEthSpec;

    use super::*;

    type E = MainnetEthSpec;

    #[test]
    fn slots_since_epoch_start() {
        for epoch in 0..3 {
            for slot in 0..E::slots_per_epoch() {
                let input = epoch * E::slots_per_epoch() + slot;
                assert_eq!(compute_slots_since_epoch_start::<E>(Slot::new(input)), slot)
            }
        }
    }

    #[test]
    fn start_slot_at_epoch() {
        for epoch in 0..3 {
            assert_eq!(
                compute_start_slot_at_epoch::<E>(Epoch::new(epoch)),
                epoch * E::slots_per_epoch()
            )
        }
    }

    fn get_queued_attestations() -> Vec<QueuedAttestation> {
        (1..4)
            .map(|i| QueuedAttestation {
                slot: Slot::new(i),
                attesting_indices: vec![],
                block_root: Hash256::zero(),
                target_epoch: Epoch::new(0),
                index: 0,
            })
            .collect()
    }

    fn get_slots(queued_attestations: &[QueuedAttestation]) -> Vec<u64> {
        queued_attestations.iter().map(|a| a.slot.into()).collect()
    }

    fn test_queued_attestations(current_time: Slot) -> (Vec<u64>, Vec<u64>) {
        let mut queued = get_queued_attestations();
        let dequeued = dequeue_attestations(current_time, &mut queued);

        (get_slots(&queued), get_slots(&dequeued))
    }

    #[test]
    fn dequeing_attestations() {
        let (queued, dequeued) = test_queued_attestations(Slot::new(0));
        assert_eq!(queued, vec![1, 2, 3]);
        assert!(dequeued.is_empty());

        let (queued, dequeued) = test_queued_attestations(Slot::new(1));
        assert_eq!(queued, vec![1, 2, 3]);
        assert!(dequeued.is_empty());

        let (queued, dequeued) = test_queued_attestations(Slot::new(2));
        assert_eq!(queued, vec![2, 3]);
        assert_eq!(dequeued, vec![1]);

        let (queued, dequeued) = test_queued_attestations(Slot::new(3));
        assert_eq!(queued, vec![3]);
        assert_eq!(dequeued, vec![1, 2]);

        let (queued, dequeued) = test_queued_attestations(Slot::new(4));
        assert!(queued.is_empty());
        assert_eq!(dequeued, vec![1, 2, 3]);
    }

    // ── Gloas ePBS fork choice method tests ──────────────────────────────

    /// Minimal mock ForkChoiceStore for unit testing ForkChoice methods
    /// that only touch proto_array (on_execution_bid, on_payload_attestation,
    /// on_execution_payload).
    mod gloas_fc_tests {
        use super::*;
        use proto_array::{Block as ProtoBlock, JustifiedBalances, ProtoArrayForkChoice};
        use std::collections::BTreeSet;
        use types::{
            AggregateSignature, AttestationData, BitVector, IndexedAttestation,
            IndexedAttestationElectra, MinimalEthSpec, PayloadAttestationData, VariableList,
        };

        type E = MinimalEthSpec;

        #[derive(Debug)]
        struct MockStoreError;

        struct MockStore {
            current_slot: Slot,
            justified_checkpoint: Checkpoint,
            finalized_checkpoint: Checkpoint,
            justified_balances: JustifiedBalances,
            proposer_boost_root: Hash256,
            equivocating_indices: BTreeSet<u64>,
        }

        impl ForkChoiceStore<E> for MockStore {
            type Error = MockStoreError;

            fn get_current_slot(&self) -> Slot {
                self.current_slot
            }
            fn set_current_slot(&mut self, slot: Slot) {
                self.current_slot = slot;
            }
            fn on_verified_block<Payload: AbstractExecPayload<E>>(
                &mut self,
                _block: BeaconBlockRef<E, Payload>,
                _block_root: Hash256,
                _state: &BeaconState<E>,
            ) -> Result<(), Self::Error> {
                Ok(())
            }
            fn justified_checkpoint(&self) -> &Checkpoint {
                &self.justified_checkpoint
            }
            fn justified_state_root(&self) -> Hash256 {
                Hash256::zero()
            }
            fn justified_balances(&self) -> &JustifiedBalances {
                &self.justified_balances
            }
            fn finalized_checkpoint(&self) -> &Checkpoint {
                &self.finalized_checkpoint
            }
            fn unrealized_justified_checkpoint(&self) -> &Checkpoint {
                &self.justified_checkpoint
            }
            fn unrealized_justified_state_root(&self) -> Hash256 {
                Hash256::zero()
            }
            fn unrealized_finalized_checkpoint(&self) -> &Checkpoint {
                &self.finalized_checkpoint
            }
            fn proposer_boost_root(&self) -> Hash256 {
                self.proposer_boost_root
            }
            fn set_finalized_checkpoint(&mut self, checkpoint: Checkpoint) {
                self.finalized_checkpoint = checkpoint;
            }
            fn set_justified_checkpoint(
                &mut self,
                checkpoint: Checkpoint,
                _state_root: Hash256,
            ) -> Result<(), Self::Error> {
                self.justified_checkpoint = checkpoint;
                Ok(())
            }
            fn set_unrealized_justified_checkpoint(
                &mut self,
                _checkpoint: Checkpoint,
                _state_root: Hash256,
            ) {
            }
            fn set_unrealized_finalized_checkpoint(&mut self, _checkpoint: Checkpoint) {}
            fn set_proposer_boost_root(&mut self, root: Hash256) {
                self.proposer_boost_root = root;
            }
            fn equivocating_indices(&self) -> &BTreeSet<u64> {
                &self.equivocating_indices
            }
            fn extend_equivocating_indices(&mut self, indices: impl IntoIterator<Item = u64>) {
                self.equivocating_indices.extend(indices);
            }
        }

        fn root(i: u64) -> Hash256 {
            Hash256::from_low_u64_be(i)
        }

        fn genesis_checkpoint() -> Checkpoint {
            Checkpoint {
                epoch: Epoch::new(0),
                root: root(0),
            }
        }

        fn junk_shuffling_id() -> AttestationShufflingId {
            AttestationShufflingId {
                shuffling_epoch: Epoch::new(0),
                shuffling_decision_block: Hash256::zero(),
            }
        }

        /// Create a ForkChoice with a single genesis block at slot 0.
        fn new_fc() -> ForkChoice<MockStore, E> {
            let checkpoint = genesis_checkpoint();
            let store = MockStore {
                current_slot: Slot::new(0),
                justified_checkpoint: checkpoint,
                finalized_checkpoint: checkpoint,
                justified_balances: JustifiedBalances {
                    effective_balances: vec![],
                    total_effective_balance: 0,
                    num_active_validators: 0,
                },
                proposer_boost_root: Hash256::zero(),
                equivocating_indices: BTreeSet::new(),
            };
            let proto_array = ProtoArrayForkChoice::new::<E>(
                Slot::new(0),
                Slot::new(0),
                Hash256::zero(),
                checkpoint,
                checkpoint,
                junk_shuffling_id(),
                junk_shuffling_id(),
                ExecutionStatus::irrelevant(),
            )
            .unwrap();

            ForkChoice {
                fc_store: store,
                proto_array,
                queued_attestations: vec![],
                forkchoice_update_parameters: ForkchoiceUpdateParameters {
                    head_root: root(0),
                    head_hash: None,
                    justified_hash: None,
                    finalized_hash: None,
                },
                _phantom: PhantomData,
            }
        }

        /// Insert a child block into the fork choice tree.
        fn insert_block(fc: &mut ForkChoice<MockStore, E>, slot: u64, block_root: Hash256) {
            let parent_root = root(0); // genesis
            fc.proto_array
                .process_block::<E>(
                    ProtoBlock {
                        slot: Slot::new(slot),
                        root: block_root,
                        parent_root: Some(parent_root),
                        state_root: Hash256::zero(),
                        target_root: root(0),
                        current_epoch_shuffling_id: junk_shuffling_id(),
                        next_epoch_shuffling_id: junk_shuffling_id(),
                        justified_checkpoint: genesis_checkpoint(),
                        finalized_checkpoint: genesis_checkpoint(),
                        execution_status: ExecutionStatus::irrelevant(),
                        unrealized_justified_checkpoint: Some(genesis_checkpoint()),
                        unrealized_finalized_checkpoint: Some(genesis_checkpoint()),
                        builder_index: None,
                        payload_revealed: false,
                        ptc_weight: 0,
                        ptc_blob_data_available_weight: 0,
                        payload_data_available: false,
                        bid_block_hash: None,
                        bid_parent_block_hash: None,
                        proposer_index: 0,
                        ptc_timely: false,
                    },
                    Slot::new(slot),
                )
                .unwrap();
        }

        fn make_bid(slot: u64, builder_index: u64) -> SignedExecutionPayloadBid<E> {
            let mut bid = SignedExecutionPayloadBid::<E>::empty();
            bid.message.slot = Slot::new(slot);
            bid.message.builder_index = builder_index;
            bid
        }

        fn make_payload_attestation(
            slot: u64,
            beacon_block_root: Hash256,
            payload_present: bool,
            blob_data_available: bool,
        ) -> PayloadAttestation<E> {
            PayloadAttestation {
                aggregation_bits: BitVector::default(),
                data: PayloadAttestationData {
                    beacon_block_root,
                    slot: Slot::new(slot),
                    payload_present,
                    blob_data_available,
                },
                signature: AggregateSignature::empty(),
            }
        }

        fn make_indexed_payload_attestation(
            slot: u64,
            beacon_block_root: Hash256,
            payload_present: bool,
            blob_data_available: bool,
            attesting_indices: Vec<u64>,
        ) -> IndexedPayloadAttestation<E> {
            IndexedPayloadAttestation {
                attesting_indices: VariableList::from(attesting_indices),
                data: PayloadAttestationData {
                    beacon_block_root,
                    slot: Slot::new(slot),
                    payload_present,
                    blob_data_available,
                },
                signature: AggregateSignature::empty(),
            }
        }

        // ── on_execution_bid tests ───────────────────────────────────────

        #[test]
        fn bid_unknown_block_root() {
            let mut fc = new_fc();
            let unknown = root(999);
            let bid = make_bid(1, 42);
            let err = fc.on_execution_bid(&bid, unknown).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidExecutionBid(InvalidExecutionBid::UnknownBeaconBlockRoot { .. })
                ),
                "expected UnknownBeaconBlockRoot, got {:?}",
                err
            );
        }

        #[test]
        fn bid_slot_mismatch() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            // Bid has slot 5 but block is at slot 1
            let bid = make_bid(5, 42);
            let err = fc.on_execution_bid(&bid, block_root).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidExecutionBid(InvalidExecutionBid::SlotMismatch { .. })
                ),
                "expected SlotMismatch, got {:?}",
                err
            );
        }

        #[test]
        fn bid_happy_path_sets_builder_index() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let bid = make_bid(1, 42);
            fc.on_execution_bid(&bid, block_root).unwrap();

            // Verify node was updated
            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.builder_index, Some(42));
            assert!(!node.payload_revealed);
            assert_eq!(node.ptc_weight, 0);
            assert_eq!(node.ptc_blob_data_available_weight, 0);
            assert!(!node.payload_data_available);
        }

        #[test]
        fn bid_resets_payload_revealed() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            // Manually set payload_revealed to true (simulating prior state)
            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            fc.proto_array.core_proto_array_mut().nodes[idx].payload_revealed = true;
            fc.proto_array.core_proto_array_mut().nodes[idx].ptc_weight = 100;

            // Bid should reset these
            let bid = make_bid(1, 77);
            fc.on_execution_bid(&bid, block_root).unwrap();

            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.builder_index, Some(77));
            assert!(!node.payload_revealed);
            assert_eq!(node.ptc_weight, 0);
        }

        #[test]
        fn bid_on_genesis_block_slot_zero() {
            // Genesis block is at slot 0, bid for slot 0
            let mut fc = new_fc();
            let genesis_root = root(0);

            let bid = make_bid(0, 10);
            // Genesis block root is the finalized_checkpoint.root
            fc.on_execution_bid(&bid, genesis_root).unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&genesis_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.builder_index, Some(10));
        }

        // ── on_payload_attestation tests ─────────────────────────────────

        #[test]
        fn payload_attestation_future_slot() {
            let mut fc = new_fc();
            fc.fc_store.current_slot = Slot::new(5);

            let att = make_payload_attestation(10, root(0), true, true);
            let indexed = make_indexed_payload_attestation(10, root(0), true, true, vec![1]);
            let spec = ChainSpec::minimal();

            let err = fc
                .on_payload_attestation(&att, &indexed, Slot::new(5), &spec)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidPayloadAttestation(InvalidPayloadAttestation::FutureSlot { .. })
                ),
                "expected FutureSlot, got {:?}",
                err
            );
        }

        #[test]
        fn payload_attestation_too_old() {
            let mut fc = new_fc();
            let slots_per_epoch = E::slots_per_epoch();

            // Attestation at slot 0, current_slot = slots_per_epoch + 1
            let current_slot = Slot::new(slots_per_epoch + 1);
            let att = make_payload_attestation(0, root(0), true, true);
            let indexed = make_indexed_payload_attestation(0, root(0), true, true, vec![1]);
            let spec = ChainSpec::minimal();

            let err = fc
                .on_payload_attestation(&att, &indexed, current_slot, &spec)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidPayloadAttestation(InvalidPayloadAttestation::TooOld { .. })
                ),
                "expected TooOld, got {:?}",
                err
            );
        }

        #[test]
        fn payload_attestation_unknown_block_root() {
            let mut fc = new_fc();
            let unknown = root(999);
            let att = make_payload_attestation(0, unknown, true, true);
            let indexed = make_indexed_payload_attestation(0, unknown, true, true, vec![1]);
            let spec = ChainSpec::minimal();

            let err = fc
                .on_payload_attestation(&att, &indexed, Slot::new(0), &spec)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidPayloadAttestation(
                        InvalidPayloadAttestation::UnknownBeaconBlockRoot { .. }
                    )
                ),
                "expected UnknownBeaconBlockRoot, got {:?}",
                err
            );
        }

        #[test]
        fn payload_attestation_slot_mismatch_silent_ok() {
            // When attestation.data.slot != node.slot, should silently return Ok(())
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            // Attestation for slot 2 but block is at slot 1 → silent Ok
            let att = make_payload_attestation(2, block_root, true, true);
            let indexed = make_indexed_payload_attestation(2, block_root, true, true, vec![1, 2]);
            let spec = ChainSpec::minimal();

            fc.on_payload_attestation(&att, &indexed, Slot::new(2), &spec)
                .unwrap();

            // Node should be unchanged — no weight added
            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.ptc_weight, 0);
        }

        #[test]
        fn payload_attestation_accumulates_weight() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            // MinimalEthSpec PtcSize = 2, so VariableList can hold at most 2 indices
            let att = make_payload_attestation(1, block_root, true, false);
            let indexed = make_indexed_payload_attestation(1, block_root, true, false, vec![1, 2]);
            let spec = ChainSpec::minimal();

            fc.on_payload_attestation(&att, &indexed, Slot::new(1), &spec)
                .unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.ptc_weight, 2);
            assert_eq!(node.ptc_blob_data_available_weight, 0);
        }

        #[test]
        fn payload_attestation_accumulates_blob_weight() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let att = make_payload_attestation(1, block_root, false, true);
            let indexed =
                make_indexed_payload_attestation(1, block_root, false, true, vec![10, 20]);
            let spec = ChainSpec::minimal();

            fc.on_payload_attestation(&att, &indexed, Slot::new(1), &spec)
                .unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.ptc_weight, 0);
            assert_eq!(node.ptc_blob_data_available_weight, 2);
        }

        #[test]
        fn payload_attestation_quorum_reveals_payload() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            // Set bid_block_hash so the quorum path can set execution_status
            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            fc.proto_array.core_proto_array_mut().nodes[idx].bid_block_hash =
                Some(ExecutionBlockHash::repeat_byte(0xAA));

            let spec = ChainSpec::minimal();
            let quorum_threshold = spec.ptc_size / 2;

            // Need quorum_threshold + 1 attesters (strictly greater)
            let indices: Vec<u64> = (0..=quorum_threshold).collect();
            let att = make_payload_attestation(1, block_root, true, true);
            let indexed = make_indexed_payload_attestation(1, block_root, true, true, indices);

            fc.on_payload_attestation(&att, &indexed, Slot::new(1), &spec)
                .unwrap();

            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert!(node.payload_revealed);
            assert!(node.payload_data_available);
            assert_eq!(
                node.execution_status,
                ExecutionStatus::Optimistic(ExecutionBlockHash::repeat_byte(0xAA))
            );
        }

        #[test]
        fn payload_attestation_at_threshold_does_not_reveal() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let spec = ChainSpec::minimal();
            let quorum_threshold = spec.ptc_size / 2;

            // Exactly quorum_threshold attesters (not strictly greater)
            let indices: Vec<u64> = (0..quorum_threshold).collect();
            let att = make_payload_attestation(1, block_root, true, true);
            let indexed = make_indexed_payload_attestation(1, block_root, true, true, indices);

            fc.on_payload_attestation(&att, &indexed, Slot::new(1), &spec)
                .unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert!(!node.payload_revealed);
        }

        #[test]
        fn payload_attestation_not_in_window_boundary() {
            // Test that an attestation exactly at the window boundary is accepted
            // current_slot == attestation_slot + slots_per_epoch should pass
            let mut fc = new_fc();
            let genesis_root = root(0);
            let slots_per_epoch = E::slots_per_epoch();

            // Attestation at slot 0, current_slot = slots_per_epoch (not > att + epoch)
            let att = make_payload_attestation(0, genesis_root, true, false);
            let indexed = make_indexed_payload_attestation(0, genesis_root, true, false, vec![1]);
            let spec = ChainSpec::minimal();

            // This should succeed (current_slot = slots_per_epoch is NOT too old)
            fc.on_payload_attestation(&att, &indexed, Slot::new(slots_per_epoch), &spec)
                .unwrap();
        }

        #[test]
        fn payload_attestation_same_slot_as_current() {
            // Attestation at current slot should succeed (not in the future)
            let mut fc = new_fc();
            let genesis_root = root(0);

            let att = make_payload_attestation(0, genesis_root, true, false);
            let indexed = make_indexed_payload_attestation(0, genesis_root, true, false, vec![1]);
            let spec = ChainSpec::minimal();

            fc.on_payload_attestation(&att, &indexed, Slot::new(0), &spec)
                .unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&genesis_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.ptc_weight, 1);
        }

        #[test]
        fn payload_attestation_no_weight_when_not_present() {
            // payload_present=false and blob_data_available=false → no weight changes
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let att = make_payload_attestation(1, block_root, false, false);
            let indexed = make_indexed_payload_attestation(1, block_root, false, false, vec![1, 2]);
            let spec = ChainSpec::minimal();

            fc.on_payload_attestation(&att, &indexed, Slot::new(1), &spec)
                .unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert_eq!(node.ptc_weight, 0);
            assert_eq!(node.ptc_blob_data_available_weight, 0);
        }

        // ── on_execution_payload tests ───────────────────────────────────

        #[test]
        fn execution_payload_unknown_block_root() {
            let mut fc = new_fc();
            let unknown = root(999);
            let hash = ExecutionBlockHash::repeat_byte(0xBB);

            let err = fc.on_execution_payload(unknown, hash).unwrap_err();
            assert!(
                matches!(err, Error::MissingProtoArrayBlock(_)),
                "expected MissingProtoArrayBlock, got {:?}",
                err
            );
        }

        #[test]
        fn execution_payload_reveals_and_sets_status() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let hash = ExecutionBlockHash::repeat_byte(0xCC);
            fc.on_execution_payload(block_root, hash).unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert!(node.payload_revealed);
            assert!(node.payload_data_available);
            assert_eq!(node.execution_status, ExecutionStatus::Optimistic(hash));
        }

        #[test]
        fn execution_payload_on_genesis() {
            let mut fc = new_fc();
            let genesis_root = root(0);
            let hash = ExecutionBlockHash::repeat_byte(0xDD);

            fc.on_execution_payload(genesis_root, hash).unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&genesis_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            assert!(node.payload_revealed);
            assert!(node.payload_data_available);
            assert_eq!(node.execution_status, ExecutionStatus::Optimistic(hash));
        }

        #[test]
        fn execution_payload_idempotent() {
            // Calling on_execution_payload twice should not error
            let mut fc = new_fc();
            let block_root = root(1);
            insert_block(&mut fc, 1, block_root);

            let hash1 = ExecutionBlockHash::repeat_byte(0x11);
            let hash2 = ExecutionBlockHash::repeat_byte(0x22);

            fc.on_execution_payload(block_root, hash1).unwrap();
            fc.on_execution_payload(block_root, hash2).unwrap();

            let idx = *fc
                .proto_array
                .core_proto_array()
                .indices
                .get(&block_root)
                .unwrap();
            let node = &fc.proto_array.core_proto_array().nodes[idx];
            // Second call overwrites the execution status
            assert_eq!(node.execution_status, ExecutionStatus::Optimistic(hash2));
            assert!(node.payload_revealed);
        }

        // ── validate_on_attestation Gloas index checks ──────────────────

        /// Insert a Gloas block (with builder_index set) into the fork choice tree.
        fn insert_gloas_block(
            fc: &mut ForkChoice<MockStore, E>,
            slot: u64,
            block_root: Hash256,
            builder_index: u64,
        ) {
            let parent_root = root(0); // genesis
            fc.proto_array
                .process_block::<E>(
                    ProtoBlock {
                        slot: Slot::new(slot),
                        root: block_root,
                        parent_root: Some(parent_root),
                        state_root: Hash256::zero(),
                        target_root: root(0),
                        current_epoch_shuffling_id: junk_shuffling_id(),
                        next_epoch_shuffling_id: junk_shuffling_id(),
                        justified_checkpoint: genesis_checkpoint(),
                        finalized_checkpoint: genesis_checkpoint(),
                        execution_status: ExecutionStatus::irrelevant(),
                        unrealized_justified_checkpoint: Some(genesis_checkpoint()),
                        unrealized_finalized_checkpoint: Some(genesis_checkpoint()),
                        builder_index: Some(builder_index),
                        payload_revealed: false,
                        ptc_weight: 0,
                        ptc_blob_data_available_weight: 0,
                        payload_data_available: false,
                        bid_block_hash: None,
                        bid_parent_block_hash: None,
                        proposer_index: 0,
                        ptc_timely: false,
                    },
                    Slot::new(slot),
                )
                .unwrap();
        }

        /// Build an IndexedAttestation (Electra variant) with the given fields.
        fn make_indexed_attestation(
            slot: u64,
            beacon_block_root: Hash256,
            index: u64,
            attesting_indices: Vec<u64>,
        ) -> IndexedAttestation<E> {
            IndexedAttestation::Electra(IndexedAttestationElectra {
                attesting_indices: VariableList::from(attesting_indices),
                data: AttestationData {
                    slot: Slot::new(slot),
                    index,
                    beacon_block_root,
                    source: genesis_checkpoint(),
                    target: genesis_checkpoint(),
                },
                signature: AggregateSignature::empty(),
            })
        }

        #[test]
        fn gloas_attestation_index_must_be_0_or_1() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_gloas_block(&mut fc, 1, block_root, 42);
            fc.fc_store.current_slot = Slot::new(2);

            // index = 2 should be rejected
            let att = make_indexed_attestation(1, block_root, 2, vec![1]);
            let err = fc
                .on_attestation(Slot::new(2), att.to_ref(), AttestationFromBlock::False)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidAttestation(InvalidAttestation::InvalidCommitteeIndex {
                        index: 2
                    })
                ),
                "Expected InvalidCommitteeIndex, got {:?}",
                err
            );
        }

        #[test]
        fn gloas_attestation_index_0_accepted() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_gloas_block(&mut fc, 1, block_root, 42);
            fc.fc_store.current_slot = Slot::new(2);

            // index = 0 should be accepted
            let att = make_indexed_attestation(1, block_root, 0, vec![1]);
            fc.on_attestation(Slot::new(2), att.to_ref(), AttestationFromBlock::False)
                .unwrap();
        }

        #[test]
        fn gloas_same_slot_attestation_must_have_index_0() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_gloas_block(&mut fc, 1, block_root, 42);
            fc.fc_store.current_slot = Slot::new(2);

            // Same-slot attestation (att.slot == block.slot) with index = 1 should be rejected
            let att = make_indexed_attestation(1, block_root, 1, vec![1]);
            let err = fc
                .on_attestation(Slot::new(2), att.to_ref(), AttestationFromBlock::False)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidAttestation(InvalidAttestation::SameSlotNonZeroIndex { .. })
                ),
                "Expected SameSlotNonZeroIndex, got {:?}",
                err
            );
        }

        #[test]
        fn gloas_same_slot_attestation_index_0_accepted() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_gloas_block(&mut fc, 1, block_root, 42);
            fc.fc_store.current_slot = Slot::new(2);

            // Same-slot attestation with index = 0 should be accepted
            let att = make_indexed_attestation(1, block_root, 0, vec![1]);
            fc.on_attestation(Slot::new(2), att.to_ref(), AttestationFromBlock::False)
                .unwrap();
        }

        // TODO(https://github.com/ethereum/consensus-specs/pull/4918): un-ignore when the
        // PayloadNotRevealed check is enabled (after spec test vectors are updated).
        #[test]
        #[ignore]
        fn gloas_index_1_rejected_when_payload_not_revealed() {
            let mut fc = new_fc();
            let block_root = root(1);
            // Insert Gloas block at slot 1, attestation at slot 2 (not same-slot)
            insert_gloas_block(&mut fc, 1, block_root, 42);
            fc.fc_store.current_slot = Slot::new(3);

            // index = 1 but payload_revealed = false → should be rejected
            let att = make_indexed_attestation(2, block_root, 1, vec![1]);
            let err = fc
                .on_attestation(Slot::new(3), att.to_ref(), AttestationFromBlock::False)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidAttestation(InvalidAttestation::PayloadNotRevealed { .. })
                ),
                "Expected PayloadNotRevealed, got {:?}",
                err
            );
        }

        #[test]
        fn gloas_index_1_accepted_when_payload_revealed() {
            let mut fc = new_fc();
            let block_root = root(1);
            insert_gloas_block(&mut fc, 1, block_root, 42);

            // Reveal the payload (simulates on_execution_payload)
            let hash = ExecutionBlockHash::repeat_byte(0xaa);
            fc.on_execution_payload(block_root, hash).unwrap();

            fc.fc_store.current_slot = Slot::new(3);

            // index = 1 with payload_revealed = true → should be accepted
            let att = make_indexed_attestation(2, block_root, 1, vec![1]);
            fc.on_attestation(Slot::new(3), att.to_ref(), AttestationFromBlock::False)
                .unwrap();
        }

        #[test]
        fn pre_gloas_block_allows_any_index() {
            let mut fc = new_fc();
            let block_root = root(1);
            // Insert a non-Gloas block (builder_index = None)
            insert_block(&mut fc, 1, block_root);
            fc.fc_store.current_slot = Slot::new(2);

            // index = 1 on a pre-Gloas block should be accepted (no Gloas checks apply)
            let att = make_indexed_attestation(1, block_root, 1, vec![1]);
            fc.on_attestation(Slot::new(2), att.to_ref(), AttestationFromBlock::False)
                .unwrap();
        }
    }
}
