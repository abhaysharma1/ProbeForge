pub mod password;
pub mod session;

use std::{convert::Infallible, net::SocketAddr};

use axum::{
    Extension, Json,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AppState, cookies,
    error::{ApiError, ApiJson, ApiPath},
};

const CACHE_CONTROL_NO_STORE: &str = "no-store";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: UserDto,
    pub session_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    user: UserDto,
    csrf_token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    user: UserDto,
    session_id: Uuid,
    csrf_token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CsrfResponse {
    csrf_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    id: Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    current: bool,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    display_name: String,
    password_hash: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SessionAuthRow {
    session_id: Uuid,
    user_id: Uuid,
    email: String,
    display_name: String,
    expires_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SessionListRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

/// Extract the direct peer address. Forwarded headers are deliberately ignored
/// unless a trusted-proxy design is added; otherwise a client could spoof the
/// rate-limit identity.
pub struct ClientIp(String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(Self(ip))
    }
}

pub async fn csrf(State(state): State<AppState>) -> Response {
    let token = session::generate_token();
    let mut headers = HeaderMap::new();
    cookies::append(
        &mut headers,
        cookies::csrf_cookie(
            &token,
            state.config.cookie_secure,
            state.config.session_ttl_seconds,
        ),
    );
    no_store(&mut headers);
    (headers, Json(CsrfResponse { csrf_token: token })).into_response()
}

pub async fn login(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    ApiJson(payload): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let normalized_email = password::normalize_email(&payload.email);
    let email = normalized_email.as_deref().unwrap_or("invalid-email");
    let password = Zeroizing::new(payload.password);
    let account_key = session::rate_key("account", &format!("{email}|{client_ip}"));
    let ip_key = session::rate_key("ip", &client_ip);

    let account_attempt = session::record_attempt(
        &state.pool,
        &account_key,
        state.config.login_attempt_limit,
        state.config.login_window_seconds,
    )
    .await
    .map_err(ApiError::database)?;
    let ip_attempt = session::record_attempt(
        &state.pool,
        &ip_key,
        state.config.login_ip_attempt_limit,
        state.config.login_window_seconds,
    )
    .await
    .map_err(ApiError::database)?;

    let account_retry = (account_attempt.attempts > state.config.login_attempt_limit).then(|| {
        session::retry_after_seconds(
            account_attempt.window_started_at,
            state.config.login_window_seconds,
        )
    });
    let ip_retry = (ip_attempt.attempts > state.config.login_ip_attempt_limit).then(|| {
        session::retry_after_seconds(
            ip_attempt.window_started_at,
            state.config.login_window_seconds,
        )
    });
    if let Some(retry_after) = account_retry.into_iter().chain(ip_retry).max() {
        return Err(ApiError::rate_limited(retry_after));
    }

    let password_shape_valid = password::validate_password(&password).is_ok();
    let user: Option<UserRow> = sqlx::query_as(
        "SELECT id, email, display_name, password_hash
         FROM users
         WHERE email = $1 AND status = 'active'",
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;

    let password_for_check = Zeroizing::new(password.to_string());
    let stored_hash = user.as_ref().and_then(|row| row.password_hash.clone());
    let verified = tokio::task::spawn_blocking(move || {
        if !password_shape_valid {
            return password::dummy_verify(&password_for_check);
        }
        match stored_hash {
            Some(hash) => password::verify_password(&password_for_check, &hash),
            None => password::dummy_verify(&password_for_check),
        }
    })
    .await
    .map_err(|_| ApiError::internal("password verification worker failed"))?;

    let Some(user) = user.filter(|_| verified) else {
        tracing::warn!("login rejected");
        return Err(ApiError::unauthorized(
            "invalid_credentials",
            "Email or password is incorrect.",
        ));
    };

    let session_token = session::generate_token();
    let csrf_token = session::generate_token();
    let expires_at = Utc::now() + Duration::seconds(state.config.session_ttl_seconds);
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO auth_sessions (user_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(user.id)
    .bind(session::hash_token(&session_token))
    .bind(expires_at)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    for key in [&account_key, &ip_key] {
        sqlx::query("DELETE FROM auth_login_attempts WHERE key_hash = $1")
            .bind(key)
            .execute(&mut *transaction)
            .await
            .map_err(ApiError::database)?;
    }

    crate::audit::record_in_tx(
        &mut transaction,
        None,
        Some(user.id),
        "auth.login_succeeded",
        "auth_session",
        Some(session_id),
        serde_json::json!({ "method": "password" }),
    )
    .await
    .map_err(ApiError::database)?;

    transaction.commit().await.map_err(ApiError::database)?;

    let mut headers = HeaderMap::new();
    cookies::append(
        &mut headers,
        cookies::session_cookie(
            &session_token,
            state.config.cookie_secure,
            state.config.session_ttl_seconds,
        ),
    );
    cookies::append(
        &mut headers,
        cookies::csrf_cookie(
            &csrf_token,
            state.config.cookie_secure,
            state.config.session_ttl_seconds,
        ),
    );
    no_store(&mut headers);

    Ok((
        headers,
        Json(LoginResponse {
            user: UserDto {
                id: user.id,
                email: user.email,
                display_name: user.display_name,
            },
            csrf_token,
            expires_at,
        }),
    )
        .into_response())
}

pub async fn current_session(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    headers_in: HeaderMap,
) -> Response {
    let mut headers = HeaderMap::new();
    let csrf_token = match cookies::read(&headers_in, cookies::CSRF_COOKIE) {
        Some(token) => token,
        None => {
            let token = session::generate_token();
            cookies::append(
                &mut headers,
                cookies::csrf_cookie(
                    &token,
                    state.config.cookie_secure,
                    state.config.session_ttl_seconds,
                ),
            );
            token
        }
    };
    no_store(&mut headers);

    (
        headers,
        Json(SessionResponse {
            user: context.user,
            session_id: context.session_id,
            csrf_token,
            expires_at: context.expires_at,
        }),
    )
        .into_response()
}

pub async fn logout(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Response, ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query("UPDATE auth_sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
        .bind(context.session_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    crate::audit::record_in_tx(
        &mut transaction,
        None,
        Some(context.user.id),
        "auth.logout",
        "auth_session",
        Some(context.session_id),
        serde_json::json!({ "scope": "current" }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    let mut headers = HeaderMap::new();
    cookies::append(
        &mut headers,
        cookies::clear_session_cookie(state.config.cookie_secure),
    );
    cookies::append(
        &mut headers,
        cookies::clear_csrf_cookie(state.config.cookie_secure),
    );
    no_store(&mut headers);
    Ok((StatusCode::NO_CONTENT, headers).into_response())
}

pub async fn logout_all(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Response, ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let revoked = sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now()
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(context.user.id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?
    .rows_affected();
    crate::audit::record_in_tx(
        &mut transaction,
        None,
        Some(context.user.id),
        "auth.sessions_revoked",
        "user",
        Some(context.user.id),
        serde_json::json!({ "revokedCount": revoked }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    let mut headers = HeaderMap::new();
    cookies::append(
        &mut headers,
        cookies::clear_session_cookie(state.config.cookie_secure),
    );
    cookies::append(
        &mut headers,
        cookies::clear_csrf_cookie(state.config.cookie_secure),
    );
    no_store(&mut headers);
    Ok((StatusCode::NO_CONTENT, headers).into_response())
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Json<Vec<SessionDto>>, ApiError> {
    let rows: Vec<SessionListRow> = sqlx::query_as(
        r#"
        SELECT id, created_at, last_seen_at, expires_at, revoked_at
        FROM auth_sessions
        WHERE user_id = $1
        ORDER BY created_at DESC
        LIMIT 50
        "#,
    )
    .bind(context.user.id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    Ok(Json(
        rows.into_iter()
            .map(|row| SessionDto {
                id: row.id,
                created_at: row.created_at,
                last_seen_at: row.last_seen_at,
                expires_at: row.expires_at,
                revoked_at: row.revoked_at,
                current: row.id == context.session_id,
            })
            .collect(),
    ))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(session_id): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let revoked: Option<Uuid> = sqlx::query_scalar(
        "UPDATE auth_sessions SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
         RETURNING id",
    )
    .bind(session_id)
    .bind(context.user.id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    if revoked.is_none() {
        return Err(ApiError::not_found(
            "session_not_found",
            "Session not found.",
        ));
    }

    crate::audit::record_in_tx(
        &mut transaction,
        None,
        Some(context.user.id),
        "auth.session_revoked",
        "auth_session",
        Some(session_id),
        serde_json::json!({ "revokedCurrent": session_id == context.session_id }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    let mut headers = HeaderMap::new();
    if session_id == context.session_id {
        cookies::append(
            &mut headers,
            cookies::clear_session_cookie(state.config.cookie_secure),
        );
        cookies::append(
            &mut headers,
            cookies::clear_csrf_cookie(state.config.cookie_secure),
        );
    }
    no_store(&mut headers);
    Ok((StatusCode::NO_CONTENT, headers).into_response())
}

/// Authenticate the opaque session cookie and attach the resolved user to the
/// request. Every project route is layered behind this middleware.
pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(token) = cookies::read(request.headers(), cookies::SESSION_COOKIE) else {
        return Err(ApiError::unauthorized(
            "authentication_required",
            "Authentication is required.",
        ));
    };

    let row: Option<SessionAuthRow> = sqlx::query_as(
        r#"
        SELECT s.id AS session_id, s.user_id, u.email, u.display_name, s.expires_at
        FROM auth_sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1
          AND s.revoked_at IS NULL
          AND s.expires_at > now()
          AND u.status = 'active'
        "#,
    )
    .bind(session::hash_token(&token))
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;

    let Some(row) = row else {
        return Err(ApiError::unauthorized(
            "authentication_required",
            "Authentication is required.",
        ));
    };

    if let Err(error) = sqlx::query(
        "UPDATE auth_sessions SET last_seen_at = now()
         WHERE id = $1 AND last_seen_at < now() - interval '5 minutes'",
    )
    .bind(row.session_id)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(error = %error, "failed to update session last-seen timestamp");
    }

    request.extensions_mut().insert(AuthContext {
        user: UserDto {
            id: row.user_id,
            email: row.email,
            display_name: row.display_name,
        },
        session_id: row.session_id,
        expires_at: row.expires_at,
    });

    Ok(next.run(request).await)
}

/// Reject unsafe requests without a matching double-submit CSRF token. The
/// session cookie is SameSite=Strict; this check also covers non-browser clients
/// and any future cross-site configuration. Origin/Sec-Fetch-Site checks add a
/// second layer when the browser supplies them.
pub async fn csrf_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if request.method() == axum::http::Method::GET
        || request.method() == axum::http::Method::HEAD
        || request.method() == axum::http::Method::OPTIONS
    {
        return Ok(next.run(request).await);
    }

    if let Some(site) = request.headers().get("sec-fetch-site")
        && site.as_bytes() == b"cross-site"
    {
        return Err(ApiError::forbidden(
            "cross_site_request",
            "Cross-site state-changing requests are not allowed.",
        ));
    }

    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin = origin.to_str().map_err(|_| {
            ApiError::forbidden("origin_not_allowed", "Request origin is not allowed.")
        })?;
        if !state
            .config
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin)
        {
            return Err(ApiError::forbidden(
                "origin_not_allowed",
                "Request origin is not allowed.",
            ));
        }
    }

    let cookie_token = cookies::read(request.headers(), cookies::CSRF_COOKIE)
        .ok_or_else(|| ApiError::forbidden("csrf_required", "A CSRF token is required."))?;
    let header_token = request
        .headers()
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::forbidden("csrf_required", "A CSRF token is required."))?;

    if !session::constant_time_eq(&cookie_token, header_token) {
        return Err(ApiError::forbidden(
            "csrf_invalid",
            "The CSRF token is invalid.",
        ));
    }

    Ok(next.run(request).await)
}

fn no_store(headers: &mut HeaderMap) {
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL_NO_STORE),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_rejects_unknown_fields() {
        let error = serde_json::from_str::<LoginRequest>(
            r#"{"email":"a@example.test","password":"long enough password","extra":true}"#,
        )
        .expect_err("unknown field must fail");
        assert!(error.to_string().contains("unknown field"));
    }
}
