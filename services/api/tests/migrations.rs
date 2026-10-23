//! Migration integration test. Requires a local PostgreSQL database.
//!
//! The test creates a unique schema inside `DATABASE_URL`, applies every
//! migration there, verifies the expected tables/indexes, inserts a complete
//! fixture chain, and asserts that invalid or mutable records are rejected.
//! It never touches the `public` schema.
//!
//! `bun run check:db` sets `PROBE_FORGE_REQUIRE_DB_TESTS=1` so a missing
//! database is a failure, not a silent skip.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};

fn database_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => {
            assert_ne!(
                std::env::var("PROBE_FORGE_REQUIRE_DB_TESTS").as_deref(),
                Ok("1"),
                "DATABASE_URL is required when PROBE_FORGE_REQUIRE_DB_TESTS=1"
            );
            eprintln!("skipping database migration test: DATABASE_URL is not set");
            None
        }
    }
}

async fn assert_sql_fails(pool: &PgPool, sql: &'static str, expected: &str) {
    let error = sqlx::query(sql)
        .execute(pool)
        .await
        .expect_err("statement should have been rejected");
    let message = error.to_string();
    assert!(
        message.contains(expected),
        "expected error containing {expected:?}, received {message:?}"
    );
}

#[tokio::test]
async fn migrations_create_and_enforce_core_schema() {
    let Some(database_url) = database_url() else {
        return;
    };

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let schema = format!("pf_test_{}_{}", std::process::id(), suffix);

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("database connection failed");
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("test schema creation failed");

    let schema_pool = PgPoolOptions::new()
        .max_connections(3)
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
        .expect("schema-scoped database connection failed");

    let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let migrator = Migrator::new(migrations_dir)
        .await
        .expect("migrations directory could not be loaded");

    migrator.run(&schema_pool).await.expect("migrations failed");
    // Forward-only migrations must be safe to execute repeatedly.
    migrator
        .run(&schema_pool)
        .await
        .expect("migrations are not idempotent");

    let tables: Vec<String> =
        sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname = current_schema()")
            .fetch_all(&schema_pool)
            .await
            .expect("table inventory failed");
    for expected in [
        "users",
        "projects",
        "project_members",
        "git_sources",
        "repository_snapshots",
        "jobs",
        "job_events",
        "test_plans",
        "planned_tests",
        "plan_approvals",
        "targets",
        "target_authorizations",
        "load_profiles",
        "load_scenarios",
        "schedules",
        "test_runs",
        "test_results",
        "evidence",
        "run_metrics",
        "agent_sessions",
        "audit_log",
        "auth_sessions",
        "auth_login_attempts",
    ] {
        assert!(
            tables.iter().any(|table| table == expected),
            "missing table {expected}"
        );
    }

    let indexes: Vec<String> =
        sqlx::query_scalar("SELECT indexname FROM pg_indexes WHERE schemaname = current_schema()")
            .fetch_all(&schema_pool)
            .await
            .expect("index inventory failed");
    for expected in [
        "jobs_queue_idx",
        "jobs_lease_idx",
        "jobs_idempotency_key_key",
        "job_events_job_sequence_idx",
        "test_plans_job_version_idx",
        "plan_approvals_project_idx",
        "target_authorizations_active_idx",
        "schedules_due_idx",
        "test_runs_queue_idx",
        "test_runs_lease_idx",
        "test_results_run_idx",
        "run_metrics_run_time_idx",
        "auth_sessions_user_created_idx",
        "auth_login_attempts_last_attempt_idx",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing index {expected}"
        );
    }

    let approval_scenario_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'plan_approvals'
           AND column_name = 'approved_load_scenario_ids'",
    )
    .fetch_one(&schema_pool)
    .await
    .expect("approval column inventory failed");
    assert_eq!(
        approval_scenario_columns, 1,
        "migration 0006 did not add approved_load_scenario_ids"
    );

    let password_hash_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'users'
           AND column_name = 'password_hash'",
    )
    .fetch_one(&schema_pool)
    .await
    .expect("password column inventory failed");
    assert_eq!(
        password_hash_columns, 1,
        "migration 0007 did not add users.password_hash"
    );

    seed_complete_fixture(&schema_pool).await;

    // Unknown enum values are rejected before persistence.
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO jobs (id, project_id, created_by, goal, intensity, display_name, git_source_id, source_reference)
        VALUES ('12121212-1212-4212-8212-121212121212'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid,
                'exploit', 'standard', 'Invalid goal',
                '33333333-3333-4333-8333-333333333333'::uuid, 'main')
        "#,
        "jobs_goal_values",
    ).await;

    // Idempotency keys are unique per project.
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO jobs (id, project_id, created_by, goal, intensity, display_name, git_source_id, source_reference, idempotency_key)
        VALUES ('13131313-1313-4313-8313-131313131313'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid,
                'vulnerability_testing', 'standard', 'Duplicate idempotency',
                '33333333-3333-4333-8333-333333333333'::uuid, 'main', 'job-1')
        "#,
        "jobs_idempotency_key_key",
    ).await;

    // Event sequence numbers are unique per job and events are append-only.
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO job_events (id, job_id, sequence, event_type)
        VALUES ('14141414-1414-4414-8414-141414141414'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid, 1, 'job.queued')
        "#,
        "job_events_job_id_sequence_key",
    )
    .await;
    assert_sql_fails(
        &schema_pool,
        r#"
        UPDATE job_events SET payload = '{"changed":true}'::jsonb
        WHERE job_id = '44444444-4444-4444-8444-444444444444'::uuid AND sequence = 1
        "#,
        "immutable",
    )
    .await;

    // Approval snapshots cannot be rewritten after approval.
    assert_sql_fails(
        &schema_pool,
        r#"
        UPDATE plan_approvals SET approval_snapshot = '{"changed":true}'::jsonb
        WHERE id = '88888888-8888-4888-8888-888888888888'::uuid
        "#,
        "immutable",
    )
    .await;

    // Authorization scope is immutable and authorizations are retained.
    assert_sql_fails(
        &schema_pool,
        r#"
        UPDATE target_authorizations
        SET scope = '{"origin":"https://evil.example.test"}'::jsonb
        WHERE id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid
        "#,
        "immutable",
    )
    .await;
    assert_sql_fails(
        &schema_pool,
        "DELETE FROM target_authorizations WHERE id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid",
        "cannot be deleted",
    )
    .await;
    sqlx::query(
        "UPDATE target_authorizations SET revoked_at = now() WHERE id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid",
    )
    .execute(&schema_pool)
    .await
    .expect("revocation should be allowed");

    // Failed results require a severity, and result keys are idempotent.
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO test_results (id, project_id, run_id, catalog_test_id, result_key, outcome, summary)
        VALUES ('15151515-1515-4515-8515-151515151515'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'::uuid,
                'http.cors.policy', 'missing-severity', 'failed', 'No severity')
        "#,
        "test_results_failed_severity",
    ).await;
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO test_results (id, project_id, run_id, catalog_test_id, result_key, outcome, severity, summary)
        VALUES ('16161616-1616-4616-8616-161616161616'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'::uuid,
                'http.cors.policy', 'http-cors-1', 'failed', 'high', 'Duplicate result')
        "#,
        "test_results_run_id_result_key_key",
    ).await;

    // Run type determines which target shape is legal.
    assert_sql_fails(
        &schema_pool,
        r#"
        INSERT INTO test_runs (id, project_id, job_id, plan_id, plan_version, plan_approval_id,
                               run_type, planned_test_id, load_scenario_id, load_profile_id,
                               target_authorization_id, idempotency_key)
        VALUES ('17171717-1717-4717-8717-171717171717'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid, 1,
                '88888888-8888-4888-8888-888888888888'::uuid,
                'vulnerability_testing', NULL,
                'cccccccc-cccc-4ccc-8ccc-cccccccccccc'::uuid,
                'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'::uuid,
                'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid, 'run-mismatch')
        "#,
        "test_runs_target_shape",
    )
    .await;

    // Foreign keys enforce tenancy boundaries.
    assert_sql_fails(
        &schema_pool,
        "INSERT INTO targets (id, project_id, normalized_origin, environment, allowed_hosts, allowed_paths, created_by) VALUES ('18181818-1818-4818-8818-181818181818'::uuid, '99999999-9999-4999-8999-999999999999'::uuid, 'https://orphan.example.test', 'staging', '[\"orphan.example.test\"]'::jsonb, '[\"/\"]'::jsonb, '11111111-1111-4111-8111-111111111111'::uuid)",
        "violates foreign key",
    ).await;

    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("test schema cleanup failed");
}

async fn seed_complete_fixture(pool: &PgPool) {
    for sql in [
        r#"
        INSERT INTO users (id, email, display_name)
        VALUES ('11111111-1111-4111-8111-111111111111'::uuid, 'owner@example.test', 'Owner')
        "#,
        r#"
        INSERT INTO projects (id, name, slug, owner_id)
        VALUES ('22222222-2222-4222-8222-222222222222'::uuid, 'Payments', 'payments',
                '11111111-1111-4111-8111-111111111111'::uuid)
        "#,
        r#"
        INSERT INTO project_members (project_id, user_id, role)
        VALUES ('22222222-2222-4222-8222-222222222222'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid, 'owner')
        "#,
        r#"
        INSERT INTO git_sources (id, project_id, provider, normalized_url, default_reference, credential_ref)
        VALUES ('33333333-3333-4333-8333-333333333333'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid, 'github',
                'https://github.com/acme/payments.git', 'main', 'secret://git/acme-payments')
        "#,
        r#"
        INSERT INTO jobs (id, project_id, created_by, goal, intensity, status, display_name,
                          git_source_id, source_reference, idempotency_key)
        VALUES ('44444444-4444-4444-8444-444444444444'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid,
                'vulnerability_testing', 'standard', 'queued', 'Payments review',
                '33333333-3333-4333-8333-333333333333'::uuid, 'main', 'job-1')
        "#,
        r#"
        INSERT INTO repository_snapshots (id, project_id, git_source_id, commit_sha, detected_languages, frameworks)
        VALUES ('55555555-5555-4555-8555-555555555555'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '33333333-3333-4333-8333-333333333333'::uuid,
                repeat('a', 40), ARRAY['TypeScript'], ARRAY['Express'])
        "#,
        r#"
        INSERT INTO test_plans (id, project_id, job_id, version, status, summary, confidence,
                                plan_payload, agent_provider, agent_model)
        VALUES ('66666666-6666-4666-8666-666666666666'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid, 1, 'proposed',
                'Review the payments API for transport and CORS issues.', 0.70,
                '{"schemaVersion":"1"}'::jsonb, 'test-provider', 'test-model')
        "#,
        r#"
        UPDATE jobs
        SET current_plan_id = '66666666-6666-4666-8666-666666666666'::uuid,
            status = 'awaiting_approval'
        WHERE id = '44444444-4444-4444-8444-444444444444'::uuid
        "#,
        r#"
        INSERT INTO planned_tests (id, project_id, plan_id, catalog_test_id, name, rationale,
                                   risk, prerequisites, expected_evidence, target_requirements,
                                   requires_explicit_confirmation, configuration, selected)
        VALUES ('77777777-7777-4777-8777-777777777777'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid,
                'http.cors.policy', 'CORS policy review',
                'The application configures permissive CORS middleware.', 'medium',
                '["authorized target"]'::jsonb, 'Preflight response headers.',
                '["https origin"]'::jsonb, false, '{"maxRequests":5}'::jsonb, true)
        "#,
        r#"
        INSERT INTO plan_approvals (id, project_id, plan_id, plan_version, approved_by,
                                    approved_test_ids, approval_snapshot)
        VALUES ('88888888-8888-4888-8888-888888888888'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid, 1,
                '11111111-1111-4111-8111-111111111111'::uuid,
                ARRAY['77777777-7777-4777-8777-777777777777'::uuid],
                '{"planId":"66666666-6666-4666-8666-666666666666"}'::jsonb)
        "#,
        r#"
        INSERT INTO targets (id, project_id, normalized_origin, environment, allowed_hosts,
                             allowed_paths, verification_status, created_by)
        VALUES ('99999999-9999-4999-8999-999999999999'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'https://staging.example.test', 'staging',
                '["staging.example.test"]'::jsonb, '["/api/"]'::jsonb, 'verified',
                '11111111-1111-4111-8111-111111111111'::uuid)
        "#,
        r#"
        INSERT INTO target_authorizations (id, project_id, target_id, authorized_by,
                                           authorization_method, scope, limits, expires_at)
        VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '99999999-9999-4999-8999-999999999999'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid,
                'explicit_confirmation',
                '{"origin":"https://staging.example.test","allowedHosts":["staging.example.test"],"allowedPaths":["/api/"],"allowedPorts":[443]}'::jsonb,
                '{"maxRequestsPerSecond":10,"maxConcurrency":5,"maxDurationSeconds":600,"maxRequests":1000}'::jsonb,
                now() + interval '1 day')
        "#,
        r#"
        INSERT INTO load_profiles (id, project_id, name, virtual_users, ramp_up_seconds,
                                   duration_seconds, max_requests, max_requests_per_second, stop_conditions)
        VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid, 'Product listing', 10, 30, 120, 5000, 50,
                '{"errorRate":0.05}'::jsonb)
        "#,
        r#"
        INSERT INTO load_scenarios (id, project_id, plan_id, profile_id, name, scenario, selected)
        VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid,
                'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'::uuid,
                'Product listing', '{"name":"Product listing"}'::jsonb, true)
        "#,
        r#"
        INSERT INTO schedules (id, project_id, job_id, plan_id, load_scenario_id, load_profile_id,
                               timezone, recurrence, enabled, next_run_at, created_by)
        VALUES ('dddddddd-dddd-4ddd-8ddd-dddddddddddd'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid,
                'cccccccc-cccc-4ccc-8ccc-cccccccccccc'::uuid,
                'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'::uuid,
                'UTC', '0 * * * *', true, now() + interval '1 hour',
                '11111111-1111-4111-8111-111111111111'::uuid)
        "#,
        r#"
        INSERT INTO test_runs (id, project_id, job_id, plan_id, plan_version, plan_approval_id,
                               run_type, status, planned_test_id, target_authorization_id,
                               schedule_id, idempotency_key)
        VALUES ('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid,
                '66666666-6666-4666-8666-666666666666'::uuid, 1,
                '88888888-8888-4888-8888-888888888888'::uuid,
                'vulnerability_testing', 'queued',
                '77777777-7777-4777-8777-777777777777'::uuid,
                'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'::uuid,
                'dddddddd-dddd-4ddd-8ddd-dddddddddddd'::uuid, 'run-1')
        "#,
        r#"
        INSERT INTO test_results (id, project_id, run_id, catalog_test_id, result_key, outcome,
                                  severity, confidence, summary, details, tool_name, tool_version, duration_ms)
        VALUES ('ffffffff-ffff-4fff-8fff-ffffffffffff'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'::uuid,
                'http.cors.policy', 'http-cors-1', 'failed', 'high', 0.90,
                'The preflight response reflected an untrusted origin.',
                '{"statusCode":204}'::jsonb, 'http-adapter', '1.0.0', 412)
        "#,
        r#"
        INSERT INTO evidence (id, project_id, result_id, storage_ref, sha256, mime_type, size_bytes)
        VALUES ('19191919-1919-4919-8919-191919191919'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'ffffffff-ffff-4fff-8fff-ffffffffffff'::uuid,
                'evidence/result-1.json', repeat('a', 64), 'application/json', 256)
        "#,
        r#"
        INSERT INTO job_events (id, job_id, sequence, event_type, payload)
        VALUES ('20202020-2020-4020-8020-202020202020'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid, 1, 'job.queued',
                '{"status":"queued"}'::jsonb)
        "#,
        r#"
        INSERT INTO run_metrics (id, project_id, run_id, bucket_start, bucket_seconds, endpoint,
                                 status_code, request_count, error_count, latency_p50_ms,
                                 latency_p95_ms, latency_p99_ms, throughput_rps, response_bytes)
        VALUES ('21212121-2121-4121-8121-212121212121'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'::uuid,
                date_trunc('minute', now()), 60, '/api/products', 200, 100, 0, 120.5, 240.0, 410.0, 8.4, 65536)
        "#,
        r#"
        INSERT INTO agent_sessions (id, project_id, job_id, provider, model, status, trace_metadata, finished_at)
        VALUES ('22222222-2222-4222-8222-222222222223'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '44444444-4444-4444-8444-444444444444'::uuid,
                'test-provider', 'test-model', 'completed', '{"redacted":true}'::jsonb, now())
        "#,
        r#"
        INSERT INTO audit_log (id, project_id, actor_id, action, resource_type, resource_id, metadata)
        VALUES ('23232323-2323-4323-8323-232323232323'::uuid,
                '22222222-2222-4222-8222-222222222222'::uuid,
                '11111111-1111-4111-8111-111111111111'::uuid,
                'job.created', 'job', '44444444-4444-4444-8444-444444444444'::uuid,
                '{"redacted":true}'::jsonb)
        "#,
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("fixture insert failed: {error}\n{sql}"));
    }
}
