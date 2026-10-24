//! Database-backed queue tests. Each test creates an isolated schema and uses
//! inert job/run rows; no repository or target is contacted.

use std::path::Path;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use probe_forge_load_runner::{MetricBucket, RunMetrics, RunSummary, persist_metrics};
use probe_forge_queue::{
    FailOutcome, NewSchedule, cancel_schedule, claim_due_run, claim_due_run_fair, claim_job,
    complete_job, complete_run, create_schedule, fail_job, fail_run, heartbeat_job, heartbeat_run,
    promote_due_runs, promote_due_schedules, recover_expired_job_leases,
    recover_expired_run_leases, recover_expired_schedules,
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
    authorization_id: Uuid,
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

        let target_id = Uuid::new_v4();
        let authorization_id = Uuid::new_v4();
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
        .bind(project_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("target insert");
        sqlx::query(
            "INSERT INTO target_authorizations (
                id, project_id, target_id, authorized_by, authorization_method,
                scope, limits, expires_at
             )
             VALUES ($1, $2, $3, $4, 'explicit_confirmation',
                     '{\"origin\":\"https://inert.example.test\",\"allowedHosts\":[\"inert.example.test\"],\"allowedPaths\":[\"/api/\"],\"allowedPorts\":[443]}'::jsonb,
                     '{\"maxRequestsPerSecond\":10,\"maxConcurrency\":10,\"maxDurationSeconds\":3600,\"maxRequests\":10000}'::jsonb,
                     now() + interval '30 days')",
        )
        .bind(authorization_id)
        .bind(project_id)
        .bind(target_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("authorization insert");

        Some(Self {
            admin,
            pool,
            schema,
            user_id,
            project_id,
            git_source_id,
            authorization_id,
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

    async fn seed_load_schedule(
        &self,
        label: &str,
        start_at: DateTime<Utc>,
        recurrence: &str,
        timezone: &str,
    ) -> Uuid {
        let job_id: Uuid = sqlx::query_scalar(
            "INSERT INTO jobs (
                project_id, created_by, goal, intensity, status, display_name,
                git_source_id, source_reference
             )
             VALUES ($1, $2, 'load_testing', 'standard', 'awaiting_approval', $3, $4, 'main')
             RETURNING id",
        )
        .bind(self.project_id)
        .bind(self.user_id)
        .bind(format!("Load schedule fixture {label}"))
        .bind(self.git_source_id)
        .fetch_one(&self.pool)
        .await
        .expect("load job insert");

        let plan_id = Uuid::new_v4();
        let approval_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let scenario_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO test_plans (
                id, project_id, job_id, version, status, summary, confidence, plan_payload
             )
             VALUES ($1, $2, $3, 1, 'approved', 'Inert load fixture plan', 0.50, '{}')",
        )
        .bind(plan_id)
        .bind(self.project_id)
        .bind(job_id)
        .execute(&self.pool)
        .await
        .expect("load plan insert");
        sqlx::query(
            "INSERT INTO load_profiles (
                id, project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
                max_requests, max_requests_per_second, stop_conditions
             )
             VALUES ($1, $2, 'Inert profile', 1, 1, 5, 20, 2, '{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(self.project_id)
        .execute(&self.pool)
        .await
        .expect("load profile insert");
        sqlx::query(
            "INSERT INTO load_scenarios (
                id, project_id, plan_id, profile_id, name, schema_version, scenario, selected
             )
             VALUES ($1, $2, $3, $4, 'Inert scenario', '1', $5, true)",
        )
        .bind(scenario_id)
        .bind(self.project_id)
        .bind(plan_id)
        .bind(profile_id)
        .bind(serde_json::json!({
            "name": "Inert scenario",
            "description": "Inert queue fixture; no traffic.",
            "requests": [{
                "id": "baseline",
                "method": "GET",
                "path": "/api/items",
                "expectedStatuses": [200]
            }],
            "profile": {
                "virtualUsers": 1,
                "rampUpSeconds": 1,
                "durationSeconds": 5,
                "maxRequests": 20,
                "maxRequestsPerSecond": 2
            }
        }))
        .execute(&self.pool)
        .await
        .expect("load scenario insert");
        sqlx::query(
            "INSERT INTO plan_approvals (
                id, project_id, plan_id, plan_version, approved_by,
                approved_test_ids, approved_load_scenario_ids, approval_snapshot
             )
             VALUES ($1, $2, $3, 1, $4, '{}'::uuid[], ARRAY[$5]::uuid[], '{}')",
        )
        .bind(approval_id)
        .bind(self.project_id)
        .bind(plan_id)
        .bind(self.user_id)
        .bind(scenario_id)
        .execute(&self.pool)
        .await
        .expect("load approval insert");

        let schedule = create_schedule(
            &self.pool,
            NewSchedule {
                project_id: self.project_id,
                job_id,
                plan_id,
                load_scenario_id: scenario_id,
                target_authorization_id: self.authorization_id,
                timezone: timezone.to_owned(),
                recurrence: recurrence.to_owned(),
                start_at,
                expires_at: Some(Utc::now() + ChronoDuration::days(3)),
                created_by: self.user_id,
            },
        )
        .await
        .expect("schedule create");
        schedule.id
    }

    async fn seed_due_run(&self, job_id: Uuid) -> Uuid {
        let test_id = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let approval_id = Uuid::new_v4();
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
        .bind(self.authorization_id)
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

async fn expire_run_lease(pool: &PgPool, run_id: Uuid, seconds_ago: i64) {
    sqlx::query(
        "UPDATE test_runs
         SET lease_expires_at = now() - make_interval(secs => $2::double precision),
             heartbeat_at = now() - make_interval(secs => $2::double precision)
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(seconds_ago as f64)
    .execute(pool)
    .await
    .expect("expire run lease");
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

#[tokio::test]
async fn scheduler_persists_and_handles_due_future_cancelled_and_expired_schedules() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };

    let due_schedule = app
        .seed_load_schedule(
            "due",
            Utc::now() - ChronoDuration::minutes(1),
            "once",
            "UTC",
        )
        .await;
    let future_schedule = app
        .seed_load_schedule(
            "future",
            Utc::now() + ChronoDuration::hours(1),
            "daily",
            "+02:00",
        )
        .await;

    let promoted = promote_due_schedules(&app.pool, 10)
        .await
        .expect("promote schedules");
    assert_eq!(promoted.len(), 1);
    let due_run = promoted[0].id;
    let run_status: String = sqlx::query_scalar("SELECT status FROM test_runs WHERE id = $1")
        .bind(due_run)
        .fetch_one(&app.pool)
        .await
        .expect("due run status");
    assert_eq!(run_status, "queued");
    let schedule_state: (bool, Option<DateTime<Utc>>, String) =
        sqlx::query_as("SELECT enabled, next_run_at, timezone FROM schedules WHERE id = $1")
            .bind(due_schedule)
            .fetch_one(&app.pool)
            .await
            .expect("due schedule state");
    assert!(!schedule_state.0);
    assert!(schedule_state.1.is_none());

    let future_state: (bool, Option<DateTime<Utc>>, String) =
        sqlx::query_as("SELECT enabled, next_run_at, timezone FROM schedules WHERE id = $1")
            .bind(future_schedule)
            .fetch_one(&app.pool)
            .await
            .expect("future schedule state");
    assert!(future_state.0);
    assert!(future_state.1.is_some());
    assert_eq!(future_state.2, "+02:00");

    let cancelled_schedule = app
        .seed_load_schedule(
            "cancelled",
            Utc::now() - ChronoDuration::minutes(1),
            "once",
            "UTC",
        )
        .await;
    promote_due_schedules(&app.pool, 10)
        .await
        .expect("promote cancelled candidate");
    assert!(
        cancel_schedule(&app.pool, cancelled_schedule)
            .await
            .expect("cancel schedule")
    );
    let cancelled_run_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM test_runs WHERE schedule_id = $1 AND status = 'cancelled'",
    )
    .bind(cancelled_schedule)
    .fetch_one(&app.pool)
    .await
    .expect("cancelled run count");
    assert_eq!(cancelled_run_count, 1);
    let cancelled_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT cancelled_at FROM schedules WHERE id = $1")
            .bind(cancelled_schedule)
            .fetch_one(&app.pool)
            .await
            .expect("cancelled timestamp");
    assert!(cancelled_at.is_some());

    let expired_schedule = app
        .seed_load_schedule(
            "expired",
            Utc::now() - ChronoDuration::minutes(1),
            "once",
            "UTC",
        )
        .await;
    sqlx::query(
        "UPDATE schedules
         SET expires_at = now() - interval '1 second', next_run_at = now() - interval '1 minute'
         WHERE id = $1",
    )
    .bind(expired_schedule)
    .execute(&app.pool)
    .await
    .expect("expire schedule");
    let recovered = recover_expired_schedules(&app.pool, 10)
        .await
        .expect("recover expired schedules");
    assert!(recovered.contains(&expired_schedule));
    let expired_run_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM test_runs WHERE schedule_id = $1")
            .bind(expired_schedule)
            .fetch_one(&app.pool)
            .await
            .expect("expired run count");
    assert_eq!(expired_run_count, 0);

    app.cleanup().await;
}

#[tokio::test]
async fn fair_run_claims_enforce_global_and_project_quotas_and_leases() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("quota").await;
    let first_run = app.seed_due_run(job_id).await;
    let second_job_id = app.seed_job("quota-two").await;
    let second_run = app.seed_due_run(second_job_id).await;
    promote_due_runs(&app.pool, 10)
        .await
        .expect("promote quota runs");

    let first = claim_due_run_fair(&app.pool, "execution-a", 60, 2, 1)
        .await
        .expect("first fair claim")
        .expect("first run");
    assert!(first_run == first.id || second_run == first.id);
    assert!(
        claim_due_run_fair(&app.pool, "execution-b", 60, 2, 1)
            .await
            .expect("per-project quota claim")
            .is_none(),
        "a second run from the same project bypassed the per-project quota"
    );

    assert!(
        heartbeat_run(&app.pool, first.id, "execution-a", 60)
            .await
            .expect("heartbeat")
            .is_some()
    );
    assert!(
        heartbeat_run(&app.pool, first.id, "execution-b", 60)
            .await
            .expect("wrong worker heartbeat")
            .is_none()
    );
    assert!(
        !complete_run(&app.pool, first.id, "execution-b")
            .await
            .expect("stale completion")
    );
    assert!(
        complete_run(&app.pool, first.id, "execution-a")
            .await
            .expect("completion")
    );

    let second = claim_due_run_fair(&app.pool, "execution-b", 60, 2, 1)
        .await
        .expect("second claim")
        .expect("second run");
    assert_ne!(first.id, second.id);
    assert!(
        complete_run(&app.pool, second.id, "execution-b")
            .await
            .expect("second completion")
    );

    let third_job_id = app.seed_job("quota-three").await;
    let fourth_job_id = app.seed_job("quota-four").await;
    let third_run = app.seed_due_run(third_job_id).await;
    let fourth_run = app.seed_due_run(fourth_job_id).await;
    promote_due_runs(&app.pool, 10)
        .await
        .expect("promote global quota runs");
    let global_first = claim_due_run_fair(&app.pool, "execution-c", 60, 1, 2)
        .await
        .expect("global first")
        .expect("global first run");
    assert!(third_run == global_first.id || fourth_run == global_first.id);
    assert!(
        claim_due_run_fair(&app.pool, "execution-d", 60, 1, 2)
            .await
            .expect("global quota claim")
            .is_none(),
        "a second run bypassed the global concurrency quota"
    );

    expire_run_lease(&app.pool, global_first.id, 5).await;
    let recovered = recover_expired_run_leases(&app.pool, 10)
        .await
        .expect("recover run lease");
    assert_eq!(recovered, vec![global_first.id]);
    let recovered_status: String = sqlx::query_scalar("SELECT status FROM test_runs WHERE id = $1")
        .bind(global_first.id)
        .fetch_one(&app.pool)
        .await
        .expect("recovered status");
    assert_eq!(recovered_status, "queued");
    assert!(
        !complete_run(&app.pool, global_first.id, "execution-c")
            .await
            .expect("stale completed")
    );
    assert_eq!(
        fail_run(&app.pool, global_first.id, "execution-c", "stale", true)
            .await
            .expect("stale failure"),
        FailOutcome::LeaseLost
    );

    app.cleanup().await;
}

#[tokio::test]
async fn aggregate_run_metrics_are_persisted_and_upserted() {
    let Some(app) = TestQueue::setup().await else {
        return;
    };
    let job_id = app.seed_job("metrics").await;
    let run_id = app.seed_due_run(job_id).await;
    let started_at = Utc::now();
    let finished_at = started_at + ChronoDuration::seconds(10);
    let mut metrics = RunMetrics {
        started_at,
        finished_at,
        bucket_seconds: 10,
        buckets: vec![MetricBucket {
            bucket_start: started_at,
            bucket_seconds: 10,
            endpoint: "baseline".to_owned(),
            status_code: 200,
            request_count: 1,
            error_count: 0,
            latency_p50_ms: 10.0,
            latency_p95_ms: 10.0,
            latency_p99_ms: 10.0,
            throughput_rps: 0.1,
            response_bytes: 100,
        }],
        summary: RunSummary {
            request_count: 1,
            error_count: 0,
            timeout_count: 0,
            dropped_iterations: 0,
            latency_p50_ms: 10.0,
            latency_p95_ms: 10.0,
            latency_p99_ms: 10.0,
            throughput_rps: 0.1,
            response_bytes: 100,
        },
    };
    persist_metrics(&app.pool, app.project_id, run_id, &metrics)
        .await
        .expect("persist metrics");
    let (count, requests): (i64, i64) = sqlx::query_as(
        "SELECT count(*), COALESCE(sum(request_count), 0)::bigint FROM run_metrics WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&app.pool)
    .await
    .expect("metrics read");
    assert_eq!(count, 1);
    assert_eq!(requests, 1);

    metrics.buckets[0].request_count = 2;
    metrics.buckets[0].error_count = 1;
    persist_metrics(&app.pool, app.project_id, run_id, &metrics)
        .await
        .expect("upsert metrics");
    let (requests, errors): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(sum(request_count), 0)::bigint, COALESCE(sum(error_count), 0)::bigint
         FROM run_metrics WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&app.pool)
    .await
    .expect("metrics upsert read");
    assert_eq!(requests, 2);
    assert_eq!(errors, 1);

    app.cleanup().await;
}
