//! Per-project execution quotas.
//!
//! Quotas are enforced server-side by the queue claim path
//! (`probe_forge_queue::claim_due_run_fair`); this module only lets an
//! owner/admin view and set them, with an audit record. A missing row means the
//! server default applies.

use axum::{Extension, Json, Router, extract::State, routing::get};
use probe_forge_queue::{NewProjectQuota, QueueError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiJson, ApiPath},
    projects::load_project_role,
};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/projects/{project_id}/quota",
        get(get_quota).put(set_quota),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaDto {
    project_id: Uuid,
    /// Whether an explicit quota row exists. When false, the server default
    /// active-run cap applies and there is no request budget.
    configured: bool,
    max_active_runs: Option<i32>,
    max_requests_per_window: Option<i64>,
    window_seconds: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetQuotaRequest {
    max_active_runs: i32,
    max_requests_per_window: Option<i64>,
    window_seconds: i32,
}

async fn get_quota(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<QuotaDto>, ApiError> {
    load_project_role(&state.pool, context.user.id, project_id).await?;
    let quota = probe_forge_queue::project_quota(&state.pool, project_id)
        .await
        .map_err(quota_error)?;

    Ok(Json(match quota {
        Some(row) => QuotaDto {
            project_id: row.project_id,
            configured: true,
            max_active_runs: Some(row.max_active_runs),
            max_requests_per_window: row.max_requests_per_window,
            window_seconds: Some(row.window_seconds),
        },
        None => QuotaDto {
            project_id,
            configured: false,
            max_active_runs: None,
            max_requests_per_window: None,
            window_seconds: None,
        },
    }))
}

async fn set_quota(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<SetQuotaRequest>,
) -> Result<Json<QuotaDto>, ApiError> {
    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    if !role.can_manage_quota() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to change quotas in this project.",
        ));
    }

    let row = probe_forge_queue::set_project_quota(
        &state.pool,
        NewProjectQuota {
            project_id,
            max_active_runs: payload.max_active_runs,
            max_requests_per_window: payload.max_requests_per_window,
            window_seconds: payload.window_seconds,
        },
    )
    .await
    .map_err(quota_error)?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    audit::record_in_tx(
        &mut transaction,
        Some(project_id),
        Some(context.user.id),
        "project.quota_updated",
        "project",
        Some(project_id),
        serde_json::json!({
            "maxActiveRuns": row.max_active_runs,
            "maxRequestsPerWindow": row.max_requests_per_window,
            "windowSeconds": row.window_seconds,
        }),
    )
    .await
    .map_err(ApiError::database)?;
    transaction.commit().await.map_err(ApiError::database)?;

    Ok(Json(QuotaDto {
        project_id: row.project_id,
        configured: true,
        max_active_runs: Some(row.max_active_runs),
        max_requests_per_window: row.max_requests_per_window,
        window_seconds: Some(row.window_seconds),
    }))
}

fn quota_error(error: QueueError) -> ApiError {
    match error {
        QueueError::InvalidQuota => ApiError::invalid(
            "invalid_quota",
            "Quota values are outside the allowed range.",
        ),
        QueueError::Database(error) => ApiError::database(error),
        _ => ApiError::invalid("quota_error", "The quota could not be stored."),
    }
}
