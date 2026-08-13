use axum::Json;
use axum::extract::{Path, State};

use super::service::get_user;
use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/users/{id}",
    tag = "users",
    security(("session_cookie" = [])),
    params(("id" = i32, Path, description = "User id")),
    responses(
        (status = 200, description = "The user", body = UserResponse),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    _current: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<UserResponse>, HttpError> {
    let found = get_user(&state.db, id).await?;
    Ok(Json(found.into()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::error::ErrorBody;
    use crate::features::test_support::{
        app_router, insert_user, json_body, register_and_login, request, test_db,
    };

    #[tokio::test]
    async fn get_user_returns_inserted() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let inserted = insert_user(&db, "Ada", "ada@example.com").await?;
        let login =
            register_and_login(&db, &store, "Grace", "grace@example.com", "hunter2hunter2").await?;

        let res = request(
            login.app,
            Method::GET,
            &format!("/users/{}", inserted.id),
            None,
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::OK);
        let fetched: UserResponse = json_body(res).await?;
        assert_eq!(fetched.id, inserted.id);
        assert_eq!(fetched.name, "Ada");
        Ok(())
    }

    #[tokio::test]
    async fn get_user_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);

        let res = request(app, Method::GET, "/users/1", None, None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn get_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let res = request(
            login.app,
            Method::GET,
            "/users/1234",
            None,
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let err: ErrorBody = json_body(res).await?;
        assert_eq!(err.error, "not found");
        Ok(())
    }
}
