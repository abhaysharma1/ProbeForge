//! Run materialization and cancellation.
//!
//! Approving a plan records an immutable selection; it does not start work.
//! A run is created only when an owner/admin/collaborator chooses an active
//! target authorization for an approved plan, which produces one run per
//! approved planned test (vulnerability) or load scenario. Runs are queued and
//! claimed later by the execution worker. Creation is idempotent per
//! `(approval, selection)`, so a retried request cannot duplicate work.

use axum::{Extension, Json, Router, extract::State, routing::post};
use probe_forge_queue::append_job_event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiJson, ApiPath},
    projects::load_project_role,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/plans/{plan_id}/runs", post(create_runs))
        .route("/runs/{run_id}/cancel", post(cancel_run))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateRunsRequest {
    target_authorization_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunRef {
    id: Uuid,
    run_type: String,
    status: String,
}

#[derive(Debug, FromRow)]
struct PlanRow {
    project_id: Uuid,
    job_id: Uuid,
    version: i32,
    status: String,
    goal: Option<String>,
}

#[derive(Debug, FromRow)]
struct ApprovalRow {
    id: Uuid,
    approved_test_ids: Vec<Uuid>,
    approved_load_scenario_ids: Vec<Uuid>,
}

async fn create_runs(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(plan_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<CreateRunsRequest>,
) -> Result<Json<Vec<RunRef>>, ApiError> {
    let plan: PlanRow = sqlx::query_as(
        "SELECT project_id, job_id, version, status, plan_payload->>'goal' AS goal
         FROM test_plans WHERE id = $1",
    )
    .bind(plan_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(|| ApiError::not_found("plan_not_found", "Plan not found."))?;

    let role = load_project_role(&state.pool, context.user.id, plan.project_id).await?;
    if !role.can_create_jobs() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to create runs in this project.",
        ));
    }
    if plan.status != "approved" {
        return Err(ApiError::conflict(
            "plan_not_approved",
            "Runs can only be created from an approved plan.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    validate_authorization(
        &mut transaction,
        plan.project_id,
        payload.target_authorization_id,
    )
    .await?;

    let approval: ApprovalRow = sqlx::query_as(
        "SELECT id, approved_test_ids, approved_load_scenario_ids
         FROM plan_approvals
         WHERE plan_id = $1 AND plan_version = $2
         ORDER BY approved_at DESC
         LIMIT 1",
    )
    .bind(plan_id)
    .bind(plan.version)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(|| {
        ApiError::conflict(
            "plan_not_approved",
            "The approved plan is missing its approval record.",
        )
    })?;

    let goal = plan.goal.as_deref().unwrap_or("");
    let mut runs = Vec::new();

    match goal {
        "vulnerability_testing" => {
            let planned: Vec<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM planned_tests
                 WHERE plan_id = $1 AND selected AND id = ANY($2)
                 ORDER BY catalog_test_id",
            )
            .bind(plan_id)
            .bind(&approval.approved_test_ids)
            .fetch_all(&mut *transaction)
            .await
            .map_err(ApiError::database)?;

            for (planned_test_id,) in planned {
                let key = format!("run:{}:{}", approval.id, planned_test_id);
                let run_id = insert_run(
                    &mut transaction,
                    InsertRun {
                        project_id: plan.project_id,
                        job_id: plan.job_id,
                        actor_id: context.user.id,
                        plan_id,
                        plan_version: plan.version,
                        approval_id: approval.id,
                        target_authorization_id: payload.target_authorization_id,
                        run_type: "vulnerability_testing",
                        planned_test_id: Some(planned_test_id),
                        load_scenario_id: None,
                        load_profile_id: None,
                        idempotency_key: &key,
                    },
                )
                .await?;
                runs.push(RunRef {
                    id: run_id,
                    run_type: "vulnerability_testing".to_owned(),
                    status: "queued".to_owned(),
                });
            }
        }
        "load_testing" => {
            let scenarios: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT id, profile_id FROM load_scenarios
                 WHERE plan_id = $1 AND selected AND id = ANY($2)
                 ORDER BY name",
            )
            .bind(plan_id)
            .bind(&approval.approved_load_scenario_ids)
            .fetch_all(&mut *transaction)
            .await
            .map_err(ApiError::database)?;

            for (scenario_id, profile_id) in scenarios {
                let key = format!("run:{}:{}", approval.id, scenario_id);
                let run_id = insert_run(
                    &mut transaction,
                    InsertRun {
                        project_id: plan.project_id,
                        job_id: plan.job_id,
                        actor_id: context.user.id,
                        plan_id,
                        plan_version: plan.version,
                        approval_id: approval.id,
                        target_authorization_id: payload.target_authorization_id,
                        run_type: "load_testing",
                        planned_test_id: None,
                        load_scenario_id: Some(scenario_id),
                        load_profile_id: Some(profile_id),
                        idempotency_key: &key,
                    },
                )
                .await?;
                runs.push(RunRef {
                    id: run_id,
                    run_type: "load_testing".to_owned(),
                    status: "queued".to_owned(),
                });
            }
        }
        _ => {
            return Err(ApiError::conflict(
                "plan_goal_unsupported",
                "This plan goal cannot produce runs.",
            ));
        }
    }

    if runs.is_empty() {
        return Err(ApiError::conflict(
            "plan_no_selection",
            "The approved plan has no selected tests or scenarios.",
        ));
    }

    transaction.commit().await.map_err(ApiError::database)?;
    info!(
        plan_id = %plan_id,
        run_count = runs.len(),
        "runs queued for an approved plan"
    );
    Ok(Json(runs))
}

struct InsertRun<'a> {
    project_id: Uuid,
    job_id: Uuid,
    actor_id: Uuid,
    plan_id: Uuid,
    plan_version: i32,
    approval_id: Uuid,
    target_authorization_id: Uuid,
    run_type: &'static str,
    planned_test_id: Option<Uuid>,
    load_scenario_id: Option<Uuid>,
    load_profile_id: Option<Uuid>,
    idempotency_key: &'a str,
}

async fn insert_run(
    transaction: &mut Transaction<'_, Postgres>,
    input: InsertRun<'_>,
) -> Result<Uuid, ApiError> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        r#"
        INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, planned_test_id, load_scenario_id, load_profile_id,
            target_authorization_id, idempotency_key
        ) VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $8, $9, $10, $11)
        ON CONFLICT (project_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(input.project_id)
    .bind(input.job_id)
    .bind(input.plan_id)
    .bind(input.plan_version)
    .bind(input.approval_id)
    .bind(input.run_type)
    .bind(input.planned_test_id)
    .bind(input.load_scenario_id)
    .bind(input.load_profile_id)
    .bind(input.target_authorization_id)
    .bind(input.idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::database)?;

    if let Some(run_id) = inserted {
        append_job_event(
            transaction,
            input.job_id,
            "run.queued",
            json!({ "runId": run_id, "runType": input.run_type }),
        )
        .await
        .map_err(queue_error)?;
        audit_run(
            transaction,
            input.project_id,
            input.actor_id,
            "run.queued",
            run_id,
            json!({ "runType": input.run_type, "planId": input.plan_id }),
        )
        .await?;
        return Ok(run_id);
    }

    // Idempotent retry: return the already-queued run.
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM test_runs WHERE project_id = $1 AND idempotency_key = $2",
    )
    .bind(input.project_id)
    .bind(input.idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    existing.ok_or_else(|| ApiError::internal("idempotent run lookup failed"))
}

async fn cancel_run(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<Json<RunRef>, ApiError> {
    let row: Option<(Uuid, Uuid, String, String)> =
        sqlx::query_as("SELECT project_id, job_id, run_type, status FROM test_runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::database)?;
    let (project_id, job_id, run_type, status) =
        row.ok_or_else(|| ApiError::not_found("run_not_found", "Run not found."))?;

    let role = load_project_role(&state.pool, context.user.id, project_id).await?;
    if !role.can_create_jobs() {
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to cancel runs in this project.",
        ));
    }

    if status == "cancelled" {
        return Ok(Json(RunRef {
            id: run_id,
            run_type,
            status,
        }));
    }
    if matches!(status.as_str(), "completed" | "failed" | "expired") {
        return Err(ApiError::conflict(
            "run_terminal",
            "A terminal run cannot be cancelled.",
        ));
    }
    if status == "running" {
        // The lease owner must stop its own work and report the outcome.
        return Err(ApiError::conflict(
            "run_in_progress",
            "A running run is stopped by its worker; cancel is refused.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE test_runs
         SET status = 'cancelled', finished_at = now(), last_error_code = 'cancelled_by_user'
         WHERE id = $1 AND status IN ('draft', 'awaiting_target', 'scheduled', 'queued')",
    )
    .bind(run_id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    append_job_event(
        &mut transaction,
        job_id,
        "run.cancelled",
        json!({ "runId": run_id, "cancelledBy": context.user.id }),
    )
    .await
    .map_err(queue_error)?;
    audit_run(
        &mut transaction,
        project_id,
        context.user.id,
        "run.cancelled",
        run_id,
        json!({ "runType": run_type }),
    )
    .await?;
    transaction.commit().await.map_err(ApiError::database)?;

    Ok(Json(RunRef {
        id: run_id,
        run_type,
        status: "cancelled".to_owned(),
    }))
}

async fn validate_authorization(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    authorization_id: Uuid,
) -> Result<(), ApiError> {
    let active: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM target_authorizations
         WHERE id = $1 AND project_id = $2
           AND revoked_at IS NULL
           AND valid_from <= now()
           AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(authorization_id)
    .bind(project_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    if active.is_none() {
        return Err(ApiError::invalid(
            "target_authorization_inactive",
            "The selected target authorization is missing, revoked, or expired.",
        ));
    }
    Ok(())
}

async fn audit_run(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    actor_id: Uuid,
    action: &str,
    run_id: Uuid,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    audit::record_in_tx(
        transaction,
        Some(project_id),
        Some(actor_id),
        action,
        "test_run",
        Some(run_id),
        metadata,
    )
    .await
    .map_err(ApiError::database)
}

fn queue_error(error: probe_forge_queue::QueueError) -> ApiError {
    match error {
        probe_forge_queue::QueueError::Database(error) => ApiError::database(error),
        other => ApiError::internal(other.to_string()),
    }
}
