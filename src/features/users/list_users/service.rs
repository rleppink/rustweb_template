use sea_orm::{DatabaseConnection, EntityTrait};

use crate::entities::{prelude::User, user};
use crate::service_error::ServiceError;

pub(super) async fn list_users(db: &DatabaseConnection) -> Result<Vec<user::Model>, ServiceError> {
    User::find().all(db).await.map_err(ServiceError::from)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::features::test_support::{insert_user, test_connection};

    #[tokio::test]
    async fn list_users_is_empty_initially() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let users = list_users(&db).await?;
        assert!(users.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn list_users_returns_inserted_users() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let ada = insert_user(&db, "Ada", "ada@example.com").await?;
        let grace = insert_user(&db, "Grace", "grace@example.com").await?;

        let users = list_users(&db).await?;
        assert_eq!(users.len(), 2);
        assert!(users.iter().any(|u| u.id == ada.id));
        assert!(users.iter().any(|u| u.id == grace.id));
        Ok(())
    }
}
