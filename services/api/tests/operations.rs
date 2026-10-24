//! Phase 14 observability, quotas, rate limiting, and retention tests.
//!
//! Fixtures are inert rows; no target, scanner, or load traffic is used. Queue
//! functions are exercised directly against the same schema the API uses.

mod support;

use axum::http::{Method, StatusCode};
use probe_forge_queue::{NewProjectQuota, RetentionPolicy};
use serde_json::{Value, json};
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";
const INERT_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct World {
    app: TestApp,
    owner: Session,
    viewer: Session,
    outsider: Session,
    project_id: Uuid,
}

async fn setup() -> Option<World> {
    let app = TestApp::setup().await?;
    let owner_id = app
        .create_user("ops-owner@example.test", "Ops Owner", PASSWORD)
        .await;
    let viewer_id = app
        .create_user("ops-viewer@example.test", "Ops Viewer", PASSWORD)
        .await;
    app.create_user("ops-outsider@example.test", "Ops Outsider", PASSWORD)
        .await;

    let owner = app.login("ops-owner@example.test", PASSWORD).await;
    let viewer = app.login("ops-viewer@example.test", PASSWORD).await;
    let outsider = app.login("ops-outsider@example.test", PASSWORD).await;

    let project_id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)")
        .bind(project_id)
        .bind("Ops Fixture")
        .bind(format!("ops-fixture-{project_id}"))
        .bind(owner_id)
        .execute(&app.pool)
        .await
        .expect("project insert");
    for (user_id, role) in [(owner_id, "owner"), (viewer_id, "viewer")] {
        sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(project_id)
            .bind(user_id)
            .bind(role)
            .execute(&app.pool)
            .await
            .expect("member insert");
    }

    Some(World {
        app,
        owner,
        viewer,
        outsider,
        project_id,
    })
}

fn member_id(session: &Session) -> Uuid {
    Uuid::parse_str(
        session
            .user
            .get("id")
            .and_then(Value::as_str)
            .expect("session user id"),
    )
    .expect("session user uuid")
}

async fn seed_target_authorization(app: &TestApp, project_id: Uuid, owner_id: Uuid) -> Uuid {
    let target_id = Uuid::new_v4();
    let origin = format!("https://inert-{}.example.test", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO targets (
            id, project_id, normalized_origin, environment, allowed_hosts,
            allowed_paths, verification_status, created_by
         ) VALUES ($1, $2, $3, 'staging', $4, '[\"/\"]'::jsonb, 'verified', $5)",
    )
    .bind(target_id)
    .bind(project_id)
    .bind(&origin)
    .bind(json!([origin.trim_start_matches("https://")]))
    .bind(owner_id)
    .execute(&app.pool)
    .await
    .expect("target insert");
    sqlx::query_scalar(
        "INSERT INTO target_authorizations (
            id, project_id, target_id, authorized_by, authorization_method,
            scope, limits, expires_at
         ) VALUES ($1, $2, $3, $4, 'explicit_confirmation', $5, $6,
                   now() + interval '1 day')
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(target_id)
    .bind(owner_id)
    .bind(json!({
        "origin": origin,
        "allowedHosts": [origin.trim_start_matches("https://")],
        "allowedPaths": ["/"],
        "allowedPorts": [443]
    }))
    .bind(json!({
        "maxRequestsPerSecond": 5,
        "maxConcurrency": 5,
        "maxDurationSeconds": 60,
        "maxRequests": 100
    }))
    .fetch_one(&app.pool)
    .await
    .expect("authorization insert")
}

/// Seed an approved load plan, a queued load run, and a valid target
/// authorization. Returns the queued run id.
async fn seed_queued_load_run(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    max_requests: i64,
) -> Uuid {
    let git_source_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_sources (id, project_id, provider, normalized_url, default_reference)
         VALUES ($1, $2, 'github', $3, 'main')",
    )
    .bind(git_source_id)
    .bind(project_id)
    .bind(format!(
        "https://github.com/example/inert-{}.git",
        Uuid::new_v4()
    ))
    .execute(&app.pool)
    .await
    .expect("git source insert");

    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (
            project_id, created_by, goal, intensity, status, display_name,
            git_source_id, source_reference
         ) VALUES ($1, $2, 'load_testing', 'standard', 'awaiting_approval', $3, $4, 'main')
         RETURNING id",
    )
    .bind(project_id)
    .bind(owner_id)
    .bind(format!("Ops load {}", Uuid::new_v4()))
    .bind(git_source_id)
    .fetch_one(&app.pool)
    .await
    .expect("job insert");

    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO load_profiles (
            id, project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
            max_requests, max_requests_per_second, stop_conditions
         ) VALUES ($1, $2, $3, 2, 1, 10, $4, 5, '{}'::jsonb)",
    )
    .bind(profile_id)
    .bind(project_id)
    .bind(format!("Ops profile {}", Uuid::new_v4()))
    .bind(max_requests)
    .execute(&app.pool)
    .await
    .expect("profile insert");

    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'Inert ops plan', 0.50, '{}', 0)",
    )
    .bind(plan_id)
    .bind(project_id)
    .bind(job_id)
    .execute(&app.pool)
    .await
    .expect("plan insert");

    let scenario_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO load_scenarios (
            id, project_id, plan_id, profile_id, name, schema_version, scenario, selected
         ) VALUES ($1, $2, $3, $4, $5, '1', $6, true)",
    )
    .bind(scenario_id)
    .bind(project_id)
    .bind(plan_id)
    .bind(profile_id)
    .bind(format!("Ops scenario {}", Uuid::new_v4()))
    .bind(json!({
        "name": "ops",
        "requests": [{"id": "baseline", "method": "GET", "path": "/", "expectedStatuses": [200]}]
    }))
    .execute(&app.pool)
    .await
    .expect("scenario insert");

    let approval_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plan_approvals (
            project_id, plan_id, plan_version, approved_by, approved_test_ids,
            approved_load_scenario_ids, approval_snapshot, plan_revision
         ) VALUES ($1, $2, 1, $3, '{}'::uuid[], ARRAY[$4]::uuid[], '{}', 0)
         RETURNING id",
    )
    .bind(project_id)
    .bind(plan_id)
    .bind(owner_id)
    .bind(scenario_id)
    .fetch_one(&app.pool)
    .await
    .expect("approval insert");

    let authorization_id = seed_target_authorization(app, project_id, owner_id).await;
    sqlx::query_scalar(
        "INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, load_scenario_id, load_profile_id, target_authorization_id,
            idempotency_key, scheduled_for
         ) VALUES ($1, $2, $3, 1, $4, 'load_testing', 'queued', $5, $6, $7, $8, now())
         RETURNING id",
    )
    .bind(project_id)
    .bind(job_id)
    .bind(plan_id)
    .bind(approval_id)
    .bind(scenario_id)
    .bind(profile_id)
    .bind(authorization_id)
    .bind(format!("ops-{}", Uuid::new_v4()))
    .fetch_one(&app.pool)
    .await
    .expect("run insert")
}

#[tokio::test]
async fn request_id_is_echoed_on_every_response() {
    let Some(world) = setup().await else {
        return;
    };

    let response = world
        .app
        .send(Method::GET, "/api/projects", Some(&world.owner), None)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let echoed = response
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("x-request-id header");
    assert!(!echoed.is_empty());

    world.app.cleanup().await;
}

#[tokio::test]
async fn api_rate_limit_rejects_when_exceeded() {
    let mut auth = probe_forge_api::AuthConfig::local_test();
    auth.api_requests_per_minute = 3;
    auth.expensive_requests_per_minute = 1;
    let Some(app) = TestApp::setup_with_auth(auth).await else {
        return;
    };
    let owner_id = app
        .create_user("ops-ratelimit@example.test", "Ops Rate", PASSWORD)
        .await;
    let owner = app.login("ops-ratelimit@example.test", PASSWORD).await;
    let project_id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)")
        .bind(project_id)
        .bind("Rate Fixture")
        .bind(format!("rate-fixture-{project_id}"))
        .bind(owner_id)
        .execute(&app.pool)
        .await
        .expect("project insert");
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(project_id)
        .bind(owner_id)
        .execute(&app.pool)
        .await
        .expect("member insert");

    let mut last_status = StatusCode::OK;
    for _ in 0..5 {
        let response = app
            .send(Method::GET, "/api/projects", Some(&owner), None)
            .await;
        last_status = response.status;
        if last_status == StatusCode::TOO_MANY_REQUESTS {
            assert!(response.headers.get("retry-after").is_some());
            break;
        }
    }
    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);

    app.cleanup().await;
}

#[tokio::test]
async fn queue_metrics_report_depth_and_active_work() {
    let Some(world) = setup().await else {
        return;
    };
    seed_queued_load_run(&world.app, world.project_id, member_id(&world.owner), 100).await;

    let metrics = probe_forge_queue::queue_metrics(&world.app.pool)
        .await
        .expect("queue metrics");
    assert!(
        metrics.runs_queued >= 1,
        "expected a queued run: {metrics:?}"
    );
    assert_eq!(metrics.failed_window_seconds, 3600);

    world.app.cleanup().await;
}

#[tokio::test]
async fn project_quota_is_manageable_and_enforced_server_side() {
    let Some(world) = setup().await else {
        return;
    };
    let owner_id = member_id(&world.owner);

    // Viewer cannot change the quota.
    let viewer_put = world
        .app
        .send(
            Method::PUT,
            &format!("/api/projects/{}/quota", world.project_id),
            Some(&world.viewer),
            Some(json!({
                "maxActiveRuns": 1,
                "maxRequestsPerWindow": null,
                "windowSeconds": 86400
            })),
        )
        .await;
    assert_eq!(viewer_put.status, StatusCode::FORBIDDEN);

    // A non-member cannot even read it.
    let outsider_get = world
        .app
        .send(
            Method::GET,
            &format!("/api/projects/{}/quota", world.project_id),
            Some(&world.outsider),
            None,
        )
        .await;
    assert_eq!(outsider_get.status, StatusCode::NOT_FOUND);

    // Owner sets a one-active-run cap.
    let put = world
        .app
        .send(
            Method::PUT,
            &format!("/api/projects/{}/quota", world.project_id),
            Some(&world.owner),
            Some(json!({
                "maxActiveRuns": 1,
                "maxRequestsPerWindow": null,
                "windowSeconds": 86400
            })),
        )
        .await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
    assert_eq!(put.body["configured"], true);
    assert_eq!(put.body["maxActiveRuns"], 1);

    let get = world
        .app
        .send(
            Method::GET,
            &format!("/api/projects/{}/quota", world.project_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(get.status, StatusCode::OK);
    assert_eq!(get.body["maxActiveRuns"], 1);

    // Two queued runs, but the cap allows only one concurrent claim.
    seed_queued_load_run(&world.app, world.project_id, owner_id, 100).await;
    seed_queued_load_run(&world.app, world.project_id, owner_id, 100).await;

    let first = probe_forge_queue::claim_due_run_fair(&world.app.pool, "ops-worker", 60, 100, 5)
        .await
        .expect("first claim");
    assert!(first.is_some(), "first run should claim");
    let second = probe_forge_queue::claim_due_run_fair(&world.app.pool, "ops-worker", 60, 100, 5)
        .await
        .expect("second claim");
    assert!(
        second.is_none(),
        "per-project active-run cap must block the second claim"
    );

    world.app.cleanup().await;
}

#[tokio::test]
async fn request_budget_blocks_claims_over_the_window_budget() {
    let Some(world) = setup().await else {
        return;
    };
    let owner_id = member_id(&world.owner);
    seed_queued_load_run(&world.app, world.project_id, owner_id, 100).await;

    probe_forge_queue::set_project_quota(
        &world.app.pool,
        NewProjectQuota {
            project_id: world.project_id,
            max_active_runs: 5,
            max_requests_per_window: Some(50),
            window_seconds: 86_400,
        },
    )
    .await
    .expect("set quota");

    let blocked = probe_forge_queue::claim_due_run_fair(&world.app.pool, "ops-worker", 60, 100, 5)
        .await
        .expect("claim");
    assert!(
        blocked.is_none(),
        "a 100-request run must not fit a 50-request budget"
    );

    // Raising the budget lets the same run claim.
    probe_forge_queue::set_project_quota(
        &world.app.pool,
        NewProjectQuota {
            project_id: world.project_id,
            max_active_runs: 5,
            max_requests_per_window: Some(200),
            window_seconds: 86_400,
        },
    )
    .await
    .expect("raise quota");
    let allowed = probe_forge_queue::claim_due_run_fair(&world.app.pool, "ops-worker", 60, 100, 5)
        .await
        .expect("claim");
    assert!(allowed.is_some(), "the run should now fit the budget");

    world.app.cleanup().await;
}

#[tokio::test]
async fn retention_purges_expired_data_and_preserves_immutability() {
    let Some(world) = setup().await else {
        return;
    };
    let owner_id = member_id(&world.owner);

    // A completed run with a result and two evidence rows.
    let run_id = seed_queued_load_run(&world.app, world.project_id, owner_id, 100).await;
    let result_id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_results (
            project_id, run_id, catalog_test_id, result_key, outcome, severity,
            confidence, summary, details
         ) VALUES ($1, $2, 'http.headers.missing', $3, 'failed', 'low', 0.5,
                   'Inert retention fixture', '{}'::jsonb)
         RETURNING id",
    )
    .bind(world.project_id)
    .bind(run_id)
    .bind(format!("ret-{}", Uuid::new_v4()))
    .fetch_one(&world.app.pool)
    .await
    .expect("result insert");

    let expired_evidence: Uuid = sqlx::query_scalar(
        "INSERT INTO evidence (
            project_id, result_id, storage_ref, sha256, mime_type, size_bytes,
            redacted, retention_until
         ) VALUES ($1, $2, $3, $4, 'text/plain', 10, true, now() - interval '1 day')
         RETURNING id",
    )
    .bind(world.project_id)
    .bind(result_id)
    .bind(format!("evidence/expired-{}", Uuid::new_v4()))
    .bind(INERT_SHA256)
    .fetch_one(&world.app.pool)
    .await
    .expect("expired evidence insert");

    let retained_evidence: Uuid = sqlx::query_scalar(
        "INSERT INTO evidence (
            project_id, result_id, storage_ref, sha256, mime_type, size_bytes,
            redacted, retention_until
         ) VALUES ($1, $2, $3, $4, 'text/plain', 10, true, now() + interval '30 days')
         RETURNING id",
    )
    .bind(world.project_id)
    .bind(result_id)
    .bind(format!("evidence/keep-{}", Uuid::new_v4()))
    .bind(INERT_SHA256)
    .fetch_one(&world.app.pool)
    .await
    .expect("retained evidence insert");

    // A failed job with an old event.
    let git_source_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_sources (id, project_id, provider, normalized_url, default_reference)
         VALUES ($1, $2, 'github', $3, 'main')",
    )
    .bind(git_source_id)
    .bind(world.project_id)
    .bind(format!(
        "https://github.com/example/inert-{}.git",
        Uuid::new_v4()
    ))
    .execute(&world.app.pool)
    .await
    .expect("git source insert");
    let failed_job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (
            project_id, created_by, goal, intensity, status, display_name,
            git_source_id, source_reference
         ) VALUES ($1, $2, 'vulnerability_testing', 'standard', 'failed', $3, $4, 'main')
         RETURNING id",
    )
    .bind(world.project_id)
    .bind(owner_id)
    .bind(format!("Old failed job {}", Uuid::new_v4()))
    .bind(git_source_id)
    .fetch_one(&world.app.pool)
    .await
    .expect("job insert");
    sqlx::query(
        "INSERT INTO job_events (job_id, sequence, event_type, payload, occurred_at)
         VALUES ($1, 1, 'job.failed', '{}'::jsonb, now() - interval '100 days')",
    )
    .bind(failed_job)
    .execute(&world.app.pool)
    .await
    .expect("event insert");

    let outcome = probe_forge_queue::run_retention(&world.app.pool, &RetentionPolicy::default())
        .await
        .expect("retention purge");
    assert!(outcome.evidence_deleted >= 1, "{outcome:?}");
    assert!(outcome.job_events_deleted >= 1, "{outcome:?}");

    let expired_still_there: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM evidence WHERE id = $1)")
            .bind(expired_evidence)
            .fetch_one(&world.app.pool)
            .await
            .expect("evidence exists");
    assert!(!expired_still_there, "expired evidence must be purged");

    let retained_still_there: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM evidence WHERE id = $1)")
            .bind(retained_evidence)
            .fetch_one(&world.app.pool)
            .await
            .expect("evidence exists");
    assert!(retained_still_there, "in-window evidence must be kept");

    // Regression: outside the sanctioned purge path, immutability still holds.
    let blocked = sqlx::query("DELETE FROM evidence WHERE id = $1")
        .bind(retained_evidence)
        .execute(&world.app.pool)
        .await;
    assert!(
        blocked.is_err(),
        "ordinary deletes must still be rejected by the immutability trigger"
    );

    world.app.cleanup().await;
}
