mod delete_user;
mod get_user;
mod list_users;
mod update_user;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;

use crate::entities::user;
use crate::state::AppState;

/// API representation of a user. Deliberately excludes `password_hash`, which
/// lives on the entity and must never reach a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct UserResponse {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) email: String,
}

impl From<user::Model> for UserResponse {
    fn from(model: user::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            email: model.email,
        }
    }
}

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(list_users::route())
        .merge(get_user::route())
        .merge(update_user::route())
        .merge(delete_user::route())
}
