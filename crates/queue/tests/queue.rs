//! Database-backed queue tests. Each test creates an isolated schema and uses
//! inert job/run rows; no repository or target is contacted.

use std::path::Path;

use chrono::{DateTime, Utc};
use probe_forge_queue::{
    FailOutcome, claim_due_run, claim_job, complete_job, fail_job, heartbeat_job, promote_due_runs,
    recover_expired_job_leases,
};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use uuid::Uuid;

struct TestQueue {
    admin: PgPool,
    pool: PgPool,
    schema: String,
    user_id: Uuid,
    project_id: Uuid,
    git_source_id: Uuid,
}

fn database_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            assert_ne!(
                std::env::var("PROBE_FORGE_REQUIRE_DB_TESTS").as_deref(),
                Ok("1"),
                "DATABASE_URL is required when PROBE_FORGE_REQUIRE_DB_TESTS=1"
            );
            eprintln!("skipping queue test: DATABASE_URL is not set");
            None
        }
    }
}

impl TestQueue {
    async fn setup() -> Option<Self> {
        let database_url = database_url()?;
        let schema = format!(
            "pf_queue_{}_{}",
            std::process::id(),
            Uuid::new_v4().simple()
        );

        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("database connection failed");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("test schema creation failed");

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .after_connect({
                let schema = schema.clone();
                move |connection, _metadata| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                            .execute(connection)
                            .await?;
                        Ok(())
                    })
                }
            })
            .connect(&database_url)
            .await
            .expect("schema-scoped connection failed");

        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let migrator = Migrator::new(migrations_dir)
            .await
            .expect("migrations directory could not be loaded");
        migrator.run(&pool).await.expect("migrations failed");

        let user_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let git_source_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, email, display_name)
             VALUES ($1, 'queue@example.test', 'Queue Owner')",
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("user insert");
        sqlx::query(
            "INSERT INTO projects (id, name, slug, owner_id)
             VALUES ($1, 'Queue Fixture', 'queue-fixture', $2)",
        )
        .bind(project_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("project insert");
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, role)
             VALUES ($1, $2, 'owner')",
        )
        .bind(project_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("member insert");
        sqlx::query(
            "INSERT INTO git_sources (id, project_id, provider, normalized_url, default_reference)
             VALUES ($1, $2, 'generic', 'https://git.example.test/inert/fixture.git', 'main')",
        )
        .bind(git_source_id)
        .bind(project_id)
        .execute(&pool)
        .await
        .expect("git source insert");

        Some(Self {
            admin,
            pool,
            schema,
            user_id,
            project_id,
            git_source_id,
        })
    }

    async fn seed_job(&self, label: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO jobs (
                project_id, created_by, goal, intensity, status, display_name,
                git_source_id, source_reference
             )
             VALUES ($1, $2, 'vulnerability_testing', 'standard', 'queued', $3, $4, 'main')
             RETURNING id",
        )
        .bind(self.project_id)
        .bind(self.user_id)
        .bind(format!("Queue fixture {label}"))
        .bind(self.git_source_id)
        .fetch_one(&self.pool)
        .await
        .expect("job insert")
    }

    async fn seed_due_run(&self, job_id: Uuid) -> Uuid {
        let test_id = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let approval_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let authorization_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO test_plans (
                id, project_id, job_id, version, status, summary, confidence, plan_payload
             )
             VALUES ($1, $2, $3, 1, 'approved', 'Inert queue fixture plan', 0.50, '{}')",
        )
        .bind(plan_id)
        .bind(self.project_id)
        .bind(job_id)
        .execute(&self.pool)
        .await
        .expect("plan insert");
        sqlx::query(
            "INSERT INTO planned_tests (
                id, project_id, plan_id, catalog_test_id, name, rationale, risk,
                expected_evidence, selected
             )
             VALUES ($1, $2, $3, 'http.cors.policy', 'CORS policy', 'Inert fixture',
                     'low', 'Headers', true)",
        )
        .bind(test_id)
        .bind(self.project_id)
        .bind(plan_id)
        .execute(&self.pool)
        .await
        .expect("planned test insert");
        sqlx::query(
            "INSERT INTO plan_approvals (
                id, project_id, plan_id, plan_version, approved_by,
                approved_test_ids, approval_snapshot
             )
             VALUES ($1, $2, $3, 1, $4, ARRAY[$5]::uuid[], '{}')",
        )
        .bind(approval_id)
        .bind(self.project_id)
        .bind(plan_id)
        .bind(self.user_id)
        .bind(test_id)
        .execute(&self.pool)
        .await
        .expect("approval insert");
        sqlx::query(
            "INSERT INTO targets (
                id, project_id, normalized_origin, environment, allowed_hosts,
                allowed_paths, verification_status, created_by
             )
             VALUES ($1, $2, 'https://inert.example.test', 'staging',
                     '[\"inert.example.test\"]'::jsonb, '[\"/api/\"]'::jsonb,
                     'verified', $3)",
        )
        .bind(target_id)
        .bind(self.project_id)
        .bind(self.user_id)
        .execute(&self.pool)
        .await
        .expect("target insert");
        sqlx::query(
            "INSERT INTO target_authorizations (
                id, project_id, target_id, authorized_by, authorization_method,
                scope, limits, expires_at
             )
             VALUES ($1, $2, $3, $4, 'explicit_confirmation',
                     '{\"origin\":\"https://inert.example.test\",\"allowedHosts\":[\"inert.example.test\"],\"allowedPaths\":[\"/api/\"],\"allowedPorts\":[443]}'::jsonb,
                     '{\"maxRequestsPerSecond\":1,\"maxConcurrency\":1,\"maxDurationSeconds\":60,\"maxRequests\":10}'::jsonb,
                     now() + interval '1 day')",
        )
        .bind(authorization_id)
        .bind(self.project_id)
        .bind(target_id)
        .bind(self.user_id)
        .execute(&self.pool)
        .await
        .expect("authorization insert");
        sqlx::query_scalar(
            "INSERT INTO test_runs (
                project_id, job_id, plan_id, plan_version, plan_approval_id,
                run_type, status, planned_test_id, target_authorization_id,
                idempotency_key, scheduled_for
             )
             VALUES ($1, $2, $3, 1, $4, 'vulnerability_testing', 'scheduled',
                     $5, $6, $7, now() - interval '1 minute')
             RETURNING id",
        )
        .bind(self.project_id)
        .bind(job_id)
        .bind(plan_id)
        .bind(approval_id)
        .bind(test_id)
        .bind(authorization_id)
        .bind(format!("queue-run-{run_id}"))
        .fetch_one(&self.pool)
        .await
        .expect("run insert")
    }

    async fn cleanup(&self) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("schema cleanup failed");
    }
}

async fn job_status(pool: &PgPool, job_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
        .expect("job status")
}

async fn make_claim_eligible(pool: &PgPool, job_id: Uuid) {
    sqlx::query("UPDATE jobs SET not_before = now() WHERE id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .expect("make claim eligible");
}

async fn expire_lease(pool: &PgPool, job_id: Uuid, seconds_ago: i64) {
    sqlx::query(
        "UPDATE jobs
         SET lease_expires_at = now() - make_interval(secs => $2::double precision),
             heartbeat_at = now() - make_interval(secs => $2::double precision)
         WHERE id = $1",
    )
    .bind(job_id)
    .bind(seconds_ago as f64)
    .execute(pool)
    .await
    .expect("expire lease");
}

#[tokio::test]
async fn concurrent_workers_cannot_claim_the_same_job() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let first = app.seed_job("first").await;
    let second = app.seed_job("second").await;

    let (a, b) = tokio::join!(
        claim_job(&app.pool, "worker-a", 60),
        claim_job(&app.pool, "worker-b", 60),
    );
    let a = a.expect("worker a claim").expect("a job for worker a");
    let b = b.expect("worker b claim").expect("a job for worker b");
    assert_ne!(a.id, b.id, "two live workers claimed the same row");
    let mut claimed_ids = [a.id, b.id];
    claimed_ids.sort();
    let mut expected_ids = [first, second];
    expected_ids.sort();
    assert_eq!(claimed_ids, expected_ids);

    let third = claim_job(&app.pool, "worker-c", 60)
        .await
        .expect("third claim");
    assert!(third.is_none(), "more jobs were available than expected");

    let claimed_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_events WHERE event_type = 'job.claimed'")
            .fetch_one(&app.pool)
            .await
            .expect("event count");
    assert_eq!(claimed_events, 2);

    app.cleanup().await;
}

#[tokio::test]
async fn heartbeat_and_completion_are_fenced_by_the_lease_owner() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("fencing").await;
    let claimed = claim_job(&app.pool, "worker-a", 60)
        .await
        .expect("claim")
        .expect("job");
    assert_eq!(claimed.id, job_id);

    let renewed = heartbeat_job(&app.pool, job_id, "worker-a", 60)
        .await
        .expect("heartbeat");
    assert!(renewed.is_some());
    assert!(
        heartbeat_job(&app.pool, job_id, "worker-b", 60)
            .await
            .expect("wrong worker heartbeat")
            .is_none()
    );

    expire_lease(&app.pool, job_id, 5).await;
    assert!(
        heartbeat_job(&app.pool, job_id, "worker-a", 60)
            .await
            .expect("expired heartbeat")
            .is_none()
    );
    let recovered = recover_expired_job_leases(&app.pool, 10)
        .await
        .expect("recover");
    assert_eq!(recovered, vec![job_id]);
    make_claim_eligible(&app.pool, job_id).await;

    let claimed_by_b = claim_job(&app.pool, "worker-b", 60)
        .await
        .expect("claim b")
        .expect("job b");
    assert_eq!(claimed_by_b.attempt, 2);
    assert!(
        !complete_job(&app.pool, job_id, "worker-a", None)
            .await
            .expect("stale completion"),
        "a worker completed after losing its lease"
    );
    assert!(
        complete_job(&app.pool, job_id, "worker-b", None)
            .await
            .expect("current completion"),
        "the live lease owner could not complete"
    );
    assert_eq!(job_status(&app.pool, job_id).await, "awaiting_approval");

    let lease: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT lease_expires_at FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&app.pool)
            .await
            .expect("lease query");
    assert!(lease.is_none());

    app.cleanup().await;
}

#[tokio::test]
async fn retries_backoff_and_expiry_recovery_are_bounded() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("retry").await;
    claim_job(&app.pool, "worker-a", 60)
        .await
        .expect("claim")
        .expect("job");

    let outcome = fail_job(&app.pool, job_id, "worker-a", "analysis_transient", true)
        .await
        .expect("retryable failure");
    let FailOutcome::Retried { attempt, retry_at } = outcome else {
        panic!("expected retry, received {outcome:?}");
    };
    assert_eq!(attempt, 1);
    assert!(retry_at > Utc::now());
    assert_eq!(job_status(&app.pool, job_id).await, "queued");

    make_claim_eligible(&app.pool, job_id).await;
    let second = claim_job(&app.pool, "worker-b", 60)
        .await
        .expect("second claim")
        .expect("job");
    assert_eq!(second.attempt, 2);
    expire_lease(&app.pool, job_id, 5).await;
    recover_expired_job_leases(&app.pool, 10)
        .await
        .expect("recover second");

    make_claim_eligible(&app.pool, job_id).await;
    let third = claim_job(&app.pool, "worker-c", 60)
        .await
        .expect("third claim")
        .expect("job");
    assert_eq!(third.attempt, 3);
    expire_lease(&app.pool, job_id, 5).await;
    recover_expired_job_leases(&app.pool, 10)
        .await
        .expect("recover third");
    assert_eq!(job_status(&app.pool, job_id).await, "failed");
    assert!(
        claim_job(&app.pool, "worker-d", 60)
            .await
            .expect("terminal claim")
            .is_none()
    );

    let retry_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_events WHERE job_id = $1 AND event_type = 'job.retry_scheduled'",
    )
    .bind(job_id)
    .fetch_one(&app.pool)
    .await
    .expect("retry event count");
    assert_eq!(retry_events, 2);
    let failed_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_events WHERE job_id = $1 AND event_type = 'job.failed'",
    )
    .bind(job_id)
    .fetch_one(&app.pool)
    .await
    .expect("failed event count");
    assert_eq!(failed_events, 1);

    app.cleanup().await;
}

#[tokio::test]
async fn stale_worker_cannot_fail_or_complete_after_a_new_claim() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("stale").await;
    claim_job(&app.pool, "worker-a", 60)
        .await
        .expect("claim")
        .expect("job");
    expire_lease(&app.pool, job_id, 5).await;
    recover_expired_job_leases(&app.pool, 10)
        .await
        .expect("recover");
    make_claim_eligible(&app.pool, job_id).await;
    claim_job(&app.pool, "worker-b", 60)
        .await
        .expect("claim b")
        .expect("job");

    let outcome = fail_job(&app.pool, job_id, "worker-a", "stale", true)
        .await
        .expect("stale failure");
    assert_eq!(outcome, FailOutcome::LeaseLost);
    assert!(
        !complete_job(&app.pool, job_id, "worker-a", None)
            .await
            .expect("stale complete")
    );
    assert_eq!(
        fail_job(&app.pool, job_id, "worker-a", "stale", true)
            .await
            .expect("stale failure"),
        FailOutcome::LeaseLost
    );

    app.cleanup().await;
}

#[tokio::test]
async fn scheduler_promotes_and_claims_due_runs() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("run").await;
    let run_id = app.seed_due_run(job_id).await;

    let promoted = promote_due_runs(&app.pool, 10).await.expect("promote");
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0].id, run_id);
    assert_eq!(promoted[0].job_id, job_id);

    let claimed = claim_due_run(&app.pool, "execution-worker-a", 60)
        .await
        .expect("claim run")
        .expect("due run");
    assert_eq!(claimed.id, run_id);
    assert_eq!(claimed.status, "running");
    assert_eq!(claimed.attempt, 1);
    assert!(
        claim_due_run(&app.pool, "execution-worker-b", 60)
            .await
            .expect("second claim")
            .is_none()
    );

    let started_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_events WHERE job_id = $1 AND event_type = 'run.started'",
    )
    .bind(job_id)
    .fetch_one(&app.pool)
    .await
    .expect("run event count");
    assert_eq!(started_events, 1);

    app.cleanup().await;
}
