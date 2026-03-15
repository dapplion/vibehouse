use crate::engine_api::{Error as ApiError, PayloadStatusV1, PayloadStatusV1Status};
use crate::engines::EngineError;
use tracing::warn;
use types::ExecutionBlockHash;

/// Provides a simpler, easier to parse version of `PayloadStatusV1` for upstream users.
///
/// It primarily ensures that the `latest_valid_hash` is always present when relevant.
#[derive(Debug, Clone, PartialEq)]
pub enum PayloadStatus {
    Valid,
    Invalid {
        /// The EE will provide a `None` LVH when it is unable to determine the
        /// latest valid ancestor.
        latest_valid_hash: Option<ExecutionBlockHash>,
        validation_error: Option<String>,
    },
    Syncing,
    Accepted,
    InvalidBlockHash {
        validation_error: Option<String>,
    },
}

/// Processes the response from the execution engine.
pub fn process_payload_status(
    head_block_hash: ExecutionBlockHash,
    status: Result<PayloadStatusV1, EngineError>,
) -> Result<PayloadStatus, EngineError> {
    match status {
        Err(error) => {
            warn!(?error, "Error whilst processing payload status");
            Err(error)
        }
        Ok(response) => match &response.status {
            PayloadStatusV1Status::Valid => {
                if response
                    .latest_valid_hash
                    .is_some_and(|h| h == head_block_hash)
                {
                    // The response is only valid if `latest_valid_hash` is not `null` and
                    // equal to the provided `block_hash`.
                    Ok(PayloadStatus::Valid)
                } else {
                    let error = format!(
                        "new_payload: response.status = VALID but invalid latest_valid_hash. Expected({:?}) Found({:?})",
                        head_block_hash, response.latest_valid_hash
                    );
                    Err(EngineError::Api {
                        error: ApiError::BadResponse(error),
                    })
                }
            }
            PayloadStatusV1Status::Invalid => Ok(PayloadStatus::Invalid {
                latest_valid_hash: response.latest_valid_hash,
                validation_error: response.validation_error,
            }),
            PayloadStatusV1Status::InvalidBlockHash => {
                // In the interests of being liberal with what we accept, only raise a
                // warning here.
                if response.latest_valid_hash.is_some() {
                    warn!(
                        msg = "expected a null latest_valid_hash",
                        status = ?response.status,
                    "Malformed response from execution engine"
                    )
                }

                Ok(PayloadStatus::InvalidBlockHash {
                    validation_error: response.validation_error.clone(),
                })
            }
            PayloadStatusV1Status::Syncing => {
                // In the interests of being liberal with what we accept, only raise a
                // warning here.
                if response.latest_valid_hash.is_some() {
                    warn!(
                        msg = "expected a null latest_valid_hash",
                        status = ?response.status,
                    "Malformed response from execution engine"
                    )
                }

                Ok(PayloadStatus::Syncing)
            }
            PayloadStatusV1Status::Accepted => {
                // In the interests of being liberal with what we accept, only raise a
                // warning here.
                if response.latest_valid_hash.is_some() {
                    warn!(
                        msg = "expected a null latest_valid_hash",
                        status = ?response.status,
                    "Malformed response from execution engine"
                    )
                }

                Ok(PayloadStatus::Accepted)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> ExecutionBlockHash {
        ExecutionBlockHash::from_root(types::Hash256::repeat_byte(byte))
    }

    fn ok_status(
        status: PayloadStatusV1Status,
        latest_valid_hash: Option<ExecutionBlockHash>,
        validation_error: Option<String>,
    ) -> Result<PayloadStatusV1, EngineError> {
        Ok(PayloadStatusV1 {
            status,
            latest_valid_hash,
            validation_error,
        })
    }

    #[test]
    fn valid_with_matching_hash() {
        let head = hash(1);
        let result = process_payload_status(
            head,
            ok_status(PayloadStatusV1Status::Valid, Some(head), None),
        );
        assert_eq!(result.unwrap(), PayloadStatus::Valid);
    }

    #[test]
    fn valid_with_mismatched_hash_is_error() {
        let head = hash(1);
        let other = hash(2);
        let result = process_payload_status(
            head,
            ok_status(PayloadStatusV1Status::Valid, Some(other), None),
        );
        assert!(result.is_err());
    }

    #[test]
    fn valid_with_null_hash_is_error() {
        let head = hash(1);
        let result =
            process_payload_status(head, ok_status(PayloadStatusV1Status::Valid, None, None));
        assert!(result.is_err());
    }

    #[test]
    fn invalid_status() {
        let head = hash(1);
        let lvh = hash(2);
        let result = process_payload_status(
            head,
            ok_status(
                PayloadStatusV1Status::Invalid,
                Some(lvh),
                Some("bad block".to_string()),
            ),
        );
        assert_eq!(
            result.unwrap(),
            PayloadStatus::Invalid {
                latest_valid_hash: Some(lvh),
                validation_error: Some("bad block".to_string()),
            }
        );
    }

    #[test]
    fn invalid_with_no_lvh() {
        let head = hash(1);
        let result =
            process_payload_status(head, ok_status(PayloadStatusV1Status::Invalid, None, None));
        assert_eq!(
            result.unwrap(),
            PayloadStatus::Invalid {
                latest_valid_hash: None,
                validation_error: None,
            }
        );
    }

    #[test]
    fn invalid_block_hash_status() {
        let head = hash(1);
        let result = process_payload_status(
            head,
            ok_status(
                PayloadStatusV1Status::InvalidBlockHash,
                None,
                Some("hash mismatch".to_string()),
            ),
        );
        assert_eq!(
            result.unwrap(),
            PayloadStatus::InvalidBlockHash {
                validation_error: Some("hash mismatch".to_string()),
            }
        );
    }

    #[test]
    fn syncing_status() {
        let head = hash(1);
        let result =
            process_payload_status(head, ok_status(PayloadStatusV1Status::Syncing, None, None));
        assert_eq!(result.unwrap(), PayloadStatus::Syncing);
    }

    #[test]
    fn accepted_status() {
        let head = hash(1);
        let result =
            process_payload_status(head, ok_status(PayloadStatusV1Status::Accepted, None, None));
        assert_eq!(result.unwrap(), PayloadStatus::Accepted);
    }

    #[test]
    fn engine_error_propagated() {
        let head = hash(1);
        let result = process_payload_status(head, Err(EngineError::Offline));
        assert!(result.is_err());
    }

    #[test]
    fn syncing_with_unexpected_lvh_still_ok() {
        // The function is liberal — unexpected LVH just logs a warning
        let head = hash(1);
        let result = process_payload_status(
            head,
            ok_status(PayloadStatusV1Status::Syncing, Some(hash(2)), None),
        );
        assert_eq!(result.unwrap(), PayloadStatus::Syncing);
    }

    #[test]
    fn accepted_with_unexpected_lvh_still_ok() {
        let head = hash(1);
        let result = process_payload_status(
            head,
            ok_status(PayloadStatusV1Status::Accepted, Some(hash(2)), None),
        );
        assert_eq!(result.unwrap(), PayloadStatus::Accepted);
    }

    #[test]
    fn invalid_block_hash_with_unexpected_lvh_still_ok() {
        let head = hash(1);
        let result = process_payload_status(
            head,
            ok_status(PayloadStatusV1Status::InvalidBlockHash, Some(hash(2)), None),
        );
        assert!(matches!(
            result.unwrap(),
            PayloadStatus::InvalidBlockHash { .. }
        ));
    }
}
