//! Platform admin settings.
//!
//! A platform admin is a settings-only operator: these routes manage global
//! platform configuration and grant no cross-tenant access. Provider API keys
//! are accepted write-only and stored as ciphertext; responses expose only a
//! masked hint. Every change is audited without the secret value.

use axum::{
    Extension, Json, Router,
    extract::State,
    routing::{delete, get},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::ApiError,
    platform_settings::{PlatformSettingsRow, RuntimeLimits, load_row},
    settings_secrets,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/settings", get(get_settings).put(update_settings))
        .route(
            "/admin/settings/planning/api-key",
            delete(clear_planning_api_key),
        )
}

/// `(ciphertext, hint, updated_at)` for a stored provider API key.
type ResolvedKey = (Option<String>, Option<String>, Option<DateTime<Utc>>);

const MAX_PLANNING_LIMITS_BYTES: usize = 32_768;
const PLANNING_LIMIT_KEYS: &[&str] = &[
    "maxModelCalls",
    "maxToolCalls",
    "maxTurns",
    "maxRetries",
    "maxResponseTokens",
    "maxWallTimeMs",
    "maxPromptBytes",
    "maxToolOutputBytes",
    "maxTotalToolOutputBytes",
    "maxPlanBytes",
    "maxRepositoryReadBytes",
    "maxCatalogEntries",
    "maxPlannedTests",
    "maxLoadScenarios",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDto {
    planning: PlanningDto,
    rate_limits: RateLimitsDto,
    retention: RetentionDto,
    project_defaults: ProjectDefaultsDto,
    settings_key_configured: bool,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanningDto {
    provider: Option<String>,
    model: Option<String>,
    limits: Value,
    api_key_configured: bool,
    api_key_hint: Option<String>,
    api_key_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitsDto {
    api_requests_per_minute: i64,
    expensive_requests_per_minute: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RetentionDto {
    event_days: i32,
    report_days: i32,
    counter_days: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDefaultsDto {
    max_active_runs: i32,
    max_requests_per_window: Option<i64>,
    window_seconds: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSettingsRequest {
    planning_provider: Option<String>,
    planning_model: Option<String>,
    planning_limits: Value,
    /// A new provider API key. Absent or blank leaves the stored key unchanged.
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    clear_api_key: bool,
    api_requests_per_minute: i64,
    expensive_requests_per_minute: i64,
    retention_event_days: i32,
    retention_report_days: i32,
    retention_counter_days: i32,
    default_max_active_runs: i32,
    default_max_requests_per_window: Option<i64>,
    default_window_seconds: i32,
}

fn require_admin(context: &AuthContext) -> Result<(), ApiError> {
    if context.user.is_admin {
        Ok(())
    } else {
        tracing::warn!(
            event = "admin.forbidden",
            user_id = %context.user.id,
            "platform administrator access denied"
        );
        Err(ApiError::forbidden(
            "forbidden",
            "Platform administrator access is required.",
        ))
    }
}

fn to_dto(row: &PlatformSettingsRow) -> SettingsDto {
    SettingsDto {
        planning: PlanningDto {
            provider: row.planning_provider.clone(),
            model: row.planning_model.clone(),
            limits: row.planning_limits.clone(),
            api_key_configured: row.planning_api_key_ciphertext.is_some(),
            api_key_hint: row.planning_api_key_hint.clone(),
            api_key_updated_at: row.planning_api_key_updated_at,
        },
        rate_limits: RateLimitsDto {
            api_requests_per_minute: i64::from(row.api_requests_per_minute),
            expensive_requests_per_minute: i64::from(row.expensive_requests_per_minute),
        },
        retention: RetentionDto {
            event_days: row.retention_event_days,
            report_days: row.retention_report_days,
            counter_days: row.retention_counter_days,
        },
        project_defaults: ProjectDefaultsDto {
            max_active_runs: row.default_max_active_runs,
            max_requests_per_window: row.default_max_requests_per_window,
            window_seconds: row.default_window_seconds,
        },
        settings_key_configured: settings_secrets::load_key().ok().flatten().is_some(),
        updated_by: row.updated_by,
        updated_at: row.updated_at,
    }
}

async fn load_row_or_error(state: &AppState) -> Result<PlatformSettingsRow, ApiError> {
    load_row(&state.pool)
        .await
        .map_err(ApiError::database)?
        .ok_or_else(|| ApiError::internal("platform settings row is missing"))
}

async fn get_settings(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Json<SettingsDto>, ApiError> {
    require_admin(&context)?;
    let row = load_row_or_error(&state).await?;
    Ok(Json(to_dto(&row)))
}

async fn update_settings(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    crate::error::ApiJson(payload): crate::error::ApiJson<UpdateSettingsRequest>,
) -> Result<Json<SettingsDto>, ApiError> {
    require_admin(&context)?;
    let current = load_row_or_error(&state).await?;

    let provider = normalize_identifier(
        payload.planning_provider.as_deref(),
        "provider",
        IdentifierKind::Provider,
    )?;
    let model = normalize_identifier(
        payload.planning_model.as_deref(),
        "model",
        IdentifierKind::Model,
    )?;
    validate_planning_limits(&payload.planning_limits)?;
    validate_bounds(&payload)?;

    let (ciphertext, hint, key_updated_at) =
        resolve_api_key(payload.api_key.as_deref(), payload.clear_api_key, &current)?;

    let row: PlatformSettingsRow = sqlx::query_as(
        r#"
        UPDATE platform_settings SET
            planning_provider = $1,
            planning_model = $2,
            planning_limits = $3,
            planning_api_key_ciphertext = $4,
            planning_api_key_hint = $5,
            planning_api_key_updated_at = $6,
            api_requests_per_minute = $7,
            expensive_requests_per_minute = $8,
            retention_event_days = $9,
            retention_report_days = $10,
            retention_counter_days = $11,
            default_max_active_runs = $12,
            default_max_requests_per_window = $13,
            default_window_seconds = $14,
            updated_by = $15
        WHERE id = true
        RETURNING planning_provider, planning_model, planning_limits,
                  planning_api_key_ciphertext, planning_api_key_hint, planning_api_key_updated_at,
                  api_requests_per_minute, expensive_requests_per_minute,
                  retention_event_days, retention_report_days, retention_counter_days,
                  default_max_active_runs, default_max_requests_per_window, default_window_seconds,
                  updated_by, updated_at
        "#,
    )
    .bind(&provider)
    .bind(&model)
    .bind(&payload.planning_limits)
    .bind(&ciphertext)
    .bind(&hint)
    .bind(key_updated_at)
    .bind(i32::try_from(payload.api_requests_per_minute).unwrap_or(i32::MAX))
    .bind(i32::try_from(payload.expensive_requests_per_minute).unwrap_or(i32::MAX))
    .bind(payload.retention_event_days)
    .bind(payload.retention_report_days)
    .bind(payload.retention_counter_days)
    .bind(payload.default_max_active_runs)
    .bind(payload.default_max_requests_per_window)
    .bind(payload.default_window_seconds)
    .bind(context.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;

    *state.runtime_limits.write().await = RuntimeLimits {
        api_requests_per_minute: payload.api_requests_per_minute,
        expensive_requests_per_minute: payload.expensive_requests_per_minute,
    };

    let metadata = serde_json::json!({
        "provider": row.planning_provider,
        "model": row.planning_model,
        "apiKeyConfigured": row.planning_api_key_ciphertext.is_some(),
        "apiRequestsPerMinute": row.api_requests_per_minute,
        "expensiveRequestsPerMinute": row.expensive_requests_per_minute,
        "retentionEventDays": row.retention_event_days,
        "retentionReportDays": row.retention_report_days,
        "retentionCounterDays": row.retention_counter_days,
        "defaultMaxActiveRuns": row.default_max_active_runs,
        "defaultWindowSeconds": row.default_window_seconds,
    });
    audit_setting(&state, &context, "settings.updated", metadata).await?;

    tracing::info!(
        event = "settings.updated",
        updated_by = %context.user.id,
        provider = ?row.planning_provider,
        model = ?row.planning_model,
        api_key_configured = row.planning_api_key_ciphertext.is_some(),
        api_requests_per_minute = row.api_requests_per_minute,
        expensive_requests_per_minute = row.expensive_requests_per_minute,
        "platform settings updated"
    );

    Ok(Json(to_dto(&row)))
}

async fn clear_planning_api_key(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
) -> Result<Json<SettingsDto>, ApiError> {
    require_admin(&context)?;
    let row: PlatformSettingsRow = sqlx::query_as(
        r#"
        UPDATE platform_settings SET
            planning_api_key_ciphertext = NULL,
            planning_api_key_hint = NULL,
            planning_api_key_updated_at = NULL,
            updated_by = $1
        WHERE id = true
        RETURNING planning_provider, planning_model, planning_limits,
                  planning_api_key_ciphertext, planning_api_key_hint, planning_api_key_updated_at,
                  api_requests_per_minute, expensive_requests_per_minute,
                  retention_event_days, retention_report_days, retention_counter_days,
                  default_max_active_runs, default_max_requests_per_window, default_window_seconds,
                  updated_by, updated_at
        "#,
    )
    .bind(context.user.id)
    .fetch_one(&state.pool)
    .await
    .map_err(ApiError::database)?;

    audit_setting(
        &state,
        &context,
        "settings.planning_api_key_cleared",
        serde_json::json!({}),
    )
    .await?;
    tracing::info!(
        event = "settings.planning_api_key_cleared",
        updated_by = %context.user.id,
        "platform planning API key cleared"
    );
    Ok(Json(to_dto(&row)))
}

async fn audit_setting(
    state: &AppState,
    context: &AuthContext,
    action: &str,
    metadata: Value,
) -> Result<(), ApiError> {
    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    audit::record_in_tx(
        &mut transaction,
        None,
        Some(context.user.id),
        action,
        "platform_settings",
        None,
        metadata,
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok(())
}

enum IdentifierKind {
    Provider,
    Model,
}

fn normalize_identifier(
    value: Option<&str>,
    label: &'static str,
    kind: IdentifierKind,
) -> Result<Option<String>, ApiError> {
    let trimmed = value.map(str::trim).filter(|value| !value.is_empty());
    let Some(trimmed) = trimmed else {
        return Ok(None);
    };
    let valid = match kind {
        IdentifierKind::Provider => {
            trimmed.len() <= 80
                && trimmed
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && trimmed.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        }
        IdentifierKind::Model => {
            trimmed.len() <= 160
                && trimmed
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && trimmed.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
                })
        }
    };
    if valid {
        Ok(Some(trimmed.to_owned()))
    } else {
        Err(ApiError::invalid(
            "invalid_planning_identifier",
            format!("Planning {label} is not a valid identifier."),
        ))
    }
}

fn validate_planning_limits(value: &Value) -> Result<(), ApiError> {
    let object = value.as_object().ok_or_else(|| {
        ApiError::invalid(
            "invalid_planning_limits",
            "planningLimits must be an object.",
        )
    })?;
    if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > MAX_PLANNING_LIMITS_BYTES
    {
        return Err(ApiError::invalid(
            "invalid_planning_limits",
            "planningLimits is too large.",
        ));
    }
    for (key, value) in object {
        if !PLANNING_LIMIT_KEYS.contains(&key.as_str()) {
            return Err(ApiError::invalid(
                "invalid_planning_limits",
                format!("Unknown planning limit '{key}'."),
            ));
        }
        let number = value.as_i64().ok_or_else(|| {
            ApiError::invalid(
                "invalid_planning_limits",
                "Planning limits must be integers.",
            )
        })?;
        if !(0..=1_000_000).contains(&number) {
            return Err(ApiError::invalid(
                "invalid_planning_limits",
                "A planning limit is out of range.",
            ));
        }
    }
    Ok(())
}

fn validate_bounds(payload: &UpdateSettingsRequest) -> Result<(), ApiError> {
    if !(10..=100_000).contains(&payload.api_requests_per_minute)
        || !(1..=10_000).contains(&payload.expensive_requests_per_minute)
    {
        return Err(ApiError::invalid(
            "invalid_rate_limits",
            "API rate limits are out of range.",
        ));
    }
    for days in [
        payload.retention_event_days,
        payload.retention_report_days,
        payload.retention_counter_days,
    ] {
        if !(1..=3_650).contains(&days) {
            return Err(ApiError::invalid(
                "invalid_retention",
                "Retention days are out of range.",
            ));
        }
    }
    if !(1..=100).contains(&payload.default_max_active_runs)
        || !(60..=604_800).contains(&payload.default_window_seconds)
        || payload
            .default_max_requests_per_window
            .is_some_and(|budget| !(0..=1_000_000_000).contains(&budget))
    {
        return Err(ApiError::invalid(
            "invalid_project_defaults",
            "Project default quotas are out of range.",
        ));
    }
    Ok(())
}

fn resolve_api_key(
    api_key: Option<&str>,
    clear: bool,
    current: &PlatformSettingsRow,
) -> Result<ResolvedKey, ApiError> {
    if clear {
        return Ok((None, None, None));
    }
    let candidate = api_key.map(str::trim).filter(|value| !value.is_empty());
    let Some(candidate) = candidate else {
        return Ok((
            current.planning_api_key_ciphertext.clone(),
            current.planning_api_key_hint.clone(),
            current.planning_api_key_updated_at,
        ));
    };
    if candidate.len() > 4096 {
        return Err(ApiError::invalid(
            "invalid_api_key",
            "The provider API key is too long.",
        ));
    }
    let master = settings_secrets::load_key()
        .map_err(|_| ApiError::internal("settings key could not be loaded"))?
        .ok_or_else(|| {
            ApiError::conflict(
                "settings_key_not_configured",
                "Set PROBE_FORGE_SETTINGS_KEY before storing a provider API key.",
            )
        })?;
    let ciphertext = settings_secrets::encrypt(&master, candidate)
        .map_err(|_| ApiError::internal("the provider API key could not be encrypted"))?;
    Ok((
        Some(ciphertext),
        Some(settings_secrets::hint(candidate)),
        Some(Utc::now()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_or_non_integer_planning_limits() {
        assert!(validate_planning_limits(&serde_json::json!({ "maxTurns": 3 })).is_ok());
        assert!(validate_planning_limits(&serde_json::json!({ "exec": "rm -rf" })).is_err());
        assert!(validate_planning_limits(&serde_json::json!({ "maxTurns": 1.5 })).is_err());
        assert!(validate_planning_limits(&serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn normalizes_identifiers_and_clears_blank_values() {
        assert_eq!(
            normalize_identifier(Some("  anthropic "), "provider", IdentifierKind::Provider)
                .expect("provider"),
            Some("anthropic".to_owned())
        );
        assert_eq!(
            normalize_identifier(Some("   "), "model", IdentifierKind::Model).expect("blank"),
            None
        );
        assert!(
            normalize_identifier(Some("Bad Provider"), "provider", IdentifierKind::Provider)
                .is_err()
        );
    }

    #[test]
    fn hint_never_contains_the_full_key_is_checked_elsewhere() {
        // Guard the invariant that a blank key leaves the stored value unchanged.
        let current = PlatformSettingsRow {
            planning_provider: None,
            planning_model: None,
            planning_limits: serde_json::json!({}),
            planning_api_key_ciphertext: Some("v1.a.b".to_owned()),
            planning_api_key_hint: Some("••••abcd".to_owned()),
            planning_api_key_updated_at: None,
            api_requests_per_minute: 240,
            expensive_requests_per_minute: 20,
            retention_event_days: 30,
            retention_report_days: 180,
            retention_counter_days: 7,
            default_max_active_runs: 2,
            default_max_requests_per_window: None,
            default_window_seconds: 86_400,
            updated_by: None,
            updated_at: Utc::now(),
        };
        let (ciphertext, hint, _) = resolve_api_key(Some("  "), false, &current).expect("blank");
        assert_eq!(ciphertext.as_deref(), Some("v1.a.b"));
        assert_eq!(hint.as_deref(), Some("••••abcd"));
        let (cleared, _, _) = resolve_api_key(None, true, &current).expect("clear");
        assert!(cleared.is_none());
    }
}
