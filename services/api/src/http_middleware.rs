//! Cross-cutting HTTP middleware: request correlation and rate limiting.

use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method},
    middleware::Next,
    response::Response,
};
use probe_forge_observability::RequestId;
use tracing::Instrument;

use crate::{AppState, auth::AuthContext, error::ApiError, rate_limit as rate_limits};

/// Rate-limit scopes. `expensive` covers endpoints that trigger model work or
/// report generation; `api` covers everything else.
const SCOPE_API: &str = "api";
const SCOPE_EXPENSIVE: &str = "expensive";
const RATE_WINDOW_SECONDS: i64 = 60;

/// Attach a correlation ID to every request and echo it back.
///
/// A client-supplied `X-Request-Id` is honored only when it is printable ASCII
/// of bounded length; otherwise one is generated. The ID is inserted into
/// request extensions and carried on a tracing span so every log line for the
/// request includes it.
pub async fn request_id(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(RequestId::parse)
        .unwrap_or_else(RequestId::generate);

    request.extensions_mut().insert(id.clone());
    let span = tracing::info_span!("http_request", request_id = %id);
    let mut response = next.run(request).instrument(span).await;

    if let Ok(value) = HeaderValue::from_str(id.as_str()) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

/// Enforce per-user API and expensive-operation rate limits.
///
/// Runs after authentication, so it reads the resolved user from the request
/// extensions. Anonymous requests never reach the protected routes.
pub async fn rate_limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(context) = request.extensions().get::<AuthContext>() {
        let expensive = is_expensive_request(request.method(), request.uri().path());
        let (scope, limit) = if expensive {
            (SCOPE_EXPENSIVE, state.config.expensive_requests_per_minute)
        } else {
            (SCOPE_API, state.config.api_requests_per_minute)
        };
        rate_limits::enforce(
            &state.pool,
            scope,
            &context.user.id.to_string(),
            limit,
            RATE_WINDOW_SECONDS,
        )
        .await?;
    }
    Ok(next.run(request).await)
}

/// Endpoints that trigger model work or report generation. Matched against the
/// path with any `/api` nest prefix removed.
fn is_expensive_request(method: &Method, path: &str) -> bool {
    if method != Method::POST {
        return false;
    }
    let relative = path.strip_prefix("/api").unwrap_or(path);
    relative == "/jobs" || relative.ends_with("/report")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_expensive_requests() {
        assert!(is_expensive_request(&Method::POST, "/api/jobs"));
        assert!(is_expensive_request(&Method::POST, "/jobs"));
        assert!(is_expensive_request(&Method::POST, "/api/runs/abc/report"));
        assert!(!is_expensive_request(&Method::GET, "/api/jobs"));
        assert!(!is_expensive_request(&Method::POST, "/api/projects"));
    }
}
