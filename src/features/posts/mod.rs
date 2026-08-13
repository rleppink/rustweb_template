mod create_post;
mod list_user_posts;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(create_post::route())
        .merge(list_user_posts::route())
}
