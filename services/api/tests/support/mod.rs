#![allow(dead_code)]

use std::path::Path;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use probe_forge_api::{AppState, AuthConfig, auth::password::hash_password, router};
use probe_forge_target_policy::TargetPolicy;
use serde_json::{Value, json};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

pub struct TestApp {
    pub admin: PgPool,
    pub pool: PgPool,
    pub router: Router,
    schema: String,
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

pub struct TestRawResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct Session {
    pub cookie_header: String,
    pub csrf: String,
    pub user: Value,
}

impl TestApp {
    pub async fn setup() -> Option<Self> {
        Self::setup_with_target_policy(TargetPolicy::default()).await
    }

    pub async fn setup_with_private_targets() -> Option<Self> {
        Self::setup_with_target_policy(TargetPolicy::local_test()).await
    }

    async fn setup_with_target_policy(target_policy: TargetPolicy) -> Option<Self> {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                assert_ne!(
                    std::env::var("PROBE_FORGE_REQUIRE_DB_TESTS").as_deref(),
                    Ok("1"),
                    "DATABASE_URL is required when PROBE_FORGE_REQUIRE_DB_TESTS=1"
                );
                eprintln!("skipping API integration test: DATABASE_URL is not set");
                return None;
            }
        };

        // UUIDs avoid collisions between parallel tests on coarse Windows clocks.
        let schema = format!("pf_test_{}_{}", std::process::id(), Uuid::new_v4().simple());

        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("database connection failed");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("test schema creation failed");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .after_connect({
                let schema = schema.clone();
                move |connection, _metadata| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                            .execute(connection)
                            .await?;
                        Ok(())
                    })
                }
            })
            .connect(&database_url)
            .await
            .expect("schema-scoped database connection failed");

        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let migrator = Migrator::new(migrations_dir)
            .await
            .expect("migrations directory could not be loaded");
        migrator.run(&pool).await.expect("migrations failed");

        let state =
            AppState::new_with_target_policy(pool.clone(), AuthConfig::local_test(), target_policy);
        let router = router(state);

        Some(Self {
            admin,
            pool,
            router,
            schema,
        })
    }

    pub async fn cleanup(&self) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("test schema cleanup failed");
    }

    pub async fn create_user(&self, email: &str, display_name: &str, password: &str) -> Uuid {
        let password_hash = hash_password(password).expect("password hash");
        sqlx::query_scalar(
            "INSERT INTO users (email, display_name, password_hash)
             VALUES ($1, $2, $3)
             RETURNING id",
        )
        .bind(email)
        .bind(display_name)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .expect("user insert")
    }

    pub async fn send(
        &self,
        method: Method,
        uri: &str,
        session: Option<&Session>,
        body: Option<Value>,
    ) -> TestResponse {
        send_request(&self.router, method, uri, session, body, None, None).await
    }

    pub async fn send_with_idempotency_key(
        &self,
        method: Method,
        uri: &str,
        session: Option<&Session>,
        body: Option<Value>,
        idempotency_key: &str,
    ) -> TestResponse {
        send_request(
            &self.router,
            method,
            uri,
            session,
            body,
            None,
            Some(("idempotency-key", idempotency_key)),
        )
        .await
    }

    pub async fn send_with_origin(
        &self,
        method: Method,
        uri: &str,
        session: Option<&Session>,
        body: Option<Value>,
        origin: &str,
    ) -> TestResponse {
        send_request(&self.router, method, uri, session, body, Some(origin), None).await
    }

    pub async fn send_raw_with_last_event_id(
        &self,
        method: Method,
        uri: &str,
        session: Option<&Session>,
        last_event_id: Option<&str>,
    ) -> TestRawResponse {
        send_raw_request(&self.router, method, uri, session, last_event_id).await
    }

    pub async fn send_raw(
        &self,
        method: Method,
        uri: &str,
        session: Option<&Session>,
    ) -> TestRawResponse {
        send_raw_request(&self.router, method, uri, session, None).await
    }

    pub async fn login(&self, email: &str, password: &str) -> Session {
        let response = self.login_attempt(email, password).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "login failed: {:?}",
            response.body
        );
        let session_token =
            cookie_value(&response.headers, "pf_session").expect("session Set-Cookie");
        let csrf_cookie = cookie_value(&response.headers, "pf_csrf").expect("csrf Set-Cookie");
        let csrf_token = response.body["csrfToken"]
            .as_str()
            .expect("login response csrf token")
            .to_owned();
        assert_eq!(csrf_cookie, csrf_token);
        Session {
            cookie_header: format!("pf_session={session_token}; pf_csrf={csrf_cookie}"),
            csrf: csrf_token,
            user: response.body["user"].clone(),
        }
    }

    pub async fn login_attempt(&self, email: &str, password: &str) -> TestResponse {
        let csrf = self.csrf_bootstrap().await;
        send_request(
            &self.router,
            Method::POST,
            "/api/auth/login",
            Some(&csrf),
            Some(json!({ "email": email, "password": password })),
            None,
            None,
        )
        .await
    }

    async fn csrf_bootstrap(&self) -> Session {
        let response = send_request(
            &self.router,
            Method::GET,
            "/api/auth/csrf",
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(response.status, StatusCode::OK);
        let token = response.body["csrfToken"]
            .as_str()
            .expect("csrf token")
            .to_owned();
        let cookie = cookie_value(&response.headers, "pf_csrf").expect("csrf Set-Cookie");
        Session {
            cookie_header: format!("pf_csrf={cookie}"),
            csrf: token,
            user: Value::Null,
        }
    }
}

async fn send_request(
    router: &Router,
    method: Method,
    uri: &str,
    session: Option<&Session>,
    body: Option<Value>,
    origin: Option<&str>,
    extra_header: Option<(&str, &str)>,
) -> TestResponse {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(session) = session {
        builder = builder.header(header::COOKIE, &session.cookie_header);
        builder = builder.header("x-csrf-token", &session.csrf);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some((name, value)) = extra_header {
        builder = builder.header(name, value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).expect("json body"))
        }
        None => Body::empty(),
    };
    let request = builder.body(body).expect("request");
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    TestResponse {
        status,
        headers,
        body,
    }
}

async fn send_raw_request(
    router: &Router,
    method: Method,
    uri: &str,
    session: Option<&Session>,
    last_event_id: Option<&str>,
) -> TestRawResponse {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(session) = session {
        builder = builder.header(header::COOKIE, &session.cookie_header);
        builder = builder.header("x-csrf-token", &session.csrf);
    }
    if let Some(last_event_id) = last_event_id {
        builder = builder.header("last-event-id", last_event_id);
    }
    let request = builder.body(Body::empty()).expect("request");
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body")
        .to_vec();
    TestRawResponse {
        status,
        headers,
        body,
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::SET_COOKIE) {
        let value = value.to_str().ok()?;
        let first = value.split(';').next()?;
        let (cookie_name, cookie_value) = first.split_once('=')?;
        if cookie_name == name {
            return Some(cookie_value.to_owned());
        }
    }
    None
}
