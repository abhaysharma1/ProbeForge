use std::{convert::Infallible, time::Duration};

use async_stream::stream;
use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_core::Stream;
use probe_forge_queue::{QueueError, append_job_event};
use probeforge_contracts::{Intensity, JobCreation, JobGoal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthContext,
    error::{ApiError, ApiPath, ApiQuery, ContractJson},
    projects::{ProjectRole, load_project_role},
};

const MAX_LISTED_JOBS: i64 = 100;
const MAX_EVENT_BATCH: i64 = 200;
const MAX_IDEMPOTENCY_KEY_LEN: usize = 200;
const EVENT_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/jobs", post(create_job))
        .route("/jobs/{job_id}", get(get_job))
        .route("/jobs/{job_id}/cancel", post(cancel_job))
        .route("/jobs/{job_id}/events", get(job_events))
        .route("/projects/{project_id}/jobs", get(list_jobs))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateJobQuery {
    project_id: Uuid,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventQuery {
    after: Option<i64>,
    follow: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobGitDto {
    provider: String,
    repository_url: String,
    reference: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobDto {
    id: Uuid,
    project_id: Uuid,
    created_by: Uuid,
    goal: String,
    intensity: String,
    status: String,
    priority: i16,
    display_name: String,
    git: JobGitDto,
    source_reference: String,
    current_commit_sha: Option<String>,
    current_plan_id: Option<Uuid>,
    attempt: i32,
    max_attempts: i32,
    worker_id: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    heartbeat_at: Option<DateTime<Utc>>,
    last_error_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct JobRow {
    id: Uuid,
    project_id: Uuid,
    created_by: Uuid,
    goal: String,
    intensity: String,
    status: String,
    priority: i16,
    display_name: String,
    source_reference: String,
    current_commit_sha: Option<String>,
    current_plan_id: Option<Uuid>,
    attempt: i32,
    max_attempts: i32,
    worker_id: Option<String>,
    lease_expires_at: Option<DateTime<Utc>>,
    heartbeat_at: Option<DateTime<Utc>>,
    last_error_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    git_provider: String,
    repository_url: String,
    default_reference: String,
    credential_ref: Option<String>,
    role: Option<String>,
}

#[derive(Debug, FromRow)]
struct JobEventRow {
    sequence: i64,
    event_type: String,
    payload: Value,
    occurred_at: DateTime<Utc>,
}

async fn create_job(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<CreateJobQuery>,
    headers: HeaderMap,
    ContractJson(payload): ContractJson<JobCreation>,
) -> Result<Response, ApiError> {
    let role = load_project_role(&state.pool, context.user.id, query.project_id).await?;
    if !role.can_create_jobs() {
        tracing::warn!(
            event = "job.create_forbidden",
            project_id = %query.project_id,
            user_id = %context.user.id,
            "job creation denied: insufficient project role"
        );
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to create jobs in this project.",
        ));
    }

    let goal = goal_to_db(payload.goal);
    let intensity = intensity_to_db(payload.intensity);
    let display_name = payload.display_name.trim();
    if display_name.is_empty() {
        return Err(ApiError::invalid(
            "invalid_display_name",
            "Job display name must not be empty.",
        ));
    }

    let repository_url = payload.git.repository_url.trim().to_owned();
    let provider = provider_for(&repository_url)?;
    let source_reference = payload.git.reference.trim().to_owned();
    let credential_ref = payload.git.credential_ref.clone();
    let idempotency_key = read_idempotency_key(&headers)?;

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let git_source_id = upsert_git_source(
        &mut transaction,
        query.project_id,
        provider,
        &repository_url,
        &source_reference,
        credential_ref.as_deref(),
    )
    .await?;

    let inserted: Option<Uuid> = sqlx::query_scalar(
        r#"
        INSERT INTO jobs (
            project_id, created_by, goal, intensity, status, display_name,
            git_source_id, source_reference, idempotency_key
        )
        VALUES ($1, $2, $3, $4, 'queued', $5, $6, $7, $8)
        ON CONFLICT (project_id, idempotency_key)
            WHERE idempotency_key IS NOT NULL
            DO NOTHING
        RETURNING id
        "#,
    )
    .bind(query.project_id)
    .bind(context.user.id)
    .bind(goal)
    .bind(intensity)
    .bind(display_name)
    .bind(git_source_id)
    .bind(&source_reference)
    .bind(&idempotency_key)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    let (status, job_id) = match inserted {
        Some(job_id) => {
            append_job_event(
                &mut transaction,
                job_id,
                "job.queued",
                json!({ "goal": goal, "intensity": intensity }),
            )
            .await
            .map_err(queue_error)?;

            audit::record_in_tx(
                &mut transaction,
                Some(query.project_id),
                Some(context.user.id),
                "job.created",
                "job",
                Some(job_id),
                json!({ "goal": goal, "intensity": intensity }),
            )
            .await
            .map_err(ApiError::database)?;
            tracing::info!(
                event = "job.created",
                job_id = %job_id,
                project_id = %query.project_id,
                user_id = %context.user.id,
                goal,
                intensity,
                "job created and queued"
            );
            (StatusCode::CREATED, job_id)
        }
        None => {
            let existing = find_job_by_idempotency_key(
                &mut transaction,
                query.project_id,
                idempotency_key.as_deref().unwrap_or_default(),
            )
            .await?
            .ok_or_else(|| ApiError::internal("idempotent job lookup failed"))?;

            if existing.goal != goal
                || existing.intensity != intensity
                || existing.display_name != display_name
                || existing.repository_url != repository_url
                || existing.source_reference != source_reference
                || existing.credential_ref != credential_ref
            {
                transaction.rollback().await.map_err(ApiError::database)?;
                tracing::warn!(
                    event = "job.idempotency_conflict",
                    project_id = %query.project_id,
                    user_id = %context.user.id,
                    "idempotency key reused for a different job request"
                );
                return Err(ApiError::conflict(
                    "idempotency_key_reused",
                    "That idempotency key was already used for a different job request.",
                ));
            }

            tracing::debug!(
                event = "job.idempotent_replay",
                job_id = %existing.id,
                project_id = %query.project_id,
                "idempotent job creation returned the existing job"
            );
            (StatusCode::OK, existing.id)
        }
    };

    let row = load_job_row(&mut transaction, context.user.id, job_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok((status, Json(job_from_row(row)?)).into_response())
}

async fn get_job(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(job_id): ApiPath<Uuid>,
) -> Result<Json<JobDto>, ApiError> {
    let (job, _) = load_job_for_user(&state.pool, context.user.id, job_id).await?;
    Ok(Json(job))
}

async fn list_jobs(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<JobDto>>, ApiError> {
    load_project_role(&state.pool, context.user.id, project_id).await?;

    let rows: Vec<JobRow> = sqlx::query_as(
        r#"
        SELECT j.id, j.project_id, j.created_by, j.goal, j.intensity, j.status,
               j.priority, j.display_name, j.source_reference, j.current_commit_sha,
               j.current_plan_id, j.attempt, j.max_attempts, j.worker_id,
               j.lease_expires_at, j.heartbeat_at, j.last_error_code,
               j.created_at, j.updated_at,
               g.provider AS git_provider, g.normalized_url AS repository_url,
               g.default_reference, g.credential_ref, NULL::text AS role
        FROM jobs j
        JOIN git_sources g ON g.id = j.git_source_id
        WHERE j.project_id = $1
        ORDER BY j.created_at DESC
        LIMIT $2
        "#,
    )
    .bind(project_id)
    .bind(MAX_LISTED_JOBS)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;

    rows.into_iter()
        .map(job_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

async fn cancel_job(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(job_id): ApiPath<Uuid>,
) -> Result<Json<JobDto>, ApiError> {
    let (job, role) = load_job_for_user(&state.pool, context.user.id, job_id).await?;
    if !role.can_create_jobs() {
        tracing::warn!(
            event = "job.cancel_forbidden",
            job_id = %job_id,
            project_id = %job.project_id,
            user_id = %context.user.id,
            "job cancellation denied: insufficient project role"
        );
        return Err(ApiError::forbidden(
            "forbidden",
            "You do not have permission to cancel jobs in this project.",
        ));
    }

    let mut transaction = state.pool.begin().await.map_err(ApiError::database)?;
    let current_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1 AND project_id = $2 FOR UPDATE")
            .bind(job_id)
            .bind(job.project_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(ApiError::database)?;

    let Some(current_status) = current_status else {
        return Err(ApiError::not_found("job_not_found", "Job not found."));
    };

    match current_status.as_str() {
        "cancelled" => {
            let row = load_job_row(&mut transaction, context.user.id, job_id).await?;
            transaction.commit().await.map_err(ApiError::database)?;
            tracing::debug!(
                event = "job.cancel_noop",
                job_id = %job_id,
                "job was already cancelled"
            );
            return Ok(Json(job_from_row(row)?));
        }
        "failed" | "expired" => {
            return Err(ApiError::conflict(
                "job_terminal",
                "A failed or expired job cannot be cancelled.",
            ));
        }
        _ => {}
    }

    sqlx::query(
        r#"
        UPDATE jobs
        SET status = 'cancelled',
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = NULL
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::database)?;

    append_job_event(
        &mut transaction,
        job_id,
        "job.cancelled",
        json!({ "cancelledBy": context.user.id }),
    )
    .await
    .map_err(queue_error)?;

    audit::record_in_tx(
        &mut transaction,
        Some(job.project_id),
        Some(context.user.id),
        "job.cancelled",
        "job",
        Some(job_id),
        json!({ "goal": job.goal }),
    )
    .await
    .map_err(ApiError::database)?;

    let row = load_job_row(&mut transaction, context.user.id, job_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    tracing::info!(
        event = "job.cancelled",
        job_id = %job_id,
        project_id = %job.project_id,
        user_id = %context.user.id,
        "job cancelled"
    );
    Ok(Json(job_from_row(row)?))
}

async fn job_events(
    State(state): State<AppState>,
    Extension(context): Extension<AuthContext>,
    ApiPath(job_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<EventQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    load_job_for_user(&state.pool, context.user.id, job_id).await?;

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
            let batch = match load_event_batch(&pool, job_id, cursor, MAX_EVENT_BATCH).await {
                Ok(batch) => batch,
                Err(_) => {
                    yield Ok::<Event, Infallible>(stream_error_event());
                    break;
                }
            };
            let batch_len = batch.len();
            for row in batch {
                cursor = row.sequence;
                let data = json!({
                    "schemaVersion": "1",
                    "jobId": job_id,
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
                if batch_len < MAX_EVENT_BATCH as usize {
                    break;
                }
                continue;
            }

            if batch_len == 0 {
                match job_status(&pool, job_id).await {
                    Ok(Some(status)) if is_terminal_status(&status) => break,
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                    Err(_) => {
                        yield Ok::<Event, Infallible>(stream_error_event());
                        break;
                    }
                }
                tokio::time::sleep(EVENT_POLL_INTERVAL).await;
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn load_job_for_user(
    pool: &PgPool,
    user_id: Uuid,
    job_id: Uuid,
) -> Result<(JobDto, ProjectRole), ApiError> {
    let row = load_job_row_pool(pool, user_id, job_id).await?;
    let role = row
        .role
        .as_deref()
        .and_then(ProjectRole::from_db)
        .ok_or_else(|| ApiError::internal("job membership role is invalid"))?;
    Ok((job_from_row(row)?, role))
}

async fn load_job_row(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    job_id: Uuid,
) -> Result<JobRow, ApiError> {
    load_job_query(transaction, user_id, job_id).await
}

async fn load_job_row_pool(pool: &PgPool, user_id: Uuid, job_id: Uuid) -> Result<JobRow, ApiError> {
    let mut transaction = pool.begin().await.map_err(ApiError::database)?;
    let row = load_job_query(&mut transaction, user_id, job_id).await?;
    transaction.commit().await.map_err(ApiError::database)?;
    Ok(row)
}

async fn load_job_query(
    connection: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    job_id: Uuid,
) -> Result<JobRow, ApiError> {
    let row: Option<JobRow> = sqlx::query_as(
        r#"
        SELECT j.id, j.project_id, j.created_by, j.goal, j.intensity, j.status,
               j.priority, j.display_name, j.source_reference, j.current_commit_sha,
               j.current_plan_id, j.attempt, j.max_attempts, j.worker_id,
               j.lease_expires_at, j.heartbeat_at, j.last_error_code,
               j.created_at, j.updated_at,
               g.provider AS git_provider, g.normalized_url AS repository_url,
               g.default_reference, g.credential_ref, pm.role
        FROM jobs j
        JOIN git_sources g ON g.id = j.git_source_id
        JOIN project_members pm ON pm.project_id = j.project_id AND pm.user_id = $2
        WHERE j.id = $1
        "#,
    )
    .bind(job_id)
    .bind(user_id)
    .fetch_optional(&mut **connection)
    .await
    .map_err(ApiError::database)?;

    row.ok_or_else(|| ApiError::not_found("job_not_found", "Job not found."))
}

async fn find_job_by_idempotency_key(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<JobRow>, ApiError> {
    let row: Option<JobRow> = sqlx::query_as(
        r#"
        SELECT j.id, j.project_id, j.created_by, j.goal, j.intensity, j.status,
               j.priority, j.display_name, j.source_reference, j.current_commit_sha,
               j.current_plan_id, j.attempt, j.max_attempts, j.worker_id,
               j.lease_expires_at, j.heartbeat_at, j.last_error_code,
               j.created_at, j.updated_at,
               g.provider AS git_provider, g.normalized_url AS repository_url,
               g.default_reference, g.credential_ref, NULL::text AS role
        FROM jobs j
        JOIN git_sources g ON g.id = j.git_source_id
        WHERE j.project_id = $1 AND j.idempotency_key = $2
        "#,
    )
    .bind(project_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    Ok(row)
}

async fn upsert_git_source(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    provider: &'static str,
    repository_url: &str,
    default_reference: &str,
    credential_ref: Option<&str>,
) -> Result<Uuid, ApiError> {
    let source_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO git_sources (
            project_id, provider, normalized_url, default_reference, credential_ref
        )
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (project_id, normalized_url)
        DO UPDATE SET
            default_reference = EXCLUDED.default_reference,
            credential_ref = COALESCE(EXCLUDED.credential_ref, git_sources.credential_ref),
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(project_id)
    .bind(provider)
    .bind(repository_url)
    .bind(default_reference)
    .bind(credential_ref)
    .fetch_one(&mut **transaction)
    .await
    .map_err(ApiError::database)?;
    Ok(source_id)
}

fn provider_for(repository_url: &str) -> Result<&'static str, ApiError> {
    let parsed = Url::parse(repository_url)
        .map_err(|_| ApiError::invalid("invalid_repository_url", "Repository URL is not valid."))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    Ok(match host.as_str() {
        "github.com" | "www.github.com" => "github",
        "gitlab.com" | "www.gitlab.com" => "gitlab",
        "bitbucket.org" | "www.bitbucket.org" => "bitbucket",
        _ => "generic",
    })
}

fn read_idempotency_key(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| {
        ApiError::invalid(
            "invalid_idempotency_key",
            "Idempotency-Key must be printable text.",
        )
    })?;
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_IDEMPOTENCY_KEY_LEN
        || value.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(
            "invalid_idempotency_key",
            "Idempotency-Key must be 1-200 printable characters.",
        ));
    }
    Ok(Some(value.to_owned()))
}

fn goal_to_db(goal: JobGoal) -> &'static str {
    match goal {
        JobGoal::VulnerabilityTesting => "vulnerability_testing",
        JobGoal::LoadTesting => "load_testing",
    }
}

fn intensity_to_db(intensity: Intensity) -> &'static str {
    match intensity {
        Intensity::Low => "low",
        Intensity::Standard => "standard",
        Intensity::High => "high",
    }
}

fn job_from_row(row: JobRow) -> Result<JobDto, ApiError> {
    Ok(JobDto {
        id: row.id,
        project_id: row.project_id,
        created_by: row.created_by,
        goal: row.goal,
        intensity: row.intensity,
        status: row.status,
        priority: row.priority,
        display_name: row.display_name,
        git: JobGitDto {
            provider: row.git_provider,
            repository_url: row.repository_url,
            reference: row.default_reference,
        },
        source_reference: row.source_reference,
        current_commit_sha: row.current_commit_sha,
        current_plan_id: row.current_plan_id,
        attempt: row.attempt,
        max_attempts: row.max_attempts,
        worker_id: row.worker_id,
        lease_expires_at: row.lease_expires_at,
        heartbeat_at: row.heartbeat_at,
        last_error_code: row.last_error_code,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn load_event_batch(
    pool: &PgPool,
    job_id: Uuid,
    after: i64,
    limit: i64,
) -> Result<Vec<JobEventRow>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT sequence, event_type, payload, occurred_at
        FROM job_events
        WHERE job_id = $1 AND sequence > $2
        ORDER BY sequence ASC
        LIMIT $3
        "#,
    )
    .bind(job_id)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

async fn job_status(pool: &PgPool, job_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(pool)
        .await
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "failed" | "cancelled" | "expired")
}

fn stream_error_event() -> Event {
    Event::default()
        .event("stream.error")
        .data(json!({ "code": "event_stream_failed" }).to_string())
}

fn queue_error(error: QueueError) -> ApiError {
    match error {
        QueueError::NotFound => ApiError::not_found("job_not_found", "Job not found."),
        QueueError::Database(error) => ApiError::database(error),
        other => ApiError::invalid("queue_error", other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_job_roles() {
        assert!(ProjectRole::Owner.can_create_jobs());
        assert!(ProjectRole::Admin.can_create_jobs());
        assert!(ProjectRole::Collaborator.can_create_jobs());
        assert!(!ProjectRole::Viewer.can_create_jobs());
    }

    #[test]
    fn maps_known_git_providers() {
        assert_eq!(
            provider_for("https://github.com/acme/app.git").expect("provider"),
            "github"
        );
        assert_eq!(
            provider_for("ssh://git@gitlab.com/acme/app.git").expect("provider"),
            "gitlab"
        );
        assert_eq!(
            provider_for("https://git.example.test/acme/app.git").expect("provider"),
            "generic"
        );
    }

    #[test]
    fn rejects_invalid_idempotency_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            "job-request-1".parse().expect("header value"),
        );
        assert_eq!(
            read_idempotency_key(&headers).expect("valid"),
            Some("job-request-1".to_owned())
        );

        headers.insert("idempotency-key", "   ".parse().expect("header value"));
        assert!(read_idempotency_key(&headers).is_err());
    }
}
