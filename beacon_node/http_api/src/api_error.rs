use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use eth2::types::{ErrorMessage, Failure, IndexedErrorMessage};
use std::fmt;

/// Unified error type for the beacon node HTTP API.
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    ServerError(String),
    ServiceUnavailable(String),
    Forbidden(String),
    Unauthorized(String),
    UnsupportedMediaType(String),
    /// 202 ACCEPTED — object was broadcast but not fully imported.
    BroadcastWithoutImport(String),
    /// 400 with indexed error body (batch operations).
    IndexedBadRequest {
        message: String,
        failures: Vec<Failure>,
    },
}

impl fmt::Debug for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotFound(msg) => write!(f, "NotFound({msg})"),
            ApiError::BadRequest(msg) => write!(f, "BadRequest({msg})"),
            ApiError::ServerError(msg) => write!(f, "ServerError({msg})"),
            ApiError::ServiceUnavailable(msg) => write!(f, "ServiceUnavailable({msg})"),
            ApiError::Forbidden(msg) => write!(f, "Forbidden({msg})"),
            ApiError::Unauthorized(msg) => write!(f, "Unauthorized({msg})"),
            ApiError::UnsupportedMediaType(msg) => write!(f, "UnsupportedMediaType({msg})"),
            ApiError::BroadcastWithoutImport(msg) => write!(f, "BroadcastWithoutImport({msg})"),
            ApiError::IndexedBadRequest { message, .. } => {
                write!(f, "IndexedBadRequest({message})")
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::IndexedBadRequest { message, failures } => {
                let code = StatusCode::BAD_REQUEST;
                let body = IndexedErrorMessage {
                    code: code.as_u16(),
                    message: format!("BAD_REQUEST: {message}"),
                    failures,
                };
                (code, axum::Json(body)).into_response()
            }
            ApiError::BroadcastWithoutImport(msg) => {
                let code = StatusCode::ACCEPTED;
                let body = ErrorMessage {
                    code: code.as_u16(),
                    message: format!(
                        "ACCEPTED: the object was broadcast to the network without being \
                         fully imported to the local database: {msg}"
                    ),
                    stacktraces: vec![],
                };
                (code, axum::Json(body)).into_response()
            }
            other => {
                let (code, message) = match other {
                    ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, format!("NOT_FOUND: {msg}")),
                    ApiError::BadRequest(msg) => {
                        (StatusCode::BAD_REQUEST, format!("BAD_REQUEST: {msg}"))
                    }
                    ApiError::ServerError(msg) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("INTERNAL_SERVER_ERROR: {msg}"),
                    ),
                    ApiError::ServiceUnavailable(msg) => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("SERVICE_UNAVAILABLE: {msg}"),
                    ),
                    ApiError::Forbidden(msg) => {
                        (StatusCode::FORBIDDEN, format!("FORBIDDEN: {msg}"))
                    }
                    ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
                    ApiError::UnsupportedMediaType(msg) => (
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        format!("UNSUPPORTED_MEDIA_TYPE: {msg}"),
                    ),
                    // Already handled above.
                    ApiError::BroadcastWithoutImport(_) | ApiError::IndexedBadRequest { .. } => {
                        unreachable!()
                    }
                };

                let body = ErrorMessage {
                    code: code.as_u16(),
                    message,
                    stacktraces: vec![],
                };
                (code, axum::Json(body)).into_response()
            }
        }
    }
}

/// Convenience constructors for common HTTP error responses.
impl ApiError {
    pub fn not_found(msg: impl Into<String>) -> Self {
        ApiError::NotFound(msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        ApiError::BadRequest(msg.into())
    }

    pub fn server_error(msg: impl Into<String>) -> Self {
        ApiError::ServerError(msg.into())
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        ApiError::ServiceUnavailable(msg.into())
    }

    pub fn unsupported_media_type(msg: impl Into<String>) -> Self {
        ApiError::UnsupportedMediaType(msg.into())
    }

    pub fn object_invalid(msg: impl Into<String>) -> Self {
        ApiError::BadRequest(format!("Invalid object: {}", msg.into()))
    }

    pub fn beacon_state_error(e: types::BeaconStateError) -> Self {
        ApiError::ServerError(format!("{e:?}"))
    }

    pub fn arith_error(e: safe_arith::ArithError) -> Self {
        ApiError::ServerError(format!("{e:?}"))
    }

    pub fn unhandled_error(e: impl fmt::Debug) -> Self {
        ApiError::ServerError(format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;

    async fn response_status_and_body(error: ApiError) -> (StatusCode, String) {
        let resp = error.into_response();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[test]
    fn debug_not_found() {
        let e = ApiError::not_found("missing item");
        assert_eq!(format!("{e:?}"), "NotFound(missing item)");
    }

    #[test]
    fn debug_bad_request() {
        let e = ApiError::bad_request("bad input");
        assert_eq!(format!("{e:?}"), "BadRequest(bad input)");
    }

    #[test]
    fn debug_server_error() {
        let e = ApiError::server_error("internal");
        assert_eq!(format!("{e:?}"), "ServerError(internal)");
    }

    #[test]
    fn debug_service_unavailable() {
        let e = ApiError::service_unavailable("down");
        assert_eq!(format!("{e:?}"), "ServiceUnavailable(down)");
    }

    #[test]
    fn debug_unsupported_media_type() {
        let e = ApiError::unsupported_media_type("wrong ct");
        assert_eq!(format!("{e:?}"), "UnsupportedMediaType(wrong ct)");
    }

    #[test]
    fn debug_broadcast_without_import() {
        let e = ApiError::BroadcastWithoutImport("partial".to_string());
        assert_eq!(format!("{e:?}"), "BroadcastWithoutImport(partial)");
    }

    #[test]
    fn debug_indexed_bad_request() {
        let e = ApiError::IndexedBadRequest {
            message: "batch fail".to_string(),
            failures: vec![],
        };
        assert_eq!(format!("{e:?}"), "IndexedBadRequest(batch fail)");
    }

    #[test]
    fn debug_forbidden() {
        let e = ApiError::Forbidden("denied".to_string());
        assert_eq!(format!("{e:?}"), "Forbidden(denied)");
    }

    #[test]
    fn debug_unauthorized() {
        let e = ApiError::Unauthorized("no auth".to_string());
        assert_eq!(format!("{e:?}"), "Unauthorized(no auth)");
    }

    #[test]
    fn constructor_object_invalid() {
        let e = ApiError::object_invalid("bad data");
        assert_eq!(format!("{e:?}"), "BadRequest(Invalid object: bad data)");
    }

    #[test]
    fn constructor_beacon_state_error() {
        let e = ApiError::beacon_state_error(types::BeaconStateError::InsufficientValidators);
        let dbg = format!("{e:?}");
        assert!(dbg.starts_with("ServerError("));
        assert!(dbg.contains("InsufficientValidators"));
    }

    #[test]
    fn constructor_arith_error() {
        let e = ApiError::arith_error(safe_arith::ArithError::Overflow);
        let dbg = format!("{e:?}");
        assert!(dbg.starts_with("ServerError("));
        assert!(dbg.contains("Overflow"));
    }

    #[test]
    fn constructor_unhandled_error() {
        let e = ApiError::unhandled_error("something broke");
        let dbg = format!("{e:?}");
        assert!(dbg.starts_with("ServerError("));
    }

    #[tokio::test]
    async fn into_response_not_found() {
        let (status, body) = response_status_and_body(ApiError::not_found("gone")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("NOT_FOUND: gone"));
    }

    #[tokio::test]
    async fn into_response_bad_request() {
        let (status, body) = response_status_and_body(ApiError::bad_request("invalid")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("BAD_REQUEST: invalid"));
    }

    #[tokio::test]
    async fn into_response_server_error() {
        let (status, body) = response_status_and_body(ApiError::server_error("oops")).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("INTERNAL_SERVER_ERROR: oops"));
    }

    #[tokio::test]
    async fn into_response_service_unavailable() {
        let (status, body) =
            response_status_and_body(ApiError::service_unavailable("maintenance")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("SERVICE_UNAVAILABLE: maintenance"));
    }

    #[tokio::test]
    async fn into_response_forbidden() {
        let (status, body) =
            response_status_and_body(ApiError::Forbidden("nope".to_string())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("FORBIDDEN: nope"));
    }

    #[tokio::test]
    async fn into_response_unauthorized() {
        let (status, body) =
            response_status_and_body(ApiError::Unauthorized("bad token".to_string())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("bad token"));
    }

    #[tokio::test]
    async fn into_response_unsupported_media_type() {
        let (status, body) =
            response_status_and_body(ApiError::unsupported_media_type("not json")).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(body.contains("UNSUPPORTED_MEDIA_TYPE: not json"));
    }

    #[tokio::test]
    async fn into_response_broadcast_without_import() {
        let (status, body) =
            response_status_and_body(ApiError::BroadcastWithoutImport("sent".to_string())).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.contains("ACCEPTED"));
        assert!(body.contains("sent"));
    }

    #[tokio::test]
    async fn into_response_indexed_bad_request() {
        let failures = vec![Failure {
            index: 0,
            message: "bad item".to_string(),
        }];
        let (status, body) = response_status_and_body(ApiError::IndexedBadRequest {
            message: "batch".to_string(),
            failures,
        })
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("BAD_REQUEST: batch"));
        assert!(body.contains("bad item"));
    }
}
