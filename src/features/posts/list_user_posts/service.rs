use sea_orm::{DatabaseConnection, EntityTrait, ModelTrait};

use crate::entities::{post, prelude::User};
use crate::service_error::ServiceError;

pub(super) async fn list_user_posts(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<Vec<post::Model>, ServiceError> {
    let user = User::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(ServiceError::NotFound)?;
    user.find_related(post::Entity)
        .all(db)
        .await
        .map_err(ServiceError::from)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::features::test_support::{insert_post, insert_user, test_connection};

    #[tokio::test]
    async fn list_user_posts_is_empty_initially() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = insert_user(&db, "Ada", "ada@example.com").await?;

        let posts = list_user_posts(&db, user.id).await?;
        assert!(posts.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn list_user_posts_returns_inserted() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = insert_user(&db, "Ada", "ada@example.com").await?;
        let post = insert_post(&db, user.id, "Hello", "world").await?;

        let posts = list_user_posts(&db, user.id).await?;
        assert_eq!(posts, vec![post]);
        Ok(())
    }

    #[tokio::test]
    async fn list_posts_for_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::NotFound) = list_user_posts(&db, 1234).await else {
            panic!("posts for missing user must be NotFound");
        };
        Ok(())
    }
}
