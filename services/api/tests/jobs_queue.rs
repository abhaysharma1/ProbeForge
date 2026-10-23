//! Phase 4 job API, idempotency, cancellation, event ordering, and SSE replay
//! tests. Every test uses an isolated schema and inert repository metadata.

mod support;

use axum::http::{StatusCode, header};
use probe_forge_queue::{claim_job, complete_job, heartbeat_job};
use serde_json::{Value, json};
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct Fixture {
    app: TestApp,
    owner: Session,
    viewer: Session,
    outsider: Session,
    project_id: String,
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
            axum::http::Method::POST,
            "/api/projects",
            Some(&owner),
            Some(json!({ "name": "Queue Project", "slug": "queue-project" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let project_id = response.body["id"].as_str().expect("project id").to_owned();

    let response = app
        .send(
            axum::http::Method::POST,
            &format!("/api/projects/{project_id}/members"),
            Some(&owner),
            Some(json!({ "email": "viewer@example.test", "role": "viewer" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);

    Some(Fixture {
        app,
        owner,
        viewer,
        outsider,
        project_id,
    })
}

fn job_payload(display_name: &str) -> Value {
    json!({
        "schemaVersion": "1",
        "goal": "vulnerability_testing",
        "intensity": "standard",
        "displayName": display_name,
        "git": {
            "repositoryUrl": "https://github.com/example/inert-fixture.git",
            "reference": "main"
        }
    })
}

async fn create_job_with_key(
    app: &TestApp,
    session: &Session,
    project_id: &str,
    idempotency_key: &str,
    display_name: &str,
) -> support::TestResponse {
    app.send_with_idempotency_key(
        axum::http::Method::POST,
        &format!("/api/jobs?projectId={project_id}"),
        Some(session),
        Some(job_payload(display_name)),
        idempotency_key,
    )
    .await
}

#[tokio::test]
async fn creation_validates_contract_authorization_and_idempotency() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;

    let response = app
        .send(
            axum::http::Method::POST,
            &format!("/api/jobs?projectId={}", fixture.project_id),
            None,
            Some(job_payload("Unauthenticated attempt")),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let response = app
        .send(
            axum::http::Method::POST,
            &format!("/api/jobs?projectId={}", fixture.project_id),
            Some(&fixture.owner),
            Some(job_payload("Payments analysis")),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(response.body["status"], "queued");
    assert_eq!(response.body["goal"], "vulnerability_testing");
    assert_eq!(response.body["git"]["provider"], "github");
    assert_eq!(response.body["sourceReference"], "main");
    let job_id = response.body["id"].as_str().expect("job id").to_owned();

    let response = app
        .send(
            axum::http::Method::GET,
            &format!("/api/jobs/{job_id}"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["id"], job_id);

    let response = app
        .send(
            axum::http::Method::GET,
            &format!("/api/projects/{}/jobs", fixture.project_id),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body.as_array().expect("jobs").len(), 1);

    let response = app
        .send_raw_with_last_event_id(
            axum::http::Method::GET,
            &format!("/api/jobs/{job_id}/events?follow=false"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let event_body = String::from_utf8_lossy(&response.body);
    assert!(event_body.contains("\"sequence\":1"));
    assert!(event_body.contains("job.queued"));

    let event_count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_events WHERE job_id = $1")
        .bind(Uuid::parse_str(&job_id).expect("job uuid"))
        .fetch_one(&app.pool)
        .await
        .expect("event count");
    assert_eq!(event_count, 1);

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'job.created' AND resource_id = $1",
    )
    .bind(Uuid::parse_str(&job_id).expect("job uuid"))
    .fetch_one(&app.pool)
    .await
    .expect("audit count");
    assert_eq!(audit_count, 1);

    let response = create_job_with_key(
        app,
        &fixture.viewer,
        &fixture.project_id,
        "viewer-job",
        "Viewer attempt",
    )
    .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = create_job_with_key(
        app,
        &fixture.outsider,
        &fixture.project_id,
        "outsider-job",
        "Outsider attempt",
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    let invalid_cases = [
        json!({
            "schemaVersion": "1",
            "goal": "vulnerability_testing",
            "intensity": "standard",
            "displayName": "Embedded credentials",
            "git": {
                "repositoryUrl": "https://user:password@github.com/example/inert.git",
                "reference": "main"
            }
        }),
        json!({
            "schemaVersion": "1",
            "goal": "vulnerability_testing",
            "intensity": "standard",
            "displayName": "Unknown field",
            "git": {
                "repositoryUrl": "https://github.com/example/inert.git",
                "reference": "main"
            },
            "extra": true
        }),
        json!({
            "schemaVersion": "1",
            "goal": "vulnerability_testing",
            "intensity": "standard",
            "displayName": "Missing reference",
            "git": {
                "repositoryUrl": "https://github.com/example/inert.git"
            }
        }),
    ];
    for body in invalid_cases {
        let response = app
            .send(
                axum::http::Method::POST,
                &format!("/api/jobs?projectId={}", fixture.project_id),
                Some(&fixture.owner),
                Some(body),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{:?}",
            response.body
        );
    }

    let response = app
        .send(
            axum::http::Method::POST,
            "/api/jobs",
            Some(&fixture.owner),
            Some(job_payload("Missing project")),
        )
        .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);

    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "duplicate-key",
        "Duplicate key",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let duplicate_id = response.body["id"].as_str().expect("job id").to_owned();
    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "duplicate-key",
        "Duplicate key",
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["id"], duplicate_id);

    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "duplicate-key",
        "Duplicate key retry",
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert!(response.body["error"]["message"].is_string());

    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "different-key",
        "Different request",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&app.pool)
        .await
        .expect("job count");
    assert_eq!(count, 3);

    fixture.app.cleanup().await;
}

#[tokio::test]
async fn cancellation_clears_leases_and_sse_replays_after_last_event_id() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;

    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "cancelled-job",
        "Cancellation test",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let job_id = Uuid::parse_str(response.body["id"].as_str().expect("job id")).expect("uuid");

    let claimed = claim_job(&app.pool, "worker-cancel-test", 60)
        .await
        .expect("claim")
        .expect("queued job");
    assert_eq!(claimed.id, job_id);

    let response = app
        .send(
            axum::http::Method::POST,
            &format!("/api/jobs/{job_id}/cancel"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["status"], "cancelled");

    assert!(
        heartbeat_job(&app.pool, job_id, "worker-cancel-test", 60)
            .await
            .expect("heartbeat")
            .is_none()
    );
    assert!(
        !complete_job(&app.pool, job_id, "worker-cancel-test", None)
            .await
            .expect("complete")
    );
    assert!(
        claim_job(&app.pool, "worker-after-cancel", 60)
            .await
            .expect("claim after cancel")
            .is_none()
    );

    let response = create_job_with_key(
        app,
        &fixture.owner,
        &fixture.project_id,
        "sse-job",
        "SSE replay test",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let sse_job_id = Uuid::parse_str(response.body["id"].as_str().expect("job id")).expect("uuid");

    let mut transaction = app.pool.begin().await.expect("transaction");
    probe_forge_queue::append_job_event(
        &mut transaction,
        sse_job_id,
        "plan.proposed",
        json!({ "planId": null }),
    )
    .await
    .expect("append event");
    transaction.commit().await.expect("commit");

    let response = app
        .send_raw_with_last_event_id(
            axum::http::Method::GET,
            &format!("/api/jobs/{sse_job_id}/events?follow=false"),
            Some(&fixture.owner),
            Some("1"),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let body = String::from_utf8_lossy(&response.body);
    assert!(body.contains("id: 2"), "replay body: {body}");
    assert!(body.contains("event: plan.proposed"));
    assert!(!body.contains("id: 1\n"), "replayed an already-seen event");

    let response = app
        .send_raw_with_last_event_id(
            axum::http::Method::GET,
            &format!("/api/jobs/{sse_job_id}/events?follow=false"),
            Some(&fixture.owner),
            Some("1"),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let body = String::from_utf8_lossy(&response.body);
    assert!(body.contains("\"sequence\":2"));
    assert!(body.contains("plan.proposed"));

    let response = app
        .send_raw_with_last_event_id(
            axum::http::Method::GET,
            &format!("/api/jobs/{sse_job_id}/events?follow=false"),
            Some(&fixture.outsider),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    let response = app
        .send_raw_with_last_event_id(
            axum::http::Method::GET,
            &format!("/api/jobs/{sse_job_id}/events?follow=false"),
            None,
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    fixture.app.cleanup().await;
}
