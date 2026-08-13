use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::service::register_user;
use crate::auth::start_session;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct RegisterRequest {
    name: String,
    email: String,
    password: String,
}

#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "User registered and logged in", body = UserResponse),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<UserResponse>), HttpError> {
    let saved = register_user(&state.db, req.name, req.email, req.password).await?;
    start_session(&session, &saved).await?;
    Ok((StatusCode::CREATED, Json(saved.into())))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::features::test_support::{app_router, json_body, request, test_db};

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    #[tokio::test]
    async fn register_returns_201_and_sets_session_cookie() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "name": "Ada",
            "email": "ada@example.com",
            "password": "hunter2hunter2",
        }));

        let res = request(app, Method::POST, "/auth/register", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::CREATED);
        let has_session_cookie = res
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|c| c.to_str().is_ok_and(|v| v.starts_with("id=")));
        let created: UserResponse = json_body(res).await?;
        assert_eq!(created.name, "Ada");
        assert!(created.id > 0);
        assert!(
            has_session_cookie,
            "register must hand out a session cookie"
        );
        Ok(())
    }

    #[tokio::test]
    async fn register_response_never_exposes_password_hash() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "name": "Ada",
            "email": "ada@example.com",
            "password": "hunter2hunter2",
        }));

        let res = request(app, Method::POST, "/auth/register", Some(body), None).await?;
        let json: serde_json::Value = json_body(res).await?;
        assert!(json.get("password_hash").is_none());
        assert!(json.get("password").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn register_duplicate_email_is_conflict() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "name": "Ada",
            "email": "ada@example.com",
            "password": "hunter2hunter2",
        }));

        let res = request(
            app.clone(),
            Method::POST,
            "/auth/register",
            Some(body),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::CREATED);
        let res = request(
            app,
            Method::POST,
            "/auth/register",
            Some(req_body(&serde_json::json!({
                "name": "Ada",
                "email": "ada@example.com",
                "password": "hunter2hunter2",
            }))),
            None,
        )
        .await?;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        Ok(())
    }

    #[tokio::test]
    async fn register_short_password_is_bad_request() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "name": "Ada",
            "email": "ada@example.com",
            "password": "short",
        }));

        let res = request(app, Method::POST, "/auth/register", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }
}
