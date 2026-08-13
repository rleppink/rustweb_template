use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use utoipa::ToSchema;

use super::service::update_user;
use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct UpdateUserRequest {
    name: String,
    email: String,
}

#[utoipa::path(
    put,
    path = "/users/{id}",
    tag = "users",
    security(("session_cookie" = [])),
    params(("id" = i32, Path, description = "User id")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The updated user", body = UserResponse),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i32>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, HttpError> {
    if current.id() != id {
        return Err(HttpError::Forbidden);
    }
    let updated = update_user(&state.db, id, req.name, req.email).await?;
    Ok(Json(updated.into()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::features::test_support::{
        app_router, insert_user, json_body, register_and_login, request, test_db,
    };

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    #[tokio::test]
    async fn update_user_updates_fields() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let body = req_body(&serde_json::json!({
            "name": "Ada L",
            "email": "ada2@example.com",
        }));

        let res = request(
            login.app,
            Method::PUT,
            &format!("/users/{}", login.user_id),
            Some(body),
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::OK);
        let updated: UserResponse = json_body(res).await?;
        assert_eq!(updated.id, login.user_id);
        assert_eq!(updated.name, "Ada L");
        assert_eq!(updated.email, "ada2@example.com");
        Ok(())
    }

    #[tokio::test]
    async fn update_other_user_is_forbidden() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let grace = insert_user(&db, "Grace", "grace@example.com").await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        assert_ne!(grace.id, login.user_id, "test must target a different user");
        let body = req_body(&serde_json::json!({
            "name": "Grace X",
            "email": "grace@example.com",
        }));

        let res = request(
            login.app,
            Method::PUT,
            &format!("/users/{}", grace.id),
            Some(body),
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[tokio::test]
    async fn update_user_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({
            "name": "Ada",
            "email": "ada@example.com",
        }));

        let res = request(app, Method::PUT, "/users/1", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn update_user_duplicate_email_is_conflict() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        insert_user(&db, "Ada", "ada@example.com").await?;
        let login =
            register_and_login(&db, &store, "Grace", "grace@example.com", "hunter2hunter2").await?;
        let body = req_body(&serde_json::json!({
            "name": "Grace",
            "email": "ada@example.com",
        }));

        let res = request(
            login.app,
            Method::PUT,
            &format!("/users/{}", login.user_id),
            Some(body),
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        Ok(())
    }
}
