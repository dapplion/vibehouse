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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_beacon_state_error() {
        let e: EpochProcessingError = BeaconStateError::InsufficientValidators.into();
        assert_eq!(
            e,
            EpochProcessingError::BeaconStateError(BeaconStateError::InsufficientValidators)
        );
    }

    #[test]
    fn from_ssz_types_error() {
        let e: EpochProcessingError = ssz_types::Error::OutOfBounds { i: 5, len: 3 }.into();
        match e {
            EpochProcessingError::SszTypesError(ssz_types::Error::OutOfBounds { i, len }) => {
                assert_eq!(i, 5);
                assert_eq!(len, 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn from_bitfield_error() {
        let e: EpochProcessingError = ssz::BitfieldError::OutOfBounds { i: 10, len: 8 }.into();
        match e {
            EpochProcessingError::BitfieldError(ssz::BitfieldError::OutOfBounds { i, len }) => {
                assert_eq!(i, 10);
                assert_eq!(len, 8);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn from_arith_error() {
        let e: EpochProcessingError = safe_arith::ArithError::Overflow.into();
        assert_eq!(
            e,
            EpochProcessingError::ArithError(safe_arith::ArithError::Overflow)
        );
    }

    #[test]
    fn from_milhouse_error() {
        let e: EpochProcessingError = milhouse::Error::InvalidListUpdate.into();
        match e {
            EpochProcessingError::MilhouseError(_) => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn from_epoch_cache_error() {
        let e: EpochProcessingError = EpochCacheError::CacheNotInitialized.into();
        match e {
            EpochProcessingError::EpochCache(EpochCacheError::CacheNotInitialized) => {}
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn delta_out_of_bounds() {
        let e = EpochProcessingError::DeltaOutOfBounds(42);
        match e {
            EpochProcessingError::DeltaOutOfBounds(i) => assert_eq!(i, 42),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn invalid_flag_index() {
        let e = EpochProcessingError::InvalidFlagIndex(7);
        match e {
            EpochProcessingError::InvalidFlagIndex(i) => assert_eq!(i, 7),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn proposer_lookahead_out_of_bounds() {
        let e = EpochProcessingError::ProposerLookaheadOutOfBounds(100);
        match e {
            EpochProcessingError::ProposerLookaheadOutOfBounds(i) => assert_eq!(i, 100),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn singleton_variants_debug() {
        let variants = vec![
            EpochProcessingError::ValidatorStatusesInconsistent,
            EpochProcessingError::SinglePassMissingActivationQueue,
            EpochProcessingError::MissingEarliestExitEpoch,
            EpochProcessingError::MissingExitBalanceToConsume,
            EpochProcessingError::PendingDepositsLogicError,
        ];
        for v in &variants {
            let dbg = format!("{v:?}");
            assert!(!dbg.is_empty());
        }
    }

    #[test]
    fn equality() {
        assert_eq!(
            EpochProcessingError::ValidatorStatusesInconsistent,
            EpochProcessingError::ValidatorStatusesInconsistent
        );
        assert_ne!(
            EpochProcessingError::ValidatorStatusesInconsistent,
            EpochProcessingError::MissingEarliestExitEpoch
        );
    }
}
