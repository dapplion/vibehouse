use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use eth2::types::{ErrorMessage, Failure, IndexedErrorMessage};
use std::fmt;

/// Unified error type for the beacon node HTTP API.
///
/// Replaces the collection of warp rejection types previously scattered across
/// `warp_utils::reject`. Each variant maps to a specific HTTP status code.
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

// Bridge: allow ApiError to be used as a warp rejection during the migration.
// Once lib.rs is fully migrated to axum, these impls can be removed.
impl warp::reject::Reject for ApiError {}

// Note: warp already provides `impl<T: Reject> From<T> for Rejection`, so
// `ApiError` (which impls Reject) auto-converts via `?` in warp closures.
// No manual From impl needed.

// Bridge: allow warp::Rejection to convert into ApiError during migration.
// This lets closures returning Result<_, ApiError> use `?` on warp::Rejection values
// (e.g. from query parsing, deserialization filters still using warp types).
impl From<warp::Rejection> for ApiError {
    fn from(rejection: warp::Rejection) -> Self {
        use warp_utils::reject;
        if rejection.is_not_found() {
            ApiError::NotFound("NOT_FOUND".into())
        } else if let Some(e) = rejection.find::<reject::CustomBadRequest>() {
            ApiError::BadRequest(e.0.clone())
        } else if let Some(e) = rejection.find::<reject::CustomNotFound>() {
            ApiError::NotFound(e.0.clone())
        } else if let Some(e) = rejection.find::<reject::CustomServerError>() {
            ApiError::ServerError(e.0.clone())
        } else if let Some(e) = rejection.find::<reject::CustomDeserializeError>() {
            ApiError::BadRequest(format!("body deserialize error: {}", e.0))
        } else if let Some(e) = rejection.find::<reject::NotSynced>() {
            ApiError::ServiceUnavailable(format!("beacon node is syncing: {}", e.0))
        } else if let Some(e) = rejection.find::<reject::InvalidAuthorization>() {
            ApiError::Forbidden(format!("Invalid auth token: {}", e.0))
        } else if let Some(e) = rejection.find::<reject::BroadcastWithoutImport>() {
            ApiError::BroadcastWithoutImport(e.0.clone())
        } else if let Some(e) = rejection.find::<reject::ObjectInvalid>() {
            ApiError::BadRequest(format!("Invalid object: {}", e.0))
        } else if let Some(e) = rejection.find::<reject::UnsupportedMediaType>() {
            ApiError::UnsupportedMediaType(e.0.clone())
        } else if let Some(e) = rejection.find::<reject::IndexedBadRequestErrors>() {
            ApiError::IndexedBadRequest {
                message: e.message.clone(),
                failures: e.failures.clone(),
            }
        } else if let Some(e) = rejection.find::<reject::BeaconStateError>() {
            ApiError::ServerError(format!("{:?}", e.0))
        } else if let Some(e) = rejection.find::<reject::ArithError>() {
            ApiError::ServerError(format!("{:?}", e.0))
        } else if let Some(e) = rejection.find::<reject::UnhandledError>() {
            ApiError::ServerError(format!("{:?}", e.0))
        } else if let Some(e) = rejection.find::<warp::filters::body::BodyDeserializeError>() {
            ApiError::BadRequest(format!("body deserialize error: {e}"))
        } else if let Some(e) = rejection.find::<warp::reject::InvalidQuery>() {
            ApiError::BadRequest(format!("invalid query: {e}"))
        } else if let Some(e) = rejection.find::<warp::reject::MissingHeader>() {
            if e.name() == "Authorization" {
                ApiError::Unauthorized("missing Authorization header".into())
            } else {
                ApiError::BadRequest(format!("missing {} header", e.name()))
            }
        } else if let Some(e) = rejection.find::<warp::reject::InvalidHeader>() {
            ApiError::BadRequest(format!("invalid {} header", e.name()))
        } else if rejection.find::<warp::reject::MethodNotAllowed>().is_some() {
            ApiError::BadRequest("METHOD_NOT_ALLOWED".into())
        } else {
            ApiError::ServerError(format!("{rejection:?}"))
        }
    }
}

/// Warp rejection handler that handles both `ApiError` and legacy warp_utils rejections.
///
/// This is used as the `.recover()` handler in lib.rs. It checks for legacy warp_utils
/// rejection types first (since they may coexist with ApiError in combined rejections
/// from `.or()` chains and should take priority), then checks for `ApiError`.
pub async fn handle_rejection(
    err: warp::Rejection,
) -> Result<warp::reply::Response, std::convert::Infallible> {
    use warp::http::StatusCode;
    use warp::reply::Reply;
    use warp_utils::reject;

    // Check legacy warp_utils rejection types first.
    // When `.or()` combines rejections from multiple routes, a legacy rejection
    // (e.g. UnsupportedMediaType from json_no_body) might coexist with an ApiError
    // from a different route. Legacy types must be checked first to preserve correct
    // HTTP status codes.
    if let Some(e) = err.find::<reject::UnsupportedMediaType>() {
        let code = StatusCode::UNSUPPORTED_MEDIA_TYPE;
        let json = warp::reply::json(&ErrorMessage {
            code: code.as_u16(),
            message: format!("UNSUPPORTED_MEDIA_TYPE: {}", e.0),
            stacktraces: vec![],
        });
        return Ok(warp::reply::with_status(json, code).into_response());
    }

    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message, indexed) = match api_error {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, format!("NOT_FOUND: {msg}"), None),
            ApiError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, format!("BAD_REQUEST: {msg}"), None)
            }
            ApiError::ServerError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("INTERNAL_SERVER_ERROR: {msg}"),
                None,
            ),
            ApiError::ServiceUnavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("SERVICE_UNAVAILABLE: {msg}"),
                None,
            ),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, format!("FORBIDDEN: {msg}"), None),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone(), None),
            ApiError::UnsupportedMediaType(msg) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("UNSUPPORTED_MEDIA_TYPE: {msg}"),
                None,
            ),
            ApiError::BroadcastWithoutImport(msg) => (
                StatusCode::ACCEPTED,
                format!(
                    "ACCEPTED: the object was broadcast to the network without being \
                     fully imported to the local database: {msg}"
                ),
                None,
            ),
            ApiError::IndexedBadRequest { message, failures } => (
                StatusCode::BAD_REQUEST,
                format!("BAD_REQUEST: {message}"),
                Some(failures.clone()),
            ),
        };

        if let Some(failures) = indexed {
            let json = warp::reply::json(&IndexedErrorMessage {
                code: code.as_u16(),
                message,
                failures,
            });
            return Ok(warp::reply::with_status(json, code).into_response());
        }

        let json = warp::reply::json(&ErrorMessage {
            code: code.as_u16(),
            message,
            stacktraces: vec![],
        });
        return Ok(warp::reply::with_status(json, code).into_response());
    }

    // Delegate to the standard warp_utils handler for legacy rejection types.
    let Ok(reply) = warp_utils::reject::handle_rejection(err).await;
    Ok(reply.into_response())
}

/// Convert a `Result<T, ApiError>` into a warp `Response`.
///
/// Used in warp `.then()` closures where the handler returns `Result<T, ApiError>`
/// but warp needs a concrete `Response`.
pub fn convert_api_error<T: warp::Reply>(res: Result<T, ApiError>) -> warp::reply::Response {
    match res {
        Ok(response) => response.into_response(),
        Err(e) => e.into_warp_response(),
    }
}

impl ApiError {
    /// Convert this error into a warp Response (for use during the migration period
    /// where lib.rs still uses warp routing but handler modules return ApiError).
    pub fn into_warp_response(self) -> warp::reply::Response {
        use warp::http::StatusCode;
        use warp::reply::Reply;

        match self {
            ApiError::IndexedBadRequest { message, failures } => {
                let code = StatusCode::BAD_REQUEST;
                let body = IndexedErrorMessage {
                    code: code.as_u16(),
                    message: format!("BAD_REQUEST: {message}"),
                    failures,
                };
                warp::reply::with_status(warp::reply::json(&body), code).into_response()
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
                    ApiError::BroadcastWithoutImport(msg) => (
                        StatusCode::ACCEPTED,
                        format!(
                            "ACCEPTED: the object was broadcast to the network without being \
                             fully imported to the local database: {msg}"
                        ),
                    ),
                    ApiError::IndexedBadRequest { .. } => unreachable!(),
                };

                let body = ErrorMessage {
                    code: code.as_u16(),
                    message,
                    stacktraces: vec![],
                };
                warp::reply::with_status(warp::reply::json(&body), code).into_response()
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
