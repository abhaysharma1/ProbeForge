pub mod audit;
pub mod auth;
pub mod catalog;
pub mod cookies;
pub mod error;
pub mod jobs;
pub mod plans;
pub mod projects;

use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next, map_response},
    response::Response,
    routing::{get, post},
};
use serde::Serialize;
use sqlx::{PgPool, postgres::PgPoolOptions};
use thiserror::Error;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::info;
use url::Url;

pub const SERVICE_NAME: &str = "probe-forge-api";

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;
const MIN_SESSION_TTL_SECONDS: i64 = 5 * 60;
const MAX_SESSION_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
const LOGIN_WINDOW_SECONDS: i64 = 15 * 60;
const LOGIN_ACCOUNT_ATTEMPT_LIMIT: i32 = 5;
const LOGIN_IP_ATTEMPT_LIMIT: i32 = 50;
const DEFAULT_ALLOWED_ORIGINS: &str = "http://127.0.0.1:5173,http://localhost:5173";
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DB_MAX_CONNECTIONS: u32 = 10;
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub cookie_secure: bool,
    pub session_ttl_seconds: i64,
    pub allowed_origins: Vec<String>,
    pub login_window_seconds: i64,
    pub login_attempt_limit: i32,
    pub login_ip_attempt_limit: i32,
}

impl AuthConfig {
    pub fn local_test() -> Self {
        Self {
            cookie_secure: false,
            session_ttl_seconds: DEFAULT_SESSION_TTL_SECONDS,
            allowed_origins: vec!["http://127.0.0.1:5173".to_owned()],
            login_window_seconds: LOGIN_WINDOW_SECONDS,
            login_attempt_limit: LOGIN_ACCOUNT_ATTEMPT_LIMIT,
            login_ip_attempt_limit: LOGIN_IP_ATTEMPT_LIMIT,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("database_url", &"<redacted>")
            .field("auth", &self.auth)
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
    #[error("environment variable {name} must be true or false: {value}")]
    InvalidBoolean { name: &'static str, value: String },
    #[error("environment variable {name} must be an integer: {value}")]
    InvalidInteger { name: &'static str, value: String },
    #[error("environment variable {name} is outside the allowed range")]
    IntegerOutOfRange { name: &'static str },
    #[error("environment variable {name} contains an invalid origin: {value}")]
    InvalidOrigin { name: &'static str, value: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_value =
            std::env::var("PROBE_FORGE_API_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = parse_bind_addr("PROBE_FORGE_API_ADDR", &bind_value)?;

        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| ConfigError::MissingDatabaseUrl)?;
        if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
            return Err(ConfigError::InvalidDatabaseUrl);
        }

        let cookie_secure = parse_bool("PROBE_FORGE_COOKIE_SECURE", true)?;
        let session_ttl_seconds = parse_i64(
            "PROBE_FORGE_SESSION_TTL_SECONDS",
            DEFAULT_SESSION_TTL_SECONDS,
            MIN_SESSION_TTL_SECONDS,
            MAX_SESSION_TTL_SECONDS,
        )?;
        let origins_value = std::env::var("PROBE_FORGE_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| DEFAULT_ALLOWED_ORIGINS.to_owned());
        let allowed_origins = parse_allowed_origins(&origins_value)?;

        Ok(Self {
            bind_addr,
            database_url,
            auth: AuthConfig {
                cookie_secure,
                session_ttl_seconds,
                allowed_origins,
                login_window_seconds: LOGIN_WINDOW_SECONDS,
                login_attempt_limit: LOGIN_ACCOUNT_ATTEMPT_LIMIT,
                login_ip_attempt_limit: LOGIN_IP_ATTEMPT_LIMIT,
            },
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<AuthConfig>,
}

impl AppState {
    pub fn new(pool: PgPool, config: AuthConfig) -> Self {
        Self {
            pool,
            config: Arc::new(config),
        }
    }
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
    let protected = Router::new()
        .route("/auth/session", get(auth::current_session))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/logout-all", post(auth::logout_all))
        .route("/auth/sessions", get(auth::list_sessions))
        .route(
            "/auth/sessions/{session_id}",
            axum::routing::delete(auth::revoke_session),
        )
        .merge(projects::routes())
        .merge(jobs::routes())
        .merge(plans::routes())
        // Authentication runs before CSRF so an anonymous unsafe request gets a
        // consistent 401 rather than a CSRF 403. Authenticated unsafe requests
        // still pass through the CSRF guard.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::csrf_guard,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let public = Router::new()
        .route("/auth/csrf", get(auth::csrf))
        .route("/auth/login", post(auth::login))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::csrf_guard,
        ));

    let api = Router::new()
        .merge(public)
        .merge(protected)
        .layer(map_response(api_no_store));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readiness))
        .nest("/api", api)
        .with_state(state)
        .layer(middleware::from_fn(request_timeout))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
}

/// Apply the normal request budget, but leave SSE streams open. A fixed
/// `TimeoutLayer` would terminate every event stream at 30 seconds.
async fn request_timeout(
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.uri().path().ends_with("/events") {
        return Ok(next.run(request).await);
    }
    tokio::time::timeout(REQUEST_TIMEOUT, next.run(request))
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)
}

/// Authenticated API responses must not be cached by browsers or intermediaries.
async fn api_no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: SERVICE_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<Readiness>) {
    let migrated: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success LIMIT 1)")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(false);

    if migrated {
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

#[derive(Debug, Error)]
pub enum ServeError {
    #[error("database connection failed; verify DATABASE_URL and server availability")]
    Database,
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(addr = %config.bind_addr, service = SERVICE_NAME, "service listening");
    let state = AppState::new(pool, config.auth);
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
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

fn parse_bind_addr(name: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value
        .trim()
        .parse()
        .map_err(|_| ConfigError::InvalidSocketAddress {
            name,
            value: value.to_owned(),
        })
}

fn parse_bool(name: &'static str, default: bool) -> Result<bool, ConfigError> {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(ConfigError::InvalidBoolean { name, value }),
        },
        Err(_) => Ok(default),
    }
}

fn parse_i64(name: &'static str, default: i64, min: i64, max: i64) -> Result<i64, ConfigError> {
    let value = match std::env::var(name) {
        Ok(value) => value
            .trim()
            .parse::<i64>()
            .map_err(|_| ConfigError::InvalidInteger {
                name,
                value: value.clone(),
            })?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        return Err(ConfigError::IntegerOutOfRange { name });
    }
    Ok(value)
}

fn parse_allowed_origins(value: &str) -> Result<Vec<String>, ConfigError> {
    let mut origins = Vec::new();
    for raw in value.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let parsed = Url::parse(raw).map_err(|_| ConfigError::InvalidOrigin {
            name: "PROBE_FORGE_ALLOWED_ORIGINS",
            value: raw.to_owned(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
        {
            return Err(ConfigError::InvalidOrigin {
                name: "PROBE_FORGE_ALLOWED_ORIGINS",
                value: raw.to_owned(),
            });
        }
        origins.push(parsed.origin().ascii_serialization());
    }
    if origins.is_empty() {
        return Err(ConfigError::InvalidOrigin {
            name: "PROBE_FORGE_ALLOWED_ORIGINS",
            value: value.to_owned(),
        });
    }
    Ok(origins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        response::Response,
    };
    use sqlx::postgres::PgPoolOptions;
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

    fn test_state() -> AppState {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(150))
            .connect_lazy("postgres://probeforge:test@127.0.0.1:1/probeforge")
            .expect("lazy pool");
        AppState::new(pool, AuthConfig::local_test())
    }

    #[tokio::test]
    async fn healthz_reports_ok() {
        let response = router(test_state())
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
        let response = router(test_state())
            .oneshot(request("/readyz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = json_body(response).await;
        assert_eq!(body["status"], "not_ready");
    }

    #[tokio::test]
    async fn unknown_route_is_not_found() {
        let response = router(test_state())
            .oneshot(request("/missing"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn project_routes_require_authentication() {
        let response = router(test_state())
            .oneshot(request("/api/projects"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn parses_loopback_address() {
        let config = parse_bind_addr("PROBE_FORGE_API_ADDR", "127.0.0.1:8080").expect("valid");
        assert_eq!(config.port(), 8080);
        assert!(config.ip().is_loopback());
    }

    #[test]
    fn rejects_invalid_address() {
        let error = parse_bind_addr("PROBE_FORGE_API_ADDR", "not-an-address").expect_err("invalid");
        assert!(error.to_string().contains("PROBE_FORGE_API_ADDR"));
    }

    #[test]
    fn normalizes_allowed_origins_and_rejects_paths() {
        let origins = parse_allowed_origins("http://127.0.0.1:5173, https://probe.example.test/")
            .expect("origins");
        assert_eq!(
            origins,
            vec![
                "http://127.0.0.1:5173".to_owned(),
                "https://probe.example.test".to_owned()
            ]
        );
        assert!(parse_allowed_origins("https://probe.example.test/app").is_err());
        assert!(parse_allowed_origins("javascript:alert(1)").is_err());
        assert!(parse_allowed_origins("").is_err());
    }
}
