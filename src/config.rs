use std::str::FromStr;

use thiserror::Error;

/// Typed configuration, loaded from the environment once at startup.
///
/// `from_env` fails fast on malformed or out-of-range values: a server that
/// starts with a silent default the operator never asked for is harder to
/// debug than a startup that refuses to boot.
#[derive(Clone, Debug)]
pub(crate) struct Config {
    /// Postgres connection string (`DATABASE_URL`, required).
    pub(crate) database_url: String,
    /// TCP port to listen on (`PORT`, default 3000).
    pub(crate) port: u16,
    /// Mark the session cookie `Secure` (`COOKIE_SECURE`, default off).
    pub(crate) cookie_secure: bool,
    /// Refill rate of the per-IP bucket on `/auth/*` (`AUTH_RATE_LIMIT_PER_SECOND`).
    pub(crate) auth_rate_limit_per_second: u64,
    /// Burst size of the per-IP bucket on `/auth/*` (`AUTH_RATE_LIMIT_BURST`).
    pub(crate) auth_rate_limit_burst: u32,
    /// How long a password-reset token stays valid (`PASSWORD_RESET_TOKEN_TTL_MINUTES`).
    pub(crate) password_reset_token_ttl: chrono::Duration,
    /// Public origin used to build reset links in emails (`PUBLIC_BASE_URL`).
    pub(crate) public_base_url: String,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self, ConfigError> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| ConfigError::MissingDatabaseUrl)?;
        let port = parse_env("PORT", 3000, "a number between 0 and 65535")?;
        let cookie_secure = parse_bool_env("COOKIE_SECURE", false)?;
        let auth_rate_limit_per_second = positive_u64_env("AUTH_RATE_LIMIT_PER_SECOND", 5)?;
        let auth_rate_limit_burst = positive_u32_env("AUTH_RATE_LIMIT_BURST", 10)?;
        let ttl_minutes =
            parse_env::<i64>("PASSWORD_RESET_TOKEN_TTL_MINUTES", 60, "a whole number")?;
        if !(1..=1440).contains(&ttl_minutes) {
            return Err(ConfigError::invalid_value(
                "PASSWORD_RESET_TOKEN_TTL_MINUTES",
                ttl_minutes,
                "must be between 1 and 1440",
            ));
        }
        let public_base_url = std::env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());

        Ok(Self {
            database_url,
            port,
            cookie_secure,
            auth_rate_limit_per_second,
            auth_rate_limit_burst,
            password_reset_token_ttl: chrono::Duration::minutes(ttl_minutes),
            public_base_url,
        })
    }
}

/// A configuration value could not be loaded from the environment.
#[derive(Debug, Error)]
pub(crate) enum ConfigError {
    #[error("DATABASE_URL must be set (see .env.example)")]
    MissingDatabaseUrl,
    #[error("invalid value for {name}: `{value}` — {reason}")]
    InvalidValue {
        name: String,
        value: String,
        reason: String,
    },
}

impl ConfigError {
    fn invalid_value<T: std::fmt::Display>(
        name: &str,
        value: T,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidValue {
            name: name.to_string(),
            value: value.to_string(),
            reason: reason.into(),
        }
    }
}

/// Parse `name` from the environment, falling back to `default` when unset.
fn parse_env<T: FromStr>(name: &str, default: T, expected: &str) -> Result<T, ConfigError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| ConfigError::invalid_value(name, &value, expected)),
        Err(_) => Ok(default),
    }
}

/// Parse `name` as a boolean: 1/true/yes/on and 0/false/no/off, case-insensitive.
fn parse_bool_env(name: &str, default: bool) -> Result<bool, ConfigError> {
    match std::env::var(name) {
        Ok(value) if parse_bool(&value) => Ok(true),
        Ok(value) if parse_bool_off(&value) => Ok(false),
        Ok(value) => Err(ConfigError::invalid_value(
            name,
            &value,
            "expected a boolean (1/0, true/false, yes/no, on/off)",
        )),
        Err(_) => Ok(default),
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_bool_off(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn positive_u64_env(name: &str, default: u64) -> Result<u64, ConfigError> {
    let value = parse_env(name, default, "a whole number greater than 0")?;
    if value == 0 {
        return Err(ConfigError::invalid_value(
            name,
            value,
            "must be at least 1",
        ));
    }
    Ok(value)
}

fn positive_u32_env(name: &str, default: u32) -> Result<u32, ConfigError> {
    let value = parse_env(name, default, "a whole number greater than 0")?;
    if value == 0 {
        return Err(ConfigError::invalid_value(
            name,
            value,
            "must be at least 1",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_parser_accepts_truey_spellings() {
        for value in ["1", "true", "TRUE", "True", "yes", "on"] {
            assert!(parse_bool(value), "{value:?} must parse as true");
        }
    }

    #[test]
    fn boolean_parser_accepts_falsey_spellings() {
        for value in ["0", "false", "FALSE", "no", "off"] {
            assert!(parse_bool_off(value), "{value:?} must parse as false");
        }
    }

    #[test]
    fn boolean_parser_rejects_anything_else() {
        for value in ["maybe", "2", "yes please", ""] {
            assert!(
                !parse_bool(value) && !parse_bool_off(value),
                "{value:?} must be rejected"
            );
        }
    }

    #[test]
    fn error_names_the_variable_and_its_value() {
        let err = ConfigError::invalid_value("PORT", "nope", "must be a number");
        assert!(
            err.to_string().contains("PORT") && err.to_string().contains("nope"),
            "error must name the variable and its value: {err}"
        );
    }
}
