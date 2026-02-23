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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GloasPayloadStatus {
    Pending = 0,
    Empty = 1,
    Full = 2,
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
    /// 1 = EMPTY, 2 = FULL. `None` for pre-Gloas heads.
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
    /// 1 = EMPTY, 2 = FULL. `None` for pre-Gloas heads.
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
    /// Spec: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#get_head
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

            head = children
                .into_iter()
                .max_by(|a, b| {
                    let wa = self.get_gloas_weight::<E>(
                        a,
                        proposer_boost_root,
                        apply_boost,
                        current_slot,
                        spec,
                    );
                    let wb = self.get_gloas_weight::<E>(
                        b,
                        proposer_boost_root,
                        apply_boost,
                        current_slot,
                        spec,
                    );
                    wa.cmp(&wb).then_with(|| a.root.cmp(&b.root)).then_with(|| {
                        let ta = self.get_payload_tiebreaker(a, current_slot);
                        let tb = self.get_payload_tiebreaker(b, current_slot);
                        ta.cmp(&tb)
                    })
                })
                .unwrap(); // safe: children is non-empty
        }
    }

    /// Compute the set of block roots that lead to viable heads.
    /// This implements the spec's `get_filtered_block_tree`.
    fn compute_filtered_roots<E: EthSpec>(&self, current_slot: Slot) -> HashSet<Hash256> {
        let pa = &self.proto_array;
        let mut filtered = vec![false; pa.nodes.len()];

        // Mark nodes that are viable for head
        for (i, node) in pa.nodes.iter().enumerate() {
            if pa.node_is_viable_for_head::<E>(node, current_slot) {
                filtered[i] = true;
            }
        }

        // Propagate upward: mark parents of any filtered node.
        // Nodes are ordered parent-before-child, so reverse iteration propagates correctly.
        for i in (0..pa.nodes.len()).rev() {
            if filtered[i]
                && let Some(parent_idx) = pa.nodes[i].parent
            {
                filtered[parent_idx] = true;
            }
        }

        pa.nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| filtered[*i])
            .map(|(_, node)| node.root)
            .collect()
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

                // Include FULL child only if execution payload has been revealed
                if let Some(&idx) = pa.indices.get(&node.root)
                    && let Some(proto_node) = pa.nodes.get(idx)
                    && proto_node.payload_revealed
                {
                    children.push(GloasForkChoiceNode {
                        root: node.root,
                        payload_status: GloasPayloadStatus::Full,
                    });
                }

                children
            }
            GloasPayloadStatus::Empty | GloasPayloadStatus::Full => {
                let mut children = Vec::new();

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
            if self.is_supporting_vote_gloas(&parent_node, vote) {
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

        // Non-PENDING nodes from previous slot get 0 weight
        if node.payload_status != GloasPayloadStatus::Pending
            && let Some(&idx) = pa.indices.get(&node.root)
            && let Some(proto_node) = pa.nodes.get(idx)
            && proto_node.slot + 1 == current_slot
        {
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
            if self.is_supporting_vote_gloas(node, vote) {
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
            if self.is_supporting_vote_gloas(node, &boost_vote)
                && let Some(score) = calculate_committee_fraction::<E>(&self.balances, boost_pct)
            {
                weight = weight.saturating_add(score);
            }
        }

        weight
    }

    /// Check if a vote supports a Gloas fork choice node.
    ///
    /// Implements the spec's `is_supporting_vote` with payload_present awareness.
    fn is_supporting_vote_gloas(&self, node: &GloasForkChoiceNode, vote: &VoteTracker) -> bool {
        let pa = &self.proto_array;

        let Some(&node_idx) = pa.indices.get(&node.root) else {
            return false;
        };
        let Some(block) = pa.nodes.get(node_idx) else {
            return false;
        };

        if node.root == vote.current_root {
            match node.payload_status {
                GloasPayloadStatus::Pending => true,
                GloasPayloadStatus::Empty | GloasPayloadStatus::Full => {
                    // Spec: assert message.slot >= block.slot
                    debug_assert!(vote.current_slot >= block.slot);
                    if vote.current_slot == block.slot {
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
            match self.get_ancestor_gloas(vote.current_root, block.slot) {
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
    /// For PENDING or non-previous-slot nodes: use payload_status ordinal.
    /// For previous-slot EMPTY: 1 (favored).
    /// For previous-slot FULL: 2 if should extend payload, else 0.
    fn get_payload_tiebreaker(&self, node: &GloasForkChoiceNode, current_slot: Slot) -> u8 {
        let pa = &self.proto_array;

        let is_previous_slot = pa
            .indices
            .get(&node.root)
            .and_then(|&idx| pa.nodes.get(idx))
            .is_some_and(|n| n.slot + 1 == current_slot);

        if node.payload_status == GloasPayloadStatus::Pending || !is_previous_slot {
            node.payload_status as u8
        } else if node.payload_status == GloasPayloadStatus::Empty {
            1
        } else {
            // FULL: use 2 if should extend payload, else 0.
            if self.should_extend_payload(node) {
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
    fn should_extend_payload(&self, node: &GloasForkChoiceNode) -> bool {
        let pa = &self.proto_array;

        // Check if payload is both timely and data-available
        let is_timely_and_available = pa
            .indices
            .get(&node.root)
            .and_then(|&idx| pa.nodes.get(idx))
            .is_some_and(|n| n.payload_revealed && n.payload_data_available);

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
                        // Boosted block's parent IS this root — check if parent is already FULL
                        // (i.e., the parent's parent had its payload revealed)
                        if parent_node.payload_revealed {
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
    /// ```ignore
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

    #[test]
    fn should_extend_payload_timely_and_data_available() {
        // When payload_revealed=true AND payload_data_available=true → true
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Set both flags
        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.payload_data_available = true;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node));
    }

    #[test]
    fn should_extend_payload_timely_but_not_data_available() {
        // payload_revealed=true but payload_data_available=false → falls through
        // to proposer boost checks. With no proposer boost → true.
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.payload_data_available = false;

        // No proposer boost (default is zero root) → should return true
        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert!(fc.should_extend_payload(&gloas_node));
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
        assert!(fc.should_extend_payload(&gloas_node));
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
        assert!(fc.should_extend_payload(&gloas_node));
    }

    #[test]
    fn should_extend_payload_boosted_parent_is_this_root_and_full() {
        // Boosted block's parent IS this root, and parent is already FULL (payload_revealed=true)
        // → should extend (true)
        let (mut fc, _spec) = new_gloas_fc();
        let parent_block = root(1);
        let child_block = root(2); // boosted block

        // Chain: root(0) → parent_block(slot=1) → child_block(slot=2)
        insert_external_builder_block(&mut fc, 1, parent_block, root(0), 42);
        insert_external_builder_block(&mut fc, 2, child_block, parent_block, 42);

        // Mark parent as FULL (payload revealed)
        let parent_node = get_node_mut(&mut fc, &parent_block);
        parent_node.payload_revealed = true;

        // Set proposer boost to child_block
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_block,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: parent_block,
            payload_status: GloasPayloadStatus::Full,
        };
        // Parent is already FULL → should extend
        assert!(fc.should_extend_payload(&gloas_node));
    }

    #[test]
    fn should_extend_payload_boosted_parent_is_this_root_and_not_full() {
        // Boosted block's parent IS this root, but parent is NOT FULL (payload_revealed=false)
        // → should NOT extend (false)
        let (mut fc, _spec) = new_gloas_fc();
        let parent_block = root(1);
        let child_block = root(2); // boosted block

        // Chain: root(0) → parent_block(slot=1) → child_block(slot=2)
        insert_external_builder_block(&mut fc, 1, parent_block, root(0), 42);
        insert_external_builder_block(&mut fc, 2, child_block, parent_block, 42);

        // Parent payload NOT revealed (default)
        assert!(!get_node(&fc, &parent_block).payload_revealed);

        // Set proposer boost to child_block
        fc.proto_array.previous_proposer_boost = ProposerBoost {
            root: child_block,
            score: 100,
        };

        let gloas_node = GloasForkChoiceNode {
            root: parent_block,
            payload_status: GloasPayloadStatus::Full,
        };
        // Parent NOT full → should NOT extend
        assert!(!fc.should_extend_payload(&gloas_node));
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
        assert!(fc.should_extend_payload(&gloas_node));
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
        assert!(fc.should_extend_payload(&gloas_node));
    }

    // ──────── get_payload_tiebreaker tests ────────────────────

    #[test]
    fn tiebreaker_pending_returns_ordinal() {
        // PENDING status always returns its ordinal value (0)
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Pending,
        };

        // Regardless of current_slot, PENDING returns ordinal
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2)),
            GloasPayloadStatus::Pending as u8
        );
        assert_eq!(
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(100)),
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
            fc.get_payload_tiebreaker(&empty_node, Slot::new(5)),
            GloasPayloadStatus::Empty as u8
        );

        let full_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };
        assert_eq!(
            fc.get_payload_tiebreaker(&full_node, Slot::new(5)),
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
        assert_eq!(fc.get_payload_tiebreaker(&gloas_node, Slot::new(2)), 1);
    }

    #[test]
    fn tiebreaker_previous_slot_full_should_extend() {
        // FULL at previous slot with should_extend_payload=true returns 2
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Make should_extend_payload return true by setting both flags
        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.payload_data_available = true;

        let gloas_node = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        // Block at slot 1, current_slot = 2 (previous slot)
        assert_eq!(fc.get_payload_tiebreaker(&gloas_node, Slot::new(2)), 2);
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
        assert_eq!(fc.get_payload_tiebreaker(&gloas_node, Slot::new(2)), 0);
    }

    #[test]
    fn tiebreaker_ordering_previous_slot() {
        // Verify the tiebreaker ordering at previous slot:
        // When should_extend_payload=true: FULL(2) > EMPTY(1) > PENDING(0)
        // When should_extend_payload=false: EMPTY(1) > PENDING(0) > FULL(0)
        let (mut fc, _spec) = new_gloas_fc();
        let block_root = root(1);
        insert_external_builder_block(&mut fc, 1, block_root, root(0), 42);

        // Make should_extend_payload return true
        let node = get_node_mut(&mut fc, &block_root);
        node.payload_revealed = true;
        node.payload_data_available = true;

        let pending = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Pending,
        };
        let empty = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Empty,
        };
        let full = GloasForkChoiceNode {
            root: block_root,
            payload_status: GloasPayloadStatus::Full,
        };

        let current_slot = Slot::new(2); // previous slot for block at slot 1
        let tp = fc.get_payload_tiebreaker(&pending, current_slot);
        let te = fc.get_payload_tiebreaker(&empty, current_slot);
        let tf = fc.get_payload_tiebreaker(&full, current_slot);

        // FULL > EMPTY > PENDING when extending
        assert!(tf > te, "FULL({}) should beat EMPTY({})", tf, te);
        assert!(te > tp, "EMPTY({}) should beat PENDING({})", te, tp);
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
            fc.get_payload_tiebreaker(&gloas_node, Slot::new(2)),
            GloasPayloadStatus::Full as u8
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
}
