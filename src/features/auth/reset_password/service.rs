use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHasher, SaltString};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};

use crate::auth::hash_password_reset_token;
use crate::entities::password_reset_token;
use crate::entities::user;
use crate::service_error::ServiceError;

fn validate_password(password: &str) -> Result<(), ServiceError> {
    if password.len() < 8 {
        return Err(ServiceError::Validation(
            "password must be at least 8 characters".into(),
        ));
    }
    Ok(())
}

/// Set a new password for the user holding `token`.
///
/// The token is sha256-hashed before lookup (the table stores only digests),
/// consumed in the same transaction as the password update, and invalid or
/// expired tokens get the same error so the endpoint cannot distinguish them.
/// Sessions issued before the reset stay valid — server-side sessions are not
/// user-scoped in tower-sessions, so revoking them needs a session table
/// redesign.
pub(super) async fn reset_password(
    db: &DatabaseConnection,
    token: String,
    password: String,
) -> Result<(), ServiceError> {
    validate_password(&password)?;
    let token_hash = hash_password_reset_token(&token);

    let tx = db.begin().await?;
    let Some(stored) = password_reset_token::Entity::find()
        .filter(password_reset_token::Column::TokenHash.eq(&token_hash))
        .one(&tx)
        .await?
    else {
        return Err(ServiceError::Validation(
            "reset token is invalid or expired".into(),
        ));
    };

    if stored.expires_at < chrono::Utc::now() {
        password_reset_token::Entity::delete_by_id(stored.id)
            .exec(&tx)
            .await?;
        tx.commit().await?;
        return Err(ServiceError::Validation(
            "reset token is invalid or expired".into(),
        ));
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| {
            tracing::error!(error = %err, "password hashing failed");
            ServiceError::Internal
        })?;

    let mut user_active = user::ActiveModel {
        id: Set(stored.user_id),
        ..Default::default()
    };
    user_active.password_hash = Set(hash.to_string());
    user_active
        .update(&tx)
        .await
        .map_err(ServiceError::from_db_err)?;

    password_reset_token::Entity::delete_by_id(stored.id)
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use argon2::password_hash::{PasswordHash, PasswordVerifier};

    use super::*;
    use crate::features::auth::forgot_password::service::request_password_reset;
    use crate::features::test_support::{
        TestMailer, register_user_direct, test_config, test_connection,
    };

    /// Request a reset for `email` through the business layer and return the
    /// raw token the mailer received.
    async fn valid_token_for(
        db: &DatabaseConnection,
        email: &str,
    ) -> Result<String, Box<dyn Error>> {
        let mailer = TestMailer::new();
        request_password_reset(db, &mailer, email.to_string(), &test_config()).await?;
        Ok(mailer
            .sent()
            .first()
            .ok_or_else(|| std::io::Error::other("no email sent"))?
            .token
            .clone())
    }

    #[tokio::test]
    async fn reset_changes_the_password() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let token = valid_token_for(&db, "ada@example.com").await?;

        reset_password(&db, token, "newpassword123".to_string()).await?;

        let stored = user::Entity::find_by_id(user.id)
            .one(&db)
            .await?
            .ok_or_else(|| std::io::Error::other("user vanished"))?;
        let parsed = PasswordHash::new(&stored.password_hash)
            .map_err(|err| format!("stored hash is malformed: {err}"))?;
        assert!(
            Argon2::default()
                .verify_password(b"newpassword123", &parsed)
                .is_ok(),
            "new password must verify"
        );
        assert!(
            Argon2::default()
                .verify_password(b"hunter2hunter2", &parsed)
                .is_err(),
            "old password must no longer verify"
        );
        Ok(())
    }

    #[tokio::test]
    async fn token_is_single_use() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let token = valid_token_for(&db, "ada@example.com").await?;

        reset_password(&db, token.clone(), "newpassword123".to_string()).await?;
        let Err(ServiceError::Validation(_)) =
            reset_password(&db, token, "anotherpassword".to_string()).await
        else {
            panic!("a consumed token must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn unknown_token_is_validation_error() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let Err(ServiceError::Validation(_)) =
            reset_password(&db, "f".repeat(64), "newpassword123".to_string()).await
        else {
            panic!("unknown token must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn expired_token_is_validation_error() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let user = register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let token = "expiredtoken".repeat(4);
        password_reset_token::ActiveModel {
            user_id: Set(user.id),
            token_hash: Set(hash_password_reset_token(&token)),
            expires_at: Set(chrono::DateTime::<chrono::FixedOffset>::from(
                chrono::Utc::now() - chrono::Duration::minutes(1),
            )),
            ..Default::default()
        }
        .insert(&db)
        .await?;

        let Err(ServiceError::Validation(_)) =
            reset_password(&db, token, "newpassword123".to_string()).await
        else {
            panic!("expired token must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn short_password_is_validation_error() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let token = valid_token_for(&db, "ada@example.com").await?;

        let Err(ServiceError::Validation(_)) =
            reset_password(&db, token, "short".to_string()).await
        else {
            panic!("short password must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn reset_does_not_touch_other_users_tokens() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let grace =
            register_user_direct(&db, "Grace", "grace@example.com", "hunter2hunter2").await?;
        let grace_token = valid_token_for(&db, "grace@example.com").await?;
        let _grace_token = valid_token_for(&db, "ada@example.com").await?;

        reset_password(&db, grace_token, "newpassword123".to_string()).await?;

        let stored = user::Entity::find_by_id(grace.id)
            .one(&db)
            .await?
            .ok_or_else(|| std::io::Error::other("user vanished"))?;
        let parsed = PasswordHash::new(&stored.password_hash)
            .map_err(|err| format!("stored hash is malformed: {err}"))?;
        assert!(
            Argon2::default()
                .verify_password(b"newpassword123", &parsed)
                .is_ok()
        );
        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert_eq!(rows.len(), 1, "Ada's token must survive Grace's reset");
        Ok(())
    }
}
