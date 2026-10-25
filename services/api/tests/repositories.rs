//! Repository registry API tests: listing, registration, archive-based removal,
//! RBAC, tenant isolation, and audit. Every test uses an isolated schema and inert
//! repository metadata; no repository is ever contacted.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct Fixture {
    app: TestApp,
    owner: Session,
    admin: Session,
    collaborator: Session,
    viewer: Session,
    outsider: Session,
    project_id: String,
}

async fn fixture() -> Option<Fixture> {
    let app = TestApp::setup().await?;
    for (email, name) in [
        ("owner@example.test", "Owner"),
        ("admin@example.test", "Admin"),
        ("collaborator@example.test", "Collaborator"),
        ("viewer@example.test", "Viewer"),
        ("outsider@example.test", "Outsider"),
    ] {
        app.create_user(email, name, PASSWORD).await;
    }

    let owner = app.login("owner@example.test", PASSWORD).await;
    let admin = app.login("admin@example.test", PASSWORD).await;
    let collaborator = app.login("collaborator@example.test", PASSWORD).await;
    let viewer = app.login("viewer@example.test", PASSWORD).await;
    let outsider = app.login("outsider@example.test", PASSWORD).await;

    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&owner),
            Some(json!({ "name": "Repository Project", "slug": "repository-project" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let project_id = response.body["id"].as_str().expect("project id").to_owned();

    for (session, role) in [
        (&admin, "admin"),
        (&collaborator, "collaborator"),
        (&viewer, "viewer"),
    ] {
        let response = app
            .send(
                Method::POST,
                &format!("/api/projects/{project_id}/members"),
                Some(&owner),
                Some(json!({ "email": session.user["email"], "role": role })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    }

    Some(Fixture {
        app,
        owner,
        admin,
        collaborator,
        viewer,
        outsider,
        project_id,
    })
}

fn repository_body(url: &str, reference: &str) -> Value {
    json!({ "repositoryUrl": url, "reference": reference })
}

async fn register(
    app: &TestApp,
    session: &Session,
    project_id: &str,
    url: &str,
    reference: &str,
) -> support::TestResponse {
    app.send(
        Method::POST,
        &format!("/api/projects/{project_id}/repositories"),
        Some(session),
        Some(repository_body(url, reference)),
    )
    .await
}

#[tokio::test]
async fn registers_lists_and_archives_repositories() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;
    let project_id = &fixture.project_id;

    // Empty initially.
    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body.as_array().expect("list").len(), 0);

    // Register two repositories; the second uses an explicit display name.
    let response = register(
        app,
        &fixture.owner,
        project_id,
        "https://github.com/acme/inert.git",
        "main",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(response.body["provider"], "github");
    assert_eq!(
        response.body["repositoryUrl"],
        "https://github.com/acme/inert.git"
    );
    assert_eq!(response.body["defaultReference"], "main");
    assert_eq!(response.body["jobCount"], 0);
    let first_id = response.body["id"].as_str().expect("id").to_owned();

    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.owner),
            Some(json!({
                "repositoryUrl": "https://gitlab.com/acme/second.git",
                "reference": "release/1.x",
                "displayName": "Second service",
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(response.body["displayName"], "Second service");
    assert_eq!(response.body["provider"], "gitlab");

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body.as_array().expect("list").len(), 2);

    // Removal archives the row and hides it from the active list.
    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{first_id}"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{:?}",
        response.body
    );

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.body.as_array().expect("list").len(), 1);

    // The row is archived, not deleted, so history stays referenced.
    let archived_at: Option<String> =
        sqlx::query_scalar("SELECT archived_at::text FROM git_sources WHERE id = $1")
            .bind(Uuid::parse_str(&first_id).expect("uuid"))
            .fetch_one(&app.pool)
            .await
            .expect("archived_at");
    assert!(archived_at.is_some());

    // Removing again is idempotent.
    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{first_id}"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);

    // Re-registering the same URL reactivates it.
    let response = register(
        app,
        &fixture.owner,
        project_id,
        "https://github.com/acme/inert.git",
        "main",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.body.as_array().expect("list").len(), 2);

    // Audit records the lifecycle.
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'repository.registered' AND project_id = $1",
    )
    .bind(Uuid::parse_str(project_id).expect("uuid"))
    .fetch_one(&app.pool)
    .await
    .expect("audit count");
    assert_eq!(audit, 3);
    let removed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'repository.removed' AND project_id = $1",
    )
    .bind(Uuid::parse_str(project_id).expect("uuid"))
    .fetch_one(&app.pool)
    .await
    .expect("audit count");
    assert_eq!(removed, 1);

    fixture.app.cleanup().await;
}

#[tokio::test]
async fn repository_access_is_role_gated_and_tenant_isolated() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;
    let project_id = &fixture.project_id;

    // Viewers cannot register.
    let response = register(
        app,
        &fixture.viewer,
        project_id,
        "https://github.com/acme/viewer.git",
        "main",
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::FORBIDDEN,
        "{:?}",
        response.body
    );

    // Non-members get a 404 so project existence is not disclosed.
    let response = register(
        app,
        &fixture.outsider,
        project_id,
        "https://github.com/acme/outsider.git",
        "main",
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // Collaborators may register but not remove.
    let response = register(
        app,
        &fixture.collaborator,
        project_id,
        "https://github.com/acme/collaborator.git",
        "main",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let repository_id = response.body["id"].as_str().expect("id").to_owned();

    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{repository_id}"),
            Some(&fixture.collaborator),
            None,
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
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{repository_id}"),
            Some(&fixture.outsider),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // Admins may remove.
    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{repository_id}"),
            Some(&fixture.admin),
            None,
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{:?}",
        response.body
    );

    // A viewer cannot list a project they do not belong to.
    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.outsider),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    fixture.app.cleanup().await;
}

#[tokio::test]
async fn repository_removal_is_blocked_while_analysis_is_active() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;
    let project_id = &fixture.project_id;

    let response = register(
        app,
        &fixture.owner,
        project_id,
        "https://github.com/acme/busy.git",
        "main",
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let repository_id = response.body["id"].as_str().expect("id").to_owned();

    // Creating an analysis for the same URL reuses the registered repository.
    let response = app
        .send(
            Method::POST,
            &format!("/api/jobs?projectId={project_id}"),
            Some(&fixture.owner),
            Some(json!({
                "schemaVersion": "1",
                "goal": "vulnerability_testing",
                "intensity": "standard",
                "displayName": "Busy analysis",
                "git": {
                    "repositoryUrl": "https://github.com/acme/busy.git",
                    "reference": "main"
                }
            })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    let job_id = response.body["id"].as_str().expect("job id").to_owned();

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.body[0]["jobCount"], 1);

    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{repository_id}"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "repository_busy");

    // Cancelling the job clears the block.
    let response = app
        .send(
            Method::POST,
            &format!("/api/jobs/{job_id}/cancel"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);

    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{project_id}/repositories/{repository_id}"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::NO_CONTENT,
        "{:?}",
        response.body
    );

    fixture.app.cleanup().await;
}

#[tokio::test]
async fn repository_input_is_validated() {
    let Some(fixture) = fixture().await else {
        return;
    };
    let app = &fixture.app;
    let project_id = &fixture.project_id;

    let invalid_urls = [
        "http://192.168.0.10/repo",
        "https://user:pass@github.com/acme/inert.git",
        "https://user@github.com/acme/inert.git",
        "ssh://root@gitlab.com/acme/inert.git",
        "https://github.com/acme/inert.git?token=1",
    ];
    for url in invalid_urls {
        let response = register(app, &fixture.owner, project_id, url, "main").await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "url {url}: {:?}",
            response.body
        );
    }

    for reference in ["", "-flag", "feature/../etc", "has space"] {
        let response = register(
            app,
            &fixture.owner,
            project_id,
            "https://github.com/acme/inert.git",
            reference,
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "reference {reference}: {:?}",
            response.body
        );
    }

    fixture.app.cleanup().await;
}
