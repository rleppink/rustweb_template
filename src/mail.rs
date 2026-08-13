use thiserror::Error;

/// Outbound email delivery, injected into [`crate::state::AppState`] so slices
/// never construct mailers themselves.
///
/// The trait is deliberately sync and object-safe: real transports (SMTP, an
/// email API) should do their blocking I/O in `tokio::task::spawn_blocking`,
/// keeping the caller's async context free.
pub(crate) trait Mailer: Send + Sync {
    /// Deliver a password-reset token. The token is single-use and short-lived
    /// (see `Config::password_reset_token_ttl`); it exists nowhere on the
    /// server except the reset email.
    fn send_password_reset(&self, to: &str, token: &str) -> Result<(), MailerError>;
}

/// A mailer could not deliver a message.
#[derive(Debug, Error)]
#[error("{0}")]
pub(crate) struct MailerError(pub(crate) String);

/// The default mailer: logs a ready-to-paste reset command instead of sending
/// mail.
///
/// This is the honest default for a template — there is no SMTP transport to
/// configure — but a real deployment must replace it: `Arc::new(LogMailer)`
/// leaves every reset token in the application logs. Swap in a real transport
/// before real users arrive.
pub(crate) struct LogMailer {
    public_base_url: String,
}

impl LogMailer {
    pub(crate) fn new(public_base_url: String) -> Self {
        Self { public_base_url }
    }
}

impl Mailer for LogMailer {
    fn send_password_reset(&self, to: &str, token: &str) -> Result<(), MailerError> {
        // `confirm` is a POST endpoint, so a plain URL would 405 in a browser;
        // emit a curl the dev can paste directly (substituting a password).
        tracing::warn!(
            to,
            reset_curl = format!(
                "curl -X POST {}/auth/password-reset/confirm -H 'content-type: application/json' \
                 -d '{{\"token\":\"{token}\",\"password\":\"NEW_PASSWORD\"}}'",
                self.public_base_url
            ),
            "password reset requested — dev mailer: no real email was sent",
        );
        Ok(())
    }
}
