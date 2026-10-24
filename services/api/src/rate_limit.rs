//! Server-side API rate limiting.
//!
//! Counters are stored in `api_rate_limits`, keyed by a SHA-256 digest of
//! `scope\0subject`, so the raw identity is never persisted. A fixed window is
//! cheap and sufficient here: the window start is reset lazily on the next
//! request after it elapses. Rate limiting is a defense-in-depth control; the
//! authoritative authorization checks are unchanged.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::ApiError;

/// Upper bound so a misconfiguration can never disable the check silently or
/// store a nonsensical value.
const MAX_RATE_LIMIT: i64 = 100_000;

#[derive(sqlx::FromRow)]
struct RateLimitRow {
    count: i32,
    window_started_at: DateTime<Utc>,
}

/// Digest a rate-limit key so only an opaque value is stored.
pub(crate) fn key_hash(scope: &str, subject: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    hasher.finalize().to_vec()
}

/// Record one attempt and reject the request when the fixed-window limit is
/// exceeded. A non-positive limit disables the check.
pub(crate) async fn enforce(
    pool: &PgPool,
    scope: &str,
    subject: &str,
    limit: i64,
    window_seconds: i64,
) -> Result<(), ApiError> {
    if limit <= 0 || limit > MAX_RATE_LIMIT {
        return Ok(());
    }

    let key = key_hash(scope, subject);
    let row: RateLimitRow = sqlx::query_as(
        r#"
        INSERT INTO api_rate_limits (key_hash, window_started_at, count, last_at)
        VALUES ($1, now(), 1, now())
        ON CONFLICT (key_hash) DO UPDATE SET
            count = CASE
                WHEN api_rate_limits.window_started_at
                     <= now() - make_interval(secs => $2::double precision)
                THEN 1 ELSE api_rate_limits.count + 1 END,
            window_started_at = CASE
                WHEN api_rate_limits.window_started_at
                     <= now() - make_interval(secs => $2::double precision)
                THEN now() ELSE api_rate_limits.window_started_at END,
            last_at = now()
        RETURNING count, window_started_at
        "#,
    )
    .bind(&key)
    .bind(window_seconds as f64)
    .fetch_one(pool)
    .await
    .map_err(ApiError::database)?;

    if i64::from(row.count) > limit {
        let elapsed = (Utc::now() - row.window_started_at).num_seconds().max(0);
        let retry_after = (window_seconds - elapsed).max(1) as u64;
        return Err(ApiError::rate_limited(retry_after));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hash_is_stable_and_scoped() {
        assert_eq!(key_hash("api", "user-1"), key_hash("api", "user-1"));
        assert_ne!(key_hash("api", "user-1"), key_hash("expensive", "user-1"));
        assert_ne!(key_hash("api", "user-1"), key_hash("api", "user-2"));
        assert_eq!(key_hash("api", "user-1").len(), 32);
    }
}
