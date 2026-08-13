use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use axum::response::Response;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr, Set};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower_sessions_sqlx_store::PostgresStore;

use crate::config::Config;
use crate::entities::{post, user};
use crate::features::auth::register_user;
use crate::features::users::UserResponse;
use crate::mail::{Mailer, MailerError};
use crate::state::AppState;

/// Every test request pretends to come from this peer; rate-limit tests that
/// need distinct clients use [`request_from`].
const TEST_PEER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345);

/// Scratch databases live for a test only; a hung connect (e.g. postgres went
/// down mid-suite) should fail fast rather than stall each teardown 30s.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A fresh, migrated, scratch postgres database for slice *service* tests,
/// which don't need the session store. Handler tests use [`test_db`], which
/// provisions one.
pub(crate) async fn test_connection() -> Result<(DatabaseConnection, TestDbGuard), Box<dyn Error>> {
    let (admin_url, db_name) = create_scratch_db().await?;
    let (db, _) = provision_or_cleanup(&admin_url, &db_name, false).await?;
    Ok((db, TestDbGuard { admin_url, db_name }))
}

/// A fresh, migrated, scratch postgres database for slice handler tests,
/// plus a session store sharing the same pool.
///
/// Each call creates a uniquely-named database, so parallel tests never
/// collide, and returns a [`TestDbGuard`] that drops it when the test ends.
/// `DATABASE_URL` must point at a server where the current user may create
/// databases (the postgres image's superuser qualifies).
pub(crate) async fn test_db()
-> Result<(DatabaseConnection, PostgresStore, TestDbGuard), Box<dyn Error>> {
    let (admin_url, db_name) = create_scratch_db().await?;
    let (db, store) = provision_or_cleanup(&admin_url, &db_name, true).await?;
    let store = store.ok_or_else(|| std::io::Error::other("session store not provisioned"))?;
    Ok((db, store, TestDbGuard { admin_url, db_name }))
}

/// Create a uniquely-named scratch database, returning its name plus the
/// admin connection URL used to create (and later drop) it.
async fn create_scratch_db() -> Result<(String, String), Box<dyn Error>> {
    let admin_url = std::env::var("DATABASE_URL").map_err(|_| {
        std::io::Error::other("DATABASE_URL must be set: tests create a scratch postgres database")
    })?;
    let db_name = unique_db_name();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect(&admin_url)
        .await?;
    let created = sqlx::raw_sql(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(&admin)
        .await;
    admin.close().await;
    created?;
    Ok((admin_url, db_name))
}

/// Connect to `db_name` and run the app migrations against it, plus the
/// session-store migration when `with_session_store` is set.
async fn provision(
    admin_url: &str,
    db_name: &str,
    with_session_store: bool,
) -> Result<(DatabaseConnection, Option<PostgresStore>), Box<dyn Error>> {
    let opts = PgConnectOptions::from_str(admin_url)?.database(db_name);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect_with(opts)
        .await?;

    let db: DatabaseConnection = pool.clone().into();
    Migrator::up(&db, None).await?;

    let store = if with_session_store {
        let store = PostgresStore::new(pool);
        store.migrate().await?;
        Some(store)
    } else {
        None
    };

    Ok((db, store))
}

/// [`provision`] with a safety net: a failed setup drops the scratch database
/// again, so an error between `CREATE DATABASE` and the guard's construction
/// never orphans it.
async fn provision_or_cleanup(
    admin_url: &str,
    db_name: &str,
    with_session_store: bool,
) -> Result<(DatabaseConnection, Option<PostgresStore>), Box<dyn Error>> {
    match provision(admin_url, db_name, with_session_store).await {
        Ok(pair) => Ok(pair),
        Err(err) => {
            if let Err(cleanup_err) = drop_database(admin_url, db_name).await {
                eprintln!("warning: failed to clean up test database {db_name}: {cleanup_err}");
            }
            Err(err)
        }
    }
}

/// Drop `db_name` with `FORCE`, terminating any connections the test's pool
/// left open. Used both for teardown and to clean up after a failed setup.
async fn drop_database(admin_url: &str, db_name: &str) -> Result<(), String> {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect(admin_url)
        .await
        .map_err(|err| format!("connect: {err}"))?;
    let dropped = sqlx::raw_sql(&format!("DROP DATABASE \"{db_name}\" WITH (FORCE)"))
        .execute(&admin)
        .await;
    admin.close().await;
    dropped.map(|_| ()).map_err(|err| format!("drop: {err}"))
}

/// A scratch database created by [`test_db`] (or [`test_connection`]), dropped
/// when this guard is dropped at the end of the test.
///
/// Cleanup runs after the `db`/`store` locals — locals drop in reverse
/// binding order, so the guard goes first — and the test's pool connections
/// may still be alive, which is why the drop needs `FORCE`. A failed drop
/// fails the test, so leaks never go unnoticed; when the test is already
/// failing it only logs a warning (a second panic would abort the process).
pub(crate) struct TestDbGuard {
    admin_url: String,
    db_name: String,
}

impl Drop for TestDbGuard {
    fn drop(&mut self) {
        // `Drop` runs inside the test's own runtime, where building another
        // runtime would panic, so cleanup runs on a fresh thread with its
        // own runtime.
        let admin_url = self.admin_url.clone();
        let db_name = self.db_name.clone();
        let drop_db_name = db_name.clone();
        let handle = std::thread::spawn(move || {
            let built = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match built {
                Ok(rt) => rt.block_on(drop_database(&admin_url, &drop_db_name)),
                Err(err) => Err(format!("build cleanup runtime: {err}")),
            }
        });
        let result = handle
            .join()
            .unwrap_or_else(|_| Err("cleanup thread panicked".to_string()));
        if let Err(msg) = result {
            if std::thread::panicking() {
                eprintln!("warning: test database {db_name} leaked: {msg}");
            } else {
                panic!("test database {db_name} leaked: {msg}");
            }
        }
    }
}

/// A unique, postgres-identifier-safe database name: timestamps alone can
/// collide when two tests start in the same nanosecond, so a per-process
/// counter is mixed in, and the pid keeps concurrent test runs apart.
fn unique_db_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("rustweb_test_{}_{nanos:x}_{seq:x}", std::process::id())
}

/// Insert a user directly, bypassing the business layer, so slice tests stay
/// independent of one another. The placeholder hash can never be logged in
/// with; tests that need a real credential use [`register_and_login`].
pub(crate) async fn insert_user(
    db: &DatabaseConnection,
    name: &str,
    email: &str,
) -> Result<user::Model, DbErr> {
    user::ActiveModel {
        name: Set(name.to_string()),
        email: Set(email.to_string()),
        password_hash: Set("not-a-real-hash".to_string()),
        ..Default::default()
    }
    .insert(db)
    .await
}

/// Insert a post directly, bypassing the business layer, so slice tests stay
/// independent of one another.
pub(crate) async fn insert_post(
    db: &DatabaseConnection,
    user_id: i32,
    title: &str,
    body: &str,
) -> Result<post::Model, DbErr> {
    post::ActiveModel {
        user_id: Set(user_id),
        title: Set(title.to_string()),
        body: Set(body.to_string()),
        ..Default::default()
    }
    .insert(db)
    .await
}

/// Register a user through the business layer, so the stored hash is a real
/// argon2 hash.
pub(crate) async fn register_user_direct(
    db: &DatabaseConnection,
    name: &str,
    email: &str,
    password: &str,
) -> Result<user::Model, Box<dyn Error>> {
    Ok(register_user(
        db,
        name.to_string(),
        email.to_string(),
        password.to_string(),
    )
    .await?)
}

/// A freshly registered and logged-in user: the router to drive, the session
/// cookie to echo back, and the id of the registered user.
pub(crate) struct TestLogin {
    pub(crate) app: axum::Router<()>,
    pub(crate) jar: String,
    pub(crate) user_id: i32,
}

/// The config used by slice tests: generous auth rate limits so unrelated
/// tests never trip the limiter, a long-enough reset TTL, TLS off.
pub(crate) fn test_config() -> Config {
    Config {
        database_url: "unused in tests".to_string(),
        port: 3000,
        cookie_secure: false,
        auth_rate_limit_per_second: 1_000,
        auth_rate_limit_burst: 1_000,
        password_reset_token_ttl: chrono::Duration::minutes(60),
        public_base_url: "http://test.invalid".to_string(),
    }
}

/// A mailer that records every message, so tests can inspect (or replay) what
/// would have been sent.
pub(crate) struct TestMailer {
    sent: Mutex<Vec<SentMail>>,
}

/// One recorded password-reset email.
#[derive(Clone, Debug)]
pub(crate) struct SentMail {
    pub(crate) to: String,
    pub(crate) token: String,
}

impl TestMailer {
    pub(crate) fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
        }
    }

    /// A snapshot of everything sent so far.
    pub(crate) fn sent(&self) -> Vec<SentMail> {
        let Ok(guard) = self.sent.lock() else {
            return Vec::new();
        };
        guard.clone()
    }
}

impl Mailer for TestMailer {
    fn send_password_reset(&self, to: &str, token: &str) -> Result<(), MailerError> {
        let Ok(mut guard) = self.sent.lock() else {
            return Ok(());
        };
        guard.push(SentMail {
            to: to.to_string(),
            token: token.to_string(),
        });
        Ok(())
    }
}

/// The full HTTP router over a fresh scratch database, ready for
/// [`tower::ServiceExt::oneshot`] requests, with a default test config and a
/// fresh [`TestMailer`] (discarded).
pub(crate) fn app_router(db: DatabaseConnection, store: PostgresStore) -> axum::Router<()> {
    let mailer: Arc<dyn Mailer> = Arc::new(TestMailer::new());
    app_router_with(db, store, test_config(), mailer)
}

/// [`app_router`] with a specific config and mailer (e.g. tight rate limits
/// or a mailer whose messages the test wants to read).
pub(crate) fn app_router_with(
    db: DatabaseConnection,
    store: PostgresStore,
    config: Config,
    mailer: Arc<dyn Mailer>,
) -> axum::Router<()> {
    crate::router::app_router(AppState { db, mailer, config }, store)
}

/// [`app_router`] with a specific mailer, default config otherwise.
pub(crate) fn app_router_with_mailer(
    db: DatabaseConnection,
    store: PostgresStore,
    mailer: Arc<dyn Mailer>,
) -> axum::Router<()> {
    app_router_with(db, store, test_config(), mailer)
}

/// Register a user through the HTTP layer and capture the session cookie,
/// returning the router, the cookie jar, and the registered user's id.
pub(crate) async fn register_and_login(
    db: &DatabaseConnection,
    store: &PostgresStore,
    name: &str,
    email: &str,
    password: &str,
) -> Result<TestLogin, Box<dyn Error>> {
    let app = app_router(db.clone(), store.clone());
    let body = Body::from(
        serde_json::json!({
            "name": name,
            "email": email,
            "password": password,
        })
        .to_string(),
    );

    let res = request(
        app.clone(),
        Method::POST,
        "/auth/register",
        Some(body),
        None,
    )
    .await?;
    assert_eq!(res.status(), axum::http::StatusCode::CREATED);
    let jar = session_cookie(&res)
        .ok_or_else(|| std::io::Error::other("register response has no session cookie"))?;
    let created: UserResponse = json_body(res).await?;
    Ok(TestLogin {
        app,
        jar,
        user_id: created.id,
    })
}

/// The `Set-Cookie` header value for the session cookie, truncated at the
/// first attribute so it can be echoed back in a `Cookie` header.
fn session_cookie(res: &Response) -> Option<String> {
    let set = res.headers().get_all("set-cookie").iter().next()?;
    let value = set.to_str().ok()?;
    value.split(';').next().map(str::trim).map(str::to_string)
}

/// Drive the router with a single request, no network involved. `cookie` is
/// the raw value of a `Cookie` header (e.g. `"id=abc"`) to send, or `None`.
/// The request pretends to come from [`TEST_PEER`], which the rate limiter
/// keys on.
pub(crate) async fn request(
    app: axum::Router<()>,
    method: Method,
    uri: &str,
    body: Option<Body>,
    cookie: Option<&str>,
) -> Result<Response, Box<dyn Error>> {
    request_from(app, method, uri, body, cookie, TEST_PEER).await
}

/// [`request`] with an explicit peer address, so rate-limit tests can speak
/// as several clients.
pub(crate) async fn request_from(
    app: axum::Router<()>,
    method: Method,
    uri: &str,
    body: Option<Body>,
    cookie: Option<&str>,
    peer: SocketAddr,
) -> Result<Response, Box<dyn Error>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(peer));
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let req = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(body)?,
        None => builder.body(Body::empty())?,
    };
    match tower::ServiceExt::oneshot(app, req).await {
        Ok(res) => Ok(res),
        Err(never) => match never {},
    }
}

/// Deserialize a response body as JSON.
pub(crate) async fn json_body<T: serde::de::DeserializeOwned>(
    res: Response,
) -> Result<T, Box<dyn Error>> {
    let bytes = to_bytes(res.into_body(), 4 * 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::TestDbGuard;

    /// A failed teardown must fail the test, or scratch databases would leak
    /// silently forever. A guard pointed at an unreachable admin server
    /// exercises exactly that path (the expected panic *is* the behavior).
    #[tokio::test]
    #[should_panic(expected = "leaked")]
    async fn guard_fails_the_test_when_cleanup_fails() {
        let guard = TestDbGuard {
            admin_url: "postgres://nobody:nothing@127.0.0.1:1/none".to_string(),
            db_name: "rustweb_test_leak_probe".to_string(),
        };
        drop(guard);
    }
}
