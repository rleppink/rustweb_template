use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::service::reset_password;
use crate::error::{ErrorResponses, HttpError};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct ResetPasswordRequest {
    token: String,
    password: String,
}

#[utoipa::path(
    post,
    path = "/auth/password-reset/confirm",
    tag = "auth",
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "Password changed; the token is single-use"),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<StatusCode, HttpError> {
    reset_password(&state.db, req.token, req.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};
    use sea_orm::DatabaseConnection;
    use tower_sessions_sqlx_store::PostgresStore;

    use crate::features::test_support::{
        TestMailer, app_router_with_mailer, insert_user, json_body, register_user_direct, request,
        test_db,
    };

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    /// Register a user through the business layer, request a reset through the
    /// HTTP layer, and return the raw token from the mailer.
    async fn reset_token_for(
        db: &DatabaseConnection,
        store: &PostgresStore,
        mailer: &Arc<TestMailer>,
    ) -> Result<String, Box<dyn Error>> {
        let app = app_router_with_mailer(db.clone(), store.clone(), mailer.clone());
        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/request",
            Some(req_body(&serde_json::json!({ "email": "ada@example.com" }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        Ok(mailer
            .sent()
            .first()
            .ok_or_else(|| std::io::Error::other("no email recorded"))?
            .token
            .clone())
    }

    #[tokio::test]
    async fn confirm_returns_204_and_the_new_password_logs_in() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let token = reset_token_for(&db, &store, &mailer).await?;
        let app = app_router_with_mailer(db.clone(), store.clone(), mailer.clone());

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/confirm",
            Some(req_body(&serde_json::json!({
                "token": token,
                "password": "newpassword123",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // The new password must log in through the full HTTP layer.
        let login_app = app_router_with_mailer(db, store, mailer);
        let res = request(
            login_app,
            Method::POST,
            "/auth/login",
            Some(req_body(&serde_json::json!({
                "email": "ada@example.com",
                "password": "newpassword123",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::OK, "new password must log in");
        let _body: serde_json::Value = json_body(res).await?;
        Ok(())
    }

    #[tokio::test]
    async fn confirm_with_invalid_token_is_bad_request() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let app = app_router_with_mailer(db, store, mailer);

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/confirm",
            Some(req_body(&serde_json::json!({
                "token": "f".repeat(64),
                "password": "newpassword123",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = json_body(res).await?;
        assert!(
            body.get("error")
                .is_some_and(|e| e.as_str().is_some_and(|m| m.contains("token"))),
            "error must mention the token: {body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn confirm_with_short_password_is_bad_request() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = Arc::new(TestMailer::new());
        let token = reset_token_for(&db, &store, &mailer).await?;
        let app = app_router_with_mailer(db, store, mailer);

        let res = request(
            app,
            Method::POST,
            "/auth/password-reset/confirm",
            Some(req_body(&serde_json::json!({
                "token": token,
                "password": "short",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_user_without_hash_can_reset_and_login() -> Result<(), Box<dyn Error>> {
        // Users that predate the auth migration have an empty password_hash
        // and can never log in; password reset is the recovery path.
        let (db, store, _guard) = test_db().await?;
        insert_user(&db, "Ada", "ada@example.com").await?;
        let mailer = Arc::new(TestMailer::new());
        let token = reset_token_for(&db, &store, &mailer).await?;
        let app = app_router_with_mailer(db.clone(), store, mailer);

        let res = request(
            app.clone(),
            Method::POST,
            "/auth/password-reset/confirm",
            Some(req_body(&serde_json::json!({
                "token": token,
                "password": "newpassword123",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        let res = request(
            app,
            Method::POST,
            "/auth/login",
            Some(req_body(&serde_json::json!({
                "email": "ada@example.com",
                "password": "newpassword123",
            }))),
            None,
        )
        .await?;
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "legacy user must log in after a reset"
        );
        Ok(())
    }
}
