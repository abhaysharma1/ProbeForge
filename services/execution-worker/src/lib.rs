use std::{net::SocketAddr, time::Duration};

use axum::{Json, Router, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use thiserror::Error;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::{info, warn};

pub const SERVICE_NAME: &str = "probe-forge-execution-worker";

/// The constrained load-runner library is owned by the execution worker. The
/// HTTP service stays health-only; queued runs are executed by the operator
/// `dispatch` binary, by the background loop when enabled, or by calling
/// `dispatch_once` directly.
pub use probe_forge_load_runner as load_runner;

pub mod dispatch;
pub mod snapshot;

pub use dispatch::{DispatchConfig, DispatchError, DispatchedRun, dispatch_once};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8082";
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DISPATCH_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DB_MAX_CONNECTIONS: u32 = 5;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    /// Set when `PROBE_FORGE_EXECUTION_DISPATCH_LOOP=true`; enables a background
    /// poll loop that claims and executes queued runs.
    pub dispatch_loop: bool,
    pub database_url: Option<String>,
    pub allow_private_targets: bool,
    pub worker_id: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("environment variable {name} is not a valid socket address: {value}")]
    InvalidSocketAddress { name: &'static str, value: String },
    #[error("environment variable {name} must be true or false: {value}")]
    InvalidBoolean { name: &'static str, value: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let value = std::env::var("PROBE_FORGE_EXECUTION_WORKER_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr = parse_bind_addr("PROBE_FORGE_EXECUTION_WORKER_ADDR", &value)?;
        let dispatch_loop = parse_bool("PROBE_FORGE_EXECUTION_DISPATCH_LOOP", false)?;
        let allow_private_targets = parse_bool("PROBE_FORGE_ALLOW_PRIVATE_TARGETS", false)?;
        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let worker_id = std::env::var("PROBE_FORGE_EXECUTION_WORKER_ID")
            .unwrap_or_else(|_| "execution-worker-1".to_owned());
        Ok(Self {
            bind_addr,
            dispatch_loop,
            database_url,
            allow_private_targets,
            worker_id,
        })
    }

    pub fn parse(name: &'static str, value: &str) -> Result<Self, ConfigError> {
        Ok(Self {
            bind_addr: parse_bind_addr(name, value)?,
            dispatch_loop: false,
            database_url: None,
            allow_private_targets: false,
            worker_id: "execution-worker-1".to_owned(),
        })
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
    tools: ToolHealth,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolHealth {
    git: &'static str,
    tls_roots: &'static str,
}

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readiness))
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

async fn readiness() -> Json<Readiness> {
    // A missing optional tool degrades static/TLS coverage; it does not stop
    // HTTP-only dispatch, so the probe stays 200 with an explicit tool status.
    let git = if crate::snapshot::git_available().await {
        "available"
    } else {
        "unavailable"
    };
    let tls_roots = if probe_forge_execution_adapters::tls_root_store_available() {
        "available"
    } else {
        "unavailable"
    };
    let status = if git == "available" && tls_roots == "available" {
        "ready"
    } else {
        "degraded"
    };
    Json(Readiness {
        status,
        service: SERVICE_NAME,
        tools: ToolHealth { git, tls_roots },
    })
}

pub async fn serve(config: Config) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(addr = %config.bind_addr, service = SERVICE_NAME, "service listening");

    let dispatch_task = if config.dispatch_loop {
        let database_url = config.database_url.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "PROBE_FORGE_EXECUTION_DISPATCH_LOOP requires DATABASE_URL",
            )
        })?;
        let pool = PgPoolOptions::new()
            .max_connections(DB_MAX_CONNECTIONS)
            .connect(&database_url)
            .await
            .map_err(|error| {
                std::io::Error::other(format!("dispatch database connection failed: {error}"))
            })?;
        let worker_id = config.worker_id.clone();
        let allow_private_targets = config.allow_private_targets;
        info!(service = SERVICE_NAME, "dispatch loop started");
        Some(tokio::spawn(async move {
            run_dispatch_loop(pool, worker_id, allow_private_targets).await;
        }))
    } else {
        None
    };

    let result = axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Some(task) = dispatch_task {
        task.abort();
    }
    result
}

async fn run_dispatch_loop(pool: sqlx::PgPool, worker_id: String, allow_private_targets: bool) {
    let mut config = DispatchConfig::new(worker_id);
    config.allow_private_targets = allow_private_targets;
    loop {
        // The platform default concurrency cap is admin-configurable and applied
        // live; a per-project quota row still overrides it at claim time.
        if let Ok(Some(value)) = sqlx::query_scalar::<_, i32>(
            "SELECT default_max_active_runs FROM platform_settings WHERE id = true",
        )
        .fetch_optional(&pool)
        .await
        {
            config.max_active_per_project = i64::from(value);
        }
        match dispatch_once(&pool, &config).await {
            Ok(Some(run)) => info!(
                event = "run.dispatched",
                run_id = %run.run_id,
                run_type = %run.run_type,
                status = run.status,
                worker_id = %config.worker_id,
                "dispatched run"
            ),
            Ok(None) => {}
            Err(error) => warn!(
                event = "run.dispatch_failed",
                error = %error,
                worker_id = %config.worker_id,
                "dispatch attempt failed"
            ),
        }
        tokio::time::sleep(DISPATCH_POLL_INTERVAL).await;
    }
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

    #[tokio::test]
    async fn healthz_reports_ok() {
        let response = router()
            .oneshot(request("/healthz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], SERVICE_NAME);
    }

    #[tokio::test]
    async fn readyz_reports_ready() {
        let response = router()
            .oneshot(request("/readyz"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert!(matches!(
            body["status"].as_str(),
            Some("ready" | "degraded")
        ));
        assert_eq!(body["service"], SERVICE_NAME);
        assert!(body["tools"]["git"].is_string());
        assert!(body["tools"]["tlsRoots"].is_string());
    }

    #[tokio::test]
    async fn unknown_route_is_not_found() {
        let response = router()
            .oneshot(request("/missing"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn parses_loopback_address() {
        let config =
            Config::parse("PROBE_FORGE_EXECUTION_WORKER_ADDR", "127.0.0.1:8082").expect("valid");
        assert_eq!(config.bind_addr.port(), 8082);
        assert!(config.bind_addr.ip().is_loopback());
    }

    #[test]
    fn rejects_invalid_address() {
        let error = Config::parse("PROBE_FORGE_EXECUTION_WORKER_ADDR", "not-an-address")
            .expect_err("invalid");
        assert!(
            error
                .to_string()
                .contains("PROBE_FORGE_EXECUTION_WORKER_ADDR")
        );
    }
}
