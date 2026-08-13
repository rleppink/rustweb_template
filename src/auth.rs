use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sea_orm::EntityTrait;
use sha2::{Digest, Sha256};
use tower_sessions::Session;

use crate::entities::user;
use crate::error::HttpError;
use crate::state::AppState;

/// Session key holding the id of the authenticated user.
pub(crate) const SESSION_USER_ID: &str = "user_id";

/// The authenticated user, loaded from the session cookie on every request.
///
/// Handlers that take this extractor reject with `401 Unauthorized` when no
/// valid session is present. Authorization beyond authentication (ownership
/// checks, roles) is done by handlers and services, not here.
#[derive(Clone)]
pub(crate) struct CurrentUser(pub(crate) user::Model);

/// `Debug` prints only the id: the model inside carries `password_hash`, which
/// must never reach a log line.
impl std::fmt::Debug for CurrentUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentUser")
            .field("id", &self.id())
            .finish()
    }
}

impl CurrentUser {
    pub(crate) fn id(&self) -> i32 {
        self.0.id
    }
}

/// Records `user` as the session's authenticated subject, rotating the
/// session id to prevent fixation.
pub(crate) async fn start_session(session: &Session, user: &user::Model) -> Result<(), HttpError> {
    session
        .cycle_id()
        .await
        .map_err(|err| session_io_failure(&err))?;
    session
        .insert(SESSION_USER_ID, user.id)
        .await
        .map_err(|err| session_io_failure(&err))
}

/// A fresh password-reset token and its sha256 digest, as a `(raw, hash)` pair.
///
/// The raw token is only ever handed to the mailer; the hash is the only form
/// the database stores, so a leaked `password_reset_tokens` table yields no
/// usable tokens. 32 random bytes encode to 64 hex chars.
pub(crate) fn new_password_reset_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let hash = hash_password_reset_token(&token);
    (token, hash)
}

/// The digest of `token` as stored in `password_reset_tokens.token_hash`.
pub(crate) fn hash_password_reset_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn session_io_failure(err: &tower_sessions::session::Error) -> HttpError {
    tracing::error!(error = %err, "session operation failed");
    HttpError::InternalServerError
}

fn session_extraction_failure((_status, msg): (axum::http::StatusCode, &'static str)) -> HttpError {
    tracing::error!(error = msg, "session extraction failed");
    HttpError::InternalServerError
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(session_extraction_failure)?;

        let Some(user_id) = session
            .get::<i32>(SESSION_USER_ID)
            .await
            .map_err(|err| session_io_failure(&err))?
        else {
            return Err(HttpError::Unauthorized);
        };

        let Some(user) = user::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .map_err(|err| {
                tracing::error!(error = %err, "failed to load session user");
                HttpError::InternalServerError
            })?
        else {
            return Err(HttpError::Unauthorized);
        };

        Ok(Self(user))
    }
}
