use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;

pub const TOKEN_BYTES: usize = 32;

/// Opaque bearer token for the HttpOnly session cookie. Only its digest is
/// persisted; the plaintext exists only in the cookie and the current response.
pub fn generate_token() -> String {
    let mut bytes = [0_u8; TOKEN_BYTES];
    super::password::fill_random(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn rate_key(scope: &str, value: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(scope.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    hasher.finalize().to_vec()
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

#[derive(Debug)]
pub struct AttemptRecord {
    pub attempts: i32,
    pub window_started_at: DateTime<Utc>,
}

pub async fn record_attempt(
    pool: &PgPool,
    key: &[u8],
    max_attempts: i32,
    window_seconds: i64,
) -> Result<AttemptRecord, sqlx::Error> {
    let (attempts, window_started_at): (i32, DateTime<Utc>) = sqlx::query_as(
        r#"
        INSERT INTO auth_login_attempts (key_hash, window_started_at, attempts, last_attempt_at)
        VALUES ($1, now(), 1, now())
        ON CONFLICT (key_hash) DO UPDATE SET
            window_started_at = CASE
                WHEN auth_login_attempts.window_started_at <= now() - make_interval(secs => $2)
                THEN now()
                ELSE auth_login_attempts.window_started_at
            END,
            attempts = CASE
                WHEN auth_login_attempts.window_started_at <= now() - make_interval(secs => $2)
                THEN 1
                ELSE auth_login_attempts.attempts + 1
            END,
            last_attempt_at = now()
        RETURNING attempts, window_started_at
        "#,
    )
    .bind(key)
    .bind(window_seconds as f64)
    .fetch_one(pool)
    .await?;

    tracing::debug!(max_attempts, "login attempt recorded");
    Ok(AttemptRecord {
        attempts,
        window_started_at,
    })
}

pub fn retry_after_seconds(window_started_at: DateTime<Utc>, window_seconds: i64) -> u64 {
    let elapsed = Utc::now().signed_duration_since(window_started_at);
    let remaining = window_seconds - elapsed.num_seconds();
    remaining.max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_hash_to_32_bytes() {
        let first = generate_token();
        let second = generate_token();
        assert_ne!(first, second);
        assert_eq!(hash_token(&first).len(), 32);
        assert_eq!(TOKEN_BYTES, 32);
    }

    #[test]
    fn rate_keys_are_scoped() {
        assert_ne!(rate_key("account", "a"), rate_key("ip", "a"));
        assert_eq!(rate_key("account", "a").len(), 32);
    }

    #[test]
    fn constant_time_comparison_rejects_different_values() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}
