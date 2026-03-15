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
/// Rejects SSZ content-type and parses JSON from the request body.
pub async fn json_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<T, ApiError> {
    check_not_ssz(headers)?;
    serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("body deserialize error: {e:?}")))
}

/// Extract and validate a JSON body that may be empty (returns T::default() for empty bodies).
/// Returns `T::default()` for empty bodies, otherwise parses JSON.
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn accept_header_none_when_missing() {
        let headers = HeaderMap::new();
        assert!(accept_header(&headers).is_none());
    }

    #[test]
    fn accept_header_json() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "application/json".parse().unwrap());
        let accept = accept_header(&headers);
        assert_eq!(accept, Some(api_types::Accept::Json));
    }

    #[test]
    fn accept_header_ssz() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "application/octet-stream".parse().unwrap());
        let accept = accept_header(&headers);
        assert_eq!(accept, Some(api_types::Accept::Ssz));
    }

    #[test]
    fn accept_header_unknown_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "text/html".parse().unwrap());
        assert!(accept_header(&headers).is_none());
    }

    #[test]
    fn consensus_version_header_missing() {
        let headers = HeaderMap::new();
        let result = consensus_version_header(&headers);
        assert!(result.is_err());
    }

    #[test]
    fn consensus_version_header_valid() {
        let mut headers = HeaderMap::new();
        headers.insert(eth2::CONSENSUS_VERSION_HEADER, "deneb".parse().unwrap());
        let fork = consensus_version_header(&headers).unwrap();
        assert_eq!(fork, ForkName::Deneb);
    }

    #[test]
    fn consensus_version_header_unknown_fork() {
        let mut headers = HeaderMap::new();
        headers.insert(
            eth2::CONSENSUS_VERSION_HEADER,
            "unknownfork".parse().unwrap(),
        );
        assert!(consensus_version_header(&headers).is_err());
    }

    #[test]
    fn optional_consensus_version_header_missing() {
        let headers = HeaderMap::new();
        assert!(optional_consensus_version_header(&headers).is_none());
    }

    #[test]
    fn optional_consensus_version_header_valid() {
        let mut headers = HeaderMap::new();
        headers.insert(eth2::CONSENSUS_VERSION_HEADER, "capella".parse().unwrap());
        assert_eq!(
            optional_consensus_version_header(&headers),
            Some(ForkName::Capella)
        );
    }

    #[test]
    fn optional_consensus_version_header_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(eth2::CONSENSUS_VERSION_HEADER, "garbage".parse().unwrap());
        assert!(optional_consensus_version_header(&headers).is_none());
    }

    #[tokio::test]
    async fn json_body_valid() {
        let headers = HeaderMap::new();
        let body = axum::body::Bytes::from(r#"{"value": 42}"#);
        let result: Result<serde_json::Value, _> = json_body(&headers, body).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["value"], 42);
    }

    #[tokio::test]
    async fn json_body_rejects_ssz_content_type() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE_HEADER,
            SSZ_CONTENT_TYPE_HEADER.parse().unwrap(),
        );
        let body = axum::body::Bytes::from("{}");
        let result: Result<serde_json::Value, _> = json_body(&headers, body).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn json_body_invalid_json() {
        let headers = HeaderMap::new();
        let body = axum::body::Bytes::from("not json");
        let result: Result<serde_json::Value, _> = json_body(&headers, body).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn json_body_or_default_empty() {
        let headers = HeaderMap::new();
        let body = axum::body::Bytes::new();
        let result: Result<serde_json::Value, _> = json_body_or_default(&headers, body).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn json_body_or_default_with_data() {
        let headers = HeaderMap::new();
        let body = axum::body::Bytes::from(r#"{"key": "val"}"#);
        let result: Result<serde_json::Value, _> = json_body_or_default(&headers, body).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["key"], "val");
    }

    #[tokio::test]
    async fn json_body_or_default_rejects_ssz() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE_HEADER,
            SSZ_CONTENT_TYPE_HEADER.parse().unwrap(),
        );
        let body = axum::body::Bytes::new();
        let result: Result<serde_json::Value, _> = json_body_or_default(&headers, body).await;
        assert!(result.is_err());
    }

    #[test]
    fn parse_endpoint_version_v1() {
        let v = parse_endpoint_version("v1").unwrap();
        assert_eq!(v, api_types::EndpointVersion(1));
    }

    #[test]
    fn parse_endpoint_version_v2() {
        let v = parse_endpoint_version("v2").unwrap();
        assert_eq!(v, api_types::EndpointVersion(2));
    }

    #[test]
    fn parse_endpoint_version_invalid() {
        assert!(parse_endpoint_version("xyz").is_err());
    }

    #[test]
    fn parse_endpoint_version_empty() {
        assert!(parse_endpoint_version("").is_err());
    }
}
