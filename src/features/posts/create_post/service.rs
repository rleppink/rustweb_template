use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set, TransactionTrait};

use crate::entities::{post, prelude::User};
use crate::service_error::ServiceError;

fn validate(title: &str) -> Result<(), ServiceError> {
    if title.trim().is_empty() {
        return Err(ServiceError::Validation("title must not be empty".into()));
    }
    Ok(())
}

pub(super) async fn create_post(
    db: &DatabaseConnection,
    user_id: i32,
    title: String,
    body: String,
) -> Result<post::Model, ServiceError> {
    validate(&title)?;

    let txn = db.begin().await?;

    if User::find_by_id(user_id).one(&txn).await?.is_none() {
        return Err(ServiceError::NotFound);
    }

    let saved = post::ActiveModel {
        user_id: Set(user_id),
        title: Set(title),
        body: Set(body),
        ..Default::default()
    }
    .insert(&txn)
    .await?;

    txn.commit().await?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use proptest::prelude::*;

    use super::*;
    use crate::features::test_support::{insert_user, test_connection};

    #[tokio::test]
    async fn create_post_round_trips() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = insert_user(&db, "Ada", "ada@example.com").await?;

        let created = create_post(&db, user.id, "Hello".to_string(), "world".to_string()).await?;
        assert_eq!(created.user_id, user.id);
        assert_eq!(created.title, "Hello");
        assert_eq!(created.body, "world");
        assert!(created.id > 0);
        Ok(())
    }

    #[tokio::test]
    async fn create_post_for_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::NotFound) =
            create_post(&db, 1234, "Hello".to_string(), "world".to_string()).await
        else {
            panic!("post for missing user must be NotFound");
        };
        Ok(())
    }

    #[tokio::test]
    async fn create_post_rejects_blank_title() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = insert_user(&db, "Ada", "ada@example.com").await?;

        let Err(ServiceError::Validation(_)) =
            create_post(&db, user.id, "   ".to_string(), "world".to_string()).await
        else {
            panic!("blank title must be rejected");
        };
        Ok(())
    }

    proptest! {
        /// Validation must never panic, whatever bytes arrive in the request.
        #[test]
        fn validate_never_panics(title in ".*") {
            let _outcome = validate(&title);
        }

        /// A title that is empty once trimmed is rejected.
        #[test]
        fn blank_title_is_rejected(title in r"\s*") {
            prop_assert!(validate(&title).is_err());
        }

        /// Any title with a non-whitespace char is accepted.
        #[test]
        fn non_blank_title_is_accepted(title in r"\s*\S+.*") {
            prop_assert!(validate(&title).is_ok());
        }
    }
}
