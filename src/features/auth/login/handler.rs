use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use tower_sessions::Session;
use utoipa::ToSchema;

use super::service::login_user;
use crate::auth::start_session;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct LoginRequest {
    email: String,
    password: String,
}

#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Logged in", body = UserResponse),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    session: Session,
    Json(req): Json<LoginRequest>,
) -> Result<Json<UserResponse>, HttpError> {
    let user = login_user(&state.db, req.email, req.password).await?;
    start_session(&session, &user).await?;
    Ok(Json(user.into()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::features::test_support::{
        app_router, json_body, register_user_direct, request, test_db,
    };

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    #[tokio::test]
    async fn login_sets_session_cookie() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "email": "ada@example.com",
            "password": "hunter2hunter2",
        }));

        let res = request(app, Method::POST, "/auth/login", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::OK);
        let has_session_cookie = res
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|c| c.to_str().is_ok_and(|v| v.starts_with("id=")));
        let logged_in: UserResponse = json_body(res).await?;
        assert_eq!(logged_in.email, "ada@example.com");
        assert!(has_session_cookie, "login must hand out a session cookie");
        Ok(())
    }

    #[tokio::test]
    async fn login_with_wrong_password_is_unauthorized() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "email": "ada@example.com",
            "password": "wrong-password",
        }));

        let res = request(app, Method::POST, "/auth/login", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
