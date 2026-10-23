use std::{net::SocketAddr, time::Duration};

use axum::{Json, Router, http::StatusCode, routing::get};
use serde::Serialize;
use thiserror::Error;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::info;

pub const SERVICE_NAME: &str = "probe-forge-execution-worker";

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8082";
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub bind_addr: SocketAddr,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("environment variable {name} is not a valid socket address: {value}")]
    InvalidSocketAddress { name: &'static str, value: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let value = std::env::var("PROBE_FORGE_EXECUTION_WORKER_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
        Self::parse("PROBE_FORGE_EXECUTION_WORKER_ADDR", &value)
    }

    pub fn parse(name: &'static str, value: &str) -> Result<Self, ConfigError> {
        let bind_addr = value
            .trim()
            .parse()
            .map_err(|_| ConfigError::InvalidSocketAddress {
                name,
                value: value.to_owned(),
            })?;
        Ok(Self { bind_addr })
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
    // Phases 10 and 12 add adapters and the load runner; readiness will then require them.
    Json(Readiness {
        status: "ready",
        service: SERVICE_NAME,
    })
}

pub async fn serve(config: Config) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(addr = %config.bind_addr, service = SERVICE_NAME, "service listening");
    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await
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
        assert_eq!(body["status"], "ready");
        assert_eq!(body["service"], SERVICE_NAME);
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
