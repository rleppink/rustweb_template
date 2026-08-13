use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use super::service::list_user_posts;
use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub(super) struct PostView {
    id: i32,
    title: String,
    body: String,
}

#[utoipa::path(
    get,
    path = "/me/posts",
    tag = "posts",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "Posts for the current user", body = Vec<PostView>),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<PostView>>, HttpError> {
    let posts = list_user_posts(&state.db, current.id())
        .await?
        .into_iter()
        .map(|p| PostView {
            id: p.id,
            title: p.title,
            body: p.body,
        })
        .collect();
    Ok(Json(posts))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use crate::features::test_support::{
        app_router, insert_post, insert_user, json_body, register_and_login, request, test_db,
    };

    #[tokio::test]
    async fn list_posts_is_empty_initially() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let res = request(login.app, Method::GET, "/me/posts", None, Some(&login.jar)).await?;
        assert_eq!(res.status(), StatusCode::OK);
        let posts: Vec<serde_json::Value> = json_body(res).await?;
        assert!(posts.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn list_posts_returns_only_own_posts() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let grace = insert_user(&db, "Grace", "grace@example.com").await?;
        insert_post(&db, grace.id, "other", "post").await?;
        insert_post(&db, login.user_id, "mine", "own").await?;

        let res = request(login.app, Method::GET, "/me/posts", None, Some(&login.jar)).await?;
        assert_eq!(res.status(), StatusCode::OK);
        let posts: Vec<serde_json::Value> = json_body(res).await?;
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts.first().and_then(|p| p.get("title")),
            Some(&serde_json::Value::String("mine".to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn list_posts_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);

        let res = request(app, Method::GET, "/me/posts", None, None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
