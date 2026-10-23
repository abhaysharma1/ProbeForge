//! Phase 3 authentication, session, RBAC, CSRF, rate-limit, audit, and
//! database-readiness integration tests.
//!
//! Each test creates an isolated `pf_test_*` schema and never touches `public`.
//! `bun run check:db` sets `PROBE_FORGE_REQUIRE_DB_TESTS=1`, so a missing
//! database fails instead of silently skipping the gate.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use support::{Session, TestApp};

const PASSWORD: &str = "correct horse battery staple";

fn project_id(response: &support::TestResponse) -> String {
    response.body["id"]
        .as_str()
        .expect("project id in response")
        .to_owned()
}

fn csrf_only(session: &Session) -> Session {
    Session {
        cookie_header: format!("pf_csrf={}", session.csrf),
        csrf: session.csrf.clone(),
        user: Value::Null,
    }
}

#[tokio::test]
async fn authentication_logout_and_session_revocation_work() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let email = "owner@example.test";
    let user_id = app.create_user(email, "Owner", PASSWORD).await;
    let session = app.login(email, PASSWORD).await;

    assert_eq!(session.user["email"], email);
    let set_cookie = app
        .send(Method::GET, "/api/auth/session", Some(&session), None)
        .await
        .headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !set_cookie.contains("pf_session"),
        "session endpoint must not rotate the session cookie"
    );

    let response = app
        .send(Method::GET, "/api/auth/session", Some(&session), None)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response
            .headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(response.body["user"]["id"], user_id.to_string());
    assert_eq!(response.body["csrfToken"], session.csrf);

    let response = app
        .send(Method::GET, "/api/auth/sessions", Some(&session), None)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let sessions = response.body.as_array().expect("session array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["current"], true);

    let mut wrong_csrf = session.clone();
    wrong_csrf.csrf = "not-the-token".to_owned();
    let response = app
        .send(Method::POST, "/api/auth/logout", Some(&wrong_csrf), None)
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(Method::POST, "/api/auth/logout", Some(&session), None)
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert!(
        response
            .headers
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .any(|value| value
                .to_str()
                .is_ok_and(|value| value.starts_with("pf_session=;")))
    );

    let response = app
        .send(Method::GET, "/api/auth/session", Some(&session), None)
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let revoked: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM auth_sessions WHERE user_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&app.pool)
    .await
    .expect("session row");
    assert!(revoked.is_some(), "logout must revoke the session row");

    let second = app.login(email, PASSWORD).await;
    let response = app
        .send(Method::GET, "/api/auth/sessions", Some(&second), None)
        .await;
    let current_id = response
        .body
        .as_array()
        .expect("sessions")
        .iter()
        .find(|session| session["current"] == true)
        .and_then(|session| session["id"].as_str())
        .expect("current session id")
        .to_owned();

    let response = app
        .send(
            Method::DELETE,
            &format!("/api/auth/sessions/{current_id}"),
            Some(&second),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let response = app
        .send(Method::GET, "/api/auth/session", Some(&second), None)
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let third = app.login(email, PASSWORD).await;
    let fourth = app.login(email, PASSWORD).await;
    let response = app
        .send(Method::POST, "/api/auth/logout-all", Some(&third), None)
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    for session in [&third, &fourth] {
        let response = app
            .send(Method::GET, "/api/auth/session", Some(session), None)
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }

    let audit_actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE actor_id = $1 ORDER BY created_at")
            .bind(user_id)
            .fetch_all(&app.pool)
            .await
            .expect("audit rows");
    assert!(audit_actions.iter().any(|action| action == "auth.logout"));
    assert!(
        audit_actions
            .iter()
            .any(|action| action == "auth.sessions_revoked")
    );

    app.cleanup().await;
}

#[tokio::test]
async fn csrf_origin_and_authentication_guards_are_enforced() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let email = "owner@example.test";
    app.create_user(email, "Owner", PASSWORD).await;
    let session = app.login(email, PASSWORD).await;

    let response = app
        .send_with_origin(
            Method::POST,
            "/api/projects",
            Some(&session),
            Some(json!({ "name": "Disallowed origin" })),
            "https://evil.example.test",
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let mut wrong_csrf = session.clone();
    wrong_csrf.csrf = "wrong".to_owned();
    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&wrong_csrf),
            Some(json!({ "name": "Wrong csrf" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&csrf_only(&session)),
            Some(json!({ "name": "Unauthenticated" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    let response = app
        .send_with_origin(
            Method::POST,
            "/api/projects",
            Some(&session),
            Some(json!({ "name": "Allowed origin" })),
            "http://127.0.0.1:5173",
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);

    let response = app.send(Method::GET, "/api/projects", None, None).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);

    app.cleanup().await;
}

#[tokio::test]
async fn project_rbac_and_cross_project_access_are_enforced() {
    let Some(app) = TestApp::setup().await else {
        return;
    };

    let owner_id = app
        .create_user("owner@example.test", "Owner", PASSWORD)
        .await;
    let collaborator_id = app
        .create_user("collaborator@example.test", "Collaborator", PASSWORD)
        .await;
    let _viewer_id = app
        .create_user("viewer@example.test", "Viewer", PASSWORD)
        .await;
    let _admin_id = app
        .create_user("admin@example.test", "Admin", PASSWORD)
        .await;
    let _outsider_id = app
        .create_user("outsider@example.test", "Outsider", PASSWORD)
        .await;

    let owner = app.login("owner@example.test", PASSWORD).await;
    let collaborator = app.login("collaborator@example.test", PASSWORD).await;
    let viewer = app.login("viewer@example.test", PASSWORD).await;
    let admin = app.login("admin@example.test", PASSWORD).await;
    let outsider = app.login("outsider@example.test", PASSWORD).await;

    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&outsider),
            Some(json!({ "name": "Outsider project", "slug": "outsider-project" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED);
    let outsider_project = project_id(&response);

    let response = app
        .send(
            Method::POST,
            "/api/projects",
            Some(&owner),
            Some(json!({ "name": "Alpha", "slug": "alpha" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    assert_eq!(
        response
            .headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let alpha = project_id(&response);
    assert_eq!(response.body["role"], "owner");
    assert_eq!(response.body["memberCount"], 1);

    for (email, role) in [
        ("collaborator@example.test", "collaborator"),
        ("viewer@example.test", "viewer"),
        ("admin@example.test", "admin"),
    ] {
        let response = app
            .send(
                Method::POST,
                &format!("/api/projects/{alpha}/members"),
                Some(&owner),
                Some(json!({ "email": email, "role": role })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
        assert_eq!(response.body["role"], role);
    }

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}"),
            Some(&collaborator),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["role"], "collaborator");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/projects/{alpha}"),
            Some(&collaborator),
            Some(json!({ "name": "Collaborator edit" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{alpha}/members"),
            Some(&collaborator),
            Some(json!({ "email": "outsider@example.test", "role": "viewer" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}/members"),
            Some(&viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body.as_array().expect("members").len(), 4);

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/projects/{alpha}"),
            Some(&viewer),
            Some(json!({ "name": "Viewer edit" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{alpha}/members"),
            Some(&viewer),
            Some(json!({ "email": "outsider@example.test", "role": "viewer" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}/audit"),
            Some(&viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    // A non-member receives 404 for every Alpha read/write/audit route.
    for (method, uri, body) in [
        (Method::GET, format!("/api/projects/{alpha}"), None),
        (
            Method::PATCH,
            format!("/api/projects/{alpha}"),
            Some(json!({ "name": "Break tenant" })),
        ),
        (Method::GET, format!("/api/projects/{alpha}/members"), None),
        (
            Method::POST,
            format!("/api/projects/{alpha}/members"),
            Some(json!({ "email": "viewer@example.test", "role": "viewer" })),
        ),
        (Method::GET, format!("/api/projects/{alpha}/audit"), None),
    ] {
        let response = app.send(method, &uri, Some(&outsider), body).await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "cross-project access was not denied for {uri}"
        );
    }

    // The owner of Alpha also cannot reach the outsider's project.
    for (method, uri, body) in [
        (
            Method::GET,
            format!("/api/projects/{outsider_project}"),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/projects/{outsider_project}"),
            Some(json!({ "name": "Cross-project edit" })),
        ),
        (
            Method::POST,
            format!("/api/projects/{outsider_project}/members"),
            Some(json!({ "email": "viewer@example.test", "role": "viewer" })),
        ),
    ] {
        let response = app.send(method, &uri, Some(&owner), body).await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "cross-project access was not denied for {uri}"
        );
    }

    // Admin can add a viewer but cannot change or remove the owner.
    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{alpha}/members"),
            Some(&admin),
            Some(json!({ "email": "outsider@example.test", "role": "viewer" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/projects/{alpha}/members/{owner_id}"),
            Some(&admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::DELETE,
            &format!("/api/projects/{alpha}/members/{owner_id}"),
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}"),
            Some(&outsider),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["role"], "viewer");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/projects/{alpha}"),
            Some(&outsider),
            Some(json!({ "name": "Viewer edit" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    // Only the owner can transfer ownership, and the former owner becomes admin.
    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{alpha}/transfer-owner"),
            Some(&admin),
            Some(json!({ "userId": collaborator_id })),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(
            Method::POST,
            &format!("/api/projects/{alpha}/transfer-owner"),
            Some(&owner),
            Some(json!({ "userId": collaborator_id })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["ownerId"], collaborator_id.to_string());
    assert_eq!(response.body["role"], "admin");

    let response = app
        .send(
            Method::PATCH,
            &format!("/api/projects/{alpha}"),
            Some(&collaborator),
            Some(json!({ "name": "Alpha Renamed" })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["name"], "Alpha Renamed");

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}/audit"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let actions = response
        .body
        .as_array()
        .expect("audit array")
        .iter()
        .filter_map(|row| row["action"].as_str())
        .collect::<Vec<_>>();
    assert!(actions.contains(&"project.created"));
    assert!(actions.contains(&"project.member_added"));
    assert!(actions.contains(&"project.owner_transferred"));
    assert!(actions.contains(&"project.updated"));

    let response = app
        .send(
            Method::GET,
            &format!("/api/projects/{alpha}/audit"),
            Some(&viewer),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    app.cleanup().await;
}

#[tokio::test]
async fn login_rate_limit_blocks_after_failures_and_success_resets_it() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let email = "rate-limit@example.test";
    app.create_user(email, "Rate Limit", PASSWORD).await;

    for attempt in 1..=5 {
        let response = app.login_attempt(email, "wrong password value").await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} should be rejected as invalid credentials"
        );
    }
    let response = app.login_attempt(email, "wrong password value").await;
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response
            .headers
            .get(axum::http::header::RETRY_AFTER)
            .is_some()
    );
    let response = app.login_attempt(email, PASSWORD).await;
    assert_eq!(
        response.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a blocked account cannot bypass the limit with the correct password"
    );

    // A different account starts clean; a successful login clears its keys.
    let other = "rate-reset@example.test";
    app.create_user(other, "Rate Reset", PASSWORD).await;
    for _ in 0..4 {
        let response = app.login_attempt(other, "wrong password value").await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    let _session = app.login(other, PASSWORD).await;
    for _ in 0..5 {
        let response = app.login_attempt(other, "wrong password value").await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    let response = app.login_attempt(other, "wrong password value").await;
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);

    app.cleanup().await;
}

#[tokio::test]
async fn readiness_requires_migrated_database() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let response = app.send(Method::GET, "/readyz", None, None).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["status"], "ready");
    assert_eq!(response.body["service"], "probe-forge-api");
    app.cleanup().await;
}
