use sea_orm::{DatabaseConnection, EntityTrait};

use crate::entities::prelude::User;
use crate::service_error::ServiceError;

pub(super) async fn delete_user(db: &DatabaseConnection, id: i32) -> Result<(), ServiceError> {
    let res = User::delete_by_id(id).exec(db).await?;
    if res.rows_affected == 0 {
        return Err(ServiceError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::features::test_support::{insert_user, test_connection};

    #[tokio::test]
    async fn delete_user_removes_it() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let inserted = insert_user(&db, "Ada", "ada@example.com").await?;

        delete_user(&db, inserted.id).await?;

        let left = User::find_by_id(inserted.id).one(&db).await?;
        assert!(left.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn delete_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::NotFound) = delete_user(&db, 1234).await else {
            panic!("missing user must be NotFound");
        };
        Ok(())
    }
}
