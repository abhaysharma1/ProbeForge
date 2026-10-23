use std::{fmt, net::SocketAddr, time::Duration};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::{PgPool, postgres::PgPoolOptions};
use thiserror::Error;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::info;

pub const SERVICE_NAME: &str = "probe-forge-scheduler";

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8081";
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;
const MIN_POLL_INTERVAL_SECONDS: u64 = 1;
const MAX_POLL_INTERVAL_SECONDS: u64 = 300;
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DB_MAX_CONNECTIONS: u32 = 5;
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub poll_interval_seconds: u64,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("database_url", &"<redacted>")
            .field("poll_interval_seconds", &self.poll_interval_seconds)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("environment variable {name} is not a valid socket address: {value}")]
    InvalidSocketAddress { name: &'static str, value: String },
    #[error("DATABASE_URL must be set")]
    MissingDatabaseUrl,
    #[error("DATABASE_URL must use the postgres:// or postgresql:// scheme")]
    InvalidDatabaseUrl,
    #[error("environment variable {name} must be an integer: {value}")]
    InvalidInteger { name: &'static str, value: String },
    #[error("environment variable {name} is outside the allowed range")]
    IntegerOutOfRange { name: &'static str },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_value = std::env::var("PROBE_FORGE_SCHEDULER_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = parse_bind_addr("PROBE_FORGE_SCHEDULER_ADDR", &bind_value)?;
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| ConfigError::MissingDatabaseUrl)?;
        if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
            return Err(ConfigError::InvalidDatabaseUrl);
        }
        let poll_interval_seconds = parse_interval()?;
        Ok(Self {
            bind_addr,
            database_url,
            poll_interval_seconds,
        })
    }

    /// Explicit constructor for tests and future embedding.
    pub fn new(bind_addr: SocketAddr, database_url: impl Into<String>) -> Self {
        Self {
            bind_addr,
            database_url: database_url.into(),
            poll_interval_seconds: DEFAULT_POLL_INTERVAL_SECONDS,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    /// `None` is used by unit tests that exercise liveness without a database.
    /// Production startup always supplies a connected pool.
    pub pool: Option<PgPool>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self { pool: Some(pool) }
    }
}

#[derive(Debug, Error)]
pub enum ServeError {
    #[error("database connection failed; verify DATABASE_URL and server availability")]
    Database,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct Readiness {
    status: &'static str,
    service: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readiness))
        .with_state(state)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: SERVICE_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<Readiness>) {
    let ready = match &state.pool {
        Some(pool) => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success LIMIT 1)",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(false),
        None => false,
    };

    if ready {
        (
            StatusCode::OK,
            Json(Readiness {
                status: "ready",
                service: SERVICE_NAME,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(Readiness {
                status: "not_ready",
                service: SERVICE_NAME,
            }),
        )
    }
}

pub async fn serve(config: Config) -> Result<(), ServeError> {
    let pool = PgPoolOptions::new()
        .max_connections(DB_MAX_CONNECTIONS)
        .acquire_timeout(DB_ACQUIRE_TIMEOUT)
        .connect(&config.database_url)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "database connection failed");
            ServeError::Database
        })?;

    let state = AppState::new(pool.clone());
    let loop_pool = pool.clone();
    let interval = Duration::from_secs(config.poll_interval_seconds);
    let scheduler = tokio::spawn(async move {
        scheduler_loop(loop_pool, interval).await;
    });

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(
        addr = %config.bind_addr,
        interval_seconds = config.poll_interval_seconds,
        service = SERVICE_NAME,
        "service listening"
    );
    let result = axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await;
    scheduler.abort();
    result?;
    Ok(())
}

async fn scheduler_loop(pool: PgPool, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;

        match probe_forge_queue::promote_due_runs(&pool, 50).await {
            Ok(runs) if !runs.is_empty() => {
                tracing::info!(count = runs.len(), "promoted due scheduled runs");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error, "due-run promotion failed"),
        }

        match probe_forge_queue::recover_expired_job_leases(&pool, 50).await {
            Ok(jobs) if !jobs.is_empty() => {
                tracing::info!(count = jobs.len(), "recovered expired job leases");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error, "expired job lease recovery failed"),
        }
    }
}

fn parse_bind_addr(name: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value
        .trim()
        .parse()
        .map_err(|_| ConfigError::InvalidSocketAddress {
            name,
            value: value.to_owned(),
        })
}

fn parse_interval() -> Result<u64, ConfigError> {
    let raw = match std::env::var("PROBE_FORGE_SCHEDULER_POLL_SECONDS") {
        Ok(value) => value,
        Err(_) => return Ok(DEFAULT_POLL_INTERVAL_SECONDS),
    };
    let value = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| ConfigError::InvalidInteger {
            name: "PROBE_FORGE_SCHEDULER_POLL_SECONDS",
            value: raw.clone(),
        })?;
    if !(MIN_POLL_INTERVAL_SECONDS..=MAX_POLL_INTERVAL_SECONDS).contains(&value) {
        return Err(ConfigError::IntegerOutOfRange {
            name: "PROBE_FORGE_SCHEDULER_POLL_SECONDS",
        });
    }
    Ok(value)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!(service = SERVICE_NAME, "shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        response::Response,
    };
    use tower::ServiceExt;

    async fn json_body(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 4096)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    fn request(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request")
    }

    fn test_router() -> Router {
        router(AppState { pool: None })
    }

    #[tokio::test]
    async fn healthz_reports_ok() {
        let response = test_router()
            .oneshot(request("/healthz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], SERVICE_NAME);
    }

    #[tokio::test]
    async fn readyz_fails_closed_without_database() {
        let response = test_router()
            .oneshot(request("/readyz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = json_body(response).await;
        assert_eq!(body["status"], "not_ready");
        assert_eq!(body["service"], SERVICE_NAME);
    }

    #[tokio::test]
    async fn unknown_route_is_not_found() {
        let response = test_router()
            .oneshot(request("/missing"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn parses_loopback_address() {
        let addr = parse_bind_addr("PROBE_FORGE_SCHEDULER_ADDR", "127.0.0.1:8081")
            .expect("valid socket address");
        let config = Config::new(addr, "postgres://localhost/probeforge");
        assert_eq!(config.bind_addr.port(), 8081);
        assert!(config.bind_addr.ip().is_loopback());
        assert!(format!("{config:?}").contains("<redacted>"));
    }

    #[test]
    fn rejects_invalid_address() {
        let error =
            parse_bind_addr("PROBE_FORGE_SCHEDULER_ADDR", "not-an-address").expect_err("invalid");
        assert!(error.to_string().contains("PROBE_FORGE_SCHEDULER_ADDR"));
    }
}
