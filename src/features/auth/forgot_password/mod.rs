mod handler;
pub(super) mod service;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

pub(super) fn route() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handler::handle))
}
