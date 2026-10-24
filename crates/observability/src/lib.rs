//! Shared observability helpers for every ProbeForge service.
//!
//! Three concerns live here so a single implementation governs them:
//!
//! * `init_tracing` configures structured (text or JSON) logging from the
//!   environment, once per process.
//! * [`RequestId`] is the correlation identifier threaded through logs so an
//!   operator can follow one request across services.
//! * [`redact_json`] / [`redact_text`] remove credentials before any value is
//!   logged, stored in a generated report, or exported. Redaction lives here so
//!   the API and the workers cannot drift apart.

use std::fmt;

use serde_json::Value;

/// Replacement marker for a redacted value.
pub const REDACTED: &str = "[redacted]";

/// Longest accepted correlation identifier, matching common proxy defaults.
pub const MAX_REQUEST_ID_LEN: usize = 128;

/// Object keys whose values are replaced wholesale, matched case-insensitively.
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "private_key",
    "token",
];

/// Configure the global tracing subscriber for a service.
///
/// `PROBE_FORGE_LOG_FORMAT=json` selects line-delimited JSON logs; anything else
/// keeps the human-readable default. `RUST_LOG` controls filtering and defaults
/// to `info`. The caller invokes this once, before serving.
pub fn init_tracing(service: &'static str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let log_format = std::env::var("PROBE_FORGE_LOG_FORMAT").unwrap_or_default();

    if log_format.eq_ignore_ascii_case("json") {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_target(true)
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_target(false)
            .with_env_filter(filter)
            .init();
    }

    tracing::info!(service, "tracing initialized");
}

/// A validated request correlation identifier.
///
/// The value is either supplied by a trusted caller through `X-Request-Id` and
/// validated, or generated locally. It is restricted to printable ASCII so it
/// can never smuggle newlines or control characters into a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(String);

impl RequestId {
    /// Generate a fresh random correlation identifier.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    /// Parse an inbound header value, rejecting empty, oversized, or
    /// non-printable input.
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_REQUEST_ID_LEN {
            return None;
        }
        if !trimmed.bytes().all(|byte| byte.is_ascii_graphic()) {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// Borrow the identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A wrapper that redacts a value when formatted, for safe logging.
pub struct Redacted<'a>(pub &'a str);

impl fmt::Display for Redacted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&redact_text(self.0))
    }
}

/// Recursively redact sensitive object keys and credential markers.
pub fn redact_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                let normalized = key.to_ascii_lowercase();
                if SENSITIVE_KEYS.contains(&normalized.as_str()) {
                    redacted.insert(key, Value::String(REDACTED.to_owned()));
                } else {
                    redacted.insert(key, redact_json(item));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_json).collect()),
        Value::String(text) => Value::String(redact_text(&text)),
        other => other,
    }
}

/// Redact credential markers embedded in free text.
pub fn redact_text(input: &str) -> String {
    let mut output = input.to_owned();
    output = redact_marker(&output, "bearer ");
    output = redact_marker(&output, "basic ");
    output = redact_marker(&output, "authorization: ");
    output = redact_marker(&output, "cookie: ");
    redact_jwts(&output)
}

fn redact_marker(input: &str, marker: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(marker) {
        let start = cursor + relative;
        let after = start + marker.len();
        output.push_str(&input[cursor..after]);
        let token_start = after
            + input[after..]
                .len()
                .saturating_sub(input[after..].trim_start().len());
        let mut token_end = token_start;
        for character in input[token_start..].chars() {
            if character.is_whitespace() || character == '"' || character == ',' {
                break;
            }
            token_end += character.len_utf8();
        }
        if token_end > token_start {
            output.push_str(REDACTED);
        }
        cursor = token_end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn redact_jwts(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for token in
        input.split_inclusive(|character: char| character.is_whitespace() || character == '"')
    {
        let trimmed =
            token.trim_end_matches(|character: char| character.is_whitespace() || character == '"');
        if trimmed.starts_with("eyJ") && trimmed.matches('.').count() == 2 {
            output.push_str(REDACTED);
            output.push_str(&token[trimmed.len()..]);
        } else {
            output.push_str(token);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_ids_reject_unsafe_values() {
        assert_eq!(
            RequestId::parse("abc-123").expect("valid").as_str(),
            "abc-123"
        );
        assert!(RequestId::parse("").is_none());
        assert!(RequestId::parse("bad\nvalue").is_none());
        assert!(RequestId::parse(&"x".repeat(MAX_REQUEST_ID_LEN + 1)).is_none());
        assert_eq!(RequestId::generate().as_str().len(), 32);
    }

    #[test]
    fn redacts_sensitive_keys_and_markers() {
        let redacted = redact_json(json!({
            "authorization": "Bearer shorttoken",
            "nested": { "client_secret": "shh", "safe": "keep me" },
            "header": "Authorization: Bearer tok",
            "list": ["password=hunter2"],
        }));
        assert_eq!(redacted["authorization"], REDACTED);
        assert_eq!(redacted["nested"]["client_secret"], REDACTED);
        assert_eq!(redacted["nested"]["safe"], "keep me");
        assert!(redacted["header"].as_str().unwrap().contains(REDACTED));
    }

    #[test]
    fn redacts_jwt_shaped_tokens() {
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signature";
        let output = redact_text(&format!("{token} rest"));
        assert!(output.contains(REDACTED));
        assert!(!output.contains(token));
    }

    #[test]
    fn redacted_display_wraps_text() {
        let message = format!("{}", Redacted("Bearer shortsecret"));
        assert!(message.contains(REDACTED));
        assert!(!message.contains("shortsecret"));
    }
}
