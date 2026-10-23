use std::fmt;

use axum::{
    Json,
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use probeforge_contracts::{Validate, parse_and_validate_json};
use serde::{Deserialize, de::DeserializeOwned};

/// Stable, non-leaking API error envelope.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retry_after_seconds: Option<u64>,
}

#[derive(serde::Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(serde::Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn unauthorized(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message)
    }

    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn rate_limited(retry_after_seconds: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Try again later.",
        );
        error.retry_after_seconds = Some(retry_after_seconds.max(1));
        error
    }

    /// Logs the database error server-side and returns a generic response.
    pub fn database(error: sqlx::Error) -> Self {
        tracing::error!(error = %error, "database operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred.",
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        tracing::error!(message = %message.into(), "internal API error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred.",
        )
    }

    #[cfg(test)]
    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response();

        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(seconds) = self.retry_after_seconds
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

/// JSON extractor that turns every parse failure into the stable error shape.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(json_rejection)?;
        Ok(Self(value))
    }
}

fn json_rejection(rejection: JsonRejection) -> ApiError {
    tracing::debug!(reason = %rejection.body_text(), "request JSON rejected");
    ApiError::invalid("invalid_json", "Request body is not a valid JSON object.")
}

/// Contract-validated JSON body. Enforces the shared 256 KiB contract limit and
/// rejects unknown fields and executable fields through the versioned validator.
pub struct ContractJson<T>(pub T);

impl<T, S> FromRequest<S> for ContractJson<T>
where
    T: DeserializeOwned + Validate + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| ApiError::invalid("invalid_body", "Request body could not be read."))?;
        let value = parse_and_validate_json::<T>(&bytes)
            .map_err(|error| ApiError::invalid("contract_invalid", error.to_string()))?;
        Ok(Self(value))
    }
}

/// Path extractor that hides framework rejection details.
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(value) = axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::invalid("invalid_path", "A path parameter is invalid."))?;
        Ok(Self(value))
    }
}

/// Query extractor that hides framework rejection details.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    pub limit: Option<i64>,
}

pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Query(value) =
            axum::extract::Query::<T>::from_request_parts(parts, state)
                .await
                .map_err(|_| ApiError::invalid("invalid_query", "A query parameter is invalid."))?;
        Ok(Self(value))
    }
}
