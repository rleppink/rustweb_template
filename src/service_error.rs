use sea_orm::{DbErr, SqlErr};
use thiserror::Error;

/// Domain error returned by the service (business logic) layer.
///
/// Knows nothing about HTTP; handlers map these to [`crate::error::HttpError`]
/// at the slice boundary.
#[derive(Debug, Error)]
pub(crate) enum ServiceError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("conflict")]
    Conflict,
    #[error("internal error")]
    Internal,
    #[error("{0}")]
    Validation(String),
    #[error(transparent)]
    Db(#[from] DbErr),
}

impl ServiceError {
    /// Maps a database error, surfacing unique-constraint violations as
    /// [`ServiceError::Conflict`] rather than a generic [`ServiceError::Db`].
    pub(crate) fn from_db_err(err: DbErr) -> Self {
        if let Some(SqlErr::UniqueConstraintViolation(_)) = err.sql_err() {
            Self::Conflict
        } else {
            Self::Db(err)
        }
    }
}
