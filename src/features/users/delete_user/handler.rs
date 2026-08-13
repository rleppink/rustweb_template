use axum::extract::{Path, State};
use axum::http::StatusCode;

use super::service::delete_user;
use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::state::AppState;

#[utoipa::path(
    delete,
    path = "/users/{id}",
    tag = "users",
    security(("session_cookie" = [])),
    params(("id" = i32, Path, description = "User id")),
    responses(
        (status = 204, description = "User deleted"),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i32>,
) -> Result<StatusCode, HttpError> {
    if current.id() != id {
        return Err(HttpError::Forbidden);
    }
    delete_user(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use crate::features::test_support::{
        app_router, insert_user, register_and_login, request, test_db,
    };

    #[tokio::test]
    async fn delete_user_returns_204() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let res = request(
            login.app,
            Method::DELETE,
            &format!("/users/{}", login.user_id),
            None,
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        Ok(())
    }

    #[tokio::test]
    async fn delete_other_user_is_forbidden() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let grace = insert_user(&db, "Grace", "grace@example.com").await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        assert_ne!(grace.id, login.user_id, "test must target a different user");

        let res = request(
            login.app,
            Method::DELETE,
            &format!("/users/{}", grace.id),
            None,
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[tokio::test]
    async fn delete_user_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);

        let res = request(app, Method::DELETE, "/users/1", None, None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
