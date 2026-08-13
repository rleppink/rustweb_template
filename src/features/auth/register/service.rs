use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHasher, SaltString};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use crate::entities::user;
use crate::service_error::ServiceError;

fn validate(name: &str, email: &str, password: &str) -> Result<(), ServiceError> {
    if name.trim().is_empty() {
        return Err(ServiceError::Validation("name must not be empty".into()));
    }
    if !email.contains('@') {
        return Err(ServiceError::Validation("email is invalid".into()));
    }
    if password.len() < 8 {
        return Err(ServiceError::Validation(
            "password must be at least 8 characters".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn register_user(
    db: &DatabaseConnection,
    name: String,
    email: String,
    password: String,
) -> Result<user::Model, ServiceError> {
    validate(&name, &email, &password)?;

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| {
            tracing::error!(error = %err, "password hashing failed");
            ServiceError::Internal
        })?;

    let saved = user::ActiveModel {
        name: Set(name),
        email: Set(email),
        password_hash: Set(hash.to_string()),
        ..Default::default()
    }
    .insert(db)
    .await
    .map_err(ServiceError::from_db_err)?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use argon2::password_hash::{PasswordHash, PasswordVerifier};

    use super::*;
    use crate::features::test_support::test_connection;

    #[tokio::test]
    async fn register_user_round_trips() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let created = register_user(
            &db,
            "Ada".to_string(),
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await?;
        assert_eq!(created.name, "Ada");
        assert_eq!(created.email, "ada@example.com");
        assert!(created.id > 0);
        Ok(())
    }

    #[tokio::test]
    async fn register_user_stores_argon2_hash() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let created = register_user(
            &db,
            "Ada".to_string(),
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await?;
        let parsed = PasswordHash::new(&created.password_hash)
            .map_err(|err| format!("stored hash is malformed: {err}"))?;
        let verified = Argon2::default()
            .verify_password(b"hunter2hunter2", &parsed)
            .is_ok();
        assert!(verified, "stored hash must verify the original password");
        Ok(())
    }

    #[tokio::test]
    async fn register_user_never_stores_plaintext() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let created = register_user(
            &db,
            "Ada".to_string(),
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await?;
        assert_ne!(created.password_hash, "hunter2hunter2");
        assert!(created.password_hash.starts_with("$argon2"));
        Ok(())
    }

    #[tokio::test]
    async fn register_user_rejects_short_password() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::Validation(_)) = register_user(
            &db,
            "Ada".to_string(),
            "ada@example.com".to_string(),
            "short".to_string(),
        )
        .await
        else {
            panic!("short password must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn register_user_with_duplicate_email_is_conflict() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user(
            &db,
            "Ada".to_string(),
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await?;

        let Err(ServiceError::Conflict) = register_user(
            &db,
            "Grace".to_string(),
            "ada@example.com".to_string(),
            "hunter2hunter2".to_string(),
        )
        .await
        else {
            panic!("duplicate email must be a Conflict");
        };
        Ok(())
    }

    proptest::proptest! {
        /// Validation must never panic, whatever bytes arrive in the request.
        #[test]
        fn validate_never_panics(name in ".*", email in ".*", password in ".*") {
            let _outcome = validate(&name, &email, &password);
        }
    }
}
