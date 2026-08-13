use axum::http::StatusCode;
use tower_sessions::Session;

use crate::error::HttpError;

#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Session destroyed"),
    )
)]
pub(super) async fn handle(session: Session) -> Result<StatusCode, HttpError> {
    session.flush().await.map_err(|err| {
        tracing::error!(error = %err, "failed to destroy session");
        HttpError::InternalServerError
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::{Method, StatusCode};

    use crate::features::test_support::{register_and_login, request, test_db};

    #[tokio::test]
    async fn logout_returns_204_and_clears_cookie() -> Result<(), Box<dyn Error>> {
        let (db, store, _guard) = test_db().await?;
        let login =
            register_and_login(&db, &store, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let res = request(
            login.app,
            Method::POST,
            "/auth/logout",
            None,
            Some(&login.jar),
        )
        .await?;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let has_removal_cookie = res
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|c| c.to_str().is_ok_and(|v| v.contains("Max-Age=0")));
        assert!(has_removal_cookie, "logout must clear the session cookie");
        Ok(())
    }
}
