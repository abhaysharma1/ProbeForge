//! Phase 8 plan review, optimistic selection, and immutable approval tests.
//! The fixture seeds only inert plan rows; no target, scanner, or run is used.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct Fixture {
    app: TestApp,
    owner: Session,
    viewer: Session,
    outsider: Session,
    project_id: String,
    vulnerability_job_id: String,
    vulnerability_plan_id: String,
    test_ids: Vec<String>,
    load_plan_id: String,
    scenario_id: String,
}

fn vulnerability_plan() -> Value {
    json!({
        "schemaVersion": "1",
        "goal": "vulnerability_testing",
        "summary": "Review the discovered HTTP route and inspect credential handling.",
        "confidence": 0.72,
        "plannedTests": [
            {
                "catalogTestId": "http.headers.missing",
                "name": "Missing security headers",
                "rationale": "The repository exposes an HTTP route that can be checked for baseline response hardening.",
                "risk": "low",
                "prerequisites": ["Authorized target"],
                "expectedEvidence": "Response headers and observation timestamp.",
                "targetRequirements": ["authorized target origin"],
                "requiresExplicitConfirmation": false,
                "configuration": {"route": "/api/items"},
                "rationaleReferences": ["src/server.ts"]
            },
            {
                "catalogTestId": "static.secrets.exposure",
                "name": "Potential secret exposure",
                "rationale": "The pinned snapshot can be checked with a non-printing secret-pattern ruleset.",
                "risk": "high",
                "prerequisites": ["Pinned repository snapshot"],
                "expectedEvidence": "Redacted file path and pattern class.",
                "targetRequirements": ["read-only repository snapshot"],
                "requiresExplicitConfirmation": true,
                "configuration": {},
                "rationaleReferences": ["src/config.ts"]
            }
        ],
        "loadScenarios": [],
        "requiredInputs": ["Authorized target"],
        "limitations": ["No target was contacted while generating this plan."],
        "evidenceReferences": ["src/server.ts"]
    })
}

fn load_plan() -> Value {
    json!({
        "schemaVersion": "1",
        "goal": "load_testing",
        "summary": "A conservative baseline load scenario for a literal discovered route.",
        "confidence": 0.61,
        "plannedTests": [],
        "loadScenarios": [
            {
                "name": "Baseline API read",
                "description": "Conservative baseline profile for a discovered read-only route.",
                "requests": [
                    {"method": "GET", "path": "/api/items", "expectedStatuses": [200]}
                ],
                "profile": {
                    "virtualUsers": 2,
                    "rampUpSeconds": 5,
                    "durationSeconds": 30,
                    "maxRequests": 200,
                    "maxRequestsPerSecond": 2
                }
            }
        ],
        "requiredInputs": ["Authorized target"],
        "limitations": ["No target was contacted while generating this plan."]
    })
}

fn job_payload(display_name: &str, goal: &str) -> Value {
    json!({
        "schemaVersion": "1",
        "goal": goal,
        "intensity": "standard",
        "displayName": display_name,
        "git": {
            "repositoryUrl": "https://github.com/example/inert-fixture.git",
            "reference": "main"
        }
    })
}

async fn create_job(app: &TestApp, owner: &Session, project_id: &str, goal: &str) -> String {
    let response = app
        .send(
            Method::POST,
            &format!("/api/jobs?projectId={project_id}"),
            Some(owner),
            Some(job_payload(
                if goal == "load_testing" {
                    "Load fixture job"
                } else {
                    "Vulnerability fixture job"
                },
                goal,
            )),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_str().expect("job id").to_owned()
}

async fn seed_plan(
    pool: &PgPool,
    project_id: &str,
    job_id: &str,
    plan: &Value,
    tests: &[(&str, &str, &str, bool)],
) -> (String, Vec<String>) {
    let plan_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO test_plans (
            project_id, job_id, version, status, schema_version, summary,
            confidence, plan_payload, agent_provider, agent_model,
            created_by_agent, revision
        ) VALUES ($1, $2, 1, 'proposed', '1', $3, $4, $5, 'faux',
                  'planner-fixture', true, 0)
        RETURNING id
        "#,
    )
    .bind(Uuid::parse_str(project_id).expect("project uuid"))
    .bind(Uuid::parse_str(job_id).expect("job uuid"))
    .bind(plan["summary"].as_str().expect("summary"))
    .bind(plan["confidence"].as_f64().expect("confidence"))
    .bind(plan)
    .fetch_one(pool)
    .await
    .expect("plan insert");

    sqlx::query("UPDATE jobs SET current_plan_id = $1 WHERE id = $2")
        .bind(plan_id)
        .bind(Uuid::parse_str(job_id).expect("job uuid"))
        .execute(pool)
        .await
        .expect("job current plan update");

    let mut test_ids = Vec::new();
    for (catalog_test_id, name, risk, requires_confirmation) in tests {
        let test_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO planned_tests (
                project_id, plan_id, catalog_test_id, name, rationale, risk,
                prerequisites, expected_evidence, target_requirements,
                requires_explicit_confirmation, configuration, rationale_references, selected
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, false)
            RETURNING id
            "#,
        )
        .bind(Uuid::parse_str(project_id).expect("project uuid"))
        .bind(plan_id)
        .bind(catalog_test_id)
        .bind(name)
        .bind("Inert fixture rationale for a catalog-backed test.")
        .bind(risk)
        .bind(json!(["Authorized target"]))
        .bind("Bounded observation evidence.")
        .bind(json!(["authorized target origin"]))
        .bind(requires_confirmation)
        .bind(json!({"route": "/api/items"}))
        .bind(json!(["src/server.ts"]))
        .fetch_one(pool)
        .await
        .expect("planned test insert");
        test_ids.push(test_id.to_string());
    }
    (plan_id.to_string(), test_ids)
}

async fn seed_load_plan(
    pool: &PgPool,
    project_id: &str,
    job_id: &str,
    plan: &Value,
) -> (String, String) {
    let (plan_id, _) = seed_plan(pool, project_id, job_id, plan, &[]).await;
    let profile_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO load_profiles (
            project_id, name, virtual_users, ramp_up_seconds, duration_seconds,
            max_requests, max_requests_per_second
        ) VALUES ($1, 'Baseline API read', 2, 5, 30, 200, 2)
        RETURNING id
        "#,
    )
    .bind(Uuid::parse_str(project_id).expect("project uuid"))
    .fetch_one(pool)
    .await
    .expect("load profile insert");
    let scenario_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO load_scenarios (
            project_id, plan_id, profile_id, name, schema_version, scenario, selected
        ) VALUES ($1, $2, $3, 'Baseline API read', '1', $4, false)
        RETURNING id
        "#,
    )
    .bind(Uuid::parse_str(project_id).expect("project uuid"))
    .bind(Uuid::parse_str(&plan_id).expect("plan uuid"))
    .bind(profile_id)
    .bind(&plan["loadScenarios"][0])
    .fetch_one(pool)
    .await
    .expect("load scenario insert");
    (plan_id, scenario_id.to_string())
}

async fn fixture() -> Option<Fixture> {
    let app = TestApp::setup().await?;
    app.create_user("owner@example.test", "Owner", PASSWORD)
        .await;
    app.create_user("viewer@example.test", "Viewer", PASSWORD)
        .await;
    app.create_user("outsider@example.test", "Outsider", PASSWORD)
        .await;
    let owner = app.login("owner@example.test", PASSWORD).await;
    let viewer = app.login("viewer@example.test", PASSWORD).await;
    let outsider = app.login("outsider@example.test", PASSWORD).await;

    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&owner),
            Some(json!({"name": "Plan Project", "slug": "plan-project"})),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let project_id = response.body["id"].as_str().expect("project id").to_owned();

    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{project_id}/members"),
            Some(&owner),
            Some(json!({"email": "viewer@example.test", "role": "viewer"})),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);

    let vulnerability_job_id = create_job(&app, &owner, &project_id, "vulnerability_testing").await;
    let load_job_id = create_job(&app, &owner, &project_id, "load_testing").await;

    let (vulnerability_plan_id, test_ids) = seed_plan(
        &app.pool,
        &project_id,
        &vulnerability_job_id,
        &vulnerability_plan(),
        &[
            (
                "http.headers.missing",
                "Missing security headers",
                "low",
                false,
            ),
            (
                "static.secrets.exposure",
                "Potential secret exposure",
                "high",
                true,
            ),
        ],
    )
    .await;
    let (load_plan_id, scenario_id) =
        seed_load_plan(&app.pool, &project_id, &load_job_id, &load_plan()).await;

    Some(Fixture {
        app,
        owner,
        viewer,
        outsider,
        project_id,
        vulnerability_job_id,
        vulnerability_plan_id,
        test_ids,
        load_plan_id,
        scenario_id,
    })
}

#[tokio::test]
async fn plan_review_exposes_catalog_metadata_and_tenant_boundaries() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;

    let response = app
        .send(
            Method::GET,
            &format!("/api/jobs/{}/plan", fixture.vulnerability_job_id),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["status"], "proposed");
    assert_eq!(response.body["revision"], 0);
    assert_eq!(response.body["goal"], "vulnerability_testing");
    assert_eq!(response.body["catalogVersion"], "0.2.0");
    let tests = response.body["plannedTests"].as_array().expect("tests");
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["catalogTestId"], "http.headers.missing");
    assert_eq!(tests[0]["catalog"]["available"], true);
    assert!(tests[0]["catalog"]["availabilityNote"].is_string());
    assert_eq!(tests[1]["catalog"]["available"], false);
    assert_eq!(tests[1]["requiresExplicitConfirmation"], true);
    assert_eq!(tests[0]["selected"], false);

    let response = app
        .send(
            Method::GET,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);

    let response = app
        .send(
            Method::GET,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.outsider),
            None,
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{:?}",
        response.body
    );

    let response = app
        .send(
            Method::GET,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            None,
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let response = app
        .send(Method::GET, "/api/test-catalog", Some(&fixture.owner), None)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["catalogVersion"], "0.2.0");
    let entries = response.body["entries"]
        .as_array()
        .expect("catalog entries");
    let headers = entries
        .iter()
        .find(|entry| entry["id"] == "http.headers.missing")
        .expect("headers catalog entry");
    assert_eq!(headers["available"], true);
    let cors = entries
        .iter()
        .find(|entry| entry["id"] == "http.cors.misconfiguration")
        .expect("cors catalog entry");
    assert_eq!(cors["available"], true);
    let secrets = entries
        .iter()
        .find(|entry| entry["id"] == "static.secrets.exposure")
        .expect("secrets catalog entry");
    assert_eq!(secrets["available"], false);
    let load = entries
        .iter()
        .find(|entry| entry["id"] == "load.baseline")
        .expect("load catalog entry");
    assert_eq!(load["available"], false);
}

#[tokio::test]
async fn selection_updates_are_role_checked_and_optimistic() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;
    let first = &fixture.test_ids[0];
    let second = &fixture.test_ids[1];

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.viewer),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": [first]
            })),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::FORBIDDEN,
        "{:?}",
        response.body
    );

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": [first],
                "unexpected": true
            })),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": [first]
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["revision"], 1);
    assert_eq!(
        response.body["plannedTests"]
            .as_array()
            .expect("tests")
            .iter()
            .filter(|test| test["selected"] == true)
            .count(),
        1
    );

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": [second]
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "plan_conflict");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 1,
                "selectedTestIds": [Uuid::new_v4().to_string()]
            })),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "unknown_plan_test");

    let patch_uri = format!("/api/plans/{}", fixture.vulnerability_plan_id);
    let (left, right) = tokio::join!(
        app.send(
            Method::PATCH,
            &patch_uri,
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "selectedTestIds": [first]})),
        ),
        app.send(
            Method::PATCH,
            &patch_uri,
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "selectedTestIds": [second]})),
        )
    );
    let mut statuses = [left.status, right.status];
    statuses.sort_by_key(|status| status.as_u16());
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);

    let response = app
        .send(
            Method::GET,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["revision"], 2);
    assert_eq!(
        response.body["plannedTests"]
            .as_array()
            .expect("tests")
            .iter()
            .filter(|test| test["selected"] == true)
            .count(),
        1
    );
}

#[tokio::test]
async fn approval_requires_confirmation_and_writes_an_immutable_snapshot() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": fixture.test_ids
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "confirmed": false})),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "confirmation_required");

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 0, "confirmed": true})),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "plan_conflict");

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "confirmed": true})),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["status"], "approved");
    assert_eq!(response.body["approval"]["planRevision"], 1);
    assert_eq!(
        response.body["approval"]["approvedTestIds"]
            .as_array()
            .expect("approved tests")
            .len(),
        2
    );
    assert_eq!(response.body["approval"]["approvedByName"], "Owner");

    let approval_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM plan_approvals WHERE plan_id = $1")
            .bind(Uuid::parse_str(&fixture.vulnerability_plan_id).expect("plan uuid"))
            .fetch_one(&app.pool)
            .await
            .expect("approval count");
    assert_eq!(approval_count, 1);
    let snapshot: Value =
        sqlx::query_scalar("SELECT approval_snapshot FROM plan_approvals WHERE plan_id = $1")
            .bind(Uuid::parse_str(&fixture.vulnerability_plan_id).expect("plan uuid"))
            .fetch_one(&app.pool)
            .await
            .expect("approval snapshot");
    assert_eq!(snapshot["catalogVersion"], "0.2.0");
    assert_eq!(
        snapshot["tests"].as_array().expect("snapshot tests").len(),
        2
    );
    assert_eq!(snapshot["planRevision"], 1);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'plan.approved' AND resource_id = $1",
    )
    .bind(Uuid::parse_str(&fixture.vulnerability_plan_id).expect("plan uuid"))
    .fetch_one(&app.pool)
    .await
    .expect("audit count");
    assert_eq!(audit_count, 1);

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.vulnerability_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "confirmed": true})),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "plan_already_approved");

    let response = app
        .send(
            Method::GET,
            &format!("/api/plans/{}", fixture.vulnerability_plan_id),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["status"], "approved");
    assert_eq!(
        response.body["approval"]["approvedBy"],
        fixture.owner.user["id"]
    );
}

#[tokio::test]
async fn load_plan_selection_and_approval_use_load_scenario_ids() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.load_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 0, "confirmed": true})),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "plan_no_selection");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.load_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedTestIds": fixture.test_ids
            })),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "invalid_selection");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/plans/{}", fixture.load_plan_id),
            Some(&fixture.owner),
            Some(json!({
                "expectedRevision": 0,
                "selectedLoadScenarioIds": [fixture.scenario_id]
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["revision"], 1);
    assert_eq!(response.body["loadScenarios"][0]["selected"], true);

    let response = app
        .send(
            Method::POST,
            &format!("/api/plans/{}/approve", fixture.load_plan_id),
            Some(&fixture.owner),
            Some(json!({"expectedRevision": 1, "confirmed": true})),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["goal"], "load_testing");
    assert_eq!(
        response.body["approval"]["approvedLoadScenarioIds"]
            .as_array()
            .expect("approved scenarios")
            .len(),
        1
    );
    assert_eq!(
        response.body["approval"]["approvedTestIds"]
            .as_array()
            .expect("approved tests")
            .len(),
        0
    );
}

#[tokio::test]
async fn database_rejects_runs_without_an_approval() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'test_runs' AND column_name = 'plan_approval_id'",
    )
    .fetch_one(&fixture.app.pool)
    .await
    .expect("column lookup");
    assert_eq!(nullable, "NO");

    let result = sqlx::query(
        r#"
        INSERT INTO test_runs (
            project_id, job_id, plan_id, plan_version, run_type,
            target_authorization_id, idempotency_key
        ) VALUES ($1, $2, $3, 1, 'vulnerability_testing', $4, 'phase-8-no-approval')
        "#,
    )
    .bind(Uuid::parse_str(&fixture.project_id).expect("project uuid"))
    .bind(Uuid::parse_str(&fixture.vulnerability_job_id).expect("job uuid"))
    .bind(Uuid::parse_str(&fixture.vulnerability_plan_id).expect("plan uuid"))
    .bind(Uuid::new_v4())
    .execute(&fixture.app.pool)
    .await;
    assert!(result.is_err(), "a run without plan_approval_id must fail");
}
