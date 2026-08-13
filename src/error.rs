use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::service_error::ServiceError;

/// HTTP-facing error produced by mapping a [`ServiceError`] at the slice
/// boundary. Only handlers ever construct or return this.
#[derive(Debug, Error)]
pub(crate) enum HttpError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    BadRequest(String),
    #[error("conflict")]
    Conflict,
    #[error("internal server error")]
    InternalServerError,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub(crate) struct ErrorBody {
    pub(crate) error: String,
}

#[derive(utoipa::IntoResponses)]
#[allow(dead_code)]
pub(crate) enum ErrorResponses {
    #[response(status = 400, description = "Validation failed")]
    Validation(ErrorBody),
    #[response(status = 401, description = "Not authenticated")]
    Unauthorized(ErrorBody),
    #[response(status = 403, description = "Not authorized for this resource")]
    Forbidden(ErrorBody),
    #[response(status = 404, description = "Not found")]
    NotFound(ErrorBody),
    #[response(status = 409, description = "Conflict with existing data")]
    Conflict(ErrorBody),
    #[response(status = 500, description = "Internal server error")]
    Db(ErrorBody),
}

impl From<ServiceError> for HttpError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::NotFound => HttpError::NotFound,
            ServiceError::Unauthorized => HttpError::Unauthorized,
            ServiceError::Conflict => HttpError::Conflict,
            ServiceError::Internal => HttpError::InternalServerError,
            ServiceError::Validation(msg) => HttpError::BadRequest(msg),
            ServiceError::Db(e) => {
                tracing::error!(error = %e, "database error");
                HttpError::InternalServerError
            }
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            HttpError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            HttpError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            HttpError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            HttpError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            HttpError::Conflict => (StatusCode::CONFLICT, "conflict".to_string()),
            HttpError::InternalServerError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
