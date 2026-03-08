use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use eth2::types::ErrorMessage;

/// Unified error type for the VC HTTP API, replacing warp rejections.
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    ServerError(String),
    Forbidden(String),
    Unauthorized(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, format!("NOT_FOUND: {}", msg)),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, format!("BAD_REQUEST: {}", msg)),
            ApiError::ServerError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("INTERNAL_SERVER_ERROR: {}", msg),
            ),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, format!("FORBIDDEN: {}", msg)),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
        };

        let body = ErrorMessage {
            code: code.as_u16(),
            message,
            stacktraces: vec![],
        };

        (code, axum::Json(body)).into_response()
    }
}
