//! Run materialization and cancellation tests. Fixtures are inert rows; no
//! target, scanner, or load traffic is used.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct World {
    app: TestApp,
    owner: Session,
    viewer: Session,
    outsider: Session,
    project_id: Uuid,
    owner_id: Uuid,
}

async fn setup() -> Option<World> {
    let app = TestApp::setup().await?;
    let owner_id = app
        .create_user("runs-owner@example.test", "Runs Owner", PASSWORD)
        .await;
    let viewer_id = app
        .create_user("runs-viewer@example.test", "Runs Viewer", PASSWORD)
        .await;
    app.create_user("runs-outsider@example.test", "Runs Outsider", PASSWORD)
        .await;
    let owner = app.login("runs-owner@example.test", PASSWORD).await;
    let viewer = app.login("runs-viewer@example.test", PASSWORD).await;
    let outsider = app.login("runs-outsider@example.test", PASSWORD).await;

    let project_id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)")
        .bind(project_id)
        .bind("Runs Fixture")
        .bind(format!("runs-fixture-{project_id}"))
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
        owner_id,
    })
}

async fn seed_target_authorization(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    active: bool,
) -> Uuid {
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
    let authorization_id: Uuid = sqlx::query_scalar(
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
    .expect("authorization insert");

    if !active {
        sqlx::query("UPDATE target_authorizations SET revoked_at = now() WHERE id = $1")
            .bind(authorization_id)
            .execute(&app.pool)
            .await
            .expect("authorization revoke");
    }
    authorization_id
}

async fn seed_job(app: &TestApp, project_id: Uuid, owner_id: Uuid, goal: &str) -> Uuid {
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
    sqlx::query_scalar(
        "INSERT INTO jobs (
            project_id, created_by, goal, intensity, status, display_name,
            git_source_id, source_reference
         ) VALUES ($1, $2, $3, 'standard', 'awaiting_approval', $4, $5, 'main')
         RETURNING id",
    )
    .bind(project_id)
    .bind(owner_id)
    .bind(goal)
    .bind(format!("Runs fixture {goal}"))
    .bind(git_source_id)
    .fetch_one(&app.pool)
    .await
    .expect("job insert")
}

/// Seed an approved vulnerability plan with one selected planned test.
async fn seed_approved_vulnerability_plan(app: &TestApp, project_id: Uuid, owner_id: Uuid) -> Uuid {
    let job_id = seed_job(app, project_id, owner_id, "vulnerability_testing").await;
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'Inert runs plan', 0.50,
                   $4, 0)",
    )
    .bind(plan_id)
    .bind(project_id)
    .bind(job_id)
    .bind(json!({"goal": "vulnerability_testing"}))
    .execute(&app.pool)
    .await
    .expect("plan insert");

    let planned_test_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO planned_tests (
            id, project_id, plan_id, catalog_test_id, name, rationale, risk,
            expected_evidence, selected
         ) VALUES ($1, $2, $3, 'http.headers.missing', 'Missing headers',
                   'Inert fixture', 'low', 'Headers', true)",
    )
    .bind(planned_test_id)
    .bind(project_id)
    .bind(plan_id)
    .execute(&app.pool)
    .await
    .expect("planned test insert");

    sqlx::query(
        "INSERT INTO plan_approvals (
            project_id, plan_id, plan_version, approved_by, approved_test_ids,
            approved_load_scenario_ids, approval_snapshot, plan_revision
         ) VALUES ($1, $2, 1, $3, ARRAY[$4]::uuid[], '{}'::uuid[], '{}', 0)",
    )
    .bind(project_id)
    .bind(plan_id)
    .bind(owner_id)
    .bind(planned_test_id)
    .execute(&app.pool)
    .await
    .expect("approval insert");

    plan_id
}

/// Seed an approved load plan with one selected scenario.
async fn seed_approved_load_plan(app: &TestApp, project_id: Uuid, owner_id: Uuid) -> Uuid {
    let job_id = seed_job(app, project_id, owner_id, "load_testing").await;
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO load_profiles (
            id, project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
            max_requests, max_requests_per_second, stop_conditions
         ) VALUES ($1, $2, $3, 2, 1, 10, 100, 5, '{}'::jsonb)",
    )
    .bind(profile_id)
    .bind(project_id)
    .bind(format!("Runs profile {}", Uuid::new_v4()))
    .execute(&app.pool)
    .await
    .expect("profile insert");

    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'Inert load plan', 0.50, $4, 0)",
    )
    .bind(plan_id)
    .bind(project_id)
    .bind(job_id)
    .bind(json!({"goal": "load_testing"}))
    .execute(&app.pool)
    .await
    .expect("load plan insert");

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
    .bind(format!("Runs scenario {}", Uuid::new_v4()))
    .bind(json!({
        "name": "baseline",
        "requests": [{"id": "baseline", "method": "GET", "path": "/", "expectedStatuses": [200]}]
    }))
    .execute(&app.pool)
    .await
    .expect("scenario insert");

    sqlx::query(
        "INSERT INTO plan_approvals (
            project_id, plan_id, plan_version, approved_by, approved_test_ids,
            approved_load_scenario_ids, approval_snapshot, plan_revision
         ) VALUES ($1, $2, 1, $3, '{}'::uuid[], ARRAY[$4]::uuid[], '{}', 0)",
    )
    .bind(project_id)
    .bind(plan_id)
    .bind(owner_id)
    .bind(scenario_id)
    .execute(&app.pool)
    .await
    .expect("load approval insert");

    plan_id
}

#[tokio::test]
async fn creates_runs_idempotently_and_denies_unauthorized_callers() {
    let Some(world) = setup().await else {
        return;
    };
    let plan_id =
        seed_approved_vulnerability_plan(&world.app, world.project_id, world.owner_id).await;
    let authorization =
        seed_target_authorization(&world.app, world.project_id, world.owner_id, true).await;

    let body = json!({ "targetAuthorizationId": authorization });

    // Viewer cannot create runs.
    let viewer = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.viewer),
            Some(body.clone()),
        )
        .await;
    assert_eq!(viewer.status, StatusCode::FORBIDDEN);

    // Outsider sees nothing.
    let outsider = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.outsider),
            Some(body.clone()),
        )
        .await;
    assert_eq!(outsider.status, StatusCode::NOT_FOUND);

    let created = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.owner),
            Some(body.clone()),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{:?}", created.body);
    let runs = created.body.as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["runType"], "vulnerability_testing");
    assert_eq!(runs[0]["status"], "queued");
    let first_id = runs[0]["id"].clone();

    // Repeating the request is idempotent: same run, no duplicate.
    let again = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.owner),
            Some(body),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body.as_array().expect("runs").len(), 1);
    assert_eq!(again.body[0]["id"], first_id);

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM test_runs WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_one(&world.app.pool)
        .await
        .expect("run count");
    assert_eq!(stored, 1, "idempotent creation must not duplicate runs");

    world.app.cleanup().await;
}

#[tokio::test]
async fn rejects_inactive_authorization_and_unapproved_plan() {
    let Some(world) = setup().await else {
        return;
    };
    let plan_id =
        seed_approved_vulnerability_plan(&world.app, world.project_id, world.owner_id).await;
    let revoked =
        seed_target_authorization(&world.app, world.project_id, world.owner_id, false).await;

    let inactive = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.owner),
            Some(json!({ "targetAuthorizationId": revoked })),
        )
        .await;
    assert_eq!(inactive.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        inactive.body["error"]["code"],
        "target_authorization_inactive"
    );

    // An unapproved plan cannot produce runs.
    let draft_plan = Uuid::new_v4();
    let job_id: Uuid = sqlx::query_scalar("SELECT id FROM jobs WHERE project_id = $1 LIMIT 1")
        .bind(world.project_id)
        .fetch_one(&world.app.pool)
        .await
        .expect("job id");
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 2, 'proposed', '1', 'Draft', 0.50, $4, 0)",
    )
    .bind(draft_plan)
    .bind(world.project_id)
    .bind(job_id)
    .bind(json!({"goal": "vulnerability_testing"}))
    .execute(&world.app.pool)
    .await
    .expect("draft plan insert");
    let active =
        seed_target_authorization(&world.app, world.project_id, world.owner_id, true).await;

    let not_approved = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{draft_plan}/runs"),
            Some(&world.owner),
            Some(json!({ "targetAuthorizationId": active })),
        )
        .await;
    assert_eq!(not_approved.status, StatusCode::CONFLICT);
    assert_eq!(not_approved.body["error"]["code"], "plan_not_approved");

    world.app.cleanup().await;
}

#[tokio::test]
async fn creates_load_runs_and_cancels_queued_runs() {
    let Some(world) = setup().await else {
        return;
    };
    let plan_id = seed_approved_load_plan(&world.app, world.project_id, world.owner_id).await;
    let authorization =
        seed_target_authorization(&world.app, world.project_id, world.owner_id, true).await;

    let created = world
        .app
        .send(
            Method::POST,
            &format!("/api/plans/{plan_id}/runs"),
            Some(&world.owner),
            Some(json!({ "targetAuthorizationId": authorization })),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{:?}", created.body);
    assert_eq!(created.body[0]["runType"], "load_testing");
    let run_id = created.body[0]["id"].as_str().expect("run id");

    let cancelled = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{run_id}/cancel"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:?}", cancelled.body);
    assert_eq!(cancelled.body["status"], "cancelled");

    // Cancelling again is idempotent.
    let again = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{run_id}/cancel"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["status"], "cancelled");

    // A leased running run is stopped by its worker, not the API.
    let running = Uuid::new_v4();
    let planned_test_id: Uuid =
        sqlx::query_scalar("SELECT planned_test_id FROM test_runs WHERE id = $1")
            .bind(Uuid::parse_str(run_id).expect("run uuid"))
            .fetch_one(&world.app.pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(Uuid::new_v4);
    sqlx::query(
        "INSERT INTO test_runs (
            id, project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, planned_test_id, load_scenario_id, load_profile_id,
            target_authorization_id, idempotency_key, worker_id, lease_expires_at
         )
         SELECT $1, project_id, job_id, plan_id, plan_version, plan_approval_id,
                run_type, 'running', planned_test_id, load_scenario_id, load_profile_id,
                target_authorization_id, $2, 'worker-test', now() + interval '5 minutes'
         FROM test_runs WHERE id = $3",
    )
    .bind(running)
    .bind(format!("running-{}", Uuid::new_v4()))
    .bind(Uuid::parse_str(run_id).expect("run uuid"))
    .execute(&world.app.pool)
    .await
    .expect("running run insert");
    let _ = planned_test_id;

    let in_progress = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{running}/cancel"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(in_progress.status, StatusCode::CONFLICT);
    assert_eq!(in_progress.body["error"]["code"], "run_in_progress");

    world.app.cleanup().await;
}

#[allow(dead_code)]
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
