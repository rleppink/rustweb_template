use axum::Json;

use crate::auth::CurrentUser;
use crate::error::{ErrorResponses, HttpError};
use crate::features::users::UserResponse;

#[utoipa::path(
    get,
    path = "/auth/me",
    tag = "auth",
    security(("session_cookie" = [])),
    responses(
        (status = 200, description = "The authenticated user", body = UserResponse),
        ErrorResponses,
    )
)]
pub(super) async fn handle(current: CurrentUser) -> Result<Json<UserResponse>, HttpError> {
    Ok(Json(current.0.into()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use crate::features::test_support::{
        app_router, json_body, register_and_login, request, test_db,
    };
    use crate::features::users::UserResponse;

    #[tokio::test]
    async fn me_returns_authenticated_user() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let res = request(login.app, Method::GET, "/auth/me", None, Some(&login.jar)).await?;
        assert_eq!(res.status(), StatusCode::OK);
        let me: UserResponse = json_body(res).await?;
        assert_eq!(me.name, "Ada");
        assert_eq!(me.email, "ada@example.com");
        Ok(())
    }

    #[tokio::test]
    async fn me_requires_auth() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let app = app_router(db, store);

        let res = request(app, Method::GET, "/auth/me", None, None).await?;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }
}
