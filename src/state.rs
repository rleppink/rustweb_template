use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::config::Config;
use crate::mail::Mailer;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) db: DatabaseConnection,
    /// Outbound email, injected so slices never construct mailers themselves
    /// and tests can capture what was sent.
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) config: Config,
}
