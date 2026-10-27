//! Run results, recommendations, versioned reports, and rerun comparison.
//!
//! Raw structured observations and sanitized evidence are durable and
//! independent of any generated summary. Recommendations are traceable to a
//! result, evidence row, or metric bucket unless they are explicitly labeled a
//! hypothesis. Reports are immutable versions in a table separate from results,
//! so a generation failure cannot destroy test results.

use std::{convert::Infallible, time::Duration};

use async_stream::stream;
use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use chrono::{DateTime, Utc};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use probe_forge_observability::redact_json;
use probe_forge_queue::append_run_and_job_event;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiPath, ApiQuery},
    projects::{ProjectRole, load_project_role},
};

const MAX_LISTED_RUNS: i64 = 100;
const MAX_RESULTS_PER_RUN: i64 = 500;
const MAX_RECOMMENDATIONS_PER_RUN: i64 = 200;
const MAX_REPORT_BUCKETS: i64 = 500;
const MAX_REPORT_CONTENT_BYTES: usize = 262144;
const MAX_REPORT_VERSIONS: i64 = 50;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/runs", get(list_runs))
        .route("/runs/{run_id}", get(get_run))
        .route("/runs/{run_id}/events", get(run_events))
        .route(
            "/runs/{run_id}/report",
            get(get_report).post(generate_report),
        )
        .route("/runs/{run_id}/report/export", get(export_report))
        .route("/runs/{run_id}/compare", get(compare_runs))
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct RunRow {
    id: Uuid,
    project_id: Uuid,
    job_id: Uuid,
    plan_id: Uuid,
    plan_version: i32,
    run_type: String,
    /// Planned test name for vulnerability runs, scenario name for load runs.
    name: String,
    status: String,
    load_scenario_id: Option<Uuid>,
    target_authorization_id: Uuid,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    role: String,
}

#[derive(Debug, FromRow)]
struct RunListRow {
    id: Uuid,
    project_id: Uuid,
    job_id: Uuid,
    run_type: String,
    /// Planned test name for vulnerability runs, scenario name for load runs.
    name: String,
    status: String,
    plan_id: Uuid,
    plan_version: i32,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    passed: i64,
    failed: i64,
    skipped: i64,
    inconclusive: i64,
    error: i64,
    cancelled: i64,
    report_status: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct ResultCountsRow {
    passed: i64,
    failed: i64,
    skipped: i64,
    inconclusive: i64,
    error: i64,
    cancelled: i64,
    confirmed: i64,
    suspected: i64,
}

#[derive(Debug, Clone, FromRow)]
struct ResultRow {
    id: Uuid,
    catalog_test_id: String,
    result_key: String,
    outcome: String,
    finding_status: Option<String>,
    severity: Option<String>,
    confidence: Option<f64>,
    summary: String,
    details: Value,
    observation: Value,
    tool_name: Option<String>,
    tool_version: Option<String>,
    duration_ms: i32,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct EvidenceRow {
    id: Uuid,
    result_id: Uuid,
    storage_ref: String,
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    redacted: bool,
    retention_until: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct RecommendationRow {
    id: Uuid,
    result_id: Option<Uuid>,
    evidence_id: Option<Uuid>,
    run_metric_id: Option<Uuid>,
    kind: String,
    severity: String,
    title: String,
    body: String,
    rationale: Option<String>,
    confidence: Option<f64>,
    generated_by: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct MetricSummaryRow {
    request_count: i64,
    error_count: i64,
    response_bytes: i64,
    latency_p95_ms: f64,
    throughput_rps: f64,
}

#[derive(Debug, Clone, FromRow)]
struct MetricBucketRow {
    id: Uuid,
    bucket_start: DateTime<Utc>,
    bucket_seconds: i32,
    endpoint: String,
    status_code: i32,
    request_count: i64,
    error_count: i64,
    latency_p95_ms: Option<f64>,
    throughput_rps: Option<f64>,
    response_bytes: i64,
}

#[derive(Debug, Clone, FromRow)]
struct ScenarioRow {
    scenario_id: Uuid,
    scenario_name: String,
    virtual_users: Option<i32>,
    arrival_rate_per_second: Option<f64>,
    ramp_up_seconds: i32,
    duration_seconds: i32,
    max_requests: i64,
    max_requests_per_second: f64,
}

#[derive(Debug, Clone, FromRow)]
struct ReportRow {
    id: Uuid,
    report_type: String,
    status: String,
    current_version: i32,
    last_error_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct ReportVersionMetaRow {
    id: Uuid,
    version: i32,
    status: String,
    content_hash: String,
    summary: String,
    generated_by: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct ReportVersionFullRow {
    id: Uuid,
    version: i32,
    status: String,
    content: Value,
    content_hash: String,
    summary: String,
    generated_by: String,
    created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultCountsDto {
    passed: i64,
    failed: i64,
    skipped: i64,
    inconclusive: i64,
    error: i64,
    cancelled: i64,
    confirmed: i64,
    suspected: i64,
}

impl From<&ResultCountsRow> for ResultCountsDto {
    fn from(row: &ResultCountsRow) -> Self {
        Self {
            passed: row.passed,
            failed: row.failed,
            skipped: row.skipped,
            inconclusive: row.inconclusive,
            error: row.error,
            cancelled: row.cancelled,
            confirmed: row.confirmed,
            suspected: row.suspected,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunSummaryDto {
    id: Uuid,
    project_id: Uuid,
    job_id: Uuid,
    run_type: String,
    name: String,
    status: String,
    plan_id: Uuid,
    plan_version: i32,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    result_counts: ResultCountsDto,
    report_status: Option<String>,
}

impl From<RunListRow> for RunSummaryDto {
    fn from(row: RunListRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            job_id: row.job_id,
            run_type: row.run_type,
            name: row.name,
            status: row.status,
            plan_id: row.plan_id,
            plan_version: row.plan_version,
            started_at: row.started_at,
            finished_at: row.finished_at,
            created_at: row.created_at,
            result_counts: ResultCountsDto {
                passed: row.passed,
                failed: row.failed,
                skipped: row.skipped,
                inconclusive: row.inconclusive,
                error: row.error,
                cancelled: row.cancelled,
                confirmed: 0,
                suspected: 0,
            },
            report_status: row.report_status,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceDto {
    id: Uuid,
    storage_ref: String,
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    redacted: bool,
    retention_until: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<&EvidenceRow> for EvidenceDto {
    fn from(row: &EvidenceRow) -> Self {
        Self {
            id: row.id,
            storage_ref: row.storage_ref.clone(),
            sha256: row.sha256.clone(),
            mime_type: row.mime_type.clone(),
            size_bytes: row.size_bytes,
            redacted: row.redacted,
            retention_until: row.retention_until,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResultDto {
    id: Uuid,
    catalog_test_id: String,
    result_key: String,
    outcome: String,
    finding_status: Option<String>,
    severity: Option<String>,
    confidence: Option<f64>,
    summary: String,
    details: Value,
    observation: Value,
    tool_name: Option<String>,
    tool_version: Option<String>,
    duration_ms: i32,
    created_at: DateTime<Utc>,
    evidence: Vec<EvidenceDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecommendationDto {
    id: Uuid,
    result_id: Option<Uuid>,
    evidence_id: Option<Uuid>,
    run_metric_id: Option<Uuid>,
    kind: String,
    severity: String,
    title: String,
    body: String,
    rationale: Option<String>,
    confidence: Option<f64>,
    generated_by: String,
    created_at: DateTime<Utc>,
}

impl From<&RecommendationRow> for RecommendationDto {
    fn from(row: &RecommendationRow) -> Self {
        Self {
            id: row.id,
            result_id: row.result_id,
            evidence_id: row.evidence_id,
            run_metric_id: row.run_metric_id,
            kind: row.kind.clone(),
            severity: row.severity.clone(),
            title: row.title.clone(),
            body: row.body.clone(),
            rationale: row.rationale.clone(),
            confidence: row.confidence,
            generated_by: row.generated_by.clone(),
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricSummaryDto {
    request_count: i64,
    error_count: i64,
    response_bytes: i64,
    latency_p95_ms: f64,
    throughput_rps: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScenarioDto {
    id: Uuid,
    name: String,
    virtual_users: Option<i32>,
    arrival_rate_per_second: Option<f64>,
    ramp_up_seconds: i32,
    duration_seconds: i32,
    max_requests: i64,
    max_requests_per_second: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetRefDto {
    authorization_id: Uuid,
    origin: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunDetailDto {
    run: RunSummaryDto,
    report_type: String,
    target: TargetRefDto,
    scenario: Option<ScenarioDto>,
    results: Vec<TestResultDto>,
    recommendations: Vec<RecommendationDto>,
    metrics: MetricSummaryDto,
    report: Option<ReportDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportVersionMetaDto {
    id: Uuid,
    version: i32,
    status: String,
    content_hash: String,
    summary: String,
    generated_by: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportVersionFullDto {
    id: Uuid,
    version: i32,
    status: String,
    content: Value,
    content_hash: String,
    summary: String,
    generated_by: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportDto {
    id: Uuid,
    run_id: Uuid,
    report_type: String,
    status: String,
    current_version: i32,
    last_error_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    latest: Option<ReportVersionFullDto>,
    versions: Vec<ReportVersionMetaDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompareQuery {
    other_run_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportQuery {
    format: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonSideDto {
    run_id: Uuid,
    run_type: String,
    name: String,
    status: String,
    plan_version: i32,
    scenario_id: Option<Uuid>,
    scenario_name: Option<String>,
    target_origin: Option<String>,
    profile: Option<ScenarioDto>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    result_counts: ResultCountsDto,
    metrics: MetricSummaryDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricDeltaDto {
    request_count: i64,
    error_count: i64,
    latency_p95_ms: f64,
    throughput_rps: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonDto {
    left: ComparisonSideDto,
    right: ComparisonSideDto,
    delta: MetricDeltaDto,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_runs(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<RunSummaryDto>>, ApiError> {
    load_project_role(&state.pool, context.user.id, project_id).await?;
    let rows: Vec<RunListRow> = sqlx::query_as(
        r#"
        SELECT r.id, r.project_id, r.job_id, r.run_type,
               COALESCE(pt.name, ls.name, r.run_type) AS name,
               r.status, r.plan_id,
               r.plan_version, r.started_at, r.finished_at, r.created_at,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'passed') AS passed,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'failed') AS failed,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'skipped') AS skipped,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'inconclusive') AS inconclusive,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'error') AS error,
               (SELECT count(*) FROM test_results tr
                 WHERE tr.run_id = r.id AND tr.outcome = 'cancelled') AS cancelled,
               (SELECT rep.status FROM reports rep
                 WHERE rep.run_id = r.id
                   AND rep.report_type = CASE
                       WHEN r.run_type = 'load_testing' THEN 'load' ELSE 'security'
                   END) AS report_status
        FROM test_runs r
        LEFT JOIN planned_tests pt
               ON pt.id = r.planned_test_id AND pt.project_id = r.project_id
        LEFT JOIN load_scenarios ls
               ON ls.id = r.load_scenario_id AND ls.project_id = r.project_id
        WHERE r.project_id = $1
        ORDER BY r.created_at DESC
        LIMIT $2
        "#,
    )
    .bind(project_id)
    .bind(MAX_LISTED_RUNS)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    Ok(Json(rows.into_iter().map(RunSummaryDto::from).collect()))
}

async fn get_run(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<Json<RunDetailDto>, ApiError> {
    let run = load_run_for_user(&state.pool, context.user.id, run_id).await?;
    let detail = load_run_detail(&state.pool, &run).await?;
    Ok(Json(detail))
}

// ---------------------------------------------------------------------------
// Run event stream (SSE)
// ---------------------------------------------------------------------------

const MAX_RUN_EVENT_BATCH: i64 = 200;
const RUN_EVENT_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunEventQuery {
    after: Option<i64>,
    follow: Option<bool>,
}

#[derive(Debug, FromRow)]
struct RunEventRow {
    sequence: i64,
    event_type: String,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

/// Statuses that cannot gain new lifecycle, execution, or report events. A
/// stream is only worth opening while a run can still move; a terminal run has
/// already drained its last event.
fn run_is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "completed_with_report_error" | "failed" | "cancelled" | "expired"
    )
}

async fn load_run_event_batch(
    pool: &PgPool,
    run_id: Uuid,
    after: i64,
    limit: i64,
) -> Result<Vec<RunEventRow>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT sequence, event_type, payload, occurred_at
        FROM run_events
        WHERE run_id = $1 AND sequence > $2
        ORDER BY sequence ASC
        LIMIT $3
        "#,
    )
    .bind(run_id)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

async fn run_status(pool: &PgPool, run_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM test_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
}

fn queue_error(error: probe_forge_queue::QueueError) -> ApiError {
    match error {
        probe_forge_queue::QueueError::Database(error) => ApiError::database(error),
        other => ApiError::internal(other.to_string()),
    }
}

/// Live event stream for one run, replayable with `Last-Event-ID` / `after`
/// exactly like the job stream. PostgreSQL stays authoritative: the stream is a
/// prompt for the client to refresh, and per-run sequences survive reconnects.
async fn run_events(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<RunEventQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // Membership is the authorization gate, checked before the stream opens.
    load_run_for_user(&state.pool, context.user.id, run_id).await?;

    let header_cursor = match headers.get("last-event-id") {
        Some(value) => Some(
            value
                .to_str()
                .ok()
                .and_then(|value| value.trim().parse::<i64>().ok())
                .filter(|value| *value >= 0)
                .ok_or_else(|| {
                    ApiError::invalid(
                        "invalid_last_event_id",
                        "Last-Event-ID must be a non-negative integer.",
                    )
                })?,
        ),
        None => None,
    };
    let cursor = header_cursor.or(query.after).unwrap_or(0);
    if cursor < 0 {
        return Err(ApiError::invalid(
            "invalid_after",
            "after must be a non-negative event sequence.",
        ));
    }
    let follow = query.follow.unwrap_or(true);

    let pool = state.pool.clone();
    let event_stream = stream! {
        let mut cursor = cursor;
        loop {
            let batch = match load_run_event_batch(&pool, run_id, cursor, MAX_RUN_EVENT_BATCH).await {
                Ok(batch) => batch,
                Err(_) => {
                    yield Ok::<Event, Infallible>(Event::default()
                        .event("stream.error")
                        .data(json!({ "code": "event_stream_failed" }).to_string()));
                    break;
                }
            };
            let batch_len = batch.len();
            for row in batch {
                cursor = row.sequence;
                let data = json!({
                    "schemaVersion": "1",
                    "runId": run_id,
                    "sequence": row.sequence,
                    "type": row.event_type,
                    "occurredAt": row.occurred_at,
                    "payload": row.payload,
                });
                let event = Event::default()
                    .id(row.sequence.to_string())
                    .event(row.event_type)
                    .data(data.to_string())
                    .retry(Duration::from_secs(2));
                yield Ok::<Event, Infallible>(event);
            }

            if !follow {
                if batch_len < MAX_RUN_EVENT_BATCH as usize {
                    break;
                }
                continue;
            }

            if batch_len == 0 {
                match run_status(&pool, run_id).await {
                    Ok(Some(status)) if run_is_terminal(&status) => break,
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => {
                        yield Ok::<Event, Infallible>(Event::default()
                            .event("stream.error")
                            .data(json!({ "code": "event_stream_failed" }).to_string()));
                        break;
                    }
                }
                tokio::time::sleep(RUN_EVENT_POLL_INTERVAL).await;
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn get_report(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<Json<ReportDto>, ApiError> {
    let run = load_run_for_user(&state.pool, context.user.id, run_id).await?;
    let report_type = report_type_for(&run.run_type);
    let report = load_report_dto(&state.pool, run_id, report_type).await?;
    match report {
        Some(report) => Ok(Json(report)),
        None => Err(ApiError::not_found(
            "report_not_found",
            "No report has been generated for this run.",
        )),
    }
}

async fn generate_report(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
) -> Result<(StatusCode, Json<ReportDto>), ApiError> {
    let run = load_run_for_user(&state.pool, context.user.id, run_id).await?;
    let role = ProjectRole::from_db(&run.role)
        .ok_or_else(|| ApiError::internal("run membership role is invalid"))?;
    require_report_writer(role)?;
    let report = build_and_store_report(&state.pool, &run, context.user.id).await?;
    tracing::info!(
        event = "report.generated",
        run_id = %run.id,
        project_id = %run.project_id,
        user_id = %context.user.id,
        "report generated"
    );
    Ok((StatusCode::CREATED, Json(report)))
}

async fn export_report(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<ExportQuery>,
) -> Result<Response, ApiError> {
    let run = load_run_for_user(&state.pool, context.user.id, run_id).await?;
    let latest: Option<ReportVersionFullRow> = sqlx::query_as(
        r#"
        SELECT id, version, status, content, content_hash, summary, generated_by, created_at
        FROM report_versions
        WHERE run_id = $1
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;

    let Some(latest) = latest else {
        return Err(ApiError::not_found(
            "report_not_found",
            "No report has been generated for this run.",
        ));
    };

    let format = query.format.unwrap_or_else(|| "json".to_owned());
    if format == "markdown" {
        let body = render_markdown(&latest.content);
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            body,
        )
            .into_response());
    }
    Ok(Json(json!({
        "runId": run_id,
        "reportType": report_type_for(&run.run_type),
        "version": latest.version,
        "contentHash": latest.content_hash,
        "generatedBy": latest.generated_by,
        "createdAt": latest.created_at,
        "content": latest.content,
    }))
    .into_response())
}

async fn compare_runs(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(run_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<CompareQuery>,
) -> Result<Json<ComparisonDto>, ApiError> {
    let left = load_run_for_user(&state.pool, context.user.id, run_id).await?;
    let right = load_run_for_user(&state.pool, context.user.id, query.other_run_id).await?;
    let left_side = build_comparison_side(&state.pool, &left).await?;
    let right_side = build_comparison_side(&state.pool, &right).await?;
    let delta = MetricDeltaDto {
        request_count: right_side.metrics.request_count - left_side.metrics.request_count,
        error_count: right_side.metrics.error_count - left_side.metrics.error_count,
        latency_p95_ms: right_side.metrics.latency_p95_ms - left_side.metrics.latency_p95_ms,
        throughput_rps: right_side.metrics.throughput_rps - left_side.metrics.throughput_rps,
    };
    Ok(Json(ComparisonDto {
        left: left_side,
        right: right_side,
        delta,
    }))
}

// ---------------------------------------------------------------------------
// Loaders
// ---------------------------------------------------------------------------

async fn load_run_for_user(pool: &PgPool, user_id: Uuid, run_id: Uuid) -> Result<RunRow, ApiError> {
    let row: Option<RunRow> = sqlx::query_as(
        r#"
        SELECT r.id, r.project_id, r.job_id, r.plan_id, r.plan_version, r.run_type,
               COALESCE(pt.name, ls.name, r.run_type) AS name,
               r.status, r.load_scenario_id, r.target_authorization_id,
               r.started_at, r.finished_at, r.created_at, pm.role
        FROM test_runs r
        LEFT JOIN planned_tests pt
               ON pt.id = r.planned_test_id AND pt.project_id = r.project_id
        LEFT JOIN load_scenarios ls
               ON ls.id = r.load_scenario_id AND ls.project_id = r.project_id
        JOIN project_members pm ON pm.project_id = r.project_id AND pm.user_id = $2
        WHERE r.id = $1
        "#,
    )
    .bind(run_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)?;

    row.ok_or_else(|| ApiError::not_found("run_not_found", "Run not found."))
}

async fn load_result_counts(pool: &PgPool, run_id: Uuid) -> Result<ResultCountsRow, ApiError> {
    let row: ResultCountsRow = sqlx::query_as(
        r#"
        SELECT
            count(*) FILTER (WHERE outcome = 'passed')::bigint AS passed,
            count(*) FILTER (WHERE outcome = 'failed')::bigint AS failed,
            count(*) FILTER (WHERE outcome = 'skipped')::bigint AS skipped,
            count(*) FILTER (WHERE outcome = 'inconclusive')::bigint AS inconclusive,
            count(*) FILTER (WHERE outcome = 'error')::bigint AS error,
            count(*) FILTER (WHERE outcome = 'cancelled')::bigint AS cancelled,
            count(*) FILTER (WHERE finding_status = 'confirmed')::bigint AS confirmed,
            count(*) FILTER (WHERE finding_status = 'suspected')::bigint AS suspected
        FROM test_results
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::database)?;
    Ok(row)
}

struct ResultBundle {
    result: ResultRow,
    evidence: Vec<EvidenceRow>,
}

async fn load_result_bundles(pool: &PgPool, run_id: Uuid) -> Result<Vec<ResultBundle>, ApiError> {
    let results: Vec<ResultRow> = sqlx::query_as(
        r#"
        SELECT id, catalog_test_id, result_key, outcome, finding_status, severity,
               confidence::float8 AS confidence, summary, details, observation,
               tool_name, tool_version, duration_ms, created_at
        FROM test_results
        WHERE run_id = $1
        ORDER BY created_at, id
        LIMIT $2
        "#,
    )
    .bind(run_id)
    .bind(MAX_RESULTS_PER_RUN)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)?;

    let result_ids: Vec<Uuid> = results.iter().map(|row| row.id).collect();
    let evidence: Vec<EvidenceRow> = sqlx::query_as(
        r#"
        SELECT id, result_id, storage_ref, sha256, mime_type, size_bytes, redacted,
               retention_until, created_at
        FROM evidence
        WHERE result_id = ANY($1)
        ORDER BY created_at, id
        "#,
    )
    .bind(&result_ids)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)?;

    let mut bundles: Vec<ResultBundle> = Vec::with_capacity(results.len());
    for result in results {
        let evidence_for_result: Vec<EvidenceRow> = evidence
            .iter()
            .filter(|row| row.result_id == result.id)
            .cloned()
            .collect();
        bundles.push(ResultBundle {
            result,
            evidence: evidence_for_result,
        });
    }
    Ok(bundles)
}

async fn load_recommendations(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<RecommendationRow>, ApiError> {
    sqlx::query_as(
        r#"
        SELECT id, result_id, evidence_id, run_metric_id, kind, severity, title, body,
               rationale, confidence::float8 AS confidence, generated_by, created_at
        FROM recommendations
        WHERE run_id = $1
        ORDER BY created_at, id
        LIMIT $2
        "#,
    )
    .bind(run_id)
    .bind(MAX_RECOMMENDATIONS_PER_RUN)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_metric_summary(pool: &PgPool, run_id: Uuid) -> Result<MetricSummaryRow, ApiError> {
    sqlx::query_as(
        r#"
        SELECT
            COALESCE(sum(request_count), 0)::bigint AS request_count,
            COALESCE(sum(error_count), 0)::bigint AS error_count,
            COALESCE(sum(response_bytes), 0)::bigint AS response_bytes,
            COALESCE(max(latency_p95_ms), 0)::float8 AS latency_p95_ms,
            COALESCE(
                sum(request_count)::float8
                    / GREATEST(
                        EXTRACT(EPOCH FROM (max(bucket_start) - min(bucket_start))),
                        1
                    )::float8,
                0
            )::float8 AS throughput_rps
        FROM run_metrics
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_metric_buckets(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<MetricBucketRow>, ApiError> {
    sqlx::query_as(
        r#"
        SELECT id, bucket_start, bucket_seconds, endpoint, status_code, request_count,
               error_count, latency_p95_ms::float8 AS latency_p95_ms,
               throughput_rps::float8 AS throughput_rps, response_bytes
        FROM run_metrics
        WHERE run_id = $1
        ORDER BY bucket_start, endpoint, status_code
        LIMIT $2
        "#,
    )
    .bind(run_id)
    .bind(MAX_REPORT_BUCKETS)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_scenario(pool: &PgPool, scenario_id: Uuid) -> Result<Option<ScenarioRow>, ApiError> {
    sqlx::query_as(
        r#"
        SELECT s.id AS scenario_id, s.name AS scenario_name, p.virtual_users,
               p.arrival_rate_per_second::float8 AS arrival_rate_per_second,
               p.ramp_up_seconds, p.duration_seconds, p.max_requests,
               p.max_requests_per_second::float8 AS max_requests_per_second
        FROM load_scenarios s
        JOIN load_profiles p ON p.id = s.profile_id
        WHERE s.id = $1
        "#,
    )
    .bind(scenario_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_target_origin(
    pool: &PgPool,
    authorization_id: Uuid,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar(
        r#"
        SELECT t.normalized_origin
        FROM target_authorizations ta
        JOIN targets t ON t.id = ta.target_id
        WHERE ta.id = $1
        "#,
    )
    .bind(authorization_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_report_row(
    pool: &PgPool,
    run_id: Uuid,
    report_type: &str,
) -> Result<Option<ReportRow>, ApiError> {
    sqlx::query_as(
        r#"
        SELECT id, report_type, status, current_version, last_error_code,
               created_at, updated_at
        FROM reports
        WHERE run_id = $1 AND report_type = $2
        "#,
    )
    .bind(run_id)
    .bind(report_type)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_report_versions(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<ReportVersionMetaRow>, ApiError> {
    sqlx::query_as(
        r#"
        SELECT id, version, status, content_hash, summary, generated_by, created_at
        FROM report_versions
        WHERE run_id = $1
        ORDER BY version DESC
        LIMIT $2
        "#,
    )
    .bind(run_id)
    .bind(MAX_REPORT_VERSIONS)
    .fetch_all(pool)
    .await
    .map_err(ApiError::database)
}

async fn load_report_dto(
    pool: &PgPool,
    run_id: Uuid,
    report_type: &str,
) -> Result<Option<ReportDto>, ApiError> {
    let Some(report) = load_report_row(pool, run_id, report_type).await? else {
        return Ok(None);
    };
    let versions = load_report_versions(pool, run_id).await?;
    let latest: Option<ReportVersionFullRow> = sqlx::query_as(
        r#"
        SELECT id, version, status, content, content_hash, summary, generated_by, created_at
        FROM report_versions
        WHERE run_id = $1
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::database)?;

    Ok(Some(ReportDto {
        id: report.id,
        run_id,
        report_type: report.report_type,
        status: report.status,
        current_version: report.current_version,
        last_error_code: report.last_error_code,
        created_at: report.created_at,
        updated_at: report.updated_at,
        latest: latest.map(|row| ReportVersionFullDto {
            id: row.id,
            version: row.version,
            status: row.status,
            content: row.content,
            content_hash: row.content_hash,
            summary: row.summary,
            generated_by: row.generated_by,
            created_at: row.created_at,
        }),
        versions: versions
            .into_iter()
            .map(|row| ReportVersionMetaDto {
                id: row.id,
                version: row.version,
                status: row.status,
                content_hash: row.content_hash,
                summary: row.summary,
                generated_by: row.generated_by,
                created_at: row.created_at,
            })
            .collect(),
    }))
}

async fn load_run_detail(pool: &PgPool, run: &RunRow) -> Result<RunDetailDto, ApiError> {
    let counts = load_result_counts(pool, run.id).await?;
    let bundles = load_result_bundles(pool, run.id).await?;
    let recommendations = load_recommendations(pool, run.id).await?;
    let metrics = load_metric_summary(pool, run.id).await?;
    let scenario = match run.load_scenario_id {
        Some(id) => load_scenario(pool, id).await?,
        None => None,
    };
    let report_type = report_type_for(&run.run_type);
    let report = load_report_dto(pool, run.id, report_type).await?;
    let target_origin = load_target_origin(pool, run.target_authorization_id).await?;

    let results = bundles
        .iter()
        .map(|bundle| TestResultDto {
            id: bundle.result.id,
            catalog_test_id: bundle.result.catalog_test_id.clone(),
            result_key: bundle.result.result_key.clone(),
            outcome: bundle.result.outcome.clone(),
            finding_status: bundle.result.finding_status.clone(),
            severity: bundle.result.severity.clone(),
            confidence: bundle.result.confidence,
            summary: bundle.result.summary.clone(),
            details: redact_json(bundle.result.details.clone()),
            observation: redact_json(bundle.result.observation.clone()),
            tool_name: bundle.result.tool_name.clone(),
            tool_version: bundle.result.tool_version.clone(),
            duration_ms: bundle.result.duration_ms,
            created_at: bundle.result.created_at,
            evidence: bundle.evidence.iter().map(EvidenceDto::from).collect(),
        })
        .collect();

    Ok(RunDetailDto {
        run: RunSummaryDto {
            id: run.id,
            project_id: run.project_id,
            job_id: run.job_id,
            run_type: run.run_type.clone(),
            name: run.name.clone(),
            status: run.status.clone(),
            plan_id: run.plan_id,
            plan_version: run.plan_version,
            started_at: run.started_at,
            finished_at: run.finished_at,
            created_at: run.created_at,
            result_counts: ResultCountsDto::from(&counts),
            report_status: report.as_ref().map(|report| report.status.clone()),
        },
        report_type: report_type.to_owned(),
        target: TargetRefDto {
            authorization_id: run.target_authorization_id,
            origin: target_origin,
        },
        scenario: scenario.as_ref().map(scenario_dto),
        results,
        recommendations: recommendations
            .iter()
            .map(RecommendationDto::from)
            .collect(),
        metrics: MetricSummaryDto {
            request_count: metrics.request_count,
            error_count: metrics.error_count,
            response_bytes: metrics.response_bytes,
            latency_p95_ms: metrics.latency_p95_ms,
            throughput_rps: metrics.throughput_rps,
        },
        report,
    })
}

async fn build_comparison_side(pool: &PgPool, run: &RunRow) -> Result<ComparisonSideDto, ApiError> {
    let counts = load_result_counts(pool, run.id).await?;
    let metrics = load_metric_summary(pool, run.id).await?;
    let scenario = match run.load_scenario_id {
        Some(id) => load_scenario(pool, id).await?,
        None => None,
    };
    let target_origin = load_target_origin(pool, run.target_authorization_id).await?;
    Ok(ComparisonSideDto {
        run_id: run.id,
        run_type: run.run_type.clone(),
        name: run.name.clone(),
        status: run.status.clone(),
        plan_version: run.plan_version,
        scenario_id: scenario.as_ref().map(|row| row.scenario_id),
        scenario_name: scenario.as_ref().map(|row| row.scenario_name.clone()),
        target_origin,
        profile: scenario.as_ref().map(scenario_dto),
        started_at: run.started_at,
        finished_at: run.finished_at,
        created_at: run.created_at,
        result_counts: ResultCountsDto::from(&counts),
        metrics: MetricSummaryDto {
            request_count: metrics.request_count,
            error_count: metrics.error_count,
            response_bytes: metrics.response_bytes,
            latency_p95_ms: metrics.latency_p95_ms,
            throughput_rps: metrics.throughput_rps,
        },
    })
}

fn scenario_dto(row: &ScenarioRow) -> ScenarioDto {
    ScenarioDto {
        id: row.scenario_id,
        name: row.scenario_name.clone(),
        virtual_users: row.virtual_users,
        arrival_rate_per_second: row.arrival_rate_per_second,
        ramp_up_seconds: row.ramp_up_seconds,
        duration_seconds: row.duration_seconds,
        max_requests: row.max_requests,
        max_requests_per_second: row.max_requests_per_second,
    }
}

// ---------------------------------------------------------------------------
// Report generation
// ---------------------------------------------------------------------------

struct GeneratedRecommendation {
    kind: &'static str,
    severity: String,
    title: String,
    body: String,
    rationale: Option<String>,
    confidence: Option<f64>,
    result_id: Option<Uuid>,
    evidence_id: Option<Uuid>,
    run_metric_id: Option<Uuid>,
}

async fn build_and_store_report(
    pool: &PgPool,
    run: &RunRow,
    user_id: Uuid,
) -> Result<ReportDto, ApiError> {
    let report_type = report_type_for(&run.run_type).to_owned();
    let counts = load_result_counts(pool, run.id).await?;
    let bundles = load_result_bundles(pool, run.id).await?;
    let metrics = load_metric_summary(pool, run.id).await?;
    let buckets = load_metric_buckets(pool, run.id).await?;
    let scenario = match run.load_scenario_id {
        Some(id) => load_scenario(pool, id).await?,
        None => None,
    };
    let target_origin = load_target_origin(pool, run.target_authorization_id).await?;

    let generated = generate_recommendations(run, &counts, &bundles, &metrics, &buckets);
    let existing_recommendations = recommendations_from_generated(&generated);
    let content = build_report_content(
        run,
        &report_type,
        target_origin.as_deref(),
        scenario.as_ref(),
        &counts,
        &bundles,
        &existing_recommendations,
        &metrics,
        &buckets,
    );
    let content = redact_json(content);
    let bytes = serde_json::to_vec(&content)
        .map_err(|_| ApiError::internal("report content could not be serialized"))?;
    if bytes.len() > MAX_REPORT_CONTENT_BYTES {
        mark_report_failed(pool, run, user_id, &report_type, "report_content_too_large").await?;
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "report_content_too_large",
            "The report exceeded the platform size cap; stored results were preserved.",
        ));
    }
    let content_hash = hex(&Sha256::digest(&bytes));
    let summary = report_summary(run, &counts, &metrics);

    let mut transaction = pool.begin().await.map_err(ApiError::database)?;
    let (report_id, current_version): (Uuid, i32) = sqlx::query_as(
        r#"
        INSERT INTO reports (project_id, run_id, report_type, status, current_version, created_by)
        VALUES ($1, $2, $3, 'draft', 0, $4)
        ON CONFLICT (run_id, report_type) DO UPDATE SET updated_at = now()
        RETURNING id, current_version
        "#,
    )
    .bind(run.project_id)
    .bind(run.id)
    .bind(&report_type)
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    sqlx::query("DELETE FROM recommendations WHERE run_id = $1 AND generated_by = 'system'")
        .bind(run.id)
        .execute(&mut *transaction)
        .await
        .map_err(ApiError::database)?;
    for recommendation in &generated {
        insert_recommendation(&mut transaction, run, recommendation).await?;
    }

    let version = current_version + 1;
    sqlx::query(
        r#"
        INSERT INTO report_versions (
            project_id, report_id, run_id, version, status, content, content_hash,
            summary, generated_by
        ) VALUES ($1, $2, $3, $4, 'final', $5, $6, $7, 'system')
        "#,
    )
    .bind(run.project_id)
    .bind(report_id)
    .bind(run.id)
    .bind(version)
    .bind(&content)
    .bind(&content_hash)
    .bind(&summary)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    sqlx::query(
        "UPDATE reports SET status = 'final', current_version = $2, last_error_code = NULL WHERE id = $1",
    )
    .bind(report_id)
    .bind(version)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(run.project_id),
        Some(user_id),
        "report.generated",
        "report",
        Some(report_id),
        json!({ "runId": run.id, "reportType": report_type, "version": version }),
    )
    .await
    .map_err(ApiError::database)?;
    // The report is now visible from the run stream; the Run section closes
    // over this event to drop the review gate without a manual refresh.
    append_run_and_job_event(
        &mut transaction,
        run.id,
        run.job_id,
        "report.completed",
        json!({ "reportId": report_id, "reportType": report_type, "version": version }),
    )
    .await
    .map_err(queue_error)?;
    transaction.commit().await.map_err(ApiError::database)?;

    load_report_dto(pool, run.id, &report_type)
        .await?
        .ok_or_else(|| ApiError::internal("report missing after generation"))
}

async fn insert_recommendation(
    transaction: &mut Transaction<'_, Postgres>,
    run: &RunRow,
    recommendation: &GeneratedRecommendation,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO recommendations (
            project_id, run_id, result_id, evidence_id, run_metric_id, kind, severity,
            title, body, rationale, confidence, generated_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'system')
        "#,
    )
    .bind(run.project_id)
    .bind(run.id)
    .bind(recommendation.result_id)
    .bind(recommendation.evidence_id)
    .bind(recommendation.run_metric_id)
    .bind(recommendation.kind)
    .bind(&recommendation.severity)
    .bind(&recommendation.title)
    .bind(&recommendation.body)
    .bind(&recommendation.rationale)
    .bind(recommendation.confidence)
    .execute(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

async fn mark_report_failed(
    pool: &PgPool,
    run: &RunRow,
    user_id: Uuid,
    report_type: &str,
    code: &str,
) -> Result<(), ApiError> {
    let mut transaction = pool.begin().await.map_err(ApiError::database)?;
    let report_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO reports (
            project_id, run_id, report_type, status, current_version, last_error_code, created_by
        ) VALUES ($1, $2, $3, 'failed', 0, $4, $5)
        ON CONFLICT (run_id, report_type)
        DO UPDATE SET status = 'failed', last_error_code = $4, updated_at = now()
        RETURNING id
        "#,
    )
    .bind(run.project_id)
    .bind(run.id)
    .bind(report_type)
    .bind(code)
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    audit::record_in_tx(
        &mut transaction,
        Some(run.project_id),
        Some(user_id),
        "report.failed",
        "report",
        Some(report_id),
        json!({ "runId": run.id, "reportType": report_type, "errorCode": code }),
    )
    .await
    .map_err(ApiError::database)?;
    append_run_and_job_event(
        &mut transaction,
        run.id,
        run.job_id,
        "report.failed",
        json!({ "reportId": report_id, "reportType": report_type, "errorCode": code }),
    )
    .await
    .map_err(queue_error)?;
    transaction.commit().await.map_err(ApiError::database)?;
    tracing::warn!(
        event = "report.failed",
        run_id = %run.id,
        project_id = %run.project_id,
        report_type,
        error_code = code,
        "report generation failed; stored results were preserved"
    );
    Ok(())
}

fn generate_recommendations(
    run: &RunRow,
    counts: &ResultCountsRow,
    bundles: &[ResultBundle],
    metrics: &MetricSummaryRow,
    buckets: &[MetricBucketRow],
) -> Vec<GeneratedRecommendation> {
    let mut recommendations = Vec::new();
    if run.run_type == "load_testing" {
        if let Some(bucket) = buckets.iter().max_by_key(|bucket| {
            (
                bucket.error_count,
                bucket.latency_p95_ms.unwrap_or(0.0) as i64,
            )
        }) {
            if metrics.error_count > 0 {
                recommendations.push(GeneratedRecommendation {
                    kind: "observed",
                    severity: "medium".to_owned(),
                    title: "Load run recorded error responses".to_owned(),
                    body: format!(
                        "{} of {} requests returned an unexpected status or failed an assertion. The worst endpoint was {} (status {}).",
                        metrics.error_count, metrics.request_count, bucket.endpoint, bucket.status_code
                    ),
                    rationale: None,
                    confidence: None,
                    result_id: None,
                    evidence_id: None,
                    run_metric_id: Some(bucket.id),
                });
            }
            if metrics.latency_p95_ms > 0.0 {
                recommendations.push(GeneratedRecommendation {
                    kind: "hypothesis",
                    severity: "info".to_owned(),
                    title: "Latency may reflect target capacity".to_owned(),
                    body: format!(
                        "The observed p95 latency was {:.0} ms for endpoint {}. The root cause is unknown; compare against a baseline run before drawing conclusions.",
                        metrics.latency_p95_ms, bucket.endpoint
                    ),
                    rationale: Some(
                        "No baseline run is attached, so this is a hypothesis about cause, not an observed metric."
                            .to_owned(),
                    ),
                    confidence: None,
                    result_id: None,
                    evidence_id: None,
                    run_metric_id: None,
                });
            }
        }
    } else {
        for bundle in bundles {
            if bundle.result.outcome == "failed" {
                let evidence_id = bundle.evidence.first().map(|row| row.id);
                recommendations.push(GeneratedRecommendation {
                    kind: "observed",
                    severity: bundle
                        .result
                        .severity
                        .clone()
                        .unwrap_or_else(|| "medium".to_owned()),
                    title: format!("Review finding {}", bundle.result.catalog_test_id),
                    body: bundle.result.summary.clone(),
                    rationale: None,
                    confidence: bundle.result.confidence,
                    result_id: Some(bundle.result.id),
                    evidence_id,
                    run_metric_id: None,
                });
            }
        }
    }

    if counts.error > 0 || counts.inconclusive > 0 {
        recommendations.push(GeneratedRecommendation {
            kind: "hypothesis",
            severity: "info".to_owned(),
            title: "Incomplete execution may hide results".to_owned(),
            body: format!(
                "{} checks errored and {} were inconclusive. Those checks must be rerun before relying on the summary.",
                counts.error, counts.inconclusive
            ),
            rationale: Some(
                "The cause of the incomplete checks is not established from stored evidence.".to_owned(),
            ),
            confidence: None,
            result_id: None,
            evidence_id: None,
            run_metric_id: None,
        });
    }
    recommendations
}

fn recommendations_from_generated(generated: &[GeneratedRecommendation]) -> Vec<RecommendationRow> {
    // Build in-memory recommendation rows for report content. Persistent IDs are
    // assigned by the database; the report content references result/evidence
    // and metric IDs directly, which are stable.
    generated
        .iter()
        .map(|recommendation| RecommendationRow {
            id: Uuid::nil(),
            result_id: recommendation.result_id,
            evidence_id: recommendation.evidence_id,
            run_metric_id: recommendation.run_metric_id,
            kind: recommendation.kind.to_owned(),
            severity: recommendation.severity.to_owned(),
            title: recommendation.title.clone(),
            body: recommendation.body.clone(),
            rationale: recommendation.rationale.clone(),
            confidence: recommendation.confidence,
            generated_by: "system".to_owned(),
            created_at: Utc::now(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_report_content(
    run: &RunRow,
    report_type: &str,
    target_origin: Option<&str>,
    scenario: Option<&ScenarioRow>,
    counts: &ResultCountsRow,
    bundles: &[ResultBundle],
    recommendations: &[RecommendationRow],
    metrics: &MetricSummaryRow,
    buckets: &[MetricBucketRow],
) -> Value {
    let findings: Vec<Value> = bundles
        .iter()
        .filter(|bundle| bundle.result.outcome == "failed")
        .map(|bundle| {
            json!({
                "resultId": bundle.result.id,
                "catalogTestId": bundle.result.catalog_test_id,
                "findingStatus": bundle.result.finding_status,
                "severity": bundle.result.severity,
                "confidence": bundle.result.confidence,
                "summary": bundle.result.summary,
                "observation": bundle.result.observation,
                "evidenceIds": bundle.evidence.iter().map(|row| row.id).collect::<Vec<_>>(),
            })
        })
        .collect();

    let checks: Vec<Value> = bundles
        .iter()
        .filter(|bundle| bundle.result.outcome != "failed")
        .map(|bundle| {
            json!({
                "resultId": bundle.result.id,
                "catalogTestId": bundle.result.catalog_test_id,
                "outcome": bundle.result.outcome,
                "summary": bundle.result.summary,
                "observation": bundle.result.observation,
            })
        })
        .collect();

    let mut claims: Vec<Value> = Vec::new();
    for bundle in bundles {
        if bundle.result.outcome == "failed" {
            claims.push(json!({
                "claimKind": "observed",
                "text": bundle.result.summary,
                "references": {
                    "resultIds": [bundle.result.id],
                    "evidenceIds": bundle.evidence.iter().map(|row| row.id).collect::<Vec<_>>(),
                    "metricIds": [],
                },
            }));
        }
    }
    for bucket in buckets {
        if bucket.error_count > 0 {
            claims.push(json!({
                "claimKind": "observed",
                "text": format!(
                    "Endpoint {} returned {} errors for status {}.",
                    bucket.endpoint, bucket.error_count, bucket.status_code
                ),
                "references": {
                    "resultIds": [],
                    "evidenceIds": [],
                    "metricIds": [bucket.id],
                },
            }));
        }
    }
    for recommendation in recommendations {
        if recommendation.kind == "hypothesis" {
            claims.push(json!({
                "claimKind": "hypothesis",
                "text": recommendation.body,
                "references": {
                    "resultIds": [],
                    "evidenceIds": [],
                    "metricIds": [],
                },
            }));
        }
    }

    json!({
        "schemaVersion": "1",
        "reportType": report_type,
        "generatedAt": Utc::now(),
        "run": {
            "id": run.id,
            "runType": run.run_type,
            "status": run.status,
            "planId": run.plan_id,
            "planVersion": run.plan_version,
            "targetAuthorizationId": run.target_authorization_id,
            "targetOrigin": target_origin,
            "startedAt": run.started_at,
            "finishedAt": run.finished_at,
            "createdAt": run.created_at,
        },
        "scenario": scenario.map(|row| json!({
            "id": row.scenario_id,
            "name": row.scenario_name,
            "planVersion": run.plan_version,
            "profile": {
                "virtualUsers": row.virtual_users,
                "arrivalRatePerSecond": row.arrival_rate_per_second,
                "rampUpSeconds": row.ramp_up_seconds,
                "durationSeconds": row.duration_seconds,
                "maxRequests": row.max_requests,
                "maxRequestsPerSecond": row.max_requests_per_second,
            },
        })),
        "summary": {
            "resultCounts": ResultCountsDto::from(counts),
            "findingCounts": {
                "confirmed": counts.confirmed,
                "suspected": counts.suspected,
            },
            "metrics": {
                "requestCount": metrics.request_count,
                "errorCount": metrics.error_count,
                "responseBytes": metrics.response_bytes,
                "latencyP95Ms": metrics.latency_p95_ms,
                "throughputRps": metrics.throughput_rps,
            },
        },
        "findings": findings,
        "checks": checks,
        "loadObservations": buckets.iter().map(|bucket| json!({
            "metricId": bucket.id,
            "bucketStart": bucket.bucket_start,
            "bucketSeconds": bucket.bucket_seconds,
            "endpoint": bucket.endpoint,
            "statusCode": bucket.status_code,
            "requestCount": bucket.request_count,
            "errorCount": bucket.error_count,
            "latencyP95Ms": bucket.latency_p95_ms,
            "throughputRps": bucket.throughput_rps,
            "responseBytes": bucket.response_bytes,
        })).collect::<Vec<_>>(),
        "recommendations": recommendations.iter().map(|row| json!({
            "kind": row.kind,
            "severity": row.severity,
            "title": row.title,
            "body": row.body,
            "rationale": row.rationale,
            "confidence": row.confidence,
            "resultId": row.result_id,
            "evidenceId": row.evidence_id,
            "runMetricId": row.run_metric_id,
        })).collect::<Vec<_>>(),
        "claims": claims,
    })
}

fn report_summary(run: &RunRow, counts: &ResultCountsRow, metrics: &MetricSummaryRow) -> String {
    if run.run_type == "load_testing" {
        format!(
            "Load run {}: {} requests, {} errors, p95 {:.0} ms.",
            run.status, metrics.request_count, metrics.error_count, metrics.latency_p95_ms
        )
    } else {
        format!(
            "Security run {}: {} confirmed, {} suspected, {} passed, {} errored.",
            run.status, counts.confirmed, counts.suspected, counts.passed, counts.error
        )
    }
}

fn report_type_for(run_type: &str) -> &'static str {
    if run_type == "load_testing" {
        "load"
    } else {
        "security"
    }
}

fn require_report_writer(role: ProjectRole) -> Result<(), ApiError> {
    if matches!(role, ProjectRole::Viewer) {
        tracing::warn!(
            event = "report.generate_forbidden",
            "report generation denied: viewers cannot generate reports"
        );
        Err(ApiError::forbidden(
            "forbidden",
            "Viewers cannot generate reports.",
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Markdown export
// ---------------------------------------------------------------------------

fn render_markdown(content: &Value) -> String {
    let mut out = String::new();
    out.push_str("# ProbeForge report\n\n");
    if let Some(run) = content.get("run") {
        out.push_str(&format!(
            "- Run: `{}` ({})\n- Status: `{}`\n- Plan version: {}\n- Target: {}\n- Started: {}\n- Finished: {}\n\n",
            run.get("id").and_then(Value::as_str).unwrap_or("unknown"),
            run.get("runType").and_then(Value::as_str).unwrap_or("unknown"),
            run.get("status").and_then(Value::as_str).unwrap_or("unknown"),
            run.get("planVersion").and_then(Value::as_i64).unwrap_or(0),
            run.get("targetOrigin").and_then(Value::as_str).unwrap_or("unknown"),
            run.get("startedAt").and_then(Value::as_str).unwrap_or("unknown"),
            run.get("finishedAt").and_then(Value::as_str).unwrap_or("unknown"),
        ));
    }

    if let Some(summary) = content.get("summary") {
        out.push_str("## Summary\n\n");
        out.push_str(&format!("```json\n{}\n```\n\n", summary));
    }

    for (key, heading) in [
        ("findings", "Findings"),
        ("checks", "Checks"),
        ("loadObservations", "Load observations"),
        ("recommendations", "Recommendations"),
        ("claims", "Claims"),
    ] {
        if let Some(items) = content.get(key).and_then(Value::as_array) {
            out.push_str(&format!("## {heading}\n\n"));
            if items.is_empty() {
                out.push_str("_None._\n\n");
                continue;
            }
            for item in items {
                out.push_str(&format!("- {}\n", summarize_item(item)));
            }
            out.push('\n');
        }
    }
    out
}

fn summarize_item(item: &Value) -> String {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return text.to_owned();
    }
    if let Some(summary) = item.get("summary").and_then(Value::as_str) {
        return summary.to_owned();
    }
    if let Some(title) = item.get("title").and_then(Value::as_str) {
        let body = item.get("body").and_then(Value::as_str).unwrap_or("");
        let kind = item.get("kind").and_then(Value::as_str).unwrap_or("note");
        return format!("[{kind}] {title}: {body}");
    }
    item.to_string()
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use probe_forge_observability::REDACTED;

    fn run_row(run_type: &str) -> RunRow {
        RunRow {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            plan_version: 2,
            run_type: run_type.to_owned(),
            name: "Fixture test".to_owned(),
            status: "completed".to_owned(),
            load_scenario_id: None,
            target_authorization_id: Uuid::new_v4(),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            created_at: Utc::now(),
            role: "owner".to_owned(),
        }
    }

    fn counts(failed: i64) -> ResultCountsRow {
        ResultCountsRow {
            passed: 1,
            failed,
            skipped: 0,
            inconclusive: 0,
            error: 0,
            cancelled: 0,
            confirmed: failed,
            suspected: 0,
        }
    }

    #[test]
    fn redaction_removes_credentials_from_nested_content() {
        let value = json!({
            "details": {
                "authorization": "Bearer super-secret-token",
                "note": "sent Bearer abc123 to the target",
            },
            "list": ["cookie: session=deadbeef"],
        });
        let redacted = redact_json(value);
        let rendered = redacted.to_string();
        assert!(!rendered.contains("super-secret-token"));
        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("deadbeef"));
        assert!(rendered.contains(REDACTED));
    }

    #[test]
    fn report_claims_are_traceable_or_hypotheses() {
        let run = run_row("load_testing");
        let bucket = MetricBucketRow {
            id: Uuid::new_v4(),
            bucket_start: Utc::now(),
            bucket_seconds: 10,
            endpoint: "baseline".to_owned(),
            status_code: 500,
            request_count: 10,
            error_count: 3,
            latency_p95_ms: Some(250.0),
            throughput_rps: Some(1.0),
            response_bytes: 100,
        };
        let metrics = MetricSummaryRow {
            request_count: 10,
            error_count: 3,
            response_bytes: 100,
            latency_p95_ms: 250.0,
            throughput_rps: 1.0,
        };
        let generated = generate_recommendations(
            &run,
            &counts(0),
            &[],
            &metrics,
            std::slice::from_ref(&bucket),
        );
        let recommendations = recommendations_from_generated(&generated);
        let content = redact_json(build_report_content(
            &run,
            "load",
            Some("http://127.0.0.1:9000"),
            None,
            &counts(0),
            &[],
            &recommendations,
            &metrics,
            &[bucket],
        ));
        let claims = content["claims"].as_array().expect("claims");
        assert!(!claims.is_empty());
        for claim in claims {
            let kind = claim["claimKind"].as_str().expect("claim kind");
            if kind == "hypothesis" {
                continue;
            }
            let references = &claim["references"];
            let total = references["resultIds"].as_array().map_or(0, Vec::len)
                + references["evidenceIds"].as_array().map_or(0, Vec::len)
                + references["metricIds"].as_array().map_or(0, Vec::len);
            assert!(total > 0, "observed claim has no stored reference: {claim}");
        }
    }

    #[test]
    fn markdown_export_contains_run_metadata_and_observations() {
        let run = run_row("security");
        let content = json!({
            "run": {
                "id": run.id,
                "runType": "security",
                "status": "completed",
                "planVersion": 3,
                "targetOrigin": "https://inert.example.test",
                "startedAt": "2026-10-24T00:00:00Z",
                "finishedAt": "2026-10-24T00:05:00Z",
            },
            "summary": {"findingCounts": {"confirmed": 1}},
            "findings": [{"summary": "Missing header observed"}],
            "checks": [],
            "loadObservations": [],
            "recommendations": [],
            "claims": [],
        });
        let markdown = render_markdown(&content);
        assert!(markdown.contains("https://inert.example.test"));
        assert!(markdown.contains("Missing header observed"));
        assert!(markdown.contains("planVersion") || markdown.contains("Plan version"));
    }

    #[test]
    fn report_type_maps_by_run_type() {
        assert_eq!(report_type_for("load_testing"), "load");
        assert_eq!(report_type_for("vulnerability_testing"), "security");
    }

    #[test]
    fn hex_encodes_sha256_output() {
        let hash = hex(&Sha256::digest(b"probeforge"));
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|character| character.is_ascii_hexdigit()));
    }
}
