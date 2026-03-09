//! Custom axum extractors for the beacon node HTTP API.

use crate::api_error::ApiError;
use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;
use eth2::types as api_types;
use eth2::{CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER};
use serde::de::DeserializeOwned;
use types::ForkName;

/// Extract `Accept` header as an optional `api_types::Accept`.
pub fn accept_header(headers: &HeaderMap) -> Option<api_types::Accept> {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
}

/// Extract the `Eth-Consensus-Version` header as a required `ForkName`.
pub fn consensus_version_header(headers: &HeaderMap) -> Result<ForkName, ApiError> {
    headers
        .get(eth2::CONSENSUS_VERSION_HEADER)
        .ok_or_else(|| ApiError::bad_request("missing Eth-Consensus-Version header"))?
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid Eth-Consensus-Version header"))?
        .parse()
        .map_err(|_| ApiError::bad_request("unknown fork name in Eth-Consensus-Version header"))
}

/// Extract the `Eth-Consensus-Version` header as an optional `ForkName`.
pub fn optional_consensus_version_header(headers: &HeaderMap) -> Option<ForkName> {
    headers
        .get(eth2::CONSENSUS_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
}

/// Custom extractor for query strings with duplicate keys (e.g., `?topics=head&topics=block`).
/// Uses `serde_array_query` for deserialization.
pub struct MultiKeyQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for MultiKeyQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query_str = parts.uri.query().unwrap_or_default();
        let value = serde_array_query::from_str(query_str)
            .map_err(|e| ApiError::bad_request(format!("invalid query: {e}")))?;
        Ok(MultiKeyQuery(value))
    }
}

/// Extract and validate a JSON body, rejecting SSZ content-type.
/// This replaces `warp_utils::json::json()`.
pub async fn json_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<T, ApiError> {
    check_not_ssz(headers)?;
    serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("body deserialize error: {e:?}")))
}

/// Extract and validate a JSON body that may be empty (returns T::default() for empty bodies).
/// This replaces `warp_utils::json::json_no_body()`.
pub async fn json_body_or_default<T: DeserializeOwned + Default>(
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<T, ApiError> {
    check_not_ssz(headers)?;
    if body.is_empty() {
        return Ok(T::default());
    }
    serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("body deserialize error: {e:?}")))
}

fn check_not_ssz(headers: &HeaderMap) -> Result<(), ApiError> {
    if let Some(ct) = headers.get(CONTENT_TYPE_HEADER)
        && ct.as_bytes() == SSZ_CONTENT_TYPE_HEADER.as_bytes()
    {
        return Err(ApiError::unsupported_media_type(
            "The request's content-type is not supported",
        ));
    }
    Ok(())
}

/// Parse an `EndpointVersion` from a path segment string like "v1", "v2", "v3".
pub fn parse_endpoint_version(version_str: &str) -> Result<api_types::EndpointVersion, ApiError> {
    version_str
        .parse::<api_types::EndpointVersion>()
        .map_err(|_| ApiError::bad_request("Invalid version identifier"))
}
