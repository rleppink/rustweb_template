use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};

use crate::entities::{prelude::User, user};
use crate::service_error::ServiceError;

fn validate(name: &str, email: &str) -> Result<(), ServiceError> {
    if name.trim().is_empty() {
        return Err(ServiceError::Validation("name must not be empty".into()));
    }
    if !email.contains('@') {
        return Err(ServiceError::Validation("email is invalid".into()));
    }
    Ok(())
}

pub(super) async fn update_user(
    db: &DatabaseConnection,
    id: i32,
    name: String,
    email: String,
) -> Result<user::Model, ServiceError> {
    validate(&name, &email)?;
    let mut active = User::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ServiceError::NotFound)?
        .into_active_model();
    active.name = Set(name);
    active.email = Set(email);
    let updated = active.update(db).await.map_err(ServiceError::from_db_err)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use proptest::prelude::*;

    use super::*;
    use crate::features::test_support::{insert_user, test_connection};

    #[tokio::test]
    async fn update_user_updates_fields() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let inserted = insert_user(&db, "Ada", "ada@example.com").await?;

        let updated = update_user(
            &db,
            inserted.id,
            "Ada L".to_string(),
            "ada2@example.com".to_string(),
        )
        .await?;
        assert_eq!(updated.id, inserted.id);
        assert_eq!(updated.name, "Ada L");
        assert_eq!(updated.email, "ada2@example.com");
        Ok(())
    }

    #[tokio::test]
    async fn update_missing_user_is_not_found() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::NotFound) =
            update_user(&db, 1234, "Ada".to_string(), "ada@example.com".to_string()).await
        else {
            panic!("missing user must be NotFound");
        };
        Ok(())
    }

    #[tokio::test]
    async fn update_user_rejects_blank_name() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let inserted = insert_user(&db, "Ada", "ada@example.com").await?;

        let Err(ServiceError::Validation(_)) = update_user(
            &db,
            inserted.id,
            " ".to_string(),
            "ada@example.com".to_string(),
        )
        .await
        else {
            panic!("blank name must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn update_user_with_duplicate_email_is_conflict() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        insert_user(&db, "Ada", "ada@example.com").await?;
        let grace = insert_user(&db, "Grace", "grace@example.com").await?;

        let Err(ServiceError::Conflict) = update_user(
            &db,
            grace.id,
            "Grace".to_string(),
            "ada@example.com".to_string(),
        )
        .await
        else {
            panic!("duplicate email must be a Conflict");
        };
        let unchanged = User::find_by_id(grace.id).one(&db).await?;
        assert_eq!(unchanged, Some(grace));
        Ok(())
    }

    proptest! {
        /// Validation must never panic, whatever bytes arrive in the request.
        #[test]
        fn validate_never_panics(name in ".*", email in ".*") {
            let _outcome = validate(&name, &email);
        }

        /// A name that is empty once trimmed is rejected, regardless of email.
        #[test]
        fn blank_name_is_rejected(name in r"\s*", email in ".*") {
            prop_assert!(validate(&name, &email).is_err());
        }

        /// A name with at least one non-whitespace char plus an `@` in the
        /// email is always accepted.
        #[test]
        fn valid_input_is_accepted(
            name in r"\s*\S+.*",
            local in "[^@]*",
            domain in "[^@]*",
        ) {
            let email = format!("{local}@{domain}");
            prop_assert!(validate(&name, &email).is_ok());
        }
    }
}
