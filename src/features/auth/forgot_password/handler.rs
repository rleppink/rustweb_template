use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::service::request_password_reset;
use crate::error::{ErrorResponses, HttpError};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct RequestResetRequest {
    email: String,
}

/// Always answers 202 for well-formed requests — including unknown emails —
/// so the endpoint cannot be used to enumerate accounts. The response body is
/// empty; the token travels by email only.
#[utoipa::path(
    post,
    path = "/auth/password-reset/request",
    tag = "auth",
    request_body = RequestResetRequest,
    responses(
        (status = 202, description = "Reset email accepted — sent if the email exists"),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    Json(req): Json<RequestResetRequest>,
) -> Result<StatusCode, HttpError> {
    request_password_reset(&state.db, state.mailer.as_ref(), req.email, &state.config).await?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use crate::auth::hash_password_reset_token;
    use crate::entities::password_reset_token;
    use crate::entities::{prelude::User, user};
    use crate::features::test_support::{
        TestMailer, app_router_with_mailer, register_user_direct, request, test_db,
    };

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    #[tokio::test]
    async fn known_email_returns_202_and_emails_a_single_use_token() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db.clone(), store, mailer.clone());

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(&serde_json::json!({ "email": "ada@example.com" }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let sent = mailer.sent();
        assert_eq!(sent.len(), 1, "exactly one reset email");
        let mail = sent
            .first()
            .ok_or_else(|| std::io::Error::other("no email recorded"))?;
        assert_eq!(mail.to, "ada@example.com");
        let raw_token = mail.token.clone();

        let stored = password_reset_token::Entity::find()
            .one(&db)
            .await?
            .ok_or_else(|| std::io::Error::other("no token row stored"))?;
        let ada = User::find()
            .filter(user::Column::Email.eq("ada@example.com"))
            .one(&db)
            .await?
            .ok_or_else(|| std::io::Error::other("registered user not found"))?;
        assert_eq!(stored.user_id, ada.id);
        assert_ne!(stored.token_hash, raw_token, "raw token must not be stored");
        assert_eq!(
            stored.token_hash,
            hash_password_reset_token(&raw_token),
            "stored value must be the sha256 of the emailed token"
        );
        assert!(
            stored.expires_at > chrono::Utc::now(),
            "token must not expire immediately"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unknown_email_returns_202_and_sends_nothing() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db.clone(), store, mailer.clone());

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(
                &serde_json::json!({ "email": "nobody@example.com" }),
            )),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        assert!(mailer.sent().is_empty(), "no email for unknown accounts");
        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert!(rows.is_empty(), "no token rows for unknown accounts");
        Ok(())
    }

    #[tokio::test]
    async fn new_request_invalidates_the_previous_token() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db.clone(), store, mailer.clone());

        for _ in 0..2 {
            let res = request(
                app.clone(),
                Method::POST,
                "/auth/password-reset/request",
                Some(req_body(&serde_json::json!({ "email": "ada@example.com" }))),
                None,
            )
            .await?;
            assert_eq!(res.status(), StatusCode::ACCEPTED);
        }

        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert_eq!(rows.len(), 1, "only the newest token may be outstanding");
        let sent = mailer.sent();
        assert_eq!(sent.len(), 2);
        let last_token = sent
            .last()
            .ok_or_else(|| std::io::Error::other("no email recorded"))?
            .token
            .clone();
        let stored_hash = rows
            .first()
            .ok_or_else(|| std::io::Error::other("no token row"))?
            .token_hash
            .clone();
        assert_eq!(
            stored_hash,
            hash_password_reset_token(&last_token),
            "the stored token must be the one from the last request"
        );
        Ok(())
    }

    #[tokio::test]
    async fn malformed_email_is_bad_request() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db, store, mailer.clone());

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(&serde_json::json!({ "email": "not-an-email" }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert!(mailer.sent().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn response_is_identical_for_known_and_unknown_emails() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db, store, mailer);

        let known = request(
            app.clone(),
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(&serde_json::json!({ "email": "ada@example.com" }))),
            None,
        )
        .await?;
        let unknown = request(
            app,
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(
                &serde_json::json!({ "email": "nobody@example.com" }),
            )),
            None,
        )
        .await?;
        assert_eq!(
            known.status(),
            unknown.status(),
            "the endpoint must not leak whether the email exists"
        );
        assert_eq!(known.status(), StatusCode::ACCEPTED);
        Ok(())
    }
}
