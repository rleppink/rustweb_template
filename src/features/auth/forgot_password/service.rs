use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};

use crate::auth::new_password_reset_token;
use crate::config::Config;
use crate::entities::password_reset_token;
use crate::entities::{prelude::User, user};
use crate::mail::Mailer;
use crate::service_error::ServiceError;

fn validate_email(email: &str) -> Result<(), ServiceError> {
    if !email.contains('@') {
        return Err(ServiceError::Validation("email is invalid".into()));
    }
    Ok(())
}

/// Issue a single-use password-reset token for `email` and hand it to the
/// mailer.
///
/// Unknown emails are silently accepted: the caller cannot tell an existing
/// account from a typo'd one by status code, so the endpoint leaks nothing
/// directly. (Known emails do extra work — a transaction, an insert, and a
/// mailer call — so response timing can still hint at existence; the per-IP
/// rate limit on `/auth/*` is the backstop against both enumeration vectors.)
///
/// Requesting a reset invalidates any previous outstanding tokens for the
/// user, and does so atomically with the insert. If the mailer then fails,
/// the new token is deleted again, so no valid-but-never-emailed token is
/// left outstanding.
///
/// `pub(crate)` so the sibling `reset_password` slice's tests can mint tokens
/// through the business layer.
pub(crate) async fn request_password_reset(
    db: &DatabaseConnection,
    mailer: &dyn Mailer,
    email: String,
    config: &Config,
) -> Result<(), ServiceError> {
    validate_email(&email)?;

    let Some(user) = User::find()
        .filter(user::Column::Email.eq(&email))
        .one(db)
        .await?
    else {
        return Ok(());
    };

    let (token, token_hash) = new_password_reset_token();

    let tx = db.begin().await?;
    password_reset_token::Entity::delete_many()
        .filter(password_reset_token::Column::UserId.eq(user.id))
        .exec(&tx)
        .await?;
    password_reset_token::ActiveModel {
        user_id: Set(user.id),
        token_hash: Set(token_hash),
        expires_at: Set(chrono::DateTime::<chrono::FixedOffset>::from(
            chrono::Utc::now() + config.password_reset_token_ttl,
        )),
        ..Default::default()
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;

    if let Err(err) = mailer.send_password_reset(&user.email, &token) {
        tracing::error!(error = %err, to = %user.email, "failed to send password reset email");
        // Best-effort cleanup: do not leave a valid token that was never
        // emailed outstanding for the TTL.
        password_reset_token::Entity::delete_many()
            .filter(password_reset_token::Column::UserId.eq(user.id))
            .exec(db)
            .await?;
        return Err(ServiceError::Internal);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::auth::hash_password_reset_token;
    use crate::features::test_support::{
        TestMailer, register_user_direct, test_config, test_connection,
    };
    use crate::mail::MailerError;

    #[tokio::test]
    async fn stores_sha256_of_the_emailed_token() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = TestMailer::new();
        request_password_reset(&db, &mailer, "ada@example.com".to_string(), &test_config()).await?;

        let raw_token = mailer
            .sent()
            .first()
            .ok_or_else(|| std::io::Error::other("no email sent"))?
            .token
            .clone();
        let row = password_reset_token::Entity::find()
            .one(&db)
            .await?
            .ok_or_else(|| std::io::Error::other("no token row"))?;
        assert_eq!(
            row.token_hash,
            hash_password_reset_token(&raw_token),
            "only the sha256 of the raw token may be stored"
        );
        assert_ne!(
            row.token_hash, raw_token,
            "the raw token must not be stored"
        );
        assert!(
            row.expires_at > chrono::Utc::now(),
            "token must not expire immediately"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unknown_email_is_ok_and_sends_nothing() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let mailer = TestMailer::new();
        request_password_reset(
            &db,
            &mailer,
            "nobody@example.com".to_string(),
            &test_config(),
        )
        .await?;
        assert!(mailer.sent().is_empty());
        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert!(rows.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn malformed_email_is_validation_error() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        let mailer = TestMailer::new();
        let Err(ServiceError::Validation(_)) =
            request_password_reset(&db, &mailer, "not-an-email".to_string(), &test_config()).await
        else {
            panic!("malformed email must be rejected");
        };
        Ok(())
    }

    #[tokio::test]
    async fn second_request_replaces_the_first_token() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = TestMailer::new();
        for _ in 0..2 {
            request_password_reset(&db, &mailer, "ada@example.com".to_string(), &test_config())
                .await?;
        }

        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert_eq!(rows.len(), 1, "only the newest token may be outstanding");
        let last_token = mailer
            .sent()
            .last()
            .ok_or_else(|| std::io::Error::other("no email sent"))?
            .token
            .clone();
        assert_eq!(
            rows.first()
                .ok_or_else(|| std::io::Error::other("no token row"))?
                .token_hash,
            hash_password_reset_token(&last_token)
        );
        Ok(())
    }

    #[derive(Debug)]
    struct FailingMailer;

    impl Mailer for FailingMailer {
        fn send_password_reset(&self, _to: &str, _token: &str) -> Result<(), MailerError> {
            Err(MailerError("delivery failed".into()))
        }
    }

    #[tokio::test]
    async fn mailer_failure_is_internal_error_and_leaves_no_token() -> Result<(), Box<dyn Error>> {
        let (db, _guard) = test_connection().await?;
        register_user_direct(&db, "Ada", "ada@example.com", "hunter2hunter2").await?;
        let mailer = FailingMailer;

        let Err(err) =
            request_password_reset(&db, &mailer, "ada@example.com".to_string(), &test_config())
                .await
        else {
            panic!("mailer failure must surface as an error");
        };
        assert!(matches!(err, ServiceError::Internal));

        let rows = password_reset_token::Entity::find().all(&db).await?;
        assert!(
            rows.is_empty(),
            "no token may remain when the email was never sent"
        );
        Ok(())
    }
}
