//! Platform admin settings tests: guard, defaults, and non-secret updates.

mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::TestApp;

const PASSWORD: &str = "correct horse battery staple";

async fn make_admin(app: &TestApp, email: &str) -> support::Session {
    let user_id = app.create_user(email, "Platform Admin", PASSWORD).await;
    sqlx::query("INSERT INTO platform_admins (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .execute(&app.pool)
        .await
        .expect("grant platform admin");
    app.login(email, PASSWORD).await
}

fn full_update(provider: &str, model: &str) -> serde_json::Value {
    json!({
        "planningProvider": provider,
        "planningModel": model,
        "planningLimits": { "maxTurns": 6, "maxModelCalls": 4 },
        "apiRequestsPerMinute": 300,
        "expensiveRequestsPerMinute": 30,
        "retentionEventDays": 14,
        "retentionReportDays": 120,
        "retentionCounterDays": 5,
        "defaultMaxActiveRuns": 3,
        "defaultMaxRequestsPerWindow": 5000,
        "defaultWindowSeconds": 86400
    })
}

fn custom_endpoint_update(base_url: &str) -> serde_json::Value {
    json!({
        "planningProvider": "nebius",
        "planningModel": "deepseek-ai/DeepSeek-V3",
        "planningBaseUrl": base_url,
        "planningContextWindow": 131072,
        "planningMaxOutputTokens": 8192,
        "planningLimits": {},
        "apiRequestsPerMinute": 240,
        "expensiveRequestsPerMinute": 20,
        "retentionEventDays": 30,
        "retentionReportDays": 180,
        "retentionCounterDays": 7,
        "defaultMaxActiveRuns": 2,
        "defaultMaxRequestsPerWindow": null,
        "defaultWindowSeconds": 86400
    })
}

#[tokio::test]
async fn settings_require_admin_and_round_trip_non_secret_values() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let admin = make_admin(&app, "admin-settings@example.test").await;
    app.create_user("plain-user@example.test", "Plain", PASSWORD)
        .await;
    let plain = app.login("plain-user@example.test", PASSWORD).await;

    // Non-admin is forbidden.
    let denied = app
        .send(Method::GET, "/api/admin/settings", Some(&plain), None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);

    // Admin reads defaults.
    let initial = app
        .send(Method::GET, "/api/admin/settings", Some(&admin), None)
        .await;
    assert_eq!(initial.status, StatusCode::OK, "{:?}", initial.body);
    assert!(initial.body["planning"]["apiKeyConfigured"] == false);

    // Admin updates non-secret settings.
    let updated = app
        .send(
            Method::PUT,
            "/api/admin/settings",
            Some(&admin),
            Some(full_update("anthropic", "claude-sonnet-4-6")),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{:?}", updated.body);
    assert_eq!(updated.body["planning"]["provider"], "anthropic");
    assert_eq!(updated.body["rateLimits"]["apiRequestsPerMinute"], 300);
    assert_eq!(updated.body["retention"]["reportDays"], 120);
    assert_eq!(updated.body["projectDefaults"]["maxActiveRuns"], 3);

    // The persisted row reflects the change.
    let provider: Option<String> =
        sqlx::query_scalar("SELECT planning_provider FROM platform_settings WHERE id = true")
            .fetch_one(&app.pool)
            .await
            .expect("settings row");
    assert_eq!(provider.as_deref(), Some("anthropic"));

    // Invalid identifiers and out-of-range values are rejected.
    let bad_provider = app
        .send(
            Method::PUT,
            "/api/admin/settings",
            Some(&admin),
            Some(full_update("Bad Provider!", "model")),
        )
        .await;
    assert_eq!(bad_provider.status, StatusCode::BAD_REQUEST);

    let bad_range = app
        .send(
            Method::PUT,
            "/api/admin/settings",
            Some(&admin),
            Some(json!({
                "planningProvider": null,
                "planningModel": null,
                "planningLimits": {},
                "apiRequestsPerMinute": 1,
                "expensiveRequestsPerMinute": 30,
                "retentionEventDays": 14,
                "retentionReportDays": 120,
                "retentionCounterDays": 5,
                "defaultMaxActiveRuns": 3,
                "defaultMaxRequestsPerWindow": null,
                "defaultWindowSeconds": 86400
            })),
        )
        .await;
    assert_eq!(bad_range.status, StatusCode::BAD_REQUEST);

    app.cleanup().await;
}

#[tokio::test]
async fn settings_round_trip_a_custom_openai_compatible_endpoint() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let admin = make_admin(&app, "admin-custom-provider@example.test").await;

    // A public HTTPS OpenAI-compatible endpoint is accepted and stored.
    let updated = app
        .send(
            Method::PUT,
            "/api/admin/settings",
            Some(&admin),
            Some(custom_endpoint_update("https://api.studio.nebius.com/v1")),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{:?}", updated.body);
    assert_eq!(
        updated.body["planning"]["baseUrl"],
        "https://api.studio.nebius.com/v1"
    );
    assert_eq!(updated.body["planning"]["contextWindow"], 131_072);
    assert_eq!(updated.body["planning"]["maxOutputTokens"], 8_192);

    // Non-HTTPS and internal base URLs are rejected before persistence.
    for base_url in ["http://api.studio.nebius.com/v1", "https://127.0.0.1/v1"] {
        let rejected = app
            .send(
                Method::PUT,
                "/api/admin/settings",
                Some(&admin),
                Some(custom_endpoint_update(base_url)),
            )
            .await;
        assert_eq!(
            rejected.status,
            StatusCode::BAD_REQUEST,
            "{:?}",
            rejected.body
        );
        assert_eq!(rejected.body["error"]["code"], "invalid_planning_base_url");
    }

    app.cleanup().await;
}

#[tokio::test]
async fn settings_never_expose_a_stored_key() {
    let Some(app) = TestApp::setup().await else {
        return;
    };
    let admin = make_admin(&app, "admin-key@example.test").await;

    // Without a master key configured, storing a key fails closed.
    if std::env::var("PROBE_FORGE_SETTINGS_KEY").is_err() {
        let mut with_key = full_update("anthropic", "claude-sonnet-4-6");
        with_key["apiKey"] = json!("sk-secret-value");
        let refused = app
            .send(
                Method::PUT,
                "/api/admin/settings",
                Some(&admin),
                Some(with_key),
            )
            .await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{:?}", refused.body);
        assert_eq!(refused.body["error"]["code"], "settings_key_not_configured");
    }

    app.cleanup().await;
}
