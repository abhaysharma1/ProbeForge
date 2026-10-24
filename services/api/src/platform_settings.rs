//! Platform settings storage and the in-memory runtime view.
//!
//! Settings live in a singleton `platform_settings` row. Secret material is
//! stored only as ciphertext (see `settings_secrets`). The rate-limit middleware
//! reads a small cached copy so it never queries per request; the cache is
//! refreshed at startup and whenever an admin updates the settings.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow)]
pub struct PlatformSettingsRow {
    pub planning_provider: Option<String>,
    pub planning_model: Option<String>,
    pub planning_limits: Value,
    pub planning_api_key_ciphertext: Option<String>,
    pub planning_api_key_hint: Option<String>,
    pub planning_api_key_updated_at: Option<DateTime<Utc>>,
    pub api_requests_per_minute: i32,
    pub expensive_requests_per_minute: i32,
    pub retention_event_days: i32,
    pub retention_report_days: i32,
    pub retention_counter_days: i32,
    pub default_max_active_runs: i32,
    pub default_max_requests_per_window: Option<i64>,
    pub default_window_seconds: i32,
    pub updated_by: Option<uuid::Uuid>,
    pub updated_at: DateTime<Utc>,
}

const COLUMNS: &str = r#"
    planning_provider, planning_model, planning_limits,
    planning_api_key_ciphertext, planning_api_key_hint, planning_api_key_updated_at,
    api_requests_per_minute, expensive_requests_per_minute,
    retention_event_days, retention_report_days, retention_counter_days,
    default_max_active_runs, default_max_requests_per_window, default_window_seconds,
    updated_by, updated_at
"#;

pub async fn load_row(pool: &PgPool) -> Result<Option<PlatformSettingsRow>, sqlx::Error> {
    let query = format!("SELECT {COLUMNS} FROM platform_settings WHERE id = true");
    sqlx::query_as(sqlx::AssertSqlSafe(query))
        .fetch_optional(pool)
        .await
}

/// The subset of settings the API request path needs on every call.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    pub api_requests_per_minute: i64,
    pub expensive_requests_per_minute: i64,
}

impl RuntimeLimits {
    pub fn from_row(row: &PlatformSettingsRow) -> Self {
        Self {
            api_requests_per_minute: i64::from(row.api_requests_per_minute),
            expensive_requests_per_minute: i64::from(row.expensive_requests_per_minute),
        }
    }
}
