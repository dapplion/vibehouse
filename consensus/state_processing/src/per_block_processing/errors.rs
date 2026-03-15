use super::signature_sets::Error as SignatureSetError;
use crate::ContextError;
use crate::envelope_processing::EnvelopeProcessingError;
use merkle_proof::MerkleTreeError;
use safe_arith::ArithError;
use ssz::DecodeError;
use types::*;

/// The error returned from the `per_block_processing` function. Indicates that a block is either
/// invalid, or we were unable to determine its validity (we encountered an unexpected error).
///
/// Any of the `...Error` variants indicate that at some point during block (and block operation)
/// verification, there was an error. There is no indication as to _where_ that error happened
/// (e.g., when processing attestations instead of when processing deposits).
#[derive(Debug, PartialEq, Clone)]
pub enum BlockProcessingError {
    /// Logic error indicating that the wrong state type was provided.
    IncorrectStateType,
    RandaoSignatureInvalid,
    BulkSignatureVerificationFailed,
    StateRootMismatch,
    DepositCountInvalid {
        expected: usize,
        found: usize,
    },
    HeaderInvalid {
        reason: HeaderInvalid,
    },
    ProposerSlashingInvalid {
        index: usize,
        reason: ProposerSlashingInvalid,
    },
    AttesterSlashingInvalid {
        index: usize,
        reason: AttesterSlashingInvalid,
    },
    IndexedAttestationInvalid {
        index: usize,
        reason: IndexedAttestationInvalid,
    },
    AttestationInvalid {
        index: usize,
        reason: AttestationInvalid,
    },
    DepositInvalid {
        index: usize,
        reason: DepositInvalid,
    },
    ExitInvalid {
        index: usize,
        reason: ExitInvalid,
    },
    BlsExecutionChangeInvalid {
        index: usize,
        reason: BlsExecutionChangeInvalid,
    },
    SyncAggregateInvalid {
        reason: SyncAggregateInvalid,
    },
    BeaconStateError(BeaconStateError),
    SignatureSetError(SignatureSetError),
    SszTypesError(ssz_types::Error),
    SszDecodeError(DecodeError),
    BitfieldError(ssz::BitfieldError),
    MerkleTreeError(MerkleTreeError),
    ArithError(ArithError),
    InconsistentBlockFork(InconsistentFork),
    InconsistentStateFork(InconsistentFork),
    ExecutionHashChainIncontiguous {
        expected: ExecutionBlockHash,
        found: ExecutionBlockHash,
    },
    ExecutionRandaoMismatch {
        expected: Hash256,
        found: Hash256,
    },
    ExecutionInvalidTimestamp {
        expected: u64,
        found: u64,
    },
    ExecutionInvalidBlobsLen {
        max: usize,
        actual: usize,
    },
    ExecutionInvalid,
    ConsensusContext(ContextError),
    MilhouseError(milhouse::Error),
    EpochCacheError(EpochCacheError),
    WithdrawalsRootMismatch {
        expected: Hash256,
        found: Hash256,
    },
    WithdrawalCredentialsInvalid,
    InvalidBuilderCredentials,
    WithdrawalBuilderIndexInvalid {
        builder_index: u64,
        builders_count: u64,
    },
    PendingAttestationInElectra,
    PayloadBidInvalid {
        reason: String,
    },
    PayloadAttestationInvalid(PayloadAttestationInvalid),
    EnvelopeProcessingError(Box<EnvelopeProcessingError>),
}

impl From<BeaconStateError> for BlockProcessingError {
    fn from(e: BeaconStateError) -> Self {
        BlockProcessingError::BeaconStateError(e)
    }
}

impl From<SignatureSetError> for BlockProcessingError {
    fn from(e: SignatureSetError) -> Self {
        BlockProcessingError::SignatureSetError(e)
    }
}

impl From<ssz_types::Error> for BlockProcessingError {
    fn from(error: ssz_types::Error) -> Self {
        BlockProcessingError::SszTypesError(error)
    }
}

impl From<DecodeError> for BlockProcessingError {
    fn from(error: DecodeError) -> Self {
        BlockProcessingError::SszDecodeError(error)
    }
}

impl From<ArithError> for BlockProcessingError {
    fn from(e: ArithError) -> Self {
        BlockProcessingError::ArithError(e)
    }
}

impl From<SyncAggregateInvalid> for BlockProcessingError {
    fn from(reason: SyncAggregateInvalid) -> Self {
        BlockProcessingError::SyncAggregateInvalid { reason }
    }
}

impl From<ContextError> for BlockProcessingError {
    fn from(e: ContextError) -> Self {
        BlockProcessingError::ConsensusContext(e)
    }
}

impl From<EpochCacheError> for BlockProcessingError {
    fn from(e: EpochCacheError) -> Self {
        BlockProcessingError::EpochCacheError(e)
    }
}

impl From<milhouse::Error> for BlockProcessingError {
    fn from(e: milhouse::Error) -> Self {
        Self::MilhouseError(e)
    }
}

impl From<BlockOperationError<HeaderInvalid>> for BlockProcessingError {
    fn from(e: BlockOperationError<HeaderInvalid>) -> BlockProcessingError {
        match e {
            BlockOperationError::Invalid(reason) => BlockProcessingError::HeaderInvalid { reason },
            BlockOperationError::BeaconStateError(e) => BlockProcessingError::BeaconStateError(e),
            BlockOperationError::SignatureSetError(e) => BlockProcessingError::SignatureSetError(e),
            BlockOperationError::SszTypesError(e) => BlockProcessingError::SszTypesError(e),
            BlockOperationError::BitfieldError(e) => BlockProcessingError::BitfieldError(e),
            BlockOperationError::ConsensusContext(e) => BlockProcessingError::ConsensusContext(e),
            BlockOperationError::ArithError(e) => BlockProcessingError::ArithError(e),
        }
    }
}

/// A conversion that consumes `self` and adds an `index` variable to resulting struct.
///
/// Used here to allow converting an error into an upstream error that points to the object that
/// caused the error. For example, pointing to the index of an attestation that caused the
/// `AttestationInvalid` error.
pub trait IntoWithIndex<T>: Sized {
    fn into_with_index(self, index: usize) -> T;
}

macro_rules! impl_into_block_processing_error_with_index {
    ($($type: ident),*) => {
        $(
            impl IntoWithIndex<BlockProcessingError> for BlockOperationError<$type> {
                fn into_with_index(self, index: usize) -> BlockProcessingError {
                    match self {
                        BlockOperationError::Invalid(reason) => BlockProcessingError::$type {
                            index,
                            reason
                        },
                        BlockOperationError::BeaconStateError(e) => BlockProcessingError::BeaconStateError(e),
                        BlockOperationError::SignatureSetError(e) => BlockProcessingError::SignatureSetError(e),
                        BlockOperationError::SszTypesError(e) => BlockProcessingError::SszTypesError(e),
                        BlockOperationError::BitfieldError(e) => BlockProcessingError::BitfieldError(e),
                        BlockOperationError::ConsensusContext(e) => BlockProcessingError::ConsensusContext(e),
                        BlockOperationError::ArithError(e) => BlockProcessingError::ArithError(e),
                    }
                }
            }
        )*
    };
}

impl_into_block_processing_error_with_index!(
    ProposerSlashingInvalid,
    AttesterSlashingInvalid,
    IndexedAttestationInvalid,
    AttestationInvalid,
    DepositInvalid,
    ExitInvalid,
    BlsExecutionChangeInvalid
);

pub type HeaderValidationError = BlockOperationError<HeaderInvalid>;
pub type AttesterSlashingValidationError = BlockOperationError<AttesterSlashingInvalid>;
pub type ProposerSlashingValidationError = BlockOperationError<ProposerSlashingInvalid>;
pub type AttestationValidationError = BlockOperationError<AttestationInvalid>;
pub type SyncCommitteeMessageValidationError = BlockOperationError<SyncAggregateInvalid>;
pub type DepositValidationError = BlockOperationError<DepositInvalid>;
pub type ExitValidationError = BlockOperationError<ExitInvalid>;
pub type BlsExecutionChangeValidationError = BlockOperationError<BlsExecutionChangeInvalid>;

#[derive(Debug, PartialEq, Clone)]
pub enum BlockOperationError<T> {
    Invalid(T),
    BeaconStateError(BeaconStateError),
    SignatureSetError(SignatureSetError),
    SszTypesError(ssz_types::Error),
    BitfieldError(ssz::BitfieldError),
    ConsensusContext(ContextError),
    ArithError(ArithError),
}

impl<T> BlockOperationError<T> {
    pub fn invalid(reason: T) -> BlockOperationError<T> {
        BlockOperationError::Invalid(reason)
    }
}

impl<T> From<BeaconStateError> for BlockOperationError<T> {
    fn from(e: BeaconStateError) -> Self {
        BlockOperationError::BeaconStateError(e)
    }
}
impl<T> From<SignatureSetError> for BlockOperationError<T> {
    fn from(e: SignatureSetError) -> Self {
        BlockOperationError::SignatureSetError(e)
    }
}

impl<T> From<ssz_types::Error> for BlockOperationError<T> {
    fn from(error: ssz_types::Error) -> Self {
        BlockOperationError::SszTypesError(error)
    }
}

impl<T> From<ssz::BitfieldError> for BlockOperationError<T> {
    fn from(error: ssz::BitfieldError) -> Self {
        BlockOperationError::BitfieldError(error)
    }
}

impl<T> From<ArithError> for BlockOperationError<T> {
    fn from(e: ArithError) -> Self {
        BlockOperationError::ArithError(e)
    }
}

impl<T> From<ContextError> for BlockOperationError<T> {
    fn from(e: ContextError) -> Self {
        BlockOperationError::ConsensusContext(e)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum HeaderInvalid {
    ProposalSignatureInvalid,
    StateSlotMismatch,
    OlderThanLatestBlockHeader {
        latest_block_header_slot: Slot,
        block_slot: Slot,
    },
    ProposerIndexMismatch {
        block_proposer_index: u64,
        state_proposer_index: u64,
    },
    ParentBlockRootMismatch {
        state: Hash256,
        block: Hash256,
    },
    ProposerSlashed(u64),
}

#[derive(Debug, PartialEq, Clone)]
pub enum ProposerSlashingInvalid {
    /// The proposer index is not a known validator.
    ProposerUnknown(u64),
    /// The two proposal have different slots.
    ///
    /// (proposal_1_slot, proposal_2_slot)
    ProposalSlotMismatch(Slot, Slot),
    /// The two proposals have different proposer indices.
    ///
    /// (proposer_index_1, proposer_index_2)
    ProposerIndexMismatch(u64, u64),
    /// The proposals are identical and therefore not slashable.
    ProposalsIdentical,
    /// The specified proposer cannot be slashed because they are already slashed, or not active.
    ProposerNotSlashable(u64),
    /// The first proposal signature was invalid.
    BadProposal1Signature,
    /// The second proposal signature was invalid.
    BadProposal2Signature,
}

#[derive(Debug, PartialEq, Clone)]
pub enum AttesterSlashingInvalid {
    /// The attestations were not in conflict.
    NotSlashable,
    /// The first `IndexedAttestation` was invalid.
    IndexedAttestation1Invalid(BlockOperationError<IndexedAttestationInvalid>),
    /// The second `IndexedAttestation` was invalid.
    IndexedAttestation2Invalid(BlockOperationError<IndexedAttestationInvalid>),
    /// The validator index is unknown. One cannot slash one who does not exist.
    UnknownValidator(u64),
    /// There were no indices able to be slashed.
    NoSlashableIndices,
}

/// Describes why an object is invalid.
#[derive(Debug, PartialEq, Clone)]
pub enum AttestationInvalid {
    /// Committee index exceeds number of committees in that slot.
    BadCommitteeIndex,
    /// Attestation included before the inclusion delay.
    IncludedTooEarly {
        state: Slot,
        delay: u64,
        attestation: Slot,
    },
    /// Attestation slot is too far in the past to be included in a block.
    IncludedTooLate { state: Slot, attestation: Slot },
    /// Attestation target epoch does not match attestation slot.
    TargetEpochSlotMismatch {
        target_epoch: Epoch,
        slot_epoch: Epoch,
    },
    /// Attestation target epoch does not match the current or previous epoch.
    BadTargetEpoch,
    /// Attestation justified checkpoint doesn't match the state's current or previous justified
    /// checkpoint.
    ///
    /// `is_current` is `true` if the attestation was compared to the
    /// `state.current_justified_checkpoint`, `false` if compared to `state.previous_justified_checkpoint`.
    ///
    /// Checkpoints have been boxed to keep the error size down and prevent clippy failures.
    WrongJustifiedCheckpoint {
        state: Box<Checkpoint>,
        attestation: Box<Checkpoint>,
        is_current: bool,
    },
    /// The attestation signature verification failed.
    BadSignature,
    /// The indexed attestation created from this attestation was found to be invalid.
    BadIndexedAttestation(IndexedAttestationInvalid),
}

impl From<BlockOperationError<IndexedAttestationInvalid>>
    for BlockOperationError<AttestationInvalid>
{
    fn from(e: BlockOperationError<IndexedAttestationInvalid>) -> Self {
        match e {
            BlockOperationError::Invalid(e) => {
                BlockOperationError::invalid(AttestationInvalid::BadIndexedAttestation(e))
            }
            BlockOperationError::BeaconStateError(e) => BlockOperationError::BeaconStateError(e),
            BlockOperationError::SignatureSetError(e) => BlockOperationError::SignatureSetError(e),
            BlockOperationError::SszTypesError(e) => BlockOperationError::SszTypesError(e),
            BlockOperationError::BitfieldError(e) => BlockOperationError::BitfieldError(e),
            BlockOperationError::ConsensusContext(e) => BlockOperationError::ConsensusContext(e),
            BlockOperationError::ArithError(e) => BlockOperationError::ArithError(e),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum IndexedAttestationInvalid {
    /// The number of indices is 0.
    IndicesEmpty,
    /// The validator indices were not in increasing order.
    ///
    /// The error occurred between the given `index` and `index + 1`
    BadValidatorIndicesOrdering(usize),
    /// The indexed attestation aggregate signature was not valid.
    BadSignature,
}

#[derive(Debug, PartialEq, Clone)]
pub enum DepositInvalid {
    /// The signature (proof-of-possession) does not match the given pubkey.
    BadSignature,
    /// The signature or pubkey does not represent a valid BLS point.
    BadBlsBytes,
    /// The specified `branch` and `index` did not form a valid proof that the deposit is included
    /// in the eth1 deposit root.
    BadMerkleProof,
}

#[derive(Debug, PartialEq, Clone)]
pub enum ExitInvalid {
    /// The specified validator is not active.
    NotActive(u64),
    /// The specified validator is not in the state's validator registry.
    ValidatorUnknown(u64),
    /// The specified validator has a non-maximum exit epoch.
    AlreadyExited(u64),
    /// The exit is for a future epoch.
    FutureEpoch {
        state: Epoch,
        exit: Epoch,
    },
    /// The validator has not been active for long enough.
    TooYoungToExit {
        current_epoch: Epoch,
        earliest_exit_epoch: Epoch,
    },
    /// The exit signature was not signed by the validator.
    BadSignature,
    /// There was an error whilst attempting to get a set of signatures. The signatures may have
    /// been invalid or an internal error occurred.
    SignatureSetError(SignatureSetError),
    PendingWithdrawalInQueue(u64),
    /// [New in Gloas:EIP7732] The builder index does not exist in the registry.
    BuilderUnknown(u64),
    /// [New in Gloas:EIP7732] The builder is not active.
    BuilderNotActive(u64),
    /// [New in Gloas:EIP7732] The builder has pending withdrawals in the queue.
    BuilderPendingWithdrawalInQueue(u64),
}

#[derive(Debug, PartialEq, Clone)]
pub enum BlsExecutionChangeInvalid {
    /// The specified validator is not in the state's validator registry.
    ValidatorUnknown(u64),
    /// Validator does not have BLS Withdrawal credentials before this change.
    NonBlsWithdrawalCredentials,
    /// Provided BLS pubkey does not match withdrawal credentials.
    WithdrawalCredentialsMismatch,
    /// The signature is invalid.
    BadSignature,
}

#[derive(Debug, PartialEq, Clone)]
pub enum SyncAggregateInvalid {
    /// The signature is invalid.
    SignatureInvalid,
}

#[derive(Debug, PartialEq, Clone)]
pub enum PayloadAttestationInvalid {
    /// Attestation is for the wrong slot
    WrongSlot { expected: Slot, actual: Slot },
    /// Attestation beacon_block_root does not match state
    WrongBeaconBlockRoot,
    /// Signature verification failed
    BadSignature,
    /// Failed to decompress a public key
    InvalidPubkey,
    /// One or more attesting indices are out of bounds
    AttesterIndexOutOfBounds,
    /// No active validators to form PTC
    NoActiveValidators,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BlockProcessingError From conversions ──

    #[test]
    fn from_beacon_state_error() {
        let e = BeaconStateError::InsufficientValidators;
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::BeaconStateError(e));
    }

    #[test]
    fn from_ssz_types_error() {
        let e = ssz_types::Error::OutOfBounds { i: 5, len: 3 };
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::SszTypesError(e));
    }

    #[test]
    fn from_decode_error() {
        let e = DecodeError::InvalidByteLength {
            len: 10,
            expected: 32,
        };
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::SszDecodeError(e));
    }

    #[test]
    fn from_arith_error() {
        let e = ArithError::Overflow;
        let bpe: BlockProcessingError = e.into();
        assert_eq!(bpe, BlockProcessingError::ArithError(ArithError::Overflow));
    }

    #[test]
    fn from_sync_aggregate_invalid() {
        let reason = SyncAggregateInvalid::SignatureInvalid;
        let bpe: BlockProcessingError = reason.clone().into();
        assert_eq!(bpe, BlockProcessingError::SyncAggregateInvalid { reason });
    }

    #[test]
    fn from_context_error() {
        let e = ContextError::SlotMismatch {
            slot: Slot::new(5),
            expected: Slot::new(10),
        };
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::ConsensusContext(e));
    }

    #[test]
    fn from_epoch_cache_error() {
        let e = EpochCacheError::IncorrectEpoch {
            cache: Epoch::new(1),
            state: Epoch::new(2),
        };
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::EpochCacheError(e));
    }

    #[test]
    fn from_milhouse_error() {
        let e = milhouse::Error::InvalidListUpdate;
        let bpe: BlockProcessingError = e.clone().into();
        assert_eq!(bpe, BlockProcessingError::MilhouseError(e));
    }

    // ── BlockOperationError conversions ──

    #[test]
    fn block_operation_error_invalid() {
        let op: BlockOperationError<HeaderInvalid> =
            BlockOperationError::invalid(HeaderInvalid::StateSlotMismatch);
        assert_eq!(
            op,
            BlockOperationError::Invalid(HeaderInvalid::StateSlotMismatch)
        );
    }

    #[test]
    fn block_operation_error_from_beacon_state_error() {
        let e = BeaconStateError::InsufficientValidators;
        let op: BlockOperationError<HeaderInvalid> = e.clone().into();
        assert_eq!(op, BlockOperationError::BeaconStateError(e));
    }

    #[test]
    fn block_operation_error_from_arith_error() {
        let e = ArithError::DivisionByZero;
        let op: BlockOperationError<DepositInvalid> = e.into();
        assert_eq!(
            op,
            BlockOperationError::ArithError(ArithError::DivisionByZero)
        );
    }

    #[test]
    fn block_operation_error_from_ssz_types_error() {
        let e = ssz_types::Error::OutOfBounds { i: 0, len: 0 };
        let op: BlockOperationError<ExitInvalid> = e.clone().into();
        assert_eq!(op, BlockOperationError::SszTypesError(e));
    }

    // ── HeaderInvalid → BlockProcessingError ──

    #[test]
    fn header_invalid_to_block_processing_error() {
        let op = BlockOperationError::Invalid(HeaderInvalid::ProposalSignatureInvalid);
        let bpe: BlockProcessingError = op.into();
        assert_eq!(
            bpe,
            BlockProcessingError::HeaderInvalid {
                reason: HeaderInvalid::ProposalSignatureInvalid,
            }
        );
    }

    #[test]
    fn header_beacon_state_error_to_block_processing_error() {
        let op: BlockOperationError<HeaderInvalid> =
            BlockOperationError::BeaconStateError(BeaconStateError::InsufficientValidators);
        let bpe: BlockProcessingError = op.into();
        assert_eq!(
            bpe,
            BlockProcessingError::BeaconStateError(BeaconStateError::InsufficientValidators)
        );
    }

    // ── IntoWithIndex macro-generated conversions ──

    #[test]
    fn proposer_slashing_invalid_into_with_index() {
        let op = BlockOperationError::Invalid(ProposerSlashingInvalid::ProposalsIdentical);
        let bpe: BlockProcessingError = op.into_with_index(3);
        assert_eq!(
            bpe,
            BlockProcessingError::ProposerSlashingInvalid {
                index: 3,
                reason: ProposerSlashingInvalid::ProposalsIdentical,
            }
        );
    }

    #[test]
    fn attester_slashing_invalid_into_with_index() {
        let op = BlockOperationError::Invalid(AttesterSlashingInvalid::NotSlashable);
        let bpe: BlockProcessingError = op.into_with_index(7);
        assert_eq!(
            bpe,
            BlockProcessingError::AttesterSlashingInvalid {
                index: 7,
                reason: AttesterSlashingInvalid::NotSlashable,
            }
        );
    }

    #[test]
    fn attestation_invalid_into_with_index() {
        let op = BlockOperationError::Invalid(AttestationInvalid::BadTargetEpoch);
        let bpe: BlockProcessingError = op.into_with_index(0);
        assert_eq!(
            bpe,
            BlockProcessingError::AttestationInvalid {
                index: 0,
                reason: AttestationInvalid::BadTargetEpoch,
            }
        );
    }

    #[test]
    fn deposit_invalid_into_with_index() {
        let op = BlockOperationError::Invalid(DepositInvalid::BadMerkleProof);
        let bpe: BlockProcessingError = op.into_with_index(42);
        assert_eq!(
            bpe,
            BlockProcessingError::DepositInvalid {
                index: 42,
                reason: DepositInvalid::BadMerkleProof,
            }
        );
    }

    #[test]
    fn exit_invalid_into_with_index() {
        let op = BlockOperationError::Invalid(ExitInvalid::AlreadyExited(99));
        let bpe: BlockProcessingError = op.into_with_index(1);
        assert_eq!(
            bpe,
            BlockProcessingError::ExitInvalid {
                index: 1,
                reason: ExitInvalid::AlreadyExited(99),
            }
        );
    }

    #[test]
    fn bls_execution_change_invalid_into_with_index() {
        let op =
            BlockOperationError::Invalid(BlsExecutionChangeInvalid::NonBlsWithdrawalCredentials);
        let bpe: BlockProcessingError = op.into_with_index(5);
        assert_eq!(
            bpe,
            BlockProcessingError::BlsExecutionChangeInvalid {
                index: 5,
                reason: BlsExecutionChangeInvalid::NonBlsWithdrawalCredentials,
            }
        );
    }

    #[test]
    fn into_with_index_beacon_state_error_passthrough() {
        let op: BlockOperationError<AttestationInvalid> =
            BlockOperationError::BeaconStateError(BeaconStateError::InsufficientValidators);
        let bpe: BlockProcessingError = op.into_with_index(0);
        assert_eq!(
            bpe,
            BlockProcessingError::BeaconStateError(BeaconStateError::InsufficientValidators)
        );
    }

    #[test]
    fn into_with_index_arith_error_passthrough() {
        let op: BlockOperationError<DepositInvalid> =
            BlockOperationError::ArithError(ArithError::Overflow);
        let bpe: BlockProcessingError = op.into_with_index(0);
        assert_eq!(bpe, BlockProcessingError::ArithError(ArithError::Overflow));
    }

    // ── IndexedAttestationInvalid → AttestationInvalid ──

    #[test]
    fn indexed_attestation_invalid_to_attestation_invalid() {
        let op = BlockOperationError::Invalid(IndexedAttestationInvalid::IndicesEmpty);
        let result: BlockOperationError<AttestationInvalid> = op.into();
        assert_eq!(
            result,
            BlockOperationError::Invalid(AttestationInvalid::BadIndexedAttestation(
                IndexedAttestationInvalid::IndicesEmpty
            ))
        );
    }

    #[test]
    fn indexed_attestation_beacon_state_passthrough() {
        let op: BlockOperationError<IndexedAttestationInvalid> =
            BlockOperationError::BeaconStateError(BeaconStateError::InsufficientValidators);
        let result: BlockOperationError<AttestationInvalid> = op.into();
        assert_eq!(
            result,
            BlockOperationError::BeaconStateError(BeaconStateError::InsufficientValidators)
        );
    }

    // ── Error variant field tests ──

    #[test]
    fn header_invalid_older_than_latest() {
        let h = HeaderInvalid::OlderThanLatestBlockHeader {
            latest_block_header_slot: Slot::new(10),
            block_slot: Slot::new(5),
        };
        assert_eq!(
            format!("{:?}", h),
            "OlderThanLatestBlockHeader { latest_block_header_slot: Slot(10), block_slot: Slot(5) }"
        );
    }

    #[test]
    fn header_invalid_proposer_index_mismatch() {
        let h = HeaderInvalid::ProposerIndexMismatch {
            block_proposer_index: 1,
            state_proposer_index: 2,
        };
        match h {
            HeaderInvalid::ProposerIndexMismatch {
                block_proposer_index,
                state_proposer_index,
            } => {
                assert_eq!(block_proposer_index, 1);
                assert_eq!(state_proposer_index, 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn attestation_invalid_included_too_early() {
        let a = AttestationInvalid::IncludedTooEarly {
            state: Slot::new(10),
            delay: 1,
            attestation: Slot::new(10),
        };
        match a {
            AttestationInvalid::IncludedTooEarly {
                state,
                delay,
                attestation,
            } => {
                assert_eq!(state, Slot::new(10));
                assert_eq!(delay, 1);
                assert_eq!(attestation, Slot::new(10));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn exit_invalid_too_young_to_exit() {
        let e = ExitInvalid::TooYoungToExit {
            current_epoch: Epoch::new(5),
            earliest_exit_epoch: Epoch::new(10),
        };
        match e {
            ExitInvalid::TooYoungToExit {
                current_epoch,
                earliest_exit_epoch,
            } => {
                assert_eq!(current_epoch, Epoch::new(5));
                assert_eq!(earliest_exit_epoch, Epoch::new(10));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn payload_attestation_invalid_wrong_slot() {
        let p = PayloadAttestationInvalid::WrongSlot {
            expected: Slot::new(1),
            actual: Slot::new(2),
        };
        match p {
            PayloadAttestationInvalid::WrongSlot { expected, actual } => {
                assert_eq!(expected, Slot::new(1));
                assert_eq!(actual, Slot::new(2));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_processing_error_deposit_count_invalid() {
        let e = BlockProcessingError::DepositCountInvalid {
            expected: 16,
            found: 8,
        };
        match e {
            BlockProcessingError::DepositCountInvalid { expected, found } => {
                assert_eq!(expected, 16);
                assert_eq!(found, 8);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_processing_error_execution_hash_chain_incontiguous() {
        let expected = ExecutionBlockHash::zero();
        let found = ExecutionBlockHash::repeat_byte(0xff);
        let e = BlockProcessingError::ExecutionHashChainIncontiguous { expected, found };
        match e {
            BlockProcessingError::ExecutionHashChainIncontiguous {
                expected: e,
                found: f,
            } => {
                assert_eq!(e, ExecutionBlockHash::zero());
                assert_eq!(f, ExecutionBlockHash::repeat_byte(0xff));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_processing_error_clone_eq() {
        let e1 = BlockProcessingError::IncorrectStateType;
        let e2 = e1.clone();
        assert_eq!(e1, e2);
    }

    #[test]
    fn block_processing_error_debug_format() {
        let e = BlockProcessingError::RandaoSignatureInvalid;
        let dbg = format!("{:?}", e);
        assert!(dbg.contains("RandaoSignatureInvalid"));
    }

    // ── Gloas-specific error variants ──

    #[test]
    fn exit_invalid_builder_unknown() {
        let e = ExitInvalid::BuilderUnknown(42);
        match e {
            ExitInvalid::BuilderUnknown(idx) => assert_eq!(idx, 42),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn exit_invalid_builder_not_active() {
        let e = ExitInvalid::BuilderNotActive(7);
        match e {
            ExitInvalid::BuilderNotActive(idx) => assert_eq!(idx, 7),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn exit_invalid_builder_pending_withdrawal() {
        let e = ExitInvalid::BuilderPendingWithdrawalInQueue(99);
        match e {
            ExitInvalid::BuilderPendingWithdrawalInQueue(idx) => assert_eq!(idx, 99),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_processing_error_withdrawal_builder_index_invalid() {
        let e = BlockProcessingError::WithdrawalBuilderIndexInvalid {
            builder_index: 5,
            builders_count: 3,
        };
        match e {
            BlockProcessingError::WithdrawalBuilderIndexInvalid {
                builder_index,
                builders_count,
            } => {
                assert_eq!(builder_index, 5);
                assert_eq!(builders_count, 3);
            }
            _ => panic!("wrong variant"),
        }
    }
}
