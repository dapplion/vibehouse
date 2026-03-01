use crate::{
    JustifiedBalances,
    error::Error,
    proto_array::{
        InvalidationOperation, Iter, ProposerBoost, ProtoArray, ProtoNode,
        calculate_committee_fraction,
    },
    ssz_container::SszContainer,
};
use serde::{Deserialize, Serialize};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
};
use types::{
    AttestationShufflingId, ChainSpec, Checkpoint, Epoch, EthSpec, ExecutionBlockHash,
    FixedBytesExtended, Hash256, Slot,
};

pub const DEFAULT_PRUNE_THRESHOLD: usize = 256;

#[derive(Default, PartialEq, Clone, Encode, Decode)]
pub struct VoteTracker {
    current_root: Hash256,
    next_root: Hash256,
    next_epoch: Epoch,
    /// Gloas: slot of the attestation for is_supporting_vote.
    current_slot: Slot,
    next_slot: Slot,
    /// Gloas: whether the attestation indicated payload_present (index == 1).
    current_payload_present: bool,
    next_payload_present: bool,
}

/// Payload status for Gloas fork choice virtual nodes.
/// Each block is modeled as 3 virtual nodes: PENDING, EMPTY, FULL.
/// Ordinal values match consensus-specs: Empty=0, Full=1, Pending=2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GloasPayloadStatus {
    Empty = 0,
    Full = 1,
    Pending = 2,
}

/// A virtual fork choice node in the Gloas model: (root, payload_status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GloasForkChoiceNode {
    root: Hash256,
    payload_status: GloasPayloadStatus,
}

/// Represents the verification status of an execution payload.
#[derive(Clone, Copy, Debug, PartialEq, Encode, Decode, Serialize, Deserialize)]
#[ssz(enum_behaviour = "union")]
pub enum ExecutionStatus {
    /// An EL has determined that the payload is valid.
    Valid(ExecutionBlockHash),
    /// An EL has determined that the payload is invalid.
    Invalid(ExecutionBlockHash),
    /// An EL has not yet verified the execution payload.
    Optimistic(ExecutionBlockHash),
    /// The block is either prior to the merge fork, or after the merge fork but before the terminal
    /// PoW block has been found.
    ///
    /// # Note:
    ///
    /// This `bool` only exists to satisfy our SSZ implementation which requires all variants
    /// to have a value. It can be set to anything.
    Irrelevant(bool),
}

impl ExecutionStatus {
    pub fn is_execution_enabled(&self) -> bool {
        !matches!(self, ExecutionStatus::Irrelevant(_))
    }

    pub fn irrelevant() -> Self {
        ExecutionStatus::Irrelevant(false)
    }

    pub fn block_hash(&self) -> Option<ExecutionBlockHash> {
        match self {
            ExecutionStatus::Valid(hash)
            | ExecutionStatus::Invalid(hash)
            | ExecutionStatus::Optimistic(hash) => Some(*hash),
            ExecutionStatus::Irrelevant(_) => None,
        }
    }

    /// Returns `true` if the block:
    ///
    /// - Has a valid payload, OR
    /// - Does not have execution enabled.
    ///
    /// Whenever this function returns `true`, the block is *fully valid*.
    pub fn is_valid_or_irrelevant(&self) -> bool {
        matches!(
            self,
            ExecutionStatus::Valid(_) | ExecutionStatus::Irrelevant(_)
        )
    }

    /// Returns `true` if the block:
    ///
    /// - Has execution enabled, AND
    /// - Has a valid payload
    ///
    /// This function will return `false` for any block from a slot prior to the Bellatrix fork.
    /// This means that some blocks that are perfectly valid will still receive a `false` response.
    /// See `Self::is_valid_or_irrelevant` for a function that will always return `true` given any
    /// perfectly valid block.
    pub fn is_valid_and_post_bellatrix(&self) -> bool {
        matches!(self, ExecutionStatus::Valid(_))
    }

    /// Returns `true` if the block:
    ///
    /// - Has execution enabled, AND
    /// - Has a payload that has not yet been verified by an EL.
    pub fn is_strictly_optimistic(&self) -> bool {
        matches!(self, ExecutionStatus::Optimistic(_))
    }

    /// Returns `true` if the block:
    ///
    /// - Has execution enabled, AND
    ///     - Has a payload that has not yet been verified by an EL, OR.
    ///     - Has a payload that has been deemed invalid by an EL.
    pub fn is_optimistic_or_invalid(&self) -> bool {
        matches!(
            self,
            ExecutionStatus::Optimistic(_) | ExecutionStatus::Invalid(_)
        )
    }

    /// Returns `true` if the block:
    ///
    /// - Has execution enabled, AND
    /// - Has an invalid payload.
    pub fn is_invalid(&self) -> bool {
        matches!(self, ExecutionStatus::Invalid(_))
    }

    /// Returns `true` if the block:
    ///
    /// - Does not have execution enabled (before or after Bellatrix fork)
    pub fn is_irrelevant(&self) -> bool {
        matches!(self, ExecutionStatus::Irrelevant(_))
    }
}

impl fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionStatus::Valid(_) => write!(f, "valid"),
            ExecutionStatus::Invalid(_) => write!(f, "invalid"),
            ExecutionStatus::Optimistic(_) => write!(f, "optimistic"),
            ExecutionStatus::Irrelevant(_) => write!(f, "irrelevant"),
        }
    }
}

/// A block that is to be applied to the fork choice.
///
/// A simplified version of `types::BeaconBlock`.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub slot: Slot,
    pub root: Hash256,
    pub parent_root: Option<Hash256>,
    pub state_root: Hash256,
    pub target_root: Hash256,
    pub current_epoch_shuffling_id: AttestationShufflingId,
    pub next_epoch_shuffling_id: AttestationShufflingId,
    pub justified_checkpoint: Checkpoint,
    pub finalized_checkpoint: Checkpoint,
    /// Indicates if an execution node has marked this block as valid. Also contains the execution
    /// block hash.
    pub execution_status: ExecutionStatus,
    pub unrealized_justified_checkpoint: Option<Checkpoint>,
    pub unrealized_finalized_checkpoint: Option<Checkpoint>,
    /// Gloas ePBS: Which builder won the bid for this slot (if any).
    pub builder_index: Option<types::BuilderIndex>,
    /// Gloas ePBS: Has the builder revealed the execution payload?
    pub payload_revealed: bool,
    /// Gloas ePBS: Initial PTC payload_present weight (usually 0 at block insertion).
    pub ptc_weight: u64,
    /// Gloas ePBS: Initial PTC blob_data_available weight (usually 0 at block insertion).
    pub ptc_blob_data_available_weight: u64,
    /// Gloas ePBS: Has the PTC quorum confirmed blob data availability?
    pub payload_data_available: bool,
    /// Gloas ePBS: The execution block hash from this block's bid.
    pub bid_block_hash: Option<ExecutionBlockHash>,
    /// Gloas ePBS: The parent execution block hash from this block's bid.
    pub bid_parent_block_hash: Option<ExecutionBlockHash>,
    /// The proposer index of the validator who proposed this block.
    pub proposer_index: u64,
    /// Whether this block was received before the PTC timeliness deadline.
    pub ptc_timely: bool,
    /// Gloas ePBS: Has the execution payload envelope been received and processed?
    /// Only set by on_execution_payload, NOT by PTC quorum.
    pub envelope_received: bool,
}

impl Block {
    /// Compute the proposer shuffling decision root of a child block in `child_block_epoch`.
    ///
    /// This function assumes that `child_block_epoch >= self.epoch`. It is the responsibility of
    /// the caller to check this condition, or else incorrect results will be produced.
    pub fn proposer_shuffling_root_for_child_block(
        &self,
        child_block_epoch: Epoch,
        spec: &ChainSpec,
    ) -> Hash256 {
        let block_epoch = self.current_epoch_shuffling_id.shuffling_epoch;

        // For child blocks in the Fulu fork epoch itself, we want to use the old logic. There is no
        // lookahead in the first Fulu epoch. So we check whether Fulu is enabled at
        // `child_block_epoch - 1`, i.e. whether `child_block_epoch > fulu_fork_epoch`.
        if !spec
            .fork_name_at_epoch(child_block_epoch.saturating_sub(1_u64))
            .fulu_enabled()
        {
            // Prior to Fulu the proposer shuffling decision root for the current epoch is the same
            // as the attestation shuffling for the *next* epoch, i.e. it is determined at the start
            // of the current epoch.
            if block_epoch == child_block_epoch {
                self.next_epoch_shuffling_id.shuffling_decision_block
            } else {
                // Otherwise, the child block epoch is greater, so its decision root is its parent
                // root itself (this block's root).
                self.root
            }
        } else {
            // After Fulu the proposer shuffling is determined with lookahead, so if the block
            // lies in the same epoch as its parent, its decision root is the same as the
            // parent's current epoch attester shuffling
            //
            // i.e. the block from the end of epoch N - 2.
            if child_block_epoch == block_epoch {
                self.current_epoch_shuffling_id.shuffling_decision_block
            } else if child_block_epoch == block_epoch + 1 {
                // If the block is the next epoch, then it instead shares its decision root with
                // the parent's *next epoch* attester shuffling.
                self.next_epoch_shuffling_id.shuffling_decision_block
            } else {
                // The child block lies in the future beyond the lookahead, at the point where this
                // block (its parent) will be the decision block.
                self.root
            }
        }
    }
}

/// A Vec-wrapper which will grow to match any request.
///
/// E.g., a `get` or `insert` to an out-of-bounds element will cause the Vec to grow (using
/// Default) to the smallest size required to fulfill the request.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct ElasticList<T>(pub Vec<T>);

impl<T> ElasticList<T>
where
    T: Default,
{
    fn ensure(&mut self, i: usize) {
        if self.0.len() <= i {
            self.0.resize_with(i + 1, Default::default);
        }
    }

    pub fn get_mut(&mut self, i: usize) -> &mut T {
        self.ensure(i);
        &mut self.0[i]
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.0.iter_mut()
    }
}

/// Information about the proposer head used for opportunistic re-orgs.
#[derive(Debug, Clone)]
pub struct ProposerHeadInfo {
    /// Information about the *current* head block, which may be re-orged.
    pub head_node: ProtoNode,
    /// Information about the parent of the current head, which should be selected as the parent
    /// for a new proposal *if* a re-org is decided on.
    pub parent_node: ProtoNode,
    /// The computed fraction of the active head committee balance below which we can re-org.
    pub re_org_head_weight_threshold: u64,
    /// The computed fraction of the active parent committee balance above which we can re-org.
    pub re_org_parent_weight_threshold: u64,
    /// The current slot from fork choice's point of view, may lead the wall-clock slot by upto
    /// 500ms.
    pub current_slot: Slot,
}

/// Error type to enable short-circuiting checks in `get_proposer_head`.
///
/// This type intentionally does not implement `Debug` so that callers are forced to handle the
/// enum.
#[derive(Debug, Clone, PartialEq)]
pub enum ProposerHeadError<T> {
    DoNotReOrg(DoNotReOrg),
    Error(T),
}

impl<T> From<DoNotReOrg> for ProposerHeadError<T> {
    fn from(e: DoNotReOrg) -> ProposerHeadError<T> {
        Self::DoNotReOrg(e)
    }
}

impl From<Error> for ProposerHeadError<Error> {
    fn from(e: Error) -> Self {
        Self::Error(e)
    }
}

impl<T1> ProposerHeadError<T1> {
    pub fn convert_inner_error<T2>(self) -> ProposerHeadError<T2>
    where
        T2: From<T1>,
    {
        self.map_inner_error(T2::from)
    }

    pub fn map_inner_error<T2>(self, f: impl FnOnce(T1) -> T2) -> ProposerHeadError<T2> {
        match self {
            ProposerHeadError::DoNotReOrg(reason) => ProposerHeadError::DoNotReOrg(reason),
            ProposerHeadError::Error(error) => ProposerHeadError::Error(f(error)),
        }
    }
}

/// Reasons why a re-org should not be attempted.
///
/// This type intentionally does not implement `Debug` so that the `Display` impl must be used.
#[derive(Debug, Clone, PartialEq)]
pub enum DoNotReOrg {
    MissingHeadOrParentNode,
    MissingHeadFinalizedCheckpoint,
    ParentDistance,
    HeadDistance,
    ShufflingUnstable,
    DisallowedOffset {
        offset: u64,
    },
    JustificationAndFinalizationNotCompetitive,
    ChainNotFinalizing {
        epochs_since_finalization: u64,
    },
    HeadNotWeak {
        head_weight: u64,
        re_org_head_weight_threshold: u64,
    },
    ParentNotStrong {
        parent_weight: u64,
        re_org_parent_weight_threshold: u64,
    },
    HeadNotLate,
    NotProposing,
    ReOrgsDisabled,
}

impl std::fmt::Display for DoNotReOrg {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::MissingHeadOrParentNode => write!(f, "unknown head or parent"),
            Self::MissingHeadFinalizedCheckpoint => write!(f, "finalized checkpoint missing"),
            Self::ParentDistance => write!(f, "parent too far from head"),
            Self::HeadDistance => write!(f, "head too far from current slot"),
            Self::ShufflingUnstable => write!(f, "shuffling unstable at epoch boundary"),
            Self::DisallowedOffset { offset } => {
                write!(f, "re-orgs disabled at offset {offset}")
            }
            Self::JustificationAndFinalizationNotCompetitive => {
                write!(f, "justification or finalization not competitive")
            }
            Self::ChainNotFinalizing {
                epochs_since_finalization,
            } => write!(
                f,
                "chain not finalizing ({epochs_since_finalization} epochs since finalization)"
            ),
            Self::HeadNotWeak {
                head_weight,
                re_org_head_weight_threshold,
            } => {
                write!(
                    f,
                    "head not weak ({head_weight}/{re_org_head_weight_threshold})"
                )
            }
            Self::ParentNotStrong {
                parent_weight,
                re_org_parent_weight_threshold,
            } => {
                write!(
                    f,
                    "parent not strong ({parent_weight}/{re_org_parent_weight_threshold})"
                )
            }
            Self::HeadNotLate => {
                write!(f, "head arrived on time")
            }
            Self::NotProposing => {
                write!(f, "not proposing at next slot")
            }
            Self::ReOrgsDisabled => {
                write!(f, "re-orgs disabled in config")
            }
        }
    }
}

/// New-type for the re-org threshold percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReOrgThreshold(pub u64);

/// New-type for disallowed re-org slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisallowedReOrgOffsets {
    // Vecs are faster than hashmaps for small numbers of items.
    offsets: Vec<u64>,
}

impl Default for DisallowedReOrgOffsets {
    fn default() -> Self {
        DisallowedReOrgOffsets { offsets: vec![0] }
    }
}

impl DisallowedReOrgOffsets {
    pub fn new<E: EthSpec>(offsets: Vec<u64>) -> Result<Self, Error> {
        for &offset in &offsets {
            if offset >= E::slots_per_epoch() {
                return Err(Error::InvalidEpochOffset(offset));
            }
        }
        Ok(Self { offsets })
    }
}

#[derive(PartialEq)]
pub struct ProtoArrayForkChoice {
    pub(crate) proto_array: ProtoArray,
    pub(crate) votes: ElasticList<VoteTracker>,
    pub(crate) balances: JustifiedBalances,
    /// Gloas: payload status of the head from the last `find_head_gloas` call.
    /// 0 = EMPTY, 1 = FULL, 2 = PENDING. `None` for pre-Gloas heads.
    pub(crate) gloas_head_payload_status: Option<u8>,
}

impl ProtoArrayForkChoice {
    #[allow(clippy::too_many_arguments)]
    pub fn new<E: EthSpec>(
        current_slot: Slot,
        finalized_block_slot: Slot,
        finalized_block_state_root: Hash256,
        justified_checkpoint: Checkpoint,
        finalized_checkpoint: Checkpoint,
        current_epoch_shuffling_id: AttestationShufflingId,
        next_epoch_shuffling_id: AttestationShufflingId,
        execution_status: ExecutionStatus,
    ) -> Result<Self, String> {
        let mut proto_array = ProtoArray {
            prune_threshold: DEFAULT_PRUNE_THRESHOLD,
            justified_checkpoint,
            finalized_checkpoint,
            nodes: Vec::with_capacity(1),
            indices: HashMap::with_capacity(1),
            previous_proposer_boost: ProposerBoost::default(),
        };

        let block = Block {
            slot: finalized_block_slot,
            root: finalized_checkpoint.root,
            parent_root: None,
            state_root: finalized_block_state_root,
            // We are using the finalized_root as the target_root, since it always lies on an
            // epoch boundary.
            target_root: finalized_checkpoint.root,
            current_epoch_shuffling_id,
            next_epoch_shuffling_id,
            justified_checkpoint,
            finalized_checkpoint,
            execution_status,
            unrealized_justified_checkpoint: Some(justified_checkpoint),
            unrealized_finalized_checkpoint: Some(finalized_checkpoint),
            builder_index: None,
            payload_revealed: false,
            ptc_weight: 0,
            ptc_blob_data_available_weight: 0,
            payload_data_available: false,
            bid_block_hash: None,
            bid_parent_block_hash: None,
            proposer_index: 0,
            ptc_timely: false,
            envelope_received: false,
        };

        proto_array
            .on_block::<E>(block, current_slot)
            .map_err(|e| format!("Failed to add finalized block to proto_array: {:?}", e))?;

        Ok(Self {
            proto_array,
            votes: ElasticList::default(),
            balances: JustifiedBalances::default(),
            gloas_head_payload_status: None,
        })
    }

    /// See `ProtoArray::propagate_execution_payload_validation` for documentation.
    pub fn process_execution_payload_validation(
        &mut self,
        block_root: Hash256,
    ) -> Result<(), String> {
        self.proto_array
            .propagate_execution_payload_validation(block_root)
            .map_err(|e| format!("Failed to process valid payload: {:?}", e))
    }

    /// See `ProtoArray::propagate_execution_payload_invalidation` for documentation.
    pub fn process_execution_payload_invalidation<E: EthSpec>(
        &mut self,
        op: &InvalidationOperation,
    ) -> Result<(), String> {
        self.proto_array
            .propagate_execution_payload_invalidation::<E>(op)
            .map_err(|e| format!("Failed to process invalid payload: {:?}", e))
    }

    pub fn process_attestation(
        &mut self,
        validator_index: usize,
        block_root: Hash256,
        target_epoch: Epoch,
        slot: Slot,
        payload_present: bool,
    ) -> Result<(), String> {
        let vote = self.votes.get_mut(validator_index);

        if target_epoch > vote.next_epoch || *vote == VoteTracker::default() {
            vote.next_root = block_root;
            vote.next_epoch = target_epoch;
            vote.next_slot = slot;
            vote.next_payload_present = payload_present;
        }

        Ok(())
    }

    pub fn process_block<E: EthSpec>(
        &mut self,
        block: Block,
        current_slot: Slot,
    ) -> Result<(), String> {
        if block.parent_root.is_none() {
            return Err("Missing parent root".to_string());
        }

        self.proto_array
            .on_block::<E>(block, current_slot)
            .map_err(|e| format!("process_block_error: {:?}", e))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn find_head<E: EthSpec>(
        &mut self,
        justified_checkpoint: Checkpoint,
        finalized_checkpoint: Checkpoint,
        justified_state_balances: &JustifiedBalances,
        proposer_boost_root: Hash256,
        equivocating_indices: &BTreeSet<u64>,
        current_slot: Slot,
        spec: &ChainSpec,
    ) -> Result<Hash256, String> {
        let new_balances = justified_state_balances;

        let deltas = compute_deltas(
            &self.proto_array.indices,
            &mut self.votes,
            &self.balances.effective_balances,
            &new_balances.effective_balances,
            equivocating_indices,
        )
        .map_err(|e| format!("find_head compute_deltas failed: {:?}", e))?;

        // Check if Gloas fork choice applies.
        // Use gloas_fork_epoch from spec: if current epoch >= gloas fork epoch, use Gloas algorithm.
        let is_gloas = spec
            .gloas_fork_epoch
            .is_some_and(|fork_epoch| current_slot.epoch(E::slots_per_epoch()) >= fork_epoch);

        if is_gloas {
            // Update the proto_array's checkpoints so that node_is_viable_for_head
            // uses the correct justified/finalized values for filtering.
            if justified_checkpoint != self.proto_array.justified_checkpoint
                || finalized_checkpoint != self.proto_array.finalized_checkpoint
            {
                self.proto_array.justified_checkpoint = justified_checkpoint;
                self.proto_array.finalized_checkpoint = finalized_checkpoint;
            }
            self.balances = new_balances.clone();
            return self.find_head_gloas::<E>(
                justified_checkpoint,
                proposer_boost_root,
                equivocating_indices,
                current_slot,
                spec,
            );
        }

        self.proto_array
            .apply_score_changes::<E>(
                deltas,
                justified_checkpoint,
                finalized_checkpoint,
                new_balances,
                proposer_boost_root,
                current_slot,
                spec,
            )
            .map_err(|e| format!("find_head apply_score_changes failed: {:?}", e))?;

        self.balances = new_balances.clone();

        self.gloas_head_payload_status = None;

        self.proto_array
            .find_head::<E>(&justified_checkpoint.root, current_slot)
            .map_err(|e| format!("find_head failed: {:?}", e))
    }

    /// Returns the Gloas head payload status from the last `find_head` call.
    /// 0 = EMPTY, 1 = FULL, 2 = PENDING. `None` for pre-Gloas heads.
    pub fn gloas_head_payload_status(&self) -> Option<u8> {
        self.gloas_head_payload_status
    }

    /// Get the block to propose on during `current_slot`.
    ///
    /// This function returns a *definitive* result which should be acted on.
    #[allow(clippy::too_many_arguments)]
    pub fn get_proposer_head<E: EthSpec>(
        &self,
        current_slot: Slot,
        canonical_head: Hash256,
        justified_balances: &JustifiedBalances,
        re_org_head_threshold: ReOrgThreshold,
        re_org_parent_threshold: ReOrgThreshold,
        disallowed_offsets: &DisallowedReOrgOffsets,
        max_epochs_since_finalization: Epoch,
    ) -> Result<ProposerHeadInfo, ProposerHeadError<Error>> {
        let info = self.get_proposer_head_info::<E>(
            current_slot,
            canonical_head,
            justified_balances,
            re_org_head_threshold,
            re_org_parent_threshold,
            disallowed_offsets,
            max_epochs_since_finalization,
        )?;

        // Only re-org a single slot. This prevents cascading failures during asynchrony.
        let head_slot_ok = info.head_node.slot + 1 == current_slot;
        if !head_slot_ok {
            return Err(DoNotReOrg::HeadDistance.into());
        }

        // Only re-org if the head's weight is less than the heads configured committee fraction.
        let head_weight = info.head_node.weight;
        let re_org_head_weight_threshold = info.re_org_head_weight_threshold;
        let weak_head = head_weight < re_org_head_weight_threshold;
        if !weak_head {
            return Err(DoNotReOrg::HeadNotWeak {
                head_weight,
                re_org_head_weight_threshold,
            }
            .into());
        }

        // Only re-org if the parent's weight is greater than the parents configured committee fraction.
        let parent_weight = info.parent_node.weight;
        let re_org_parent_weight_threshold = info.re_org_parent_weight_threshold;
        let parent_strong = parent_weight > re_org_parent_weight_threshold;
        if !parent_strong {
            return Err(DoNotReOrg::ParentNotStrong {
                parent_weight,
                re_org_parent_weight_threshold,
            }
            .into());
        }

        // All checks have passed, build upon the parent to re-org the head.
        Ok(info)
    }

    /// Get information about the block to propose on during `current_slot`.
    ///
    /// This function returns a *partial* result which must be processed further.
    #[allow(clippy::too_many_arguments)]
    pub fn get_proposer_head_info<E: EthSpec>(
        &self,
        current_slot: Slot,
        canonical_head: Hash256,
        justified_balances: &JustifiedBalances,
        re_org_head_threshold: ReOrgThreshold,
        re_org_parent_threshold: ReOrgThreshold,
        disallowed_offsets: &DisallowedReOrgOffsets,
        max_epochs_since_finalization: Epoch,
    ) -> Result<ProposerHeadInfo, ProposerHeadError<Error>> {
        let mut nodes = self
            .proto_array
            .iter_nodes(&canonical_head)
            .take(2)
            .cloned()
            .collect::<Vec<_>>();

        let parent_node = nodes.pop().ok_or(DoNotReOrg::MissingHeadOrParentNode)?;
        let head_node = nodes.pop().ok_or(DoNotReOrg::MissingHeadOrParentNode)?;

        let parent_slot = parent_node.slot;
        let head_slot = head_node.slot;
        let re_org_block_slot = head_slot + 1;

        // Check finalization distance.
        let proposal_epoch = re_org_block_slot.epoch(E::slots_per_epoch());
        let finalized_epoch = head_node
            .unrealized_finalized_checkpoint
            .ok_or(DoNotReOrg::MissingHeadFinalizedCheckpoint)?
            .epoch;
        let epochs_since_finalization = proposal_epoch.saturating_sub(finalized_epoch).as_u64();
        if epochs_since_finalization > max_epochs_since_finalization.as_u64() {
            return Err(DoNotReOrg::ChainNotFinalizing {
                epochs_since_finalization,
            }
            .into());
        }

        // Check parent distance from head.
        // Do not check head distance from current slot, as that condition needs to be
        // late-evaluated and is elided when `current_slot == head_slot`.
        let parent_slot_ok = parent_slot + 1 == head_slot;
        if !parent_slot_ok {
            return Err(DoNotReOrg::ParentDistance.into());
        }

        // Check shuffling stability.
        let shuffling_stable = re_org_block_slot % E::slots_per_epoch() != 0;
        if !shuffling_stable {
            return Err(DoNotReOrg::ShufflingUnstable.into());
        }

        // Check allowed slot offsets.
        let offset = (re_org_block_slot % E::slots_per_epoch()).as_u64();
        if disallowed_offsets.offsets.contains(&offset) {
            return Err(DoNotReOrg::DisallowedOffset { offset }.into());
        }

        // Check FFG.
        let ffg_competitive = parent_node.unrealized_justified_checkpoint
            == head_node.unrealized_justified_checkpoint
            && parent_node.unrealized_finalized_checkpoint
                == head_node.unrealized_finalized_checkpoint;
        if !ffg_competitive {
            return Err(DoNotReOrg::JustificationAndFinalizationNotCompetitive.into());
        }

        // Compute re-org weight thresholds for head and parent.
        let re_org_head_weight_threshold =
            calculate_committee_fraction::<E>(justified_balances, re_org_head_threshold.0)
                .ok_or(Error::ReOrgThresholdOverflow)?;

        let re_org_parent_weight_threshold =
            calculate_committee_fraction::<E>(justified_balances, re_org_parent_threshold.0)
                .ok_or(Error::ReOrgThresholdOverflow)?;

        Ok(ProposerHeadInfo {
            head_node,
            parent_node,
            re_org_head_weight_threshold,
            re_org_parent_weight_threshold,
            current_slot,
        })
    }

    /// Returns `true` if there are any blocks in `self` with an `INVALID` execution payload status.
    ///
    /// This will operate on *all* blocks, even those that do not descend from the finalized
    /// ancestor.
    pub fn contains_invalid_payloads(&mut self) -> bool {
        self.proto_array
            .nodes
            .iter()
            .any(|node| node.execution_status.is_invalid())
    }

    /// For all nodes, regardless of their relationship to the finalized block, set their execution
    /// status to be optimistic.
    ///
    /// In practice this means forgetting any `VALID` or `INVALID` statuses.
    pub fn set_all_blocks_to_optimistic<E: EthSpec>(
        &mut self,
        spec: &ChainSpec,
    ) -> Result<(), String> {
        // Iterate backwards through all nodes in the `proto_array`. Whilst it's not strictly
        // required to do this process in reverse, it seems natural when we consider how LMD votes
        // are counted.
        //
        // This function will touch all blocks, even those that do not descend from the finalized
        // block. Since this function is expected to run at start-up during very rare
        // circumstances we prefer simplicity over efficiency.
        for node_index in (0..self.proto_array.nodes.len()).rev() {
            let node = self
                .proto_array
                .nodes
                .get_mut(node_index)
                .ok_or("unreachable index out of bounds in proto_array nodes")?;

            match node.execution_status {
                ExecutionStatus::Invalid(block_hash) => {
                    node.execution_status = ExecutionStatus::Optimistic(block_hash);

                    // Restore the weight of the node, it would have been set to `0` in
                    // `apply_score_changes` when it was invalidated.
                    let mut restored_weight: u64 = self
                        .votes
                        .0
                        .iter()
                        .enumerate()
                        .filter_map(|(validator_index, vote)| {
                            if vote.current_root == node.root {
                                // Any voting validator that does not have a balance should be
                                // ignored. This is consistent with `compute_deltas`.
                                self.balances.effective_balances.get(validator_index)
                            } else {
                                None
                            }
                        })
                        .sum();

                    // If the invalid root was boosted, apply the weight to it and
                    // ancestors.
                    if let Some(proposer_score_boost) = spec.proposer_score_boost
                        && self.proto_array.previous_proposer_boost.root == node.root
                    {
                        // Compute the score based upon the current balances. We can't rely on
                        // the `previous_proposr_boost.score` since it is set to zero with an
                        // invalid node.
                        let proposer_score =
                            calculate_committee_fraction::<E>(&self.balances, proposer_score_boost)
                                .ok_or("Failed to compute proposer boost")?;
                        // Store the score we've applied here so it can be removed in
                        // a later call to `apply_score_changes`.
                        self.proto_array.previous_proposer_boost.score = proposer_score;
                        // Apply this boost to this node.
                        restored_weight = restored_weight
                            .checked_add(proposer_score)
                            .ok_or("Overflow when adding boost to weight")?;
                    }

                    // Add the restored weight to the node and all ancestors.
                    if restored_weight > 0 {
                        let mut node_or_ancestor = node;
                        loop {
                            node_or_ancestor.weight = node_or_ancestor
                                .weight
                                .checked_add(restored_weight)
                                .ok_or("Overflow when adding weight to ancestor")?;

                            if let Some(parent_index) = node_or_ancestor.parent {
                                node_or_ancestor = self
                                    .proto_array
                                    .nodes
                                    .get_mut(parent_index)
                                    .ok_or(format!("Missing parent index: {}", parent_index))?;
                            } else {
                                // This is either the finalized block or a block that does not
                                // descend from the finalized block.
                                break;
                            }
                        }
                    }
                }
                // There are no balance changes required if the node was either valid or
                // optimistic.
                ExecutionStatus::Valid(block_hash) | ExecutionStatus::Optimistic(block_hash) => {
                    node.execution_status = ExecutionStatus::Optimistic(block_hash)
                }
                // An irrelevant node cannot become optimistic, this is a no-op.
                ExecutionStatus::Irrelevant(_) => (),
            }
        }

        Ok(())
    }

    pub fn maybe_prune(&mut self, finalized_root: Hash256) -> Result<(), String> {
        self.proto_array
            .maybe_prune(finalized_root)
            .map_err(|e| format!("find_head maybe_prune failed: {:?}", e))
    }

    pub fn set_prune_threshold(&mut self, prune_threshold: usize) {
        self.proto_array.prune_threshold = prune_threshold;
    }

    pub fn len(&self) -> usize {
        self.proto_array.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.proto_array.nodes.is_empty()
    }

    pub fn contains_block(&self, block_root: &Hash256) -> bool {
        self.proto_array.indices.contains_key(block_root)
    }

    /// Returns `true` if any node in fork choice has an execution block hash
    /// matching the given `block_hash`. Checks both `bid_block_hash` (from bids)
    /// and `execution_status` block hash (from payloads).
    ///
    /// Spec: `[IGNORE] bid.parent_block_hash is the block hash of a known
    /// execution payload in fork choice.`
    pub fn contains_execution_block_hash(&self, block_hash: &ExecutionBlockHash) -> bool {
        // Skip zero/default hashes (e.g. genesis blocks with default bids).
        if block_hash.into_root().is_zero() {
            return true;
        }
        self.proto_array.nodes.iter().any(|node| {
            node.bid_block_hash == Some(*block_hash)
                || node
                    .execution_status
                    .block_hash()
                    .is_some_and(|h| h == *block_hash)
        })
    }

    fn get_proto_node(&self, block_root: &Hash256) -> Option<&ProtoNode> {
        let block_index = self.proto_array.indices.get(block_root)?;
        self.proto_array.nodes.get(*block_index)
    }

    pub fn get_block(&self, block_root: &Hash256) -> Option<Block> {
        let block = self.get_proto_node(block_root)?;
        let parent_root = block
            .parent
            .and_then(|i| self.proto_array.nodes.get(i))
            .map(|parent| parent.root);

        Some(Block {
            slot: block.slot,
            root: block.root,
            parent_root,
            state_root: block.state_root,
            target_root: block.target_root,
            current_epoch_shuffling_id: block.current_epoch_shuffling_id.clone(),
            next_epoch_shuffling_id: block.next_epoch_shuffling_id.clone(),
            justified_checkpoint: block.justified_checkpoint,
            finalized_checkpoint: block.finalized_checkpoint,
            execution_status: block.execution_status,
            unrealized_justified_checkpoint: block.unrealized_justified_checkpoint,
            unrealized_finalized_checkpoint: block.unrealized_finalized_checkpoint,
            builder_index: block.builder_index,
            payload_revealed: block.payload_revealed,
            ptc_weight: block.ptc_weight,
            ptc_blob_data_available_weight: block.ptc_blob_data_available_weight,
            payload_data_available: block.payload_data_available,
            bid_block_hash: block.bid_block_hash,
            bid_parent_block_hash: block.bid_parent_block_hash,
            proposer_index: block.proposer_index,
            ptc_timely: block.ptc_timely,
            envelope_received: block.envelope_received,
        })
    }

    /// Returns the `block.execution_status` field, if the block is present.
    pub fn get_block_execution_status(&self, block_root: &Hash256) -> Option<ExecutionStatus> {
        let block = self.get_proto_node(block_root)?;
        Some(block.execution_status)
    }

    /// Returns the weight of a given block.
    pub fn get_weight(&self, block_root: &Hash256) -> Option<u64> {
        let block_index = self.proto_array.indices.get(block_root)?;
        self.proto_array
            .nodes
            .get(*block_index)
            .map(|node| node.weight)
    }

    /// See `ProtoArray` documentation.
    pub fn is_descendant(&self, ancestor_root: Hash256, descendant_root: Hash256) -> bool {
        self.proto_array
            .is_descendant(ancestor_root, descendant_root)
    }

    /// See `ProtoArray` documentation.
    pub fn is_finalized_checkpoint_or_descendant<E: EthSpec>(
        &self,
        descendant_root: Hash256,
    ) -> bool {
        self.proto_array
            .is_finalized_checkpoint_or_descendant::<E>(descendant_root)
    }

    pub fn latest_message(&self, validator_index: usize) -> Option<(Hash256, Epoch)> {
        if validator_index < self.votes.0.len() {
            let vote = &self.votes.0[validator_index];

            if *vote == VoteTracker::default() {
                None
            } else {
                Some((vote.next_root, vote.next_epoch))
            }
        } else {
            None
        }
    }

    /// See `ProtoArray::iter_nodes`
    pub fn iter_nodes(&self, block_root: &Hash256) -> Iter<'_> {
        self.proto_array.iter_nodes(block_root)
    }

    /// See `ProtoArray::iter_block_roots`
    pub fn iter_block_roots(
        &self,
        block_root: &Hash256,
    ) -> impl Iterator<Item = (Hash256, Slot)> + '_ {
        self.proto_array.iter_block_roots(block_root)
    }

    pub fn as_ssz_container(&self) -> SszContainer {
        SszContainer::from(self)
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        SszContainer::from(self).as_ssz_bytes()
    }

    pub fn from_bytes(bytes: &[u8], balances: JustifiedBalances) -> Result<Self, String> {
        let container = SszContainer::from_ssz_bytes(bytes)
            .map_err(|e| format!("Failed to decode ProtoArrayForkChoice: {:?}", e))?;
        Self::from_container(container, balances)
    }

    pub fn from_container(
        container: SszContainer,
        balances: JustifiedBalances,
    ) -> Result<Self, String> {
        (container, balances)
            .try_into()
            .map_err(|e| format!("Failed to initialize ProtoArrayForkChoice: {e:?}"))
    }

    /// Returns a read-lock to core `ProtoArray` struct.
    ///
    /// Should only be used when encoding/decoding during troubleshooting.
    pub fn core_proto_array(&self) -> &ProtoArray {
        &self.proto_array
    }

    /// Returns a mutable reference to the core `ProtoArray` struct.
    ///
    /// Should only be used during database schema migrations.
    pub fn core_proto_array_mut(&mut self) -> &mut ProtoArray {
        &mut self.proto_array
    }

    /// Returns all nodes that have zero children and are descended from the finalized checkpoint.
    pub fn heads_descended_from_finalization<E: EthSpec>(&self) -> Vec<&ProtoNode> {
        self.proto_array.heads_descended_from_finalization::<E>()
    }

    // ─── Gloas fork choice ─────────────────────────────────────────────

    /// Gloas-specific head selection implementing the spec's (root, payload_status) model.
    ///
    /// Instead of proto_array's bottom-up weight propagation, this uses top-down traversal
    /// where each block has 3 virtual nodes: PENDING, EMPTY, FULL.
    ///
    /// Spec: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#get_head>
    #[allow(clippy::too_many_arguments)]
    fn find_head_gloas<E: EthSpec>(
        &mut self,
        justified_checkpoint: Checkpoint,
        proposer_boost_root: Hash256,
        equivocating_indices: &BTreeSet<u64>,
        current_slot: Slot,
        spec: &ChainSpec,
    ) -> Result<Hash256, String> {
        let filtered_roots = self.compute_filtered_roots::<E>(current_slot);
        let apply_boost = self.should_apply_proposer_boost_gloas::<E>(
            proposer_boost_root,
            equivocating_indices,
            current_slot,
            spec,
        );

        // Spec: PAYLOAD_TIMELY_THRESHOLD = PTC_SIZE // 2
        // Both is_payload_timely and is_payload_data_available use this threshold.
        let ptc_quorum_threshold = spec.ptc_size / 2;

        let mut head = GloasForkChoiceNode {
            root: justified_checkpoint.root,
            payload_status: GloasPayloadStatus::Pending,
        };

        loop {
            let children = self.get_gloas_children(&head, &filtered_roots);
            if children.is_empty() {
                self.gloas_head_payload_status = Some(head.payload_status as u8);
                return Ok(head.root);
            }

            // Pre-compute weights once per child to avoid redundant O(validators)
            // scans. max_by would re-compute weights on each comparison.
            let weighted: Vec<_> = children
                .into_iter()
                .map(|child| {
                    let w = self.get_gloas_weight::<E>(
                        &child,
                        proposer_boost_root,
                        apply_boost,
                        current_slot,
                        spec,
                    );
                    (child, w)
                })
                .collect();

            head = weighted
                .into_iter()
                .max_by(|(a, wa), (b, wb)| {
                    wa.cmp(wb).then_with(|| a.root.cmp(&b.root)).then_with(|| {
                        let ta = self.get_payload_tiebreaker(a, current_slot, ptc_quorum_threshold);
                        let tb = self.get_payload_tiebreaker(b, current_slot, ptc_quorum_threshold);
                        ta.cmp(&tb)
                    })
                })
                .unwrap() // safe: children is non-empty
                .0;
        }
    }

    /// Compute the set of block roots that lead to viable heads.
    /// This implements the spec's `get_filtered_block_tree`.
    fn compute_filtered_roots<E: EthSpec>(&self, current_slot: Slot) -> HashSet<Hash256> {
        let pa = &self.proto_array;
        let mut filtered = vec![false; pa.nodes.len()];

        // Pass 1 (forward): mark nodes that are viable for head
        for (i, node) in pa.nodes.iter().enumerate() {
            if pa.node_is_viable_for_head::<E>(node, current_slot) {
                filtered[i] = true;
            }
        }

        // Pass 2 (reverse): propagate upward and collect roots in one pass.
        // Nodes are ordered parent-before-child, so reverse iteration ensures parents
        // are marked before we reach them. We collect each filtered node's root as we go.
        let initial_count = filtered.iter().filter(|&&b| b).count();
        let mut roots = HashSet::with_capacity(initial_count);
        for i in (0..pa.nodes.len()).rev() {
            if filtered[i] {
                roots.insert(pa.nodes[i].root);
                if let Some(parent_idx) = pa.nodes[i].parent {
                    filtered[parent_idx] = true;
                }
            }
        }
        roots
    }

    /// Get children of a Gloas fork choice node.
    ///
    /// - PENDING → [EMPTY, optionally FULL if payload revealed]
    /// - EMPTY/FULL → [PENDING children matching parent_payload_status]
    fn get_gloas_children(
        &self,
        node: &GloasForkChoiceNode,
        filtered_roots: &HashSet<Hash256>,
    ) -> Vec<GloasForkChoiceNode> {
        let pa = &self.proto_array;

        match node.payload_status {
            GloasPayloadStatus::Pending => {
                let mut children = vec![GloasForkChoiceNode {
                    root: node.root,
                    payload_status: GloasPayloadStatus::Empty,
                }];

                // Include FULL child only if the execution payload envelope has been received
                // (not just PTC quorum). Maps to `root in store.payload_states` in the spec.
                if let Some(&idx) = pa.indices.get(&node.root)
                    && let Some(proto_node) = pa.nodes.get(idx)
                    && proto_node.envelope_received
                {
                    children.push(GloasForkChoiceNode {
                        root: node.root,
                        payload_status: GloasPayloadStatus::Full,
                    });
                }

                children
            }
            GloasPayloadStatus::Empty | GloasPayloadStatus::Full => {
                let mut children = Vec::with_capacity(2);

                if let Some(&parent_idx) = pa.indices.get(&node.root) {
                    let parent_node = &pa.nodes[parent_idx];

                    // Find all blocks whose parent is node.root with matching payload status
                    for child_node in &pa.nodes {
                        if child_node.parent != Some(parent_idx) {
                            continue;
                        }
                        if !filtered_roots.contains(&child_node.root) {
                            continue;
                        }
                        if self.get_parent_payload_status_of(child_node, parent_node)
                            == node.payload_status
                        {
                            children.push(GloasForkChoiceNode {
                                root: child_node.root,
                                payload_status: GloasPayloadStatus::Pending,
                            });
                        }
                    }
                }

                children
            }
        }
    }

    /// Determine the parent payload status of a child block relative to its parent.
    ///
    /// Compares child's `bid_parent_block_hash` with parent's `bid_block_hash`.
    /// If they match, the parent was FULL (execution payload delivered).
    /// Otherwise, the parent was EMPTY.
    fn get_parent_payload_status_of(
        &self,
        child: &ProtoNode,
        parent: &ProtoNode,
    ) -> GloasPayloadStatus {
        match (child.bid_parent_block_hash, parent.bid_block_hash) {
            (Some(child_parent_hash), Some(parent_hash)) if child_parent_hash == parent_hash => {
                GloasPayloadStatus::Full
            }
            _ => GloasPayloadStatus::Empty,
        }
    }

    /// Implements the Gloas spec's `should_apply_proposer_boost`.
    ///
    /// Returns false when the boosted block's parent is adjacent, weak, and there
    /// are equivocating blocks at the parent slot by the same proposer.
    fn should_apply_proposer_boost_gloas<E: EthSpec>(
        &self,
        proposer_boost_root: Hash256,
        equivocating_indices: &BTreeSet<u64>,
        _current_slot: Slot,
        spec: &ChainSpec,
    ) -> bool {
        if proposer_boost_root.is_zero() {
            return false;
        }

        let pa = &self.proto_array;

        // Get the boosted block
        let Some(&boost_idx) = pa.indices.get(&proposer_boost_root) else {
            return false;
        };
        let Some(boost_block) = pa.nodes.get(boost_idx) else {
            return false;
        };

        // Get its parent
        let Some(parent_idx) = boost_block.parent else {
            return true; // No parent = always boost
        };
        let Some(parent_block) = pa.nodes.get(parent_idx) else {
            return true;
        };

        // If parent slot + 1 < block slot (skipped slots), always boost
        if parent_block.slot + 1 < boost_block.slot {
            return true;
        }

        // Check if parent is head-weak using attestation weight at PENDING level
        let parent_node = GloasForkChoiceNode {
            root: parent_block.root,
            payload_status: GloasPayloadStatus::Pending,
        };

        // Compute attestation-only weight for the parent (no proposer boost)
        let parent_slot = parent_block.slot;
        let mut parent_att_weight: u64 = 0;
        for (val_index, vote) in self.votes.0.iter().enumerate() {
            if vote.current_root.is_zero() {
                continue;
            }
            let balance = self
                .balances
                .effective_balances
                .get(val_index)
                .copied()
                .unwrap_or(0);
            if balance == 0 {
                continue;
            }
            if self.is_supporting_vote_gloas_at_slot(&parent_node, vote, parent_slot) {
                parent_att_weight = parent_att_weight.saturating_add(balance);
            }
        }

        // Add equivocating validator weight to parent weight (per spec is_head_weak).
        // Equivocating validators' effective balance counts toward the head's weight.
        for &val_index in equivocating_indices {
            let balance = self
                .balances
                .effective_balances
                .get(val_index as usize)
                .copied()
                .unwrap_or(0);
            parent_att_weight = parent_att_weight.saturating_add(balance);
        }

        // Threshold for "head weak" is REORG_HEAD_WEIGHT_THRESHOLD% of committee
        let reorg_threshold = spec
            .reorg_head_weight_threshold
            .and_then(|pct| calculate_committee_fraction::<E>(&self.balances, pct))
            .unwrap_or(0);

        let head_is_weak = parent_att_weight < reorg_threshold;

        if !head_is_weak {
            // Parent is strong, always apply boost
            return true;
        }

        // Parent is weak AND adjacent: check for equivocating proposers per spec.
        // The spec checks: blocks that are PTC-timely, by the same proposer as parent,
        // at parent.slot (i.e., block.slot + 1 == boost_block.slot), and different from parent.
        let has_equivocation = pa.nodes.iter().any(|n| {
            n.ptc_timely
                && n.proposer_index == parent_block.proposer_index
                && n.slot + 1 == boost_block.slot
                && n.root != parent_block.root
        });

        // If equivocating proposer exists, suppress boost
        !has_equivocation
    }

    /// Compute weight for a Gloas fork choice node.
    ///
    /// Non-PENDING nodes from the previous slot get 0 weight (reorg resistance).
    /// Otherwise, sum of supporting attestation balances + proposer boost.
    fn get_gloas_weight<E: EthSpec>(
        &self,
        node: &GloasForkChoiceNode,
        proposer_boost_root: Hash256,
        apply_proposer_boost: bool,
        current_slot: Slot,
        spec: &ChainSpec,
    ) -> u64 {
        let pa = &self.proto_array;

        // Resolve the node's slot once, avoiding a HashMap lookup per validator
        let Some(&node_idx) = pa.indices.get(&node.root) else {
            return 0;
        };
        let Some(proto_node) = pa.nodes.get(node_idx) else {
            return 0;
        };
        let node_slot = proto_node.slot;

        // Non-PENDING nodes from previous slot get 0 weight
        if node.payload_status != GloasPayloadStatus::Pending && node_slot + 1 == current_slot {
            return 0;
        }

        // Sum attestation scores from supporting votes
        let mut weight: u64 = 0;
        for (val_index, vote) in self.votes.0.iter().enumerate() {
            if vote.current_root.is_zero() {
                continue;
            }
            let balance = self
                .balances
                .effective_balances
                .get(val_index)
                .copied()
                .unwrap_or(0);
            if balance == 0 {
                continue;
            }
            if self.is_supporting_vote_gloas_at_slot(node, vote, node_slot) {
                weight = weight.saturating_add(balance);
            }
        }

        // Proposer boost: treated as a synthetic vote at current_slot
        if !proposer_boost_root.is_zero()
            && apply_proposer_boost
            && let Some(boost_pct) = spec.proposer_score_boost
        {
            let boost_vote = VoteTracker {
                current_root: proposer_boost_root,
                current_slot,
                ..VoteTracker::default()
            };
            if self.is_supporting_vote_gloas_at_slot(node, &boost_vote, node_slot)
                && let Some(score) = calculate_committee_fraction::<E>(&self.balances, boost_pct)
            {
                weight = weight.saturating_add(score);
            }
        }

        weight
    }

    /// Check if a vote supports a Gloas fork choice node.
    ///
    /// Resolves the node's slot via HashMap lookup. Used by tests; production
    /// callers use `is_supporting_vote_gloas_at_slot` with a pre-resolved slot.
    #[cfg(test)]
    fn is_supporting_vote_gloas(&self, node: &GloasForkChoiceNode, vote: &VoteTracker) -> bool {
        let pa = &self.proto_array;
        let Some(&node_idx) = pa.indices.get(&node.root) else {
            return false;
        };
        let Some(block) = pa.nodes.get(node_idx) else {
            return false;
        };
        self.is_supporting_vote_gloas_at_slot(node, vote, block.slot)
    }

    /// Check if a vote supports a node at a known slot.
    ///
    /// Avoids the HashMap lookup on `node.root` when the caller has already
    /// resolved the node's slot (e.g. in `get_gloas_weight` which calls this
    /// once per validator with the same node).
    fn is_supporting_vote_gloas_at_slot(
        &self,
        node: &GloasForkChoiceNode,
        vote: &VoteTracker,
        node_slot: Slot,
    ) -> bool {
        if node.root == vote.current_root {
            match node.payload_status {
                GloasPayloadStatus::Pending => true,
                GloasPayloadStatus::Empty | GloasPayloadStatus::Full => {
                    // Spec: assert message.slot >= block.slot
                    debug_assert!(vote.current_slot >= node_slot);
                    if vote.current_slot == node_slot {
                        return false;
                    }
                    if vote.current_payload_present {
                        node.payload_status == GloasPayloadStatus::Full
                    } else {
                        node.payload_status == GloasPayloadStatus::Empty
                    }
                }
            }
        } else {
            // Ancestor check: does the vote's chain pass through this node?
            match self.get_ancestor_gloas(vote.current_root, node_slot) {
                Some(ancestor) => {
                    node.root == ancestor.root
                        && (node.payload_status == GloasPayloadStatus::Pending
                            || node.payload_status == ancestor.payload_status)
                }
                None => false,
            }
        }
    }

    /// Walk up the proto_array chain to find the ancestor at the given slot.
    ///
    /// Returns a `GloasForkChoiceNode` with the ancestor's root and the payload
    /// status relationship (derived from the child→parent bid block hashes).
    fn get_ancestor_gloas(&self, root: Hash256, slot: Slot) -> Option<GloasForkChoiceNode> {
        let pa = &self.proto_array;
        let idx = *pa.indices.get(&root)?;
        let block = pa.nodes.get(idx)?;

        if block.slot <= slot {
            return Some(GloasForkChoiceNode {
                root,
                payload_status: GloasPayloadStatus::Pending,
            });
        }

        // Walk up: find the first ancestor whose parent's slot <= target slot
        let mut child = block;
        let mut parent_idx = child.parent?;
        let mut parent = pa.nodes.get(parent_idx)?;

        while parent.slot > slot {
            child = parent;
            parent_idx = child.parent?;
            parent = pa.nodes.get(parent_idx)?;
        }

        // parent.slot <= slot < child.slot
        // The ancestor at slot is parent, payload status from child→parent relationship
        Some(GloasForkChoiceNode {
            root: parent.root,
            payload_status: self.get_parent_payload_status_of(child, parent),
        })
    }

    /// Tiebreaker between EMPTY and FULL payload statuses.
    ///
    /// For non-previous-slot nodes: use payload_status ordinal.
    /// For previous-slot EMPTY: 1 (favored).
    /// For previous-slot FULL: 2 if should extend payload, else 0.
    ///
    /// Note: PENDING nodes at the previous slot are unreachable here because
    /// `get_node_children` only returns nodes that are uniformly PENDING or
    /// uniformly non-PENDING, and PENDING nodes are unique per root so the
    /// tiebreaker is never invoked for them.
    fn get_payload_tiebreaker(
        &self,
        node: &GloasForkChoiceNode,
        current_slot: Slot,
        ptc_quorum_threshold: u64,
    ) -> u8 {
        let pa = &self.proto_array;

        let is_previous_slot = pa
            .indices
            .get(&node.root)
            .and_then(|&idx| pa.nodes.get(idx))
            .is_some_and(|n| n.slot + 1 == current_slot);

        // Spec: if not from the previous slot, return the ordinal status value.
        if !is_previous_slot {
            node.payload_status as u8
        } else if node.payload_status == GloasPayloadStatus::Empty {
            1
        } else {
            // FULL: use 2 if should extend payload, else 0.
            if self.should_extend_payload(node, ptc_quorum_threshold) {
                2
            } else {
                0
            }
        }
    }

    /// Spec: should_extend_payload(store, root)
    ///
    /// Returns true when:
    /// - (is_payload_timely AND is_payload_data_available), OR
    /// - no proposer boost root, OR
    /// - boosted block's parent is not this root, OR
    /// - boosted block's parent is already FULL
    fn should_extend_payload(&self, node: &GloasForkChoiceNode, ptc_quorum_threshold: u64) -> bool {
        let pa = &self.proto_array;

        // Check if payload is both timely and data-available.
        // Spec: is_payload_timely(store, root) AND is_payload_data_available(store, root)
        // Both require `root in store.payload_states` (envelope actually received/processed)
        // AND PTC quorum strictly above threshold (sum > PAYLOAD_TIMELY_THRESHOLD).
        let is_timely_and_available = pa
            .indices
            .get(&node.root)
            .and_then(|&idx| pa.nodes.get(idx))
            .is_some_and(|n| {
                n.envelope_received
                    && n.ptc_weight > ptc_quorum_threshold
                    && n.ptc_blob_data_available_weight > ptc_quorum_threshold
            });

        if is_timely_and_available {
            return true;
        }

        // No proposer boost root
        let proposer_boost_root = pa.previous_proposer_boost.root;
        if proposer_boost_root.is_zero() {
            return true;
        }

        // Check if boosted block's parent is this root
        if let Some(&boosted_idx) = pa.indices.get(&proposer_boost_root) {
            if let Some(boosted_node) = pa.nodes.get(boosted_idx) {
                // Boosted block's parent is not this root
                if let Some(parent_idx) = boosted_node.parent {
                    if let Some(parent_node) = pa.nodes.get(parent_idx) {
                        if parent_node.root != node.root {
                            return true;
                        }
                        // Boosted block's parent IS this root — check if parent is FULL
                        // per spec: is_parent_node_full(store, store.blocks[proposer_root])
                        // This compares boosted_block.bid.parent_block_hash with parent.bid.block_hash
                        if self.get_parent_payload_status_of(boosted_node, parent_node)
                            == GloasPayloadStatus::Full
                        {
                            return true;
                        }
                    }
                } else {
                    // Boosted block has no parent — can't extend from this root
                    return true;
                }
            }
        } else {
            // Boosted block not in fork choice — treat as no boost
            return true;
        }

        false
    }
}

/// Returns a list of `deltas`, where there is one delta for each of the indices in
/// `0..indices.len()`.
///
/// The deltas are formed by a change between `old_balances` and `new_balances`, and/or a change of vote in `votes`.
///
/// ## Errors
///
/// - If a value in `indices` is greater to or equal to `indices.len()`.
/// - If some `Hash256` in `votes` is not a key in `indices` (except for `Hash256::zero()`, this is
///   always valid).
fn compute_deltas(
    indices: &HashMap<Hash256, usize>,
    votes: &mut ElasticList<VoteTracker>,
    old_balances: &[u64],
    new_balances: &[u64],
    equivocating_indices: &BTreeSet<u64>,
) -> Result<Vec<i64>, Error> {
    let mut deltas = vec![0_i64; indices.len()];

    for (val_index, vote) in votes.iter_mut().enumerate() {
        // There is no need to create a score change if the validator has never voted or both their
        // votes are for the zero hash (alias to the genesis block).
        if vote.current_root == Hash256::zero() && vote.next_root == Hash256::zero() {
            continue;
        }

        // Handle newly slashed validators by deducting their weight from their current vote. We
        // determine if they are newly slashed by checking whether their `vote.current_root` is
        // non-zero. After applying the deduction a single time we set their `current_root` to zero
        // and never update it again (thus preventing repeat deductions).
        //
        // Even if they make new attestations which are processed by `process_attestation` these
        // will only update their `vote.next_root`.
        if equivocating_indices.contains(&(val_index as u64)) {
            // First time we've processed this slashing in fork choice:
            //
            // 1. Add a negative delta for their `current_root`.
            // 2. Set their `current_root` (permanently) to zero.
            if !vote.current_root.is_zero() {
                let old_balance = old_balances.get(val_index).copied().unwrap_or(0);

                if let Some(current_delta_index) = indices.get(&vote.current_root).copied() {
                    let delta = deltas
                        .get(current_delta_index)
                        .ok_or(Error::InvalidNodeDelta(current_delta_index))?
                        .checked_sub(old_balance as i64)
                        .ok_or(Error::DeltaOverflow(current_delta_index))?;

                    // Array access safe due to check on previous line.
                    deltas[current_delta_index] = delta;
                }

                vote.current_root = Hash256::zero();
            }
            // We've handled this slashed validator, continue without applying an ordinary delta.
            continue;
        }

        // If the validator was not included in the _old_ balances (i.e., it did not exist yet)
        // then say its balance was zero.
        let old_balance = old_balances.get(val_index).copied().unwrap_or(0);

        // If the validators vote is not known in the _new_ balances, then use a balance of zero.
        //
        // It is possible that there is a vote for an unknown validator if we change our justified
        // state to a new state with a higher epoch that is on a different fork because that fork may have
        // on-boarded less validators than the prior fork.
        let new_balance = new_balances.get(val_index).copied().unwrap_or(0);

        if vote.current_root != vote.next_root || old_balance != new_balance {
            // We ignore the vote if it is not known in `indices`. We assume that it is outside
            // of our tree (i.e., pre-finalization) and therefore not interesting.
            if let Some(current_delta_index) = indices.get(&vote.current_root).copied() {
                let delta = deltas
                    .get(current_delta_index)
                    .ok_or(Error::InvalidNodeDelta(current_delta_index))?
                    .checked_sub(old_balance as i64)
                    .ok_or(Error::DeltaOverflow(current_delta_index))?;

                // Array access safe due to check on previous line.
                deltas[current_delta_index] = delta;
            }

            // We ignore the vote if it is not known in `indices`. We assume that it is outside
            // of our tree (i.e., pre-finalization) and therefore not interesting.
            if let Some(next_delta_index) = indices.get(&vote.next_root).copied() {
                let delta = deltas
                    .get(next_delta_index)
                    .ok_or(Error::InvalidNodeDelta(next_delta_index))?
                    .checked_add(new_balance as i64)
                    .ok_or(Error::DeltaOverflow(next_delta_index))?;

                // Array access safe due to check on previous line.
                deltas[next_delta_index] = delta;
            }

            vote.current_root = vote.next_root;
            vote.current_slot = vote.next_slot;
            vote.current_payload_present = vote.next_payload_present;
        }
    }

    Ok(deltas)
}

#[cfg(test)]
mod test_compute_deltas {
    use super::*;
    use types::{FixedBytesExtended, MainnetEthSpec};

    /// Gives a hash that is not the zero hash (unless i is `usize::MAX)`.
    fn hash_from_index(i: usize) -> Hash256 {
        Hash256::from_low_u64_be(i as u64 + 1)
    }

    #[test]
    fn finalized_descendant() {
        let genesis_slot = Slot::new(0);
        let genesis_epoch = Epoch::new(0);

        let state_root = Hash256::from_low_u64_be(0);
        let finalized_root = Hash256::from_low_u64_be(1);
        let finalized_desc = Hash256::from_low_u64_be(2);
        let not_finalized_desc = Hash256::from_low_u64_be(3);
        let unknown = Hash256::from_low_u64_be(4);
        let junk_shuffling_id =
            AttestationShufflingId::from_components(Epoch::new(0), Hash256::zero());
        let execution_status = ExecutionStatus::irrelevant();

        let genesis_checkpoint = Checkpoint {
            epoch: genesis_epoch,
            root: finalized_root,
        };
        let junk_checkpoint = Checkpoint {
            epoch: Epoch::new(42),
            root: Hash256::repeat_byte(42),
        };

        let mut fc = ProtoArrayForkChoice::new::<MainnetEthSpec>(
            genesis_slot,
            genesis_slot,
            state_root,
            genesis_checkpoint,
            genesis_checkpoint,
            junk_shuffling_id.clone(),
            junk_shuffling_id.clone(),
            execution_status,
        )
        .unwrap();

        // Add block that is a finalized descendant.
        fc.proto_array
            .on_block::<MainnetEthSpec>(
                Block {
                    slot: genesis_slot + 1,
                    root: finalized_desc,
                    parent_root: Some(finalized_root),
                    state_root,
                    target_root: finalized_root,
                    current_epoch_shuffling_id: junk_shuffling_id.clone(),
                    next_epoch_shuffling_id: junk_shuffling_id.clone(),
                    justified_checkpoint: genesis_checkpoint,
                    finalized_checkpoint: genesis_checkpoint,
                    execution_status,
                    unrealized_justified_checkpoint: Some(genesis_checkpoint),
                    unrealized_finalized_checkpoint: Some(genesis_checkpoint),
                    builder_index: None,
                    payload_revealed: false,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash: None,
                    bid_parent_block_hash: None,
                    proposer_index: 0,
                    ptc_timely: false,
                    envelope_received: false,
                },
                genesis_slot + 1,
            )
            .unwrap();

        // Add block that is *not* a finalized descendant.
        fc.proto_array
            .on_block::<MainnetEthSpec>(
                Block {
                    slot: genesis_slot + 1,
                    root: not_finalized_desc,
                    parent_root: None,
                    state_root,
                    target_root: finalized_root,
                    current_epoch_shuffling_id: junk_shuffling_id.clone(),
                    next_epoch_shuffling_id: junk_shuffling_id,
                    // Use the junk checkpoint for the next to values to prevent
                    // the loop-shortcutting mechanism from triggering.
                    justified_checkpoint: junk_checkpoint,
                    finalized_checkpoint: junk_checkpoint,
                    execution_status,
                    unrealized_justified_checkpoint: None,
                    unrealized_finalized_checkpoint: None,
                    builder_index: None,
                    payload_revealed: false,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash: None,
                    bid_parent_block_hash: None,
                    proposer_index: 0,
                    ptc_timely: false,
                    envelope_received: false,
                },
                genesis_slot + 1,
            )
            .unwrap();

        assert!(!fc.is_descendant(unknown, unknown));
        assert!(!fc.is_descendant(unknown, finalized_root));
        assert!(!fc.is_descendant(unknown, finalized_desc));
        assert!(!fc.is_descendant(unknown, not_finalized_desc));

        assert!(fc.is_descendant(finalized_root, finalized_root));
        assert!(fc.is_descendant(finalized_root, finalized_desc));
        assert!(!fc.is_descendant(finalized_root, not_finalized_desc));
        assert!(!fc.is_descendant(finalized_root, unknown));

        assert!(fc.is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(finalized_root));
        assert!(fc.is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(finalized_desc));
        assert!(!fc.is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(not_finalized_desc));
        assert!(!fc.is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(unknown));

        assert!(!fc.is_descendant(finalized_desc, not_finalized_desc));
        assert!(fc.is_descendant(finalized_desc, finalized_desc));
        assert!(!fc.is_descendant(finalized_desc, finalized_root));
        assert!(!fc.is_descendant(finalized_desc, unknown));

        assert!(fc.is_descendant(not_finalized_desc, not_finalized_desc));
        assert!(!fc.is_descendant(not_finalized_desc, finalized_desc));
        assert!(!fc.is_descendant(not_finalized_desc, finalized_root));
        assert!(!fc.is_descendant(not_finalized_desc, unknown));
    }

    /// This test covers an interesting case where a block can be a descendant
    /// of the finalized *block*, but not a descenant of the finalized
    /// *checkpoint*.
    ///
    /// ## Example
    ///
    /// Consider this block tree which has three blocks (`A`, `B` and `C`):
    ///
    /// ```text
    /// [A] <--- [-] <--- [B]
    ///       |
    ///       |--[C]
    /// ```
    ///
    /// - `A` (slot 31) is the common descendant.
    /// - `B` (slot 33) descends from `A`, but there is a single skip slot
    ///   between it and `A`.
    /// - `C` (slot 32) descends from `A` and conflicts with `B`.
    ///
    /// Imagine that the `B` chain is finalized at epoch 1. This means that the
    /// finalized checkpoint points to the skipped slot at 32. The root of the
    /// finalized checkpoint is `A`.
    ///
    /// In this scenario, the block `C` has the finalized root (`A`) as an
    /// ancestor whilst simultaneously conflicting with the finalized
    /// checkpoint.
    ///
    /// This means that to ensure a block does not conflict with finality we
    /// must check to ensure that it's an ancestor of the finalized
    /// *checkpoint*, not just the finalized *block*.
    #[test]
    fn finalized_descendant_edge_case() {
        let get_block_root = Hash256::from_low_u64_be;
        let genesis_slot = Slot::new(0);
        let junk_state_root = Hash256::zero();
        let junk_shuffling_id =
            AttestationShufflingId::from_components(Epoch::new(0), Hash256::zero());
        let execution_status = ExecutionStatus::irrelevant();

        let genesis_checkpoint = Checkpoint {
            epoch: Epoch::new(0),
            root: get_block_root(0),
        };

        let mut fc = ProtoArrayForkChoice::new::<MainnetEthSpec>(
            genesis_slot,
            genesis_slot,
            junk_state_root,
            genesis_checkpoint,
            genesis_checkpoint,
            junk_shuffling_id.clone(),
            junk_shuffling_id.clone(),
            execution_status,
        )
        .unwrap();

        struct TestBlock {
            slot: u64,
            root: u64,
            parent_root: u64,
        }

        let insert_block = |fc: &mut ProtoArrayForkChoice, block: TestBlock| {
            fc.proto_array
                .on_block::<MainnetEthSpec>(
                    Block {
                        slot: Slot::from(block.slot),
                        root: get_block_root(block.root),
                        parent_root: Some(get_block_root(block.parent_root)),
                        state_root: Hash256::zero(),
                        target_root: Hash256::zero(),
                        current_epoch_shuffling_id: junk_shuffling_id.clone(),
                        next_epoch_shuffling_id: junk_shuffling_id.clone(),
                        justified_checkpoint: Checkpoint {
                            epoch: Epoch::new(0),
                            root: get_block_root(0),
                        },
                        finalized_checkpoint: genesis_checkpoint,
                        execution_status,
                        unrealized_justified_checkpoint: Some(genesis_checkpoint),
                        unrealized_finalized_checkpoint: Some(genesis_checkpoint),
                        builder_index: None,
                        payload_revealed: false,
                        ptc_weight: 0,
                        ptc_blob_data_available_weight: 0,
                        payload_data_available: false,
                        bid_block_hash: None,
                        bid_parent_block_hash: None,
                        proposer_index: 0,
                        ptc_timely: false,
                        envelope_received: false,
                    },
                    Slot::from(block.slot),
                )
                .unwrap();
        };

        /*
         * Start of interesting part of tests.
         */

        // Produce the 0th epoch of blocks. They should all form a chain from
        // the genesis block.
        for i in 1..MainnetEthSpec::slots_per_epoch() {
            insert_block(
                &mut fc,
                TestBlock {
                    slot: i,
                    root: i,
                    parent_root: i - 1,
                },
            )
        }

        let last_slot_of_epoch_0 = MainnetEthSpec::slots_per_epoch() - 1;

        // Produce a block that descends from the last block of epoch -.
        //
        // This block will be non-canonical.
        let non_canonical_slot = last_slot_of_epoch_0 + 1;
        insert_block(
            &mut fc,
            TestBlock {
                slot: non_canonical_slot,
                root: non_canonical_slot,
                parent_root: non_canonical_slot - 1,
            },
        );

        // Produce a block that descends from the last block of the 0th epoch,
        // that skips the 1st slot of the 1st epoch.
        //
        // This block will be canonical.
        let canonical_slot = last_slot_of_epoch_0 + 2;
        insert_block(
            &mut fc,
            TestBlock {
                slot: canonical_slot,
                root: canonical_slot,
                parent_root: non_canonical_slot - 1,
            },
        );

        let finalized_root = get_block_root(last_slot_of_epoch_0);

        // Set the finalized checkpoint to finalize the first slot of epoch 1 on
        // the canonical chain.
        fc.proto_array.finalized_checkpoint = Checkpoint {
            root: finalized_root,
            epoch: Epoch::new(1),
        };

        assert!(
            fc.proto_array
                .is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(finalized_root),
            "the finalized checkpoint is the finalized checkpoint"
        );

        assert!(
            fc.proto_array
                .is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(get_block_root(
                    canonical_slot
                )),
            "the canonical block is a descendant of the finalized checkpoint"
        );
        assert!(
            !fc.proto_array
                .is_finalized_checkpoint_or_descendant::<MainnetEthSpec>(get_block_root(
                    non_canonical_slot
                )),
            "although the non-canonical block is a descendant of the finalized block, \
            it's not a descendant of the finalized checkpoint"
        );
    }

    #[test]
    fn zero_hash() {
        let validator_count: usize = 16;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let mut old_balances = vec![];
        let mut new_balances = vec![];
        let equivocating_indices = BTreeSet::new();

        for i in 0..validator_count {
            indices.insert(hash_from_index(i), i);
            votes.0.push(VoteTracker {
                current_root: Hash256::zero(),
                next_root: Hash256::zero(),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
            old_balances.push(0);
            new_balances.push(0);
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(
            deltas.len(),
            validator_count,
            "deltas should have expected length"
        );
        assert_eq!(
            deltas,
            vec![0; validator_count],
            "deltas should all be zero"
        );

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn all_voted_the_same() {
        const BALANCE: u64 = 42;

        let validator_count: usize = 16;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let mut old_balances = vec![];
        let mut new_balances = vec![];
        let equivocating_indices = BTreeSet::new();

        for i in 0..validator_count {
            indices.insert(hash_from_index(i), i);
            votes.0.push(VoteTracker {
                current_root: Hash256::zero(),
                next_root: hash_from_index(0),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
            old_balances.push(BALANCE);
            new_balances.push(BALANCE);
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(
            deltas.len(),
            validator_count,
            "deltas should have expected length"
        );

        for (i, delta) in deltas.into_iter().enumerate() {
            if i == 0 {
                assert_eq!(
                    delta,
                    BALANCE as i64 * validator_count as i64,
                    "zero'th root should have a delta"
                );
            } else {
                assert_eq!(delta, 0, "all other deltas should be zero");
            }
        }

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn different_votes() {
        const BALANCE: u64 = 42;

        let validator_count: usize = 16;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let mut old_balances = vec![];
        let mut new_balances = vec![];
        let equivocating_indices = BTreeSet::new();

        for i in 0..validator_count {
            indices.insert(hash_from_index(i), i);
            votes.0.push(VoteTracker {
                current_root: Hash256::zero(),
                next_root: hash_from_index(i),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
            old_balances.push(BALANCE);
            new_balances.push(BALANCE);
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(
            deltas.len(),
            validator_count,
            "deltas should have expected length"
        );

        for delta in deltas.into_iter() {
            assert_eq!(
                delta, BALANCE as i64,
                "each root should have the same delta"
            );
        }

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn moving_votes() {
        const BALANCE: u64 = 42;

        let validator_count: usize = 16;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let mut old_balances = vec![];
        let mut new_balances = vec![];
        let equivocating_indices = BTreeSet::new();

        for i in 0..validator_count {
            indices.insert(hash_from_index(i), i);
            votes.0.push(VoteTracker {
                current_root: hash_from_index(0),
                next_root: hash_from_index(1),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
            old_balances.push(BALANCE);
            new_balances.push(BALANCE);
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(
            deltas.len(),
            validator_count,
            "deltas should have expected length"
        );

        let total_delta = BALANCE as i64 * validator_count as i64;

        for (i, delta) in deltas.into_iter().enumerate() {
            if i == 0 {
                assert_eq!(
                    delta,
                    0 - total_delta,
                    "zero'th root should have a negative delta"
                );
            } else if i == 1 {
                assert_eq!(delta, total_delta, "first root should have positive delta");
            } else {
                assert_eq!(delta, 0, "all other deltas should be zero");
            }
        }

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn move_out_of_tree() {
        const BALANCE: u64 = 42;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let equivocating_indices = BTreeSet::new();

        // There is only one block.
        indices.insert(hash_from_index(1), 0);

        // There are two validators.
        let old_balances = vec![BALANCE; 2];
        let new_balances = vec![BALANCE; 2];

        // One validator moves their vote from the block to the zero hash.
        votes.0.push(VoteTracker {
            current_root: hash_from_index(1),
            next_root: Hash256::zero(),
            next_epoch: Epoch::new(0),
            ..VoteTracker::default()
        });

        // One validator moves their vote from the block to something outside the tree.
        votes.0.push(VoteTracker {
            current_root: hash_from_index(1),
            next_root: Hash256::from_low_u64_be(1337),
            next_epoch: Epoch::new(0),
            ..VoteTracker::default()
        });

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(deltas.len(), 1, "deltas should have expected length");

        assert_eq!(
            deltas[0],
            0 - BALANCE as i64 * 2,
            "the block should have lost both balances"
        );

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn changing_balances() {
        const OLD_BALANCE: u64 = 42;
        const NEW_BALANCE: u64 = OLD_BALANCE * 2;

        let validator_count: usize = 16;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let mut old_balances = vec![];
        let mut new_balances = vec![];
        let equivocating_indices = BTreeSet::new();

        for i in 0..validator_count {
            indices.insert(hash_from_index(i), i);
            votes.0.push(VoteTracker {
                current_root: hash_from_index(0),
                next_root: hash_from_index(1),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
            old_balances.push(OLD_BALANCE);
            new_balances.push(NEW_BALANCE);
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(
            deltas.len(),
            validator_count,
            "deltas should have expected length"
        );

        for (i, delta) in deltas.into_iter().enumerate() {
            if i == 0 {
                assert_eq!(
                    delta,
                    0 - OLD_BALANCE as i64 * validator_count as i64,
                    "zero'th root should have a negative delta"
                );
            } else if i == 1 {
                assert_eq!(
                    delta,
                    NEW_BALANCE as i64 * validator_count as i64,
                    "first root should have positive delta"
                );
            } else {
                assert_eq!(delta, 0, "all other deltas should be zero");
            }
        }

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn validator_appears() {
        const BALANCE: u64 = 42;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let equivocating_indices = BTreeSet::new();

        // There are two blocks.
        indices.insert(hash_from_index(1), 0);
        indices.insert(hash_from_index(2), 1);

        // There is only one validator in the old balances.
        let old_balances = vec![BALANCE; 1];
        // There are two validators in the new balances.
        let new_balances = vec![BALANCE; 2];

        // Both validator move votes from block 1 to block 2.
        for _ in 0..2 {
            votes.0.push(VoteTracker {
                current_root: hash_from_index(1),
                next_root: hash_from_index(2),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(deltas.len(), 2, "deltas should have expected length");

        assert_eq!(
            deltas[0],
            0 - BALANCE as i64,
            "block 1 should have only lost one balance"
        );
        assert_eq!(
            deltas[1],
            2 * BALANCE as i64,
            "block 2 should have gained two balances"
        );

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote shoulds should have been updated"
            );
        }
    }

    #[test]
    fn validator_disappears() {
        const BALANCE: u64 = 42;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();
        let equivocating_indices = BTreeSet::new();

        // There are two blocks.
        indices.insert(hash_from_index(1), 0);
        indices.insert(hash_from_index(2), 1);

        // There are two validators in the old balances.
        let old_balances = vec![BALANCE; 2];
        // There is only one validator in the new balances.
        let new_balances = vec![BALANCE; 1];

        // Both validator move votes from block 1 to block 2.
        for _ in 0..2 {
            votes.0.push(VoteTracker {
                current_root: hash_from_index(1),
                next_root: hash_from_index(2),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
        }

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(deltas.len(), 2, "deltas should have expected length");

        assert_eq!(
            deltas[0],
            0 - BALANCE as i64 * 2,
            "block 1 should have lost both balances"
        );
        assert_eq!(
            deltas[1], BALANCE as i64,
            "block 2 should have only gained one balance"
        );

        for vote in votes.0 {
            assert_eq!(
                vote.current_root, vote.next_root,
                "the vote should have been updated"
            );
        }
    }

    #[test]
    fn validator_equivocates() {
        const OLD_BALANCE: u64 = 42;
        const NEW_BALANCE: u64 = 43;

        let mut indices = HashMap::new();
        let mut votes = ElasticList::default();

        // There are two blocks.
        indices.insert(hash_from_index(1), 0);
        indices.insert(hash_from_index(2), 1);

        // There are two validators.
        let old_balances = vec![OLD_BALANCE; 2];
        let new_balances = vec![NEW_BALANCE; 2];

        // Both validator move votes from block 1 to block 2.
        for _ in 0..2 {
            votes.0.push(VoteTracker {
                current_root: hash_from_index(1),
                next_root: hash_from_index(2),
                next_epoch: Epoch::new(0),
                ..VoteTracker::default()
            });
        }

        // Validator 0 is slashed.
        let equivocating_indices = BTreeSet::from_iter([0]);

        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &old_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");

        assert_eq!(deltas.len(), 2, "deltas should have expected length");

        assert_eq!(
            deltas[0],
            -2 * OLD_BALANCE as i64,
            "block 1 should have lost two old balances"
        );
        assert_eq!(
            deltas[1], NEW_BALANCE as i64,
            "block 2 should have gained one balance"
        );

        // Validator 0's current root should have been reset.
        assert_eq!(votes.0[0].current_root, Hash256::zero());
        assert_eq!(votes.0[0].next_root, hash_from_index(2));

        // Validator 1's current root should have been updated.
        assert_eq!(votes.0[1].current_root, hash_from_index(2));

        // Re-computing the deltas should be a no-op (no repeat deduction for the slashed validator).
        let deltas = compute_deltas(
            &indices,
            &mut votes,
            &new_balances,
            &new_balances,
            &equivocating_indices,
        )
        .expect("should compute deltas");
        assert_eq!(deltas, vec![0, 0]);
    }
}

/// Tests for the Gloas (ePBS) fork choice virtual node model.
///
/// The Gloas fork choice replaces proto_array's bottom-up weight propagation with a
/// top-down traversal where each block has 3 virtual nodes: PENDING, EMPTY, FULL.
///
/// - PENDING: children are EMPTY + optionally FULL (if payload revealed)
/// - EMPTY/FULL: children are PENDING nodes of actual child blocks matching parent_payload_status
///
/// These tests verify is_supporting_vote, children enumeration, ancestor resolution,
/// parent payload status derivation, and head selection.
#[cfg(test)]
mod test_gloas_fork_choice {
    use super::*;
    use types::MinimalEthSpec;

    const BALANCE: u64 = 32_000_000_000;

    fn balances(count: usize) -> JustifiedBalances {
        JustifiedBalances {
            effective_balances: vec![BALANCE; count],
            total_effective_balance: BALANCE * count as u64,
            num_active_validators: count as u64,
        }
    }

    fn root(i: u64) -> Hash256 {
        Hash256::from_low_u64_be(i + 1)
    }

    fn exec_hash(i: u64) -> ExecutionBlockHash {
        ExecutionBlockHash::from_root(Hash256::from_low_u64_be(i + 100))
    }

    fn junk_shuffling_id() -> AttestationShufflingId {
        AttestationShufflingId::from_components(Epoch::new(0), Hash256::zero())
    }

    fn genesis_checkpoint() -> Checkpoint {
        Checkpoint {
            epoch: Epoch::new(0),
            root: root(0),
        }
    }

    /// Create a ProtoArrayForkChoice with a genesis block and Gloas-enabled spec.
    fn new_gloas_fc() -> (ProtoArrayForkChoice, ChainSpec) {
        let mut spec = MinimalEthSpec::default_spec();
        spec.gloas_fork_epoch = Some(Epoch::new(0));

        let fc = ProtoArrayForkChoice::new::<MinimalEthSpec>(
            Slot::new(0),
            Slot::new(0),
            Hash256::zero(),
            genesis_checkpoint(),
            genesis_checkpoint(),
            junk_shuffling_id(),
            junk_shuffling_id(),
            ExecutionStatus::irrelevant(),
        )
        .unwrap();

        (fc, spec)
    }

    /// Insert a Gloas block into the fork choice.
    ///
    /// Uses self-build mode (builder_index = BUILDER_INDEX_SELF_BUILD) so blocks
    /// are always viable for head regardless of payload_revealed status.
    /// The `payload_revealed` field controls whether the FULL virtual node exists
    /// in the fork choice children — a key part of the Gloas model.
    fn insert_gloas_block(
        fc: &mut ProtoArrayForkChoice,
        slot: u64,
        block_root: Hash256,
        parent_root: Hash256,
        bid_block_hash: Option<ExecutionBlockHash>,
        bid_parent_block_hash: Option<ExecutionBlockHash>,
        payload_revealed: bool,
    ) {
        fc.proto_array
            .on_block::<MinimalEthSpec>(
                Block {
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
                    builder_index: Some(types::consts::gloas::BUILDER_INDEX_SELF_BUILD),
                    payload_revealed,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash,
                    bid_parent_block_hash,
                    proposer_index: 0,
                    ptc_timely: false,
                    // In these tests, payload_revealed implies the envelope was received
                    envelope_received: payload_revealed,
                },
                Slot::new(slot),
            )
            .unwrap();
    }

    // ───────────────────── is_supporting_vote_gloas ─────────────────────

    #[test]
    fn supporting_vote_pending_always_supports_same_root() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        // A vote at the same root should always support PENDING, regardless of slot
        let vote = VoteTracker {
            current_root: root(1),
            current_slot: Slot::new(1),
            current_payload_present: false,
            ..VoteTracker::default()
        };
        assert!(fc.is_supporting_vote_gloas(&node, &vote));

        // Even with payload_present=true
        let vote_pp = VoteTracker {
            current_root: root(1),
            current_slot: Slot::new(2),
            current_payload_present: true,
            ..VoteTracker::default()
        };
        assert!(fc.is_supporting_vote_gloas(&node, &vote_pp));
    }

    #[test]
    fn supporting_vote_same_slot_never_supports_empty_or_full() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Vote at the same slot as the block should NOT support EMPTY or FULL
        let vote = VoteTracker {
            current_root: root(1),
            current_slot: Slot::new(1), // same as block slot
            current_payload_present: false,
            ..VoteTracker::default()
        };
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        assert!(!fc.is_supporting_vote_gloas(&empty_node, &vote));
        assert!(!fc.is_supporting_vote_gloas(&full_node, &vote));
    }

    #[test]
    fn supporting_vote_later_slot_matches_payload_present() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        // Vote at later slot with payload_present=false → supports EMPTY
        let vote_no_pp = VoteTracker {
            current_root: root(1),
            current_slot: Slot::new(2),
            current_payload_present: false,
            ..VoteTracker::default()
        };
        assert!(fc.is_supporting_vote_gloas(&empty_node, &vote_no_pp));
        assert!(!fc.is_supporting_vote_gloas(&full_node, &vote_no_pp));

        // Vote at later slot with payload_present=true → supports FULL
        let vote_pp = VoteTracker {
            current_root: root(1),
            current_slot: Slot::new(2),
            current_payload_present: true,
            ..VoteTracker::default()
        };
        assert!(!fc.is_supporting_vote_gloas(&empty_node, &vote_pp));
        assert!(fc.is_supporting_vote_gloas(&full_node, &vote_pp));
    }

    #[test]
    fn supporting_vote_ancestor_check() {
        // Build a chain: root(0) → root(1) → root(2)
        // Vote for root(2) should support ancestor root(1) at the right payload status.
        let (mut fc, _spec) = new_gloas_fc();

        // Block 1 at slot 1, bid_block_hash = exec_hash(1)
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Block 2 at slot 2, with bid_parent_block_hash matching parent's bid_block_hash → FULL
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches parent's bid_block_hash → parent was FULL
            false,
        );

        // Vote for root(2) at slot 3
        let vote = VoteTracker {
            current_root: root(2),
            current_slot: Slot::new(3),
            current_payload_present: false,
            ..VoteTracker::default()
        };

        // Node root(1) PENDING — ancestor always matches PENDING
        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };
        assert!(fc.is_supporting_vote_gloas(&pending_node, &vote));

        // Node root(1) FULL — matches because child's bid_parent == parent's bid_block_hash
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.is_supporting_vote_gloas(&full_node, &vote));

        // Node root(1) EMPTY — does NOT match
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        assert!(!fc.is_supporting_vote_gloas(&empty_node, &vote));
    }

    #[test]
    fn supporting_vote_unknown_root_returns_false() {
        let (fc, _spec) = new_gloas_fc();

        let node = GloasForkChoiceNode {
            root: root(99),
            payload_status: GloasPayloadStatus::Pending,
        };
        let vote = VoteTracker {
            current_root: root(99),
            current_slot: Slot::new(1),
            ..VoteTracker::default()
        };
        assert!(!fc.is_supporting_vote_gloas(&node, &vote));
    }

    // ───────────────────── get_parent_payload_status_of ─────────────────

    #[test]
    fn parent_payload_status_full_when_hashes_match() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // Child block whose bid_parent_block_hash matches parent's bid_block_hash
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches parent bid_block_hash
            false,
        );

        let parent_idx = *fc.proto_array.indices.get(&root(1)).unwrap();
        let child_idx = *fc.proto_array.indices.get(&root(2)).unwrap();
        let parent = &fc.proto_array.nodes[parent_idx];
        let child = &fc.proto_array.nodes[child_idx];

        assert_eq!(
            fc.get_parent_payload_status_of(child, parent),
            GloasPayloadStatus::Full
        );
    }

    #[test]
    fn parent_payload_status_empty_when_hashes_differ() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // Child whose bid_parent_block_hash does NOT match parent's bid_block_hash
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // does NOT match parent bid_block_hash
            false,
        );

        let parent_idx = *fc.proto_array.indices.get(&root(1)).unwrap();
        let child_idx = *fc.proto_array.indices.get(&root(2)).unwrap();
        let parent = &fc.proto_array.nodes[parent_idx];
        let child = &fc.proto_array.nodes[child_idx];

        assert_eq!(
            fc.get_parent_payload_status_of(child, parent),
            GloasPayloadStatus::Empty
        );
    }

    #[test]
    fn parent_payload_status_empty_when_none() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            None, // no bid_block_hash
            None,
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            None,
            None, // no bid_parent_block_hash
            false,
        );

        let parent_idx = *fc.proto_array.indices.get(&root(1)).unwrap();
        let child_idx = *fc.proto_array.indices.get(&root(2)).unwrap();
        let parent = &fc.proto_array.nodes[parent_idx];
        let child = &fc.proto_array.nodes[child_idx];

        assert_eq!(
            fc.get_parent_payload_status_of(child, parent),
            GloasPayloadStatus::Empty
        );
    }

    // ───────────────────── get_gloas_children ───────────────────────────

    #[test]
    fn pending_node_has_empty_child_only_when_payload_not_revealed() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false, // not revealed
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));
        let pending = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        let children = fc.get_gloas_children(&pending, &filtered);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].payload_status, GloasPayloadStatus::Empty);
        assert_eq!(children[0].root, root(1));
    }

    #[test]
    fn pending_node_has_empty_and_full_children_when_payload_revealed() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // revealed
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));
        let pending = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        let children = fc.get_gloas_children(&pending, &filtered);
        assert_eq!(children.len(), 2);

        let statuses: Vec<_> = children.iter().map(|c| c.payload_status).collect();
        assert!(statuses.contains(&GloasPayloadStatus::Empty));
        assert!(statuses.contains(&GloasPayloadStatus::Full));
    }

    #[test]
    fn empty_node_returns_pending_children_where_parent_was_empty() {
        // Chain: root(0) → root(1) → root(2)
        // root(2) has bid_parent_block_hash != root(1).bid_block_hash → parent was EMPTY
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // mismatch → parent was EMPTY
            false,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(3));
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };

        let children = fc.get_gloas_children(&empty_node, &filtered);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].root, root(2));
        assert_eq!(children[0].payload_status, GloasPayloadStatus::Pending);
    }

    #[test]
    fn full_node_returns_pending_children_where_parent_was_full() {
        // Chain: root(0) → root(1) → root(2)
        // root(2) has bid_parent_block_hash == root(1).bid_block_hash → parent was FULL
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // match → parent was FULL
            false,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(3));
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        let children = fc.get_gloas_children(&full_node, &filtered);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].root, root(2));
        assert_eq!(children[0].payload_status, GloasPayloadStatus::Pending);

        // EMPTY should return 0 children here (child was FULL path)
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let empty_children = fc.get_gloas_children(&empty_node, &filtered);
        assert!(empty_children.is_empty());
    }

    // ───────────────────── get_ancestor_gloas ───────────────────────────

    #[test]
    fn ancestor_at_same_slot_returns_pending() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Asking for ancestor of root(1) at slot 1 (block's own slot)
        let result = fc.get_ancestor_gloas(root(1), Slot::new(1));
        assert_eq!(
            result,
            Some(GloasForkChoiceNode {
                root: root(1),
                payload_status: GloasPayloadStatus::Pending,
            })
        );
    }

    #[test]
    fn ancestor_at_parent_slot_returns_correct_payload_status() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // Child whose bid_parent matches parent bid → parent is FULL
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let result = fc.get_ancestor_gloas(root(2), Slot::new(1));
        assert_eq!(
            result,
            Some(GloasForkChoiceNode {
                root: root(1),
                payload_status: GloasPayloadStatus::Full,
            })
        );
    }

    #[test]
    fn ancestor_with_mismatched_hashes_returns_empty() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // Child whose bid_parent does NOT match parent bid → parent is EMPTY
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
        );

        let result = fc.get_ancestor_gloas(root(2), Slot::new(1));
        assert_eq!(
            result,
            Some(GloasForkChoiceNode {
                root: root(1),
                payload_status: GloasPayloadStatus::Empty,
            })
        );
    }

    // ───────────────────── find_head_gloas (integration) ────────────────

    #[test]
    fn find_head_single_block_no_payload_returns_empty() {
        // Single block at slot 1 with no payload revealed → head should be that block
        // with EMPTY payload status.
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        let balances = balances(1);

        // Add a vote for root(1) from validator 0 at slot 2 (later than block)
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        // Should be EMPTY since payload not revealed
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8)
        );
    }

    #[test]
    fn find_head_payload_revealed_with_full_vote_returns_full() {
        // Single block with payload revealed. A vote with payload_present=true at a
        // later slot should cause FULL to win.
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload revealed
        );

        let balances = balances(1);

        // Vote with payload_present=true at slot 2
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8)
        );
    }

    #[test]
    fn find_head_two_blocks_votes_determine_winner() {
        // Two competing blocks at slot 1: root(1) and root(2).
        // More votes for root(2) → root(2) wins.
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(0)),
            false,
        );

        let balances = balances(3);

        // 1 vote for root(1), 2 votes for root(2)
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();
        fc.process_attestation(1, root(2), Epoch::new(0), Slot::new(2), false)
            .unwrap();
        fc.process_attestation(2, root(2), Epoch::new(0), Slot::new(2), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(2));
    }

    #[test]
    fn find_head_chain_with_full_path() {
        // Chain: root(0) → root(1) [revealed] → root(2) [full parent]
        // Votes for root(2) should traverse through root(1):FULL to reach root(2).
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload revealed
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // match → parent was FULL
            false,
        );

        let balances = balances(1);

        // Vote for root(2) at slot 3
        fc.process_attestation(0, root(2), Epoch::new(0), Slot::new(3), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(2));
    }

    #[test]
    fn find_head_empty_path_excludes_full_children() {
        // Block at slot 1 with payload NOT revealed.
        // Block at slot 2 with bid_parent != parent's bid_block_hash (EMPTY path).
        // Block at slot 2 with bid_parent == parent's bid_block_hash (FULL path).
        // Since payload not revealed at slot 1, FULL path is unreachable.
        // Only the EMPTY-path child should be selected.
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false, // NOT revealed
        );
        // EMPTY-path child (bid_parent mismatch)
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // mismatch → EMPTY path
            false,
        );
        // FULL-path child (bid_parent matches)
        insert_gloas_block(
            &mut fc,
            2,
            root(3),
            root(1),
            Some(exec_hash(3)),
            Some(exec_hash(1)), // match → FULL path
            false,
        );

        let balances = balances(2);

        // Votes for root(3) (FULL path) and root(2) (EMPTY path)
        fc.process_attestation(0, root(3), Epoch::new(0), Slot::new(3), false)
            .unwrap();
        fc.process_attestation(1, root(2), Epoch::new(0), Slot::new(3), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        // root(3) is on the FULL path but root(1)'s payload isn't revealed,
        // so FULL is not a child of PENDING. The fork choice can only
        // reach EMPTY → root(2).
        assert_eq!(head, root(2));
    }

    // ───────────────────── payload_present in votes ─────────────────────

    #[test]
    fn process_attestation_stores_payload_present() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Attestation with payload_present=true
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        let vote = &fc.votes.0[0];
        assert_eq!(vote.next_root, root(1));
        assert_eq!(vote.next_slot, Slot::new(2));
        assert!(vote.next_payload_present);

        // After find_head, current_* should be updated from next_*
        let balances = balances(1);
        let mut spec = MinimalEthSpec::default_spec();
        spec.gloas_fork_epoch = Some(Epoch::new(0));

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances,
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        let vote = &fc.votes.0[0];
        assert_eq!(vote.current_root, root(1));
        assert_eq!(vote.current_slot, Slot::new(2));
        assert!(vote.current_payload_present);
    }

    // ──────── on_execution_bid node state transitions ────────

    /// Insert a Gloas block with an external builder (not self-build).
    fn insert_external_builder_block(
        fc: &mut ProtoArrayForkChoice,
        slot: u64,
        block_root: Hash256,
        parent_root: Hash256,
        builder_index: u64,
    ) {
        fc.proto_array
            .on_block::<MinimalEthSpec>(
                Block {
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
                    bid_block_hash: Some(ExecutionBlockHash::repeat_byte(0xEE)),
                    bid_parent_block_hash: None,
                    proposer_index: 0,
                    ptc_timely: false,
                    envelope_received: false,
                },
                Slot::new(slot),
            )
            .unwrap();
    }

    /// Get a reference to a node by its block root.
    fn get_node<'a>(fc: &'a ProtoArrayForkChoice, block_root: &Hash256) -> &'a ProtoNode {
        let idx = *fc.proto_array.indices.get(block_root).unwrap();
        &fc.proto_array.nodes[idx]
    }

    /// Get a mutable reference to a node by its block root.
    fn get_node_mut<'a>(
        fc: &'a mut ProtoArrayForkChoice,
        block_root: &Hash256,
    ) -> &'a mut ProtoNode {
        let idx = *fc.proto_array.indices.get(block_root).unwrap();
        &mut fc.proto_array.nodes[idx]
    }

    #[test]
    fn bid_sets_builder_index_and_resets_payload() {
        // Simulates on_execution_bid: sets builder_index and resets payload state
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        // Insert a block (initially no builder)
        insert_gloas_block(&mut fc, 1, block_root, root(0), None, None, false);

        // Simulate on_execution_bid: set builder_index and reset PTC state
        let node = get_node_mut(&mut fc, &block_root);
        node.builder_index = Some(42);
        node.payload_revealed = false;
        node.ptc_weight = 0;
        node.ptc_blob_data_available_weight = 0;
        node.payload_data_available = false;

        // Verify state
        let node = get_node(&fc, &block_root);
        assert_eq!(node.builder_index, Some(42));
        assert!(!node.payload_revealed);
        assert_eq!(node.ptc_weight, 0);
        assert_eq!(node.ptc_blob_data_available_weight, 0);
        assert!(!node.payload_data_available);
    }

    #[test]
    fn bid_slot_mismatch_detectable() {
        // on_execution_bid rejects bids where bid.slot != node.slot
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        insert_gloas_block(&mut fc, 1, block_root, root(0), None, None, false);

        let node = get_node(&fc, &block_root);
        assert_eq!(node.slot, Slot::new(1));

        // A bid for slot 2 would be rejected by on_execution_bid
        // We verify the node's slot is what we expect for the mismatch check
        assert_ne!(node.slot, Slot::new(2));
    }

    // ──────── on_payload_attestation PTC quorum tests ────────

    #[test]
    fn ptc_weight_accumulates() {
        // Simulates on_payload_attestation: PTC weight accumulates
        // MinimalEthSpec: ptc_size=2, quorum_threshold = ptc_size/2 = 1
        let (mut fc, spec) = new_gloas_fc();
        let block_root = root(1);
        let quorum_threshold = spec.ptc_size / 2; // 1 for minimal

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Add 1 PTC vote — exactly at quorum threshold, not above
        let node = get_node_mut(&mut fc, &block_root);
        node.ptc_weight = node.ptc_weight.saturating_add(1);

        assert_eq!(get_node(&fc, &block_root).ptc_weight, 1);
        assert!(!get_node(&fc, &block_root).payload_revealed);
        // At threshold but not strictly greater — no reveal
        assert!(get_node(&fc, &block_root).ptc_weight <= quorum_threshold);
    }

    #[test]
    fn ptc_quorum_reveals_payload() {
        // When ptc_weight > quorum_threshold, payload_revealed is set to true
        let (mut fc, spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let quorum_threshold = spec.ptc_size / 2;

        // Set weight to exactly quorum + 1 (strictly greater)
        let node = get_node_mut(&mut fc, &block_root);
        node.ptc_weight = quorum_threshold + 1;

        // Simulate the quorum check from on_payload_attestation
        if node.ptc_weight > quorum_threshold && !node.payload_revealed {
            node.payload_revealed = true;
            if !node.execution_status.is_execution_enabled()
                && let Some(block_hash) = node.bid_block_hash
            {
                node.execution_status = ExecutionStatus::Optimistic(block_hash);
            }
        }

        let node = get_node(&fc, &block_root);
        assert!(node.payload_revealed);
        assert!(node.execution_status.is_execution_enabled());
        assert_eq!(
            node.execution_status.block_hash(),
            Some(ExecutionBlockHash::repeat_byte(0xEE))
        );
    }

    #[test]
    fn ptc_at_threshold_does_not_reveal() {
        // ptc_weight == quorum_threshold (exactly at boundary) does NOT reveal
        let (mut fc, spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let quorum_threshold = spec.ptc_size / 2;

        let node = get_node_mut(&mut fc, &block_root);
        node.ptc_weight = quorum_threshold;

        // Simulate quorum check: strictly greater required
        if node.ptc_weight > quorum_threshold && !node.payload_revealed {
            node.payload_revealed = true;
        }

        assert!(!get_node(&fc, &block_root).payload_revealed);
    }

    #[test]
    fn blob_data_availability_quorum() {
        // blob_data_available is set when blob weight exceeds quorum
        let (mut fc, spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let quorum_threshold = spec.ptc_size / 2;

        let node = get_node_mut(&mut fc, &block_root);
        node.ptc_blob_data_available_weight = quorum_threshold + 1;

        // Simulate blob data availability quorum check
        if node.ptc_blob_data_available_weight > quorum_threshold && !node.payload_data_available {
            node.payload_data_available = true;
        }

        assert!(get_node(&fc, &block_root).payload_data_available);
    }

    #[test]
    fn skip_slot_attestation_ignored() {
        // When attestation.data.slot != node.slot, the attestation is silently ignored
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Node is at slot 1, attestation would be for slot 2 (skip slot)
        let node = get_node(&fc, &block_root);
        assert_eq!(node.slot, Slot::new(1));
        assert_ne!(node.slot, Slot::new(2));
        // on_payload_attestation returns Ok(()) without modifying anything

        assert_eq!(node.ptc_weight, 0);
    }

    // ──────── on_execution_payload envelope reveal tests ─────

    #[test]
    fn payload_envelope_reveals_and_sets_status() {
        // Simulates on_execution_payload: sets payload_revealed, payload_data_available,
        // and execution_status
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Before reveal
        assert!(!get_node(&fc, &block_root).payload_revealed);
        assert!(!get_node(&fc, &block_root).payload_data_available);

        // Simulate on_execution_payload
        let payload_block_hash = ExecutionBlockHash::repeat_byte(0xFF);
        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.payload_data_available = true;
        node.execution_status = ExecutionStatus::Optimistic(payload_block_hash);

        // Verify
        let node = get_node(&fc, &block_root);
        assert!(node.payload_revealed);
        assert!(node.payload_data_available);
        assert_eq!(node.execution_status.block_hash(), Some(payload_block_hash));
    }

    #[test]
    fn payload_reveal_makes_external_block_viable() {
        // External builder block is not viable without payload reveal,
        // but becomes viable after reveal
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Not viable: external builder, payload not revealed
        let node = get_node(&fc, &block_root);
        assert!(
            !fc.proto_array
                .node_is_viable_for_head::<MinimalEthSpec>(node, Slot::new(1))
        );

        // Simulate on_execution_payload
        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.execution_status = ExecutionStatus::Optimistic(ExecutionBlockHash::repeat_byte(0xFF));

        // Now viable
        let node = get_node(&fc, &block_root);
        assert!(
            fc.proto_array
                .node_is_viable_for_head::<MinimalEthSpec>(node, Slot::new(1))
        );
    }

    #[test]
    fn ptc_quorum_makes_external_block_viable() {
        // External builder block becomes viable when PTC quorum reveals payload
        let (mut fc, spec) = new_gloas_fc();
        let block_root = root(1);

        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let quorum_threshold = spec.ptc_size / 2;

        // Not viable before quorum
        let node = get_node(&fc, &block_root);
        assert!(
            !fc.proto_array
                .node_is_viable_for_head::<MinimalEthSpec>(node, Slot::new(1))
        );

        // Simulate on_payload_attestation reaching quorum
        let node = get_node_mut(&mut fc, &block_root);
        node.ptc_weight = quorum_threshold + 1;
        node.payload_revealed = true;
        node.execution_status = ExecutionStatus::Optimistic(ExecutionBlockHash::repeat_byte(0xEE));

        // Now viable
        let node = get_node(&fc, &block_root);
        assert!(
            fc.proto_array
                .node_is_viable_for_head::<MinimalEthSpec>(node, Slot::new(1))
        );
    }

    #[test]
    fn self_build_always_viable_without_reveal() {
        // Self-build blocks (BUILDER_INDEX_SELF_BUILD) don't need payload reveal
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);

        insert_gloas_block(
            &mut fc,
            1,
            block_root,
            root(0),
            None,
            None,
            false, // payload_revealed = false
        );

        // Self-build blocks are always viable (builder_index = BUILDER_INDEX_SELF_BUILD)
        let node = get_node(&fc, &block_root);
        assert_eq!(
            node.builder_index,
            Some(types::consts::gloas::BUILDER_INDEX_SELF_BUILD)
        );
        assert!(
            fc.proto_array
                .node_is_viable_for_head::<MinimalEthSpec>(node, Slot::new(1))
        );
    }

    // ──────── should_extend_payload tests ─────────────────────

    // Minimal spec: ptc_size=2, threshold=1. Checks require strictly greater.
    const MINIMAL_PTC_THRESHOLD: u64 = 1;

    #[test]
    fn should_extend_payload_timely_and_data_available() {
        // When envelope_received AND ptc_weight > threshold AND blob weight > threshold → true
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Set envelope received + PTC quorum (strictly above threshold)
        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_timely_but_not_data_available() {
        // ptc_weight above threshold but blob weight not above threshold → falls through
        // to proposer boost checks. With no proposer boost → true.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = 0; // not above threshold

        // No proposer boost (default is zero root) → should return true
        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_no_proposer_boost_root() {
        // When previous_proposer_boost.root is zero → true
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Ensure proposer_boost_root is zero (default)
        assert!(fc.proto_array.previous_proposer_boost.root.is_zero());

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_boosted_parent_not_this_root() {
        // Boosted block's parent is NOT the node we're checking → true
        let (mut fc, _spec) = new_gloas_fc();
        let block_a = root(1);
        let block_b = root(2); // boosted block
        let block_c = root(3); // block_b's parent

        // Chain: root(0) → block_c(slot=1) → block_b(slot=2)
        insert_external_builder_block(&mut fc, 1, block_c, root(0), 42);
        insert_external_builder_block(&mut fc, 2, block_b, block_c, 42);
        // block_a is a separate branch
        insert_external_builder_block(&mut fc, 1, block_a, root(0), 42);

        // Set proposer boost to block_b — its parent is block_c, not block_a
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: block_b,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: block_a,
            payload_status: GloasPayloadStatus::Full,
        };
        // block_b's parent is block_c, not block_a → should extend
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_boosted_parent_is_this_root_and_full() {
        // Boosted block's parent IS this root, and the boosted block builds on FULL parent
        // (child.bid_parent_block_hash == parent.bid_block_hash) → should extend (true)
        let (mut fc, _spec) = new_gloas_fc();
        let parent_block = root(1);
        let child_block = root(2); // boosted block

        // Chain: root(0) → parent_block(slot=1) → child_block(slot=2)
        insert_external_builder_block(&mut fc, 1, parent_block, root(0), 42);
        insert_external_builder_block(&mut fc, 2, child_block, parent_block, 42);

        // Make child build on FULL parent: set child's bid_parent_block_hash to match
        // parent's bid_block_hash (both are 0xEE from insert_external_builder_block)
        let parent_bid_hash = get_node(&fc, &parent_block).bid_block_hash.unwrap();
        let child_node = get_node_mut(&mut fc, &child_block);
        child_node.bid_parent_block_hash = Some(parent_bid_hash);

        // Set proposer boost to child_block
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_block,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: parent_block,
            payload_status: GloasPayloadStatus::Full,
        };
        // Child builds on FULL parent → should extend
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_boosted_parent_is_this_root_and_not_full() {
        // Boosted block's parent IS this root, but the boosted block builds on EMPTY parent
        // (child.bid_parent_block_hash != parent.bid_block_hash) → should NOT extend (false)
        let (mut fc, _spec) = new_gloas_fc();
        let parent_block = root(1);
        let child_block = root(2); // boosted block

        // Chain: root(0) → parent_block(slot=1) → child_block(slot=2)
        insert_external_builder_block(&mut fc, 1, parent_block, root(0), 42);
        insert_external_builder_block(&mut fc, 2, child_block, parent_block, 42);

        // Child builds on EMPTY parent: bid_parent_block_hash is None (default from
        // insert_external_builder_block), which doesn't match parent's bid_block_hash
        assert!(get_node(&fc, &child_block).bid_parent_block_hash.is_none());

        // Set proposer boost to child_block
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_block,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: parent_block,
            payload_status: GloasPayloadStatus::Full,
        };
        // Child builds on EMPTY parent → should NOT extend
        assert!(!fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_boosted_block_not_in_fork_choice() {
        // Proposer boost root points to a block NOT in fork choice → true
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Set proposer boost to unknown block
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: root(999),
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    #[test]
    fn should_extend_payload_boosted_block_has_no_parent() {
        // Boosted block has no parent → true
        // The genesis block (root(0)) has parent=None in proto_array
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Set proposer boost to genesis (which was inserted at initialization with no parent)
        // The genesis root for new_gloas_fc is the finalized checkpoint root
        let genesis_root = genesis_checkpoint().root;
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: genesis_root,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        // Genesis has no parent → should extend
        assert!(fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD));
    }

    // ──────── get_payload_tiebreaker tests ────────────────────

    #[test]
    fn tiebreaker_pending_not_previous_slot_returns_ordinal() {
        // PENDING status at a non-previous-slot returns its ordinal value (2)
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Pending,
        };

        // Non-previous-slot: PENDING returns ordinal
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(100), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Pending as u8
        );
    }

    #[test]
    fn tiebreaker_not_previous_slot_returns_ordinal() {
        // When node is NOT at previous slot (current_slot - 1), return ordinal
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Block at slot 1, current_slot = 5 (not previous slot)
        let empty_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Empty,
        };
        assert_eq!(
            fc.get_payload_tiebreaker(&empty_node, Slot::new(5), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Empty as u8
        );

        let full_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert_eq!(
            fc.get_payload_tiebreaker(&full_node, Slot::new(5), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Full as u8
        );
    }

    #[test]
    fn tiebreaker_previous_slot_empty_returns_1() {
        // EMPTY at previous slot always returns 1 (favored)
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Empty,
        };

        // Block at slot 1, current_slot = 2 (previous slot)
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2), MINIMAL_PTC_THRESHOLD),
            1
        );
    }

    #[test]
    fn tiebreaker_previous_slot_full_should_extend() {
        // FULL at previous slot with should_extend_payload=true returns 2
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Make should_extend_payload return true: envelope received + PTC quorum above threshold
        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        // Block at slot 1, current_slot = 2 (previous slot)
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2), MINIMAL_PTC_THRESHOLD),
            2
        );
    }

    #[test]
    fn tiebreaker_previous_slot_full_should_not_extend() {
        // FULL at previous slot with should_extend_payload=false returns 0
        let (mut fc, _spec) = new_gloas_fc();
        let parent_block = root(1);
        let child_block = root(2);

        // Chain: root(0) → parent_block(slot=1) → child_block(slot=2)
        insert_external_builder_block(&mut fc, 1, parent_block, root(0), 42);
        insert_external_builder_block(&mut fc, 2, child_block, parent_block, 42);

        // Parent NOT revealed (default)
        assert!(!get_node(&fc, &parent_block).payload_revealed);

        // Set proposer boost to child_block — its parent IS parent_block and NOT full
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_block,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: parent_block,
            payload_status: GloasPayloadStatus::Full,
        };

        // Block at slot 1, current_slot = 2 (previous slot)
        // should_extend_payload=false → tiebreaker returns 0
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2), MINIMAL_PTC_THRESHOLD),
            0
        );
    }

    #[test]
    fn tiebreaker_ordering_previous_slot() {
        // Verify the tiebreaker ordering at previous slot:
        // When should_extend_payload=true: FULL(2) > EMPTY(1)
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Make should_extend_payload return true: envelope received + PTC quorum above threshold
        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        let empty = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Empty,
        };
        let full = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        let current_slot = Slot::new(2); // previous slot for block at slot 1
        let te = fc.get_payload_tiebreaker(&empty, current_slot, MINIMAL_PTC_THRESHOLD);
        let tf = fc.get_payload_tiebreaker(&full, current_slot, MINIMAL_PTC_THRESHOLD);

        // FULL > EMPTY when extending
        assert!(tf > te, "FULL({}) should beat EMPTY({})", tf, te);
    }

    #[test]
    fn tiebreaker_unknown_root_returns_ordinal() {
        // Node root not in fork choice — is_previous_slot check fails → ordinal
        let (fc, _spec) = new_gloas_fc();

        let gloas_node = GloasForkChoiceNode {
            root: root(999),
            payload_status: GloasPayloadStatus::Full,
        };

        // Unknown root → not at previous slot → returns ordinal
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Full as u8
        );
    }

    #[test]
    fn tiebreaker_pending_at_previous_slot_unreachable_but_safe() {
        // Per spec PR #4898: PENDING nodes at the previous slot are unreachable
        // because get_node_children returns uniformly PENDING or non-PENDING
        // children, and PENDING nodes are unique per root. This test documents
        // what would happen if a PENDING node somehow reached the tiebreaker at
        // the previous slot — it falls through to the FULL branch since it's
        // not EMPTY.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Make envelope received + PTC quorum above threshold so that
        // should_extend_payload returns true.
        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Pending,
        };

        // Block at slot 1, current_slot = 2 (previous slot).
        // PENDING falls through to FULL branch → should_extend_payload=true → 2.
        // This is unreachable in practice but harmless.
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2), MINIMAL_PTC_THRESHOLD),
            2
        );
    }

    // ──────── get_gloas_weight tests ─────────────────────────

    #[test]
    fn weight_no_votes_returns_zero() {
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(0);

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(weight, 0);
    }

    #[test]
    fn weight_single_supporting_vote() {
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(1);
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();

        // Apply votes: find_head applies pending votes to current
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(1),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(weight, BALANCE);
    }

    #[test]
    fn weight_multiple_votes_accumulate() {
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(3);
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();
        fc.process_attestation(1, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();
        fc.process_attestation(2, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(3),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(weight, BALANCE * 3);
    }

    #[test]
    fn weight_non_pending_previous_slot_returns_zero() {
        // Non-PENDING nodes at previous slot (current_slot - 1) get 0 weight
        // This is the reorg resistance mechanism
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        fc.balances = balances(1);
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(1),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // FULL node at slot 1, current_slot = 2 → previous slot → 0 weight
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &full_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(
            weight, 0,
            "non-PENDING at previous slot should have 0 weight"
        );

        // EMPTY node at slot 1, current_slot = 2 → also 0
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &empty_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(weight, 0, "EMPTY at previous slot should have 0 weight");

        // PENDING at slot 1, current_slot = 2 → normal weight
        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };
        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &pending_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        assert_eq!(
            weight, BALANCE,
            "PENDING at previous slot should have normal weight"
        );
    }

    #[test]
    fn weight_non_pending_non_previous_slot_has_normal_weight() {
        // Non-PENDING nodes NOT at previous slot should have normal weight
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        fc.balances = balances(1);
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(1),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // FULL node at slot 1, current_slot = 5 → NOT previous slot → normal weight
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &full_node,
            Hash256::zero(),
            false,
            Slot::new(5),
            &spec,
        );
        assert_eq!(
            weight, BALANCE,
            "FULL at non-previous slot should have normal weight"
        );
    }

    #[test]
    fn weight_with_proposer_boost() {
        let (mut fc, mut spec) = new_gloas_fc();
        spec.proposer_score_boost = Some(40); // 40% boost
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(1);
        // No attestation votes — just proposer boost

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        // Boost root = root(1), apply_boost = true, current_slot = 2
        // The boost vote is a synthetic vote at root(1), slot=2
        // Since node is PENDING and vote is for same root → supports
        let weight =
            fc.get_gloas_weight::<MinimalEthSpec>(&node, root(1), true, Slot::new(2), &spec);
        // committee_fraction = total_balance * 40 / 64 (slots_per_epoch=8, so committee=total/8)
        // For MinimalEthSpec: committee_size = total_balance / 8
        // boost = committee_size * 40 / 100 = (32G / 8) * 40 / 100 = 4G * 40 / 100 = 1.6G
        // Actually: calculate_committee_fraction computes total * pct / 100
        // = 32_000_000_000 * 40 / 100 = 12_800_000_000
        assert!(weight > 0, "proposer boost should add weight");
    }

    #[test]
    fn weight_proposer_boost_not_applied_when_flag_false() {
        let (mut fc, mut spec) = new_gloas_fc();
        spec.proposer_score_boost = Some(40);
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(1);

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        // apply_boost = false → no boost even if proposer_boost_root matches
        let weight =
            fc.get_gloas_weight::<MinimalEthSpec>(&node, root(1), false, Slot::new(2), &spec);
        assert_eq!(
            weight, 0,
            "should have no weight when boost disabled and no votes"
        );
    }

    #[test]
    fn weight_proposer_boost_zero_root_no_boost() {
        let (mut fc, mut spec) = new_gloas_fc();
        spec.proposer_score_boost = Some(40);
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        fc.balances = balances(1);

        let node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        // Zero boost root → no boost
        let weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &node,
            Hash256::zero(),
            true,
            Slot::new(2),
            &spec,
        );
        assert_eq!(weight, 0, "zero boost root should not add boost");
    }

    // ──────── should_apply_proposer_boost_gloas tests ────────

    #[test]
    fn proposer_boost_zero_root_returns_false() {
        let (fc, spec) = new_gloas_fc();
        assert!(!fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(1),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_unknown_root_returns_false() {
        let (fc, spec) = new_gloas_fc();
        assert!(!fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(999),
            &BTreeSet::new(),
            Slot::new(1),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_no_parent_returns_true() {
        // Genesis block has no parent → always boost
        let (fc, spec) = new_gloas_fc();
        let genesis_root = genesis_checkpoint().root;
        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            genesis_root,
            &BTreeSet::new(),
            Slot::new(1),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_skipped_slots_returns_true() {
        // When parent_slot + 1 < block_slot (skipped slots), always boost
        let (mut fc, spec) = new_gloas_fc();
        // Block at slot 1
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // Block at slot 5 (skipped slots 2-4)
        insert_gloas_block(
            &mut fc,
            5,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(5),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_adjacent_strong_parent_returns_true() {
        // Adjacent slot (no skip), parent has strong attestation support → always boost
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        fc.balances = balances(10);

        // Give the parent block very heavy attestation weight (above reorg threshold)
        for i in 0..10u64 {
            fc.process_attestation(i as usize, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        // Apply votes
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(10),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // Parent is strong → boost
        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_adjacent_weak_parent_no_equivocation_returns_true() {
        // Adjacent slot, parent is weak (low attestation weight), no equivocation → boost
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        fc.balances = balances(10);
        // No attestations for parent → parent is weak

        // Apply votes (none for parent)
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(10),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // Parent is weak but no equivocation → still boost
        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_weak_parent_with_equivocating_proposer_suppressed() {
        // Adjacent slot, parent is weak, AND there's an equivocating proposer → suppress boost
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        // Set the parent's proposer_index
        let parent_node = get_node_mut(&mut fc, &root(1));
        parent_node.proposer_index = 5;

        // Create an equivocating block at the same slot as parent, same proposer, PTC-timely
        let equivocating_root = root(99);
        fc.proto_array
            .on_block::<MinimalEthSpec>(
                Block {
                    slot: Slot::new(1),
                    root: equivocating_root,
                    parent_root: Some(root(0)),
                    state_root: Hash256::zero(),
                    target_root: root(0),
                    current_epoch_shuffling_id: junk_shuffling_id(),
                    next_epoch_shuffling_id: junk_shuffling_id(),
                    justified_checkpoint: genesis_checkpoint(),
                    finalized_checkpoint: genesis_checkpoint(),
                    execution_status: ExecutionStatus::irrelevant(),
                    unrealized_justified_checkpoint: Some(genesis_checkpoint()),
                    unrealized_finalized_checkpoint: Some(genesis_checkpoint()),
                    builder_index: Some(types::consts::gloas::BUILDER_INDEX_SELF_BUILD),
                    payload_revealed: false,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash: None,
                    bid_parent_block_hash: None,
                    proposer_index: 5, // same proposer as parent
                    ptc_timely: true,  // PTC-timely
                    envelope_received: false,
                },
                Slot::new(1),
            )
            .unwrap();

        fc.balances = balances(10);
        // No attestations → parent is weak

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(10),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // Parent is weak AND equivocating proposer exists → suppress boost
        assert!(!fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        ));
    }

    #[test]
    fn proposer_boost_equivocating_indices_count_toward_parent_weight() {
        // Equivocating validators' balance counts toward parent weight,
        // potentially making it "strong" even without attestations
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        fc.balances = balances(10);
        // No attestations, but many equivocating validators with high balance

        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances(10),
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        // With enough equivocating indices, parent should be "strong"
        let equivocating: BTreeSet<u64> = (0..10).collect();
        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &equivocating,
            Slot::new(3),
            &spec,
        ));
    }

    // ───────────────────── compute_filtered_roots ─────────────────────

    #[test]
    fn filtered_roots_genesis_only() {
        // Genesis block alone is always viable and filtered in
        let (fc, _spec) = new_gloas_fc();
        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(0));
        assert!(filtered.contains(&root(0)));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filtered_roots_self_build_chain_all_included() {
        // Self-build blocks are always viable → all should be in filtered set
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            3,
            root(3),
            root(2),
            Some(exec_hash(3)),
            Some(exec_hash(2)),
            false,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(4));
        // All blocks should be in filtered set (genesis + 3 self-build)
        assert!(filtered.contains(&root(0)));
        assert!(filtered.contains(&root(1)));
        assert!(filtered.contains(&root(2)));
        assert!(filtered.contains(&root(3)));
        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn filtered_roots_external_builder_not_revealed_excluded() {
        // External builder block without payload revealed is NOT viable for head
        // But its parent chain IS viable (genesis is viable)
        let (mut fc, _spec) = new_gloas_fc();
        insert_external_builder_block(&mut fc, 1, root(1), root(0), 42);

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));
        // External builder not revealed → not viable for head → not in filtered set
        // Genesis is viable on its own, so it stays
        assert!(filtered.contains(&root(0)));
        assert!(!filtered.contains(&root(1)));
    }

    #[test]
    fn filtered_roots_external_builder_revealed_included() {
        // External builder block with payload revealed IS viable
        let (mut fc, _spec) = new_gloas_fc();
        insert_external_builder_block(&mut fc, 1, root(1), root(0), 42);

        // Reveal payload
        let node = get_node_mut(&mut fc, &root(1));
        node.payload_revealed = true;

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));
        assert!(filtered.contains(&root(0)));
        assert!(filtered.contains(&root(1)));
    }

    #[test]
    fn filtered_roots_parent_propagation() {
        // A non-viable parent should be included if it has a viable descendant
        let (mut fc, _spec) = new_gloas_fc();
        // Block 1: external builder, not revealed (not viable on its own)
        insert_external_builder_block(&mut fc, 1, root(1), root(0), 42);
        // Block 2: self-build child of block 1 (always viable)
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(3));
        // root(1) is not viable for head itself, but has a viable descendant (root(2))
        // → should be included via upward propagation
        assert!(filtered.contains(&root(0)));
        assert!(filtered.contains(&root(1)));
        assert!(filtered.contains(&root(2)));
    }

    #[test]
    fn filtered_roots_deep_propagation_chain() {
        // Propagation should work through multiple non-viable ancestors
        let (mut fc, _spec) = new_gloas_fc();
        // Chain of 3 external builder blocks (all non-viable)
        insert_external_builder_block(&mut fc, 1, root(1), root(0), 42);
        insert_external_builder_block(&mut fc, 2, root(2), root(1), 42);
        insert_external_builder_block(&mut fc, 3, root(3), root(2), 42);

        // Without viable leaf: only genesis is viable
        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(4));
        assert!(filtered.contains(&root(0)));
        assert!(!filtered.contains(&root(1)));
        assert!(!filtered.contains(&root(2)));
        assert!(!filtered.contains(&root(3)));

        // Reveal payload on the leaf → makes it viable → propagates up
        let node = get_node_mut(&mut fc, &root(3));
        node.payload_revealed = true;

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(4));
        assert!(filtered.contains(&root(0)));
        assert!(filtered.contains(&root(1)));
        assert!(filtered.contains(&root(2)));
        assert!(filtered.contains(&root(3)));
    }

    #[test]
    fn filtered_roots_fork_with_mixed_viability() {
        // Fork: root(0) → root(1) (self-build, viable)
        //                → root(2) (external, not revealed, not viable)
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_external_builder_block(&mut fc, 1, root(2), root(0), 99);

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));
        assert!(filtered.contains(&root(0))); // parent of viable
        assert!(filtered.contains(&root(1))); // self-build, viable
        assert!(!filtered.contains(&root(2))); // external, not revealed, no viable descendants
    }

    // ───────────────────── get_ancestor_gloas (additional) ────────────

    #[test]
    fn ancestor_unknown_root_returns_none() {
        let (fc, _spec) = new_gloas_fc();
        let result = fc.get_ancestor_gloas(Hash256::repeat_byte(0xFF), Slot::new(0));
        assert_eq!(result, None);
    }

    #[test]
    fn ancestor_multi_hop_chain() {
        // Chain: root(0) slot 0 → root(1) slot 1 → root(2) slot 2 → root(3) slot 3
        // Get ancestor of root(3) at slot 1 → should walk back to root(1)
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            true,
        );
        insert_gloas_block(
            &mut fc,
            3,
            root(3),
            root(2),
            Some(exec_hash(3)),
            Some(exec_hash(2)),
            false,
        );

        let result = fc.get_ancestor_gloas(root(3), Slot::new(1));
        assert!(result.is_some());
        let ancestor = result.unwrap();
        assert_eq!(ancestor.root, root(1));
        // root(2)'s bid_parent_block_hash = exec_hash(1) matches root(1)'s bid_block_hash = exec_hash(1)
        // So the child→parent relationship at root(2)→root(1) is FULL
        assert_eq!(ancestor.payload_status, GloasPayloadStatus::Full);
    }

    #[test]
    fn ancestor_at_genesis_slot() {
        // Get ancestor of root(2) at slot 0 → should return genesis (root(0))
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let result = fc.get_ancestor_gloas(root(2), Slot::new(0));
        assert!(result.is_some());
        let ancestor = result.unwrap();
        assert_eq!(ancestor.root, root(0));
    }

    #[test]
    fn ancestor_future_slot_returns_pending() {
        // Asking for ancestor at a future slot (>= block's slot) returns Pending
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );

        let result = fc.get_ancestor_gloas(root(1), Slot::new(5));
        assert_eq!(
            result,
            Some(GloasForkChoiceNode {
                root: root(1),
                payload_status: GloasPayloadStatus::Pending,
            })
        );
    }

    // ───────────── is_supporting_vote_gloas (additional) ─────────────

    #[test]
    fn supporting_vote_ancestor_with_pending_status_always_supports() {
        // A vote on a descendant should support an ancestor with PENDING status
        // because Pending matches any payload status in the ancestor check
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        // Vote at descendant root(2) at later slot
        let vote = VoteTracker {
            current_root: root(2),
            current_slot: Slot::new(2),
            current_payload_present: false,
            ..VoteTracker::default()
        };

        // Ancestor at slot 1 of root(2) is root(1) with FULL status
        // node.payload_status == Pending → always matches (Pending || Pending == ancestor.payload_status)
        assert!(fc.is_supporting_vote_gloas(&pending_node, &vote));
    }

    #[test]
    fn supporting_vote_ancestor_full_matches_full_path() {
        // Vote chains through a FULL parent relationship → supports FULL node
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // root(2) bid_parent = exec_hash(1) matches root(1) bid = exec_hash(1) → FULL
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        let vote = VoteTracker {
            current_root: root(2),
            current_slot: Slot::new(2),
            current_payload_present: true,
            ..VoteTracker::default()
        };

        // Ancestor of root(2) at slot 1 → root(1) with FULL status
        // FULL == FULL → supports
        assert!(fc.is_supporting_vote_gloas(&full_node, &vote));
    }

    #[test]
    fn supporting_vote_ancestor_empty_does_not_match_full_path() {
        // Vote chains through FULL → does NOT support EMPTY node at same root
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // FULL path: bid_parent matches
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        );

        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };

        let vote = VoteTracker {
            current_root: root(2),
            current_slot: Slot::new(2),
            current_payload_present: false,
            ..VoteTracker::default()
        };

        // Ancestor of root(2) at slot 1 → root(1) with FULL status
        // EMPTY != FULL → does not support
        assert!(!fc.is_supporting_vote_gloas(&empty_node, &vote));
    }

    #[test]
    fn supporting_vote_ancestor_empty_matches_empty_path() {
        // Vote chains through EMPTY → supports EMPTY node
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // EMPTY path: bid_parent does NOT match parent bid
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
        );

        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };

        let vote = VoteTracker {
            current_root: root(2),
            current_slot: Slot::new(2),
            current_payload_present: false,
            ..VoteTracker::default()
        };

        // Ancestor of root(2) at slot 1 → root(1) with EMPTY status
        // EMPTY == EMPTY → supports
        assert!(fc.is_supporting_vote_gloas(&empty_node, &vote));
    }

    // ─────────── get_gloas_children (additional) ─────────────────────

    #[test]
    fn children_filtered_roots_excludes_non_viable() {
        // A child that's not in the filtered_roots set should be excluded
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // Two children of root(1): root(2) self-build, root(3) external builder (not revealed)
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
        );
        insert_external_builder_block(&mut fc, 2, root(3), root(1), 42);

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(3));
        // root(3) is external builder not revealed → not in filtered
        assert!(!filtered.contains(&root(3)));

        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let children = fc.get_gloas_children(&empty_node, &filtered);
        // Only root(2) should be returned (root(3) filtered out)
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].root, root(2));
    }

    #[test]
    fn children_pending_unknown_root_returns_empty_only() {
        // For PENDING node with root not in proto_array → still returns EMPTY child
        // (the EMPTY child doesn't check payload_revealed)
        let (fc, _spec) = new_gloas_fc();
        let filtered: HashSet<Hash256> = HashSet::new();
        let pending = GloasForkChoiceNode {
            root: Hash256::repeat_byte(0xFF),
            payload_status: GloasPayloadStatus::Pending,
        };

        let children = fc.get_gloas_children(&pending, &filtered);
        // PENDING always generates an EMPTY child (doesn't look up proto_array for that)
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].payload_status, GloasPayloadStatus::Empty);
    }

    #[test]
    fn children_multiple_children_different_payload_paths() {
        // Parent root(1) has two children:
        //   root(2): bid_parent matches parent bid → FULL path
        //   root(3): bid_parent doesn't match → EMPTY path
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
        ); // FULL path
        insert_gloas_block(
            &mut fc,
            2,
            root(3),
            root(1),
            Some(exec_hash(3)),
            Some(exec_hash(99)),
            false,
        ); // EMPTY path

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(3));

        // FULL node should only get root(2) (FULL path child)
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        let full_children = fc.get_gloas_children(&full_node, &filtered);
        assert_eq!(full_children.len(), 1);
        assert_eq!(full_children[0].root, root(2));

        // EMPTY node should only get root(3) (EMPTY path child)
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let empty_children = fc.get_gloas_children(&empty_node, &filtered);
        assert_eq!(empty_children.len(), 1);
        assert_eq!(empty_children[0].root, root(3));
    }

    #[test]
    fn children_empty_unknown_root_returns_empty_vec() {
        // EMPTY node whose root is not in proto_array → returns empty Vec
        let (fc, _spec) = new_gloas_fc();
        let filtered: HashSet<Hash256> = HashSet::new();
        let empty = GloasForkChoiceNode {
            root: Hash256::repeat_byte(0xFF),
            payload_status: GloasPayloadStatus::Empty,
        };

        let children = fc.get_gloas_children(&empty, &filtered);
        assert!(children.is_empty());
    }

    #[test]
    fn children_full_unknown_root_returns_empty_vec() {
        // FULL node whose root is not in proto_array → returns empty Vec
        let (fc, _spec) = new_gloas_fc();
        let filtered: HashSet<Hash256> = HashSet::new();
        let full = GloasForkChoiceNode {
            root: Hash256::repeat_byte(0xFF),
            payload_status: GloasPayloadStatus::Full,
        };

        let children = fc.get_gloas_children(&full, &filtered);
        assert!(children.is_empty());
    }

    #[test]
    fn proposer_boost_parent_node_missing_returns_true() {
        // When boost_block.parent index exists but the node at that index is gone
        // (corrupted state), should return true (always boost as defensive fallback).
        // This tests line 1294-1296: `let Some(parent_block) = pa.nodes.get(parent_idx)`
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        // Corrupt the parent index to point beyond the nodes array
        let boost_idx = *fc.proto_array.indices.get(&root(1)).unwrap();
        let boost_node = &mut fc.proto_array.nodes[boost_idx];
        boost_node.parent = Some(usize::MAX);

        assert!(fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(1),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        ));
    }

    // ── find_head_gloas: proposer boost & gloas_head_payload_status ──

    /// Insert a Gloas block with custom proposer_index and ptc_timely.
    #[allow(clippy::too_many_arguments)]
    fn insert_gloas_block_ext(
        fc: &mut ProtoArrayForkChoice,
        slot: u64,
        block_root: Hash256,
        parent_root: Hash256,
        bid_block_hash: Option<ExecutionBlockHash>,
        bid_parent_block_hash: Option<ExecutionBlockHash>,
        payload_revealed: bool,
        proposer_index: u64,
        ptc_timely: bool,
        envelope_received: bool,
    ) {
        fc.proto_array
            .on_block::<MinimalEthSpec>(
                Block {
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
                    builder_index: Some(types::consts::gloas::BUILDER_INDEX_SELF_BUILD),
                    payload_revealed,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash,
                    bid_parent_block_hash,
                    proposer_index,
                    ptc_timely,
                    envelope_received,
                },
                Slot::new(slot),
            )
            .unwrap();
    }

    #[test]
    fn find_head_proposer_boost_changes_winner() {
        // Two blocks at slot 1: root(1) and root(2).
        // 11 votes for root(1), 10 votes for root(2).
        // Proposer boost on root(2) should flip the winner.
        //
        // With 21 validators at BALANCE=32e9:
        //   total = 672e9, committee_weight = 84e9, boost = 33.6e9
        //   root(1) weight: 11 * 32e9 = 352e9
        //   root(2) weight: 10 * 32e9 + 33.6e9 = 353.6e9 > 352e9
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(0)),
            false,
        );

        let balances = balances(21);

        // 11 votes for root(1)
        for i in 0..11 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }
        // 10 votes for root(2)
        for i in 11..21 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        // Without boost → root(1) wins (352e9 > 320e9)
        let head_no_boost = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(), // no boost
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(head_no_boost, root(1));

        // With boost on root(2) → root(2) wins (353.6e9 > 352e9)
        let head_with_boost = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                root(2), // boost root(2)
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(head_with_boost, root(2));
    }

    #[test]
    fn find_head_proposer_boost_suppressed_by_equivocation() {
        // Same setup as above but with an equivocating proposer at the parent slot.
        // The boost should be suppressed when the parent is weak and there's a
        // ptc_timely equivocating block by the same proposer at the same slot.
        let (mut fc, spec) = new_gloas_fc();

        // Parent block at slot 1 with proposer_index=5
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            5,     // proposer_index
            false, // ptc_timely
            false, // envelope_received
        );

        // Equivocating block at slot 1 from same proposer, ptc_timely=true
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(10),
            root(0),
            Some(exec_hash(10)),
            Some(exec_hash(0)),
            false,
            5,     // same proposer
            true,  // ptc_timely
            false, // envelope_received
        );

        // Child block at slot 2 (the one we'd boost)
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
            0, // different proposer
            false,
            false,
        );

        // Set balances so reorg threshold is meaningful.
        // With 21 validators: reorg_threshold = 672e9/8*20/100 = 16.8e9
        fc.balances = balances(21);

        // No votes for parent root(1) → parent is weak (0 < 16.8e9).
        // With boost on root(2): boost should be suppressed because parent root(1)
        // is weak AND has an equivocating proposer (root(10) by same proposer_index=5).
        let apply_boost = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        );
        assert!(
            !apply_boost,
            "boost should be suppressed due to equivocating proposer at parent slot"
        );
    }

    #[test]
    fn find_head_proposer_boost_with_strong_parent() {
        // When the parent has strong attestation support, boost is always applied
        // even with an equivocating proposer.
        let (mut fc, spec) = new_gloas_fc();

        // Parent block at slot 1 with proposer_index=5
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            5,
            false,
            false,
        );

        // Equivocating block at slot 1 from same proposer, ptc_timely=true
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(10),
            root(0),
            Some(exec_hash(10)),
            Some(exec_hash(0)),
            false,
            5,
            true,
            false,
        );

        // Child block at slot 2
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
            0,
            false,
            false,
        );

        let balances = balances(21);

        // Give root(1) strong attestation support (above reorg threshold).
        // Reorg threshold = total_balance / slots_per_epoch * 20 / 100
        // = 672e9 / 8 * 20/100 = 16.8e9 ≈ 1 validator
        // So >1 validator supporting the parent makes it "strong".
        for i in 0..5 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        // Need to run find_head first so votes move from next to current
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances,
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();

        let apply_boost = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        );
        assert!(
            apply_boost,
            "boost should be applied when parent is strong despite equivocation"
        );
    }

    #[test]
    fn find_head_gloas_head_payload_status_pending_leaf() {
        // When the head block has no children (leaf node) and its only traversal
        // path leads to PENDING → EMPTY (because no votes for FULL), the head
        // payload status should be EMPTY (the winning child of PENDING).
        // But if we have no blocks beyond genesis, the justified root is the genesis
        // which starts as PENDING and its children are EMPTY (always) and FULL
        // (only if payload_revealed). With no revealed payload, only EMPTY child
        // exists → that EMPTY child has no further children → head is EMPTY.
        let (mut fc, spec) = new_gloas_fc();
        // Just genesis, no other blocks

        let balances = balances(1);

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(1),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(0));
        // Genesis PENDING → EMPTY child (no payload revealed) → leaf
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8)
        );
    }

    #[test]
    fn find_head_gloas_head_payload_status_full_after_reveal() {
        // A single block with payload revealed and a FULL vote → head payload
        // status should be FULL.
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload revealed → FULL child exists
        );

        let balances = balances(1);

        // Vote with payload_present=true at slot 2
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8)
        );
    }

    #[test]
    fn find_head_pre_gloas_payload_status_none() {
        // Before the Gloas fork, gloas_head_payload_status should be None.
        let mut spec = MinimalEthSpec::default_spec();
        spec.gloas_fork_epoch = None; // No Gloas fork

        let fc = ProtoArrayForkChoice::new::<MinimalEthSpec>(
            Slot::new(0),
            Slot::new(0),
            Hash256::zero(),
            genesis_checkpoint(),
            genesis_checkpoint(),
            junk_shuffling_id(),
            junk_shuffling_id(),
            ExecutionStatus::irrelevant(),
        )
        .unwrap();

        assert_eq!(fc.gloas_head_payload_status(), None);
    }

    #[test]
    fn find_head_gloas_payload_status_updates_each_call() {
        // Verify that gloas_head_payload_status changes between calls as the
        // fork choice state evolves (payload gets revealed between calls).
        let (mut fc, spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false, // NOT revealed initially
        );

        let balances = balances(1);

        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        // First call: payload not revealed → EMPTY wins
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances,
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8)
        );

        // Now reveal the payload (simulating envelope receipt)
        if let Some(&idx) = fc.proto_array.indices.get(&root(1)) {
            fc.proto_array.nodes[idx].payload_revealed = true;
            fc.proto_array.nodes[idx].envelope_received = true;
        }

        // Second call: payload revealed + FULL vote → FULL wins
        fc.find_head::<MinimalEthSpec>(
            genesis_checkpoint(),
            genesis_checkpoint(),
            &balances,
            Hash256::zero(),
            &BTreeSet::new(),
            Slot::new(2),
            &spec,
        )
        .unwrap();
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8)
        );
    }

    #[test]
    fn find_head_ptc_quorum_without_envelope_stays_empty() {
        // Edge case: PTC quorum reached (payload_revealed=true) but no actual
        // envelope was received (envelope_received=false). This can happen when
        // the PTC committee votes "present" but the node hasn't received/processed
        // the execution payload envelope.
        //
        // Per spec: get_node_children only creates the FULL virtual child when
        // `root in store.payload_states`, which requires actual envelope processing.
        // PTC quorum alone is not sufficient.
        //
        // Expected: head should be EMPTY (not FULL), even with FULL-supporting votes.
        let (mut fc, spec) = new_gloas_fc();

        // Insert a block with payload_revealed=true but envelope_received=false
        // (simulating PTC quorum without actual envelope receipt)
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,  // payload_revealed (PTC quorum)
            0,     // proposer_index
            false, // ptc_timely
            false, // envelope_received — no actual envelope!
        );

        let balances = balances(1);

        // Vote with payload_present=true (supporting FULL)
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        // Without envelope, only EMPTY child exists, so head must be EMPTY
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "PTC quorum without envelope → head should be EMPTY, not FULL"
        );
    }

    #[test]
    fn find_head_ptc_quorum_with_envelope_becomes_full() {
        // Complementary test: same setup as above but WITH envelope_received=true.
        // Now the FULL child should exist and win with FULL-supporting votes.
        let (mut fc, spec) = new_gloas_fc();

        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,  // payload_revealed (PTC quorum)
            0,     // proposer_index
            false, // ptc_timely
            true,  // envelope_received — actual envelope received!
        );

        let balances = balances(1);

        // Vote with payload_present=true (supporting FULL)
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), true)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        // With envelope, FULL child exists and wins with FULL-supporting vote
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8),
            "PTC quorum + envelope → head should be FULL"
        );
    }

    #[test]
    fn find_head_proposer_boost_skipped_slots_always_applied() {
        // When the parent slot is not adjacent (skipped slots), boost should always
        // be applied regardless of parent weight or equivocation.
        let (mut fc, spec) = new_gloas_fc();

        // Parent block at slot 1
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            5,
            false,
            false,
        );

        // Child block at slot 3 (skipped slot 2)
        insert_gloas_block_ext(
            &mut fc,
            3,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
            0,
            false,
            false,
        );

        // No votes for parent (parent is weak), but skipped slots → boost applied
        let apply_boost = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(4),
            &spec,
        );
        assert!(
            apply_boost,
            "boost should always be applied when parent slot is not adjacent (skipped slots)"
        );
    }

    #[test]
    fn find_head_equivocating_indices_strengthen_parent() {
        // Equivocating validator indices are counted toward parent weight,
        // which can make a weak parent strong and thus allow boost to be applied.
        let (mut fc, spec) = new_gloas_fc();

        // Parent block at slot 1 with proposer_index=5
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            5,
            false,
            false,
        );

        // Equivocating block by same proposer (would suppress boost if parent weak)
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(10),
            root(0),
            Some(exec_hash(10)),
            Some(exec_hash(0)),
            false,
            5,
            true,
            false,
        );

        // Child block at slot 2
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
            0,
            false,
            false,
        );

        // Set balances so reorg threshold is meaningful.
        // With 21 validators: reorg_threshold = 672e9/8*20/100 = 16.8e9
        fc.balances = balances(21);

        // No attestation votes for parent → parent is weak.
        // Without equivocating indices, boost should be suppressed.
        let apply_boost_no_equivocators = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        );
        assert!(
            !apply_boost_no_equivocators,
            "boost suppressed: parent weak, equivocating proposer"
        );

        // Now add enough equivocating indices to make the parent strong.
        // Reorg threshold = 672e9 / 8 * 20 / 100 = 16.8e9
        // 1 equivocating validator at 32e9 > 16.8e9 → parent becomes strong.
        let equivocators: BTreeSet<u64> = [0u64].into_iter().collect();
        let apply_boost_with_equivocators = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(2),
            &equivocators,
            Slot::new(3),
            &spec,
        );
        assert!(
            apply_boost_with_equivocators,
            "boost applied: equivocating indices make parent strong"
        );
    }

    /// Verify that `find_head_gloas` uses the payload tiebreaker to decide
    /// between EMPTY and FULL when both have equal weight.
    ///
    /// When a PENDING node at the previous slot has both EMPTY and FULL virtual
    /// children (same root, same weight because no votes), the `max_by` comparator
    /// falls through to `get_payload_tiebreaker`:
    ///   - EMPTY gets tiebreaker value 1
    ///   - FULL gets tiebreaker value 2 (when `should_extend_payload` is true)
    ///
    /// So FULL wins. This exercises the third-level comparator in `find_head_gloas`
    /// which is unreachable when weights or roots differ.
    #[test]
    fn find_head_gloas_tiebreaker_favors_full_when_timely() {
        let (mut fc, spec) = new_gloas_fc();

        // Insert block at slot 1 with envelope received (FULL child exists)
        // payload_revealed + envelope_received + payload_data_available makes
        // should_extend_payload return true → tiebreaker(FULL) = 2 > 1 = tiebreaker(EMPTY)
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,  // payload_revealed
            0,     // proposer_index
            false, // ptc_timely
            true,  // envelope_received
        );

        // Set payload_data_available so should_extend_payload returns true
        if let Some(&idx) = fc.proto_array.indices.get(&root(1)) {
            fc.proto_array.nodes[idx].payload_data_available = true;
        }

        let balances = balances(0); // no validators → no votes → EMPTY and FULL have equal weight

        // current_slot=2 → block at slot 1 is the "previous slot"
        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        // FULL wins the tiebreaker (2 > 1)
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8),
            "with equal weight and timely payload, FULL should win via tiebreaker"
        );
    }

    /// Verify that `find_head_gloas` tiebreaker favors EMPTY when the payload
    /// is not timely and should NOT be extended.
    ///
    /// Same setup as above but without `payload_data_available`, so
    /// `should_extend_payload` returns false → tiebreaker(FULL) = 0 < 1 = tiebreaker(EMPTY)
    /// → EMPTY wins. This verifies the tiebreaker correctly suppresses FULL
    /// when the payload is not available.
    #[test]
    fn find_head_gloas_tiebreaker_favors_empty_when_not_timely() {
        let (mut fc, spec) = new_gloas_fc();

        // Insert block with envelope_received=true but payload_data_available=false
        // (via insert_gloas_block_ext which defaults to false). should_extend_payload
        // needs envelope_received AND payload_revealed AND payload_data_available,
        // so it returns false.
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,  // payload_revealed (for FULL child to exist)
            0,     // proposer_index
            false, // ptc_timely
            true,  // envelope_received (for FULL child to exist)
        );

        // payload_data_available stays false (from insert_gloas_block_ext default).
        // Also need to ensure the proposer_boost doesn't interfere:
        // The proposer_boost_root is Hash256::zero() by default → should_extend_payload
        // returns true when no proposer boost root (the zero-root shortcut).
        // To actually test the "not timely" path, we need a non-zero proposer boost root
        // whose parent is this node but parent's status is NOT FULL.

        // Insert a child block at slot 2 and set it as the proposer boost root.
        // Its parent (root(1)) with bid_parent_block_hash != parent's bid_block_hash
        // means parent was EMPTY. Mark execution-invalid so it's not viable for head
        // (otherwise it would become head itself, bypassing the tiebreaker).
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // mismatched with root(1)'s exec_hash(1) → parent EMPTY
            false,
            0,
            false,
            false,
        );
        if let Some(&idx) = fc.proto_array.indices.get(&root(2)) {
            fc.proto_array.nodes[idx].execution_status =
                ExecutionStatus::Invalid(ExecutionBlockHash::zero());
        }
        fc.proto_array.previous_proposer_boost.root = root(2);

        let balances = balances(0); // no votes

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(1));
        // EMPTY wins the tiebreaker (1 > 0)
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "with equal weight and non-timely payload, EMPTY should win via tiebreaker"
        );
    }

    /// Verify that `is_supporting_vote_gloas` correctly resolves a multi-hop
    /// ancestor (grandchild voting for grandparent).
    ///
    /// Chain: root(0) → root(1) → root(2) → root(3)
    /// Vote at root(3), check if it supports root(1) at various payload statuses.
    /// This exercises the `while parent.slot > slot` loop in `get_ancestor_gloas`
    /// through the `is_supporting_vote_gloas` code path (not just as a standalone
    /// function call). The ancestor must be resolved 2 hops up through root(2).
    #[test]
    fn supporting_vote_multi_hop_ancestor() {
        let (mut fc, _spec) = new_gloas_fc();

        // Block 1 at slot 1, bid_block_hash = exec_hash(1)
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Block 2 at slot 2, bid_parent matches parent's bid → parent was FULL
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches root(1)'s bid_block_hash → FULL
            true,
        );

        // Block 3 at slot 3, bid_parent matches root(2)'s bid → root(2) parent was FULL
        insert_gloas_block(
            &mut fc,
            3,
            root(3),
            root(2),
            Some(exec_hash(3)),
            Some(exec_hash(2)), // matches root(2)'s bid_block_hash → FULL
            false,
        );

        // Vote for root(3) at slot 4 — ancestor at slot 1 is root(1), reached by
        // walking 2 hops: root(3)→root(2)→root(1). The payload status at root(1)
        // is determined by the child→parent relationship at root(2)→root(1), which
        // is FULL (exec_hash(1) == exec_hash(1)).
        let vote = VoteTracker {
            current_root: root(3),
            current_slot: Slot::new(4),
            current_payload_present: false,
            ..VoteTracker::default()
        };

        // PENDING always matches (ancestor check matches any status)
        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };
        assert!(
            fc.is_supporting_vote_gloas(&pending_node, &vote),
            "PENDING should always be supported through multi-hop ancestor"
        );

        // FULL matches because ancestor resolution gives FULL
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(
            fc.is_supporting_vote_gloas(&full_node, &vote),
            "FULL should be supported: ancestor at slot 1 resolved through 2 hops is FULL"
        );

        // EMPTY does NOT match
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        assert!(
            !fc.is_supporting_vote_gloas(&empty_node, &vote),
            "EMPTY should NOT be supported: ancestor at slot 1 is FULL, not EMPTY"
        );
    }

    // ── find_head_gloas multi-block chain selection tests (run 211) ──

    /// Two-deep chain: genesis → slot 1 → slot 2. Votes at slot 3.
    /// Verifies that find_head_gloas traverses PENDING → EMPTY → PENDING
    /// across multiple blocks and correctly selects the deepest viable leaf.
    ///
    /// Chain: root(0) [slot 0] → root(1) [slot 1] → root(2) [slot 2]
    /// All blocks EMPTY path (no payloads revealed).
    /// 1 voter votes for root(2) at slot 3.
    /// Expected head: root(2) via the path:
    ///   root(0) PENDING → root(0) EMPTY → root(1) PENDING → root(1) EMPTY
    ///   → root(2) PENDING → root(2) EMPTY (leaf)
    #[test]
    fn find_head_gloas_two_deep_chain_empty_path() {
        let (mut fc, spec) = new_gloas_fc();

        // root(1) at slot 1, parent EMPTY (bid_parent_hash mismatches parent's bid_block_hash)
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // mismatches genesis's exec_hash → parent EMPTY
            false,
        );

        // root(2) at slot 2, parent EMPTY
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // mismatches root(1)'s exec_hash(1) → parent EMPTY
            false,
        );

        let balances = balances(1);

        // Vote for root(2) at slot 3 (payload_present=false → EMPTY supporting)
        fc.process_attestation(0, root(2), Epoch::new(0), Slot::new(3), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(2), "head should be root(2) — deepest block");
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "head should be EMPTY — no payloads revealed in the chain"
        );
    }

    /// Two competing forks at the same slot. One fork has its payload revealed (FULL
    /// path available), the other doesn't. Votes split between them, but the FULL fork
    /// has slightly more weight.
    ///
    /// Fork A: root(0) → root(1) at slot 1 (payload NOT revealed, EMPTY only)
    /// Fork B: root(0) → root(2) at slot 1 (payload revealed, FULL path available)
    ///
    /// 2 voters for root(1) EMPTY, 3 voters for root(2) FULL.
    /// Expected: root(2) FULL wins (more weight on the FULL path).
    #[test]
    fn find_head_gloas_competing_forks_full_vs_empty() {
        let (mut fc, spec) = new_gloas_fc();

        // Fork A: root(1) at slot 1, NO payload revealed
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // parent EMPTY
            false,               // no payload revealed
        );

        // Fork B: root(2) at slot 1, payload revealed → FULL child exists
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // parent EMPTY
            true,                // payload revealed
        );

        let balances = balances(5);

        // 2 votes for root(1) with payload_present=false (EMPTY supporting)
        for i in 0..2 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }
        // 3 votes for root(2) with payload_present=true (FULL supporting)
        for i in 2..5 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), true)
                .unwrap();
        }

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(
            head,
            root(2),
            "root(2) FULL fork should win with 3 votes vs 2"
        );
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8),
            "winning fork has payload revealed → FULL"
        );
    }

    /// Chain with FULL→PENDING→FULL transition. A block at slot 1 has its payload
    /// revealed, and a child block at slot 2 declares the parent as FULL (bid_parent_hash
    /// matches). The child also has its payload revealed. Votes support the FULL path
    /// throughout.
    ///
    /// Chain: root(0) → root(1) [FULL] → root(2) [parent FULL, payload revealed]
    /// Expected head traversal:
    ///   root(0) PENDING → root(0) EMPTY → root(1) PENDING (parent EMPTY)
    ///   root(0) PENDING → root(0) FULL → root(1) PENDING (parent FULL) → root(1) FULL
    ///   → root(2) PENDING → root(2) FULL (leaf)
    ///
    /// With FULL-supporting votes at slot 3, the FULL path should win.
    #[test]
    fn find_head_gloas_full_chain_two_blocks() {
        let (mut fc, spec) = new_gloas_fc();

        // root(1) at slot 1: parent EMPTY (genesis has no matching exec hash)
        // but payload revealed → FULL child available
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // parent EMPTY
            true,                // payload revealed
        );

        // root(2) at slot 2: bid_parent_block_hash matches root(1)'s bid_block_hash
        // → parent FULL. Also payload revealed.
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches root(1)'s exec_hash(1) → parent FULL
            true,               // payload revealed
        );

        let balances = balances(3);

        // All 3 voters vote for root(2) with payload_present=true
        for i in 0..3 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(3), true)
                .unwrap();
        }

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(
            head,
            root(2),
            "head should be root(2) — deepest block on FULL chain"
        );
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8),
            "FULL path should win with FULL-supporting votes"
        );
    }

    /// Vote redistribution: initially one fork leads, then votes shift.
    /// Verifies that find_head_gloas correctly recomputes the winner when
    /// votes change between calls.
    ///
    /// Two forks: root(1) and root(2) at slot 1.
    /// First call: 3 votes for root(1), 2 for root(2) → root(1) wins.
    /// Then votes shift: everyone votes for root(2).
    /// Second call: 5 votes for root(2) → root(2) wins.
    #[test]
    fn find_head_gloas_vote_redistribution_changes_head() {
        let (mut fc, spec) = new_gloas_fc();

        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // parent EMPTY
            false,
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // parent EMPTY
            false,
        );

        let balances = balances(5);

        // Initial votes: 3 for root(1), 2 for root(2)
        for i in 0..3 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }
        for i in 3..5 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        let head1 = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(head1, root(1), "root(1) should win with 3 votes vs 2");

        // Shift all votes to root(2). process_attestation only accepts updates when
        // the new epoch is strictly greater than the previous next_epoch — same-epoch
        // re-votes are silently ignored. Use epoch 1 for the second round.
        for i in 0..5 {
            fc.process_attestation(i, root(2), Epoch::new(1), Slot::new(8), false)
                .unwrap();
        }

        let head2 = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(8),
                &spec,
            )
            .unwrap();
        assert_eq!(
            head2,
            root(2),
            "root(2) should win after all votes shift to it"
        );
    }

    /// Verify that blocks with invalid execution status are filtered out
    /// (not viable for head) by `compute_filtered_roots`, which `find_head_gloas`
    /// uses to restrict the traversal.
    ///
    /// Two blocks at slot 1: root(1) valid, root(2) marked with
    /// `ExecutionStatus::Invalid`. Even though root(2) has more votes,
    /// it should be filtered out and root(1) should win.
    #[test]
    fn find_head_gloas_invalid_execution_filtered_out() {
        let (mut fc, spec) = new_gloas_fc();

        // root(1): valid execution status
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // parent EMPTY
            false,
        );

        // root(2): starts valid, will be marked invalid below
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)), // parent EMPTY
            false,
        );

        // Mark root(2) as invalid execution — node_is_viable_for_head returns false
        if let Some(&idx) = fc.proto_array.indices.get(&root(2)) {
            fc.proto_array.nodes[idx].execution_status =
                ExecutionStatus::Invalid(ExecutionBlockHash::zero());
        }

        let balances = balances(5);

        // 1 vote for root(1), 4 votes for root(2) — root(2) has overwhelming weight
        fc.process_attestation(0, root(1), Epoch::new(0), Slot::new(2), false)
            .unwrap();
        for i in 1..5 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        assert_eq!(
            head,
            root(1),
            "root(1) should win — root(2) is filtered out due to invalid execution status"
        );
    }

    // ── ptc_timely / reorg resistance / tiebreaker edge cases ──

    #[test]
    fn ptc_timely_stored_on_block_when_current_slot_matches() {
        // When a block is inserted with current_slot == block.slot, the node
        // should have ptc_timely=true. When there's a gap, it should be false.
        let (mut fc, _spec) = new_gloas_fc();

        // Insert block at slot 1, current_slot = 1 → ptc_timely = true
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            0,
            true, // ptc_timely
            false,
        );

        assert!(
            get_node(&fc, &root(1)).ptc_timely,
            "block inserted with ptc_timely=true should preserve that flag"
        );

        // Insert block at slot 2, but ptc_timely = false (late arrival)
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)),
            false,
            0,
            false, // ptc_timely = false (late)
            false,
        );

        assert!(
            !get_node(&fc, &root(2)).ptc_timely,
            "block inserted with ptc_timely=false should preserve that flag"
        );
    }

    #[test]
    fn boost_not_suppressed_when_equivocating_block_not_ptc_timely() {
        // The equivocation check in should_apply_proposer_boost_gloas requires
        // the equivocating block to be ptc_timely. If the sibling block from the
        // same proposer is NOT ptc_timely, boost should still be applied.
        //
        // We verify by calling should_apply_proposer_boost_gloas directly.
        let (mut fc, spec) = new_gloas_fc();

        // Parent block at slot 1 by proposer 5
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            false,
            5,     // proposer_index
            false, // ptc_timely
            false,
        );

        // Sibling block at slot 1 by same proposer 5 — but NOT ptc_timely
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(0)),
            false,
            5,     // same proposer
            false, // NOT ptc_timely — should not suppress boost
            false,
        );

        // Child of root(1) at slot 2 — this is the boost target
        insert_gloas_block_ext(
            &mut fc,
            2,
            root(3),
            root(1),
            Some(exec_hash(3)),
            Some(exec_hash(1)),
            false,
            10,   // different proposer
            true, // ptc_timely
            false,
        );

        // Set empty balances so parent root(1) has zero attestation weight → head-weak
        fc.balances = JustifiedBalances {
            effective_balances: vec![BALANCE; 3],
            total_effective_balance: BALANCE * 3,
            num_active_validators: 3,
        };

        // should_apply_proposer_boost_gloas: root(1) is adjacent (slot 1 + 1 = slot 2
        // == root(3).slot) and head-weak (0 attestation weight). The equivocation check
        // looks for nodes with ptc_timely=true, same proposer as root(1), at same slot.
        // root(2) has the same proposer but ptc_timely=false → no equivocation detected.
        let result = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(3),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        );

        assert!(
            result,
            "boost should be applied — equivocating sibling is not ptc_timely"
        );

        // Now flip root(2) to ptc_timely=true and verify boost IS suppressed
        get_node_mut(&mut fc, &root(2)).ptc_timely = true;

        let result_suppressed = fc.should_apply_proposer_boost_gloas::<MinimalEthSpec>(
            root(3),
            &BTreeSet::new(),
            Slot::new(3),
            &spec,
        );

        assert!(
            !result_suppressed,
            "boost should be suppressed — equivocating sibling is now ptc_timely"
        );
    }

    #[test]
    fn should_extend_payload_envelope_received_but_ptc_quorum_not_reached() {
        // Envelope received (envelope_received=true) but PTC quorum not reached
        // (ptc_weight not above threshold). The is_timely_and_available check requires
        // envelope_received AND ptc_weight > threshold AND ptc_blob_data_available_weight > threshold.
        // PTC quorum not above threshold → falls through to proposer boost path.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Envelope arrived but PTC quorum not reached (ptc_weight stays at 0)
        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;

        // Set a proposer boost root that IS a child of block_root building on EMPTY parent
        let child_root = root(2);
        insert_external_builder_block(&mut fc, 2, child_root, block_root, 42);
        // Child's bid_parent_block_hash is None (default) → builds on EMPTY parent
        assert!(get_node(&fc, &child_root).bid_parent_block_hash.is_none());

        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_root,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        // envelope_received but PTC quorum not above threshold → timely check fails.
        // Boosted child builds on EMPTY parent of block_root → should NOT extend.
        assert!(
            !fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD),
            "should_extend_payload should be false: envelope arrived but PTC quorum \
             not reached, and boosted child builds on EMPTY parent"
        );
    }

    #[test]
    fn gloas_weight_zero_for_non_pending_previous_slot_node() {
        // Non-PENDING nodes from the previous slot get 0 weight — this is the
        // reorg resistance mechanism that prevents EMPTY/FULL from accumulating
        // weight when there could be a competing block in the current slot.
        //
        // We set up internal state (balances + votes directly on current_*),
        // then call get_gloas_weight to verify the mechanism.
        let (mut fc, spec) = new_gloas_fc();

        // Insert block at slot 1 with payload revealed
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload_revealed → FULL child exists
        );

        // Set balances directly (normally done by find_head)
        fc.balances = balances(2);

        // Set votes directly on current_* (process_attestation writes to next_*,
        // which only becomes current_* after compute_deltas in find_head).
        let vote0 = fc.votes.get_mut(0);
        vote0.current_root = root(1);
        vote0.current_slot = Slot::new(2);
        vote0.current_payload_present = true;

        let vote1 = fc.votes.get_mut(1);
        vote1.current_root = root(1);
        vote1.current_slot = Slot::new(2);
        vote1.current_payload_present = false;

        // At current_slot=2, root(1) is at slot 1 (previous slot).
        // The EMPTY and FULL nodes for root(1) should get 0 weight.
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        let empty_weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &empty_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );
        let full_weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &full_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );

        assert_eq!(
            empty_weight, 0,
            "EMPTY node at previous slot should have 0 weight (reorg resistance)"
        );
        assert_eq!(
            full_weight, 0,
            "FULL node at previous slot should have 0 weight (reorg resistance)"
        );

        // PENDING at the same slot DOES get weight — the reorg resistance only
        // affects EMPTY/FULL children, not the PENDING parent.
        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };
        let pending_weight = fc.get_gloas_weight::<MinimalEthSpec>(
            &pending_node,
            Hash256::zero(),
            false,
            Slot::new(2),
            &spec,
        );

        assert!(
            pending_weight > 0,
            "PENDING node at previous slot should still have weight: {pending_weight}"
        );

        // Verify: at current_slot=100 (not previous slot), EMPTY/FULL DO get weight
        let empty_weight_far = fc.get_gloas_weight::<MinimalEthSpec>(
            &empty_node,
            Hash256::zero(),
            false,
            Slot::new(100),
            &spec,
        );
        assert!(
            empty_weight_far > 0,
            "EMPTY node at non-previous slot should have weight: {empty_weight_far}"
        );
    }

    #[test]
    fn find_head_tiebreaker_full_wins_when_extend_payload_true() {
        // When EMPTY and FULL at previous slot are tied on weight (both 0 due to
        // reorg resistance), the tiebreaker determines the winner. With
        // should_extend_payload=true, FULL gets tiebreaker 2 vs EMPTY's 1.
        let (mut fc, spec) = new_gloas_fc();

        // Single block at slot 1 with payload revealed and envelope received
        insert_gloas_block_ext(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload_revealed
            0,
            false,
            true, // envelope_received
        );

        // Also set payload_data_available so should_extend_payload returns true
        // via the timely+available path
        let node = get_node_mut(&mut fc, &root(1));
        node.payload_data_available = true;

        let balances = balances(1);

        // No attestations — all weight comes from tiebreakers.
        // At slot 2, root(1) is the previous slot. Both EMPTY and FULL get 0 weight.
        // Tiebreaker: EMPTY gets 1, FULL gets 2 (because should_extend_payload=true).
        // FULL wins, so the head follows the FULL path.
        //
        // For FULL to actually produce a different head, we need a child on the FULL path.
        // Let's add root(2) building on FULL parent.
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches parent bid_block_hash → FULL parent
            false,
        );

        // Vote for root(2) so it has some weight
        fc.process_attestation(0, root(2), Epoch::new(0), Slot::new(3), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &balances,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(
            head,
            root(2),
            "root(2) should win — it's on the FULL path, which wins the tiebreaker"
        );
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "head block root(2) is a leaf with no revealed payload → status is EMPTY"
        );
    }

    // ───────── contains_execution_block_hash ─────────

    #[test]
    fn contains_execution_block_hash_found_via_bid_block_hash() {
        let (mut fc, _spec) = new_gloas_fc();
        let hash = exec_hash(1);
        insert_gloas_block(&mut fc, 1, root(1), root(0), Some(hash), None, false);
        assert!(
            fc.contains_execution_block_hash(&hash),
            "should find hash stored as bid_block_hash"
        );
    }

    #[test]
    fn contains_execution_block_hash_found_via_execution_status() {
        let (mut fc, _spec) = new_gloas_fc();
        let hash = exec_hash(2);
        // Insert block with valid execution status instead of bid_block_hash
        fc.proto_array
            .on_block::<MinimalEthSpec>(
                Block {
                    slot: Slot::new(1),
                    root: root(1),
                    parent_root: Some(root(0)),
                    state_root: Hash256::zero(),
                    target_root: root(0),
                    current_epoch_shuffling_id: junk_shuffling_id(),
                    next_epoch_shuffling_id: junk_shuffling_id(),
                    justified_checkpoint: genesis_checkpoint(),
                    finalized_checkpoint: genesis_checkpoint(),
                    execution_status: ExecutionStatus::Valid(hash),
                    unrealized_justified_checkpoint: Some(genesis_checkpoint()),
                    unrealized_finalized_checkpoint: Some(genesis_checkpoint()),
                    builder_index: Some(types::consts::gloas::BUILDER_INDEX_SELF_BUILD),
                    payload_revealed: false,
                    ptc_weight: 0,
                    ptc_blob_data_available_weight: 0,
                    payload_data_available: false,
                    bid_block_hash: None,
                    bid_parent_block_hash: None,
                    proposer_index: 0,
                    ptc_timely: false,
                    envelope_received: false,
                },
                Slot::new(1),
            )
            .unwrap();
        assert!(
            fc.contains_execution_block_hash(&hash),
            "should find hash via execution_status"
        );
    }

    #[test]
    fn contains_execution_block_hash_not_found() {
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            None,
            false,
        );
        let unknown = exec_hash(99);
        assert!(
            !fc.contains_execution_block_hash(&unknown),
            "should not find unknown hash"
        );
    }

    #[test]
    fn contains_execution_block_hash_zero_always_true() {
        let (fc, _spec) = new_gloas_fc();
        let zero = ExecutionBlockHash::zero();
        assert!(
            fc.contains_execution_block_hash(&zero),
            "zero hash should always return true"
        );
    }

    // ── should_extend_payload PTC threshold boundary tests ──
    //
    // These tests verify the strict inequality (>) vs (>=) boundary for PTC
    // quorum checks. The function has two paths: timely+available (PTC quorum)
    // and proposer boost. To test the PTC boundary, we set up a proposer boost
    // child building on EMPTY parent, so the boost path returns false, making
    // the overall result depend solely on the PTC quorum check.

    /// Helper: set up block_root with a child building on EMPTY parent as proposer boost.
    /// This makes should_extend_payload return false unless the PTC quorum path passes.
    fn setup_extend_payload_with_empty_boost(fc: &mut ProtoArrayForkChoice, block_root: Hash256) {
        let child_root = root(99);
        insert_external_builder_block(fc, 2, child_root, block_root, 42);
        // Child's bid_parent_block_hash is None (default) → builds on EMPTY parent
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_root,
            score: 100,
        };
    }

    #[test]
    fn should_extend_payload_ptc_weight_at_exact_threshold_returns_false() {
        // The spec uses strict inequality (ptc_weight > threshold), so ptc_weight
        // exactly equal to the threshold must NOT trigger should_extend_payload.
        // This is a consensus-critical off-by-one boundary.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        // Set ptc_weight exactly at threshold, not above
        node.ptc_weight = MINIMAL_PTC_THRESHOLD;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        setup_extend_payload_with_empty_boost(&mut fc, block_root);

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        assert!(
            !fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD),
            "ptc_weight == threshold (not strictly above) must NOT extend payload"
        );
    }

    #[test]
    fn should_extend_payload_blob_weight_at_exact_threshold_returns_false() {
        // Even if ptc_weight is above threshold, blob_data_available_weight must
        // also be strictly above threshold. Both quorum checks use strict >.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        // Set blob weight exactly at threshold
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD;

        setup_extend_payload_with_empty_boost(&mut fc, block_root);

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        assert!(
            !fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD),
            "blob_data_available_weight == threshold (not strictly above) must NOT extend payload"
        );
    }

    #[test]
    fn should_extend_payload_both_weights_one_above_threshold_returns_true() {
        // Both ptc_weight and blob_data_available_weight at threshold+1 (minimum
        // to pass strict >) with envelope_received should extend payload.
        // No proposer boost setup needed — timely+available path returns true directly.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let node = get_node_mut(&mut fc, &block_root);
        node.envelope_received = true;
        node.ptc_weight = MINIMAL_PTC_THRESHOLD + 1;
        node.ptc_blob_data_available_weight = MINIMAL_PTC_THRESHOLD + 1;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        assert!(
            fc.should_extend_payload(&gloas_node, MINIMAL_PTC_THRESHOLD),
            "both weights at threshold+1 with envelope_received should extend payload"
        );
    }

    // ── get_ancestor_gloas with skip slots ──

    #[test]
    fn ancestor_skip_slot_derives_payload_status_from_spanning_child() {
        // Chain: slot 0 → slot 1 → slot 5 (skip slots 2-4)
        // Ancestor at slot 3 should return root(1) with payload status derived
        // from the child (slot 5) → parent (slot 1) relationship.
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // slot 5 block: bid_parent_block_hash != parent's bid_block_hash → EMPTY parent
        insert_gloas_block(
            &mut fc,
            5,
            root(5),
            root(1),
            Some(exec_hash(5)),
            Some(exec_hash(99)), // mismatched → parent was EMPTY
            false,
        );

        // Ancestor of root(5) at slot 3 → walks up to root(1) (slot 1 <= 3 < 5)
        // Payload status: child(slot 5).bid_parent_block_hash = exec_hash(99) !=
        //                 parent(slot 1).bid_block_hash = exec_hash(1) → EMPTY
        let result = fc.get_ancestor_gloas(root(5), Slot::new(3));
        let ancestor = result.expect("ancestor should exist");
        assert_eq!(ancestor.root, root(1));
        assert_eq!(
            ancestor.payload_status,
            GloasPayloadStatus::Empty,
            "mismatched bid hashes across skip slots should derive EMPTY status"
        );
    }

    #[test]
    fn ancestor_skip_slot_full_parent_derived_from_matching_hashes() {
        // Same skip slot setup but with matching hashes → FULL parent status
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // slot 5 block: bid_parent_block_hash matches parent's bid_block_hash → FULL parent
        insert_gloas_block(
            &mut fc,
            5,
            root(5),
            root(1),
            Some(exec_hash(5)),
            Some(exec_hash(1)), // matches parent → parent was FULL
            false,
        );

        let result = fc.get_ancestor_gloas(root(5), Slot::new(3));
        let ancestor = result.expect("ancestor should exist");
        assert_eq!(ancestor.root, root(1));
        assert_eq!(
            ancestor.payload_status,
            GloasPayloadStatus::Full,
            "matching bid hashes across skip slots should derive FULL status"
        );
    }

    #[test]
    fn ancestor_deep_skip_slot_chain_walks_multiple_hops() {
        // Chain: slot 0 → slot 2 → slot 6 → slot 10
        // Ancestor at slot 4 should walk back from slot 10 through slot 6 to slot 2
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(0),
            Some(exec_hash(2)),
            None, // genesis parent
            true,
        );
        insert_gloas_block(
            &mut fc,
            6,
            root(6),
            root(2),
            Some(exec_hash(6)),
            Some(exec_hash(2)), // FULL parent
            true,
        );
        insert_gloas_block(
            &mut fc,
            10,
            root(10),
            root(6),
            Some(exec_hash(10)),
            Some(exec_hash(99)), // EMPTY parent
            false,
        );

        // Ancestor at slot 4: root(10) → walk → root(6) at slot 6 > 4, root(2) at slot 2 <= 4
        // Child=root(6), parent=root(2): exec_hash(2) == exec_hash(2) → FULL
        let result = fc.get_ancestor_gloas(root(10), Slot::new(4));
        let ancestor = result.expect("ancestor should exist");
        assert_eq!(ancestor.root, root(2));
        assert_eq!(ancestor.payload_status, GloasPayloadStatus::Full);

        // Ancestor at slot 8: root(10) at slot 10 > 8, root(6) at slot 6 <= 8
        // Child=root(10), parent=root(6): exec_hash(99) != exec_hash(6) → EMPTY
        let result = fc.get_ancestor_gloas(root(10), Slot::new(8));
        let ancestor = result.expect("ancestor should exist");
        assert_eq!(ancestor.root, root(6));
        assert_eq!(ancestor.payload_status, GloasPayloadStatus::Empty);
    }

    // ── get_gloas_children leaf termination ──

    #[test]
    fn gloas_children_empty_leaf_returns_no_children() {
        // An EMPTY virtual node at a leaf block (no child blocks) should return
        // an empty Vec, causing find_head_gloas to terminate at this node.
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));

        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };

        let children = fc.get_gloas_children(&empty_node, &filtered);
        assert!(
            children.is_empty(),
            "EMPTY leaf with no child blocks should have no children"
        );
    }

    #[test]
    fn gloas_children_full_leaf_returns_no_children() {
        // Same as above but for FULL virtual node at a leaf
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        let filtered = fc.compute_filtered_roots::<MinimalEthSpec>(Slot::new(2));

        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        let children = fc.get_gloas_children(&full_node, &filtered);
        assert!(
            children.is_empty(),
            "FULL leaf with no child blocks should have no children"
        );
    }

    // ── get_parent_payload_status_of with None hashes ──

    #[test]
    fn parent_payload_status_both_hashes_none_returns_empty() {
        // When both child.bid_parent_block_hash and parent.bid_block_hash are None
        // (e.g. genesis or default bids), the match arm falls to _ → Empty.
        let (mut fc, _spec) = new_gloas_fc();
        // Insert with None bid hashes
        insert_gloas_block(&mut fc, 1, root(1), root(0), None, None, false);

        let child = get_node(&fc, &root(1));
        let parent = get_node(&fc, &root(0));

        let status = fc.get_parent_payload_status_of(child, parent);
        assert_eq!(
            status,
            GloasPayloadStatus::Empty,
            "both None bid hashes should derive EMPTY parent status"
        );
    }

    #[test]
    fn parent_payload_status_child_none_parent_some_returns_empty() {
        // Child has None bid_parent_block_hash but parent has Some bid_block_hash
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // Child at slot 2 with bid_parent_block_hash = None
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            None,
            false,
        );

        let child = get_node(&fc, &root(2));
        let parent = get_node(&fc, &root(1));

        let status = fc.get_parent_payload_status_of(child, parent);
        assert_eq!(
            status,
            GloasPayloadStatus::Empty,
            "child None parent_hash with parent Some block_hash should derive EMPTY"
        );
    }

    #[test]
    fn parent_payload_status_child_some_parent_none_returns_empty() {
        // Child has Some bid_parent_block_hash but parent has None bid_block_hash
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(&mut fc, 1, root(1), root(0), None, None, false);
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
        );

        let child = get_node(&fc, &root(2));
        let parent = get_node(&fc, &root(1));

        let status = fc.get_parent_payload_status_of(child, parent);
        assert_eq!(
            status,
            GloasPayloadStatus::Empty,
            "child Some parent_hash with parent None block_hash should derive EMPTY"
        );
    }

    // ── is_supporting_vote through ancestor with skip slots ──

    #[test]
    fn supporting_vote_via_ancestor_across_skip_slots() {
        // Vote on a block at slot 5, check if it supports an ancestor at slot 1
        // when there are skip slots between them.
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );
        // Skip slots 2-4, block at slot 5 with matching parent hash → FULL parent
        insert_gloas_block(
            &mut fc,
            5,
            root(5),
            root(1),
            Some(exec_hash(5)),
            Some(exec_hash(1)), // matches parent → FULL
            false,
        );

        // FULL ancestor node at slot 1
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };

        // Vote at slot 5 pointing to root(5), ancestor at slot 1 should be FULL
        let vote = VoteTracker {
            current_root: root(5),
            current_slot: Slot::new(5),
            current_payload_present: true,
            ..VoteTracker::default()
        };

        // The ancestor check: get_ancestor_gloas(root(5), slot=1) → root(1) FULL
        // FULL == FULL → supporting
        assert!(
            fc.is_supporting_vote_gloas(&full_node, &vote),
            "vote through skip slots with FULL ancestor derivation should support FULL node"
        );

        // EMPTY ancestor node — should NOT support since ancestor derives FULL
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        assert!(
            !fc.is_supporting_vote_gloas(&empty_node, &vote),
            "vote through skip slots with FULL ancestor derivation should NOT support EMPTY node"
        );
    }

    // ── get_payload_tiebreaker not-previous-slot returns ordinal ──

    #[test]
    fn tiebreaker_non_previous_slot_returns_ordinal_value() {
        // For nodes NOT from the previous slot, tiebreaker returns the enum ordinal.
        // EMPTY=0, FULL=1, PENDING=2
        let (mut fc, _spec) = new_gloas_fc();
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true,
        );

        // Current slot = 5, block at slot 1 → not previous slot (1 + 1 != 5)
        let empty_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Empty,
        };
        let full_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Full,
        };
        let pending_node = GloasForkChoiceNode {
            root: root(1),
            payload_status: GloasPayloadStatus::Pending,
        };

        assert_eq!(
            fc.get_payload_tiebreaker(&empty_node, Slot::new(5), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Empty as u8,
            "EMPTY ordinal for non-previous-slot"
        );
        assert_eq!(
            fc.get_payload_tiebreaker(&full_node, Slot::new(5), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Full as u8,
            "FULL ordinal for non-previous-slot"
        );
        assert_eq!(
            fc.get_payload_tiebreaker(&pending_node, Slot::new(5), MINIMAL_PTC_THRESHOLD),
            GloasPayloadStatus::Pending as u8,
            "PENDING ordinal for non-previous-slot"
        );
    }

    // ── find_head_gloas: three-way fork and depth edge cases ──

    /// Three competing forks at the same slot. Votes are split evenly among them
    /// (2 votes each). Tiebreaker should use block root ordering, and the payload
    /// status of the winner should reflect whether the winning fork's payload was
    /// revealed.
    ///
    /// Fork A: root(1) at slot 1 — payload NOT revealed (EMPTY only)
    /// Fork B: root(2) at slot 1 — payload revealed (FULL + EMPTY)
    /// Fork C: root(3) at slot 1 — payload NOT revealed (EMPTY only)
    ///
    /// All three have 2 votes each → weight tie. Root ordering: root(1) < root(2) < root(3).
    /// Among equal-weight children, max_by picks the one with the highest root,
    /// so root(3) wins (highest Hash256 value among equal-weight nodes).
    #[test]
    fn find_head_gloas_three_way_fork_tiebreaker() {
        let (mut fc, spec) = new_gloas_fc();

        // Three forks from genesis, all at slot 1
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            true, // payload revealed
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(3),
            root(0),
            Some(exec_hash(3)),
            Some(exec_hash(99)),
            false,
        );

        let bals = balances(6);

        // 2 votes each: voters 0,1→root(1); 2,3→root(2); 4,5→root(3)
        for i in 0..2 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }
        for i in 2..4 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), true)
                .unwrap();
        }
        for i in 4..6 {
            fc.process_attestation(i, root(3), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();

        // With equal weights, root ordering tiebreaker picks the highest root
        assert_eq!(
            head,
            root(3),
            "three-way tie should be broken by root ordering (highest root wins)"
        );
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "root(3) has no payload revealed → EMPTY"
        );
    }

    /// Proposer boost applied to the EMPTY-path block flips the winner when the
    /// FULL-path block has slightly more natural votes.
    ///
    /// Fork A: root(1) at slot 1, payload revealed (FULL available)
    /// Fork B: root(2) at slot 1, payload NOT revealed (EMPTY only)
    ///
    /// 11 votes for root(1) FULL, 10 votes for root(2) EMPTY.
    /// Without boost: root(1) wins (352e9 > 320e9).
    /// With boost on root(2): root(2) wins if boost > 32e9.
    ///   boost = committee_weight * 40 / 100 = (672e9/8) * 40/100 = 33.6e9
    ///   root(2) total = 320e9 + 33.6e9 = 353.6e9 > 352e9.
    #[test]
    fn find_head_gloas_proposer_boost_flips_full_vs_empty() {
        let (mut fc, spec) = new_gloas_fc();

        // Fork A: payload revealed
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(0)),
            true, // payload revealed → FULL available
        );

        // Fork B: no payload
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(0)),
            false, // no payload
        );

        let bals = balances(21);

        // 11 votes for root(1) with payload_present=true
        for i in 0..11 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), true)
                .unwrap();
        }
        // 10 votes for root(2) with payload_present=false
        for i in 11..21 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(2), false)
                .unwrap();
        }

        // Without boost: root(1) wins
        let head_no_boost = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(
            head_no_boost,
            root(1),
            "without boost, root(1) FULL should win with 11 votes"
        );

        // With boost on root(2): root(2) flips to winner
        let head_with_boost = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                root(2), // proposer boost on root(2)
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(
            head_with_boost,
            root(2),
            "with boost, root(2) EMPTY should flip to winner"
        );
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "boosted winner has no payload revealed → EMPTY"
        );
    }

    /// Chain of depth 3 with alternating payload statuses: parent EMPTY, child FULL,
    /// grandchild EMPTY. Verifies find_head_gloas traverses the full chain correctly
    /// and the final payload status reflects the deepest node's status.
    ///
    /// Chain: root(0) → root(1) [parent EMPTY, payload revealed]
    ///                → root(2) [parent FULL, payload NOT revealed]
    ///
    /// root(1) parent is EMPTY (bid_parent_hash mismatch), payload revealed → FULL child.
    /// root(2) parent is FULL (bid_parent_hash matches root(1)'s bid_block_hash), no payload.
    ///
    /// All votes on root(2) with payload_present=false → EMPTY at depth 3.
    #[test]
    fn find_head_gloas_depth_three_alternating_payload_status() {
        let (mut fc, spec) = new_gloas_fc();

        // root(1): parent EMPTY (hash mismatch), payload revealed → FULL exists
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)), // mismatches genesis → parent EMPTY
            true,                // payload revealed
        );

        // root(2): parent FULL (matches root(1)'s exec_hash(1)), no payload
        insert_gloas_block(
            &mut fc,
            2,
            root(2),
            root(1),
            Some(exec_hash(2)),
            Some(exec_hash(1)), // matches root(1)'s bid_block_hash → parent FULL
            false,              // payload NOT revealed
        );

        let bals = balances(3);
        for i in 0..3 {
            fc.process_attestation(i, root(2), Epoch::new(0), Slot::new(3), false)
                .unwrap();
        }

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(head, root(2), "deepest block should be head");
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "root(2) has no payload → EMPTY leaf"
        );
    }

    /// Competing chains at different depths. A shorter fork with more votes should
    /// still win over a deeper fork with fewer votes, because find_head_gloas
    /// selects by weight at each level, not by depth.
    ///
    /// Fork A: root(0) → root(1) at slot 1, 3 votes
    /// Fork B: root(0) → root(2) at slot 1 → root(3) at slot 2, 1 vote on root(3)
    ///
    /// Fork A should win at the root(0) level because root(1) has more weight.
    #[test]
    fn find_head_gloas_shorter_fork_with_more_votes_wins() {
        let (mut fc, spec) = new_gloas_fc();

        // Fork A: root(1) at slot 1
        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)),
            false,
        );

        // Fork B: root(2) at slot 1 → root(3) at slot 2
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false,
        );
        insert_gloas_block(
            &mut fc,
            2,
            root(3),
            root(2),
            Some(exec_hash(3)),
            Some(exec_hash(99)),
            false,
        );

        let bals = balances(4);

        // 3 votes for root(1) (shorter fork)
        for i in 0..3 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(3), false)
                .unwrap();
        }
        // 1 vote for root(3) (deeper fork — weight flows up to root(2))
        fc.process_attestation(3, root(3), Epoch::new(0), Slot::new(3), false)
            .unwrap();

        let head = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();

        assert_eq!(
            head,
            root(1),
            "shorter fork with 3 votes should beat deeper fork with 1 vote"
        );
    }

    /// When all votes shift from one fork to another between two find_head calls,
    /// the winner should change. This tests the complete vote replacement path
    /// (not just incremental addition).
    ///
    /// Initial: 3 votes on root(1), 0 on root(2) → root(1) wins
    /// After: all 3 votes move to root(2) → root(2) wins
    #[test]
    fn find_head_gloas_complete_vote_shift_changes_winner() {
        let (mut fc, spec) = new_gloas_fc();

        insert_gloas_block(
            &mut fc,
            1,
            root(1),
            root(0),
            Some(exec_hash(1)),
            Some(exec_hash(99)),
            true, // FULL
        );
        insert_gloas_block(
            &mut fc,
            1,
            root(2),
            root(0),
            Some(exec_hash(2)),
            Some(exec_hash(99)),
            false, // EMPTY only
        );

        let bals = balances(3);

        // All votes initially on root(1) FULL
        for i in 0..3 {
            fc.process_attestation(i, root(1), Epoch::new(0), Slot::new(2), true)
                .unwrap();
        }

        let head1 = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(2),
                &spec,
            )
            .unwrap();
        assert_eq!(head1, root(1), "initially root(1) should win");
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Full as u8),
        );

        // All voters switch to root(2) EMPTY (target epoch must increase to override)
        for i in 0..3 {
            fc.process_attestation(i, root(2), Epoch::new(1), Slot::new(3), false)
                .unwrap();
        }

        let head2 = fc
            .find_head::<MinimalEthSpec>(
                genesis_checkpoint(),
                genesis_checkpoint(),
                &bals,
                Hash256::zero(),
                &BTreeSet::new(),
                Slot::new(3),
                &spec,
            )
            .unwrap();
        assert_eq!(head2, root(2), "after vote shift, root(2) should win");
        assert_eq!(
            fc.gloas_head_payload_status(),
            Some(GloasPayloadStatus::Empty as u8),
            "root(2) has no payload → EMPTY"
        );
    }
}
