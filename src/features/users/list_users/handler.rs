use axum::Json;
use axum::extract::State;

use super::service::list_users;
use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/users",
    tag = "users",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "List of users", body = Vec<UserResponse>),
        ErrorResponses,
    )
)]
pub(super) async fn handle(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> Result<Json<Vec<UserResponse>>, HttpError> {
    let users = list_users(&state.db).await?;
    Ok(Json(users.into_iter().map(UserResponse::from).collect()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use super::*;
    use crate::features::test_support::{
        app_router, insert_user, json_body, register_and_login, request, test_db,
    };

    #[tokio::test]
    async fn list_users_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);

        let res = request(app, Method::GET, "/users", None, None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn list_users_returns_inserted_users() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;
        insert_user(&db, "Grace", "grace@example.com").await?;

        let res = request(login.app, Method::GET, "/users", None, Some(&login.jar)).await?;
        assert_eq!(res.status(), StatusCode::OK);
        let users: Vec<UserResponse> = json_body(res).await?;
        assert_eq!(users.len(), 2);
        assert!(users.iter().any(|u| u.name == "Ada"));
        assert!(users.iter().any(|u| u.name == "Grace"));
        Ok(())
    }
}
