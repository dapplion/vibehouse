use types::{BeaconStateError, EpochCacheError, InconsistentFork, milhouse};

#[derive(Debug, PartialEq)]
pub enum EpochProcessingError {
    ValidatorStatusesInconsistent,
    DeltaOutOfBounds(usize),
    BeaconStateError(BeaconStateError),
    SszTypesError(ssz_types::Error),
    BitfieldError(ssz::BitfieldError),
    ArithError(safe_arith::ArithError),
    InconsistentStateFork(InconsistentFork),
    InvalidJustificationBit(ssz::BitfieldError),
    InvalidFlagIndex(usize),
    MilhouseError(milhouse::Error),
    EpochCache(EpochCacheError),
    SinglePassMissingActivationQueue,
    MissingEarliestExitEpoch,
    MissingExitBalanceToConsume,
    PendingDepositsLogicError,
    ProposerLookaheadOutOfBounds(usize),
}

impl From<BeaconStateError> for EpochProcessingError {
    fn from(e: BeaconStateError) -> EpochProcessingError {
        EpochProcessingError::BeaconStateError(e)
    }
}

impl From<ssz_types::Error> for EpochProcessingError {
    fn from(e: ssz_types::Error) -> EpochProcessingError {
        EpochProcessingError::SszTypesError(e)
    }
}

impl From<ssz::BitfieldError> for EpochProcessingError {
    fn from(e: ssz::BitfieldError) -> EpochProcessingError {
        EpochProcessingError::BitfieldError(e)
    }
}

impl From<safe_arith::ArithError> for EpochProcessingError {
    fn from(e: safe_arith::ArithError) -> EpochProcessingError {
        EpochProcessingError::ArithError(e)
    }
}

impl From<milhouse::Error> for EpochProcessingError {
    fn from(e: milhouse::Error) -> Self {
        Self::MilhouseError(e)
    }
}

impl From<EpochCacheError> for EpochProcessingError {
    fn from(e: EpochCacheError) -> Self {
        EpochProcessingError::EpochCache(e)
    }
}
