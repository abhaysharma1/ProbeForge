//! Phase 15 end-to-end acceptance.
//!
//! A local mock HTTP service stands in for the target. Two full workflows run
//! against it: vulnerability (approved plan -> run -> native adapter ->
//! test_results/evidence -> report) and load (approved scenario -> run ->
//! constrained runner -> run_metrics). Cross-scope rejection is asserted too.
//! No real target, scanner, or external traffic is involved.

mod support;

use axum::http::{Method, StatusCode};
use probe_forge_execution_worker::{DispatchConfig, dispatch_once};
use serde_json::{Value, json};
use support::{Session, TestApp};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct MockTarget {
    port: u16,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for MockTarget {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn start_mock_target() -> MockTarget {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("mock bind");
    let port = listener.local_addr().expect("mock addr").port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0_u8; 4096];
                let Ok(_read) = stream.read(&mut buffer).await else {
                    return;
                };
                // Responses deliberately omit the baseline security headers and
                // return 200 for every path.
                write_ok(&mut stream, "{\"status\":\"ok\"}").await;
            });
        }
    });
    MockTarget { port, handle }
}

async fn write_ok(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

struct World {
    app: TestApp,
    owner: Session,
    project_id: Uuid,
    owner_id: Uuid,
}

async fn setup() -> Option<World> {
    let app = TestApp::setup().await?;
    let owner_id = app
        .create_user("e2e-owner@example.test", "E2E Owner", PASSWORD)
        .await;
    let owner = app.login("e2e-owner@example.test", PASSWORD).await;
    let project_id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)")
        .bind(project_id)
        .bind("E2E Fixture")
        .bind(format!("e2e-fixture-{project_id}"))
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
    Some(World {
        app,
        owner,
        project_id,
        owner_id,
    })
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
    .bind(format!("E2E {goal}"))
    .bind(git_source_id)
    .fetch_one(&app.pool)
    .await
    .expect("job insert")
}

async fn seed_authorization(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    origin: &str,
    host: &str,
    allowed_paths: &[&str],
    allowed_ports: &[u16],
) -> Uuid {
    let target_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO targets (
            id, project_id, normalized_origin, environment, allowed_hosts,
            allowed_paths, verification_status, created_by
         ) VALUES ($1, $2, $3, 'local', $4, $5, 'verified', $6)",
    )
    .bind(target_id)
    .bind(project_id)
    .bind(origin)
    .bind(json!([host]))
    .bind(json!(allowed_paths))
    .bind(owner_id)
    .execute(&app.pool)
    .await
    .expect("target insert");
    sqlx::query_scalar(
        "INSERT INTO target_authorizations (
            id, project_id, target_id, authorized_by, authorization_method,
            scope, limits, expires_at
         ) VALUES ($1, $2, $3, $4, 'explicit_confirmation', $5, $6,
                   now() + interval '1 hour')
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(target_id)
    .bind(owner_id)
    .bind(json!({
        "origin": origin,
        "allowedHosts": [host],
        "allowedPaths": allowed_paths,
        "allowedPorts": allowed_ports
    }))
    .bind(json!({
        "maxRequestsPerSecond": 5,
        "maxConcurrency": 2,
        "maxDurationSeconds": 10,
        "maxRequests": 10
    }))
    .fetch_one(&app.pool)
    .await
    .expect("authorization insert")
}

async fn seed_approved_vulnerability_plan(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    configuration: Value,
) -> Uuid {
    let job_id = seed_job(app, project_id, owner_id, "vulnerability_testing").await;
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'E2E vuln plan', 0.50, $4, 0)",
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
            expected_evidence, configuration, selected
         ) VALUES ($1, $2, $3, 'http.headers.missing', 'Missing headers',
                   'Inert fixture', 'low', 'Headers', $4, true)",
    )
    .bind(planned_test_id)
    .bind(project_id)
    .bind(plan_id)
    .bind(configuration)
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

async fn seed_approved_load_plan(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    scenario: Value,
) -> Uuid {
    let job_id = seed_job(app, project_id, owner_id, "load_testing").await;
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO load_profiles (
            id, project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
            max_requests, max_requests_per_second, stop_conditions
         ) VALUES ($1, $2, $3, 1, 0, 1, 2, 5, '{}'::jsonb)",
    )
    .bind(profile_id)
    .bind(project_id)
    .bind(format!("E2E profile {}", Uuid::new_v4()))
    .execute(&app.pool)
    .await
    .expect("profile insert");
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'E2E load plan', 0.50, $4, 0)",
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
    .bind(format!("E2E scenario {}", Uuid::new_v4()))
    .bind(scenario)
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

async fn create_run(world: &World, plan_id: Uuid, authorization: Uuid) -> String {
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
    created.body[0]["id"].as_str().expect("run id").to_owned()
}

fn dispatch_config() -> DispatchConfig {
    let mut config = DispatchConfig::new("e2e-worker");
    config.allow_private_targets = true;
    config
}

#[tokio::test]
async fn vulnerability_workflow_runs_end_to_end_against_a_local_mock() {
    let Some(world) = setup().await else {
        return;
    };
    let target = start_mock_target().await;
    let origin = format!("http://127.0.0.1:{}", target.port);
    let authorization = seed_authorization(
        &world.app,
        world.project_id,
        world.owner_id,
        &origin,
        "127.0.0.1",
        &["/"],
        &[target.port],
    )
    .await;
    let plan_id = seed_approved_vulnerability_plan(
        &world.app,
        world.project_id,
        world.owner_id,
        json!({"route": "/"}),
    )
    .await;
    let run_id = create_run(&world, plan_id, authorization).await;

    let dispatched = dispatch_once(&world.app.pool, &dispatch_config())
        .await
        .expect("dispatch");
    assert!(dispatched.is_some(), "a queued run should be dispatched");
    let run_uuid = Uuid::parse_str(&run_id).expect("run uuid");

    let status: String = sqlx::query_scalar("SELECT status FROM test_runs WHERE id = $1")
        .bind(run_uuid)
        .fetch_one(&world.app.pool)
        .await
        .expect("run status");
    assert_eq!(status, "completed");

    let (outcome, finding, severity, confidence): (String, Option<String>, Option<String>, f64) =
        sqlx::query_as(
            "SELECT outcome, finding_status, severity, confidence::float8
             FROM test_results WHERE run_id = $1",
        )
        .bind(run_uuid)
        .fetch_one(&world.app.pool)
        .await
        .expect("result row");
    assert_eq!(outcome, "failed");
    assert_eq!(finding.as_deref(), Some("confirmed"));
    assert_eq!(severity.as_deref(), Some("low"));
    assert!(confidence >= 0.8);

    let evidence_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM evidence e
         JOIN test_results r ON r.id = e.result_id
         WHERE r.run_id = $1",
    )
    .bind(run_uuid)
    .fetch_one(&world.app.pool)
    .await
    .expect("evidence count");
    assert!(evidence_count >= 1);

    // The stored result feeds report generation with a traceable claim.
    let report = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{run_id}/report"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(report.status, StatusCode::CREATED, "{:?}", report.body);
    let claims = report.body["latest"]["content"]["claims"]
        .as_array()
        .expect("claims");
    assert!(!claims.is_empty());
    for claim in claims {
        if claim["claimKind"] == "hypothesis" {
            continue;
        }
        let references = &claim["references"];
        let total = references["resultIds"].as_array().map_or(0, Vec::len)
            + references["evidenceIds"].as_array().map_or(0, Vec::len)
            + references["metricIds"].as_array().map_or(0, Vec::len);
        assert!(total > 0, "untraceable claim: {claim}");
    }

    world.app.cleanup().await;
}

#[tokio::test]
async fn load_workflow_runs_end_to_end_against_a_local_mock() {
    let Some(world) = setup().await else {
        return;
    };
    let target = start_mock_target().await;
    let origin = format!("http://127.0.0.1:{}", target.port);
    let authorization = seed_authorization(
        &world.app,
        world.project_id,
        world.owner_id,
        &origin,
        "127.0.0.1",
        &["/"],
        &[target.port],
    )
    .await;
    let scenario = json!({
        "name": "E2E baseline",
        "description": "Local mock; no real target.",
        "requests": [{
            "id": "baseline",
            "method": "GET",
            "path": "/",
            "headers": {},
            "secretHeaders": {},
            "dependsOn": [],
            "extract": [],
            "assertions": [],
            "expectedStatuses": [200]
        }],
        "profile": {
            "virtualUsers": 1,
            "rampUpSeconds": 0,
            "durationSeconds": 1,
            "maxRequests": 2,
            "maxRequestsPerSecond": 5.0,
            "stopConditions": {}
        }
    });
    let plan_id =
        seed_approved_load_plan(&world.app, world.project_id, world.owner_id, scenario).await;
    let run_id = create_run(&world, plan_id, authorization).await;

    let dispatched = dispatch_once(&world.app.pool, &dispatch_config())
        .await
        .expect("dispatch");
    assert!(dispatched.is_some());
    let run_uuid = Uuid::parse_str(&run_id).expect("run uuid");

    let status: String = sqlx::query_scalar("SELECT status FROM test_runs WHERE id = $1")
        .bind(run_uuid)
        .fetch_one(&world.app.pool)
        .await
        .expect("run status");
    assert_eq!(status, "completed");

    let requests: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0)::bigint FROM run_metrics WHERE run_id = $1",
    )
    .bind(run_uuid)
    .fetch_one(&world.app.pool)
    .await
    .expect("metric sum");
    assert!(requests >= 1, "load run should record request metrics");

    world.app.cleanup().await;
}

#[tokio::test]
async fn out_of_scope_load_run_is_rejected_before_any_request() {
    let Some(world) = setup().await else {
        return;
    };
    let target = start_mock_target().await;
    let origin = format!("http://127.0.0.1:{}", target.port);
    // The authorization only permits /allowed, but the scenario requests "/".
    let authorization = seed_authorization(
        &world.app,
        world.project_id,
        world.owner_id,
        &origin,
        "127.0.0.1",
        &["/allowed"],
        &[target.port],
    )
    .await;
    let scenario = json!({
        "name": "E2E out-of-scope",
        "description": "Local mock; no real target.",
        "requests": [{
            "id": "baseline",
            "method": "GET",
            "path": "/",
            "headers": {},
            "secretHeaders": {},
            "dependsOn": [],
            "extract": [],
            "assertions": [],
            "expectedStatuses": [200]
        }],
        "profile": {
            "virtualUsers": 1,
            "rampUpSeconds": 0,
            "durationSeconds": 1,
            "maxRequests": 1,
            "maxRequestsPerSecond": 1.0,
            "stopConditions": {}
        }
    });
    let plan_id =
        seed_approved_load_plan(&world.app, world.project_id, world.owner_id, scenario).await;
    let run_id = create_run(&world, plan_id, authorization).await;

    let _ = dispatch_once(&world.app.pool, &dispatch_config())
        .await
        .expect("dispatch");
    let run_uuid = Uuid::parse_str(&run_id).expect("run uuid");

    let (status, code): (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error_code FROM test_runs WHERE id = $1")
            .bind(run_uuid)
            .fetch_one(&world.app.pool)
            .await
            .expect("run row");
    assert_eq!(status, "failed");
    assert_eq!(code.as_deref(), Some("target_scope_violation"));

    let metrics: i64 = sqlx::query_scalar("SELECT count(*) FROM run_metrics WHERE run_id = $1")
        .bind(run_uuid)
        .fetch_one(&world.app.pool)
        .await
        .expect("metric count");
    assert_eq!(metrics, 0, "no request should have been sent");

    world.app.cleanup().await;
}
