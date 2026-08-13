mod forgot_password;
mod login;
mod logout;
mod me;
mod register;
mod reset_password;

/// Re-exported so `test_support` can register users through the business layer
/// without widening `register`'s module visibility.
#[cfg(test)]
pub(crate) use register::service::register_user;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(register::route())
        .merge(login::route())
        .merge(logout::route())
        .merge(me::route())
        .merge(forgot_password::route())
        .merge(reset_password::route())
}
