//! PostgreSQL-backed job and run queue.
//!
//! Claims use short transactions with `FOR UPDATE SKIP LOCKED`, then commit
//! before a worker performs any Git, model, or network work. Long-running work
//! is protected by leases and fenced with the worker ID on every transition.
//! No database row lock is held while work is in progress.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

const MAX_LEASE_SECONDS: i64 = 3_600;
const MAX_WORKER_ID_LEN: usize = 200;
const MAX_ERROR_CODE_LEN: usize = 120;
const MAX_BACKOFF_SECONDS: i64 = 300;

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
    Ok(recovered)
}

/// Promote scheduled runs whose time is due into the execution queue. This is
/// the scheduler's safe operation before an execution dispatcher exists.
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
    Ok(Some(claimed))
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
