use crate::api_error::ApiError;
use crate::api_types::EndpointVersion;
use axum::response::Response;
use eth2::{
    CONSENSUS_BLOCK_VALUE_HEADER, CONSENSUS_VERSION_HEADER, CONTENT_TYPE_HEADER,
    EXECUTION_PAYLOAD_BLINDED_HEADER, EXECUTION_PAYLOAD_VALUE_HEADER, SSZ_CONTENT_TYPE_HEADER,
};
use serde::Serialize;
use types::{
    BeaconResponse, ForkName, ForkVersionedResponse, InconsistentFork, Uint256,
    UnversionedResponse,
    beacon_response::{
        ExecutionOptimisticFinalizedBeaconResponse, ExecutionOptimisticFinalizedMetadata,
    },
};

pub const V1: EndpointVersion = EndpointVersion(1);
pub const V2: EndpointVersion = EndpointVersion(2);
pub const V3: EndpointVersion = EndpointVersion(3);

#[derive(Debug, PartialEq, Clone, Serialize)]
pub enum ResponseIncludesVersion {
    Yes(ForkName),
    No,
}

pub fn beacon_response<T: Serialize>(
    require_version: ResponseIncludesVersion,
    data: T,
) -> BeaconResponse<T> {
    match require_version {
        ResponseIncludesVersion::Yes(fork_name) => {
            BeaconResponse::ForkVersioned(ForkVersionedResponse {
                version: fork_name,
                metadata: Default::default(),
                data,
            })
        }
        ResponseIncludesVersion::No => BeaconResponse::Unversioned(UnversionedResponse {
            metadata: Default::default(),
            data,
        }),
    }
}

pub fn execution_optimistic_finalized_beacon_response<T: Serialize>(
    require_version: ResponseIncludesVersion,
    execution_optimistic: bool,
    finalized: bool,
    data: T,
) -> Result<ExecutionOptimisticFinalizedBeaconResponse<T>, ApiError> {
    let metadata = ExecutionOptimisticFinalizedMetadata {
        execution_optimistic: Some(execution_optimistic),
        finalized: Some(finalized),
    };
    match require_version {
        ResponseIncludesVersion::Yes(fork_name) => {
            Ok(BeaconResponse::ForkVersioned(ForkVersionedResponse {
                version: fork_name,
                metadata,
                data,
            }))
        }
        ResponseIncludesVersion::No => Ok(BeaconResponse::Unversioned(UnversionedResponse {
            metadata,
            data,
        })),
    }
}

/// Add the 'Content-Type application/octet-stream` header to a response.
pub fn add_ssz_content_type_header(resp: Response) -> Response {
    let mut resp = resp;
    resp.headers_mut().insert(
        CONTENT_TYPE_HEADER,
        SSZ_CONTENT_TYPE_HEADER.parse().unwrap(),
    );
    resp
}

/// Add the `Eth-Consensus-Version` header to a response.
pub fn add_consensus_version_header(resp: Response, fork_name: ForkName) -> Response {
    let mut resp = resp;
    resp.headers_mut().insert(
        CONSENSUS_VERSION_HEADER,
        fork_name.to_string().parse().unwrap(),
    );
    resp
}

/// Add the `Eth-Execution-Payload-Blinded` header to a response.
pub fn add_execution_payload_blinded_header(
    resp: Response,
    execution_payload_blinded: bool,
) -> Response {
    let mut resp = resp;
    resp.headers_mut().insert(
        EXECUTION_PAYLOAD_BLINDED_HEADER,
        execution_payload_blinded.to_string().parse().unwrap(),
    );
    resp
}

/// Add the `Eth-Execution-Payload-Value` header to a response.
pub fn add_execution_payload_value_header(
    resp: Response,
    execution_payload_value: Uint256,
) -> Response {
    let mut resp = resp;
    resp.headers_mut().insert(
        EXECUTION_PAYLOAD_VALUE_HEADER,
        execution_payload_value.to_string().parse().unwrap(),
    );
    resp
}

/// Add the `Eth-Consensus-Block-Value` header to a response.
pub fn add_consensus_block_value_header(
    resp: Response,
    consensus_payload_value: Uint256,
) -> Response {
    let mut resp = resp;
    resp.headers_mut().insert(
        CONSENSUS_BLOCK_VALUE_HEADER,
        consensus_payload_value.to_string().parse().unwrap(),
    );
    resp
}

pub fn inconsistent_fork_rejection(error: InconsistentFork) -> ApiError {
    ApiError::server_error(format!("wrong fork: {:?}", error))
}

pub fn unsupported_version_rejection(version: EndpointVersion) -> ApiError {
    ApiError::bad_request(format!("Unsupported endpoint version: {}", version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use types::ForkName;

    #[test]
    fn response_includes_version_yes_debug() {
        let v = ResponseIncludesVersion::Yes(ForkName::Deneb);
        assert_eq!(v, ResponseIncludesVersion::Yes(ForkName::Deneb));
        assert_ne!(v, ResponseIncludesVersion::No);
    }

    #[test]
    fn response_includes_version_no_debug() {
        let v = ResponseIncludesVersion::No;
        assert_eq!(v.clone(), ResponseIncludesVersion::No);
    }

    #[test]
    fn beacon_response_versioned() {
        let resp = beacon_response(ResponseIncludesVersion::Yes(ForkName::Deneb), 42u64);
        match resp {
            BeaconResponse::ForkVersioned(fvr) => {
                assert_eq!(fvr.version, ForkName::Deneb);
                assert_eq!(fvr.data, 42u64);
            }
            _ => panic!("expected ForkVersioned"),
        }
    }

    #[test]
    fn beacon_response_unversioned() {
        let resp = beacon_response(ResponseIncludesVersion::No, "hello");
        match resp {
            BeaconResponse::Unversioned(ur) => {
                assert_eq!(ur.data, "hello");
            }
            _ => panic!("expected Unversioned"),
        }
    }

    #[test]
    fn execution_optimistic_finalized_versioned() {
        let resp = execution_optimistic_finalized_beacon_response(
            ResponseIncludesVersion::Yes(ForkName::Capella),
            true,
            false,
            100u64,
        )
        .unwrap();
        match resp {
            BeaconResponse::ForkVersioned(fvr) => {
                assert_eq!(fvr.version, ForkName::Capella);
                assert_eq!(fvr.data, 100u64);
                assert_eq!(fvr.metadata.execution_optimistic, Some(true));
                assert_eq!(fvr.metadata.finalized, Some(false));
            }
            _ => panic!("expected ForkVersioned"),
        }
    }

    #[test]
    fn execution_optimistic_finalized_unversioned() {
        let resp = execution_optimistic_finalized_beacon_response(
            ResponseIncludesVersion::No,
            false,
            true,
            200u64,
        )
        .unwrap();
        match resp {
            BeaconResponse::Unversioned(ur) => {
                assert_eq!(ur.data, 200u64);
                assert_eq!(ur.metadata.execution_optimistic, Some(false));
                assert_eq!(ur.metadata.finalized, Some(true));
            }
            _ => panic!("expected Unversioned"),
        }
    }

    #[tokio::test]
    async fn add_ssz_content_type_header_sets_header() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_ssz_content_type_header(resp);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE_HEADER).unwrap(),
            SSZ_CONTENT_TYPE_HEADER
        );
    }

    #[tokio::test]
    async fn add_consensus_version_header_sets_fork() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_consensus_version_header(resp, ForkName::Bellatrix);
        assert_eq!(
            resp.headers()
                .get(CONSENSUS_VERSION_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            "bellatrix"
        );
    }

    #[tokio::test]
    async fn add_execution_payload_blinded_header_true() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_execution_payload_blinded_header(resp, true);
        assert_eq!(
            resp.headers()
                .get(EXECUTION_PAYLOAD_BLINDED_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            "true"
        );
    }

    #[tokio::test]
    async fn add_execution_payload_blinded_header_false() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_execution_payload_blinded_header(resp, false);
        assert_eq!(
            resp.headers()
                .get(EXECUTION_PAYLOAD_BLINDED_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            "false"
        );
    }

    #[tokio::test]
    async fn add_execution_payload_value_header_sets_value() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_execution_payload_value_header(resp, Uint256::from(12345u64));
        assert_eq!(
            resp.headers()
                .get(EXECUTION_PAYLOAD_VALUE_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            "12345"
        );
    }

    #[tokio::test]
    async fn add_consensus_block_value_header_sets_value() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = add_consensus_block_value_header(resp, Uint256::from(999u64));
        assert_eq!(
            resp.headers()
                .get(CONSENSUS_BLOCK_VALUE_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            "999"
        );
    }

    #[test]
    fn inconsistent_fork_rejection_produces_server_error() {
        let err = inconsistent_fork_rejection(InconsistentFork {
            fork_at_slot: ForkName::Deneb,
            object_fork: ForkName::Capella,
        });
        let dbg = format!("{err:?}");
        assert!(dbg.starts_with("ServerError("));
    }

    #[test]
    fn unsupported_version_rejection_produces_bad_request() {
        let err = unsupported_version_rejection(EndpointVersion(99));
        let dbg = format!("{err:?}");
        assert!(dbg.starts_with("BadRequest("));
        assert!(dbg.contains("99"));
    }

    #[test]
    fn v1_v2_v3_constants() {
        assert_eq!(V1, EndpointVersion(1));
        assert_eq!(V2, EndpointVersion(2));
        assert_eq!(V3, EndpointVersion(3));
    }
}
