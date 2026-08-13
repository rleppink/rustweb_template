use sea_orm::{DatabaseConnection, EntityTrait};

use crate::entities::{prelude::User, user};
use crate::service_error::ServiceError;

pub(super) async fn get_user(
    db: &DatabaseConnection,
    id: i32,
) -> Result<user::Model, ServiceError> {
    User::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ServiceError::NotFound)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::features::test_support::{insert_user, test_connection};

    #[tokio::test]
    async fn get_user_returns_inserted() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let inserted = insert_user(&db, "Ada", "ada@example.com").await?;

        let fetched = get_user(&db, inserted.id).await?;
        assert_eq!(fetched, inserted);
        Ok(())
    }

    #[tokio::test]
    async fn get_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::NotFound) = get_user(&db, 1234).await else {
            panic!("missing user must be NotFound");
        };
        Ok(())
    }
}
