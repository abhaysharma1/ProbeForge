//! PostgreSQL-backed job and run queue.
//!
//! Claims use short transactions with `FOR UPDATE SKIP LOCKED`, then commit
//! before a worker performs any Git, model, or network work. Long-running work
//! is protected by leases and fenced with the worker ID on every transition.
//! No database row lock is held while work is in progress.

use chrono::{DateTime, Duration, FixedOffset, TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

const MAX_LEASE_SECONDS: i64 = 3_600;
const MAX_WORKER_ID_LEN: usize = 200;
const MAX_ERROR_CODE_LEN: usize = 120;
const MAX_BACKOFF_SECONDS: i64 = 300;
const MAX_QUOTA: i64 = 100;
const MAX_TIMEZONE_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("job not found")]
    NotFound,
    #[error("worker id must be 1-200 printable ASCII characters")]
    InvalidWorkerId,
    #[error("lease duration must be between 1 and 3600 seconds")]
    InvalidLease,
    #[error("error code must be 1-120 printable ASCII characters")]
    InvalidErrorCode,
    #[error("event type is not a valid lifecycle event name")]
    InvalidEventType,
    #[error("limit must be between 1 and 500")]
    InvalidLimit,
    #[error("quota must be between 1 and 100")]
    InvalidQuota,
    #[error("timezone is not supported")]
    InvalidTimezone,
    #[error("recurrence is not supported")]
    InvalidRecurrence,
    #[error("schedule is not eligible for the requested plan, scenario, or target")]
    IneligibleSchedule,
    #[error("schedule start/expiry is invalid")]
    InvalidScheduleWindow,
    #[error("retention policy is outside the allowed range")]
    InvalidRetention,
}

#[derive(Debug, Clone, FromRow)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub project_id: Uuid,
    pub created_by: Uuid,
    pub goal: String,
    pub intensity: String,
    pub status: String,
    pub priority: i16,
    pub display_name: String,
    pub git_source_id: Uuid,
    pub source_reference: String,
    pub attempt: i32,
    pub max_attempts: i32,
    pub lease_expires_at: DateTime<Utc>,
    pub repository_url: String,
    pub provider: String,
    pub credential_ref: Option<String>,
    pub default_reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    Retried {
        attempt: i32,
        retry_at: DateTime<Utc>,
    },
    Failed {
        attempt: i32,
    },
    LeaseLost,
}

#[derive(Debug, Clone, FromRow)]
pub struct PromotedRun {
    pub id: Uuid,
    pub job_id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
pub struct ClaimedRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub job_id: Uuid,
    pub plan_id: Uuid,
    pub plan_version: i32,
    pub plan_approval_id: Uuid,
    pub run_type: String,
    pub status: String,
    pub attempt: i32,
    pub max_attempts: i32,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ScheduleRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub job_id: Uuid,
    pub plan_id: Uuid,
    pub load_scenario_id: Uuid,
    pub load_profile_id: Uuid,
    pub target_authorization_id: Option<Uuid>,
    pub timezone: String,
    pub recurrence: String,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewSchedule {
    pub project_id: Uuid,
    pub job_id: Uuid,
    pub plan_id: Uuid,
    pub load_scenario_id: Uuid,
    pub target_authorization_id: Uuid,
    pub timezone: String,
    pub recurrence: String,
    pub start_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recurrence {
    Once,
    Hourly,
    Daily,
    Weekly,
}

#[derive(Debug, FromRow)]
struct JobFailureRow {
    attempt: i32,
    max_attempts: i32,
}

#[derive(Debug, FromRow)]
struct ExpiredJobRow {
    id: Uuid,
    attempt: i32,
    max_attempts: i32,
}

#[derive(Debug, FromRow)]
struct RunFailureRow {
    job_id: Uuid,
    attempt: i32,
    max_attempts: i32,
}

#[derive(Debug, FromRow)]
struct ExpiredRunRow {
    id: Uuid,
    job_id: Uuid,
    attempt: i32,
    max_attempts: i32,
}

/// Append an ordered event inside the caller's transaction. The job row is
/// locked so concurrent writers cannot allocate the same sequence number.
pub async fn append_job_event(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: Uuid,
    event_type: &str,
    payload: Value,
) -> Result<i64, QueueError> {
    if !valid_event_type(event_type) {
        return Err(QueueError::InvalidEventType);
    }

    let locked: Option<Uuid> = sqlx::query_scalar("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job_id)
        .fetch_optional(&mut **transaction)
        .await?;
    if locked.is_none() {
        return Err(QueueError::NotFound);
    }

    let sequence: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM job_events WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut **transaction)
    .await?;

    sqlx::query(
        "INSERT INTO job_events (job_id, sequence, event_type, payload)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(job_id)
    .bind(sequence)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **transaction)
    .await?;

    Ok(sequence)
}

/// Atomically claim one eligible job. The claim, attempt increment, lease, and
/// `job.claimed` event commit together before the caller does any work.
pub async fn claim_job(
    pool: &PgPool,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<Option<ClaimedJob>, QueueError> {
    validate_worker_id(worker_id)?;
    validate_lease(lease_seconds)?;

    let mut transaction = pool.begin().await?;
    let claimed: Option<ClaimedJob> = sqlx::query_as(
        r#"
        UPDATE jobs AS j
        SET status = 'analyzing',
            worker_id = $1,
            attempt = j.attempt + 1,
            heartbeat_at = now(),
            lease_expires_at = now() + make_interval(secs => $2::double precision),
            last_error_code = NULL
        FROM (
            SELECT id
            FROM jobs
            WHERE status = 'queued'
              AND not_before <= now()
              AND attempt < max_attempts
            ORDER BY priority DESC, not_before ASC, created_at ASC
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        ) AS candidate,
        git_sources AS g
        WHERE j.id = candidate.id
          AND g.id = j.git_source_id
        RETURNING j.id, j.project_id, j.created_by, j.goal, j.intensity, j.status,
                  j.priority, j.display_name, j.git_source_id, j.source_reference,
                  j.attempt, j.max_attempts, j.lease_expires_at,
                  g.normalized_url AS repository_url,
                  g.provider,
                  g.credential_ref,
                  g.default_reference
        "#,
    )
    .bind(worker_id)
    .bind(lease_seconds as f64)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(claimed) = claimed else {
        transaction.rollback().await?;
        return Ok(None);
    };

    append_job_event(
        &mut transaction,
        claimed.id,
        "job.claimed",
        json!({
            "workerId": worker_id,
            "attempt": claimed.attempt,
            "leaseSeconds": lease_seconds,
        }),
    )
    .await?;
    transaction.commit().await?;
    tracing::debug!(
        event = "job.claimed",
        job_id = %claimed.id,
        project_id = %claimed.project_id,
        worker_id,
        attempt = claimed.attempt,
        lease_seconds,
        "claimed queued job"
    );
    Ok(Some(claimed))
}

/// Renew a lease only while the worker still owns an unexpired lease.
pub async fn heartbeat_job(
    pool: &PgPool,
    job_id: Uuid,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<Option<DateTime<Utc>>, QueueError> {
    validate_worker_id(worker_id)?;
    validate_lease(lease_seconds)?;

    let renewed: Option<DateTime<Utc>> = sqlx::query_scalar(
        r#"
        UPDATE jobs
        SET heartbeat_at = now(),
            lease_expires_at = now() + make_interval(secs => $3::double precision)
        WHERE id = $1
          AND worker_id = $2
          AND status IN ('analyzing', 'planning')
          AND lease_expires_at > now()
        RETURNING lease_expires_at
        "#,
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(lease_seconds as f64)
    .fetch_optional(pool)
    .await?;

    if renewed.is_none() {
        tracing::warn!(
            event = "job.heartbeat_lost",
            job_id = %job_id,
            worker_id,
            "job lease heartbeat lost; worker no longer owns an unexpired lease"
        );
    }
    Ok(renewed)
}

/// Complete analysis while the worker still owns an unexpired lease. The plan
/// row is persisted by the planning phase; the queue only fences the state
/// transition.
pub async fn complete_job(
    pool: &PgPool,
    job_id: Uuid,
    worker_id: &str,
    current_plan_id: Option<Uuid>,
) -> Result<bool, QueueError> {
    validate_worker_id(worker_id)?;

    let mut transaction = pool.begin().await?;
    let completed: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE jobs
        SET status = 'awaiting_approval',
            current_plan_id = COALESCE($3, current_plan_id),
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = NULL
        WHERE id = $1
          AND worker_id = $2
          AND status IN ('analyzing', 'planning')
          AND lease_expires_at > now()
        RETURNING id
        "#,
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(current_plan_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(completed) = completed else {
        transaction.rollback().await?;
        tracing::warn!(
            event = "job.lease_lost",
            job_id = %job_id,
            worker_id,
            "job completion fenced out by a newer lease"
        );
        return Ok(false);
    };

    append_job_event(
        &mut transaction,
        completed,
        "plan.proposed",
        json!({ "planId": current_plan_id }),
    )
    .await?;
    transaction.commit().await?;
    tracing::info!(
        event = "job.awaiting_approval",
        job_id = %completed,
        worker_id,
        plan_id = ?current_plan_id,
        "job analysis completed and is awaiting approval"
    );
    Ok(true)
}

/// Fail a claimed job. Retryable failures return to `queued` with bounded
/// exponential backoff; exhausted or non-retryable failures become terminal.
pub async fn fail_job(
    pool: &PgPool,
    job_id: Uuid,
    worker_id: &str,
    error_code: &str,
    retryable: bool,
) -> Result<FailOutcome, QueueError> {
    validate_worker_id(worker_id)?;
    validate_error_code(error_code)?;

    let mut transaction = pool.begin().await?;
    let row: Option<JobFailureRow> = sqlx::query_as(
        r#"
        SELECT attempt, max_attempts
        FROM jobs
        WHERE id = $1
          AND worker_id = $2
          AND status IN ('analyzing', 'planning')
          AND lease_expires_at > now()
        FOR UPDATE
        "#,
    )
    .bind(job_id)
    .bind(worker_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(row) = row else {
        transaction.rollback().await?;
        tracing::warn!(
            event = "job.lease_lost",
            job_id = %job_id,
            worker_id,
            "job failure fenced out by a newer lease"
        );
        return Ok(FailOutcome::LeaseLost);
    };

    if retryable && row.attempt < row.max_attempts {
        let retry_at = Utc::now() + Duration::seconds(backoff_seconds(row.attempt));
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'queued',
                worker_id = NULL,
                lease_expires_at = NULL,
                heartbeat_at = NULL,
                not_before = $2,
                last_error_code = $3
            WHERE id = $1
            "#,
        )
        .bind(job_id)
        .bind(retry_at)
        .bind(error_code)
        .execute(&mut *transaction)
        .await?;

        append_job_event(
            &mut transaction,
            job_id,
            "job.retry_scheduled",
            json!({
                "attempt": row.attempt,
                "retryAt": retry_at,
                "errorCode": error_code,
            }),
        )
        .await?;
        transaction.commit().await?;
        tracing::warn!(
            event = "job.retry_scheduled",
            job_id = %job_id,
            worker_id,
            error_code,
            attempt = row.attempt,
            retry_at = %retry_at,
            "job failed and was rescheduled with backoff"
        );
        return Ok(FailOutcome::Retried {
            attempt: row.attempt,
            retry_at,
        });
    }

    sqlx::query(
        r#"
        UPDATE jobs
        SET status = 'failed',
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = $2
        WHERE id = $1
        "#,
    )
    .bind(job_id)
    .bind(error_code)
    .execute(&mut *transaction)
    .await?;

    append_job_event(
        &mut transaction,
        job_id,
        "job.failed",
        json!({
            "attempt": row.attempt,
            "errorCode": error_code,
            "willRetry": false,
        }),
    )
    .await?;
    transaction.commit().await?;
    tracing::warn!(
        event = "job.failed",
        job_id = %job_id,
        worker_id,
        error_code,
        attempt = row.attempt,
        "job failed terminally"
    );
    Ok(FailOutcome::Failed {
        attempt: row.attempt,
    })
}

/// Recover jobs whose worker stopped heartbeating. Expired leases are retried
/// with backoff until `max_attempts`, then marked failed.
pub async fn recover_expired_job_leases(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Uuid>, QueueError> {
    validate_limit(limit)?;

    let mut transaction = pool.begin().await?;
    let rows: Vec<ExpiredJobRow> = sqlx::query_as(
        r#"
        SELECT id, attempt, max_attempts
        FROM jobs
        WHERE status IN ('analyzing', 'planning')
          AND lease_expires_at IS NOT NULL
          AND lease_expires_at <= now()
        ORDER BY lease_expires_at
        FOR UPDATE SKIP LOCKED
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await?;

    let mut recovered = Vec::with_capacity(rows.len());
    for row in rows {
        recovered.push(row.id);
        if row.attempt < row.max_attempts {
            let retry_at = Utc::now() + Duration::seconds(backoff_seconds(row.attempt));
            sqlx::query(
                r#"
                UPDATE jobs
                SET status = 'queued',
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    heartbeat_at = NULL,
                    not_before = $2,
                    last_error_code = 'lease_expired'
                WHERE id = $1
                "#,
            )
            .bind(row.id)
            .bind(retry_at)
            .execute(&mut *transaction)
            .await?;

            append_job_event(
                &mut transaction,
                row.id,
                "job.retry_scheduled",
                json!({
                    "attempt": row.attempt,
                    "retryAt": retry_at,
                    "errorCode": "lease_expired",
                }),
            )
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE jobs
                SET status = 'failed',
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    heartbeat_at = NULL,
                    last_error_code = 'lease_expired'
                WHERE id = $1
                "#,
            )
            .bind(row.id)
            .execute(&mut *transaction)
            .await?;

            append_job_event(
                &mut transaction,
                row.id,
                "job.failed",
                json!({
                    "attempt": row.attempt,
                    "errorCode": "lease_expired",
                    "willRetry": false,
                }),
            )
            .await?;
        }
    }

    transaction.commit().await?;
    if !recovered.is_empty() {
        tracing::info!(
            event = "jobs.lease_recovered",
            count = recovered.len(),
            "recovered expired job leases"
        );
    }
    Ok(recovered)
}

/// Persist a load schedule for an approved plan, selected scenario, and active
/// target authorization. `next_run_at` is always stored as UTC; the timezone is
/// used only to advance daily/weekly recurrences at the same local wall time.
pub async fn create_schedule(pool: &PgPool, input: NewSchedule) -> Result<ScheduleRow, QueueError> {
    let (timezone, _) = normalize_timezone(&input.timezone)?;
    let recurrence = parse_recurrence(&input.recurrence)?;
    if input.start_at <= Utc::now() - Duration::minutes(5)
        || input
            .expires_at
            .is_some_and(|expires_at| expires_at <= input.start_at)
    {
        return Err(QueueError::InvalidScheduleWindow);
    }

    let eligibility: Option<(i32, Uuid, Option<DateTime<Utc>>)> = sqlx::query_as(
        r#"
        SELECT p.version, s.profile_id, ta.expires_at
        FROM test_plans p
        JOIN load_scenarios s ON s.plan_id = p.id AND s.id = $4
        JOIN plan_approvals pa ON pa.plan_id = p.id AND pa.plan_version = p.version
        JOIN target_authorizations ta ON ta.id = $5 AND ta.project_id = p.project_id
        WHERE p.id = $3
          AND p.job_id = $2
          AND p.project_id = $1
          AND p.status = 'approved'
          AND s.selected
          AND ta.revoked_at IS NULL
          AND ta.valid_from <= now()
          AND (ta.expires_at IS NULL OR ta.expires_at > $6)
          AND (ta.expires_at IS NULL OR $7::timestamptz IS NULL OR ta.expires_at >= $7::timestamptz)
        "#,
    )
    .bind(input.project_id)
    .bind(input.job_id)
    .bind(input.plan_id)
    .bind(input.load_scenario_id)
    .bind(input.target_authorization_id)
    .bind(input.start_at)
    .bind(input.expires_at)
    .fetch_optional(pool)
    .await?;

    let Some((_plan_version, load_profile_id, _authorization_expiry)) = eligibility else {
        return Err(QueueError::IneligibleSchedule);
    };

    let next_run_at = Some(input.start_at);
    let enabled = true;
    let recurrence_name = recurrence_name(recurrence);
    let row = sqlx::query_as::<_, ScheduleRow>(
        r#"
        INSERT INTO schedules (
            project_id, job_id, plan_id, load_scenario_id, load_profile_id,
            target_authorization_id, timezone, recurrence, enabled,
            next_run_at, expires_at, created_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING id, project_id, job_id, plan_id, load_scenario_id,
                  load_profile_id, target_authorization_id, timezone, recurrence,
                  enabled, next_run_at, expires_at, cancelled_at
        "#,
    )
    .bind(input.project_id)
    .bind(input.job_id)
    .bind(input.plan_id)
    .bind(input.load_scenario_id)
    .bind(load_profile_id)
    .bind(input.target_authorization_id)
    .bind(&timezone)
    .bind(recurrence_name)
    .bind(enabled)
    .bind(next_run_at)
    .bind(input.expires_at)
    .bind(input.created_by)
    .fetch_one(pool)
    .await?;

    tracing::info!(
        event = "schedule.created",
        schedule_id = %row.id,
        project_id = %row.project_id,
        job_id = %row.job_id,
        recurrence = %row.recurrence,
        "load schedule created"
    );
    Ok(row)
}

/// Promote due schedules into queued load runs and advance each schedule to its
/// next UTC occurrence. Runs are idempotent per schedule occurrence.
pub async fn promote_due_schedules(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<PromotedRun>, QueueError> {
    validate_limit(limit)?;

    let mut transaction = pool.begin().await?;
    let schedules: Vec<ScheduleRow> = sqlx::query_as(
        r#"
        SELECT id, project_id, job_id, plan_id, load_scenario_id, load_profile_id,
               target_authorization_id, timezone, recurrence, enabled, next_run_at,
               expires_at, cancelled_at
        FROM schedules
        WHERE enabled
          AND cancelled_at IS NULL
          AND next_run_at IS NOT NULL
          AND next_run_at <= now()
          AND (expires_at IS NULL OR expires_at > now())
          AND target_authorization_id IS NOT NULL
        ORDER BY next_run_at, created_at
        FOR UPDATE SKIP LOCKED
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await?;

    let mut promoted = Vec::with_capacity(schedules.len());
    for schedule in schedules {
        let eligible: Option<(i32, Uuid)> = sqlx::query_as(
            r#"
            SELECT p.version, pa.id
            FROM test_plans p
            JOIN plan_approvals pa ON pa.plan_id = p.id AND pa.plan_version = p.version
            JOIN load_scenarios s ON s.plan_id = p.id AND s.id = $2 AND s.profile_id = $4
            JOIN target_authorizations ta
              ON ta.id = $3 AND ta.project_id = $5
            WHERE p.id = $1
              AND p.job_id = $6
              AND p.status = 'approved'
              AND s.selected
              AND ta.revoked_at IS NULL
              AND ta.valid_from <= now()
              AND (ta.expires_at IS NULL OR ta.expires_at > now())
            "#,
        )
        .bind(schedule.plan_id)
        .bind(schedule.load_scenario_id)
        .bind(schedule.target_authorization_id)
        .bind(schedule.load_profile_id)
        .bind(schedule.project_id)
        .bind(schedule.job_id)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some((plan_version, approval_id)) = eligible else {
            sqlx::query("UPDATE schedules SET enabled = false, next_run_at = NULL WHERE id = $1")
                .bind(schedule.id)
                .execute(&mut *transaction)
                .await?;
            tracing::info!(
                event = "schedule.disabled",
                schedule_id = %schedule.id,
                project_id = %schedule.project_id,
                reason = "ineligible",
                "disabled a due schedule that is no longer eligible"
            );
            continue;
        };

        let scheduled_for = schedule
            .next_run_at
            .ok_or(QueueError::InvalidScheduleWindow)?;
        let idempotency_key = format!("schedule:{}:{}", schedule.id, scheduled_for.timestamp());
        let run_id: Option<Uuid> = sqlx::query_scalar(
            r#"
            INSERT INTO test_runs (
                project_id, job_id, plan_id, plan_version, plan_approval_id,
                run_type, status, load_scenario_id, load_profile_id,
                target_authorization_id, schedule_id, scheduled_for, idempotency_key
            ) VALUES ($1, $2, $3, $4, $5, 'load_testing', 'queued',
                      $6, $7, $8, $9, $10, $11)
            ON CONFLICT (project_id, idempotency_key) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(schedule.project_id)
        .bind(schedule.job_id)
        .bind(schedule.plan_id)
        .bind(plan_version)
        .bind(approval_id)
        .bind(schedule.load_scenario_id)
        .bind(schedule.load_profile_id)
        .bind(schedule.target_authorization_id)
        .bind(schedule.id)
        .bind(scheduled_for)
        .bind(&idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some(run_id) = run_id {
            append_job_event(
                &mut transaction,
                schedule.job_id,
                "run.scheduled",
                json!({
                    "runId": run_id,
                    "runType": "load_testing",
                    "scheduleId": schedule.id,
                    "scheduledFor": scheduled_for,
                }),
            )
            .await?;
            promoted.push(PromotedRun {
                id: run_id,
                job_id: schedule.job_id,
            });
        }

        let next_run_at = advance_schedule(&schedule, scheduled_for);
        sqlx::query(
            "UPDATE schedules
             SET next_run_at = $2, enabled = $3, last_run_at = now()
             WHERE id = $1",
        )
        .bind(schedule.id)
        .bind(next_run_at)
        .bind(next_run_at.is_some())
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;
    if !promoted.is_empty() {
        tracing::info!(
            event = "schedules.promoted",
            count = promoted.len(),
            "promoted due schedules into queued load runs"
        );
    }
    Ok(promoted)
}

/// Disable schedules whose expiry, authorization, or approved plan is no longer
/// usable. Returns the disabled schedule IDs.
pub async fn recover_expired_schedules(pool: &PgPool, limit: i64) -> Result<Vec<Uuid>, QueueError> {
    validate_limit(limit)?;

    let rows: Vec<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE schedules AS s
        SET enabled = false, next_run_at = NULL
        WHERE s.id IN (
            SELECT id
            FROM schedules
            WHERE enabled
              AND (
                  (expires_at IS NOT NULL AND expires_at <= now())
                  OR target_authorization_id IS NULL
                  OR NOT EXISTS (
                      SELECT 1 FROM target_authorizations ta
                      WHERE ta.id = schedules.target_authorization_id
                        AND ta.revoked_at IS NULL
                        AND ta.valid_from <= now()
                        AND (ta.expires_at IS NULL OR ta.expires_at > now())
                  )
                  OR NOT EXISTS (
                      SELECT 1 FROM test_plans p
                      WHERE p.id = schedules.plan_id AND p.status = 'approved'
                  )
              )
            ORDER BY next_run_at
            FOR UPDATE SKIP LOCKED
            LIMIT $1
        )
        RETURNING s.id
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    if !rows.is_empty() {
        tracing::info!(
            event = "schedules.expired_disabled",
            count = rows.len(),
            "disabled schedules whose expiry, authorization, or plan is no longer usable"
        );
    }
    Ok(rows)
}

/// Cancel a schedule and all of its not-yet-running runs. A running run is left
/// to its lease owner so the execution worker can stop it with its own budget.
pub async fn cancel_schedule(pool: &PgPool, schedule_id: Uuid) -> Result<bool, QueueError> {
    let mut transaction = pool.begin().await?;
    let cancelled: Option<Uuid> = sqlx::query_scalar(
        "UPDATE schedules
         SET enabled = false, next_run_at = NULL, cancelled_at = now()
         WHERE id = $1 AND cancelled_at IS NULL
         RETURNING id",
    )
    .bind(schedule_id)
    .fetch_optional(&mut *transaction)
    .await?;
    if cancelled.is_none() {
        transaction.rollback().await?;
        tracing::debug!(
            event = "schedule.cancel_noop",
            schedule_id = %schedule_id,
            "schedule was already cancelled or missing"
        );
        return Ok(false);
    }

    let runs: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE test_runs
         SET status = 'cancelled', finished_at = now(), last_error_code = 'schedule_cancelled'
         WHERE schedule_id = $1 AND status IN ('scheduled', 'queued')
         RETURNING id, job_id",
    )
    .bind(schedule_id)
    .fetch_all(&mut *transaction)
    .await?;
    let cancelled_runs = runs.len();
    for (run_id, job_id) in runs {
        append_job_event(
            &mut transaction,
            job_id,
            "run.cancelled",
            json!({"runId": run_id, "scheduleId": schedule_id, "reason": "schedule_cancelled"}),
        )
        .await?;
    }

    transaction.commit().await?;
    tracing::info!(
        event = "schedule.cancelled",
        schedule_id = %schedule_id,
        cancelled_runs,
        "schedule cancelled; not-yet-running runs were cancelled"
    );
    Ok(true)
}

/// Promote due runs into queued runs. Kept for compatibility with the Phase 4
/// scheduler; Phase 12 uses `claim_due_run_fair` for quota-aware execution.
pub async fn promote_due_runs(pool: &PgPool, limit: i64) -> Result<Vec<PromotedRun>, QueueError> {
    validate_limit(limit)?;

    let rows: Vec<PromotedRun> = sqlx::query_as(
        r#"
        UPDATE test_runs AS r
        SET status = 'queued'
        FROM (
            SELECT id
            FROM test_runs
            WHERE status = 'scheduled'
              AND scheduled_for IS NOT NULL
              AND scheduled_for <= now()
            ORDER BY scheduled_for, created_at
            FOR UPDATE SKIP LOCKED
            LIMIT $1
        ) AS due
        WHERE r.id = due.id
        RETURNING r.id, r.job_id
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    if !rows.is_empty() {
        tracing::info!(
            event = "runs.promoted",
            count = rows.len(),
            "promoted due runs to queued"
        );
    }
    Ok(rows)
}

/// Atomically claim one due run for an execution worker. The run claim commits
/// before processing, and `run.started` is appended to the parent job stream.
pub async fn claim_due_run(
    pool: &PgPool,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<Option<ClaimedRun>, QueueError> {
    validate_worker_id(worker_id)?;
    validate_lease(lease_seconds)?;

    let mut transaction = pool.begin().await?;
    let claimed: Option<ClaimedRun> = sqlx::query_as(
        r#"
        UPDATE test_runs AS r
        SET status = 'running',
            worker_id = $1,
            attempt = r.attempt + 1,
            heartbeat_at = now(),
            lease_expires_at = now() + make_interval(secs => $2::double precision),
            started_at = COALESCE(r.started_at, now()),
            last_error_code = NULL
        FROM (
            SELECT id
            FROM test_runs
            WHERE status IN ('scheduled', 'queued')
              AND (scheduled_for IS NULL OR scheduled_for <= now())
              AND attempt < max_attempts
            ORDER BY scheduled_for NULLS FIRST, created_at
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        ) AS candidate
        WHERE r.id = candidate.id
        RETURNING r.id, r.project_id, r.job_id, r.plan_id, r.plan_version,
                  r.plan_approval_id, r.run_type, r.status, r.attempt,
                  r.max_attempts, r.lease_expires_at
        "#,
    )
    .bind(worker_id)
    .bind(lease_seconds as f64)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(claimed) = claimed else {
        transaction.rollback().await?;
        return Ok(None);
    };

    append_job_event(
        &mut transaction,
        claimed.job_id,
        "run.started",
        json!({
            "runId": claimed.id,
            "runType": claimed.run_type,
            "workerId": worker_id,
            "attempt": claimed.attempt,
        }),
    )
    .await?;
    transaction.commit().await?;
    tracing::debug!(
        event = "run.claimed",
        run_id = %claimed.id,
        job_id = %claimed.job_id,
        project_id = %claimed.project_id,
        run_type = %claimed.run_type,
        worker_id,
        attempt = claimed.attempt,
        "claimed due run"
    );
    Ok(Some(claimed))
}

fn normalize_timezone(value: &str) -> Result<(String, FixedOffset), QueueError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TIMEZONE_LEN {
        return Err(QueueError::InvalidTimezone);
    }
    if trimmed.eq_ignore_ascii_case("UTC") || trimmed.eq_ignore_ascii_case("Etc/UTC") {
        return Ok((
            "UTC".to_owned(),
            FixedOffset::east_opt(0).ok_or(QueueError::InvalidTimezone)?,
        ));
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() == 6
        && (bytes[0] == b'+' || bytes[0] == b'-')
        && bytes[3] == b':'
        && bytes[1..3].iter().all(u8::is_ascii_digit)
        && bytes[4..6].iter().all(u8::is_ascii_digit)
    {
        let hours = (bytes[1] - b'0') * 10 + (bytes[2] - b'0');
        let minutes = (bytes[4] - b'0') * 10 + (bytes[5] - b'0');
        if hours > 14 || minutes > 59 || (hours == 14 && minutes != 0) {
            return Err(QueueError::InvalidTimezone);
        }
        let seconds = ((hours as i32) * 3_600 + (minutes as i32) * 60)
            * if bytes[0] == b'-' { -1 } else { 1 };
        return Ok((
            trimmed.to_owned(),
            FixedOffset::east_opt(seconds).ok_or(QueueError::InvalidTimezone)?,
        ));
    }
    Err(QueueError::InvalidTimezone)
}

// ---------------------------------------------------------------------------
// Per-project quotas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
pub struct ProjectQuota {
    pub project_id: Uuid,
    pub max_active_runs: i32,
    pub max_requests_per_window: Option<i64>,
    pub window_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct NewProjectQuota {
    pub project_id: Uuid,
    pub max_active_runs: i32,
    pub max_requests_per_window: Option<i64>,
    pub window_seconds: i32,
}

/// Create or replace a project's quota. Values are validated against the same
/// bounds the claim query relies on so a bad value can never widen a cap.
pub async fn set_project_quota(
    pool: &PgPool,
    input: NewProjectQuota,
) -> Result<ProjectQuota, QueueError> {
    validate_quota(i64::from(input.max_active_runs))?;
    validate_window(i64::from(input.window_seconds))?;
    if let Some(budget) = input.max_requests_per_window {
        validate_request_budget(budget)?;
    }

    let row = sqlx::query_as::<_, ProjectQuota>(
        r#"
        INSERT INTO project_quotas (
            project_id, max_active_runs, max_requests_per_window, window_seconds
        ) VALUES ($1, $2, $3, $4)
        ON CONFLICT (project_id) DO UPDATE SET
            max_active_runs = EXCLUDED.max_active_runs,
            max_requests_per_window = EXCLUDED.max_requests_per_window,
            window_seconds = EXCLUDED.window_seconds,
            updated_at = now()
        RETURNING project_id, max_active_runs, max_requests_per_window, window_seconds
        "#,
    )
    .bind(input.project_id)
    .bind(input.max_active_runs)
    .bind(input.max_requests_per_window)
    .bind(input.window_seconds)
    .fetch_one(pool)
    .await?;
    tracing::info!(
        event = "project.quota_updated",
        project_id = %row.project_id,
        max_active_runs = row.max_active_runs,
        window_seconds = row.window_seconds,
        "project quota updated"
    );
    Ok(row)
}

/// Load a project's quota override, if one exists.
pub async fn project_quota(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Option<ProjectQuota>, QueueError> {
    let row = sqlx::query_as::<_, ProjectQuota>(
        r#"
        SELECT project_id, max_active_runs, max_requests_per_window, window_seconds
        FROM project_quotas
        WHERE project_id = $1
        "#,
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Operational metrics
// ---------------------------------------------------------------------------

const FAILURE_WINDOW_SECONDS: i64 = 3_600;
const DURATION_WINDOW_SECONDS: i64 = 86_400;

/// A snapshot of queue health for operator diagnostics. Counts are aggregate
/// and carry no tenant content, so they are safe to expose on an internal
/// metrics endpoint.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct QueueMetrics {
    pub jobs_queued: i64,
    pub jobs_active: i64,
    pub jobs_failed: i64,
    pub jobs_lease_expired: i64,
    pub oldest_job_queue_age_seconds: Option<f64>,
    pub runs_queued: i64,
    pub runs_active: i64,
    pub runs_failed: i64,
    pub runs_lease_expired: i64,
    pub oldest_run_queue_age_seconds: Option<f64>,
    pub failed_window_seconds: i64,
    pub jobs_failed_recent: i64,
    pub runs_failed_recent: i64,
    pub completed_run_avg_duration_seconds: Option<f64>,
}

/// Compute the current queue and worker-health metrics in one round trip.
pub async fn queue_metrics(pool: &PgPool) -> Result<QueueMetrics, QueueError> {
    let row: QueueMetrics = sqlx::query_as(
        r#"
        SELECT
            (SELECT count(*) FROM jobs WHERE status = 'queued') AS jobs_queued,
            (SELECT count(*) FROM jobs WHERE status IN ('analyzing', 'planning')) AS jobs_active,
            (SELECT count(*) FROM jobs WHERE status = 'failed') AS jobs_failed,
            (SELECT count(*) FROM jobs
             WHERE status IN ('analyzing', 'planning')
               AND lease_expires_at IS NOT NULL
               AND lease_expires_at <= now()) AS jobs_lease_expired,
            (SELECT EXTRACT(EPOCH FROM (now() - min(created_at)))::double precision
             FROM jobs WHERE status = 'queued') AS oldest_job_queue_age_seconds,
            (SELECT count(*) FROM test_runs WHERE status IN ('scheduled', 'queued')) AS runs_queued,
            (SELECT count(*) FROM test_runs WHERE status = 'running') AS runs_active,
            (SELECT count(*) FROM test_runs WHERE status = 'failed') AS runs_failed,
            (SELECT count(*) FROM test_runs
             WHERE status = 'running'
               AND lease_expires_at IS NOT NULL
               AND lease_expires_at <= now()) AS runs_lease_expired,
            (SELECT EXTRACT(EPOCH FROM (now() - min(COALESCE(scheduled_for, created_at))))::double precision
             FROM test_runs WHERE status IN ('scheduled', 'queued')) AS oldest_run_queue_age_seconds,
            $1::bigint AS failed_window_seconds,
            (SELECT count(*) FROM jobs
             WHERE status = 'failed'
               AND updated_at >= now() - make_interval(secs => $2::double precision)) AS jobs_failed_recent,
            (SELECT count(*) FROM test_runs
             WHERE status = 'failed'
               AND finished_at IS NOT NULL
               AND finished_at >= now() - make_interval(secs => $2::double precision)) AS runs_failed_recent,
            (SELECT AVG(EXTRACT(EPOCH FROM (finished_at - started_at)))::double precision
             FROM test_runs
             WHERE status = 'completed'
               AND started_at IS NOT NULL
               AND finished_at IS NOT NULL
               AND finished_at >= now() - make_interval(secs => $3::double precision)) AS completed_run_avg_duration_seconds
        "#,
    )
    .bind(FAILURE_WINDOW_SECONDS)
    .bind(FAILURE_WINDOW_SECONDS as f64)
    .bind(DURATION_WINDOW_SECONDS as f64)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub evidence_batch: i64,
    pub reports_batch: i64,
    pub events_batch: i64,
    pub counters_batch: i64,
    pub report_retention_days: i32,
    pub event_retention_days: i32,
    pub counter_retention_days: i32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            evidence_batch: 500,
            reports_batch: 200,
            events_batch: 2_000,
            counters_batch: 2_000,
            report_retention_days: 180,
            event_retention_days: 30,
            counter_retention_days: 7,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RetentionOutcome {
    pub evidence_deleted: u64,
    pub reports_deleted: u64,
    pub job_events_deleted: u64,
    pub sessions_deleted: u64,
    pub rate_limit_entries_deleted: u64,
    pub login_attempt_entries_deleted: u64,
}

const MAX_RETENTION_DAYS: i32 = 3_650;

/// Purge data past its retention window in bounded batches.
///
/// This is the single sanctioned path that may delete otherwise-immutable rows.
/// It opts in with a transaction-local setting; every other path still hits the
/// `reject_immutable_row` trigger. Recommendation-referenced evidence is
/// retained so traceability constraints cannot be violated.
pub async fn run_retention(
    pool: &PgPool,
    policy: &RetentionPolicy,
) -> Result<RetentionOutcome, QueueError> {
    for batch in [
        policy.evidence_batch,
        policy.reports_batch,
        policy.events_batch,
        policy.counters_batch,
    ] {
        validate_retention_batch(batch)?;
    }
    for days in [
        policy.report_retention_days,
        policy.event_retention_days,
        policy.counter_retention_days,
    ] {
        if !(1..=MAX_RETENTION_DAYS).contains(&days) {
            return Err(QueueError::InvalidRetention);
        }
    }

    let mut transaction = pool.begin().await?;
    sqlx::query("SET LOCAL probeforge.retention_purge = 'on'")
        .execute(&mut *transaction)
        .await?;

    let evidence_deleted = sqlx::query(
        r#"
        DELETE FROM evidence
        WHERE id IN (
            SELECT e.id
            FROM evidence e
            WHERE e.retention_until IS NOT NULL
              AND e.retention_until <= now()
              AND NOT EXISTS (
                  SELECT 1 FROM recommendations r WHERE r.evidence_id = e.id
              )
            ORDER BY e.retention_until
            LIMIT $1
        )
        "#,
    )
    .bind(policy.evidence_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    let reports_deleted = sqlx::query(
        r#"
        DELETE FROM reports
        WHERE id IN (
            SELECT id
            FROM reports
            WHERE created_at < now() - make_interval(days => $1::int)
            ORDER BY created_at
            LIMIT $2
        )
        "#,
    )
    .bind(policy.report_retention_days)
    .bind(policy.reports_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    let job_events_deleted = sqlx::query(
        r#"
        DELETE FROM job_events
        WHERE id IN (
            SELECT je.id
            FROM job_events je
            JOIN jobs j ON j.id = je.job_id
            WHERE j.status IN ('failed', 'cancelled', 'expired')
              AND je.occurred_at < now() - make_interval(days => $1::int)
            ORDER BY je.occurred_at
            LIMIT $2
        )
        "#,
    )
    .bind(policy.event_retention_days)
    .bind(policy.events_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    let sessions_deleted = sqlx::query(
        r#"
        DELETE FROM auth_sessions
        WHERE id IN (
            SELECT id
            FROM auth_sessions
            WHERE expires_at < now() - make_interval(days => $1::int)
            ORDER BY expires_at
            LIMIT $2
        )
        "#,
    )
    .bind(policy.counter_retention_days)
    .bind(policy.counters_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    let rate_limit_entries_deleted = sqlx::query(
        r#"
        DELETE FROM api_rate_limits
        WHERE key_hash IN (
            SELECT key_hash
            FROM api_rate_limits
            WHERE last_at < now() - make_interval(days => $1::int)
            LIMIT $2
        )
        "#,
    )
    .bind(policy.counter_retention_days)
    .bind(policy.counters_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    let login_attempt_entries_deleted = sqlx::query(
        r#"
        DELETE FROM auth_login_attempts
        WHERE key_hash IN (
            SELECT key_hash
            FROM auth_login_attempts
            WHERE last_attempt_at < now() - make_interval(days => $1::int)
            LIMIT $2
        )
        "#,
    )
    .bind(policy.counter_retention_days)
    .bind(policy.counters_batch)
    .execute(&mut *transaction)
    .await?
    .rows_affected();

    transaction.commit().await?;

    let outcome = RetentionOutcome {
        evidence_deleted,
        reports_deleted,
        job_events_deleted,
        sessions_deleted,
        rate_limit_entries_deleted,
        login_attempt_entries_deleted,
    };
    tracing::info!(
        event = "retention.purge",
        evidence_deleted = outcome.evidence_deleted,
        reports_deleted = outcome.reports_deleted,
        job_events_deleted = outcome.job_events_deleted,
        sessions_deleted = outcome.sessions_deleted,
        rate_limit_entries_deleted = outcome.rate_limit_entries_deleted,
        login_attempt_entries_deleted = outcome.login_attempt_entries_deleted,
        "retention purge completed"
    );
    Ok(outcome)
}

fn validate_window(seconds: i64) -> Result<(), QueueError> {
    if !(60..=604_800).contains(&seconds) {
        return Err(QueueError::InvalidQuota);
    }
    Ok(())
}

fn validate_retention_batch(batch: i64) -> Result<(), QueueError> {
    if !(1..=100_000).contains(&batch) {
        return Err(QueueError::InvalidRetention);
    }
    Ok(())
}

fn validate_request_budget(budget: i64) -> Result<(), QueueError> {
    if !(0..=1_000_000_000).contains(&budget) {
        return Err(QueueError::InvalidQuota);
    }
    Ok(())
}

fn parse_recurrence(value: &str) -> Result<Recurrence, QueueError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "once" => Ok(Recurrence::Once),
        "hourly" => Ok(Recurrence::Hourly),
        "daily" => Ok(Recurrence::Daily),
        "weekly" => Ok(Recurrence::Weekly),
        _ => Err(QueueError::InvalidRecurrence),
    }
}

fn recurrence_name(recurrence: Recurrence) -> &'static str {
    match recurrence {
        Recurrence::Once => "once",
        Recurrence::Hourly => "hourly",
        Recurrence::Daily => "daily",
        Recurrence::Weekly => "weekly",
    }
}

fn advance_schedule(schedule: &ScheduleRow, scheduled_for: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let recurrence = parse_recurrence(&schedule.recurrence).ok()?;
    let (_, offset) = normalize_timezone(&schedule.timezone).ok()?;
    let next = match recurrence {
        Recurrence::Once => None,
        Recurrence::Hourly => Some(scheduled_for + Duration::hours(1)),
        Recurrence::Daily => Some(advance_local(scheduled_for, offset, 1)),
        Recurrence::Weekly => Some(advance_local(scheduled_for, offset, 7)),
    }?;
    if schedule
        .expires_at
        .is_some_and(|expires_at| next >= expires_at)
    {
        None
    } else {
        Some(next)
    }
}

fn advance_local(scheduled_for: DateTime<Utc>, offset: FixedOffset, days: i64) -> DateTime<Utc> {
    let local = offset.from_utc_datetime(&scheduled_for.naive_utc());
    (local + Duration::days(days)).with_timezone(&Utc)
}

/// Claim one due run while enforcing global and per-project active-run quotas.
/// A transaction advisory lock serializes quota checks so concurrent workers
/// cannot both observe the same free slot. `max_active_per_project` is the
/// default; a `project_quotas` row overrides both the active-run cap and adds an
/// optional per-window request budget.
pub async fn claim_due_run_fair(
    pool: &PgPool,
    worker_id: &str,
    lease_seconds: i64,
    max_active_global: i64,
    max_active_per_project: i64,
) -> Result<Option<ClaimedRun>, QueueError> {
    validate_worker_id(worker_id)?;
    validate_lease(lease_seconds)?;
    validate_quota(max_active_global)?;
    validate_quota(max_active_per_project)?;

    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('probeforge-run-claim'))")
        .execute(&mut *transaction)
        .await?;

    let claimed: Option<ClaimedRun> = sqlx::query_as(
        r#"
        UPDATE test_runs AS r
        SET status = 'running',
            worker_id = $1,
            attempt = r.attempt + 1,
            heartbeat_at = now(),
            lease_expires_at = now() + make_interval(secs => $2::double precision),
            started_at = COALESCE(r.started_at, now()),
            last_error_code = NULL
        FROM (
            SELECT id
            FROM test_runs AS candidate
            WHERE candidate.status IN ('scheduled', 'queued')
              AND (candidate.scheduled_for IS NULL OR candidate.scheduled_for <= now())
              AND candidate.attempt < candidate.max_attempts
              AND EXISTS (
                  SELECT 1 FROM target_authorizations ta
                  WHERE ta.id = candidate.target_authorization_id
                    AND ta.revoked_at IS NULL
                    AND ta.valid_from <= now()
                    AND (ta.expires_at IS NULL OR ta.expires_at > now())
              )
              AND (
                  SELECT count(*) FROM test_runs active
                  WHERE active.status = 'running'
              ) < $3
              AND (
                  SELECT count(*) FROM test_runs active
                  WHERE active.status = 'running'
                    AND active.project_id = candidate.project_id
              ) < COALESCE(
                  (SELECT q.max_active_runs FROM project_quotas q
                   WHERE q.project_id = candidate.project_id),
                  $4
              )
              -- A per-project request budget, when configured, reserves the
              -- candidate profile's maximum requests against the requests
              -- already recorded in the current window. When no quota row or
              -- no request budget exists the check is skipped (unlimited).
              AND (
                  (SELECT q.max_requests_per_window FROM project_quotas q
                   WHERE q.project_id = candidate.project_id) IS NULL
                  OR (
                      COALESCE((
                          SELECT SUM(rm.request_count)
                          FROM run_metrics rm
                          JOIN test_runs consumed ON consumed.id = rm.run_id
                          WHERE consumed.project_id = candidate.project_id
                            AND consumed.created_at >= now() - make_interval(
                                secs => COALESCE(
                                    (SELECT q2.window_seconds FROM project_quotas q2
                                     WHERE q2.project_id = candidate.project_id),
                                    86400
                                )::double precision
                            )
                      ), 0)
                      + COALESCE(
                          (SELECT lp.max_requests FROM load_profiles lp
                           WHERE lp.id = candidate.load_profile_id),
                          0
                      )
                  ) <= (
                      SELECT q3.max_requests_per_window FROM project_quotas q3
                      WHERE q3.project_id = candidate.project_id
                  )
              )
            ORDER BY (
                SELECT count(*) FROM test_runs active
                WHERE active.status = 'running'
                  AND active.project_id = candidate.project_id
            ) ASC,
            candidate.scheduled_for NULLS FIRST,
            candidate.created_at
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        ) AS candidate
        WHERE r.id = candidate.id
        RETURNING r.id, r.project_id, r.job_id, r.plan_id, r.plan_version,
                  r.plan_approval_id, r.run_type, r.status, r.attempt,
                  r.max_attempts, r.lease_expires_at
        "#,
    )
    .bind(worker_id)
    .bind(lease_seconds as f64)
    .bind(max_active_global)
    .bind(max_active_per_project)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(claimed) = claimed else {
        transaction.rollback().await?;
        return Ok(None);
    };

    append_job_event(
        &mut transaction,
        claimed.job_id,
        "run.started",
        json!({
            "runId": claimed.id,
            "runType": claimed.run_type,
            "workerId": worker_id,
            "attempt": claimed.attempt,
        }),
    )
    .await?;
    transaction.commit().await?;
    tracing::debug!(
        event = "run.claimed",
        run_id = %claimed.id,
        job_id = %claimed.job_id,
        project_id = %claimed.project_id,
        run_type = %claimed.run_type,
        worker_id,
        attempt = claimed.attempt,
        "claimed due run under quota"
    );
    Ok(Some(claimed))
}

pub async fn heartbeat_run(
    pool: &PgPool,
    run_id: Uuid,
    worker_id: &str,
    lease_seconds: i64,
) -> Result<Option<DateTime<Utc>>, QueueError> {
    validate_worker_id(worker_id)?;
    validate_lease(lease_seconds)?;

    let renewed: Option<DateTime<Utc>> = sqlx::query_scalar(
        r#"
        UPDATE test_runs
        SET heartbeat_at = now(),
            lease_expires_at = now() + make_interval(secs => $3::double precision)
        WHERE id = $1
          AND worker_id = $2
          AND status = 'running'
          AND lease_expires_at > now()
        RETURNING lease_expires_at
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(lease_seconds as f64)
    .fetch_optional(pool)
    .await?;

    if renewed.is_none() {
        tracing::warn!(
            event = "run.heartbeat_lost",
            run_id = %run_id,
            worker_id,
            "run lease heartbeat lost; worker no longer owns an unexpired lease"
        );
    }
    Ok(renewed)
}

pub async fn complete_run(
    pool: &PgPool,
    run_id: Uuid,
    worker_id: &str,
) -> Result<bool, QueueError> {
    validate_worker_id(worker_id)?;

    let mut transaction = pool.begin().await?;
    let completed: Option<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        UPDATE test_runs
        SET status = 'completed',
            finished_at = now(),
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = NULL
        WHERE id = $1
          AND worker_id = $2
          AND status = 'running'
          AND lease_expires_at > now()
        RETURNING id, job_id
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some((completed, job_id)) = completed else {
        transaction.rollback().await?;
        tracing::warn!(
            event = "run.lease_lost",
            run_id = %run_id,
            worker_id,
            "run completion fenced out by a newer lease"
        );
        return Ok(false);
    };

    append_job_event(
        &mut transaction,
        job_id,
        "run.completed",
        json!({"runId": completed}),
    )
    .await?;
    transaction.commit().await?;
    tracing::info!(
        event = "run.completed",
        run_id = %completed,
        job_id = %job_id,
        worker_id,
        "run completed"
    );
    Ok(true)
}

pub async fn fail_run(
    pool: &PgPool,
    run_id: Uuid,
    worker_id: &str,
    error_code: &str,
    retryable: bool,
) -> Result<FailOutcome, QueueError> {
    validate_worker_id(worker_id)?;
    validate_error_code(error_code)?;

    let mut transaction = pool.begin().await?;
    let row: Option<RunFailureRow> = sqlx::query_as(
        r#"
        SELECT job_id, attempt, max_attempts
        FROM test_runs
        WHERE id = $1
          AND worker_id = $2
          AND status = 'running'
          AND lease_expires_at > now()
        FOR UPDATE
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(row) = row else {
        transaction.rollback().await?;
        tracing::warn!(
            event = "run.lease_lost",
            run_id = %run_id,
            worker_id,
            "run failure fenced out by a newer lease"
        );
        return Ok(FailOutcome::LeaseLost);
    };

    if retryable && row.attempt < row.max_attempts {
        let retry_at = Utc::now() + Duration::seconds(backoff_seconds(row.attempt));
        sqlx::query(
            r#"
            UPDATE test_runs
            SET status = 'queued',
                worker_id = NULL,
                lease_expires_at = NULL,
                heartbeat_at = NULL,
                scheduled_for = $2,
                last_error_code = $3
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .bind(retry_at)
        .bind(error_code)
        .execute(&mut *transaction)
        .await?;

        append_job_event(
            &mut transaction,
            row.job_id,
            "run.failed",
            json!({
                "runId": run_id,
                "attempt": row.attempt,
                "errorCode": error_code,
                "willRetry": true,
                "retryAt": retry_at,
            }),
        )
        .await?;
        transaction.commit().await?;
        tracing::warn!(
            event = "run.retry_scheduled",
            run_id = %run_id,
            job_id = %row.job_id,
            worker_id,
            error_code,
            attempt = row.attempt,
            retry_at = %retry_at,
            "run failed and was rescheduled with backoff"
        );
        return Ok(FailOutcome::Retried {
            attempt: row.attempt,
            retry_at,
        });
    }

    sqlx::query(
        r#"
        UPDATE test_runs
        SET status = 'failed',
            finished_at = now(),
            worker_id = NULL,
            lease_expires_at = NULL,
            heartbeat_at = NULL,
            last_error_code = $2
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(error_code)
    .execute(&mut *transaction)
    .await?;

    append_job_event(
        &mut transaction,
        row.job_id,
        "run.failed",
        json!({
            "runId": run_id,
            "attempt": row.attempt,
            "errorCode": error_code,
            "willRetry": false,
        }),
    )
    .await?;
    transaction.commit().await?;
    tracing::warn!(
        event = "run.failed",
        run_id = %run_id,
        job_id = %row.job_id,
        worker_id,
        error_code,
        attempt = row.attempt,
        "run failed terminally"
    );
    Ok(FailOutcome::Failed {
        attempt: row.attempt,
    })
}

pub async fn recover_expired_run_leases(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Uuid>, QueueError> {
    validate_limit(limit)?;

    let mut transaction = pool.begin().await?;
    let rows: Vec<ExpiredRunRow> = sqlx::query_as(
        r#"
        SELECT id, job_id, attempt, max_attempts
        FROM test_runs
        WHERE status = 'running'
          AND lease_expires_at IS NOT NULL
          AND lease_expires_at <= now()
        ORDER BY lease_expires_at
        FOR UPDATE SKIP LOCKED
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await?;

    let mut recovered = Vec::with_capacity(rows.len());
    for row in rows {
        recovered.push(row.id);
        if row.attempt < row.max_attempts {
            let retry_at = Utc::now() + Duration::seconds(backoff_seconds(row.attempt));
            sqlx::query(
                r#"
                UPDATE test_runs
                SET status = 'queued',
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    heartbeat_at = NULL,
                    scheduled_for = $2,
                    last_error_code = 'lease_expired'
                WHERE id = $1
                "#,
            )
            .bind(row.id)
            .bind(retry_at)
            .execute(&mut *transaction)
            .await?;

            append_job_event(
                &mut transaction,
                row.job_id,
                "run.failed",
                json!({
                    "runId": row.id,
                    "attempt": row.attempt,
                    "errorCode": "lease_expired",
                    "willRetry": true,
                    "retryAt": retry_at,
                }),
            )
            .await?;
        } else {
            sqlx::query(
                r#"
                UPDATE test_runs
                SET status = 'failed',
                    finished_at = now(),
                    worker_id = NULL,
                    lease_expires_at = NULL,
                    heartbeat_at = NULL,
                    last_error_code = 'lease_expired'
                WHERE id = $1
                "#,
            )
            .bind(row.id)
            .execute(&mut *transaction)
            .await?;

            append_job_event(
                &mut transaction,
                row.job_id,
                "run.failed",
                json!({
                    "runId": row.id,
                    "attempt": row.attempt,
                    "errorCode": "lease_expired",
                    "willRetry": false,
                }),
            )
            .await?;
        }
    }

    transaction.commit().await?;
    if !recovered.is_empty() {
        tracing::info!(
            event = "runs.lease_recovered",
            count = recovered.len(),
            "recovered expired run leases"
        );
    }
    Ok(recovered)
}

fn validate_worker_id(value: &str) -> Result<(), QueueError> {
    if value.is_empty()
        || value.len() > MAX_WORKER_ID_LEN
        || value.chars().any(|character| character.is_control())
    {
        return Err(QueueError::InvalidWorkerId);
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), QueueError> {
    if value.is_empty()
        || value.len() > MAX_ERROR_CODE_LEN
        || value.chars().any(|character| character.is_control())
    {
        return Err(QueueError::InvalidErrorCode);
    }
    Ok(())
}

fn validate_lease(seconds: i64) -> Result<(), QueueError> {
    if !(1..=MAX_LEASE_SECONDS).contains(&seconds) {
        return Err(QueueError::InvalidLease);
    }
    Ok(())
}

fn validate_limit(limit: i64) -> Result<(), QueueError> {
    if !(1..=500).contains(&limit) {
        return Err(QueueError::InvalidLimit);
    }
    Ok(())
}

fn validate_quota(quota: i64) -> Result<(), QueueError> {
    if !(1..=MAX_QUOTA).contains(&quota) {
        return Err(QueueError::InvalidQuota);
    }
    Ok(())
}

fn backoff_seconds(attempt: i32) -> i64 {
    let exponent = (attempt.clamp(1, 10) - 1) as u32;
    2_i64.pow(exponent).min(MAX_BACKOFF_SECONDS)
}

fn valid_event_type(value: &str) -> bool {
    if value.is_empty() || value.len() > 120 {
        return false;
    }
    let mut segments = value.split('.');
    if segments.clone().count() < 2 {
        return false;
    }
    segments.all(|segment| {
        !segment.is_empty()
            && segment.starts_with(|character: char| character.is_ascii_lowercase())
            && segment.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 1);
        assert_eq!(backoff_seconds(2), 2);
        assert_eq!(backoff_seconds(3), 4);
        assert_eq!(backoff_seconds(20), MAX_BACKOFF_SECONDS);
    }

    #[test]
    fn validates_event_names() {
        assert!(valid_event_type("job.claimed"));
        assert!(valid_event_type("job.retry_scheduled"));
        assert!(!valid_event_type("job"));
        assert!(!valid_event_type("Job.claimed"));
        assert!(!valid_event_type("job..claimed"));
        assert!(!valid_event_type("job.claimed\n"));
    }

    #[test]
    fn rejects_unbounded_lease_and_identifiers() {
        assert!(validate_lease(0).is_err());
        assert!(validate_lease(MAX_LEASE_SECONDS + 1).is_err());
        assert!(validate_worker_id("").is_err());
        assert!(validate_worker_id("worker\n").is_err());
        assert!(validate_error_code("").is_err());
        assert!(validate_limit(0).is_err());
        assert!(validate_limit(501).is_err());
    }
}
