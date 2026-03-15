use crate::attester_cache::Error as AttesterCacheError;
use crate::beacon_block_streamer::Error as BlockStreamerError;
use crate::beacon_chain::ForkChoiceError;
use crate::beacon_fork_choice_store::Error as ForkChoiceStoreError;
use crate::data_availability_checker::AvailabilityCheckError;
use crate::migrate::PruningError;
use crate::naive_aggregation_pool::Error as NaiveAggregationError;
use crate::observed_aggregates::Error as ObservedAttestationsError;
use crate::observed_attesters::Error as ObservedAttestersError;
use crate::observed_block_producers::Error as ObservedBlockProducersError;
use crate::observed_data_sidecars::Error as ObservedDataSidecarsError;
use execution_layer::PayloadStatus;
use fork_choice::ExecutionStatus;
use futures::channel::mpsc::TrySendError;
use operation_pool::OpPoolError;
use safe_arith::ArithError;
use ssz_types::Error as SszTypesError;
use state_processing::{
    BlockProcessingError, BlockReplayError, EpochProcessingError, SlotProcessingError,
    block_signature_verifier::Error as BlockSignatureVerifierError,
    envelope_processing::EnvelopeProcessingError,
    per_block_processing::errors::{
        AttestationValidationError, AttesterSlashingValidationError,
        BlsExecutionChangeValidationError, ExitValidationError, ProposerSlashingValidationError,
        SyncCommitteeMessageValidationError,
    },
    signature_sets::Error as SignatureSetError,
    state_advance::Error as StateAdvanceError,
};
use task_executor::ShutdownReason;
use tokio::task::JoinError;
use types::milhouse::Error as MilhouseError;
use types::*;

macro_rules! easy_from_to {
    ($from: ident, $to: ident) => {
        impl From<$from> for $to {
            fn from(e: $from) -> $to {
                $to::$from(e)
            }
        }
    };
}

#[derive(Debug)]
pub enum BeaconChainError {
    UnableToReadSlot,
    UnableToComputeTimeAtSlot,
    RevertedFinalizedEpoch {
        old: Checkpoint,
        new: Checkpoint,
    },
    NoStateForSlot(Slot),
    BeaconStateError(BeaconStateError),
    EpochCacheError(EpochCacheError),
    DBInconsistent(String),
    DBError(store::Error),
    ForkChoiceError(ForkChoiceError),
    ForkChoiceStoreError(ForkChoiceStoreError),
    MissingBeaconBlock(Hash256),
    MissingBeaconState(Hash256),
    MissingHotStateSummary(Hash256),
    SlotProcessingError(SlotProcessingError),
    EpochProcessingError(EpochProcessingError),
    StateAdvanceError(StateAdvanceError),
    CannotAttestToFutureState,
    AttestationValidationError(AttestationValidationError),
    SyncCommitteeMessageValidationError(SyncCommitteeMessageValidationError),
    ExitValidationError(ExitValidationError),
    ProposerSlashingValidationError(ProposerSlashingValidationError),
    AttesterSlashingValidationError(AttesterSlashingValidationError),
    BlsExecutionChangeValidationError(BlsExecutionChangeValidationError),
    MissingFinalizedStateRoot(Slot),
    SszTypesError(SszTypesError),
    IncorrectStateForAttestation(RelativeEpochError),
    InvalidValidatorPubkeyBytes(bls::Error),
    ValidatorPubkeyCacheIncomplete(usize),
    SignatureSetError(SignatureSetError),
    BlockSignatureVerifierError(state_processing::block_signature_verifier::Error),
    BlockReplayError(BlockReplayError),
    DuplicateValidatorPublicKey,
    ValidatorIndexUnknown(usize),
    ValidatorPubkeyUnknown(PublicKeyBytes),
    OpPoolError(OpPoolError),
    NaiveAggregationError(NaiveAggregationError),
    ObservedAttestationsError(ObservedAttestationsError),
    ObservedAttestersError(ObservedAttestersError),
    ObservedBlockProducersError(ObservedBlockProducersError),
    ObservedDataSidecarsError(ObservedDataSidecarsError),
    AttesterCacheError(AttesterCacheError),
    PruningError(PruningError),
    ArithError(ArithError),
    InvalidShufflingId {
        shuffling_epoch: Epoch,
        head_block_epoch: Epoch,
    },
    WeakSubjectivtyVerificationFailure,
    WeakSubjectivtyShutdownError(TrySendError<ShutdownReason>),
    AttestingToFinalizedSlot {
        finalized_slot: Slot,
        request_slot: Slot,
    },
    AttestingToAncientSlot {
        lowest_permissible_slot: Slot,
        request_slot: Slot,
    },
    BadPreState {
        parent_root: Hash256,
        parent_slot: Slot,
        block_root: Hash256,
        block_slot: Slot,
        state_slot: Slot,
    },
    /// Block is not available (only returned when fetching historic blocks).
    HistoricalBlockOutOfRange {
        slot: Slot,
        oldest_block_slot: Slot,
    },
    InvalidStateForShuffling {
        state_epoch: Epoch,
        shuffling_epoch: Epoch,
    },
    SyncDutiesError(BeaconStateError),
    InconsistentForwardsIter {
        request_slot: Slot,
        slot: Slot,
    },
    InvalidReorgSlotIter {
        old_slot: Slot,
        new_slot: Slot,
    },
    AltairForkDisabled,
    BuilderMissing,
    ExecutionLayerMissing,
    BlockVariantLacksExecutionPayload(Hash256),
    ExecutionLayerErrorPayloadReconstruction(ExecutionBlockHash, Box<execution_layer::Error>),
    EngineGetCapabilititesFailed(Box<execution_layer::Error>),
    ExecutionLayerGetBlockByNumberFailed(Box<execution_layer::Error>),
    BlockHashMissingFromExecutionLayer(ExecutionBlockHash),
    InconsistentPayloadReconstructed {
        slot: Slot,
        exec_block_hash: ExecutionBlockHash,
        canonical_transactions_root: Hash256,
        reconstructed_transactions_root: Hash256,
    },
    BlockStreamerError(BlockStreamerError),
    AddPayloadLogicError,
    ExecutionForkChoiceUpdateFailed(execution_layer::Error),
    PrepareProposerFailed(BlockProcessingError),
    ExecutionForkChoiceUpdateInvalid {
        status: PayloadStatus,
    },
    BlockRewardError,
    BlockRewardSlotError,
    BlockRewardAttestationError,
    BlockRewardSyncError,
    SyncCommitteeRewardsSyncError,
    AttestationRewardsError,
    HeadMissingFromForkChoice(Hash256),
    HeadBlockMissingFromForkChoice(Hash256),
    InvalidFinalizedPayload {
        finalized_root: Hash256,
        execution_block_hash: ExecutionBlockHash,
    },
    InvalidFinalizedPayloadShutdownError(TrySendError<ShutdownReason>),
    JustifiedPayloadInvalid {
        justified_root: Hash256,
        execution_block_hash: Option<ExecutionBlockHash>,
    },
    ForkchoiceUpdate(execution_layer::Error),
    InvalidCheckpoint {
        state_root: Hash256,
        checkpoint: Checkpoint,
    },
    InvalidSlot(Slot),
    HeadBlockNotFullyVerified {
        beacon_block_root: Hash256,
        execution_status: ExecutionStatus,
    },
    CannotAttestToFinalizedBlock {
        beacon_block_root: Hash256,
    },
    SyncContributionDataReferencesFinalizedBlock {
        beacon_block_root: Hash256,
    },
    RuntimeShutdown,
    TokioJoin(tokio::task::JoinError),
    ForkChoiceSignalOutOfOrder {
        current: Slot,
        latest: Slot,
    },
    HeadHasInvalidPayload {
        block_root: Hash256,
        execution_status: ExecutionStatus,
    },
    AttestationHeadNotInForkChoice(Hash256),
    MissingPersistedForkChoice,
    CommitteePromiseFailed(oneshot_broadcast::Error),
    MaxCommitteePromises(usize),
    BlsToExecutionPriorToCapella,
    BlsToExecutionConflictsWithPool,
    InconsistentFork(InconsistentFork),
    ProposerHeadForkChoiceError(fork_choice::Error<proto_array::Error>),
    UnableToPublish,
    AvailabilityCheckError(AvailabilityCheckError),
    LightClientUpdateError(LightClientUpdateError),
    LightClientBootstrapError(String),
    MilhouseError(MilhouseError),
    AttestationError(AttestationError),
    AttestationCommitteeIndexNotSet,
    InsufficientColumnsToReconstructBlobs {
        columns_found: usize,
    },
    FailedToReconstructBlobs(String),
    ProposerCacheIncorrectState {
        state_decision_block_root: Hash256,
        requested_decision_block_root: Hash256,
    },
    ProposerCacheOutOfBounds {
        slot: Slot,
        epoch: Epoch,
    },
    ProposerCacheWrongEpoch {
        request_epoch: Epoch,
        cache_epoch: Epoch,
    },
    SkipProposerPreparation,
    FailedColumnCustodyInfoUpdate,
    EnvelopeProcessingError(EnvelopeProcessingError),
    EnvelopeError(String),
    PayloadAttestationValidatorNotInPtc {
        validator_index: u64,
        slot: Slot,
    },
    PayloadAttestationBitOutOfBounds {
        position: usize,
    },
    PayloadAttestationVerificationFailed(String),
    BlockProcessingError(BlockProcessingError),
}

easy_from_to!(SlotProcessingError, BeaconChainError);
easy_from_to!(EpochProcessingError, BeaconChainError);
easy_from_to!(AttestationValidationError, BeaconChainError);
easy_from_to!(SyncCommitteeMessageValidationError, BeaconChainError);
easy_from_to!(ExitValidationError, BeaconChainError);
easy_from_to!(ProposerSlashingValidationError, BeaconChainError);
easy_from_to!(AttesterSlashingValidationError, BeaconChainError);
easy_from_to!(BlsExecutionChangeValidationError, BeaconChainError);
easy_from_to!(SszTypesError, BeaconChainError);
easy_from_to!(OpPoolError, BeaconChainError);
easy_from_to!(NaiveAggregationError, BeaconChainError);
easy_from_to!(ObservedAttestationsError, BeaconChainError);
easy_from_to!(ObservedAttestersError, BeaconChainError);
easy_from_to!(ObservedBlockProducersError, BeaconChainError);
easy_from_to!(ObservedDataSidecarsError, BeaconChainError);
easy_from_to!(AttesterCacheError, BeaconChainError);
easy_from_to!(BlockSignatureVerifierError, BeaconChainError);
easy_from_to!(PruningError, BeaconChainError);
easy_from_to!(ArithError, BeaconChainError);
easy_from_to!(ForkChoiceStoreError, BeaconChainError);
easy_from_to!(StateAdvanceError, BeaconChainError);
easy_from_to!(BlockReplayError, BeaconChainError);
easy_from_to!(InconsistentFork, BeaconChainError);
easy_from_to!(AvailabilityCheckError, BeaconChainError);
easy_from_to!(EpochCacheError, BeaconChainError);
easy_from_to!(LightClientUpdateError, BeaconChainError);
easy_from_to!(MilhouseError, BeaconChainError);
easy_from_to!(AttestationError, BeaconChainError);

#[derive(Debug)]
pub enum BlockProductionError {
    UnableToGetBlockRootFromState,
    UnableToReadSlot,
    UnableToProduceAtSlot(Slot),
    SlotProcessingError(SlotProcessingError),
    BlockProcessingError(BlockProcessingError),
    EpochCacheError(EpochCacheError),
    ForkChoiceError(ForkChoiceError),
    BeaconStateError(BeaconStateError),
    StateAdvanceError(StateAdvanceError),
    OpPoolError(OpPoolError),
    StateSlotTooHigh {
        produce_at_slot: Slot,
        state_slot: Slot,
    },
    ExecutionLayerMissing,
    TerminalPoWBlockLookupFailed(execution_layer::Error),
    GetPayloadFailed(execution_layer::Error),
    FailedToLoadState(store::Error),
    BlockTooLarge(usize),
    ShuttingDown,
    MissingBlobs,
    MissingSyncAggregate,
    MissingExecutionPayload,
    MissingKzgCommitment(String),
    TokioJoin(JoinError),
    BeaconChain(Box<BeaconChainError>),
    InvalidPayloadFork,
    InvalidBlockVariant(String),
    MissingExecutionRequests,
    EnvelopeConstructionFailed(String),
}

easy_from_to!(BlockProcessingError, BlockProductionError);
easy_from_to!(BeaconStateError, BlockProductionError);
easy_from_to!(SlotProcessingError, BlockProductionError);
easy_from_to!(StateAdvanceError, BlockProductionError);
easy_from_to!(ForkChoiceError, BlockProductionError);
easy_from_to!(EpochCacheError, BlockProductionError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_arith_error() {
        let err: BeaconChainError = ArithError::Overflow.into();
        assert!(matches!(
            err,
            BeaconChainError::ArithError(ArithError::Overflow)
        ));
    }

    #[test]
    fn from_ssz_types_error() {
        let err: BeaconChainError = SszTypesError::OutOfBounds { i: 10, len: 5 }.into();
        assert!(matches!(err, BeaconChainError::SszTypesError(_)));
    }

    #[test]
    fn from_block_replay_error() {
        let err: BeaconChainError =
            BlockReplayError::BeaconState(BeaconStateError::InsufficientValidators).into();
        assert!(matches!(err, BeaconChainError::BlockReplayError(_)));
    }

    #[test]
    fn from_state_advance_error() {
        let err: BeaconChainError = StateAdvanceError::StateRootNotProvided.into();
        assert!(matches!(err, BeaconChainError::StateAdvanceError(_)));
    }

    #[test]
    fn from_inconsistent_fork() {
        let err: BeaconChainError = InconsistentFork {
            fork_at_slot: ForkName::Base,
            object_fork: ForkName::Altair,
        }
        .into();
        assert!(matches!(err, BeaconChainError::InconsistentFork(_)));
    }

    #[test]
    fn beacon_chain_error_debug_format() {
        let err = BeaconChainError::UnableToReadSlot;
        let debug = format!("{:?}", err);
        assert!(debug.contains("UnableToReadSlot"));
    }

    #[test]
    fn beacon_chain_error_no_state_for_slot() {
        let err = BeaconChainError::NoStateForSlot(Slot::new(99));
        match err {
            BeaconChainError::NoStateForSlot(slot) => assert_eq!(slot, Slot::new(99)),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_missing_beacon_block() {
        let root = Hash256::repeat_byte(0xab);
        let err = BeaconChainError::MissingBeaconBlock(root);
        match err {
            BeaconChainError::MissingBeaconBlock(r) => assert_eq!(r, root),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_missing_beacon_state() {
        let root = Hash256::repeat_byte(0xcd);
        let err = BeaconChainError::MissingBeaconState(root);
        match err {
            BeaconChainError::MissingBeaconState(r) => assert_eq!(r, root),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_db_error() {
        let err = BeaconChainError::DBError(store::Error::DBError {
            message: "test".to_string(),
        });
        assert!(matches!(err, BeaconChainError::DBError(_)));
    }

    #[test]
    fn beacon_chain_error_db_inconsistent() {
        let err = BeaconChainError::DBInconsistent("bad data".to_string());
        match err {
            BeaconChainError::DBInconsistent(msg) => assert_eq!(msg, "bad data"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_production_error_debug_format() {
        let err = BlockProductionError::UnableToReadSlot;
        let debug = format!("{:?}", err);
        assert!(debug.contains("UnableToReadSlot"));
    }

    #[test]
    fn block_production_error_from_beacon_state_error() {
        let err: BlockProductionError = BeaconStateError::InsufficientValidators.into();
        assert!(matches!(err, BlockProductionError::BeaconStateError(_)));
    }

    #[test]
    fn block_production_error_state_slot_too_high() {
        let err = BlockProductionError::StateSlotTooHigh {
            produce_at_slot: Slot::new(5),
            state_slot: Slot::new(10),
        };
        match err {
            BlockProductionError::StateSlotTooHigh {
                produce_at_slot,
                state_slot,
            } => {
                assert_eq!(produce_at_slot, Slot::new(5));
                assert_eq!(state_slot, Slot::new(10));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_attesting_to_finalized_slot() {
        let err = BeaconChainError::AttestingToFinalizedSlot {
            finalized_slot: Slot::new(64),
            request_slot: Slot::new(32),
        };
        match err {
            BeaconChainError::AttestingToFinalizedSlot {
                finalized_slot,
                request_slot,
            } => {
                assert_eq!(finalized_slot, Slot::new(64));
                assert_eq!(request_slot, Slot::new(32));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_reverted_finalized_epoch() {
        let old = Checkpoint {
            epoch: Epoch::new(10),
            root: Hash256::repeat_byte(0x01),
        };
        let new = Checkpoint {
            epoch: Epoch::new(9),
            root: Hash256::repeat_byte(0x02),
        };
        let err = BeaconChainError::RevertedFinalizedEpoch { old, new };
        match err {
            BeaconChainError::RevertedFinalizedEpoch { old: o, new: n } => {
                assert_eq!(o.epoch, Epoch::new(10));
                assert_eq!(n.epoch, Epoch::new(9));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn beacon_chain_error_payload_attestation_not_in_ptc() {
        let err = BeaconChainError::PayloadAttestationValidatorNotInPtc {
            validator_index: 42,
            slot: Slot::new(100),
        };
        match err {
            BeaconChainError::PayloadAttestationValidatorNotInPtc {
                validator_index,
                slot,
            } => {
                assert_eq!(validator_index, 42);
                assert_eq!(slot, Slot::new(100));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_production_error_invalid_block_variant() {
        let err = BlockProductionError::InvalidBlockVariant("wrong fork".to_string());
        match err {
            BlockProductionError::InvalidBlockVariant(msg) => {
                assert_eq!(msg, "wrong fork");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn block_production_error_envelope_construction_failed() {
        let err = BlockProductionError::EnvelopeConstructionFailed("missing payload".to_string());
        match err {
            BlockProductionError::EnvelopeConstructionFailed(msg) => {
                assert_eq!(msg, "missing payload");
            }
            _ => panic!("wrong variant"),
        }
    }
}
