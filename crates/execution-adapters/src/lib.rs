//! Small, vetted security adapters with typed configuration and normalized
//! results. No adapter builds commands, invokes a shell, or accepts executable
//! text from a model, plan, or target.
//!
//! The first adapter set targets HTTP response headers and CORS behavior through
//! the shared guarded client. Unsupported catalog IDs fail closed.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use probe_forge_target_policy::{
    GuardedHttpClient, RunBudget, TargetGuard, TargetPolicy, TargetRequestError, TargetResolver,
    validate_request_url,
};
use probeforge_contracts::Severity;
use reqwest::header::{HeaderMap, HeaderValue, ORIGIN};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

const PROBE_ORIGIN: &str = "https://probe.invalid";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_ADAPTER_NAME: &str = "http_probe";
const HTTP_ADAPTER_VERSION: &str = "0.1.0";

const REQUIRED_HTTP_HEADERS: &[&str] = &[
    "content-security-policy",
    "x-content-type-options",
    "referrer-policy",
];
const REQUIRED_HTTPS_HEADERS: &[&str] = &[
    "content-security-policy",
    "x-content-type-options",
    "referrer-policy",
    "strict-transport-security",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    HttpHeaders,
    HttpCors,
}

#[derive(Debug, Clone, Copy)]
pub struct AdapterSpec {
    pub catalog_test_id: &'static str,
    pub adapter_name: &'static str,
    pub adapter_version: &'static str,
    pub kind: AdapterKind,
    pub timeout: Duration,
}

pub const ADAPTER_SPECS: &[AdapterSpec] = &[
    AdapterSpec {
        catalog_test_id: "http.headers.missing",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpHeaders,
        timeout: HTTP_TIMEOUT,
    },
    AdapterSpec {
        catalog_test_id: "http.cors.misconfiguration",
        adapter_name: HTTP_ADAPTER_NAME,
        adapter_version: HTTP_ADAPTER_VERSION,
        kind: AdapterKind::HttpCors,
        timeout: HTTP_TIMEOUT,
    },
];

pub fn adapters() -> &'static [AdapterSpec] {
    ADAPTER_SPECS
}

pub fn find_adapter(catalog_test_id: &str) -> Option<&'static AdapterSpec> {
    ADAPTER_SPECS
        .iter()
        .find(|spec| spec.catalog_test_id == catalog_test_id)
}

pub fn supported_catalog_ids() -> impl Iterator<Item = &'static str> {
    ADAPTER_SPECS.iter().map(|spec| spec.catalog_test_id)
}

pub fn is_supported(catalog_test_id: &str) -> bool {
    find_adapter(catalog_test_id).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityOutcome {
    Passed,
    Finding,
    Inconclusive,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterEvidence {
    pub kind: String,
    pub summary: String,
    pub attributes: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterResult {
    pub catalog_test_id: String,
    pub adapter_name: String,
    pub adapter_version: String,
    pub execution_status: ExecutionStatus,
    pub security_outcome: SecurityOutcome,
    pub severity: Option<Severity>,
    pub confidence: f64,
    pub summary: String,
    pub duration_ms: u64,
    pub evidence: Vec<AdapterEvidence>,
    pub error_code: Option<String>,
}

#[derive(Clone)]
pub struct AdapterEnvironment {
    pub guard: TargetGuard,
    pub policy: TargetPolicy,
    pub resolver: Arc<dyn TargetResolver>,
}

pub async fn execute(
    catalog_test_id: &str,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    let Some(spec) = find_adapter(catalog_test_id) else {
        return unsupported_result(catalog_test_id);
    };
    execute_spec(spec, environment, configuration, budget, spec.timeout).await
}

pub async fn execute_with_timeout(
    catalog_test_id: &str,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
    timeout: Duration,
) -> AdapterResult {
    let Some(spec) = find_adapter(catalog_test_id) else {
        return unsupported_result(catalog_test_id);
    };
    execute_spec(spec, environment, configuration, budget, timeout).await
}

async fn execute_spec(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
    timeout: Duration,
) -> AdapterResult {
    let started = Instant::now();
    let mut result = match tokio::time::timeout(
        timeout,
        run_adapter(spec, environment, configuration, budget),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "adapter_timeout",
            "The adapter exceeded its execution timeout.",
        ),
    };
    result.duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    result
}

async fn run_adapter(
    spec: &'static AdapterSpec,
    environment: AdapterEnvironment,
    configuration: &Value,
    budget: &RunBudget,
) -> AdapterResult {
    let path = match route_from_configuration(configuration) {
        Ok(path) => path,
        Err(summary) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_adapter_configuration",
                summary,
            );
        }
    };
    let origin = match Url::parse(&environment.guard.scope.origin) {
        Ok(origin) => origin,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_target_scope",
                "The authorized target origin is invalid.",
            );
        }
    };
    let url = match origin.join(&path) {
        Ok(url) => url,
        Err(_) => {
            return failure_result(
                spec,
                ExecutionStatus::Failed,
                "invalid_adapter_configuration",
                "The test route is not a valid target path.",
            );
        }
    };
    if let Err(error) = validate_request_url(&environment.guard, &url, &environment.policy) {
        return failure_result(
            spec,
            ExecutionStatus::Failed,
            error.code(),
            "The test route is outside the authorized target scope.",
        );
    }

    let client = GuardedHttpClient::new(environment.policy, environment.resolver);
    let response = if spec.kind == AdapterKind::HttpCors {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static(PROBE_ORIGIN));
        client
            .get_with_headers(&environment.guard, &url, &headers, budget)
            .await
    } else {
        client.get(&environment.guard, &url, budget).await
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => return request_failure(spec, error),
    };

    if response.status >= 500 {
        return AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Inconclusive,
            severity: None,
            confidence: 0.3,
            summary:
                "The target returned a server error; the security observation is inconclusive."
                    .to_owned(),
            duration_ms: 0,
            evidence: vec![status_evidence(&response)],
            error_code: None,
        };
    }

    match spec.kind {
        AdapterKind::HttpHeaders => normalize_headers(spec, &response),
        AdapterKind::HttpCors => normalize_cors(spec, &response),
    }
}

fn normalize_headers(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let scheme = response.final_url.scheme();
    let required = if scheme == "https" {
        REQUIRED_HTTPS_HEADERS
    } else {
        REQUIRED_HTTP_HEADERS
    };
    let mut missing = Vec::new();
    let mut present = Vec::new();
    for header in required {
        if response.headers.contains_key(*header) {
            present.push((*header).to_owned());
        } else {
            missing.push((*header).to_owned());
        }
    }
    let evidence = AdapterEvidence {
        kind: "http_response_headers".to_owned(),
        summary: "Observed security headers for the authorized route.".to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
            "scheme": scheme,
            "missingHeaders": missing.clone(),
            "presentHeaders": present.clone(),
        }),
    };
    if missing.is_empty() {
        AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Passed,
            severity: None,
            confidence: 0.8,
            summary: "Required baseline security headers were present.".to_owned(),
            duration_ms: 0,
            evidence: vec![evidence],
            error_code: None,
        }
    } else {
        AdapterResult {
            catalog_test_id: spec.catalog_test_id.to_owned(),
            adapter_name: spec.adapter_name.to_owned(),
            adapter_version: spec.adapter_version.to_owned(),
            execution_status: ExecutionStatus::Completed,
            security_outcome: SecurityOutcome::Finding,
            severity: Some(Severity::Low),
            confidence: 0.9,
            summary: format!("Missing baseline security headers: {}.", missing.join(", ")),
            duration_ms: 0,
            evidence: vec![evidence],
            error_code: None,
        }
    }
}

fn normalize_cors(
    spec: &'static AdapterSpec,
    response: &probe_forge_target_policy::GuardedResponse,
) -> AdapterResult {
    let allow_origin = response
        .headers
        .get("access-control-allow-origin")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    let allow_credentials = response
        .headers
        .get("access-control-allow-credentials")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let reflected = allow_origin.eq_ignore_ascii_case(PROBE_ORIGIN);
    let wildcard = allow_origin == "*";
    let (outcome, severity, confidence, summary) = if wildcard && allow_credentials {
        (
            SecurityOutcome::Finding,
            Some(Severity::High),
            0.95,
            "CORS allows any origin together with credentials.",
        )
    } else if reflected {
        (
            SecurityOutcome::Finding,
            Some(if allow_credentials {
                Severity::High
            } else {
                Severity::Medium
            }),
            0.9,
            if allow_credentials {
                "CORS reflects an arbitrary origin and allows credentials."
            } else {
                "CORS reflects an arbitrary origin."
            },
        )
    } else {
        (
            SecurityOutcome::Passed,
            None,
            0.8,
            "No overly broad CORS origin policy was observed.",
        )
    };
    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status: ExecutionStatus::Completed,
        security_outcome: outcome,
        severity,
        confidence,
        summary: summary.to_owned(),
        duration_ms: 0,
        evidence: vec![AdapterEvidence {
            kind: "cors_response".to_owned(),
            summary: "Observed CORS response headers for a harmless probe origin.".to_owned(),
            attributes: json!({
                "status": response.status,
                "finalPath": response.final_url.path(),
                "requestOrigin": PROBE_ORIGIN,
                "allowOrigin": allow_origin,
                "allowCredentials": allow_credentials,
            }),
        }],
        error_code: None,
    }
}

fn status_evidence(response: &probe_forge_target_policy::GuardedResponse) -> AdapterEvidence {
    AdapterEvidence {
        kind: "http_response_status".to_owned(),
        summary: "Observed response status without reading evidence from the body.".to_owned(),
        attributes: json!({
            "status": response.status,
            "finalPath": response.final_url.path(),
        }),
    }
}

fn request_failure(spec: &'static AdapterSpec, error: TargetRequestError) -> AdapterResult {
    match error {
        TargetRequestError::Cancelled => failure_result(
            spec,
            ExecutionStatus::Cancelled,
            "cancelled",
            "The adapter was cancelled.",
        ),
        TargetRequestError::DurationExceeded => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "duration_budget_exhausted",
            "The run duration budget was exhausted.",
        ),
        TargetRequestError::MaxRequests => failure_result(
            spec,
            ExecutionStatus::Failed,
            "request_budget_exhausted",
            "The request budget was exhausted.",
        ),
        TargetRequestError::ResponseTooLarge => failure_result(
            spec,
            ExecutionStatus::Failed,
            "response_size_exceeded",
            "The response exceeded the configured size cap.",
        ),
        TargetRequestError::TotalBytesExceeded => failure_result(
            spec,
            ExecutionStatus::Failed,
            "total_data_budget_exhausted",
            "The run total-data budget was exhausted.",
        ),
        TargetRequestError::InvalidHeader => failure_result(
            spec,
            ExecutionStatus::Failed,
            "invalid_adapter_header",
            "The adapter attempted to send a disallowed request header.",
        ),
        TargetRequestError::RequestBodyTooLarge => failure_result(
            spec,
            ExecutionStatus::Failed,
            "request_body_too_large",
            "The adapter request exceeded the platform body cap.",
        ),
        TargetRequestError::RequestTimeout => failure_result(
            spec,
            ExecutionStatus::TimedOut,
            "request_timeout",
            "The target request timed out.",
        ),
        TargetRequestError::Network => failure_result(
            spec,
            ExecutionStatus::Failed,
            "network_error",
            "The target request failed at the network boundary.",
        ),
        TargetRequestError::Validation(error) => failure_result(
            spec,
            ExecutionStatus::Failed,
            error.code(),
            error.safe_message(),
        ),
    }
}

fn failure_result(
    spec: &'static AdapterSpec,
    execution_status: ExecutionStatus,
    error_code: &str,
    summary: &str,
) -> AdapterResult {
    AdapterResult {
        catalog_test_id: spec.catalog_test_id.to_owned(),
        adapter_name: spec.adapter_name.to_owned(),
        adapter_version: spec.adapter_version.to_owned(),
        execution_status,
        security_outcome: SecurityOutcome::Inconclusive,
        severity: None,
        confidence: 0.0,
        summary: summary.to_owned(),
        duration_ms: 0,
        evidence: Vec::new(),
        error_code: Some(error_code.to_owned()),
    }
}

fn unsupported_result(catalog_test_id: &str) -> AdapterResult {
    AdapterResult {
        catalog_test_id: catalog_test_id.to_owned(),
        adapter_name: "none".to_owned(),
        adapter_version: "none".to_owned(),
        execution_status: ExecutionStatus::Unsupported,
        security_outcome: SecurityOutcome::NotRun,
        severity: None,
        confidence: 0.0,
        summary: "No vetted adapter exists for this catalog test; execution is disabled."
            .to_owned(),
        duration_ms: 0,
        evidence: Vec::new(),
        error_code: Some("unsupported_catalog_test".to_owned()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteConfiguration {
    route: Option<String>,
    path: Option<String>,
}

fn route_from_configuration(configuration: &Value) -> Result<String, &'static str> {
    let parsed: RouteConfiguration = serde_json::from_value(configuration.clone())
        .map_err(|_| "Test configuration must contain only a route or path.")?;
    let route = match (parsed.route, parsed.path) {
        (Some(route), None) => route,
        (None, Some(path)) => path,
        (Some(route), Some(path)) if route == path => route,
        _ => {
            return Err("Test configuration must contain exactly one route or path value.");
        }
    };
    let route = route.trim();
    if !route.starts_with('/')
        || route.len() > 2_000
        || route.contains('?')
        || route.contains('#')
        || route.contains('\\')
        || route.contains("..")
        || route.contains("//")
        || route.chars().any(char::is_control)
    {
        return Err("Test route must be a safe absolute path.");
    }
    Ok(route.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    struct MockServer {
        port: u16,
        requests: Arc<AtomicUsize>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn start_mock_server() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("mock bind");
        let port = listener.local_addr().expect("mock addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_server = requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_server.clone();
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 2048];
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    requests.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    match path {
                        "/headers-missing" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Content-Security-Policy", "default-src 'none'"),
                                    ("X-Content-Type-Options", "nosniff"),
                                    ("Set-Cookie", "session=server-secret"),
                                ],
                                "SECRET_RESPONSE_BODY",
                            )
                            .await;
                        }
                        "/headers-present" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Content-Security-Policy", "default-src 'none'"),
                                    ("X-Content-Type-Options", "nosniff"),
                                    ("Referrer-Policy", "no-referrer"),
                                    ("Strict-Transport-Security", "max-age=31536000"),
                                ],
                                "ok",
                            )
                            .await;
                        }
                        "/cors-reflect" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Access-Control-Allow-Origin", PROBE_ORIGIN),
                                    ("Access-Control-Allow-Credentials", "true"),
                                    ("Set-Cookie", "session=cors-server-secret"),
                                ],
                                "SECRET_CORS_BODY",
                            )
                            .await;
                        }
                        "/cors-safe" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[("Access-Control-Allow-Origin", "*")],
                                "ok",
                            )
                            .await;
                        }
                        "/cors-wildcard-credentials" => {
                            write_response(
                                &mut stream,
                                "200 OK",
                                &[
                                    ("Access-Control-Allow-Origin", "*"),
                                    ("Access-Control-Allow-Credentials", "true"),
                                ],
                                "ok",
                            )
                            .await;
                        }
                        "/server-error" => {
                            write_response(&mut stream, "500 Internal Server Error", &[], "error")
                                .await;
                        }
                        "/large" => {
                            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n";
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                        "/slow" => {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            write_response(&mut stream, "200 OK", &[], "slow").await;
                        }
                        _ => {
                            write_response(&mut stream, "404 Not Found", &[], "missing").await;
                        }
                    }
                });
            }
        });
        MockServer {
            port,
            requests,
            handle,
        }
    }

    fn environment(port: u16) -> AdapterEnvironment {
        AdapterEnvironment {
            guard: TargetGuard {
                environment: probeforge_contracts::Environment::Local,
                scope: probeforge_contracts::TargetScope {
                    origin: format!("http://127.0.0.1:{port}"),
                    allowed_hosts: vec!["127.0.0.1".to_owned()],
                    allowed_paths: vec!["/".to_owned()],
                    allowed_ports: vec![port],
                },
            },
            policy: TargetPolicy::local_test(),
            resolver: Arc::new(probe_forge_target_policy::SystemResolver),
        }
    }

    fn budget() -> RunBudget {
        RunBudget::new(
            probeforge_contracts::TargetLimits {
                max_requests_per_second: 100.0,
                max_concurrency: 1,
                max_duration_seconds: 5,
                max_requests: 10,
            },
            &TargetPolicy::local_test(),
        )
    }

    #[test]
    fn registry_is_small_versioned_and_unique() {
        assert_eq!(
            supported_catalog_ids().collect::<Vec<_>>(),
            vec!["http.headers.missing", "http.cors.misconfiguration"]
        );
        let mut seen = std::collections::HashSet::new();
        for spec in adapters() {
            assert!(seen.insert(spec.catalog_test_id));
            assert!(!spec.adapter_name.is_empty());
            assert!(!spec.adapter_version.is_empty());
        }
    }

    #[tokio::test]
    async fn normalizes_missing_and_present_security_headers_without_body_leakage() {
        let server = start_mock_server().await;
        let environment = environment(server.port);
        let missing = execute(
            "http.headers.missing",
            environment.clone(),
            &json!({"route": "/headers-missing"}),
            &budget(),
        )
        .await;
        assert_eq!(missing.execution_status, ExecutionStatus::Completed);
        assert_eq!(missing.security_outcome, SecurityOutcome::Finding);
        assert_eq!(missing.severity, Some(Severity::Low));
        assert!(missing.summary.contains("referrer-policy"));
        let serialized = serde_json::to_string(&missing).expect("serialize result");
        assert!(!serialized.contains("SECRET_RESPONSE_BODY"));
        assert!(!serialized.contains("server-secret"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);

        let present = execute(
            "http.headers.missing",
            environment,
            &json!({"path": "/headers-present"}),
            &budget(),
        )
        .await;
        assert_eq!(present.execution_status, ExecutionStatus::Completed);
        assert_eq!(present.security_outcome, SecurityOutcome::Passed);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn detects_reflected_cors_with_credentials_and_accepts_wildcard_without_credentials() {
        let server = start_mock_server().await;
        let environment = environment(server.port);
        let reflected = execute(
            "http.cors.misconfiguration",
            environment.clone(),
            &json!({"route": "/cors-reflect"}),
            &budget(),
        )
        .await;
        assert_eq!(reflected.execution_status, ExecutionStatus::Completed);
        assert_eq!(reflected.security_outcome, SecurityOutcome::Finding);
        assert_eq!(reflected.severity, Some(Severity::High));
        assert!(reflected.summary.contains("reflects"));
        let serialized = serde_json::to_string(&reflected).expect("serialize CORS result");
        assert!(!serialized.contains("SECRET_CORS_BODY"));
        assert!(!serialized.contains("cors-server-secret"));

        let safe = execute(
            "http.cors.misconfiguration",
            environment.clone(),
            &json!({"route": "/cors-safe"}),
            &budget(),
        )
        .await;
        assert_eq!(safe.security_outcome, SecurityOutcome::Passed);

        let wildcard_credentials = execute(
            "http.cors.misconfiguration",
            environment,
            &json!({"route": "/cors-wildcard-credentials"}),
            &budget(),
        )
        .await;
        assert_eq!(
            wildcard_credentials.security_outcome,
            SecurityOutcome::Finding
        );
        assert_eq!(wildcard_credentials.severity, Some(Severity::High));
        assert!(wildcard_credentials.summary.contains("any origin"));
    }

    #[tokio::test]
    async fn unsupported_ids_and_invalid_routes_fail_closed_without_requests() {
        let server = start_mock_server().await;
        let unsupported = execute(
            "static.secrets.exposure",
            environment(server.port),
            &json!({"route": "/headers-present"}),
            &budget(),
        )
        .await;
        assert_eq!(unsupported.execution_status, ExecutionStatus::Unsupported);
        assert_eq!(unsupported.security_outcome, SecurityOutcome::NotRun);
        assert_eq!(
            unsupported.error_code.as_deref(),
            Some("unsupported_catalog_test")
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        for catalog_test_id in supported_catalog_ids() {
            let missing = execute(
                catalog_test_id,
                environment(server.port),
                &json!({}),
                &budget(),
            )
            .await;
            assert_eq!(missing.execution_status, ExecutionStatus::Failed);
            assert_eq!(missing.security_outcome, SecurityOutcome::Inconclusive);
            assert_eq!(
                missing.error_code.as_deref(),
                Some("invalid_adapter_configuration")
            );

            let unknown_field = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/headers-present", "command": "whoami"}),
                &budget(),
            )
            .await;
            assert_eq!(unknown_field.execution_status, ExecutionStatus::Failed);
            assert_eq!(
                unknown_field.error_code.as_deref(),
                Some("invalid_adapter_configuration")
            );
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        let invalid = execute(
            "http.headers.missing",
            environment(server.port),
            &json!({"route": "/../admin"}),
            &budget(),
        )
        .await;
        assert_eq!(invalid.execution_status, ExecutionStatus::Failed);
        assert_eq!(invalid.security_outcome, SecurityOutcome::Inconclusive);
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn timeout_cancellation_and_server_errors_are_separate_from_security_outcome() {
        let server = start_mock_server().await;
        for catalog_test_id in supported_catalog_ids() {
            let slow = execute_with_timeout(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/slow"}),
                &budget(),
                Duration::from_millis(50),
            )
            .await;
            assert_eq!(slow.execution_status, ExecutionStatus::TimedOut);
            assert_eq!(slow.security_outcome, SecurityOutcome::Inconclusive);

            let cancelled_budget = budget();
            cancelled_budget.cancel();
            let cancelled = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/headers-present"}),
                &cancelled_budget,
            )
            .await;
            assert_eq!(cancelled.execution_status, ExecutionStatus::Cancelled);
            assert_eq!(cancelled.security_outcome, SecurityOutcome::Inconclusive);

            let server_error = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/server-error"}),
                &budget(),
            )
            .await;
            assert_eq!(server_error.execution_status, ExecutionStatus::Completed);
            assert_eq!(server_error.security_outcome, SecurityOutcome::Inconclusive);
            assert!(server_error.severity.is_none());
        }
    }

    #[tokio::test]
    async fn every_adapter_enforces_the_response_size_cap_before_reading_a_body() {
        let server = start_mock_server().await;
        for catalog_test_id in supported_catalog_ids() {
            let oversized = execute(
                catalog_test_id,
                environment(server.port),
                &json!({"route": "/large"}),
                &budget(),
            )
            .await;
            assert_eq!(oversized.execution_status, ExecutionStatus::Failed);
            assert_eq!(oversized.security_outcome, SecurityOutcome::Inconclusive);
            assert_eq!(
                oversized.error_code.as_deref(),
                Some("response_size_exceeded")
            );
        }
    }
}
