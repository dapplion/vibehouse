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

/// Convenience constructors matching the old warp_utils::reject API.
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
