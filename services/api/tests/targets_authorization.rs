//! Phase 9 target registration, validation, authorization, and tenant tests.
//! No HTTP request is sent to a target; destination validation only resolves
//! literal/private fixture addresses or public fixture literals.

mod support;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use support::{Session, TestApp};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";
const TARGET_PORT: u16 = 43123;

struct Fixture {
    app: TestApp,
    owner: Session,
    viewer: Session,
    outsider: Session,
    project_id: String,
}

fn target_payload(environment: &str) -> Value {
    json!({
        "origin": format!("http://127.0.0.1:{TARGET_PORT}"),
        "environment": environment,
        "allowedHosts": ["127.0.0.1"],
        "allowedPaths": ["/api"],
        "allowedPorts": [TARGET_PORT]
    })
}

fn authorization_payload() -> Value {
    json!({
        "authorizationMethod": "explicit_confirmation",
        "confirmed": true,
        "limits": {
            "maxRequestsPerSecond": 10.0,
            "maxConcurrency": 2,
            "maxDurationSeconds": 60,
            "maxRequests": 100
        }
    })
}

async fn create_target(
    app: &TestApp,
    session: &Session,
    project_id: &str,
    environment: &str,
) -> support::TestResponse {
    app.send(
        Method::POST,
        &format!("/api/projects/{project_id}/targets"),
        Some(session),
        Some(target_payload(environment)),
    )
    .await
}

async fn fixture(private_targets: bool) -> Option<Fixture> {
    let app = if private_targets {
        TestApp::setup_with_private_targets().await?
    } else {
        TestApp::setup().await?
    };
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
            Some(json!({"name": "Target Project", "slug": "target-project"})),
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

    Some(Fixture {
        app,
        owner,
        viewer,
        outsider,
        project_id,
    })
}

#[tokio::test]
async fn target_registration_normalizes_and_validation_records_addresses() {
    let Some(fixture) = fixture(true).await else {
        return;
    };
    let app = &fixture.app;

    let response = create_target(app, &fixture.owner, &fixture.project_id, "local").await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(
        response.body["normalizedOrigin"],
        format!("http://127.0.0.1:{TARGET_PORT}")
    );
    assert_eq!(response.body["environment"], "local");
    assert_eq!(response.body["verificationStatus"], "pending");
    assert_eq!(response.body["allowedPorts"], json!([TARGET_PORT]));
    assert_eq!(response.body["allowedPaths"], json!(["/api"]));
    let target_id = response.body["id"].as_str().expect("target id").to_owned();

    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/validate"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["verificationStatus"], "verified");
    assert_eq!(response.body["verifiedAddresses"], json!(["127.0.0.1"]));
    assert!(response.body["verificationCheckedAt"].is_string());

    let response = app
        .send(
            Method::GET,
            &format!("/api/targets/{target_id}"),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["target"]["id"], target_id);
    assert_eq!(
        response.body["authorizations"]
            .as_array()
            .expect("authorizations")
            .len(),
        0
    );

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{}/targets", fixture.project_id),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body.as_array().expect("targets").len(), 1);

    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/validate"),
            Some(&fixture.viewer),
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
            Method::GET,
            &format!("/api/targets/{target_id}"),
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

    let response = create_target(app, &fixture.owner, &fixture.project_id, "local").await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "target_exists");
}

#[tokio::test]
async fn target_registration_rejects_unsafe_shapes_and_ports() {
    let Some(fixture) = fixture(true).await else {
        return;
    };
    let app = &fixture.app;

    for payload in [
        json!({
            "origin": "ftp://127.0.0.1:43123",
            "environment": "local",
            "allowedPaths": ["/"]
        }),
        json!({
            "origin": "http://user:pass@127.0.0.1:43123",
            "environment": "local",
            "allowedPaths": ["/"]
        }),
        json!({
            "origin": "http://127.0.0.1:43123/api",
            "environment": "local",
            "allowedPaths": ["/"]
        }),
        json!({
            "origin": "http://127.0.0.1:43123",
            "environment": "local",
            "allowedPaths": ["/../admin"]
        }),
        json!({
            "origin": "http://127.0.0.1:22",
            "environment": "local",
            "allowedPaths": ["/"]
        }),
        json!({
            "origin": "http://127.0.0.1:43123",
            "environment": "production",
            "allowedPaths": ["/"]
        }),
    ] {
        let response = app
            .send(
                Method::POST,
                &format!("/api/projects/{}/targets", fixture.project_id),
                Some(&fixture.owner),
                Some(payload),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{:?}",
            response.body
        );
    }

    let response = create_target(app, &fixture.viewer, &fixture.project_id, "local").await;
    assert_eq!(
        response.status,
        StatusCode::FORBIDDEN,
        "{:?}",
        response.body
    );
    let response = create_target(app, &fixture.outsider, &fixture.project_id, "local").await;
    assert_eq!(
        response.status,
        StatusCode::NOT_FOUND,
        "{:?}",
        response.body
    );
}

#[tokio::test]
async fn default_policy_blocks_private_targets() {
    let Some(fixture) = fixture(false).await else {
        return;
    };
    let response = create_target(&fixture.app, &fixture.owner, &fixture.project_id, "local").await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "private_network_disabled");
}

#[tokio::test]
async fn authorization_requires_verification_confirmation_and_bounded_limits() {
    let Some(fixture) = fixture(true).await else {
        return;
    };
    let app = &fixture.app;
    let response = create_target(app, &fixture.owner, &fixture.project_id, "local").await;
    let target_id = response.body["id"].as_str().expect("target id").to_owned();

    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(authorization_payload()),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "target_not_verified");

    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/validate"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);

    let mut unconfirmed = authorization_payload();
    unconfirmed["confirmed"] = json!(false);
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(unconfirmed),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "confirmation_required");

    let mut ownership = authorization_payload();
    ownership["authorizationMethod"] = json!("ownership_verification");
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(ownership),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(
        response.body["error"]["code"],
        "unsupported_authorization_method"
    );

    let mut invalid_limits = authorization_payload();
    invalid_limits["limits"]["maxRequestsPerSecond"] = json!(0);
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(invalid_limits),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "invalid_target_limits");

    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.viewer),
            Some(authorization_payload()),
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
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(authorization_payload()),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(response.body["active"], true);
    assert_eq!(response.body["scope"]["allowedPorts"], json!([TARGET_PORT]));
    assert_eq!(response.body["limits"]["maxRequests"], 100);
    let authorization_id = response.body["id"]
        .as_str()
        .expect("authorization id")
        .to_owned();
    assert!(response.body["expiresAt"].is_string());

    let response = app
        .send(
            Method::GET,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body.as_array().expect("authorizations").len(), 1);
    assert_eq!(response.body[0]["active"], true);

    let response = app
        .send(
            Method::POST,
            &format!("/api/target-authorizations/{authorization_id}/revoke"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["active"], false);
    assert!(response.body["revokedAt"].is_string());

    let response = app
        .send(
            Method::POST,
            &format!("/api/target-authorizations/{authorization_id}/revoke"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(
        response.body["error"]["code"],
        "authorization_already_revoked"
    );

    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_log WHERE project_id = $1 ORDER BY created_at, id",
    )
    .bind(Uuid::parse_str(&fixture.project_id).expect("project uuid"))
    .fetch_all(&app.pool)
    .await
    .expect("audit actions");
    for expected in [
        "target.created",
        "target.validated",
        "target.authorization_created",
        "target.authorization_revoked",
    ] {
        assert!(
            actions.iter().any(|action| action == expected),
            "missing {expected}"
        );
    }
}

#[tokio::test]
async fn authorization_expiry_and_role_boundaries_are_enforced() {
    let Some(fixture) = fixture(true).await else {
        return;
    };
    let app = &fixture.app;
    let response = create_target(app, &fixture.owner, &fixture.project_id, "local").await;
    let target_id = response.body["id"].as_str().expect("target id").to_owned();
    app.send(
        Method::POST,
        &format!("/api/targets/{target_id}/validate"),
        Some(&fixture.owner),
        None,
    )
    .await;

    let mut expired = authorization_payload();
    expired["expiresAt"] = json!((Utc::now() - Duration::minutes(1)).to_rfc3339());
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(expired),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "authorization_inactive");

    let mut too_long = authorization_payload();
    too_long["expiresAt"] = json!((Utc::now() + Duration::days(30)).to_rfc3339());
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(too_long),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "authorization_inactive");

    let mut short_lived = authorization_payload();
    short_lived["expiresAt"] = json!((Utc::now() + Duration::seconds(2)).to_rfc3339());
    let response = app
        .send(
            Method::POST,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            Some(short_lived),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    tokio::time::sleep(std::time::Duration::from_millis(2_100)).await;
    let response = app
        .send(
            Method::GET,
            &format!("/api/targets/{target_id}/authorizations"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body[0]["active"], false);

    let response = app
        .send(
            Method::GET,
            &format!("/api/targets/{target_id}/authorizations"),
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
}
