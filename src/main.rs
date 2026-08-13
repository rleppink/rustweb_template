mod auth;
mod config;
mod entities;
mod error;
mod features;
mod mail;
mod router;
mod service_error;
mod state;

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use migration::{Migrator, MigratorTrait};
use sea_orm::DatabaseConnection;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower_http::LatencyUnit;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tower_sessions_sqlx_store::PostgresStore;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::mail::{LogMailer, Mailer};
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustweb=debug,tower_http=info".into()),
        )
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => panic!("invalid configuration: {err}"),
    };
    let port = config.port;

    // One Postgres pool serves both SeaORM and the session store, so sessions
    // and app data live in the same database.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(PgConnectOptions::from_str(&config.database_url)?)
        .await?;

    let db: DatabaseConnection = pool.clone().into();
    Migrator::up(&db, None).await?;

    let session_store = PostgresStore::new(pool.clone());
    session_store.migrate().await?;

    // The LogMailer prints a ready-to-paste reset curl to the logs instead of
    // sending mail. Replace it with a real transport before real users arrive.
    let mailer: Arc<dyn Mailer> = Arc::new(LogMailer::new(config.public_base_url.clone()));
    let state = AppState { db, mailer, config };

    let app = router::app_router(state, session_store).layer(
        TraceLayer::new_for_http()
            .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
            .on_response(
                DefaultOnResponse::new()
                    .level(Level::INFO)
                    .latency_unit(LatencyUnit::Millis),
            ),
    );

    // `ConnectInfo` is what the rate limiter keys on; without
    // `into_make_service_with_connect_info` every request would look peerless
    // to the governor and be rejected.
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!("listening on http://{addr}");
    tracing::info!("spec at     http://{addr}/openapi.json");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // In-flight requests are drained by the graceful shutdown; the pool can
    // close only after they finish.
    pool.close().await;
    Ok(())
}

/// Completes when SIGINT (Ctrl-C) or SIGTERM arrives, the signals container
/// orchestrators use to ask for a graceful stop. A failed signal handler is
/// logged and treated as "shut down now" — the alternative (hanging forever
/// with no way to stop) is worse.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install SIGINT handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        let mut signal =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    tracing::error!(error = %err, "failed to install SIGTERM handler");
                    return;
                }
            };
        signal.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received SIGINT, shutting down"),
        () = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}
