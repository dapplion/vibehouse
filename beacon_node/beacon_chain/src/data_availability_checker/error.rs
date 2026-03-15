use kzg::{Error as KzgError, KzgCommitment};
use types::{BeaconStateError, ColumnIndex, Hash256};

#[derive(Debug)]
pub enum Error {
    InvalidBlobs(KzgError),
    InvalidColumn((Option<ColumnIndex>, KzgError)),
    ReconstructColumnsError(KzgError),
    KzgCommitmentMismatch {
        blob_commitment: KzgCommitment,
        block_commitment: KzgCommitment,
    },
    Unexpected(String),
    SszTypes(ssz_types::Error),
    MissingBlobs,
    MissingCustodyColumns,
    BlobIndexInvalid(u64),
    DataColumnIndexInvalid(u64),
    StoreError(store::Error),
    DecodeError(ssz::DecodeError),
    ParentStateMissing(Hash256),
    BlockReplayError(state_processing::BlockReplayError),
    RebuildingStateCaches(BeaconStateError),
    SlotClockError,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Internal Errors (not caused by peers)
    Internal,
    /// Errors caused by faulty / malicious peers
    Malicious,
}

impl Error {
    pub fn category(&self) -> ErrorCategory {
        match self {
            Error::SszTypes(_)
            | Error::MissingBlobs
            | Error::MissingCustodyColumns
            | Error::StoreError(_)
            | Error::DecodeError(_)
            | Error::Unexpected(_)
            | Error::ParentStateMissing(_)
            | Error::BlockReplayError(_)
            | Error::RebuildingStateCaches(_)
            | Error::SlotClockError => ErrorCategory::Internal,
            Error::InvalidBlobs { .. }
            | Error::InvalidColumn { .. }
            | Error::ReconstructColumnsError { .. }
            | Error::BlobIndexInvalid(_)
            | Error::DataColumnIndexInvalid(_)
            | Error::KzgCommitmentMismatch { .. } => ErrorCategory::Malicious,
        }
    }
}

impl From<ssz_types::Error> for Error {
    fn from(value: ssz_types::Error) -> Self {
        Self::SszTypes(value)
    }
}

impl From<store::Error> for Error {
    fn from(value: store::Error) -> Self {
        Self::StoreError(value)
    }
}

impl From<ssz::DecodeError> for Error {
    fn from(value: ssz::DecodeError) -> Self {
        Self::DecodeError(value)
    }
}

impl From<state_processing::BlockReplayError> for Error {
    fn from(value: state_processing::BlockReplayError) -> Self {
        Self::BlockReplayError(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FixedBytesExtended;

    #[test]
    fn internal_errors_category() {
        let internal_errors: Vec<Error> = vec![
            Error::SszTypes(ssz_types::Error::OutOfBounds { i: 0, len: 0 }),
            Error::MissingBlobs,
            Error::MissingCustodyColumns,
            Error::StoreError(store::Error::DBError {
                message: "test".into(),
            }),
            Error::DecodeError(ssz::DecodeError::InvalidByteLength {
                len: 0,
                expected: 1,
            }),
            Error::Unexpected("test".into()),
            Error::ParentStateMissing(Hash256::zero()),
            Error::SlotClockError,
        ];
        for err in &internal_errors {
            assert_eq!(err.category(), ErrorCategory::Internal, "{:?}", err);
        }
    }

    #[test]
    fn malicious_errors_category() {
        let malicious_errors: Vec<Error> = vec![
            Error::BlobIndexInvalid(99),
            Error::DataColumnIndexInvalid(99),
        ];
        for err in &malicious_errors {
            assert_eq!(err.category(), ErrorCategory::Malicious, "{:?}", err);
        }
    }

    #[test]
    fn from_ssz_types_error() {
        let err: Error = ssz_types::Error::OutOfBounds { i: 0, len: 0 }.into();
        assert!(matches!(err, Error::SszTypes(_)));
    }

    #[test]
    fn from_store_error() {
        let err: Error = store::Error::DBError {
            message: "test".into(),
        }
        .into();
        assert!(matches!(err, Error::StoreError(_)));
    }

    #[test]
    fn from_decode_error() {
        let err: Error = ssz::DecodeError::InvalidByteLength {
            len: 0,
            expected: 1,
        }
        .into();
        assert!(matches!(err, Error::DecodeError(_)));
    }

    #[test]
    fn error_category_eq() {
        assert_eq!(ErrorCategory::Internal, ErrorCategory::Internal);
        assert_eq!(ErrorCategory::Malicious, ErrorCategory::Malicious);
        assert_ne!(ErrorCategory::Internal, ErrorCategory::Malicious);
    }

    #[test]
    fn debug_format() {
        let err = Error::BlobIndexInvalid(42);
        let dbg = format!("{:?}", err);
        assert!(dbg.contains("BlobIndexInvalid"));
        assert!(dbg.contains("42"));
    }
}
