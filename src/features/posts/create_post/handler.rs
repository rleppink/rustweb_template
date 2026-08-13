use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use super::service::create_post;
use crate::auth::CurrentUser;
use crate::entities::post;
use crate::error::{ErrorResponses, HttpError};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub(super) struct CreatePostRequest {
    title: String,
    body: String,
}

#[utoipa::path(
    post,
    path = "/me/posts",
    tag = "posts",
    security(("session_cookie" = [])),
    request_body = CreatePostRequest,
    responses(
        (status = 201, description = "Post created", body = post::Model),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<CreatePostRequest>,
) -> Result<(StatusCode, Json<post::Model>), HttpError> {
    let saved = create_post(&state.db, current.id(), req.title, req.body).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::body::Body;
    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::features::test_support::{
        app_router, json_body, register_and_login, request, test_db,
    };

    fn req_body(value: &serde_json::Value) -> Body {
        Body::from(value.to_string())
    }

    #[tokio::test]
    async fn create_post_returns_201() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let body = req_body(&serde_json::json!({"title": "Hello", "body": "world"}));

        let res = request(
            login.app,
            Method::POST,
            "/me/posts",
            Some(body),
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::CREATED);
        let created: post::Model = json_body(res).await?;
        assert_eq!(created.user_id, login.user_id);
        assert_eq!(created.title, "Hello");
        assert_eq!(created.body, "world");
        assert!(created.id > 0);
        Ok(())
    }

    #[tokio::test]
    async fn create_post_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);
        let body = req_body(&serde_json::json!({"title": "Hello", "body": "world"}));

        let res = request(app, Method::POST, "/me/posts", Some(body), None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn create_post_blank_title_is_bad_request() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let body = req_body(&serde_json::json!({"title": "  ", "body": "world"}));

        let res = request(
            login.app,
            Method::POST,
            "/me/posts",
            Some(body),
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }
}
