use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::entities::{prelude::User, user};
use crate::service_error::ServiceError;

pub(super) async fn login_user(
    db: &DatabaseConnection,
    email: String,
    password: String,
) -> Result<user::Model, ServiceError> {
    let Some(user) = User::find()
        .filter(user::Column::Email.eq(&email))
        .one(db)
        .await?
    else {
        return Err(ServiceError::Unauthorized);
    };

    let parsed = PasswordHash::new(&user.password_hash).map_err(|err| {
        // An unparseable stored hash — e.g. the `''` backfill for users that
        // predate the auth migration — means there is no usable credential:
        // Unauthorized, never a 500 (and never a signal to probes).
        tracing::debug!(error = %err, "stored password hash is unparseable");
        ServiceError::Unauthorized
    })?;
    if Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
    {
        Ok(user)
    } else {
        Err(ServiceError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use sea_orm::{ActiveModelTrait, Set};

    use super::*;
    use crate::features::test_support::{register_user_direct, test_connection};

    #[tokio::test]
    async fn login_user_with_correct_password() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let logged_in = login_user(
            &db,
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await?;
        assert_eq!(logged_in.email, "ada@example.com");
        Ok(())
    }

    #[tokio::test]
    async fn login_user_with_wrong_password_is_unauthorized() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;

        let Err(ServiceError::Unauthorized) = login_user(
            &db,
            "ada@example.com".to_string(),
            "wrong-password".to_string(),
        )
        .await
        else {
            panic!("wrong password must be Unauthorized");
        };
        Ok(())
    }

    #[tokio::test]
    async fn login_user_with_unknown_email_is_unauthorized() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;

        let Err(ServiceError::Unauthorized) = login_user(
            &db,
            "nobody@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await
        else {
            panic!("unknown email must be Unauthorized");
        };
        Ok(())
    }

    #[tokio::test]
    async fn login_user_with_unparseable_stored_hash_is_unauthorized() -> Result<(), Box<dyn Error>>
    {
        let (db, _guard) = test_connection().await?;
        user::ActiveModel {
            name: Set("Ada".to_string()),
            email: Set("ada@example.com".to_string()),
            password_hash: Set(String::new()),
            ..Default::default()
        }
        .insert(&db)
        .await?;

        let Err(ServiceError::Unauthorized) = login_user(
            &db,
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await
        else {
            panic!("legacy user with empty hash must be Unauthorized, not 500");
        };
        Ok(())
    }
}
