//! Phase 13 run results, recommendations, versioned reports, and comparison
//! tests. Fixtures are inert rows; no target, scanner, or load traffic is used.

mod support;

use axum::http::{Method, StatusCode};
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
        .create_user("results-owner@example.test", "Results Owner", PASSWORD)
        .await;
    let viewer_id = app
        .create_user("results-viewer@example.test", "Results Viewer", PASSWORD)
        .await;
    app.create_user(
        "results-outsider@example.test",
        "Results Outsider",
        PASSWORD,
    )
    .await;

    let owner = app.login("results-owner@example.test", PASSWORD).await;
    let viewer = app.login("results-viewer@example.test", PASSWORD).await;
    let outsider = app.login("results-outsider@example.test", PASSWORD).await;

    let project_id = Uuid::new_v4();
    sqlx::query("INSERT INTO projects (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)")
        .bind(project_id)
        .bind("Results Fixture")
        .bind(format!("results-fixture-{project_id}"))
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
    .bind(format!("Results fixture {goal}"))
    .bind(git_source_id)
    .fetch_one(&app.pool)
    .await
    .expect("job insert")
}

async fn seed_approved_plan(
    app: &TestApp,
    project_id: Uuid,
    job_id: Uuid,
    owner_id: Uuid,
    planned_test_id: Uuid,
) -> (Uuid, Uuid) {
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'Inert results plan', 0.50, '{}', 0)",
    )
    .bind(plan_id)
    .bind(project_id)
    .bind(job_id)
    .execute(&app.pool)
    .await
    .expect("plan insert");
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
    let approval_id: Uuid = sqlx::query_scalar(
        "INSERT INTO plan_approvals (
            project_id, plan_id, plan_version, approved_by, approved_test_ids,
            approved_load_scenario_ids, approval_snapshot, plan_revision
         ) VALUES ($1, $2, 1, $3, ARRAY[$4]::uuid[], '{}'::uuid[], '{}', 0)
         RETURNING id",
    )
    .bind(project_id)
    .bind(plan_id)
    .bind(owner_id)
    .bind(planned_test_id)
    .fetch_one(&app.pool)
    .await
    .expect("approval insert");
    (plan_id, approval_id)
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

struct SecurityRun {
    run_id: Uuid,
}

async fn seed_security_run(app: &TestApp, project_id: Uuid, owner_id: Uuid) -> SecurityRun {
    let job_id = seed_job(app, project_id, owner_id, "vulnerability_testing").await;
    let planned_test_id = Uuid::new_v4();
    let (plan_id, approval_id) =
        seed_approved_plan(app, project_id, job_id, owner_id, planned_test_id).await;
    let authorization_id = seed_target_authorization(app, project_id, owner_id).await;
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, planned_test_id, target_authorization_id, idempotency_key,
            started_at, finished_at
         ) VALUES ($1, $2, $3, 1, $4, 'vulnerability_testing', 'completed',
                   $5, $6, $7, now() - interval '5 minutes', now())
         RETURNING id",
    )
    .bind(project_id)
    .bind(job_id)
    .bind(plan_id)
    .bind(approval_id)
    .bind(planned_test_id)
    .bind(authorization_id)
    .bind(format!("results-{}", Uuid::new_v4()))
    .fetch_one(&app.pool)
    .await
    .expect("run insert");

    let _ = seed_result(
        app,
        project_id,
        run_id,
        "http.cors.policy",
        "passed",
        None,
        None,
        "CORS policy observed and acceptable.",
        json!({"note": "no credentials allowed"}),
    )
    .await;
    let failed_result_id = seed_result(
        app,
        project_id,
        run_id,
        "http.headers.missing",
        "failed",
        Some("confirmed"),
        Some("medium"),
        "Missing security header observed.",
        json!({
            "authorization": "Bearer super-secret-token",
            "observed": "x-content-type-options absent"
        }),
    )
    .await;
    sqlx::query("UPDATE test_results SET observation = $2 WHERE id = $1")
        .bind(failed_result_id)
        .bind(json!({
            "authorization": "Bearer super-secret-token",
            "observed": "x-content-type-options absent"
        }))
        .execute(&app.pool)
        .await
        .expect("observation update");
    seed_evidence(app, project_id, failed_result_id).await;
    SecurityRun { run_id }
}

#[allow(clippy::too_many_arguments)]
async fn seed_result(
    app: &TestApp,
    project_id: Uuid,
    run_id: Uuid,
    catalog_test_id: &str,
    outcome: &str,
    finding_status: Option<&str>,
    severity: Option<&str>,
    summary: &str,
    details: Value,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO test_results (
            project_id, run_id, catalog_test_id, result_key, outcome, finding_status,
            severity, confidence, summary, details, observation, tool_name,
            tool_version, duration_ms
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 0.80, $8, $9, '{}'::jsonb,
                   'http_probe', '0.1.0', 5)
         RETURNING id",
    )
    .bind(project_id)
    .bind(run_id)
    .bind(catalog_test_id)
    .bind(format!("result-{}", Uuid::new_v4()))
    .bind(outcome)
    .bind(finding_status)
    .bind(severity)
    .bind(summary)
    .bind(details)
    .fetch_one(&app.pool)
    .await
    .expect("result insert")
}

async fn seed_evidence(app: &TestApp, project_id: Uuid, result_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO evidence (
            project_id, result_id, storage_ref, sha256, mime_type, size_bytes, redacted
         ) VALUES ($1, $2, $3, $4, 'text/plain', 10, true)
         RETURNING id",
    )
    .bind(project_id)
    .bind(result_id)
    .bind(format!("evidence/{}", Uuid::new_v4()))
    .bind(INERT_SHA256)
    .fetch_one(&app.pool)
    .await
    .expect("evidence insert")
}

async fn seed_load_run(
    app: &TestApp,
    project_id: Uuid,
    owner_id: Uuid,
    virtual_users: i32,
    scenario_name: &str,
) -> Uuid {
    let job_id = seed_job(app, project_id, owner_id, "load_testing").await;
    let profile_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO load_profiles (
            id, project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
            max_requests, max_requests_per_second, stop_conditions
         ) VALUES ($1, $2, $3, $4, 1, 10, 100, 5, '{}'::jsonb)",
    )
    .bind(profile_id)
    .bind(project_id)
    .bind(format!("Profile {virtual_users}"))
    .bind(virtual_users)
    .execute(&app.pool)
    .await
    .expect("profile insert");

    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO test_plans (
            id, project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, revision
         ) VALUES ($1, $2, $3, 1, 'approved', '1', 'Inert load plan', 0.50, '{}', 0)",
    )
    .bind(plan_id)
    .bind(project_id)
    .bind(job_id)
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
    .bind(scenario_name)
    .bind(json!({
        "name": scenario_name,
        "requests": [{"id": "baseline", "method": "GET", "path": "/", "expectedStatuses": [200]}],
        "profile": {"virtualUsers": virtual_users}
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
    .expect("load approval insert");

    let authorization_id = seed_target_authorization(app, project_id, owner_id).await;
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, load_scenario_id, load_profile_id, target_authorization_id,
            idempotency_key, started_at, finished_at
         ) VALUES ($1, $2, $3, 1, $4, 'load_testing', 'completed', $5, $6, $7,
                   $8, now() - interval '10 minutes', now() - interval '9 minutes')
         RETURNING id",
    )
    .bind(project_id)
    .bind(job_id)
    .bind(plan_id)
    .bind(approval_id)
    .bind(scenario_id)
    .bind(profile_id)
    .bind(authorization_id)
    .bind(format!("load-{}", Uuid::new_v4()))
    .fetch_one(&app.pool)
    .await
    .expect("load run insert");

    for (endpoint, status, requests, errors, p95, throughput) in [
        ("baseline", 200, 80, 0, 120.0, 4.0),
        ("baseline", 500, 20, 20, 400.0, 1.0),
    ] {
        sqlx::query(
            "INSERT INTO run_metrics (
                project_id, run_id, bucket_start, bucket_seconds, endpoint, status_code,
                request_count, error_count, latency_p50_ms, latency_p95_ms,
                latency_p99_ms, throughput_rps, response_bytes
             ) VALUES ($1, $2, now() - interval '10 minutes', 60, $3, $4, $5, $6,
                       90, $7, 500, $8, 1024)",
        )
        .bind(project_id)
        .bind(run_id)
        .bind(endpoint)
        .bind(status)
        .bind(requests)
        .bind(errors)
        .bind(p95)
        .bind(throughput)
        .execute(&app.pool)
        .await
        .expect("metric insert");
    }
    run_id
}

#[tokio::test]
async fn run_summary_results_and_tenant_isolation() {
    let Some(world) = setup().await else {
        return;
    };
    let security = seed_security_run(&world.app, world.project_id, member_id(&world.owner)).await;

    let detail = world
        .app
        .send(
            Method::GET,
            &format!("/api/runs/{}", security.run_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{:?}", detail.body);
    assert_eq!(detail.body["run"]["name"], "Missing headers");
    assert_eq!(detail.body["run"]["resultCounts"]["failed"], 1);
    assert_eq!(detail.body["run"]["resultCounts"]["passed"], 1);
    assert_eq!(detail.body["run"]["resultCounts"]["confirmed"], 1);
    assert_eq!(detail.body["reportType"], "security");
    assert!(detail.body["target"]["origin"].as_str().is_some());
    let results = detail.body["results"].as_array().expect("results");
    assert_eq!(results.len(), 2);
    let failed = results
        .iter()
        .find(|row| row["outcome"] == "failed")
        .expect("failed result");
    assert_eq!(failed["findingStatus"], "confirmed");
    assert_eq!(failed["evidence"].as_array().expect("evidence").len(), 1);
    // The stored secret must never be returned through the API.
    assert!(!detail.body.to_string().contains("super-secret-token"));

    let list = world
        .app
        .send(
            Method::GET,
            &format!("/api/projects/{}/runs", world.project_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body.as_array().expect("runs").len(), 1);
    assert_eq!(list.body[0]["name"], "Missing headers");

    let outsider = world
        .app
        .send(
            Method::GET,
            &format!("/api/runs/{}", security.run_id),
            Some(&world.outsider),
            None,
        )
        .await;
    assert_eq!(outsider.status, StatusCode::NOT_FOUND);

    let viewer_report = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{}/report", security.run_id),
            Some(&world.viewer),
            None,
        )
        .await;
    assert_eq!(viewer_report.status, StatusCode::FORBIDDEN);

    world.app.cleanup().await;
}

#[tokio::test]
async fn report_generation_is_versioned_traceable_and_redacted() {
    let Some(world) = setup().await else {
        return;
    };
    let security = seed_security_run(&world.app, world.project_id, member_id(&world.owner)).await;

    let first = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{}/report", security.run_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
    assert_eq!(first.body["currentVersion"], 1);
    assert_eq!(first.body["status"], "final");
    let content = &first.body["latest"]["content"];
    assert!(!content.to_string().contains("super-secret-token"));
    assert!(content.to_string().contains("[redacted]"));
    let claims = content["claims"].as_array().expect("claims");
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

    let second = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{}/report", security.run_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED);
    assert_eq!(second.body["currentVersion"], 2);
    assert_eq!(
        second.body["versions"].as_array().expect("versions").len(),
        2
    );

    let fetched = world
        .app
        .send(
            Method::GET,
            &format!("/api/runs/{}/report", security.run_id),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body["currentVersion"], 2);

    let export = world
        .app
        .send_raw(
            Method::GET,
            &format!(
                "/api/runs/{}/report/export?format=markdown",
                security.run_id
            ),
            Some(&world.owner),
        )
        .await;
    assert_eq!(export.status, StatusCode::OK);
    assert_eq!(
        export
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/markdown; charset=utf-8")
    );
    let markdown = String::from_utf8(export.body).expect("markdown utf8");
    assert!(markdown.contains("ProbeForge report"));
    assert!(!markdown.contains("super-secret-token"));

    world.app.cleanup().await;
}

#[tokio::test]
async fn report_generation_failure_preserves_results() {
    let Some(world) = setup().await else {
        return;
    };
    let job_id = seed_job(
        &world.app,
        world.project_id,
        member_id(&world.owner),
        "vulnerability_testing",
    )
    .await;
    let planned_test_id = Uuid::new_v4();
    let (plan_id, approval_id) = seed_approved_plan(
        &world.app,
        world.project_id,
        job_id,
        member_id(&world.owner),
        planned_test_id,
    )
    .await;
    let authorization_id =
        seed_target_authorization(&world.app, world.project_id, member_id(&world.owner)).await;
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, plan_approval_id, run_type,
            status, planned_test_id, target_authorization_id, idempotency_key,
            started_at, finished_at
         ) VALUES ($1, $2, $3, 1, $4, 'vulnerability_testing', 'completed', $5, $6,
                   $7, now() - interval '5 minutes', now())
         RETURNING id",
    )
    .bind(world.project_id)
    .bind(job_id)
    .bind(plan_id)
    .bind(approval_id)
    .bind(planned_test_id)
    .bind(authorization_id)
    .bind(format!("oversize-{}", Uuid::new_v4()))
    .fetch_one(&world.app.pool)
    .await
    .expect("run insert");

    let big_summary = "a".repeat(8000);
    for _ in 0..40 {
        seed_result(
            &world.app,
            world.project_id,
            run_id,
            "http.cors.policy",
            "failed",
            Some("confirmed"),
            Some("low"),
            &big_summary,
            json!({}),
        )
        .await;
    }

    let generate = world
        .app
        .send(
            Method::POST,
            &format!("/api/runs/{run_id}/report"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(
        generate.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:?}",
        generate.body
    );
    assert_eq!(generate.body["error"]["code"], "report_content_too_large");

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM test_results WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(&world.app.pool)
        .await
        .expect("result count");
    assert_eq!(stored, 40, "report failure must not destroy stored results");

    let report = world
        .app
        .send(
            Method::GET,
            &format!("/api/runs/{run_id}/report"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(report.status, StatusCode::OK);
    assert_eq!(report.body["status"], "failed");
    assert_eq!(report.body["lastErrorCode"], "report_content_too_large");

    world.app.cleanup().await;
}

#[tokio::test]
async fn run_comparison_shows_profile_target_scenario_and_timestamps() {
    let Some(world) = setup().await else {
        return;
    };
    let left = seed_load_run(
        &world.app,
        world.project_id,
        member_id(&world.owner),
        2,
        "Baseline read",
    )
    .await;
    let right = seed_load_run(
        &world.app,
        world.project_id,
        member_id(&world.owner),
        8,
        "Peak read",
    )
    .await;

    let comparison = world
        .app
        .send(
            Method::GET,
            &format!("/api/runs/{left}/compare?otherRunId={right}"),
            Some(&world.owner),
            None,
        )
        .await;
    assert_eq!(comparison.status, StatusCode::OK, "{:?}", comparison.body);
    assert_eq!(comparison.body["left"]["scenarioName"], "Baseline read");
    assert_eq!(comparison.body["right"]["scenarioName"], "Peak read");
    assert_eq!(comparison.body["left"]["name"], "Baseline read");
    assert_eq!(comparison.body["right"]["name"], "Peak read");
    assert_eq!(comparison.body["left"]["profile"]["virtualUsers"], 2);
    assert_eq!(comparison.body["right"]["profile"]["virtualUsers"], 8);
    assert!(comparison.body["left"]["targetOrigin"].as_str().is_some());
    assert!(comparison.body["right"]["targetOrigin"].as_str().is_some());
    assert_eq!(comparison.body["left"]["planVersion"], 1);
    assert!(comparison.body["left"]["startedAt"].as_str().is_some());
    assert!(comparison.body["delta"]["requestCount"].is_number());

    world.app.cleanup().await;
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
