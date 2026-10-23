//! Plan review, selection, and immutable approval APIs.
//!
//! The model-generated plan payload is read-only. This module only changes the
//! `selected` flags on planned tests and load scenarios, using an optimistic
//! revision check, and writes an immutable approval snapshot. Arbitrary plan
//! fields, commands, configuration, and executable strings cannot be edited.

use std::collections::HashSet;

use axum::{
    Extension, Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use probeforge_contracts::{JobGoal, LoadProfile, LoadRequest, Plan, Risk, validate_plan};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    catalog::{self, CatalogEntryDto},
    error::{ApiError, ApiJson, ApiPath},
    projects::{ProjectRole, load_project_role},
};

const MAX_SELECTED_TESTS: usize = 100;
const MAX_SELECTED_SCENARIOS: usize = 50;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/test-catalog", get(get_test_catalog))
        .route(
            "/plans/{plan_id}",
            get(get_plan).patch(update_plan_selection),
        )
        .route("/plans/{plan_id}/approve", post(approve_plan))
        .route("/jobs/{job_id}/plan", get(get_job_plan))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedTestDto {
    id: Uuid,
    catalog_test_id: String,
    name: String,
    rationale: String,
    risk: Risk,
    prerequisites: Vec<String>,
    expected_evidence: String,
    target_requirements: Vec<String>,
    requires_explicit_confirmation: bool,
    configuration: Value,
    rationale_references: Vec<String>,
    selected: bool,
    catalog: Option<CatalogEntryDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadScenarioDto {
    id: Uuid,
    name: String,
    description: String,
    requests: Vec<LoadRequest>,
    profile: LoadProfile,
    credential_ref: Option<String>,
    selected: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalSummaryDto {
    id: Uuid,
    plan_revision: i32,
    approved_by: Uuid,
    approved_by_name: String,
    approved_by_email: String,
    approved_at: DateTime<Utc>,
    approved_test_ids: Vec<Uuid>,
    approved_load_scenario_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDto {
    id: Uuid,
    project_id: Uuid,
    job_id: Uuid,
    version: i32,
    revision: i32,
    status: String,
    goal: String,
    summary: String,
    confidence: f64,
    required_inputs: Vec<String>,
    limitations: Vec<String>,
    evidence_references: Vec<String>,
    created_by_agent: bool,
    agent_provider: Option<String>,
    agent_model: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    catalog_version: String,
    planned_tests: Vec<PlannedTestDto>,
    load_scenarios: Vec<LoadScenarioDto>,
    approval: Option<ApprovalSummaryDto>,
}

#[derive(Debug, FromRow)]
struct PlanRow {
    id: Uuid,
    project_id: Uuid,
    job_id: Uuid,
    version: i32,
    revision: i32,
    status: String,
    confidence: f64,
    plan_payload: Value,
    agent_provider: Option<String>,
    agent_model: Option<String>,
    created_by_agent: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct PlannedTestRow {
    id: Uuid,
    catalog_test_id: String,
    name: String,
    rationale: String,
    risk: String,
    prerequisites: Value,
    expected_evidence: String,
    target_requirements: Value,
    requires_explicit_confirmation: bool,
    configuration: Value,
    rationale_references: Value,
    selected: bool,
}

#[derive(Debug, FromRow)]
struct LoadScenarioRow {
    id: Uuid,
    name: String,
    scenario: Value,
    selected: bool,
}

#[derive(Debug, FromRow)]
struct ApprovalRow {
    id: Uuid,
    plan_revision: i32,
    approved_by: Uuid,
    approved_by_name: String,
    approved_by_email: String,
    approved_at: DateTime<Utc>,
    approved_test_ids: Vec<Uuid>,
    approved_load_scenario_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSelectionRequest {
    expected_revision: i32,
    #[serde(default)]
    selected_test_ids: Vec<Uuid>,
    #[serde(default)]
    selected_load_scenario_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApproveRequest {
    expected_revision: i32,
    confirmed: bool,
}

const PLAN_COLUMNS: &str = r#"
    id, project_id, job_id, version, revision, status,
    confidence::float8 AS confidence, plan_payload, agent_provider, agent_model,
    created_by_agent, created_at, updated_at
"#;

const PLANNED_TEST_COLUMNS: &str = r#"
    id, catalog_test_id, name, rationale, risk, prerequisites, expected_evidence,
    target_requirements, requires_explicit_confirmation, configuration,
    rationale_references, selected
"#;

async fn get_test_catalog() -> Result<Json<catalog::TestCatalogDto>, ApiError> {
    catalog::catalog_dto()
        .map(Json)
        .map_err(catalog_internal_error)
}

async fn get_plan(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(plan_id): ApiPath<Uuid>,
) -> Result<Json<PlanDto>, ApiError> {
    let row = load_plan_for_user(&state.pool, context.user.id, plan_id).await?;
    Ok(Json(build_plan_dto(&state.pool, row).await?))
}

async fn get_job_plan(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(job_id): ApiPath<Uuid>,
) -> Result<Json<PlanDto>, ApiError> {
    let row = load_job_plan_for_user(&state.pool, context.user.id, job_id).await?;
    Ok(Json(build_plan_dto(&state.pool, row).await?))
}

async fn update_plan_selection(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(plan_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<UpdateSelectionRequest>,
) -> Result<Json<PlanDto>, ApiError> {
    let (plan_ref, role) = load_plan_reference(&state.pool, context.user.id, plan_id).await?;
    require_plan_writer(role)?;
    if payload.expected_revision < 0 {
        return Err(ApiError::invalid(
            "invalid_revision",
            "expectedRevision must not be negative.",
        ));
    }
    if payload.selected_test_ids.len() > MAX_SELECTED_TESTS
        || payload.selected_load_scenario_ids.len() > MAX_SELECTED_SCENARIOS
    {
        return Err(ApiError::invalid(
            "too_many_selections",
            "The selected test or scenario set is too large.",
        ));
    }
    ensure_unique_uuids(&payload.selected_test_ids, "selectedTestIds")?;
    ensure_unique_uuids(
        &payload.selected_load_scenario_ids,
        "selectedLoadScenarioIds",
    )?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let row = load_plan_row_for_update(&mut transaction, plan_id).await?;
    if row.project_id != plan_ref.project_id || row.status != "proposed" && row.status != "draft" {
        return Err(ApiError::conflict(
            "plan_not_editable",
            "This plan can no longer be edited.",
        ));
    }
    if row.revision != payload.expected_revision {
        return Err(plan_conflict());
    }
    ensure_current_plan(&mut transaction, &row).await?;

    let plan = parse_stored_plan(&row.plan_payload)?;
    validate_selection_for_goal(
        plan.goal,
        payload.selected_test_ids.len(),
        payload.selected_load_scenario_ids.len(),
    )?;

    let test_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM planned_tests WHERE plan_id = $1 AND id = ANY($2::uuid[])",
    )
    .bind(plan_id)
    .bind(&payload.selected_test_ids)
    .fetch_all(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    if test_ids.len() != payload.selected_test_ids.len() {
        return Err(ApiError::invalid(
            "unknown_plan_test",
            "One or more selected tests do not belong to this plan.",
        ));
    }

    let scenario_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM load_scenarios WHERE plan_id = $1 AND id = ANY($2::uuid[])",
    )
    .bind(plan_id)
    .bind(&payload.selected_load_scenario_ids)
    .fetch_all(&mut *transaction)
    .await
    .map_err(ApiError::database)?;
    if scenario_ids.len() != payload.selected_load_scenario_ids.len() {
        return Err(ApiError::invalid(
            "unknown_load_scenario",
            "One or more selected load scenarios do not belong to this plan.",
        ));
    }

    sqlx::query("UPDATE planned_tests SET selected = (id = ANY($2::uuid[])) WHERE plan_id = $1")
        .bind(plan_id)
        .bind(&payload.selected_test_ids)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    sqlx::query("UPDATE load_scenarios SET selected = (id = ANY($2::uuid[])) WHERE plan_id = $1")
        .bind(plan_id)
        .bind(&payload.selected_load_scenario_ids)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;

    let revision: i32 = sqlx::query_scalar(
        "UPDATE test_plans SET revision = revision + 1 WHERE id = $1 RETURNING revision",
    )
    .bind(plan_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(row.project_id),
        Some(context.user.id),
        "plan.selection_updated",
        "test_plan",
        Some(plan_id),
        json!({
            "planVersion": row.version,
            "revision": revision,
            "selectedTestCount": payload.selected_test_ids.len(),
            "selectedLoadScenarioCount": payload.selected_load_scenario_ids.len(),
        }),
    )
    .await
    .map_err(ApiError::database)?;

    transaction.commit().await.map_err(ApiError::database)?;
    let updated = load_plan_row(&state.pool, plan_id).await?;
    Ok(Json(build_plan_dto(&state.pool, updated).await?))
}

async fn approve_plan(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(plan_id): ApiPath<Uuid>,
    ApiJson(payload): ApiJson<ApproveRequest>,
) -> Result<Json<PlanDto>, ApiError> {
    let (plan_ref, role) = load_plan_reference(&state.pool, context.user.id, plan_id).await?;
    require_plan_writer(role)?;
    if !payload.confirmed {
        return Err(ApiError::invalid(
            "confirmation_required",
            "Explicit confirmation is required before approving a plan.",
        ));
    }
    if payload.expected_revision < 0 {
        return Err(ApiError::invalid(
            "invalid_revision",
            "expectedRevision must not be negative.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let row = load_plan_row_for_update(&mut transaction, plan_id).await?;
    if row.project_id != plan_ref.project_id {
        return Err(plan_not_found());
    }
    if row.status == "approved" {
        return Err(ApiError::conflict(
            "plan_already_approved",
            "This plan has already been approved.",
        ));
    }
    if row.status != "proposed" && row.status != "draft" {
        return Err(ApiError::conflict(
            "plan_not_editable",
            "This plan can no longer be approved.",
        ));
    }
    if row.revision != payload.expected_revision {
        return Err(plan_conflict());
    }
    ensure_current_plan(&mut transaction, &row).await?;

    let plan = parse_stored_plan(&row.plan_payload)?;
    let tests = load_selected_tests(&mut transaction, plan_id).await?;
    let scenarios = load_selected_scenarios(&mut transaction, plan_id).await?;
    if tests.is_empty() && scenarios.is_empty() {
        return Err(ApiError::invalid(
            "plan_no_selection",
            "Select at least one test or load scenario before approval.",
        ));
    }
    validate_selection_for_goal(plan.goal, tests.len(), scenarios.len())?;

    let catalog_version = catalog::catalog_version()
        .map_err(catalog_internal_error)?
        .to_owned();
    let mut test_snapshots = Vec::with_capacity(tests.len());
    for test in &tests {
        let entry = catalog::find_entry(&test.catalog_test_id)
            .map_err(catalog_internal_error)?
            .ok_or_else(|| {
                ApiError::conflict(
                    "catalog_entry_missing",
                    "A selected catalog test is no longer available.",
                )
            })?;
        if entry.category == catalog::CatalogCategory::Load {
            return Err(ApiError::conflict(
                "catalog_entry_mismatch",
                "A selected vulnerability test has an incompatible catalog category.",
            ));
        }
        test_snapshots.push(json!({
            "plannedTestId": test.id,
            "catalogTestId": test.catalog_test_id,
            "name": test.name,
            "rationale": test.rationale,
            "risk": test.risk,
            "prerequisites": test.prerequisites,
            "expectedEvidence": test.expected_evidence,
            "targetRequirements": test.target_requirements,
            "requiresExplicitConfirmation": test.requires_explicit_confirmation,
            "configuration": test.configuration,
            "rationaleReferences": test.rationale_references,
            "catalog": entry,
        }));
    }
    let scenario_snapshots: Vec<Value> = scenarios
        .iter()
        .map(|scenario| {
            json!({
                "loadScenarioId": scenario.id,
                "name": scenario.name,
                "scenario": scenario.scenario,
            })
        })
        .collect();

    let approved_at = Utc::now();
    let approved_test_ids: Vec<Uuid> = tests.iter().map(|test| test.id).collect();
    let approved_load_scenario_ids: Vec<Uuid> =
        scenarios.iter().map(|scenario| scenario.id).collect();
    let snapshot = json!({
        "schemaVersion": "1",
        "planId": row.id,
        "planVersion": row.version,
        "planRevision": row.revision,
        "jobId": row.job_id,
        "projectId": row.project_id,
        "goal": goal_to_str(plan.goal),
        "catalogVersion": catalog_version.clone(),
        "approvedBy": context.user.id,
        "approvedByEmail": context.user.email.clone(),
        "approvedAt": approved_at,
        "summary": plan.summary,
        "confidence": plan.confidence,
        "requiredInputs": plan.required_inputs,
        "limitations": plan.limitations,
        "tests": test_snapshots,
        "loadScenarios": scenario_snapshots,
    });

    let approval_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO plan_approvals (
            project_id, plan_id, plan_version, plan_revision, approved_by,
            approved_test_ids, approved_load_scenario_ids, approval_snapshot, approved_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(row.project_id)
    .bind(plan_id)
    .bind(row.version)
    .bind(row.revision)
    .bind(context.user.id)
    .bind(&approved_test_ids)
    .bind(&approved_load_scenario_ids)
    .bind(&snapshot)
    .bind(approved_at)
    .fetch_one(&mut *transaction)
    .await
    .map_err(map_approval_error)?;

    sqlx::query("UPDATE test_plans SET status = 'approved' WHERE id = $1")
        .bind(plan_id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE test_plans SET status = 'superseded'
         WHERE job_id = $1 AND id <> $2 AND status IN ('draft', 'proposed')",
    )
    .bind(row.job_id)
    .bind(plan_id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(row.project_id),
        Some(context.user.id),
        "plan.approved",
        "test_plan",
        Some(plan_id),
        json!({
            "approvalId": approval_id,
            "planVersion": row.version,
            "planRevision": row.revision,
            "catalogVersion": catalog_version,
            "approvedTestCount": approved_test_ids.len(),
            "approvedLoadScenarioCount": approved_load_scenario_ids.len(),
        }),
    )
    .await
    .map_err(ApiError::database)?;

    transaction.commit().await.map_err(ApiError::database)?;
    let updated = load_plan_row(&state.pool, plan_id).await?;
    Ok(Json(build_plan_dto(&state.pool, updated).await?))
}

async fn load_plan_reference(
    pool: &PgPool,
    user_id: Uuid,
    plan_id: Uuid,
) -> Result<(PlanReference, ProjectRole), ApiError> {
    let project_id: Option<Uuid> =
        sqlx::query_scalar("SELECT project_id FROM test_plans WHERE id = $1")
            .bind(plan_id)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::database)?;
    let Some(project_id) = project_id else {
        return Err(plan_not_found());
    };
    let role = load_project_role(pool, user_id, project_id).await?;
    Ok((PlanReference { project_id }, role))
}

struct PlanReference {
    project_id: Uuid,
}

async fn load_plan_for_user(
    pool: &PgPool,
    user_id: Uuid,
    plan_id: Uuid,
) -> Result<PlanRow, ApiError> {
    let (reference, _) = load_plan_reference(pool, user_id, plan_id).await?;
    let row = load_plan_row(pool, plan_id).await?;
    if row.project_id != reference.project_id {
        return Err(plan_not_found());
    }
    Ok(row)
}

async fn load_job_plan_for_user(
    pool: &PgPool,
    user_id: Uuid,
    job_id: Uuid,
) -> Result<PlanRow, ApiError> {
    let job: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT project_id, current_plan_id FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::database)?;
    let Some((project_id, current_plan_id)) = job else {
        return Err(ApiError::not_found("job_not_found", "Job not found."));
    };
    load_project_role(pool, user_id, project_id).await?;
    let Some(plan_id) = current_plan_id else {
        return Err(ApiError::not_found(
            "plan_not_found",
            "This job does not have a proposed plan yet.",
        ));
    };
    let row = load_plan_row(pool, plan_id).await?;
    if row.job_id != job_id || row.project_id != project_id {
        return Err(plan_not_found());
    }
    Ok(row)
}

async fn load_plan_row(pool: &PgPool, plan_id: Uuid) -> Result<PlanRow, ApiError> {
    let query = format!("SELECT {PLAN_COLUMNS} FROM test_plans WHERE id = $1");
    let row: Option<PlanRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(plan_id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::database)?;
    row.ok_or_else(plan_not_found)
}

async fn load_plan_row_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<PlanRow, ApiError> {
    let query = format!("SELECT {PLAN_COLUMNS} FROM test_plans WHERE id = $1 FOR UPDATE");
    let row: Option<PlanRow> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(plan_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ApiError::database)?;
    row.ok_or_else(plan_not_found)
}

async fn ensure_current_plan(
    transaction: &mut Transaction<'_, Postgres>,
    row: &PlanRow,
) -> Result<(), ApiError> {
    let current_plan_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_plan_id FROM jobs WHERE id = $1 FOR UPDATE")
            .bind(row.job_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(ApiError::database)?;
    if current_plan_id != Some(row.id) {
        return Err(ApiError::conflict(
            "plan_superseded",
            "A newer plan is available for this job. Reload before editing or approving.",
        ));
    }
    Ok(())
}

async fn load_selected_tests(
    transaction: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<Vec<PlannedTestRow>, ApiError> {
    let query = format!(
        "SELECT {PLANNED_TEST_COLUMNS} FROM planned_tests
         WHERE plan_id = $1 AND selected
         ORDER BY name, id"
    );
    sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(plan_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(ApiError::database)
}

async fn load_selected_scenarios(
    transaction: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<Vec<LoadScenarioRow>, ApiError> {
    sqlx::query_as(
        "SELECT id, name, scenario, selected FROM load_scenarios
         WHERE plan_id = $1 AND selected
         ORDER BY name, id",
    )
    .bind(plan_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(ApiError::database)
}

async fn build_plan_dto(pool: &PgPool, row: PlanRow) -> Result<PlanDto, ApiError> {
    let plan = parse_stored_plan(&row.plan_payload)?;
    let catalog_version = catalog::catalog_version()
        .map_err(catalog_internal_error)?
        .to_owned();

    let test_query = format!(
        "SELECT {PLANNED_TEST_COLUMNS} FROM planned_tests WHERE plan_id = $1 ORDER BY name, id"
    );
    let tests: Vec<PlannedTestRow> = sqlx::query_as(sqlx::AssertSqlSafe(test_query))
        .bind(row.id)
        .fetch_all(pool)
        .await
        .map_err(ApiError::database)?;
    let mut planned_tests = Vec::with_capacity(tests.len());
    for test in tests {
        let catalog_entry =
            catalog::find_entry(&test.catalog_test_id).map_err(catalog_internal_error)?;
        planned_tests.push(PlannedTestDto {
            id: test.id,
            catalog_test_id: test.catalog_test_id,
            name: test.name,
            rationale: test.rationale,
            risk: risk_from_db(&test.risk)?,
            prerequisites: string_vec(test.prerequisites, "prerequisites")?,
            expected_evidence: test.expected_evidence,
            target_requirements: string_vec(test.target_requirements, "targetRequirements")?,
            requires_explicit_confirmation: test.requires_explicit_confirmation,
            configuration: test.configuration,
            rationale_references: string_vec(test.rationale_references, "rationaleReferences")?,
            selected: test.selected,
            catalog: catalog_entry,
        });
    }

    let scenarios: Vec<LoadScenarioRow> = sqlx::query_as(
        "SELECT id, name, scenario, selected FROM load_scenarios
         WHERE plan_id = $1 ORDER BY name, id",
    )
    .bind(row.id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)?;
    let mut load_scenarios = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let parsed: probeforge_contracts::LoadScenario = serde_json::from_value(scenario.scenario)
            .map_err(|_| ApiError::internal("stored load scenario is invalid"))?;
        load_scenarios.push(LoadScenarioDto {
            id: scenario.id,
            name: scenario.name,
            description: parsed.description,
            requests: parsed.requests,
            profile: parsed.profile,
            credential_ref: parsed.credential_ref,
            selected: scenario.selected,
        });
    }

    let approval: Option<ApprovalRow> = sqlx::query_as(
        r#"
        SELECT a.id, a.plan_revision, a.approved_by,
               u.display_name AS approved_by_name, u.email AS approved_by_email,
               a.approved_at, a.approved_test_ids, a.approved_load_scenario_ids
        FROM plan_approvals a
        JOIN users u ON u.id = a.approved_by
        WHERE a.plan_id = $1
        ORDER BY a.approved_at DESC
        LIMIT 1
        "#,
    )
    .bind(row.id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)?;

    Ok(PlanDto {
        id: row.id,
        project_id: row.project_id,
        job_id: row.job_id,
        version: row.version,
        revision: row.revision,
        status: row.status,
        goal: goal_to_str(plan.goal).to_owned(),
        summary: plan.summary,
        confidence: row.confidence,
        required_inputs: plan.required_inputs,
        limitations: plan.limitations,
        evidence_references: plan.evidence_references,
        created_by_agent: row.created_by_agent,
        agent_provider: row.agent_provider,
        agent_model: row.agent_model,
        created_at: row.created_at,
        updated_at: row.updated_at,
        catalog_version,
        planned_tests,
        load_scenarios,
        approval: approval.map(|approval| ApprovalSummaryDto {
            id: approval.id,
            plan_revision: approval.plan_revision,
            approved_by: approval.approved_by,
            approved_by_name: approval.approved_by_name,
            approved_by_email: approval.approved_by_email,
            approved_at: approval.approved_at,
            approved_test_ids: approval.approved_test_ids,
            approved_load_scenario_ids: approval.approved_load_scenario_ids,
        }),
    })
}

fn parse_stored_plan(value: &Value) -> Result<Plan, ApiError> {
    validate_plan(value).map_err(|error| {
        tracing::error!(error = %error, "stored plan payload failed contract validation");
        ApiError::internal("stored plan payload is invalid")
    })
}

fn validate_selection_for_goal(
    goal: JobGoal,
    test_count: usize,
    scenario_count: usize,
) -> Result<(), ApiError> {
    match goal {
        JobGoal::VulnerabilityTesting if scenario_count > 0 => Err(ApiError::invalid(
            "invalid_selection",
            "Vulnerability plans can only select planned tests.",
        )),
        JobGoal::LoadTesting if test_count > 0 => Err(ApiError::invalid(
            "invalid_selection",
            "Load plans can only select load scenarios.",
        )),
        _ => Ok(()),
    }
}

fn ensure_unique_uuids(values: &[Uuid], field: &str) -> Result<(), ApiError> {
    let unique: HashSet<Uuid> = values.iter().copied().collect();
    if unique.len() != values.len() {
        return Err(ApiError::invalid(
            "duplicate_selection",
            format!("{field} must not contain duplicate IDs."),
        ));
    }
    Ok(())
}

fn risk_from_db(value: &str) -> Result<Risk, ApiError> {
    match value {
        "low" => Ok(Risk::Low),
        "medium" => Ok(Risk::Medium),
        "high" => Ok(Risk::High),
        "critical" => Ok(Risk::Critical),
        _ => Err(ApiError::internal("stored test risk is invalid")),
    }
}

fn string_vec(value: Value, field: &str) -> Result<Vec<String>, ApiError> {
    serde_json::from_value(value)
        .map_err(|_| ApiError::internal(format!("stored {field} is invalid")))
}

fn goal_to_str(goal: JobGoal) -> &'static str {
    match goal {
        JobGoal::VulnerabilityTesting => "vulnerability_testing",
        JobGoal::LoadTesting => "load_testing",
    }
}

fn require_plan_writer(role: ProjectRole) -> Result<(), ApiError> {
    if role.can_review_plans() {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to edit or approve plans in this project.",
        ))
    }
}

fn plan_conflict() -> ApiError {
    ApiError::conflict(
        "plan_conflict",
        "This plan changed since it was loaded. Reload and review the latest revision.",
    )
}

fn plan_not_found() -> ApiError {
    ApiError::not_found("plan_not_found", "Plan not found.")
}

fn catalog_internal_error(error: &str) -> ApiError {
    tracing::error!(error, "embedded test catalog is invalid");
    ApiError::internal("The test catalog is unavailable.")
}

fn map_approval_error(error: sqlx::Error) -> ApiError {
    if matches!(
        &error,
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some("23505")
    ) {
        return ApiError::conflict(
            "plan_already_approved",
            "This plan has already been approved.",
        );
    }
    ApiError::database(error)
}
